use crate::daecore::daetype::subsetter::GlyphSet;
use super::*;
use crate::daecore::daetype::TableBytes;

// Declared once because `subset_text_rs`'s CFF branch and `subset_cff_flavored` must strip exactly
// the same set; they carried identical copies seventy lines apart.
const STRIP_FOR_CFF_SUBSET: &[&str] = &[
    "fvar", "gvar", "avar", "cvar", "HVAR", "VVAR", "MVAR", "STAT", "CFF2", "CFF ", "COLR", "CPAL",
];

fn n_source(source: &BTreeMap<String, TableBytes>) -> u16 {
    source.get("maxp").and_then(|m| read_u16_be(m, 4)).unwrap_or(0)
}

fn closure_loop<'a>(
    table: impl Fn(&str) -> Option<&'a [u8]>,
    num_glyphs: u16,
    requested: &[u16],
    // `reach` owns insertion rather than returning ids, which is what preserves a real difference:
    // `active_gids_into` filters against numGlyphs and the CFF walk deliberately does not, since a
    // caller may name a gid this font lacks and expect it ignored rather than reinterpreted.
    mut reach: impl FnMut(&[u16], &mut GlyphSet),
) -> Result<GlyphSet, String> {
    let mut active = GlyphSet::new();
    let seed: Vec<u16> = core::iter::once(0).chain(requested.iter().copied()).collect();
    reach(&seed, &mut active);

    const MAX_CLOSURE_PASSES: usize = 64;
    let mut steps = 0usize;
    loop {
        steps += 1;
        if steps > MAX_CLOSURE_PASSES {
            return Err("subset: glyph closure did not converge within its pass budget".into());
        }
        let mut new_gids: Vec<u16> = Vec::new();
        if let Some(colr) = table("COLR") {
            new_gids.extend(crate::daecore::daetype::subsetter::colr::colr_closure(colr, &active));
        }
        if let Some(gsub) = table("GSUB") {
            new_gids.extend(crate::daecore::daetype::subsetter::gsub_closure(gsub, &active));
        }
        if let Some(math) = table("MATH") {
            new_gids.extend(crate::daecore::daetype::subsetter::math_closure(math, &active));
        }
        if let Some(morx) = table("morx") {
            new_gids.extend(crate::daecore::daetype::subsetter::morx_closure(morx, &active, num_glyphs));
        }
        new_gids.retain(|g| !active.contains(g));
        if new_gids.is_empty() { break; }
        reach(&new_gids, &mut active);
    }
    Ok(active)
}

pub fn cff_color_closure(
    cff: &[u8], source: &BTreeMap<String, TableBytes>, requested: &[u16],
) -> Result<GlyphSet, String> {
    let closure_inputs = crate::daecore::daetype::subsetter::cff_charstrings_for_closure(cff).ok();
    closure_loop(
        |tag| source.get(tag).map(|t| t.as_slice()),
        n_source(source),
        requested,
        |gids, set| {
            let mut frontier: Vec<u16> = gids.iter().copied().filter(|&g| set.insert(g)).collect();
            if let Some((charstrings, charset_off)) = &closure_inputs {
                while !frontier.is_empty() {
                    let found = crate::daecore::daetype::subsetter::seac_component_gids(
                        charstrings, &frontier, cff, *charset_off, None,
                    );
                    frontier = found.into_iter().filter(|&c| set.insert(c)).collect();
                }
            }
        },
    )
}

pub fn glyf_closure(ttf: &[u8], requested: &[u16]) -> Result<GlyphSet, String> {
    use crate::daecore::daetype::subsetter::{parse_loca, parse_ttf_dir, slice_table};

    let dir = parse_ttf_dir(ttf);
    let table = |tag: &str| slice_table(ttf, &dir, tag);

    let head = table("head").ok_or("subset: missing head")?;
    let maxp = table("maxp").ok_or("subset: missing maxp")?;
    let loca_fmt = read_i16_be(head, 50).ok_or("subset: head table truncated")?;
    let num_glyphs = read_u16_be(maxp, 4).ok_or("subset: maxp table truncated")? as usize;
    if num_glyphs == 0 {
        return Err("subset: maxp reports zero glyphs".into());
    }

    let outlines = match (table("glyf"), table("loca")) {
        (Some(glyf), Some(loca)) => Some((glyf, parse_loca(loca, loca_fmt, num_glyphs))),
        (None, None) => None,
        (Some(_), None) => return Err("subset: font has glyf but no loca".into()),
        (None, Some(_)) => return Err("subset: font has loca but no glyf".into()),
    };

    closure_loop(table, num_glyphs as u16, requested, |gids, set| match &outlines {
        Some((glyf, loca)) => {
            crate::daecore::daetype::subsetter::active_gids_into(gids, glyf, loca, num_glyphs, set);
        }
        None => set.extend(gids.iter().copied().filter(|&g| (g as usize) < num_glyphs)),
    })
}

fn display_tables(mappings: &[(u32, u16)], family: &str) -> Result<(Vec<u8>, Vec<u8>), String> {
    let cmap = crate::daecore::daetype::subsetter::build_unicode_cmap(mappings);
    if cmap.is_empty() {
        return Err(format!(
            "subset_text: {} distinct codepoints exceed what a format 4 cmap with one segment each can address",
            mappings.len()
        ));
    }
    let name = crate::daecore::daetype::subsetter::build_name_table(family);
    if name.is_empty() {
        return Err("subset_text: family name too long for a uint16-addressed name table".into());
    }
    Ok((cmap, name))
}

fn identity_gid_map_for(active: &GlyphSet) -> Vec<u16> {
    let max_active = active.iter().last().unwrap_or(0);
    (0..=max_active).collect()
}

pub fn subset_cff_flavored(
    source: &BTreeMap<String, TableBytes>,
    cff: &[u8],
    gids: &[u16],
    display: Option<(&[(u32, u16)], &str)>,
) -> Result<SubsetResult, String> {
    let colr = source.get("COLR").map(|t| t.as_slice());
    let active = cff_color_closure(cff, source, gids)?;
    let closed: Vec<u16> = active.iter().collect();
    const RENUMBER_SAFE: &[&str] = &[
        "CFF ", "GDEF", "GSUB", "GPOS", "MATH", "JSTF", "VORG", "kern", "hdmx", "LTSH",
        "hmtx", "hhea", "vmtx", "vhea", "maxp", "cmap", "name", "post",
        "CBDT", "CBLC", "EBDT", "EBLC", "sbix", "prop", "kerx", "just", "morx", "lcar", "opbd", "ankr", "bsln", "fmtx", "bdat", "bloc", "Zapf", "EBSC", "xref",
        "feat", "trak", "fdsc", "ltag",
        "head", "OS/2", "gasp", "DSIG", "meta", "PCLT", "VDMX", "MERG", "hsty", "cvt ", "fpgm", "prep",
    ];

    let recognized = |t: &str| {
        RENUMBER_SAFE.contains(&t)
            || STRIP_FOR_CFF_SUBSET.contains(&t)
            || (t == "BASE" && source.get("BASE").is_some_and(|b| crate::daecore::daetype::base::base_is_glyph_free(b)))
    };
    let want_compact = display.is_some() && source.keys().all(|t| recognized(t));
    let cff_result = if want_compact {
        crate::daecore::daetype::subsetter::subset_cff_compacting(cff, &closed)?
    } else {
        crate::daecore::daetype::subsetter::subset_cff(cff, &closed)?
    };
    let compacted = !cff_result.gid_map.is_empty();
    let gid_map = if compacted { cff_result.gid_map.clone() } else { identity_gid_map_for(&active) };

    let mut out_map: BTreeMap<String, TableBytes> = BTreeMap::new();
    for (tag, data) in source {
        if !STRIP_FOR_CFF_SUBSET.contains(&tag.as_str()) {
            out_map.insert(tag.clone(), data.clone());
        }
    }

    let orig_gdef = source.get("GDEF").map(|t| t.as_slice());
    let mark_filter_sets_survive = orig_gdef.is_some_and(crate::daecore::daetype::subsetter::has_mark_glyph_sets);
    for (tag, rebuilt) in [
        ("GDEF", orig_gdef.and_then(|g| crate::daecore::daetype::subsetter::subset_gdef(g, &active, &gid_map))),
        ("GSUB", source.get("GSUB").and_then(|g| crate::daecore::daetype::subsetter::subset_gsub(g, &active, &gid_map, mark_filter_sets_survive))),
        ("GPOS", source.get("GPOS").and_then(|g| crate::daecore::daetype::subsetter::subset_gpos(g, &active, &gid_map, mark_filter_sets_survive))),
    ] {
        if !source.contains_key(tag) { continue; }
        match rebuilt {
            Some(bytes) => { out_map.insert(tag.to_string(), (bytes).into()); }
            None => { out_map.remove(tag); }
        }
    }
    if let Some(post) = out_map.remove("post") {
        out_map.insert("post".to_string(), crate::daecore::daetype::subsetter::fix_post_table(post.to_owned_vec()).into());
    }
    out_map.insert("CFF ".to_string(), (cff_result.ttf).into());

    if compacted {
        let n_active = closed.len();
        for (mtx_tag, hea_tag) in [("hmtx", "hhea"), ("vmtx", "vhea")] {
            let (Some(mtx), Some(hea)) = (source.get(mtx_tag), source.get(hea_tag)) else { continue };
            if hea.len() < 36 { continue; }
            let num_long = read_u16_be(hea, 34).unwrap_or(0) as usize;
            let mut new_hea = hea.to_owned_vec();
            write_u16_be(&mut new_hea, 34, n_active as u16);
            out_map.insert(mtx_tag.to_string(), (crate::daecore::daetype::subsetter::rebuild_metrics(mtx, num_long, &closed)).into());
            out_map.insert(hea_tag.to_string(), (new_hea).into());
        }
        if let Some(hdmx) = source.get("hdmx") {
            out_map.remove("hdmx");
            if let Some(new_hdmx) = crate::daecore::daetype::subsetter::subset_hdmx(hdmx, n_source(source) as usize, &closed) {
                out_map.insert("hdmx".to_string(), (new_hdmx).into());
            }
        }
        if let Some(maxp) = out_map.get("maxp").filter(|m| m.len() >= 6) {
            let mut patched = maxp.to_owned_vec();
            write_u16_be(&mut patched, 4, n_active as u16);
            out_map.insert("maxp".to_string(), patched.into());
        }
        for (tag, rebuilt) in [
            ("VORG", source.get("VORG").and_then(|v| crate::daecore::daetype::subsetter::remap_vorg(v, &gid_map, &active))),
            ("kern", source.get("kern").and_then(|k| crate::daecore::daetype::subsetter::remap_kern(k, &gid_map, &active))),
            ("MATH", source.get("MATH").and_then(|m| crate::daecore::daetype::subsetter::subset_math(m, &active, &gid_map))),
            ("JSTF", source.get("JSTF").and_then(|j| crate::daecore::daetype::subsetter::subset_jstf(j, &active, &gid_map))),
            ("lcar", source.get("lcar").and_then(|d| crate::daecore::daetype::subsetter::subset_lcar(d, &active, &gid_map, n_source(source)))),
            ("opbd", source.get("opbd").and_then(|d| crate::daecore::daetype::subsetter::subset_opbd(d, &active, &gid_map, n_source(source)))),
            ("ankr", source.get("ankr").and_then(|d| crate::daecore::daetype::subsetter::subset_ankr(d, &active, &gid_map, n_source(source)))),
            ("bsln", source.get("bsln").and_then(|d| crate::daecore::daetype::subsetter::subset_bsln(d, &active, &gid_map, n_source(source)))),
            ("fmtx", source.get("fmtx").and_then(|d| crate::daecore::daetype::subsetter::subset_fmtx(d, &active, &gid_map))),
            ("EBSC", source.get("EBSC").and_then(|e| {
                let surviving = source.get("EBLC").map(|b| crate::daecore::daetype::subsetter::strike_sizes(b)).unwrap_or_default();
                crate::daecore::daetype::subsetter::subset_ebsc(e, &surviving)
            })),
            ("Zapf", source.get("Zapf").and_then(|d| {
                crate::daecore::daetype::subsetter::subset_zapf(d, n_source(source) as usize, &closed, &active, &gid_map)
            })),
            ("morx", source.get("morx").and_then(|d| crate::daecore::daetype::subsetter::subset_morx(d, &active, &gid_map, n_source(source)))),
            ("just", source.get("just").and_then(|d| crate::daecore::daetype::subsetter::subset_just(d, &active, &gid_map, n_source(source)))),
            ("kerx", source.get("kerx").and_then(|d| crate::daecore::daetype::subsetter::subset_kerx(d, &active, &gid_map, n_source(source)))),
            ("prop", source.get("prop").and_then(|d| crate::daecore::daetype::subsetter::subset_prop(d, n_source(source) as usize, &active, &gid_map))),
            ("LTSH", source.get("LTSH").and_then(|l| crate::daecore::daetype::subsetter::subset_ltsh(l, &closed))),
        ] {
            if !source.contains_key(tag) { continue; }
            match rebuilt {
                Some(bytes) => { out_map.insert(tag.to_string(), (bytes).into()); }
                None => { out_map.remove(tag); }
            }
        }
        for (data_tag, loc_tag) in [("CBDT", "CBLC"), ("EBDT", "EBLC"), ("bdat", "bloc")] {
            let (Some(data), Some(loc)) = (source.get(data_tag), source.get(loc_tag)) else { continue };
            out_map.remove(data_tag);
            out_map.remove(loc_tag);
            if let Some((l, d)) = crate::daecore::daetype::subsetter::subset_bitmap_strikes(loc, data, &active, &gid_map) {
                out_map.insert(loc_tag.to_string(), (l).into());
                out_map.insert(data_tag.to_string(), (d).into());
            }
        }
        if let Some(x) = source.get("xref") {
            let kerx_stable = match (source.get("kerx"), out_map.get("kerx")) {
                (Some(before), Some(after)) => read_u32_be(before, 4) == read_u32_be(after, 4),
                (None, None) => true,
                _ => false,
            };
            out_map.remove("xref");
            if let Some(new_xref) = crate::daecore::daetype::subsetter::subset_xref(x, |tag| tag != b"kerx" || kerx_stable) {
                out_map.insert("xref".to_string(), (new_xref).into());
            }
        }
        if let Some(sbix) = source.get("sbix") {
            out_map.remove("sbix");
            if let Some(new_sbix) = crate::daecore::daetype::subsetter::subset_sbix(sbix, n_source(source) as usize, &closed, &gid_map) {
                out_map.insert("sbix".to_string(), (new_sbix).into());
            }
        }
    }
    if let Some(colr) = colr
        && let Some(new_colr) = crate::daecore::daetype::subsetter::colr::subset_colr(colr, &active, &gid_map) {
            out_map.insert("COLR".to_string(), (new_colr).into());
            if let Some(cpal) = source.get("CPAL") {
                out_map.insert("CPAL".to_string(), cpal.clone());
            }
        }
    if let Some((mappings, family)) = display {
        let remapped: Vec<(u32, u16)>;
        let mappings = if compacted {
            remapped = mappings.iter()
                .map(|&(cp, g)| (cp, gid_map.get(g as usize).copied().unwrap_or(0)))
                .collect();
            &remapped[..]
        } else {
            mappings
        };
        let (cmap, name) = display_tables(mappings, family)?;
        out_map.insert("cmap".to_string(), (cmap).into());
        out_map.insert("name".to_string(), (name).into());
    }

    Ok(SubsetResult {
        ttf: crate::daecore::daetype::decoder::build_ttf(&out_map),
        gid_map: if compacted { gid_map } else { vec![] },
    })
}

impl FontCache {
    pub fn subset_font_rs(&self, axis_values: &[(String, f64)], gids: &[u16]) -> Result<SubsetResult, String> {
        if self.table_map.contains_key("CFF2") {
            let instanced_bytes = self.get_or_instance(axis_values);
            let instanced_map = crate::daecore::daetype::decoder::extract_ttf_tables(&instanced_bytes)?;
            let cff = instanced_map.get("CFF ").ok_or("missing CFF")?.clone();
            return subset_cff_flavored(&instanced_map, &cff, gids, None);
        }
        if self.table_map.contains_key("CFF ") {
            let cff = self.table_map.get("CFF ").ok_or("missing CFF")?.clone();
            return subset_cff_flavored(&self.table_map, &cff, gids, None);
        }
        let ttf = self.get_or_instance(axis_values);
        crate::daecore::daetype::subsetter::subset_ttf(&ttf, gids)
    }

    pub fn glyph_closure_rs(
        &self,
        axis_values: &[(String, f64)],
        gids: &[u16],
    ) -> Result<Vec<u16>, String> {
        let bounded = |num_glyphs: u16, set: GlyphSet| -> Vec<u16> {
            set.iter().filter(|&g| g < num_glyphs).collect()
        };
        if self.table_map.contains_key("CFF2") {
            let instanced = self.get_or_instance(axis_values);
            let instanced_map = crate::daecore::daetype::decoder::extract_ttf_tables(&instanced)?;
            let cff = instanced_map.get("CFF ").ok_or("missing CFF")?.clone();
            let closure = cff_color_closure(&cff, &instanced_map, gids)?;
            return Ok(bounded(n_source(&instanced_map), closure));
        }
        if let Some(cff) = self.table_map.get("CFF ") {
            let closure = cff_color_closure(cff, &self.table_map, gids)?;
            return Ok(bounded(n_source(&self.table_map), closure));
        }
        let ttf = self.get_or_instance(axis_values);
        Ok(bounded(n_source(&self.table_map), glyf_closure(&ttf, gids)?))
    }

    pub fn subset_text_rs(&self, axis_values: &[(String, f64)], text: &str) -> Result<SubsetResult, String> {
        let instanced = self.get_or_instance(axis_values);
        let instanced_map = crate::daecore::daetype::decoder::extract_ttf_tables(&instanced)?;
        let cmap = instanced_map.get("cmap").ok_or("subset_text: font has no cmap")?;

        let mut seen: alloc::collections::BTreeMap<u32, u16> = alloc::collections::BTreeMap::new();
        for ch in text.chars() {
            let cp = ch as u32;
            if seen.contains_key(&cp) { continue; }
            if let Some(gid) = crate::daecore::daetype::subsetter::cmap_glyph_id(cmap, cp) { seen.insert(cp, gid); }
        }
        let gids: Vec<u16> = seen.values().copied().collect();
        let family = crate::daecore::daetype::decoder::read_font_family_name(&instanced_map).unwrap_or_else(|| "DaegunSubset".to_string());

        if !instanced_map.contains_key("CFF2") && !instanced_map.contains_key("CFF ") {
            let result = crate::daecore::daetype::subsetter::subset_ttf(&instanced, &gids)?;
            let mappings: Vec<(u32, u16)> = seen.iter()
                .filter_map(|(&cp, &orig_gid)| Some((cp, *result.gid_map.get(orig_gid as usize)?)))
                .collect();
            let mut out_map = crate::daecore::daetype::decoder::extract_ttf_tables(&result.ttf)?;
            let (cmap, name) = display_tables(&mappings, &family)?;
            out_map.insert("cmap".to_string(), (cmap).into());
            out_map.insert("name".to_string(), (name).into());
            return Ok(SubsetResult { ttf: crate::daecore::daetype::decoder::build_ttf(&out_map), gid_map: result.gid_map });
        }

        let cff = if instanced_map.contains_key("CFF2") {
            let dir = crate::daecore::daetype::subsetter::parse_ttf_dir(&instanced);
            crate::daecore::daetype::subsetter::slice_table(&instanced, &dir, "CFF ")
                .ok_or("CFF2 instancing produced no CFF table")?.to_vec()
        } else {
            instanced_map.get("CFF ").ok_or("subset_text: font has neither CFF2 nor CFF")?.to_owned_vec()
        };
        let mappings: Vec<(u32, u16)> = seen.into_iter().collect();
        subset_cff_flavored(&instanced_map, &cff, &gids, Some((&mappings, &family)))
    }
}
