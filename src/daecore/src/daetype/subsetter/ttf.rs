use super::*;

pub fn subset_ttf(ttf: &[u8], requested: &[u16]) -> Result<SubsetResult, String> {
    subset_ttf_with(ttf, requested, None)
}

// The cmap holds `text`'s mappings and sequences, in the source's glyph ids, where given, and
// otherwise every mapping of the source's whose glyph the subset keeps.
pub(crate) fn subset_ttf_with(
    ttf: &[u8], requested: &[u16], text: Option<TextCmap>,
) -> Result<SubsetResult, String> {
    let dir = parse_ttf_dir(ttf);
    // A subset carries no variations, so a variable font is subset at its default instance, with the
    // variation data its other tables name resolved rather than left naming an fvar that is gone.
    if slice_table(ttf, &dir, "fvar").is_some() {
        let instanced = super::super::instancer::instance_font_from_map(&super::super::decoder::extract_ttf_tables(ttf)?, &[])?;
        if slice_table(&instanced, &parse_ttf_dir(&instanced), "fvar").is_some() {
            return Err("subset: the default instance kept its variations".into());
        }
        return subset_ttf_with(&instanced, requested, text);
    }
    let head = slice_table(ttf, &dir, "head").ok_or("subset: missing head")?;
    let (num_glyphs, outlines) = closure::glyf_outlines(ttf, &dir)?;
    let active = closure::glyf_closure_of(ttf, &dir, num_glyphs, &outlines, requested)?;
    let orig_colr = slice_table(ttf, &dir, "COLR");
    let orig_math = slice_table(ttf, &dir, "MATH");
    let orig_morx = slice_table(ttf, &dir, "morx");

    let active_sorted: Vec<u16> = active.iter().collect();
    let n_active = active_sorted.len();

    let max_orig = *active_sorted.last().unwrap_or(&0) as usize;
    let mut gid_map = vec![0u16; max_orig + 1];
    for (compact, &orig) in active_sorted.iter().enumerate() {
        gid_map[orig as usize] = compact as u16;
    }

    let mut new_glyf: Vec<u8> = Vec::new();
    let mut new_loca = vec![0u32; n_active + 1];
    // Glyph blocks run in glyph order, so a font copies at most its glyf; loca ranges that overlap copy
    // the same bytes again and stop at the subset budget.
    let budget = subset_budget(outlines.as_ref().map_or(0, |(glyf, _)| glyf.len()));
    let mut copied = 0usize;
    let offset = |len: usize| u32::try_from(len).map_err(|_| String::from("subset: glyf past 4 GB"));

    for (compact, &orig_gid) in active_sorted.iter().enumerate() {
        let Some((glyf, loca_offs)) = outlines.as_ref() else { break };
        new_loca[compact] = offset(new_glyf.len())?;
        let (s, e) = (loca_offs[orig_gid as usize], loca_offs[orig_gid as usize + 1]);
        if s < e && e <= glyf.len() {
            copied += e - s;
            if copied > budget {
                return Err("subset: glyf ranges past the subset budget".into());
            }
            let glyph_start = new_glyf.len();
            new_glyf.extend_from_slice(&glyf[s..e]);
            if is_composite(&new_glyf, glyph_start) {
                let glyph_end = new_glyf.len();
                patch_compound_gids(&mut new_glyf, glyph_start, glyph_end, &gid_map);
            }
            new_glyf.resize(new_glyf.len().next_multiple_of(4), 0);
        }
    }
    new_loca[n_active] = offset(new_glyf.len())?;

    let use_short_loca = new_glyf.len() <= 0x1_FFFE;
    let new_loca_bytes = if use_short_loca {
        let mut b = vec![0u8; (n_active + 1) * 2];
        for (i, &off) in new_loca.iter().enumerate() {
            write_u16_be(&mut b, i * 2, (off / 2) as u16);
        }
        b
    } else {
        let mut b = vec![0u8; (n_active + 1) * 4];
        for (i, &off) in new_loca.iter().enumerate() {
            write_u32_be(&mut b, i * 4, off);
        }
        b
    };

    let mut new_head = head.to_vec();
    write_i16_be(&mut new_head, 50, if use_short_loca { 0 } else { 1 });

    // Metrics are rebuilt only where the source has them and a header counting them.
    let metrics = |mtx_tag: &str, hea_tag: &str| match (slice_table(ttf, &dir, mtx_tag), owned_table(ttf, &dir, hea_tag)) {
        (Some(mtx), Some(mut hea)) if hea.len() >= 36 => {
            let num_long = read_u16_be(&hea, 34).unwrap_or(0) as usize;
            let m = rebuild_metrics(mtx, num_long, &active_sorted);
            write_u16_be(&mut hea, 34, n_active as u16);
            Some((m, hea))
        }
        _ => None,
    };
    let horizontal = metrics("hmtx", "hhea");
    let vertical = metrics("vmtx", "vhea");

    let new_maxp = {
        let mut m = owned_table(ttf, &dir, "maxp").unwrap_or_default();
        if m.len() >= 6 { write_u16_be(&mut m, 4, n_active as u16); }
        m
    };

    let mut tmap: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for &tag in GLYPH_FREE_TABLES {
        if let Some(d) = owned_table(ttf, &dir, tag) { tmap.insert(tag.to_string(), d); }
    }
    if let Some(base) = slice_table(ttf, &dir, "BASE").filter(|b| super::super::base::base_is_glyph_free(b)) {
        tmap.insert("BASE".to_string(), base.to_vec());
    }
    if let Some(post) = owned_table(ttf, &dir, "post") {
        tmap.insert("post".to_string(), fix_post_table(post));
    }
    tmap.insert("maxp".to_string(), new_maxp);
    if let Some((new_hmtx, new_hhea)) = horizontal {
        tmap.insert("hmtx".to_string(), new_hmtx);
        tmap.insert("hhea".to_string(), new_hhea);
    }
    tmap.insert("head".to_string(), new_head);
    if outlines.is_some() {
        tmap.insert("glyf".to_string(), new_glyf);
        tmap.insert("loca".to_string(), new_loca_bytes);
    }
    for (data_tag, loc_tag) in [("CBDT", "CBLC"), ("EBDT", "EBLC")] {
        let (Some(data), Some(loc)) = (slice_table(ttf, &dir, data_tag), slice_table(ttf, &dir, loc_tag))
        else { continue };
        if let Some((new_loc, new_data)) = bitmap::subset_bitmap_strikes(loc, data, &active, &gid_map) {
            tmap.insert(loc_tag.to_string(), new_loc);
            tmap.insert(data_tag.to_string(), new_data);
        }
    }
    if let Some(sbix) = slice_table(ttf, &dir, "sbix")
        && let Some(new_sbix) = bitmap::subset_sbix(sbix, num_glyphs, &active_sorted, &gid_map) {
            tmap.insert("sbix".to_string(), new_sbix);
        }
    if let Some((new_vmtx, new_vhea)) = vertical {
        tmap.insert("vmtx".to_string(), new_vmtx);
        tmap.insert("vhea".to_string(), new_vhea);
    }
    if let Some(vorg) = slice_table(ttf, &dir, "VORG")
        && let Some(remapped) = remap_vorg(vorg, &gid_map, &active) {
            tmap.insert("VORG".to_string(), remapped);
        }
    let new_gdef = slice_table(ttf, &dir, "GDEF").and_then(|gdef| otl::gdef::subset_gdef(gdef, &active, &gid_map));
    let mark_sets = new_gdef.as_deref().map_or(0, otl::gdef::mark_glyph_set_count);
    if let Some(new_gdef) = new_gdef {
        tmap.insert("GDEF".to_string(), new_gdef);
    }
    if let Some(gsub) = slice_table(ttf, &dir, "GSUB")
        && let Some(new_gsub) = otl::gsub::subset_gsub(gsub, &active, &gid_map, mark_sets) {
            tmap.insert("GSUB".to_string(), new_gsub);
        }
    if let Some(gpos) = slice_table(ttf, &dir, "GPOS")
        && let Some(new_gpos) = otl::gpos::subset_gpos(gpos, &active, &gid_map, mark_sets) {
            tmap.insert("GPOS".to_string(), new_gpos);
        }
    if let Some(m) = orig_math
        && let Some(new_math) = math::subset_math(m, &active, &gid_map) {
            tmap.insert("MATH".to_string(), new_math);
        }
    if let Some(j) = slice_table(ttf, &dir, "JSTF")
        && let Some(new_jstf) = jstf::subset_jstf(j, &active, &gid_map) {
            tmap.insert("JSTF".to_string(), new_jstf);
        }
    if let Some(h) = slice_table(ttf, &dir, "hdmx")
        && let Some(new_hdmx) = device_metrics::subset_hdmx(h, num_glyphs, &active_sorted) {
            tmap.insert("hdmx".to_string(), new_hdmx);
        }
    if let Some(l) = slice_table(ttf, &dir, "LTSH")
        && let Some(new_ltsh) = device_metrics::subset_ltsh(l, &active_sorted) {
            tmap.insert("LTSH".to_string(), new_ltsh);
        }
    if let Some(p) = slice_table(ttf, &dir, "prop")
        && let Some(new_prop) = aat::prop::subset_prop(p, num_glyphs, &active, &gid_map) {
            tmap.insert("prop".to_string(), new_prop);
        }
    for (tag, rebuilt) in [
        ("lcar", slice_table(ttf, &dir, "lcar").and_then(|d| aat::simple::subset_lcar(d, &active, &gid_map, num_glyphs as u16))),
        ("opbd", slice_table(ttf, &dir, "opbd").and_then(|d| aat::simple::subset_opbd(d, &active, &gid_map, num_glyphs as u16))),
        ("ankr", slice_table(ttf, &dir, "ankr").and_then(|d| aat::simple::subset_ankr(d, &active, &gid_map, num_glyphs as u16))),
        ("bsln", slice_table(ttf, &dir, "bsln").and_then(|d| aat::simple::subset_bsln(d, &active, &gid_map, num_glyphs as u16))),
        ("fmtx", slice_table(ttf, &dir, "fmtx").and_then(|d| aat::simple::subset_fmtx(d, &active, &gid_map))),
        ("Zapf", slice_table(ttf, &dir, "Zapf").and_then(|d| aat::zapf::subset_zapf(d, num_glyphs, &active_sorted, &active, &gid_map))),
    ] {
        if let Some(bytes) = rebuilt { tmap.insert(tag.to_string(), bytes); }
    }
    if let (Some(data), Some(loc)) = (slice_table(ttf, &dir, "bdat"), slice_table(ttf, &dir, "bloc"))
        && let Some((new_loc, new_data)) = bitmap::subset_bitmap_strikes(loc, data, &active, &gid_map) {
            tmap.insert("bloc".to_string(), new_loc);
            tmap.insert("bdat".to_string(), new_data);
        }
    if let Some(e) = slice_table(ttf, &dir, "EBSC") {
        let surviving = tmap.get("EBLC").map(|b| aat::descriptive::strike_sizes(b)).unwrap_or_default();
        if let Some(new_ebsc) = aat::descriptive::subset_ebsc(e, &surviving) {
            tmap.insert("EBSC".to_string(), new_ebsc);
        }
    }
    if let Some(m) = orig_morx
        && let Some(new_morx) = aat::morx::subset_morx(m, &active, &gid_map, num_glyphs as u16) {
            tmap.insert("morx".to_string(), new_morx);
        }
    if let Some(j) = slice_table(ttf, &dir, "just")
        && let Some(new_just) = aat::just::subset_just(j, &active, &gid_map, num_glyphs as u16) {
            tmap.insert("just".to_string(), new_just);
        }
    if let Some(k) = slice_table(ttf, &dir, "kerx")
        && let Some(new_kerx) = aat::kerx::subset_kerx(k, &active, &gid_map, num_glyphs as u16) {
            tmap.insert("kerx".to_string(), new_kerx);
        }
    if let Some(x) = slice_table(ttf, &dir, "xref") {
        let stable = |tag: &[u8]| {
            let name = core::str::from_utf8(tag).unwrap_or_default();
            aat::descriptive::subtable_counts(tag, slice_table(ttf, &dir, name))
                == aat::descriptive::subtable_counts(tag, tmap.get(name).map(Vec::as_slice))
        };
        if let Some(new_xref) = aat::descriptive::subset_xref(x, stable) {
            tmap.insert("xref".to_string(), new_xref);
        }
    }
    if let Some(kern) = slice_table(ttf, &dir, "kern")
        && let Some(remapped) = remap_kern(kern, &gid_map, &active) {
            tmap.insert("kern".to_string(), remapped);
        }
    if let Some(colr) = orig_colr
        && let Some(new_colr) = colr::subset_colr(colr, &active, &gid_map) {
            tmap.insert("COLR".to_string(), new_colr);
            if let Some(cpal) = owned_table(ttf, &dir, "CPAL") {
                tmap.insert("CPAL".to_string(), cpal);
            }
        }

    let new_gid = |g: u16| if active.contains(&g) { gid_map.get(usize::from(g)).copied() } else { None };
    let source_cmap = slice_table(ttf, &dir, "cmap");
    let (mappings, sequences) = match text {
        Some(text) => remap_text(text, new_gid),
        None => source_cmap.map(|c| kept_cmap(c, new_gid)).unwrap_or_default(),
    };
    let os2 = slice_table(ttf, &dir, "OS/2");
    let (cmap, name, new_os2) = display_tables(source_cmap, &mappings, &sequences, slice_table(ttf, &dir, "name"), os2, |tag| tmap.get(tag).map(Vec::as_slice));
    tmap.insert("cmap".to_string(), cmap);
    tmap.insert("name".to_string(), name);
    if let Some(os2) = new_os2.or_else(|| os2.map(<[u8]>::to_vec)) { tmap.insert("OS/2".to_string(), os2); }

    Ok(SubsetResult { ttf: build_ttf(&tmap), gid_map })
}
