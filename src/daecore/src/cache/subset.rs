use crate::daecore::daetype::subsetter::GlyphSet;
use super::*;
use crate::daecore::daetype::TableBytes;

// Tables a CFF subset leaves out or rebuilds: variations, color, and DSIG, which signs the source's bytes.
const STRIP_FOR_CFF_SUBSET: &[&str] = &[
    "fvar", "gvar", "avar", "cvar", "HVAR", "VVAR", "MVAR", "STAT", "CFF2", "CFF ", "COLR", "CPAL", "DSIG",
];

fn n_source(source: &BTreeMap<String, TableBytes>) -> u16 {
    source.get("maxp").and_then(|m| read_u16_be(m, 4)).unwrap_or(0)
}

pub fn cff_color_closure(
    cff: &[u8], outlines: Option<&crate::daecore::daetype::outline::CffOutlines>, source: &BTreeMap<String, TableBytes>, requested: &[u16],
) -> Result<GlyphSet, String> {
    crate::daecore::daetype::subsetter::closure_loop(
        |tag| source.get(tag).map(|t| t.as_slice()),
        n_source(source),
        requested,
        |gids, set| {
            let frontier: Vec<u16> = gids.iter().copied().filter(|&g| set.insert(g)).collect();
            if let Some(outlines) = outlines {
                crate::daecore::daetype::subsetter::close_over_seacs(outlines, cff, frontier, set);
            }
        },
    )
}

fn identity_gid_map_for(active: &GlyphSet) -> Vec<u16> {
    let max_active = active.iter().last().unwrap_or(0);
    (0..=max_active).collect()
}

// `text` is the text's mappings and sequences in source glyph ids, which the subset's cmap holds in
// place of the source's; a subset of them may also be compacted.
pub fn subset_cff_flavored(
    source: &BTreeMap<String, TableBytes>,
    cff: &[u8],
    gids: &[u16],
    text: Option<crate::daecore::daetype::subsetter::TextCmap>,
) -> Result<SubsetResult, String> {
    let colr = source.get("COLR").map(|t| t.as_slice());
    let outlines = crate::daecore::daetype::outline::CffOutlines::parse(cff).ok();
    let active = cff_color_closure(cff, outlines.as_ref(), source, gids)?;
    let closed: Vec<u16> = active.iter().collect();
    const RENUMBER_SAFE: &[&str] = &[
        "CFF ", "GDEF", "GSUB", "GPOS", "MATH", "JSTF", "VORG", "kern", "hdmx", "LTSH",
        "hmtx", "hhea", "vmtx", "vhea", "maxp", "cmap", "name", "post",
        "CBDT", "CBLC", "EBDT", "EBLC", "sbix", "prop", "kerx", "just", "morx", "lcar", "opbd", "ankr", "bsln", "fmtx", "bdat", "bloc", "Zapf", "EBSC", "xref",
        "feat", "trak", "fdsc", "ltag",
        "head", "OS/2", "MERG",
    ];

    let recognized = |t: &str| {
        RENUMBER_SAFE.contains(&t)
            || crate::daecore::daetype::subsetter::GLYPH_FREE_TABLES.contains(&t)
            || STRIP_FOR_CFF_SUBSET.contains(&t)
            || (t == "BASE" && source.get("BASE").is_some_and(|b| crate::daecore::daetype::base::base_is_glyph_free(b)))
    };
    let want_compact = text.is_some() && source.keys().all(|t| recognized(t));
    let cff_result = crate::daecore::daetype::subsetter::subset_cff_closed(cff, outlines.as_ref(), &closed, want_compact)?;
    let compacted = !cff_result.gid_map.is_empty();
    // The closure keeps a gid past the CFF, as a cmap or GSUB entry can name; the subset holds none.
    let limit = if compacted { cff_result.gid_map.len() } else { usize::from(n_source(source)) };
    let closed: Vec<u16> = closed.into_iter().filter(|&g| usize::from(g) < limit).collect();
    let active: GlyphSet = closed.iter().copied().collect();
    let gid_map = if compacted { cff_result.gid_map.clone() } else { identity_gid_map_for(&active) };

    let mut out_map: BTreeMap<String, TableBytes> = BTreeMap::new();
    for (tag, data) in source {
        if !STRIP_FOR_CFF_SUBSET.contains(&tag.as_str()) {
            out_map.insert(tag.clone(), data.clone());
        }
    }

    use crate::daecore::daetype::subsetter::{mark_glyph_set_count, subset_gdef, subset_gpos, subset_gsub};
    let new_gdef = source.get("GDEF").and_then(|g| subset_gdef(g, &active, &gid_map));
    let mark_sets = new_gdef.as_deref().map_or(0, mark_glyph_set_count);
    for (tag, rebuilt) in [
        ("GSUB", source.get("GSUB").and_then(|g| subset_gsub(g, &active, &gid_map, mark_sets))),
        ("GPOS", source.get("GPOS").and_then(|g| subset_gpos(g, &active, &gid_map, mark_sets))),
        ("GDEF", new_gdef),
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
        // MERG's class definitions name glyph ids; without it, glyphs are always merged.
        out_map.remove("MERG");
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
        if let Some(e) = source.get("EBSC") {
            out_map.remove("EBSC");
            let surviving = out_map.get("EBLC").map(|b| crate::daecore::daetype::subsetter::strike_sizes(b)).unwrap_or_default();
            if let Some(new_ebsc) = crate::daecore::daetype::subsetter::subset_ebsc(e, &surviving) {
                out_map.insert("EBSC".to_string(), new_ebsc.into());
            }
        }
        if let Some(x) = source.get("xref") {
            use crate::daecore::daetype::subsetter::subtable_counts;
            let stable = |tag: &[u8]| {
                let name = core::str::from_utf8(tag).unwrap_or_default();
                subtable_counts(tag, source.get(name).map(|t| t.as_slice()))
                    == subtable_counts(tag, out_map.get(name).map(|t| t.as_slice()))
            };
            let new_xref = crate::daecore::daetype::subsetter::subset_xref(x, stable);
            out_map.remove("xref");
            if let Some(new_xref) = new_xref {
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
    use crate::daecore::daetype::subsetter::{display_tables, kept_cmap, remap_text};
    let new_gid = |g: u16| if active.contains(&g) { gid_map.get(usize::from(g)).copied() } else { None };
    let source_cmap = source.get("cmap").map(|t| t.as_slice());
    let (mappings, sequences) = match text {
        Some(text) => remap_text(text, new_gid),
        None => source_cmap.map(|c| kept_cmap(c, new_gid)).unwrap_or_default(),
    };
    let os2 = source.get("OS/2").map(|t| t.as_slice());
    let (cmap, name, new_os2) = display_tables(
        source_cmap, &mappings, &sequences, source.get("name").map(|t| t.as_slice()), os2, |tag| out_map.get(tag).map(|t| t.as_slice()),
    );
    out_map.insert("cmap".to_string(), cmap.into());
    out_map.insert("name".to_string(), name.into());
    if let Some(os2) = new_os2 { out_map.insert("OS/2".to_string(), os2.into()); }

    Ok(SubsetResult {
        ttf: crate::daecore::daetype::decoder::build_ttf(&out_map),
        gid_map: if compacted { gid_map } else { vec![] },
    })
}

impl FontCache {
    pub fn subset_font_rs(&self, axis_values: &[(String, f64)], gids: &[u16]) -> Result<SubsetResult, String> {
        if self.table_map.contains_key("CFF2") {
            let instanced_map = crate::daecore::daetype::decoder::extract_ttf_tables_shared(self.get_or_instance(axis_values))?;
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
            let instanced_map = crate::daecore::daetype::decoder::extract_ttf_tables_shared(self.get_or_instance(axis_values))?;
            let cff = instanced_map.get("CFF ").ok_or("missing CFF")?.clone();
            let closure = cff_color_closure(&cff, crate::daecore::daetype::outline::CffOutlines::parse(&cff).ok().as_ref(), &instanced_map, gids)?;
            return Ok(bounded(n_source(&instanced_map), closure));
        }
        if let Some(cff) = self.table_map.get("CFF ") {
            let closure = cff_color_closure(cff, self.cff_outlines().as_deref(), &self.table_map, gids)?;
            return Ok(bounded(n_source(&self.table_map), closure));
        }
        let ttf = self.get_or_instance(axis_values);
        Ok(bounded(n_source(&self.table_map), crate::daecore::daetype::subsetter::glyf_closure(&ttf, gids)?))
    }

    pub fn subset_text_rs(&self, axis_values: &[(String, f64)], text: &str) -> Result<SubsetResult, String> {
        use crate::daecore::daetype::subsetter::{cmap_variation_glyph_id, is_variation_selector, CmapLookup, UvsLookup};
        let instanced = self.get_or_instance(axis_values);
        let instanced_map = crate::daecore::daetype::decoder::extract_ttf_tables_shared(Shared::clone(&instanced))?;
        let cmap = instanced_map.get("cmap").ok_or("subset_text: font has no cmap")?;
        let lookup = CmapLookup::new(cmap);

        // Each character's glyph, and each base and selector's: its own glyph joins the subset, and
        // the sequence its format 14 entry.
        let mut seen: alloc::collections::BTreeMap<u32, u16> = alloc::collections::BTreeMap::new();
        let mut sequences: Vec<crate::daecore::daetype::subsetter::Sequence> = Vec::new();
        let mut chars = text.chars().map(u32::from).peekable();
        while let Some(cp) = chars.next() {
            if let alloc::collections::btree_map::Entry::Vacant(slot) = seen.entry(cp)
                && let Some(gid) = lookup.glyph_id(cmap, cp) {
                    slot.insert(gid);
                }
            let Some(&vs) = chars.peek().filter(|&&vs| is_variation_selector(vs) && !is_variation_selector(cp)) else { continue };
            match cmap_variation_glyph_id(cmap, cp, vs) {
                Some(UvsLookup::Explicit(gid)) => sequences.push((vs, cp, Some(gid))),
                Some(UvsLookup::UseDefault) => sequences.push((vs, cp, None)),
                None => {}
            }
        }
        let gids: Vec<u16> = seen.values().copied().chain(sequences.iter().filter_map(|s| s.2)).collect();
        let mappings: Vec<(u32, u16)> = seen.into_iter().collect();
        let text_cmap = crate::daecore::daetype::subsetter::TextCmap { mappings: &mappings, sequences: &sequences };

        if !instanced_map.contains_key("CFF2") && !instanced_map.contains_key("CFF ") {
            return crate::daecore::daetype::subsetter::subset_ttf_with(&instanced, &gids, Some(text_cmap));
        }
        let cff = instanced_map.get("CFF ").cloned().ok_or(if instanced_map.contains_key("CFF2") {
            "CFF2 instancing produced no CFF table"
        } else {
            "subset_text: font has neither CFF2 nor CFF"
        })?;
        subset_cff_flavored(&instanced_map, &cff, &gids, Some(text_cmap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A gid past the CFF, as a cmap or GSUB entry can name, survives the closure but not the compacting
    // CFF subset; maxp and the metrics must count only the glyphs the new CFF holds.
    #[test]
    fn a_compacted_cff_subset_counts_only_the_glyphs_it_keeps() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/stix-two-math/STIX2Math.otf");
        let bytes = std::fs::read(path).expect("STIX is a fixture");
        let source = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("STIX parses");
        let cff = source.get("CFF ").expect("STIX is CFF").to_owned_vec();
        let out = subset_cff_flavored(&source, &cff, &[36, 6_000], Some(crate::daecore::daetype::subsetter::TextCmap { mappings: &[(65, 36)], sequences: &[] })).expect("a subset");
        let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&out.ttf).expect("the subset parses");
        let kept = crate::daecore::daetype::outline::CffOutlines::parse(tables.get("CFF ").expect("CFF"))
            .expect("the subset CFF parses").num_glyphs();
        let maxp = read_u16_be(tables.get("maxp").expect("maxp"), 4).expect("numGlyphs");
        assert_eq!(usize::from(maxp), kept, "maxp counts {maxp} glyphs, the CFF holds {kept}");
        let metrics = read_u16_be(tables.get("hhea").expect("hhea"), 34).expect("numberOfHMetrics");
        assert_eq!(usize::from(metrics), kept, "hhea counts {metrics} metrics, the CFF holds {kept} glyphs");
    }

    fn u16s(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    // STIX plus a kern and a VORG naming glyph 6,000, past its glyphs, beside 36 and 37; a DSIG and a
    // MERG; and an EBSC scaling from a strike that holds no glyph, so the subset keeps none of it.
    #[test]
    fn a_compacted_cff_subset_keeps_or_drops_each_glyph_table_as_it_should() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/stix-two-math/STIX2Math.otf");
        let bytes = std::fs::read(path).expect("STIX is a fixture");
        let mut source = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("STIX parses");
        let mut strike = vec![0u8; 48];
        strike[44..46].copy_from_slice(&[12, 12]);
        let mut scale = vec![0u8; 28];
        scale[24..28].copy_from_slice(&[24, 24, 12, 12]);
        for (tag, table) in [
            ("kern", u16s(&[0, 1, 0, 26, 0x0001, 2, 12, 1, 0, 36, 37, 0xFFCE, 36, 6_000, 0xFFC4])),
            ("VORG", u16s(&[1, 0, 880, 2, 36, 900, 6_000, 910])),
            ("DSIG", u16s(&[0, 1, 0, 0])),
            ("MERG", u16s(&[0, 0, 0, 0])),
            ("EBLC", [u16s(&[2, 0, 0, 1]), strike].concat()),
            ("EBDT", u16s(&[2, 0])),
            ("EBSC", [u16s(&[2, 0, 0, 1]), scale].concat()),
        ] {
            source.insert(tag.to_string(), table.into());
        }
        let cff = source.get("CFF ").expect("STIX is CFF").to_owned_vec();
        let text = crate::daecore::daetype::subsetter::TextCmap { mappings: &[(65, 36)], sequences: &[] };
        let out = subset_cff_flavored(&source, &cff, &[36, 37, 6_000], Some(text)).expect("a subset");
        let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&out.ttf).expect("the subset parses");
        let new = |g| out.new_gid(g).expect("kept");
        assert_eq!(tables.get("kern").map(|t| t.to_owned_vec()), Some(u16s(&[0, 1, 0, 20, 0x0001, 1, 6, 0, 0, new(36), new(37), 0xFFCE])));
        assert_eq!(tables.get("VORG").map(|t| t.to_owned_vec()), Some(u16s(&[1, 0, 880, 1, new(36), 900])));
        for tag in ["DSIG", "MERG", "EBLC", "EBSC"] {
            assert!(!tables.contains_key(tag), "{tag} was kept");
        }
    }
}
