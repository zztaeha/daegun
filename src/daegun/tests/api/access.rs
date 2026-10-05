use daegun::Font;

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

const GARAMOND: &str = "eb-garamond/EBGaramond.ttf";
const INTER: &str = "inter/InterVariable.ttf";
const STIX: &str = "stix-two-math/STIX2Math.otf";
const SOURCE_SERIF: &str = "source-serif/SourceSerif4Variable-Roman.otf";
const MORX: &str = "aat/TestMORXOne.ttf";

#[test]
fn tables_come_back_as_stored_bytes() {
    let f = font(GARAMOND);
    let tags = f.table_tags();
    assert!(tags.contains(&"head"), "no head table in {tags:?}");
    assert!(tags.contains(&"cmap"), "no cmap table");
    assert!(tags.len() > 5, "only {} tables, which is not a real font", tags.len());

    let head = f.table("head").expect("head is in table_tags but table() declined it");
    assert_eq!(
        daegun::bytes::read_u32_be(head, 12),
        Some(0x5F0F_3CF5),
        "head does not carry its own magic number, so the slice is wrong"
    );

    assert!(f.has_table("head"));
    assert!(!f.has_table("ZZZZ"), "a tag no font carries reported present");
    assert!(f.table("ZZZZ").is_none());

    for tag in &tags {
        assert!(f.table(tag).is_some(), "{tag} is listed but does not resolve");
    }
}

#[test]
fn the_byte_readers_decline_past_the_end() {
    let data = [0u8, 1, 0, 2];
    assert_eq!(daegun::bytes::read_u16_be(&data, 0), Some(1));
    assert_eq!(daegun::bytes::read_u16_be(&data, 2), Some(2));
    assert_eq!(daegun::bytes::read_u16_be(&data, 3), None, "a read past the end must decline");
    assert_eq!(daegun::bytes::read_u32_be(&data, 1), None);
    assert!(!daegun::bytes::records_fit(0, 3, 2, 4), "3 records of 2 do not fit in 4 bytes");
    assert!(daegun::bytes::records_fit(0, 2, 2, 4));
}

#[test]
fn coverage_agrees_with_glyph_id() {
    let f = font(GARAMOND);
    let cov = f.coverage();
    assert!(cov.len() > 100, "only {} codepoints covered, which is not a text face", cov.len());

    for w in cov.windows(2) {
        assert!(w[0].0 < w[1].0, "coverage is not strictly ascending at {:#x}", w[0].0);
    }

    for &(cp, gid) in cov.iter().step_by(cov.len() / 50 + 1) {
        assert_eq!(
            f.glyph_id(cp),
            Some(gid),
            "coverage maps {cp:#x} to {gid} but glyph_id disagrees",
        );
    }
    assert!(cov.iter().all(|&(cp, _)| f.has_glyph(cp)));

    let cps = f.codepoints();
    assert_eq!(cps.len(), cov.len());
    assert_eq!(cps.first().copied(), cov.first().map(|e| e.0));
    assert!(cov.iter().any(|&(cp, _)| cp == 'A' as u32), "EBGaramond does not cover A");
}

#[test]
fn glyph_bounds_are_tight_and_instanced() {
    let f = font(GARAMOND);
    let gid = f.glyph_id('H' as u32).expect("EBGaramond has H");
    let (x0, y0, x1, y1) = f.glyph_bounds(gid, &[]).expect("H has an outline");
    assert!(x1 > x0 && y1 > y0, "H's box is empty: {x0},{y0},{x1},{y1}");

    let b = f.bbox();
    assert!(
        x0 >= b[0] as f64 - 1.0 && y0 >= b[1] as f64 - 1.0
            && x1 <= b[2] as f64 + 1.0 && y1 <= b[3] as f64 + 1.0,
        "H's box {x0},{y0},{x1},{y1} escapes the font box {b:?}",
    );
    assert!(y0.abs() < 20.0, "H's bottom is at {y0}, not on the baseline");
    assert!(y1 > 500.0, "H's top is at {y1}, which is not a capital");

    assert!(f.glyph_bounds(u16::MAX, &[]).is_none(), "a gid past the end produced a box");
    let space = f.glyph_id(' ' as u32).expect("EBGaramond has a space");
    assert!(f.glyph_bounds(space, &[]).is_none(), "the space glyph produced a box");
    assert!(
        f.glyph_bounds(0, &[]).is_some(),
        ".notdef has no outline, so the doc's example is right after all – check the fixture",
    );

    let v = font(INTER);
    let h = v.glyph_id('H' as u32).expect("Inter has H");
    let light = v.glyph_bounds(h, &[("wght", 100.0)]).expect("light H");
    let heavy = v.glyph_bounds(h, &[("wght", 900.0)]).expect("heavy H");
    assert!(
        heavy.2 - heavy.0 > light.2 - light.0,
        "wght 900 H is not wider than wght 100: {heavy:?} against {light:?}",
    );
}

#[test]
fn os2_info_reads_the_selection_and_the_windows_box() {
    let f = font(GARAMOND);
    let os2 = f.os2_info().expect("EBGaramond carries OS/2");
    assert!(os2.version <= 5, "OS/2 version {} is not a real version", os2.version);
    assert!(os2.selection.is_some(), "no fsSelection came back");
    assert!(!os2.is_italic(), "EBGaramond Regular reported italic");
    assert!(!os2.is_oblique());

    let w = os2.win_metrics.expect("EBGaramond states usWinAscent/Descent");
    assert!(w.ascent > 0 && w.descent > 0, "win metrics are magnitudes: {w:?}");
    assert!(
        w.ascent >= f.ascender() - 1,
        "usWinAscent {} is below the typographic ascender {}, which is the wrong scale",
        w.ascent,
        f.ascender(),
    );
    assert!(w.ascent < 3000, "usWinAscent {} was not scaled to 1000 upm", w.ascent);

    assert!(!(os2.is_regular() && os2.is_italic()), "the font claims both regular and italic");
}

#[test]
fn names_agree_with_name_string() {
    let f = font(GARAMOND);
    let all = f.names();
    assert!(all.len() > 3, "only {} names, which is not a shipping font", all.len());

    for (&id, text) in all.iter() {
        assert_eq!(
            f.name_string(id).as_ref(),
            Some(text),
            "names() and name_string() disagree about name id {id}",
        );
        assert!(!text.is_empty(), "name id {id} came back as an empty string");
    }
    assert!(all.contains_key(&1), "no family name in {:?}", all.keys().collect::<Vec<_>>());
}

#[test]
fn normalized_axes_span_minus_one_to_one() {
    let v = font(INTER);
    let axes = v.axes();
    assert!(!axes.is_empty(), "Inter is variable and should report axes");

    let default = v.normalized_axes(&[]);
    assert_eq!(default.len(), axes.len(), "one coordinate per fvar axis");
    assert!(
        default.iter().all(|&c| c == 0.0),
        "the default location is not the origin: {default:?}",
    );

    let wght = axes.iter().position(|a| a.tag == "wght").expect("Inter varies weight");
    let heavy = v.normalized_axes(&[("wght", axes[wght].max)]);
    assert!(
        (heavy[wght] - 1.0).abs() < 1e-6,
        "the maximum should normalize to 1.0, got {}",
        heavy[wght],
    );
    let light = v.normalized_axes(&[("wght", axes[wght].min)]);
    assert!(
        (light[wght] + 1.0).abs() < 1e-6,
        "the minimum should normalize to -1.0, got {}",
        light[wght],
    );
    let past = v.normalized_axes(&[("wght", 100_000.0)]);
    assert!(past.iter().all(|&c| (-1.0..=1.0).contains(&c)), "out of range: {past:?}");

    let static_face = font(STIX);
    assert!(!static_face.is_variable(), "STIX2Math gained axes; pick another fixture");
    assert!(static_face.normalized_axes(&[]).is_empty());
}

#[test]
fn tracking_is_zero_without_a_trak_table() {
    let f = font(GARAMOND);
    assert!(!f.has_table("trak"), "EBGaramond gained a trak table; pick another fixture");
    assert_eq!(f.tracking(12.0, true), 0.0, "a face with no trak reported tracking");
    assert_eq!(f.tracking(72.0, false), 0.0);
}

#[test]
fn a_path_records_and_replays_an_outline() {
    let f = font(STIX);
    let gid = f.glyph_id('x' as u32).expect("STIX has x");

    let mut path = daegun::Path::default();
    f.outline_glyph_instanced(gid, &[], &mut path).expect("x has an outline");
    assert!(!path.is_empty(), "the pen recorded nothing");

    let (verbs, points) = path.parts();
    assert!(!verbs.is_empty() && !points.is_empty());
    assert!(verbs.contains(&daegun::Verb::Move), "an outline with no move_to");

    let mut again = daegun::Path::default();
    path.replay(None, &mut again);
    assert_eq!(path.parts().1, again.parts().1, "replay moved the points");

    let mut shifted = daegun::Path::default();
    path.replay(Some(&[1.0, 0.0, 0.0, 1.0, 100.0, 0.0]), &mut shifted);
    let (bx0, _, _, _) = path.bounds().expect("x has bounds");
    let (sx0, _, _, _) = shifted.bounds().expect("the shifted copy has bounds");
    assert!((sx0 - bx0 - 100.0).abs() < 0.01, "the transform did not move the box: {bx0} to {sx0}");
}

#[test]
fn the_variation_store_resolves_the_advance_delta_the_engine_applies() {
    let f = font(INTER);
    let hvar = f.table("HVAR").expect("InterVariable carries HVAR");

    let ivs_off = daegun::bytes::read_u32_be(hvar, 4).expect("HVAR header truncated") as usize;
    assert_ne!(ivs_off, 0, "HVAR names no variation store");
    let store =
        daegun::format::parse_item_variation_store(hvar, ivs_off).expect("the store does not parse");
    assert!(!store.regions.is_empty(), "a store with no regions varies nothing");
    assert!(store.axis_count > 0, "the store declares no axes");
    assert!(
        store.ivd_data.iter().any(|ivd| ivd.rows() > 0),
        "every ItemVariationData subtable is empty",
    );

    let map_off = daegun::bytes::read_u32_be(hvar, 8).expect("HVAR header truncated") as usize;
    assert_ne!(map_off, 0, "InterVariable's HVAR grew an implicit mapping; pick another fixture");
    let map = daegun::format::parse_delta_set_index_map(hvar, map_off).expect("the map does not parse");
    assert!(!map.is_empty(), "the delta-set index map is empty");

    let axes = f.axes();
    let wght = axes.iter().find(|a| a.tag == "wght").expect("Inter varies weight");
    let heavy = [("wght", wght.max)];
    let scalars = daegun::format::precompute_region_scalars(&store, &f.normalized_axes(&heavy));
    assert!(
        scalars.iter().any(|&s| s != 0.0),
        "no region is live at the heaviest weight, so nothing could vary",
    );
    let at_origin = daegun::format::precompute_region_scalars(&store, &f.normalized_axes(&[]));
    assert!(at_origin.iter().all(|&s| s == 0.0), "a region is live at the default location");

    let to_scaled = f64::from(1000u16) / f64::from(f.upm());
    let (mut checked, mut moved) = (0usize, 0usize);
    for cp in "HOnexamg0123".chars() {
        let Some(gid) = f.glyph_id(cp as u32) else { continue };
        let (outer, inner) = daegun::format::delta_set_index_map_lookup(&map, gid as usize);
        let delta = daegun::format::compute_ivs_delta_f64(&store, outer, inner, &scalars);

        let base = f.advance_widths(&[gid], &[])[0];
        let bold = f.advance_widths(&[gid], &heavy)[0];
        assert!(
            (bold - base - delta * to_scaled).abs() <= 1.0,
            "{cp}: the store says the advance moves by {delta} units, the engine moved it by {}",
            (bold - base) / to_scaled,
        );
        if delta != 0.0 {
            moved += 1;
        }
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} glyphs had ids, which is not a real face");
    assert!(moved >= 4, "only {moved} of {checked} advances varied, so the store went unexercised");
}

// The spec ignores an axis whose region is backwards or crosses zero; fontTools 4.63 gives 1 for each
// of these at every location.
#[test]
fn a_malformed_region_ignores_its_axis() {
    use daegun::format::{ItemVariationStore, RegionAxis};
    let region = |start, peak, end| vec![RegionAxis { start, peak, end }];
    let store = ItemVariationStore {
        regions:    vec![region(-1.0, 1.0, 1.0), region(0.5, 0.25, 1.0), region(0.0, 1.0, 0.5)],
        ivd_data:   Vec::new(),
        axis_count: 1,
    };
    for at in [0.0, 0.25, 0.75] {
        let scalars = daegun::format::precompute_region_scalars(&store, &[at]);
        assert_eq!(scalars, [1.0; 3], "at {at}");
    }
}

// Every region index and delta of the store, as fontTools 4.63.0 reads it. Only this font and a fixture
// copy set the long-words flag: subtables 2, 6–12 and 22–27, with 6–12 also holding 16-bit deltas.
#[test]
fn a_long_word_variation_store_decodes_to_the_values_fonttools_reads() {
    const EXPECTED: &[(&[usize], &[&[i32]])] = &[
        (&[0, 1], &[&[-8192, 8192], &[0, 8192]]),
        (&[2, 3], &[&[-8192, 8192], &[-8191, 0]]),
        (&[42], &[&[46875], &[48241], &[49151]]),
        (&[77, 78], &[&[-500, 500]]),
        (&[79, 80], &[&[-500, 500], &[500, -500]]),
        (&[81, 82], &[&[-16384, 0], &[0, -16384]]),
        (&[5, 4], &[&[32768, -32768]]),
        (&[7, 6], &[&[32768, -32768]]),
        (&[9, 8], &[&[32768, -32768]]),
        (&[11, 10], &[&[32768, -32768]]),
        (&[37, 36], &[&[32768, -32768]]),
        (&[39, 38], &[&[32768, -32768]]),
        (&[41, 40], &[&[32768, -32768]]),
        (&[12, 13, 83], &[&[-200, 200, 0], &[0, 0, -16384]]),
        (&[14, 15, 16, 17], &[&[-200, 200, 0, 0], &[0, 0, -32768, 32767]]),
        (&[18, 19, 20, 21], &[&[-32768, 32767, 0, 0], &[0, 0, -1000, 1000]]),
        (&[22, 23, 24, 25], &[&[-1000, 1000, 0, 0], &[0, 0, -1000, 1000]]),
        (&[26, 27, 28, 29], &[&[-1000, 1000, 0, 0], &[0, 0, -1000, 1000]]),
        (&[30, 31, 32, 33], &[&[-1000, 1000, 0, 0], &[0, 0, -1000, 1000]]),
        (&[34, 35, 43, 44], &[&[-1000, 1000, 0, 0], &[0, 0, -500, 500]]),
        (&[45, 46, 47, 48], &[&[-500, 500, 0, 0], &[0, 0, -8192, 8192]]),
        (&[49, 50, 51, 52], &[&[-8192, 8192, 0, 0], &[0, 0, -500, 500]]),
        (&[55, 56], &[&[-131072, 131072]]),
        (&[57, 58], &[&[-131072, 131072]]),
        (&[59, 60], &[&[-131072, 131072]]),
        (&[61, 62], &[&[-131072, 131072]]),
        (&[63, 64], &[&[-32768000, 32768000]]),
        (&[65, 66], &[&[-32768000, 32768000]]),
        (&[53, 54, 67, 68], &[&[-500, 500, 0, 0], &[0, 0, -500, 500]]),
        (&[69, 70, 71, 72], &[&[-500, 500, 0, 0], &[0, 0, -500, 500]]),
        (&[73, 74, 75, 76], &[&[-500, 500, 0, 0], &[0, 0, -500, 500]]),
    ];

    let f = font("colr-v1-test-glyphs/test_glyphs_variable.ttf");
    let colr = f.table("COLR").expect("the variable COLR fixture carries COLR");
    let ivs_off = daegun::bytes::read_u32_be(colr, 30).expect("COLR v1 header truncated") as usize;
    let store =
        daegun::format::parse_item_variation_store(colr, ivs_off).expect("the store does not parse");

    assert_eq!(store.regions.len(), 84, "region count");
    assert_eq!(store.ivd_data.len(), EXPECTED.len(), "ItemVariationData count");
    for (i, (ivd, (regions, rows))) in store.ivd_data.iter().zip(EXPECTED).enumerate() {
        assert_eq!(ivd.region_indices, *regions, "subtable {i}: region indices");
        assert_eq!(ivd.rows(), rows.len(), "subtable {i}: row count");
        for (r, want) in rows.iter().enumerate() {
            assert_eq!(ivd.row(r), Some(*want), "subtable {i}, row {r}");
        }
    }
}

// IVD offsets may repeat, and each copy is held: 64 offsets at one IVD of 65,535 regions would hold
// 50 MB from 197 KB. A store holds about 32 times the bytes from its start, or a few megabytes when small.
#[test]
fn a_variation_store_cannot_hold_far_more_than_its_bytes() {
    let store = |copies: usize, items: usize, regions: usize| {
        let ivd = 8 + copies * 4 + 4;
        let mut buf = vec![0u8; ivd + 6 + regions * 2 + items * regions];
        daegun::bytes::write_u16_be(&mut buf, 0, 1);
        daegun::bytes::write_u32_be(&mut buf, 2, (8 + copies * 4) as u32);
        daegun::bytes::write_u16_be(&mut buf, 6, copies as u16);
        for i in 0..copies {
            daegun::bytes::write_u32_be(&mut buf, 8 + i * 4, ivd as u32);
        }
        daegun::bytes::write_u16_be(&mut buf, ivd, items as u16);
        daegun::bytes::write_u16_be(&mut buf, ivd + 4, regions as u16);
        buf
    };
    let one = daegun::format::parse_item_variation_store(&store(1, 1, 65_535), 0).expect("one wide IVD parses");
    assert_eq!(one.ivd_data[0].row(0).map(<[i32]>::len), Some(65_535), "the wide IVD lost its row");
    assert!(
        daegun::format::parse_item_variation_store(&store(64, 1, 65_535), 0).is_err(),
        "64 offsets at one wide IVD were all held",
    );
    // Callers pass the whole table, so the same store after 10 MiB of other data must still decline.
    let mut table = vec![0u8; 10 << 20];
    table.extend(store(64, 1, 65_535));
    assert!(
        daegun::format::parse_item_variation_store(&table, 10 << 20).is_err(),
        "the bytes before the store paid for its copies",
    );
    // A row of no regions costs no bytes, so only the row budget bounds 640 copies of 65,535 of them.
    assert!(daegun::format::parse_item_variation_store(&store(1, 65_535, 0), 0).is_ok(), "one tall IVD failed");
    assert!(
        daegun::format::parse_item_variation_store(&store(640, 65_535, 0), 0).is_err(),
        "640 offsets at one tall IVD of empty rows were all held",
    );
    // The base comes from the caller, from C as a size_t, so an offset near the top must decline.
    for base in [usize::MAX, usize::MAX - 1, usize::MAX - 7] {
        assert!(daegun::format::parse_item_variation_store(&[0u8; 64], base).is_err(), "base {base:#x} parsed");
    }
}

#[test]
fn ot_round_breaks_ties_toward_positive_infinity() {
    for (v, want) in [(0.5, 1), (1.5, 2), (2.5, 3), (-0.5, 0), (-1.5, -1), (-2.5, -2)] {
        assert_eq!(daegun::format::ot_round(v), want, "ot_round({v})");
    }
    assert_eq!(daegun::format::ot_round(-0.5), 0);
    assert_eq!((-0.5f64).round() as i32, -1);
    for (v, want) in [(0.0, 0), (0.4, 0), (0.6, 1), (-0.4, 0), (-0.6, -1), (7.0, 7)] {
        assert_eq!(daegun::format::ot_round(v), want, "ot_round({v})");
    }
}

#[test]
fn coverage_index_walks_a_real_lookup() {
    use daegun::bytes::read_u16_be;
    let f = font(GARAMOND);
    let gsub = f.table("GSUB").expect("EBGaramond carries GSUB");

    let lookup_list = read_u16_be(gsub, 8).expect("GSUB header truncated") as usize;
    let lookup_count = read_u16_be(gsub, lookup_list).expect("LookupList truncated");
    assert!(lookup_count > 0, "GSUB declares no lookups");

    let mut found = 0usize;
    for i in 0..lookup_count as usize {
        let Some(rel) = read_u16_be(gsub, lookup_list + 2 + i * 2) else { continue };
        let lookup = lookup_list + rel as usize;
        let (Some(kind), Some(subs)) = (read_u16_be(gsub, lookup), read_u16_be(gsub, lookup + 4))
        else {
            continue;
        };
        if kind == 7 || subs == 0 {
            continue;
        }
        let Some(sub_rel) = read_u16_be(gsub, lookup + 6) else { continue };
        let sub = lookup + sub_rel as usize;
        let Some(cov_rel) = read_u16_be(gsub, sub + 2) else { continue };
        if cov_rel == 0 {
            continue;
        }
        let cov = &gsub[sub + cov_rel as usize..];

        let mut seen: Vec<(u16, u16)> = Vec::new();
        for gid in 0..f.num_glyphs() {
            if let Some(idx) = daegun::format::coverage_index(cov, gid) {
                seen.push((gid, idx));
            }
        }
        if seen.is_empty() {
            continue;
        }
        for (n, &(gid, idx)) in seen.iter().enumerate() {
            assert_eq!(
                idx as usize, n,
                "glyph {gid} is the {n}th covered glyph but reports index {idx}",
            );
        }
        found += 1;
        if found == 3 {
            break;
        }
    }
    assert!(found > 0, "no GSUB lookup in EBGaramond produced a readable Coverage");

    assert!(daegun::format::coverage_index(&[0, 3, 0, 1, 0, 0], 0).is_none());
    assert!(daegun::format::coverage_index(&[], 0).is_none());
}

#[test]
fn an_aat_lookup_enumerates_what_it_answers() {
    use daegun::bytes::{read_u16_be, read_u32_be};
    let f = font(MORX);
    let morx = f.table("morx").expect("TestMORXOne carries morx");

    let chains = read_u32_be(morx, 4).expect("morx header truncated");
    assert_eq!(chains, 1, "the fixture is a single-chain morx");
    let chain_len = read_u32_be(morx, 12).expect("chain header truncated") as usize;
    let feature_count = read_u32_be(morx, 16).expect("chain header truncated") as usize;
    let subtable_count = read_u32_be(morx, 20).expect("chain header truncated");
    assert!(subtable_count > 0, "the chain declares no subtables");
    let chain = &morx[8..8 + chain_len];

    let sub_at = 16 + feature_count * 12;
    let sub_len = read_u32_be(chain, sub_at).expect("subtable header truncated") as usize;
    let coverage = read_u32_be(chain, sub_at + 4).expect("subtable header truncated");
    assert_eq!(coverage & 0xFF, 4, "TestMORXOne's subtable is no longer non-contextual");
    let body = &chain[sub_at + 12..sub_at + sub_len];

    let lookup = daegun::format::Lookup::parse(body, f.num_glyphs()).expect("the lookup does not parse");
    let entries = lookup.entries();
    assert!(!entries.is_empty(), "the lookup substitutes nothing");

    for &(gid, to) in &entries {
        assert_eq!(lookup.value(gid), Some(to), "entries() and value() disagree about glyph {gid}");
        assert_ne!(to, gid, "a substitution that changes nothing");
    }
    for gid in 0..f.num_glyphs() {
        let listed = entries.iter().find(|&&(g, _)| g == gid).map(|&(_, v)| v);
        assert_eq!(lookup.value(gid), listed, "value() covers glyph {gid} and entries() does not");
    }
    assert!(daegun::format::Lookup::parse(&[0, 6], f.num_glyphs()).is_none_or(|l| l.entries().is_empty()));

    assert!(read_u16_be(morx, 0).is_some());
}

#[test]
fn feature_variations_selects_by_axis_range() {
    let mut t = vec![0u8; 100];
    t[2..4].copy_from_slice(&1u16.to_be_bytes());
    t[10..14].copy_from_slice(&100u32.to_be_bytes());

    let mut fv: Vec<u8> = Vec::new();
    let push32 = |v: u32, out: &mut Vec<u8>| out.extend_from_slice(&v.to_be_bytes());
    fv.extend_from_slice(&1u16.to_be_bytes());
    fv.extend_from_slice(&0u16.to_be_bytes());
    push32(2, &mut fv);
    push32(24, &mut fv);
    push32(40, &mut fv);
    push32(58, &mut fv);
    push32(0, &mut fv);
    assert_eq!(fv.len(), 24, "the record array is not where the offsets say");

    fv.extend_from_slice(&1u16.to_be_bytes());
    push32(8, &mut fv);
    fv.extend_from_slice(&[0, 0]);
    fv.extend_from_slice(&1u16.to_be_bytes());
    fv.extend_from_slice(&0u16.to_be_bytes());
    fv.extend_from_slice(&8192i16.to_be_bytes());
    fv.extend_from_slice(&16384i16.to_be_bytes());
    assert_eq!(fv.len(), 40, "the condition set is not where the record says");

    fv.extend_from_slice(&1u16.to_be_bytes());
    fv.extend_from_slice(&0u16.to_be_bytes());
    fv.extend_from_slice(&1u16.to_be_bytes());
    fv.extend_from_slice(&3u16.to_be_bytes());
    push32(12, &mut fv);
    fv.extend_from_slice(&[0; 6]);
    assert_eq!(fv.len(), 58, "the substitution table is not where the record says");
    fv.extend_from_slice(&0u16.to_be_bytes());

    t.extend_from_slice(&fv);
    let table = daegun::format::FeatureVariations::parse(&t).expect("a 1.1 header with an offset");

    assert_eq!(table.find(&[0]), Some(1), "the empty condition set should match at the origin");
    assert_eq!(table.find(&[16384]), Some(0), "axis 0 at 1.0 is inside [0.5, 1.0]");
    assert_eq!(table.find(&[8192]), Some(0), "the range is inclusive at its minimum");
    assert_eq!(table.find(&[8191]), Some(1), "8191 is below the range and must not match record 0");

    assert_eq!(table.substitute(0, 3), Some(100 + 40 + 12));
    assert_eq!(table.substitute(0, 4), None, "feature 4 has no substitution");
    assert_eq!(table.substitute(1, 3), None, "record 1 declares no substitution table");

    assert!(daegun::format::FeatureVariations::parse(&[0u8; 100]).is_none());
    // `at` can come from C as any size_t, and every offset is added to it.
    for at in [usize::MAX, usize::MAX - 1, usize::MAX - 7] {
        let far = daegun::format::FeatureVariations::at(&t, at);
        assert_eq!((far.find(&[]), far.substitute(0, 0)), (None, None), "at {at:#x} answered");
    }
    let mut null = t.clone();
    null[10..14].copy_from_slice(&0u32.to_be_bytes());
    assert!(daegun::format::FeatureVariations::parse(&null).is_none());
}

#[test]
fn base_is_glyph_free_answers_for_the_font_that_has_one() {
    let serif = font(SOURCE_SERIF);
    assert!(serif.has_table("BASE"), "SourceSerif4Variable lost its BASE; pick another fixture");
    let free = serif.base_is_glyph_free();
    assert!(
        serif.base_info("latn", false, &[]).is_some(),
        "BASE does not describe the Latin script, so nothing here is being read",
    );
    assert!(free, "SourceSerif4Variable's BASE names a glyph, which would be a first for a text face");

    let g = font(GARAMOND);
    assert!(!g.has_table("BASE"), "EBGaramond gained a BASE; pick another fixture");
    assert!(!g.base_is_glyph_free());
}

#[test]
fn the_default_point_size_is_what_trak_falls_back_to() {
    assert_eq!(daegun::DEFAULT_POINT_SIZE, 12.0);
    let f = font(GARAMOND);
    assert_eq!(f.tracking(daegun::DEFAULT_POINT_SIZE, true), 0.0, "EBGaramond has no trak");
}

#[test]
fn a_font_rebuilt_from_its_tables_survives_an_edit() {
    use std::collections::BTreeMap;
    let f = font(GARAMOND);
    let gid = f.glyph_id('H' as u32).expect("EBGaramond has H");

    let mut tables: BTreeMap<String, Vec<u8>> = f
        .table_tags()
        .into_iter()
        .map(|t| (t.to_string(), f.table(t).expect("listed but absent").to_vec()))
        .collect();

    let rebuilt = daegun::build_font(&tables);
    assert!(!rebuilt.is_empty(), "build_font produced nothing from a real font");
    let again = daegun::Font::from_bytes(&rebuilt).expect("the rebuilt font does not parse");
    assert_eq!(again.table_tags(), f.table_tags(), "the rebuilt font lost or gained a table");
    assert_eq!(again.num_glyphs(), f.num_glyphs());
    assert_eq!(again.upm(), f.upm());
    assert_eq!(again.advance_widths(&[gid], &[]), f.advance_widths(&[gid], &[]));
    for tag in f.table_tags() {
        let (before, after) = (f.table(tag).expect("listed"), again.table(tag).expect("rebuilt"));
        if tag == "head" {
            assert_eq!(before.len(), after.len(), "head changed length");
            assert_eq!(before[..8], after[..8], "head changed before checkSumAdjustment");
            assert_eq!(before[12..], after[12..], "head changed after checkSumAdjustment");
            continue;
        }
        assert_eq!(before, after, "{tag} did not survive the rebuild");
    }

    let original_upm = f.upm();
    let head = tables.get_mut("head").expect("every font has head");
    daegun::bytes::write_u16_be(head, 18, original_upm * 2);
    assert_eq!(
        daegun::bytes::read_u16_be(head, 18),
        Some(original_upm * 2),
        "the writer and the reader disagree about the same two bytes",
    );

    let patched = daegun::Font::from_bytes(&daegun::build_font(&tables)).expect("the patched font parses");
    assert_eq!(patched.upm(), original_upm * 2, "the edit did not reach the rebuilt font");
    let before = f.advance_widths(&[gid], &[])[0];
    let after = patched.advance_widths(&[gid], &[])[0];
    assert!(
        (after * 2.0 - before).abs() <= 1.0,
        "doubling the em should halve the scaled advance: {before} became {after}",
    );

    let mut two = [0u8; 2];
    daegun::bytes::write_u16_be(&mut two, 1, 0xFFFF);
    assert_eq!(two, [0, 0], "a write past the end changed bytes it does not own");
}

#[test]
fn instanced_tables_agree_with_the_instanced_metrics() {
    let f = font(INTER);
    let gid = f.glyph_id('H' as u32).expect("Inter has H");
    let axes = [("wght", 900.0)];

    let tables = f.instance_tables(&axes).expect("InterVariable instances");
    for gone in ["fvar", "gvar", "HVAR", "MVAR", "avar"] {
        assert!(!tables.contains_key(gone), "{gone} survived instancing, so the font is still variable");
    }
    assert!(tables.contains_key("glyf") && tables.contains_key("hmtx"));

    let cmap = tables.get("cmap").expect("cmap passes through instancing");
    assert!(matches!(cmap, std::borrow::Cow::Borrowed(_)), "cmap was copied rather than borrowed");
    assert_eq!(cmap.as_ref(), f.table("cmap").expect("cmap"));
    let glyf = tables.get("glyf").expect("glyf");
    assert!(matches!(glyf, std::borrow::Cow::Owned(_)), "glyf came back borrowed, so nothing varied");
    assert_ne!(glyf.as_ref(), f.table("glyf").expect("glyf"), "the instanced glyf is the stored one");

    let rebuilt = daegun::Font::from_bytes(&daegun::build_font(&tables)).expect("the instance parses");
    assert!(!rebuilt.is_variable(), "the rebuilt instance still claims to be variable");
    assert_eq!(rebuilt.upm(), f.upm());
    let direct = f.advance_widths(&[gid], &axes)[0];
    let through_tables = rebuilt.advance_widths(&[gid], &[])[0];
    assert!(
        (direct - through_tables).abs() <= 1.0,
        "instance_tables says {through_tables} where advance_widths says {direct}",
    );
    let default_advance = f.advance_widths(&[gid], &[])[0];
    assert!(
        (through_tables - default_advance).abs() > 1.0,
        "the instanced advance equals the default, so nothing was resolved",
    );
}

#[test]
fn a_hinted_glyph_is_geometry_in_device_space() {
    use daegun::HintMode;
    let f = font(GARAMOND);
    let gid = f.glyph_id('H' as u32).expect("EBGaramond has H");

    assert!(
        f.hinted_glyph(gid, 16.0, &[], HintMode::Subpixel).is_none(),
        "EBGaramond gained hinting bytecode; pick another fixture",
    );
    let out = f.hinted_glyph(gid, 16.0, &[], HintMode::AutoForce).expect("the autohinter hints H");
    assert!(!out.x.is_empty(), "a hinted outline with no points");
    assert_eq!(out.x.len(), out.y.len());
    assert_eq!(out.x.len(), out.flags.len());
    assert!(!out.contour_ends.is_empty(), "no contours");
    assert!(
        out.contour_ends.iter().all(|&e| e < out.x.len()),
        "a contour ends past the last point",
    );
    assert!(out.flags.iter().any(|f| f & daegun::FLAG_ON_CURVE != 0), "no on-curve point");

    let mut path = daegun::Path::default();
    daegun::draw_hinted(&out, &mut path);
    let (x0, y0, x1, y1) = path.bounds().expect("the replayed outline has bounds");
    assert!(x1 > x0 && y1 > y0, "the hinted box is empty: {x0},{y0},{x1},{y1}");
    assert!(
        (y1 - y0) > 4.0 && (y1 - y0) < 20.0,
        "an H at 16px is {} tall, which is not pixels",
        y1 - y0,
    );
    let big = f.hinted_glyph(gid, 64.0, &[], HintMode::AutoForce).expect("hinted at 64px");
    let mut big_path = daegun::Path::default();
    daegun::draw_hinted(&big, &mut big_path);
    let (_, by0, _, by1) = big_path.bounds().expect("bounds");
    assert!(
        (by1 - by0) > (y1 - y0) * 3.0,
        "64px is {} tall against 16px at {}, which is not a scale",
        by1 - by0,
        y1 - y0,
    );

    assert!(f.hinted_glyph(gid, 16.0, &[], HintMode::None).is_none());
    assert!(f.hinted_glyph(gid, 0.0, &[], HintMode::AutoForce).is_none(), "zero ppem");
    assert!(f.hinted_glyph(u16::MAX, 16.0, &[], HintMode::AutoForce).is_none(), "a gid past the end");

    let hinted = font("test-fixtures/hinted.ttf");
    assert!(
        hinted.hinted_glyph(1, 16.0, &[], HintMode::Subpixel).is_some(),
        "the hinted fixture lost its bytecode",
    );
}

#[test]
fn cff_hints_report_the_declared_stems() {
    let f = font(STIX);
    let (gid, hints) = (0..f.num_glyphs())
        .filter_map(|g| f.cff_hints(g).map(|h| (g, h)))
        .find(|(_, h)| !h.stems.is_empty())
        .expect("no STIX2Math glyph declares a stem, which is not a hinted CFF font");

    // Not min <= max: an edge hint keeps its negative width, as the test below pins.
    for s in &hints.stems {
        assert!(s.min.is_finite() && s.max.is_finite(), "glyph {gid} declares a stem from {} to {}", s.min, s.max);
    }
    for (at, mask) in &hints.masks {
        assert!(*at <= 100_000, "a hintmask at point {at}");
        assert_eq!(mask.len(), hints.stems.len().div_ceil(8), "a mask that does not cover the stems");
    }

    let g = font(GARAMOND);
    assert!(!g.has_table("CFF "), "EBGaramond gained a CFF table; pick another fixture");
    assert!(g.cff_hints(g.glyph_id('H' as u32).expect("H")).is_none());

    // A CFF2 font's stems are its default instance's, as the CFF hinter reads them.
    let serif = font("source-serif/SourceSerif4Variable-Roman.otf");
    let h = serif.cff_hints(serif.glyph_id('H' as u32).expect("H")).expect("a CFF2 font gave no hints");
    assert!(!h.stems.is_empty(), "Source Serif's H declares no stems");
}

// Stem edges as fontTools 4.63.0 decodes them from the charstrings: within each stem operator the pairs
// run on from the previous edge, and an edge hint keeps its negative width, as A's (662, 642) does.
#[test]
fn cff_stems_decode_to_the_edges_fonttools_reads() {
    type Stem = (f32, f32, bool);
    const EXPECTED: &[(char, u16, &[Stem])] = &[
        ('H', 10, &[(0.0, 30.0, false), (318.0, 357.0, false), (627.0, 657.0, false), (133.0, 225.0, true),
            (553.0, 644.0, true)]),
        ('O', 17, &[(-12.0, 30.0, false), (627.0, 669.0, false), (51.0, 156.0, true), (583.0, 688.0, true)]),
        ('x', 279, &[(0.0, 29.0, false), (444.0, 473.0, false), (445.0, 473.0, false), (-2.0, 165.0, true),
            (11.0, 226.0, true), (273.0, 482.0, true), (300.0, 465.0, true), (341.0, 465.0, true)]),
        ('B', 4, &[(0.0, 30.0, false), (0.0, 37.0, false), (320.0, 362.0, false), (618.0, 657.0, false),
            (627.0, 657.0, false), (133.0, 225.0, true), (452.0, 551.0, true), (478.0, 583.0, true)]),
        ('A', 3, &[(0.0, 30.0, false), (219.0, 259.0, false), (662.0, 642.0, false), (3.0, 146.0, true),
            (3.0, 213.0, true), (3.0, 714.0, true)]),
    ];

    let f = font(STIX);
    for &(ch, gid, want) in EXPECTED {
        assert_eq!(f.glyph_id(ch as u32), Some(gid), "{ch} maps to a different glyph than fontTools named");
        let hints = f.cff_hints(gid).unwrap_or_else(|| panic!("{ch} has no CFF hints"));
        let got: Vec<Stem> = hints.stems.iter().map(|s| (s.min, s.max, s.vertical)).collect();
        assert_eq!(got, want, "{ch}: stems");
    }
}

#[test]
fn coverage_glyphs_agrees_with_coverage_index() {
    use daegun::bytes::read_u16_be;
    let f = font(GARAMOND);
    let gsub = f.table("GSUB").expect("EBGaramond carries GSUB");
    let lookup_list = read_u16_be(gsub, 8).expect("GSUB header") as usize;
    let lookup_count = read_u16_be(gsub, lookup_list).expect("LookupList");

    let mut checked = 0usize;
    for i in 0..lookup_count as usize {
        let Some(rel) = read_u16_be(gsub, lookup_list + 2 + i * 2) else { continue };
        let lookup = lookup_list + rel as usize;
        let (Some(kind), Some(subs)) = (read_u16_be(gsub, lookup), read_u16_be(gsub, lookup + 4))
        else {
            continue;
        };
        if kind == 7 || subs == 0 {
            continue;
        }
        let Some(sub_rel) = read_u16_be(gsub, lookup + 6) else { continue };
        let sub = lookup + sub_rel as usize;
        let Some(cov_rel) = read_u16_be(gsub, sub + 2).filter(|&r| r != 0) else { continue };
        let at = sub + cov_rel as usize;

        let Ok(glyphs) = daegun::format::coverage_glyphs(gsub, at) else { continue };
        if glyphs.is_empty() {
            continue;
        }
        for w in glyphs.windows(2) {
            assert!(w[0] < w[1], "coverage is not strictly ascending at {}", w[0]);
        }
        let cov = &gsub[at..];
        for (n, &g) in glyphs.iter().enumerate() {
            assert_eq!(
                daegun::format::coverage_index(cov, g),
                u16::try_from(n).ok(),
                "glyph {g} enumerates at {n} but coverage_index disagrees",
            );
        }
        checked += 1;
        if checked == 3 {
            break;
        }
    }
    assert!(checked > 0, "no readable Coverage found in EBGaramond's GSUB");
}

#[test]
fn the_aat_sentinels_drive_a_state_table() {
    use daegun::bytes::read_u32_be;
    use daegun::format::{class, state, Lookup, StateTable};

    assert_eq!(class::END_OF_TEXT, 0);
    assert_eq!(class::DELETED_GLYPH, 2);
    assert_eq!(state::START_OF_TEXT, 0);

    let f = font("aat/TestMORXTwentyfour.ttf");
    let morx = f.table("morx").expect("carries morx");
    let chain_len = read_u32_be(morx, 12).expect("chain header") as usize;
    let features = read_u32_be(morx, 16).expect("chain header") as usize;
    let chain = &morx[8..8 + chain_len];
    let sub_at = 16 + features * 12;
    let sub_len = read_u32_be(chain, sub_at).expect("subtable header") as usize;
    assert_eq!(
        read_u32_be(chain, sub_at + 4).expect("coverage") & 0xFF,
        1,
        "the fixture's subtable is no longer contextual",
    );
    let body = &chain[sub_at + 12..sub_at + sub_len];

    let table = StateTable::parse(body, 2, f.num_glyphs()).expect("the state table parses");
    // `extra_words` comes from C as any size_t, and an entry's stride is 4 + 2 * extra_words bytes.
    for words in [usize::MAX, 1 << 63, usize::MAX / 2] {
        assert!(StateTable::parse(body, words, f.num_glyphs()).is_none(), "{words} extra words were accepted");
    }
    assert_eq!(table.class(0xFFFF), class::DELETED_GLYPH);
    let reachable = (0..f.num_glyphs())
        .filter(|&g| table.class(g) > class::DELETED_GLYPH)
        .filter_map(|g| table.entry(state::START_OF_TEXT, table.class(g)))
        .count();
    assert!(reachable > 0, "no glyph's class has an entry in the start state");

    let class_table = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
    let classes = Lookup::parse(&body[class_table..], f.num_glyphs()).expect("the class table parses").entries();
    assert!(!classes.is_empty() && classes.len() <= usize::from(f.num_glyphs()));
    assert!(classes.iter().all(|&(g, c)| table.class(g) == c), "the class lookup disagrees with the state table");

    assert_eq!(daegun::format::ankr_version(&[0, 0, 0, 1]), Some(0));
    assert_eq!(daegun::format::ankr_version(&[0]), None);
    assert_eq!(daegun::format::control_point(&[0xFF, 0xFE, 0x00, 0x05], 0), Some((-2, 5)));
    assert_eq!(daegun::format::control_point(&[0, 0], 0), None);
}

#[test]
fn os2_reports_the_typographic_line_box() {
    let f = font(INTER);
    let os2 = f.os2_info().expect("InterVariable carries OS/2");
    let typo = os2.typo_metrics.expect("Inter states sTypoAscender/Descender/LineGap");

    assert!(typo.ascender > 0, "the typographic ascender is above the baseline");
    assert!(typo.descender < 0, "the typographic descender is below it, and signed");
    assert!(typo.line_gap >= 0);
    assert!(typo.ascender < 3000, "ascender {} was not scaled to 1000 upm", typo.ascender);

    let raw = f.table("OS/2").expect("OS/2");
    let scale = 1000.0 / f64::from(f.upm());
    for (offset, got, name) in [
        (68usize, typo.ascender, "sTypoAscender"),
        (70, typo.descender, "sTypoDescender"),
        (72, typo.line_gap, "sTypoLineGap"),
    ] {
        let stored = daegun::bytes::read_i16_be(raw, offset).expect("OS/2 truncated");
        let want = (f64::from(stored) * scale).round() as i32;
        assert_eq!(got, want, "{name}: {stored} font units scaled to {want}, not {got}");
    }

    let line = f.line_metrics(false);
    if os2.uses_typo_metrics() {
        assert!(
            (line.ascent - f64::from(typo.ascender)).abs() <= 1.0,
            "the font asks for typo metrics but line_metrics reports {} against {}",
            line.ascent,
            typo.ascender,
        );
    }
}

fn closure_of(f: &Font, gids: &[u16]) -> Vec<u16> {
    let closure = f.glyph_closure(gids, &[]).expect("the closure converges");
    assert!(closure.contains(&0), "the closure dropped .notdef");
    for w in closure.windows(2) {
        assert!(w[0] < w[1], "the closure is not strictly ascending at {}", w[0]);
    }
    for g in gids {
        assert!(closure.contains(g), "the closure dropped requested glyph {g}");
    }
    closure
}

#[test]
fn the_glyph_closure_is_exactly_what_a_subset_keeps() {
    for face in [GARAMOND, INTER] {
        let f = font(face);
        let gids: Vec<u16> = "fifl AVTo".chars().filter_map(|c| f.glyph_id(c as u32)).collect();
        assert!(gids.len() > 5, "{face}: only {} of the sample glyphs resolved", gids.len());

        let closure = closure_of(&f, &gids);
        let result = f.subset(&gids, &[]).expect("the subset builds");
        assert!(!result.gid_map.is_empty(), "{face}: a glyf subset compacts and reports a map");

        let kept: Vec<u16> = (0..result.gid_map.len() as u16)
            .filter(|&i| i == 0 || result.gid_map[i as usize] != 0)
            .collect();
        assert_eq!(closure, kept, "{face}: the closure and the subset disagree about what survives");

        assert!(
            closure.len() > gids.len() + 1,
            "{face}: {} requested became {}, so nothing was reached",
            gids.len(),
            closure.len(),
        );
        assert!(closure.len() < usize::from(f.num_glyphs()), "{face}: the closure kept the whole font");

        let built = daegun::Font::from_bytes(&result.ttf).expect("the subset parses");
        assert_eq!(
            usize::from(built.num_glyphs()),
            closure.len(),
            "{face}: the subset has {} glyphs where the closure named {}",
            built.num_glyphs(),
            closure.len(),
        );
    }
}

#[test]
fn the_closure_reaches_components_and_stretchy_variants() {
    use daegun::bytes::{read_i16_be, read_u16_be, read_u32_be};

    let f = font(GARAMOND);
    let (glyf, loca, head) = (
        f.table("glyf").expect("glyf"),
        f.table("loca").expect("loca"),
        f.table("head").expect("head"),
    );
    let long_loca = read_i16_be(head, 50).expect("indexToLocFormat") != 0;
    let offset_of = |g: usize| {
        if long_loca {
            read_u32_be(loca, g * 4).map(|v| v as usize)
        } else {
            read_u16_be(loca, g * 2).map(|v| v as usize * 2)
        }
    };

    let composite = (0..usize::from(f.num_glyphs()))
        .find(|&g| {
            let (Some(s), Some(e)) = (offset_of(g), offset_of(g + 1)) else { return false };
            s < e && read_i16_be(glyf, s) == Some(-1)
        })
        .expect("EBGaramond has no composite glyph, which is not a real text face");

    let mut components: Vec<u16> = Vec::new();
    let mut pos = offset_of(composite).expect("offset") + 10;
    while let (Some(flags), Some(gid)) = (read_u16_be(glyf, pos), read_u16_be(glyf, pos + 2)) {
        components.push(gid);
        pos += 4;
        pos += if flags & 0x0001 != 0 { 4 } else { 2 };
        if flags & 0x0080 != 0 {
            pos += 8;
        } else if flags & 0x0040 != 0 {
            pos += 4;
        } else if flags & 0x0008 != 0 {
            pos += 2;
        }
        if flags & 0x0020 == 0 {
            break;
        }
    }
    assert!(!components.is_empty(), "glyph {composite} is composite but names no component");

    let closure = closure_of(&f, &[composite as u16]);
    for c in &components {
        assert!(
            closure.contains(c),
            "component {c} of composite {composite} is missing from its own closure {closure:?}",
        );
    }

    let m = font(STIX);
    let stretchy = (0..m.num_glyphs())
        .find(|&g| m.math_glyph_variants(g, true).is_some_and(|c| !c.variants.is_empty()))
        .expect("STIX2Math has no vertically stretchy glyph");
    let variants: Vec<u16> = m
        .math_glyph_variants(stretchy, true)
        .expect("variants")
        .variants
        .iter()
        .map(|v| v.glyph_id)
        .collect();
    assert!(variants.len() > 1, "only {} variant, so nothing is pulled in", variants.len());

    let math_closure = closure_of(&m, &[stretchy]);
    for v in &variants {
        assert!(
            math_closure.contains(v),
            "stretchy variant {v} of {stretchy} is missing from {math_closure:?}",
        );
    }
}

#[test]
fn the_closure_answers_for_cff_where_the_subset_cannot() {
    let f = font(STIX);
    assert!(f.has_table("CFF "), "STIX2Math is the CFF fixture");
    let gids: Vec<u16> = "xyz+=".chars().filter_map(|c| f.glyph_id(c as u32)).collect();
    assert!(gids.len() >= 4, "only {} of the sample glyphs resolved", gids.len());

    let result = f.subset(&gids, &[]).expect("the subset builds");
    assert!(
        result.gid_map.is_empty(),
        "the CFF subset now compacts, so `glyph_closure`'s documented asymmetry is stale",
    );

    let closure = closure_of(&f, &gids);
    assert!(closure.len() > gids.len(), "the closure reached nothing beyond the request");
    assert!(closure.len() < usize::from(f.num_glyphs()), "the closure kept the whole font");
}

#[test]
fn the_closure_handles_the_edges() {
    let f = font(GARAMOND);

    assert_eq!(f.glyph_closure(&[], &[]).expect("empty request"), vec![0]);

    let h = f.glyph_id('H' as u32).expect("H");
    let real = f.glyph_closure(&[h], &[]).expect("H");
    let padded = f.glyph_closure(&[h, u16::MAX], &[]).expect("H plus a gid past the end");
    assert_eq!(real, padded, "a gid the font does not have changed the closure");

    let v = font(INTER);
    let vh = v.glyph_id('H' as u32).expect("Inter has H");
    for wght in [100.0, 900.0] {
        let c = v.glyph_closure(&[vh], &[("wght", wght)]).expect("closes at every location");
        assert!(c.contains(&vh) && c.contains(&0), "wght {wght} lost a requested glyph");
    }
}

#[test]
fn an_out_of_range_request_is_ignored_by_both_flavors() {
    for (face, ch) in [(GARAMOND, 'H'), (STIX, 'x'), (INTER, 'H')] {
        let f = font(face);
        let gid = f.glyph_id(ch as u32).unwrap_or_else(|| panic!("{face} has no {ch}"));

        let real = f.glyph_closure(&[gid], &[]).expect("closes");
        let padded = f.glyph_closure(&[gid, u16::MAX], &[]).expect("closes");
        assert_eq!(real, padded, "{face}: a gid past the end changed the closure");

        let n = f.num_glyphs();
        for g in &padded {
            assert!(*g < n, "{face}: the closure names glyph {g} of {n}");
        }
        assert_eq!(
            f.glyph_closure(&[u16::MAX, n], &[]).expect("closes"),
            vec![0],
            "{face}: an entirely out-of-range request produced more than .notdef",
        );
    }
}

#[test]
fn instance_tables_answers_for_a_static_font_too() {
    for face in [STIX, "test-fixtures/hinted.ttf"] {
        let f = font(face);
        assert!(!f.is_variable(), "{face} is the static fixture");

        let tables = f.instance_tables(&[]).unwrap_or_else(|| panic!("{face}: no instanced tables"));
        assert_eq!(tables.len(), f.table_tags().len(), "{face}: a table went missing");
        for tag in f.table_tags() {
            let got = tables.get(tag).unwrap_or_else(|| panic!("{face}: {tag} missing"));
            assert_eq!(got.as_ref(), f.table(tag).expect("listed"), "{face}: {tag} changed");
            assert!(matches!(got, std::borrow::Cow::Borrowed(_)), "{face}: {tag} was copied");
        }
        assert_eq!(
            f.instance_tables(&[("wght", 900.0)]).map(|t| t.len()),
            Some(f.table_tags().len()),
            "{face}: naming an axis it does not have changed the answer",
        );
    }
}

#[test]
fn a_composite_hints_only_under_the_autohinter() {
    use daegun::{bytes::{read_i16_be, read_u16_be, read_u32_be}, HintMode};
    let f = font(GARAMOND);
    let (glyf, loca, head) = (
        f.table("glyf").expect("glyf"),
        f.table("loca").expect("loca"),
        f.table("head").expect("head"),
    );
    let long_loca = read_i16_be(head, 50).expect("indexToLocFormat") != 0;
    let offset_of = |g: usize| {
        if long_loca {
            read_u32_be(loca, g * 4).map(|v| v as usize)
        } else {
            read_u16_be(loca, g * 2).map(|v| v as usize * 2)
        }
    };
    let composite = (0..usize::from(f.num_glyphs()))
        .find(|&g| {
            let (Some(s), Some(e)) = (offset_of(g), offset_of(g + 1)) else { return false };
            s < e && read_i16_be(glyf, s) == Some(-1)
        })
        .expect("EBGaramond has no composite glyph") as u16;

    for mode in [HintMode::Subpixel, HintMode::Classic] {
        assert!(
            f.hinted_glyph(composite, 16.0, &[], mode).is_none(),
            "the interpreter hinted composite {composite} under {mode:?}",
        );
    }
    for mode in [HintMode::Auto, HintMode::AutoForce] {
        let out = f
            .hinted_glyph(composite, 16.0, &[], mode)
            .unwrap_or_else(|| panic!("the autohinter declined composite {composite} under {mode:?}"));
        assert!(!out.x.is_empty(), "{mode:?}: a hinted composite with no points");
        assert_eq!(out.x.len(), out.y.len());
    }
}

#[test]
fn each_hinting_mode_tries_its_own_set_of_strategies() {
    use daegun::HintMode;

    let cff = font(STIX);
    let (gid, hints) = (0..cff.num_glyphs())
        .filter_map(|g| cff.cff_hints(g).map(|h| (g, h)))
        .find(|(_, h)| !h.stems.is_empty())
        .expect("STIX2Math declares no stems anywhere");
    assert!(!hints.stems.is_empty());

    for mode in [HintMode::Subpixel, HintMode::Classic] {
        assert!(
            cff.hinted_glyph(gid, 16.0, &[], mode).is_none(),
            "{mode:?} reached CFF's stems; the interpreter modes stop at the bytecode attempt",
        );
    }
    for mode in [HintMode::Auto, HintMode::AutoForce] {
        assert!(
            cff.hinted_glyph(gid, 16.0, &[], mode).is_some(),
            "{mode:?} hinted nothing on a CFF face that declares stems",
        );
    }
    assert!(cff.hinted_glyph(gid, 16.0, &[], HintMode::None).is_none());

    let glyf = font(GARAMOND);
    let h = glyf.glyph_id('H' as u32).expect("EBGaramond has H");
    assert!(
        glyf.hinted_glyph(h, 16.0, &[], HintMode::Subpixel).is_none(),
        "EBGaramond gained hinting bytecode; pick another fixture",
    );
    assert!(glyf.hinted_glyph(h, 16.0, &[], HintMode::Auto).is_some());

    let hinted = font("test-fixtures/hinted.ttf");
    assert!(
        hinted.hinted_glyph(1, 16.0, &[], HintMode::Subpixel).is_some(),
        "the hinted fixture lost its bytecode",
    );
}

#[test]
fn the_vertical_advance_agrees_with_shaping() {
    let f = font(GARAMOND);
    assert!(!f.has_table("vmtx"), "EBGaramond gained vmtx; pick another fixture");
    let gid = f.glyph_id('H' as u32).expect("EBGaramond has H");

    let advance = f.vertical_advance(gid, &[]);
    assert!(advance > 0, "a glyph set vertically advanced by nothing");
    let shaped = f.shape("H", &[], true).expect("vertical shaping");
    assert_eq!(shaped.glyphs.len(), 1, "H shaped to more than one glyph");
    assert!(
        (f64::from(advance) - shaped.advances[0]).abs() <= 1.0,
        "vertical_advance says {advance}, shaping says {}",
        shaped.advances[0],
    );
    assert_eq!(
        i32::try_from(advance).expect("fits"),
        f.ascender() - f.descender(),
        "the fallback is not ascender-to-descender",
    );

    let cjk = font("source-han-sans/SourceHanSansJP-VF.otf");
    assert!(cjk.has_table("vmtx"), "the CJK fixture lost vmtx");
    let cg = cjk.glyph_id('あ' as u32).expect("SourceHanSans has あ");
    let cjk_advance = cjk.vertical_advance(cg, &[]);
    assert!(cjk_advance > 0);
    let cjk_shaped = cjk.shape("あ", &[], true).expect("vertical shaping");
    assert!(
        (f64::from(cjk_advance) - cjk_shaped.advances[0]).abs() <= 1.0,
        "vertical_advance says {cjk_advance}, shaping says {}",
        cjk_shaped.advances[0],
    );
    assert_ne!(
        i32::try_from(cjk_advance).expect("fits"),
        cjk.ascender() - cjk.descender(),
        "the CJK face's stated advance happens to equal the fallback, so this proves nothing",
    );
}

#[test]
fn a_shaped_run_places_its_marks() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");

    for (name, text, above) in [
        ("fatha", "\u{0628}\u{064E}", true),
        ("kasra", "\u{0628}\u{0650}", false),
    ] {
        let run = f.shape(text, &[], false).expect("Arabic shapes");
        assert_eq!(run.offsets.len(), run.glyphs.len(), "{name}: offsets are not parallel");
        assert_eq!(run.advances.len(), run.glyphs.len());
        assert_eq!(run.clusters.len(), run.glyphs.len());
        assert_eq!(run.glyphs.len(), 2, "{name}: expected a base and a mark");

        let mark = run
            .advances
            .iter()
            .position(|&a| a == 0.0)
            .unwrap_or_else(|| panic!("{name}: no zero-advance mark, so nothing needs placing"));
        let (dx, dy) = run.offsets[mark];
        assert!(
            dx != 0.0 || dy != 0.0,
            "{name}: the mark has neither advance nor offset, so it cannot be placed at all",
        );

        let (_, y0, _, y1) = f
            .glyph_bounds(run.glyphs[mark], &[])
            .unwrap_or_else(|| panic!("{name}: the mark has no outline"));
        let (placed_low, placed_high) = (y0 + dy, y1 + dy);
        if above {
            assert!(
                placed_low > 0.0,
                "{name} should sit above the baseline, its ink is at {placed_low}..{placed_high}",
            );
        } else {
            assert!(
                placed_high < 0.0,
                "{name} should sit below the baseline, its ink is at {placed_low}..{placed_high}",
            );
        }
    }

    let latin = font(GARAMOND);
    for text in ["AVA", "o\u{0301}"] {
        let run = latin.shape(text, &[], false).expect("Latin shapes");
        assert_eq!(run.offsets.len(), run.glyphs.len());
        assert!(
            run.offsets.iter().all(|&(x, y)| x == 0.0 && y == 0.0),
            "{text:?} produced offsets with nothing to place: {:?}",
            run.offsets,
        );
        assert!(run.advances.iter().all(|&a| a > 0.0));
    }

    let vertical = latin.shape("AV", &[], true).expect("vertical shaping");
    assert_eq!(vertical.offsets.len(), vertical.glyphs.len());
    assert!(vertical.advances.iter().all(|&a| a > 0.0), "vertical advances are positive magnitudes");
}

#[test]
fn a_run_says_where_it_may_be_cut() {
    let f = font(GARAMOND);

    let (mut flagged, mut checked) = (0usize, 0usize);
    for text in ["HIn", "nun", "HH", "mm", "AVA", "ox"] {
        let whole = f.shape(text, &[], false).expect("shapes");
        assert_eq!(whole.unsafe_to_break.len(), whole.glyphs.len(), "{text:?}: not parallel");
        assert_eq!(
            whole.glyphs.len(),
            text.chars().count(),
            "{text:?} does not shape one glyph per character, so the cut indices do not line up",
        );
        assert!(!whole.unsafe_to_break[0], "{text:?}: the run's own start is flagged");

        for cut in 1..text.chars().count() {
            let (head, tail) = text.split_at(cut);
            let a = f.shape(head, &[], false).expect("head shapes");
            let b = f.shape(tail, &[], false).expect("tail shapes");
            let spliced: Vec<u16> = a.glyphs.iter().chain(b.glyphs.iter()).copied().collect();
            let split_width: f64 =
                a.advances.iter().chain(b.advances.iter()).sum();
            let whole_width: f64 = whole.advances.iter().sum();

            checked += 1;
            if whole.unsafe_to_break[cut] {
                flagged += 1;
                continue;
            }
            assert_eq!(
                spliced, whole.glyphs,
                "{text:?} cut at {cut} is marked safe but shaped to different glyphs",
            );
            assert!(
                (split_width - whole_width).abs() < 0.01,
                "{text:?} cut at {cut} is marked safe but the width moved by {}",
                split_width - whole_width,
            );
        }
    }
    assert!(checked >= 8, "only {checked} cut positions examined");
    assert!(flagged > 0, "nothing was ever flagged, so the safe cases prove nothing");

    let ava = f.shape("AVA", &[], false).expect("shapes");
    assert!(ava.unsafe_to_break[1], "the A/V pair is no longer flagged; pick another fixture");
    let lost: f64 = f.shape("A", &[], false).expect("A").advances.iter().sum::<f64>()
        + f.shape("VA", &[], false).expect("VA").advances.iter().sum::<f64>()
        - ava.advances.iter().sum::<f64>();
    assert!(lost > 1.0, "splitting a flagged cut changed nothing, so the flag means nothing");
}

#[test]
fn advance_widths_is_the_glyph_metric_and_not_the_shaped_width() {
    let f = font(GARAMOND);
    assert_eq!(f.upm(), 1000, "EBGaramond's em changed; the exactness below depends on it");
    let h = f.glyph_id('H' as u32).expect("H");
    let single = f.shape("H", &[], false).expect("shapes");
    assert_eq!(single.glyphs.len(), 1);
    assert!(
        (f.advance_widths(&[h], &[])[0] - single.advances[0]).abs() < 0.01,
        "one unkerned glyph should measure the same both ways",
    );

    let text = "The quick brown fox jumps over the lazy dog";
    let gids: Vec<u16> = text.chars().filter_map(|c| f.glyph_id(c as u32)).collect();
    let summed: f64 = f.advance_widths(&gids, &[]).iter().sum();
    let run = f.shape(text, &[], false).expect("shapes");
    let shaped: f64 = run.advances.iter().sum();
    assert!(
        (summed - shaped).abs() > 10.0,
        "the two agree over a whole line, so this fixture kerns nothing and proves nothing",
    );
    assert!(
        (f.measure_width(text, &[], 1000.0) - shaped).abs() < 0.01,
        "measure_width disagrees with shaping",
    );

    let v = font(INTER);
    assert_ne!(v.upm(), 1000, "Inter's em changed; the rounding below depends on it");
    let o = v.glyph_id('o' as u32).expect("Inter has o");
    let metric = v.advance_widths(&[o], &[])[0];
    let shaped_one = v.shape("o", &[], false).expect("shapes").advances[0];
    assert_eq!(metric, metric.round(), "the metrics path is integer-valued: {metric}");
    assert!(
        (metric - shaped_one).abs() < 1.0 && metric != shaped_one,
        "expected a sub-unit rounding difference, got {metric} against {shaped_one}",
    );
}

#[test]
fn typographic_metrics_survives_a_missing_os2() {
    let f = font(GARAMOND);
    let intact = f.typographic_metrics(&[]).expect("EBGaramond states these");
    assert!(intact.underline_position < 0, "underline sits below the baseline");
    assert!(intact.underline_thickness > 0, "an underline of no thickness draws nothing");
    assert!(intact.x_height > 0 && intact.strikeout_size > 0, "the OS/2 block is present here");

    let mut tables = f.instance_tables(&[]).expect("a static font yields its tables");
    assert!(tables.contains_key("OS/2") && tables.contains_key("post"), "fixture lost a table");

    tables.remove("OS/2");
    let g = Font::from_bytes(&daegun::build_font(&tables)).expect("parses without OS/2");
    let m = g.typographic_metrics(&[]).expect("post alone is still an answer");
    assert_eq!(
        (m.underline_position, m.underline_thickness),
        (intact.underline_position, intact.underline_thickness),
        "dropping OS/2 changed the underline, which post states and OS/2 does not",
    );
    assert_eq!(
        (m.x_height, m.strikeout_size, m.strikeout_position),
        (0, 0, 0),
        "an absent OS/2 field must read zero rather than carry over",
    );
    assert_eq!(m.subscript, Default::default(), "absent sub/superscript boxes are zero");

    let mut short = f.instance_tables(&[]).expect("tables");
    let os2 = short["OS/2"][..28].to_vec();
    short.insert("OS/2".into(), std::borrow::Cow::Owned(os2));
    let h = Font::from_bytes(&daegun::build_font(&short)).expect("parses with a short OS/2");
    let m = h.typographic_metrics(&[]).expect("post alone is still an answer");
    assert_eq!(m.underline_thickness, intact.underline_thickness, "short OS/2 lost the underline");
    assert_eq!(m.strikeout_size, 0, "a truncated OS/2 block reported a strikeout anyway");

    let mut bare = f.instance_tables(&[]).expect("tables");
    bare.remove("OS/2");
    bare.remove("post");
    if let Ok(b) = Font::from_bytes(&daegun::build_font(&bare)) {
        assert!(b.typographic_metrics(&[]).is_none(), "a font stating none of these reported some");
    }
}

#[test]
fn the_types_a_caller_debugs_with_can_be_debugged() {
    let f = font(GARAMOND);

    let axes = f.axes();
    assert!(!axes.is_empty(), "EBGaramond is variable; the rest of this needs an axis");
    assert_eq!(axes, axes.clone(), "FvarAxis compares unequal to a clone of itself");
    assert!(
        format!("{axes:?}").contains(&axes[0].tag),
        "an axis debug-prints without naming its tag, which is the only part a reader needs",
    );

    let opts = daegun::OutlineOptions::default().with_oblique(0.25);
    let shown = format!("{opts:?}");
    assert!(shown.contains("oblique") && shown.contains("hinting"), "OutlineOptions hid its fields");

    let gray = format!("{:?}", daegun::SubpixelLayout::grayscale());
    let rgb = format!("{:?}", daegun::SubpixelLayout::horizontal(daegun::StripeOrder::Rgb));
    assert!(gray.len() < 200, "the layout dumped its weight table: {gray}");
    assert_ne!(gray, rgb, "two different layouts debug-print identically");

    let gid = f.glyph_id('A' as u32).expect("A");
    let hinted = f.hinted_glyph(gid, 16.0, &[], daegun::HintMode::Auto).expect("A hints");
    assert_eq!(hinted, hinted.clone(), "HintedOutline compares unequal to its own clone");
    assert!(!format!("{hinted:?}").is_empty());
}

#[test]
fn shape_bidi_tells_each_run_what_surrounds_it() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");

    let with = f.shape_with_options("سل", &[], false,
        &daegun::ShapeOptions { after: "ام", ..Default::default() }).expect("shapes");
    let without = f.shape("سل", &[], false).expect("shapes");
    assert_ne!(
        with.glyphs, without.glyphs,
        "a following letter did not change the joining form, so this font cannot show the bug",
    );

    let text = "سلام\u{200E}سلام";
    let runs = f.shape_bidi(text, &[], None).expect("bidi shapes");
    assert_eq!(runs.len(), 3, "LRM no longer splits the Arabic; the case below tests nothing");

    let chars: Vec<char> = text.chars().collect();
    for (i, r) in runs.iter().enumerate() {
        let (lo, hi) = (r.chars[0], r.chars[r.chars.len() - 1]);
        let slice: String = chars[lo..=hi].iter().collect();
        let reference = f.shape_with_options(&slice, &[], false, &daegun::ShapeOptions {
            before: &chars[..lo].iter().collect::<String>(),
            after: &chars[hi + 1..].iter().collect::<String>(),
            ..Default::default()
        }).expect("the reference shapes");
        assert_eq!(
            r.run.glyphs, reference.glyphs,
            "run {i} of {text:?} was shaped without knowing its neighbors",
        );
    }
}

#[test]
fn shape_options_composes_features_with_fragment_context() {
    let g = font(GARAMOND);

    let on = g.shape_with_options("fi", &[], false, &daegun::ShapeOptions::default()).expect("shapes");
    let off = g.shape_with_options("fi", &[], false,
        &daegun::ShapeOptions { features: &[("liga", 0)], ..Default::default() }).expect("shapes");
    assert_ne!(on.glyphs, off.glyphs, "turning liga off changed nothing, so nothing is proven below");
    assert_eq!(on.glyphs.len(), 1, "EBGaramond should form an fi ligature");
    assert_eq!(
        off.glyphs,
        g.shape_with_features("fi", &[], false, None, &[("liga", 0)]).expect("shapes").glyphs,
        "ShapeOptions::features disagrees with shape_with_features on the same request",
    );

    let m = font(STIX);
    let plain = m.shape_with_options("x", &[], false, &daegun::ShapeOptions::default()).expect("shapes");
    let ssty = m.shape_with_options("x", &[], false, &daegun::ShapeOptions {
        features: &[("ssty", 1)], script: Some("math"), ..Default::default()
    }).expect("shapes");
    assert_ne!(ssty.glyphs, plain.glyphs, "ssty under the math script selected nothing");
    assert_eq!(
        ssty.glyphs,
        m.shape_with_features("x", &[], false, Some("math"), &[("ssty", 1)]).expect("shapes").glyphs,
        "ShapeOptions::script disagrees with shape_with_features",
    );

    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let joined = f.shape_with_options("سل", &[], false,
        &daegun::ShapeOptions { after: "ام", ..Default::default() }).expect("shapes");
    let alone = f.shape("سل", &[], false).expect("shapes");
    assert_ne!(joined.glyphs, alone.glyphs, "context changes nothing here; the case below is vacuous");

    let both = f.shape_with_options("سل", &[], false, &daegun::ShapeOptions {
        after: "ام", features: &[("liga", 0)], ..Default::default()
    }).expect("shapes");
    assert_eq!(
        both.glyphs, joined.glyphs,
        "asking for a feature dropped the fragment context that was asked for alongside it",
    );
}

#[test]
fn layout_shapes_each_run_at_its_own_bidi_level() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let opts = daegun::LayoutOptions { max_inline_size: f64::INFINITY, ..Default::default() };

    let text = "سلام (a) سلام";
    let lay = f.layout(text, &[], &opts).expect("lays out");
    let run = lay.lines[0].runs.iter()
        .find(|r| r.chars == (7, 9))
        .expect("the `) ` piece between the Latin word and the Arabic");
    assert_eq!(run.level % 2, 1, "that piece is at an odd level, or the case below is vacuous");
    assert!(
        run.run.glyphs.contains(&9),
        "the closing parenthesis was not mirrored: {:?} still holds gid 10",
        run.run.glyphs,
    );
    assert!(
        run.run.clusters.windows(2).all(|w| w[0] >= w[1]),
        "an odd-level run came back in logical order: clusters {:?}",
        run.run.clusters,
    );

    for text in [
        "سلام !? سلام", "سلام (a) سلام", "سلام ۱۲ سلام", "سلام .، سلام",
        "سلام [b] سلام", "abc (د) abc", "سلام {a} (b) سلام",
    ] {
        let bidi: Vec<u16> = f.shape_bidi(text, &[], None).expect("bidi")
            .iter().flat_map(|r| r.run.glyphs.clone()).collect();
        let laid: Vec<u16> = f.layout(text, &[], &opts).expect("lays out")
            .lines[0].runs.iter().flat_map(|r| r.run.glyphs.clone()).collect();
        assert_eq!(laid, bidi, "layout and shape_bidi disagree on {text:?}");
    }
}

#[test]
fn typographic_metrics_vary_by_axis_the_way_instancing_does() {
    let inter = font(INTER);
    let axis = |tag: &str| inter.axes().into_iter().find(|a| a.tag == tag)
        .unwrap_or_else(|| panic!("Inter varies {tag}"));
    let at = |tag: &str, v: f64| inter.typographic_metrics(&[(tag, v)]).expect("Inter states these");

    let (wght, opsz) = (axis("wght"), axis("opsz"));
    let (thin, black) = (at("wght", wght.min), at("wght", wght.max));
    assert!(
        black.underline_thickness > thin.underline_thickness * 3,
        "the underline barely moved with the weight: {} against {}",
        thin.underline_thickness, black.underline_thickness,
    );

    assert!(
        at("opsz", opsz.max).x_height < at("opsz", opsz.min).x_height,
        "x-height did not fall as the optical size rose",
    );

    for rel in [
        GARAMOND, INTER,
        "source-serif/SourceSerif4Variable-Roman.otf",
        "source-han-sans/SourceHanSansJP-VF.otf",
    ] {
        let f = font(rel);
        let axes = f.axes();
        assert!(!axes.is_empty(), "{rel} is not variable; it proves nothing here");
        for t in [0.0, 0.5, 1.0] {
            let owned: Vec<(String, f64)> = axes.iter()
                .map(|a| (a.tag.clone(), a.min + (a.max - a.min) * t))
                .collect();
            let at: Vec<(&str, f64)> = owned.iter().map(|(k, v)| (k.as_str(), *v)).collect();
            let instanced = Font::from_bytes(&f.instance(&at)).expect("the instance reopens");
            assert_eq!(
                f.typographic_metrics(&at),
                instanced.typographic_metrics(&[]),
                "{rel} at t={t}: asking at the axes disagrees with instancing to them",
            );
        }
    }
}

#[test]
fn subset_glyph_ids_are_readable_whatever_the_outline_format() {
    let mut saw_map = false;
    let mut saw_empty = false;

    for rel in [
        GARAMOND, INTER,
        STIX,
        "source-serif/SourceSerif4Variable-Roman.otf",
    ] {
        let f = font(rel);
        let gids: Vec<u16> = "Hamburg".chars().filter_map(|c| f.glyph_id(c as u32)).collect();
        assert!(gids.len() >= 5, "{rel} did not map the test text");

        let sub = f.subset(&gids, &[]).expect("subsets");
        let out = Font::from_bytes(&sub.ttf).expect("the subset reopens");
        if sub.gid_map.is_empty() { saw_empty = true } else { saw_map = true }

        for &old in &gids {
            let new = sub.new_gid(old).unwrap_or_else(|| panic!("{rel}: requested gid {old} was dropped"));
            assert_eq!(
                f.glyph_bounds(old, &[]),
                out.glyph_bounds(new, &[]),
                "{rel}: gid {old} -> {new} is not the same glyph after subsetting",
            );
        }

        assert_eq!(sub.new_gid(0), Some(0), "{rel}: .notdef did not survive");
    }

    assert!(saw_map && saw_empty, "the fixtures no longer cover both subsetting strategies");

    let f = font(GARAMOND);
    let high = f.glyph_id('z' as u32).filter(|&g| g > 8).expect("a glyph well past .notdef");
    let sub = f.subset(&[high], &[]).expect("subsets");
    assert_ne!(sub.new_gid(high), Some(high), "the glyf subset did not compact its glyph ids");
}

// CFF2 with no fvar is a valid static font. Made by dropping a variable font's fvar, its charstrings
// still blend, so it has to draw and subset exactly as that font's default.
#[test]
fn a_static_cff2_font_is_its_default_instance() {
    let variable = font(SOURCE_SERIF);
    let tables: std::collections::BTreeMap<String, std::borrow::Cow<[u8]>> = variable
        .table_tags()
        .into_iter()
        .filter(|&tag| tag != "fvar")
        .map(|tag| (tag.to_string(), std::borrow::Cow::Borrowed(variable.table(tag).expect("listed"))))
        .collect();
    let f = Font::from_vec(daegun::build_font(&tables)).expect("the static font parses");
    assert!(f.has_table("CFF2") && !f.is_variable(), "the fixture did not become a static CFF2 font");

    let gids: Vec<u16> = "Hamburg".chars().filter_map(|c| f.glyph_id(c as u32)).collect();
    assert!(gids.len() >= 5, "the static font did not map the test text");
    for &g in &gids {
        let (mut got, mut want) = (daegun::Path::default(), daegun::Path::default());
        f.outline_glyph_instanced(g, &[], &mut got).expect("a static CFF2 glyph draws");
        variable.outline_glyph_instanced(g, &[], &mut want).expect("the variable glyph draws");
        assert_eq!(got, want, "gid {g} is not the variable font's default");
    }

    let sub = f.subset(&gids, &[]).expect("a static CFF2 font subsets");
    let out = Font::from_bytes(&sub.ttf).expect("the subset reopens");
    for &old in &gids {
        let new = sub.new_gid(old).unwrap_or_else(|| panic!("requested gid {old} was dropped"));
        assert_eq!(f.glyph_bounds(old, &[]), out.glyph_bounds(new, &[]), "gid {old} changed in the subset");
    }
    assert_eq!(
        f.glyph_closure(&gids, &[]).expect("a static CFF2 closure"),
        variable.glyph_closure(&gids, &[]).expect("a variable CFF2 closure"),
    );
    f.subset_text("Hamburg", &[]).expect("a static CFF2 font subsets by text");

    // Drawing and subsetting convert to CFF inside; a static font asked for its instance is itself.
    let instance = f.instance_tables(&[]).expect("an answer");
    assert!(instance.contains_key("CFF2") && !instance.contains_key("CFF "), "the static font came back converted");
    assert_eq!(f.instance(&[]), daegun::build_font(&tables), "instance() is not the font as stored");
}

// A .notdef built from another glyph keeps that glyph in a subset, as the closure says it must.
#[test]
fn a_composite_notdef_keeps_its_components() {
    let base = font("test-fixtures/hinted.ttf");
    let (a, f) = (base.glyph_id('A' as u32).expect("A"), base.glyph_id('F' as u32).expect("F"));
    let mut notdef: Vec<u8> = Vec::new();
    for v in [-1i16, 0, 0, 300, 700, 0x0003, f as i16, 0, 0] {
        notdef.extend(v.to_be_bytes());
    }
    let font = crate::prepared::hinted_with(0, &notdef);
    assert!(closure_of(&font, &[a]).contains(&f), "the closure lost .notdef's component");
    let sub = font.subset(&[a], &[]).expect("subsets");
    assert!(sub.new_gid(f).is_some(), "the subset dropped .notdef's component");
}

// An fvar whose header claims axes that do not parse (an axis record 10 bytes long, not 20) leaves the
// font static to the instancer, and so it comes back as stored too, CFF2 and fvar both.
#[test]
fn a_cff2_font_whose_axes_do_not_parse_is_its_own_instance() {
    let variable = font(SOURCE_SERIF);
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> = variable
        .table_tags()
        .into_iter()
        .map(|tag| (tag.to_string(), variable.table(tag).expect("listed").to_vec()))
        .collect();
    daegun::bytes::write_u16_be(tables.get_mut("fvar").expect("fvar"), 10, 10);
    let f = Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses");
    assert!(f.is_variable() && f.axes().is_empty(), "the fixture is not a variable font with unreadable axes");
    let instance = f.instance_tables(&[("wght", 900.0)]).expect("an answer");
    assert!(instance.contains_key("CFF2") && instance.contains_key("fvar"), "the font came back converted");
    assert_eq!(f.instance(&[("wght", 900.0)]), daegun::build_font(&tables), "instance() is not the font as stored");

    // And to draw, hint and subset it is its default instance, as a font with no fvar is.
    let gids: Vec<u16> = "Hamburg".chars().filter_map(|c| f.glyph_id(c as u32)).collect();
    for &g in &gids {
        let (mut got, mut want) = (daegun::Path::default(), daegun::Path::default());
        f.outline_glyph(g, &mut got).expect("the glyph draws");
        variable.outline_glyph(g, &mut want).expect("the variable glyph draws");
        assert_eq!(got, want, "gid {g} is not the variable font's default");
    }
    assert!(f.hinted_glyph(gids[0], 16.4, &[], daegun::HintMode::Auto).is_some(), "the glyph did not hint");
    f.subset(&gids, &[]).expect("the font subsets");
}

// Both outline calls draw CFF2 through its CFF conversion. The default instance, written out as CFF
// and opened as a font of its own, is a second route to the same outline.
#[test]
fn outline_glyph_draws_a_cff2_font_as_its_default() {
    for rel in [SOURCE_SERIF, "source-han-sans/SourceHanSansJP-VF.otf"] {
        let f = font(rel);
        let defaults: Vec<(String, f64)> = f.axes().iter().map(|a| (a.tag.clone(), a.default)).collect();
        let at: Vec<(&str, f64)> = defaults.iter().map(|(t, v)| (t.as_str(), *v)).collect();
        let cff = Font::from_bytes(&f.instance(&at)).expect("the instance reopens");
        assert!(cff.has_table("CFF ") && !cff.has_table("CFF2"), "{rel}: the instance is not CFF");
        let mut drawn = 0;
        for gid in 1..40 {
            let (mut got, mut want) = (daegun::Path::default(), daegun::Path::default());
            assert_eq!(f.outline_glyph(gid, &mut got), cff.outline_glyph(gid, &mut want), "{rel} gid {gid}");
            assert_eq!(got, want, "{rel} gid {gid}: outline_glyph is not the default instance's outline");
            drawn += usize::from(!got.is_empty());
        }
        assert!(drawn > 30, "{rel}: only {drawn} of 39 glyphs drew anything");
    }
}

// CFF2 allows 513 operands to an operator and Type 2 only 48, so the conversion writes a longer one as
// several. Source Han Sans has operators past 48 from gid 1024 on: those and a sample of the rest draw.
#[test]
fn a_cff2_font_with_long_operators_draws_every_glyph() {
    let f = font("source-han-sans/SourceHanSansJP-VF.otf");
    let gids: Vec<u16> = (1024..1040).chain([2233]).chain((0..f.num_glyphs()).step_by(97)).collect();
    let lost: Vec<u16> = gids
        .into_iter()
        .filter(|&g| f.outline_glyph(g, &mut daegun::Path::default()).is_none())
        .collect();
    assert!(lost.is_empty(), "{} glyphs did not draw: {lost:?}", lost.len());
}

#[test]
fn a_glyph_the_font_does_not_have_is_declined_the_same_way_everywhere() {
    for rel in [GARAMOND, STIX, "bungee-tint/BungeeTint-Regular.ttf"] {
        let f = font(rel);
        let n = f.num_glyphs();
        assert!(n > 0, "{rel} has no glyphs");

        for gid in [n, n.saturating_add(1), u16::MAX] {
            assert_eq!(f.advance_widths(&[gid], &[]), vec![0.0], "{rel}: advance_widths({gid})");
            assert_eq!(
                f.vertical_advance(gid, &[]), 0,
                "{rel}: vertical_advance({gid}) invented an advance for a glyph that is not there, \
                 where advance_widths reports zero",
            );
            assert!(f.glyph_bounds(gid, &[]).is_none(), "{rel}: glyph_bounds({gid})");
            assert!(f.glyph_name(gid).is_none(), "{rel}: glyph_name({gid})");
            assert!(f.vertical_origin(gid, &[]).is_none(), "{rel}: vertical_origin({gid})");
            assert_eq!(f.glyph_quads(gid, &[]), Err(daegun::QuadError::NoOutline), "{rel}: glyph_quads({gid})");
            let mut sink = daegun::Path::default();
            let opts = daegun::OutlineOptions::default();
            assert!(f.prepared_outline(gid, 16.0, &[], &opts, &mut sink).is_none(), "{rel}: prepared_outline({gid})");
            assert!(sink.is_empty(), "{rel}: prepared_outline({gid}) drew before declining");
            assert!(f.colr_scene(gid, &[], 0).is_none(), "{rel}: colr_scene({gid})");
            assert!(f.hinted_glyph(gid, 16.0, &[], daegun::HintMode::Auto).is_none(), "{rel}: hinted");
            assert!(f.ligature_carets(gid, &[], false).is_empty(), "{rel}: ligature_carets({gid})");
            assert!(f.colr_layers(gid).is_none(), "{rel}: colr_layers({gid})");
            assert!(f.colr_v1_paint(gid, &[], 0).is_none(), "{rel}: colr_v1_paint({gid})");
            assert!(f.math_glyph_variants(gid, true).is_none(), "{rel}: math_glyph_variants({gid})");
        }

        let last = n - 1;
        assert!(
            f.vertical_advance(last, &[]) > 0,
            "{rel}: the last real glyph {last} lost its vertical advance to the range guard",
        );
    }
}

#[test]
fn no_method_hands_back_a_glyph_id_the_font_cannot_service() {
    let f = font(GARAMOND);
    let real = f.num_glyphs();
    assert!(real > 100, "the fixture shrank; this test needs room to cut");

    const KEPT: u16 = 40;
    let mut tables = f.instance_tables(&[]).expect("a static font yields its tables");
    let mut maxp = tables["maxp"].to_vec();
    maxp[4..6].copy_from_slice(&KEPT.to_be_bytes());
    tables.insert("maxp".into(), std::borrow::Cow::Owned(maxp));
    let g = Font::from_bytes(&daegun::build_font(&tables)).expect("still parses");
    assert_eq!(g.num_glyphs(), KEPT, "the maxp edit did not take");

    let out_of_range = f.coverage().iter().filter(|&&(_, id)| id >= KEPT).count();
    assert!(out_of_range > 0, "no codepoint maps past {KEPT}; the case below is vacuous");
    assert!(
        g.coverage().iter().all(|&(_, id)| id < KEPT),
        "coverage returned a glyph id past the {KEPT} the font declares",
    );

    for (cp, id) in f.coverage() {
        if id < KEPT { continue }
        assert_eq!(g.glyph_id(cp), None, "glyph_id(U+{cp:04X}) returned {id}, which is past {KEPT}");
        assert!(!g.has_glyph(cp), "has_glyph(U+{cp:04X}) is true for a glyph nothing can service");
    }

    for text in ["Hamburgefonstiv", "fi ffl office", "AV To 123", "\u{0}\u{FFFD}"] {
        for vertical in [false, true] {
            let Some(run) = g.shape(text, &[], vertical) else { continue };
            for (i, &id) in run.glyphs.iter().enumerate() {
                assert!(
                    id == 0 || id < KEPT,
                    "shape({text:?}, vertical={vertical}) emitted {id} at {i}, past the {KEPT} \
                     glyphs the font declares",
                );
            }
        }
    }

    assert!(f.coverage().iter().any(|&(_, id)| id >= KEPT), "sanity: the real font is unbounded by KEPT");
    assert!(f.coverage().iter().all(|&(_, id)| id < real), "the real font's cmap is already in range");
}

#[test]
fn instancing_does_not_depend_on_the_order_the_axes_were_listed_in() {
    let f = font("source-serif/SourceSerif4Variable-Roman.otf");
    let axes = f.axes();
    assert!(axes.len() >= 2, "the fixture needs two axes to reorder");
    assert!(f.has_table("CFF2"), "the fixture is no longer CFF2; this tests the name it generates");

    let forward: Vec<(&str, f64)> = axes.iter().map(|a| (a.tag.as_str(), a.default)).collect();
    let mut reversed = forward.clone();
    reversed.reverse();

    assert_eq!(f.instance(&forward), f.instance(&reversed), "instance depends on the axis order");
    assert_eq!(
        f.instance_tables(&forward),
        f.instance_tables(&reversed),
        "instance_tables depends on the axis order",
    );

    for at in [&forward, &reversed] {
        let tables = f.instance_tables(at).expect("a variable font instances");
        assert_eq!(
            daegun::build_font(&tables),
            f.instance(at),
            "build_font(instance_tables) is not the font instance() produces",
        );
    }

    let g = font(INTER);
    let ga = g.axes();
    if ga.len() >= 2 {
        let fwd: Vec<(&str, f64)> = ga.iter().map(|a| (a.tag.as_str(), a.default)).collect();
        let mut rev = fwd.clone();
        rev.reverse();
        assert_eq!(g.instance_tables(&fwd), g.instance_tables(&rev), "glyf instancing reordered");
    }
}

#[test]
fn instance_tables_answers_exactly_as_instance_does() {
    let f = font(INTER);
    assert!(!f.axes().is_empty(), "the fixture is not variable; nothing below is meaningful");

    let mut tables: std::collections::BTreeMap<String, Vec<u8>> = f.table_tags().into_iter()
        .filter_map(|t| f.table(t).map(|d| (t.to_string(), d.to_vec())))
        .collect();
    let fvar = tables.get_mut("fvar").expect("Inter carries fvar");
    fvar[8..10].copy_from_slice(&0u16.to_be_bytes());
    let broken = Font::from_bytes(&daegun::build_font(&tables)).expect("still parses");
    assert!(broken.has_table("fvar") && broken.axes().is_empty(), "the fvar edit did not take");
    // An fvar of no axes still parses, so the instancer instances it, as 1.1.7 did: its variation tables go.
    let instanced = broken.instance_tables(&[]).expect("always answers");
    assert!(!instanced.contains_key("fvar") && !instanced.contains_key("gvar"), "an fvar of no axes came back as stored");

    for probe in [&broken, &f] {
        let at: Vec<(&str, f64)> = Vec::new();
        let tables = probe.instance_tables(&at).expect("always answers");
        assert_eq!(
            daegun::build_font(&tables),
            probe.instance(&at),
            "instance_tables and instance disagree about the same font at the same axes",
        );
    }

    let s = font(STIX);
    assert!(!s.has_table("fvar"), "the static fixture gained an fvar");
    let st = s.instance_tables(&[]).expect("a static font yields its own tables");
    assert!(st.contains_key("head") && st.contains_key("cmap"), "the static path lost tables");
    assert_eq!(daegun::build_font(&st), s.instance(&[]), "static: the two routes disagree");
}

#[test]
fn a_font_can_be_asked_what_features_it_declares() {
    let ar = font("scheherazade-new/ScheherazadeNew-Regular.ttf");

    let scripts = ar.script_tags();
    assert!(scripts.contains(&"arab".to_string()), "an Arabic face declares arab: {scripts:?}");
    let langs = ar.language_tags("arab");
    assert!(langs.contains(&"SND ".to_string()), "Scheherazade declares Sindhi: {langs:?}");
    assert!(langs.iter().all(|l| l.len() == 4), "a language tag is not four bytes: {langs:?}");

    let feats = ar.feature_tags(Some("arab"), None);
    for required in ["init", "medi", "fina", "rlig"] {
        assert!(feats.contains(&required.to_string()), "no {required} for arab: {feats:?}");
    }
    assert!(feats.windows(2).all(|w| w[0] < w[1]), "not sorted and deduplicated: {feats:?}");
    assert!(feats.iter().all(|f| f.len() == 4), "a feature tag is not four bytes");

    let latin = font(GARAMOND);
    let offered = latin.feature_tags(None, None);
    let plain = latin.shape("fi", &[], false).expect("shapes").glyphs.clone();
    assert!(offered.contains(&"kern".to_string()), "EBGaramond declares kern: {offered:?}");
    assert!(!offered.contains(&"zzzz".to_string()), "no font declares zzzz");

    let off = latin.shape_with_features("fi", &[], false, None, &[("liga", 0)])
        .expect("shapes").glyphs;
    assert_ne!(off, plain, "liga is declared and turning it off changed nothing");
    let bogus = latin.shape_with_features("fi", &[], false, None, &[("zzzz", 1)])
        .expect("shapes").glyphs;
    assert_eq!(bogus, plain, "an undeclared tag changed the shaping");

    let dv = font("noto-devanagari/NotoSansDevanagari.ttf");
    let dv_scripts = dv.script_tags();
    assert!(
        dv_scripts.contains(&"deva".to_string()) && dv_scripts.contains(&"dev2".to_string()),
        "both Devanagari models should be listed: {dv_scripts:?}",
    );

    assert!(ar.feature_tags(Some("zzzz"), None).is_empty(), "an undeclared script offered features");
    assert!(ar.language_tags("ab").is_empty(), "a malformed tag was padded rather than refused");
    assert_eq!(
        ar.feature_tags(Some("arab"), Some("nope")),
        ar.feature_tags(Some("arab"), None),
        "an undeclared language should report the default LangSys, as shaping would use it",
    );

    let emoji = font("noto-color-emoji/NotoColorEmoji.ttf");
    let _ = emoji.script_tags();
    let _ = emoji.feature_tags(None, None);
}

#[test]
fn the_other_two_glyph_flags_can_be_asked_for() {
    let ar = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let text = "سلام دنيا";

    let plain = ar.shape(text, &[], false).expect("shapes");
    assert!(plain.unsafe_to_concat.is_empty(), "reported without being asked");
    assert!(plain.safe_to_insert_tatweel.is_empty(), "reported without being asked");
    assert!(!plain.unsafe_to_break.is_empty(), "unsafe_to_break is always reported");

    let both = ar.shape_with_options(text, &[], false, &daegun::ShapeOptions {
        report_unsafe_to_concat: true,
        report_tatweel_positions: true,
        ..Default::default()
    }).expect("shapes");

    let n = both.glyphs.len();
    assert_eq!(both.unsafe_to_concat.len(), n, "unsafe_to_concat is not parallel");
    assert_eq!(both.safe_to_insert_tatweel.len(), n, "safe_to_insert_tatweel is not parallel");
    assert_eq!(both.glyphs, plain.glyphs, "asking for the flags changed the glyphs");
    assert_eq!(both.advances, plain.advances, "asking for the flags changed the advances");

    assert!(
        both.safe_to_insert_tatweel.iter().any(|&b| b),
        "no tatweel position in Arabic text: {:?}", both.safe_to_insert_tatweel,
    );
    assert!(
        both.unsafe_to_concat.iter().any(|&b| b),
        "nothing unsafe to concatenate in a joining script",
    );

    let only_concat = ar.shape_with_options(text, &[], false, &daegun::ShapeOptions {
        report_unsafe_to_concat: true, ..Default::default()
    }).expect("shapes");
    assert_eq!(only_concat.unsafe_to_concat.len(), n, "concat alone did not report");
    assert!(only_concat.safe_to_insert_tatweel.is_empty(), "tatweel reported unasked");

    let latin = font(GARAMOND);
    let l = latin.shape_with_options("office", &[], false, &daegun::ShapeOptions {
        report_tatweel_positions: true, ..Default::default()
    }).expect("shapes");
    assert_eq!(l.safe_to_insert_tatweel.len(), l.glyphs.len(), "not reported for Latin");
    assert!(!l.safe_to_insert_tatweel.iter().any(|&b| b), "Latin offered a kashida position");
}

#[test]
fn a_glyph_can_be_asked_what_gdef_calls_it() {
    let f = font(GARAMOND);
    assert!(f.has_table("GDEF"), "the fixture lost its GDEF");

    let mut seen = [0usize; 4];
    let mut marks = Vec::new();
    for gid in 0..f.num_glyphs() {
        match f.glyph_class(gid) {
            Some(daegun::GlyphClass::Base) => seen[0] += 1,
            Some(daegun::GlyphClass::Ligature) => seen[1] += 1,
            Some(daegun::GlyphClass::Mark) => { seen[2] += 1; marks.push(gid) }
            Some(daegun::GlyphClass::Component) => seen[3] += 1,
            None => {}
        }
    }
    assert!(seen[0] > 100, "EBGaramond should classify many bases, saw {}", seen[0]);
    assert!(seen[1] > 0, "EBGaramond declares ligatures, saw {}", seen[1]);
    assert!(!marks.is_empty(), "EBGaramond declares marks, saw none");

    for &m in marks.iter().take(40) {
        assert_eq!(
            f.advance_widths(&[m], &[])[0], 0.0,
            "gid {m} is classified a mark but advances",
        );
    }

    let none = font("noto-color-emoji/NotoColorEmoji.ttf");
    if !none.has_table("GDEF") {
        assert_eq!(none.glyph_class(1), None, "a font with no GDEF classified a glyph");
        assert_eq!(none.mark_attachment_class(1), 0, "a font with no GDEF has no attach class");
    }

    let past = f.num_glyphs();
    assert_eq!(f.glyph_class(past), None, "an out-of-range gid was classified");
    assert_eq!(f.glyph_class(u16::MAX), None, "u16::MAX was classified");
    assert_eq!(f.mark_attachment_class(past), 0, "an out-of-range gid has an attach class");

    let mut tables: std::collections::BTreeMap<String, Vec<u8>> = f.table_tags().into_iter()
        .filter_map(|t| f.table(t).map(|d| (t.to_string(), d.to_vec())))
        .collect();
    const KEPT: u16 = 40;
    tables.get_mut("maxp").expect("maxp")[4..6].copy_from_slice(&KEPT.to_be_bytes());
    let shrunk = Font::from_bytes(&daegun::build_font(&tables)).expect("parses");

    let outside: Vec<u16> = (KEPT..f.num_glyphs())
        .filter(|&g| f.glyph_class(g).is_some())
        .take(5)
        .collect();
    assert!(!outside.is_empty(), "no classified glyph past {KEPT}; the case below is vacuous");
    for g in outside {
        assert_eq!(shrunk.glyph_class(g), None, "gid {g} is past maxp and was still classified");
        assert_eq!(shrunk.mark_attachment_class(g), 0, "gid {g} is past maxp and has an attach class");
        assert!(shrunk.glyph_bounds(g, &[]).is_none(), "sanity: gid {g} should have no bounds either");
    }

    let run = f.shape("e\u{0301}", &[], false).expect("shapes");
    if run.glyphs.len() == 2 {
        let classes: Vec<_> = run.glyphs.iter().map(|&g| f.glyph_class(g)).collect();
        assert!(
            classes.contains(&Some(daegun::GlyphClass::Mark)),
            "a combining acute did not shape to a Mark-class glyph: {classes:?}",
        );
    }
}

#[test]
fn the_characters_that_draw_nothing_can_be_hidden_dropped_or_kept() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let text = "س\u{200D}لام";

    let by = |ig| f.shape_with_options(text, &[], false,
        &daegun::ShapeOptions { ignorables: ig, ..Default::default() }).expect("shapes");
    let hide = by(daegun::Ignorables::Hide);
    let remove = by(daegun::Ignorables::Remove);
    let preserve = by(daegun::Ignorables::Preserve);

    assert_eq!(f.shape(text, &[], false).expect("shapes").glyphs, hide.glyphs, "the default moved");

    assert_eq!(remove.glyphs.len(), hide.glyphs.len() - 1, "Remove did not drop the ZWJ");
    assert_eq!(preserve.glyphs.len(), hide.glyphs.len(), "Preserve changed the run length");
    for r in [&hide, &remove, &preserve] {
        assert_eq!(r.advances.len(), r.glyphs.len(), "arrays fell out of step");
        assert_eq!(r.clusters.len(), r.glyphs.len(), "clusters fell out of step");
    }

    assert_ne!(hide.glyphs, preserve.glyphs, "Hide and Preserve produced the same glyphs");

    let named = f.shape_with_options(text, &[], false, &daegun::ShapeOptions {
        invisible_glyph: Some(5), ..Default::default()
    }).expect("shapes");
    assert!(named.glyphs.contains(&5), "invisible_glyph was ignored: {:?}", named.glyphs);

    let joined = f.shape("لا", &[], false).expect("shapes");
    let broken_kept = f.shape("ل\u{200C}ا", &[], false).expect("shapes");
    let broken_dropped = f.shape_with_options("ل\u{200C}ا", &[], false, &daegun::ShapeOptions {
        ignorables: daegun::Ignorables::Remove, ..Default::default()
    }).expect("shapes");

    assert_ne!(
        broken_kept.glyphs.first(), joined.glyphs.first(),
        "the ZWNJ did not break the join, so the case below proves nothing",
    );
    assert_eq!(
        broken_dropped.glyphs.len(), broken_kept.glyphs.len() - 1,
        "Remove did not drop the ZWNJ",
    );
    assert_eq!(
        broken_dropped.glyphs.first(), broken_kept.glyphs.first(),
        "removing the ZWNJ from the output undid its effect on the shaping",
    );
}

#[test]
fn a_shaped_run_says_whether_it_finished() {
    for rel in [
        GARAMOND, INTER, STIX,
        "scheherazade-new/ScheherazadeNew-Regular.ttf",
        "noto-devanagari/NotoSansDevanagari.ttf",
        "noto-khmer/NotoSansKhmer.ttf",
    ] {
        let f = font(rel);
        for text in ["Hamburgefonstiv", "", "سلام دنيا", "हिन्दी", "ខ្មែរ", "fi ffl office"] {
            if let Some(r) = f.shape(text, &[], false) {
                assert!(r.complete, "{rel} did not finish shaping {text:?}");
            }
        }
    }
}

#[test]
fn the_dotted_circle_can_be_turned_off() {
    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let circle = f.glyph_id(0x25CC).expect("the fixture has U+25CC");

    let orphan = "\u{093F}";
    let on = f.shape(orphan, &[], false).expect("shapes");
    let off = f.shape_with_options(orphan, &[], false, &daegun::ShapeOptions {
        suppress_dotted_circle: true, ..Default::default()
    }).expect("shapes");

    assert!(on.glyphs.contains(&circle), "no dotted circle for an orphaned matra: {:?}", on.glyphs);
    assert!(!off.glyphs.contains(&circle), "suppressing it left one in: {:?}", off.glyphs);
    assert_eq!(off.glyphs.len(), on.glyphs.len() - 1, "suppressing removed more than the circle");

    let dflt = f.shape_with_options(orphan, &[], false, &daegun::ShapeOptions::default())
        .expect("shapes");
    assert_eq!(dflt.glyphs, on.glyphs, "the default stopped inserting the dotted circle");

    let based = "क\u{093F}";
    assert_eq!(
        f.shape(based, &[], false).expect("shapes").glyphs,
        f.shape_with_options(based, &[], false, &daegun::ShapeOptions {
            suppress_dotted_circle: true, ..Default::default()
        }).expect("shapes").glyphs,
        "suppressing changed a run that had no dotted circle in it",
    );
}

#[test]
fn an_itemized_script_reaches_the_fonts_feature_list() {
    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let runs = daegun::script_runs("हिन्दी");
    assert_eq!(runs.len(), 1, "the sample should itemize as one script");
    let script = runs[0].script;
    assert_eq!(script.name(), "Devanagari");

    let tags = script.opentype_tags();
    let declared = f.script_tags();
    let chosen = tags.iter().find(|t| declared.contains(t)).expect("the font declares one of them");
    assert_eq!(chosen, "dev2", "Noto Devanagari ships the second-generation model");

    let feats = f.feature_tags(Some(chosen), None);
    assert!(feats.len() > 10, "dev2 should select a full Indic feature set, saw {feats:?}");
    for required in ["abvs", "blws", "akhn", "nukt"] {
        assert!(feats.contains(&required.to_string()), "no {required} under {chosen}: {feats:?}");
    }

    assert!(
        f.feature_tags(Some("dev3"), None).is_empty(),
        "dev3 is not declared by this font and should offer nothing",
    );
}

#[test]
fn a_cluster_level_is_a_trade_and_the_output_shows_which() {
    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let text = "क्षि";
    let at = |lvl| f.shape_with_options(text, &[], false,
        &daegun::ShapeOptions { cluster_level: lvl, ..Default::default() }).expect("shapes");

    let mono_g = at(daegun::ClusterLevel::MonotoneGraphemes);
    let mono_c = at(daegun::ClusterLevel::MonotoneCharacters);
    let chars = at(daegun::ClusterLevel::Characters);
    let graph = at(daegun::ClusterLevel::Graphemes);

    for other in [&mono_c, &chars, &graph] {
        assert_eq!(other.glyphs, mono_g.glyphs, "a cluster level changed the glyphs");
        assert_eq!(other.advances, mono_g.advances, "a cluster level changed the advances");
    }

    let ascends = |r: &daegun::ShapedRun| r.clusters.windows(2).all(|w| w[0] <= w[1]);
    assert!(ascends(&mono_g), "MonotoneGraphemes ran backwards: {:?}", mono_g.clusters);
    assert!(ascends(&mono_c), "MonotoneCharacters ran backwards: {:?}", mono_c.clusters);
    assert!(
        !ascends(&chars) || !ascends(&graph),
        "neither non-monotone level reordered, so the fixture no longer shows the trade: \
         Characters {:?}, Graphemes {:?}", chars.clusters, graph.clusters,
    );

    assert!(daegun::ClusterLevel::MonotoneGraphemes.is_monotone());
    assert!(daegun::ClusterLevel::MonotoneCharacters.is_monotone());
    assert!(!daegun::ClusterLevel::Characters.is_monotone());
    assert!(!daegun::ClusterLevel::Graphemes.is_monotone());
    assert!(daegun::ClusterLevel::MonotoneGraphemes.is_graphemes());
    assert!(!daegun::ClusterLevel::MonotoneCharacters.is_graphemes());

    assert_eq!(daegun::ClusterLevel::default(), daegun::ClusterLevel::MonotoneGraphemes);
    assert_eq!(f.shape(text, &[], false).expect("shapes").clusters, mono_g.clusters);

    let ar = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let rtl = ar.shape("سلام (a) سلام", &[], false).expect("shapes");
    let asc = rtl.clusters.windows(2).all(|w| w[0] <= w[1]);
    let desc = rtl.clusters.windows(2).all(|w| w[0] >= w[1]);
    assert!(desc && !asc, "a right-to-left run should descend: {:?}", rtl.clusters);
    assert!(asc || desc, "monotone means one direction throughout, either one");
}

#[test]
fn a_run_says_whether_the_text_was_well_formed_for_its_script() {
    let dv = font("noto-devanagari/NotoSansDevanagari.ttf");
    let circle = dv.glyph_id(0x25CC).expect("the fixture has U+25CC");

    for good in ["हिन्दी", "क", "कि"] {
        let r = dv.shape(good, &[], false).expect("shapes");
        assert!(!r.has_broken_syllable, "{good:?} is well-formed and was reported broken");
        assert!(!r.glyphs.contains(&circle), "{good:?} got a dotted circle");
    }
    for bad in ["\u{093F}", "\u{094D}", "\u{093F}क"] {
        let r = dv.shape(bad, &[], false).expect("shapes");
        assert!(r.has_broken_syllable, "{bad:?} is a broken syllable and was not reported");
        assert!(r.glyphs.contains(&circle), "{bad:?} was called broken and drew no circle");
    }

    let bad = "\u{093F}";
    let quiet = dv.shape_with_options(bad, &[], false, &daegun::ShapeOptions {
        suppress_dotted_circle: true, ..Default::default()
    }).expect("shapes");
    assert!(!quiet.glyphs.contains(&circle), "the circle survived suppression");
    assert!(quiet.has_broken_syllable, "suppressing the circle also suppressed the finding");

    for (rel, good, bad) in [
        ("noto-khmer/NotoSansKhmer.ttf", "ខ្មែរ", "\u{17D2}"),
        ("noto-myanmar/NotoSansMyanmar.ttf", "မြန်မာ", "\u{1039}"),
    ] {
        let f = font(rel);
        assert!(!f.shape(good, &[], false).expect("shapes").has_broken_syllable, "{rel}: {good:?}");
        assert!(f.shape(bad, &[], false).expect("shapes").has_broken_syllable, "{rel}: {bad:?}");
    }

    for (rel, texts) in [
        (GARAMOND, ["Hamburg", "\u{0301}", "e\u{0301}"]),
        ("scheherazade-new/ScheherazadeNew-Regular.ttf", ["سلام", "\u{064E}", "لا"]),
    ] {
        let f = font(rel);
        for t in texts {
            assert!(
                !f.shape(t, &[], false).expect("shapes").has_broken_syllable,
                "{rel} has no syllable grammar but reported {t:?} broken",
            );
        }
    }
}

#[test]
fn a_run_says_which_shaping_model_it_went_through() {
    const KNOWN: [&str; 10] = [
        "default", "arabic", "hangul", "hebrew", "indic",
        "khmer", "myanmar", "myanmar_zawgyi", "thai", "universal",
    ];

    let ar = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    assert_eq!(ar.shape("سلام", &[], false).expect("shapes").shaper, "arabic");
    assert_eq!(ar.shape("abc", &[], false).expect("shapes").shaper, "default");

    let dv = font("noto-devanagari/NotoSansDevanagari.ttf");
    assert_eq!(dv.shape("हिन्दी", &[], false).expect("shapes").shaper, "indic");
    assert_eq!(dv.shape("abc", &[], false).expect("shapes").shaper, "default");

    for (rel, text, expect) in [
        (GARAMOND, "Hamburg", "default"),
        (GARAMOND, "Ελληνικά", "default"),
        ("noto-khmer/NotoSansKhmer.ttf", "ខ្មែរ", "khmer"),
        ("noto-myanmar/NotoSansMyanmar.ttf", "မြန်မာ", "myanmar"),
        ("source-han-sans/SourceHanSansJP-VF.otf", "한국어", "hangul"),
        ("source-han-sans/SourceHanSansJP-VF.otf", "日本語", "default"),
    ] {
        let got = font(rel).shape(text, &[], false).expect("shapes").shaper;
        assert_eq!(got, expect, "{rel} shaped {text:?} through {got}");
        assert!(KNOWN.contains(&got), "unknown shaper name {got}");
    }

    for (rel, text) in [
        (GARAMOND, "\u{0301}"),
        ("scheherazade-new/ScheherazadeNew-Regular.ttf", "\u{064E}"),
    ] {
        let r = font(rel).shape(text, &[], false).expect("shapes");
        assert!(!r.has_broken_syllable && r.shaper == "default" || r.shaper == "arabic",
                "{rel}: {text:?} -> {}", r.shaper);
    }
    let broken = dv.shape("\u{093F}", &[], false).expect("shapes");
    assert_eq!(broken.shaper, "indic", "a broken syllable must come from a syllabic model");
    assert!(broken.has_broken_syllable, "and it must report one");
}

#[test]
fn an_incomplete_run_reports_itself() {
    use daegun::bytes::{read_u16_be, write_u16_be};

    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> = f.table_tags().into_iter()
        .filter_map(|t| f.table(t).map(|d| (t.to_string(), d.to_vec())))
        .collect();

    let mut patched = 0usize;
    {
        let g = tables.get_mut("GSUB").expect("the fixture carries GSUB");
        let list = read_u16_be(g, 8).expect("lookup list offset") as usize;
        let count = read_u16_be(g, list).expect("lookup count");
        for i in 0..count {
            let lookup = list + read_u16_be(g, list + 2 + 2 * i as usize).expect("lookup offset") as usize;
            if read_u16_be(g, lookup) != Some(6) { continue }
            let subs = read_u16_be(g, lookup + 4).expect("subtable count");
            for s in 0..subs as usize {
                let sub = lookup + read_u16_be(g, lookup + 6 + 2 * s).expect("subtable offset") as usize;
                if read_u16_be(g, sub) != Some(3) { continue }
                let mut at = sub + 2;
                for _ in 0..3 {
                    let n = read_u16_be(g, at).expect("coverage count") as usize;
                    at += 2 + 2 * n;
                }
                let records = read_u16_be(g, at).expect("record count") as usize;
                at += 2;
                for k in 0..records {
                    write_u16_be(g, at + 4 * k + 2, i);
                    patched += 1;
                }
            }
        }
    }
    assert!(patched > 0, "no ChainContext records to patch; the fixture's GSUB changed shape");

    let looping = Font::from_bytes(&daegun::build_font(&tables)).expect("still parses");

    let incomplete: Vec<&str> = ["हिन्दी", "क्षि", "नमस्ते"]
        .into_iter()
        .filter(|t| looping.shape(t, &[], false).is_some_and(|r| !r.complete))
        .collect();
    assert!(
        !incomplete.is_empty(),
        "a self-referential lookup produced no incomplete run; the nesting guard may have moved",
    );

    for t in incomplete {
        let r = looping.shape(t, &[], false).expect("shapes");
        assert!(!r.complete, "{t:?}");
        assert_eq!(r.advances.len(), r.glyphs.len(), "{t:?}: arrays fell out of step");
        assert_eq!(r.clusters.len(), r.glyphs.len(), "{t:?}: clusters fell out of step");
        assert!(r.advances.iter().all(|a| a.is_finite()), "{t:?}: non-finite advance");
        assert!(r.glyphs.iter().all(|&g| g < looping.num_glyphs()), "{t:?}: glyph past the end");
    }

    for t in ["हिन्दी", "क्षि", "नमस्ते"] {
        assert!(f.shape(t, &[], false).expect("shapes").complete, "the sound font reported {t:?} partial");
    }
}

#[test]
fn the_subpixel_layout_bound_is_published() {
    let ok = daegun::SubpixelLayout::from_weights(
        (daegun::MAX_OVERSAMPLE, 1), (1, 1), (0, 0), [&[1.0], &[1.0], &[1.0]],
    );
    assert!(ok.is_some(), "the published maximum was itself refused");

    let over = daegun::SubpixelLayout::from_weights(
        (daegun::MAX_OVERSAMPLE + 1, 1), (1, 1), (0, 0), [&[1.0], &[1.0], &[1.0]],
    );
    assert!(over.is_none(), "one sample past the published maximum was accepted");
    let over_y = daegun::SubpixelLayout::from_weights(
        (1, daegun::MAX_OVERSAMPLE + 1), (1, 1), (0, 0), [&[1.0], &[1.0], &[1.0]],
    );
    assert!(over_y.is_none(), "one sample past the published maximum was accepted on y");
    for bad in [f32::NAN, f32::INFINITY] {
        let weight = daegun::SubpixelLayout::from_weights((1, 1), (1, 1), (0, 0), [&[bad], &[1.0], &[1.0]]);
        assert!(weight.is_none(), "a weight of {bad} was accepted");
    }

    assert!(
        daegun::SubpixelLayout::from_weights((0, 1), (1, 1), (0, 0), [&[1.0], &[1.0], &[1.0]]).is_none(),
        "an oversample of zero was accepted",
    );
    for taps in [(0, 0), (0, 5), (5, 0)] {
        let none: [&[f32]; 3] = [&[], &[], &[]];
        assert!(daegun::SubpixelLayout::from_weights((1, 1), taps, (0, 0), none).is_none(), "{taps:?} taps were accepted");
    }

    for layout in [
        daegun::SubpixelLayout::grayscale(),
        daegun::SubpixelLayout::horizontal(daegun::StripeOrder::Rgb),
        daegun::SubpixelLayout::vertical(daegun::StripeOrder::Bgr),
        daegun::SubpixelLayout::unfiltered(daegun::StripeOrder::Rgb, true),
    ] {
        let (ox, oy) = layout.oversample();
        assert!(ox <= daegun::MAX_OVERSAMPLE && oy <= daegun::MAX_OVERSAMPLE,
                "a preset oversamples past the published maximum: {ox}x{oy}");
    }
}

#[test]
fn a_gradient_can_be_sampled_without_engine_internals() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let mut sampled = 0;

    for gid in 0..f.num_glyphs() {
        let Some(graph) = f.colr_v1_paint(gid, &[], 0) else { continue };
        let mut scene = daegun::paint::DisplayList::default();
        let mut outline = |g: u16| {
            let mut path = daegun::Path::default();
            f.outline_glyph_instanced(g, &[], &mut path)?;
            (!path.is_empty()).then_some(path)
        };
        daegun::paint::lower(
            &graph,
            daegun::paint::IDENTITY,
            f.colr_clip_box(gid, &[]),
            &mut outline,
            daegun::paint::Rgba::default(),
            &mut scene,
        );

        for op in scene.ops() {
            let daegun::paint::Op::Fill { paint, transform, .. } = op else { continue };
            let daegun::paint::Paint::Gradient(g) = paint else { continue };

            let ramp = daegun::paint::gradient::Ramp::new(g, transform);
            let colors: Vec<_> = (0..64)
                .filter_map(|i| ramp.at(f64::from(i) * 16.0, f64::from(i) * 8.0))
                .collect();
            assert!(!colors.is_empty(), "gid {gid}: a gradient sampled to nothing anywhere");
            assert!(
                colors.iter().any(|c| c.a > 0),
                "gid {gid}: every sample was fully transparent",
            );
            sampled += 1;
            if sampled >= 3 { return }
        }
    }
    assert!(sampled > 0, "no gradient found in the COLR v1 fixture; the chain is untested");
}

#[test]
fn the_widest_layout_fits_the_table() {
    let taps = u8::try_from(daegun::MAX_SUBPIXEL_TAPS).expect("the tap bound fits a u8");
    let need = usize::from(taps) * usize::from(taps);
    let weights: Vec<f32> = core::iter::repeat_n(1.0 / need as f32, need).collect();

    let layout = daegun::SubpixelLayout::from_weights(
        (daegun::MAX_OVERSAMPLE, 1),
        (taps, taps),
        (-(i8::try_from(taps).expect("taps fit an i8") / 2), 0),
        [&weights, &weights, &weights],
    ).expect("the widest layout the table is sized for was refused");

    assert_eq!(layout.taps(), (taps, taps), "the layout did not keep its taps");
    for c in 0..3 {
        assert_eq!(layout.weights(c).map(<[f32]>::len), Some(need), "channel {c} lost weights");
        assert_eq!(layout.weights(c).and_then(<[f32]>::last), Some(&weights[need - 1]), "channel {c} lost its last weight");
    }

    // As many weights as its taps want, so only the tap bound can refuse it.
    let wider: Vec<f32> = vec![1.0 / f32::from(taps + 1); usize::from(taps) + 1];
    assert!(
        daegun::SubpixelLayout::from_weights(
            (daegun::MAX_OVERSAMPLE, 1), (taps + 1, 1), (0, 0),
            [&wider, &wider, &wider],
        ).is_none(),
        "a filter wider than the table was accepted",
    );
    let taller: Vec<f32> = vec![1.0 / f32::from(taps * (taps + 1)); usize::from(taps) * usize::from(taps + 1)];
    assert!(
        daegun::SubpixelLayout::from_weights(
            (1, daegun::MAX_OVERSAMPLE), (taps, taps + 1), (0, 0),
            [&taller, &taller, &taller],
        ).is_none(),
        "a filter taller than the table was accepted",
    );
}

#[test]
fn no_two_subpixel_layouts_share_an_identity() {
    use daegun::{StripeOrder::{Bgr, Rgb}, SubpixelLayout};

    let mut layouts = vec![
        ("grayscale", SubpixelLayout::grayscale()),
        ("horizontal rgb", SubpixelLayout::horizontal(Rgb)),
        ("horizontal bgr", SubpixelLayout::horizontal(Bgr)),
        ("vertical rgb", SubpixelLayout::vertical(Rgb)),
        ("vertical bgr", SubpixelLayout::vertical(Bgr)),
        ("unfiltered h rgb", SubpixelLayout::unfiltered(Rgb, true)),
        ("unfiltered h bgr", SubpixelLayout::unfiltered(Bgr, true)),
        ("unfiltered v rgb", SubpixelLayout::unfiltered(Rgb, false)),
        ("unfiltered v bgr", SubpixelLayout::unfiltered(Bgr, false)),
    ];

    let flat = [0.25f32; 4];
    let tilt = [0.25f32, 0.25, 0.25, 0.2501];
    let build = |name, os, taps, origin, first: &[f32]| {
        let l = SubpixelLayout::from_weights(os, taps, origin, [first, &flat, &flat])
            .unwrap_or_else(|| panic!("{name} was refused"));
        (name, l)
    };
    layouts.extend([
        build("base", (2, 2), (2, 2), (0, 0), &flat),
        build("oversample x", (1, 2), (2, 2), (0, 0), &flat),
        build("oversample y", (2, 1), (2, 2), (0, 0), &flat),
        build("taps swapped", (2, 2), (4, 1), (0, 0), &flat),
        build("origin", (2, 2), (2, 2), (-1, 0), &flat),
        build("one weight", (2, 2), (2, 2), (0, 0), &tilt),
    ]);

    for (i, (na, a)) in layouts.iter().enumerate() {
        for (nb, b) in layouts.iter().skip(i + 1) {
            assert_ne!(
                a.key(), b.key(),
                "'{na}' and '{nb}' share the key {:#x}, so a cache keyed on it serves one the \
                 other's bitmaps",
                a.key(),
            );
        }
    }

    assert_eq!(
        SubpixelLayout::horizontal(Rgb).key(), SubpixelLayout::horizontal(Rgb).key(),
        "the same layout built twice produced two identities",
    );
}

#[test]
fn a_caller_can_tell_an_empty_gradient_from_a_collapsed_one() {
    use daegun::paint::{resolve_stops, Extend, Gradient, GradientKind, Rgba, Stop, Stops};
    use daegun::paint::gradient::Ramp;

    let line = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 };
    let two = vec![
        Stop { offset: 0.0, color: Rgba::opaque(255, 0, 0) },
        Stop { offset: 1.0, color: Rgba::opaque(0, 0, 255) },
    ];
    let grad = |stops: Vec<Stop>| Gradient {
        kind: line, stops, extend: Extend::Pad, transform: daegun::paint::IDENTITY,
    };

    let empty = Ramp::new(&grad(Vec::new()), &daegun::paint::IDENTITY);
    let collapsed = Ramp::new(&grad(two.clone()), &[0.0; 6]);
    for x in [-50.0, 0.0, 1.0, 5.0, 9.5, 60.0] {
        assert_eq!(empty.at(x, 1.0), None, "an empty stop list painted at {x}");
        assert_eq!(collapsed.at(x, 1.0), None, "a singular transform painted at {x}");
    }

    assert!(matches!(resolve_stops(Vec::new()), Stops::Nothing), "an empty list was not Nothing");
    assert!(
        matches!(resolve_stops(two.clone()), Stops::Many(ref m) if m.len() == 2),
        "two stops did not survive as two",
    );
    assert!(
        matches!(resolve_stops(vec![two[0]]), Stops::Solid(c) if c == two[0].color),
        "one stop was not reported as a solid fill",
    );
}

// COLR defines a color line over its own stops, so repeat and reflect cycle over them: test_glyphs.ttf's
// lines run red to blue from x 100 to 900, with stops at 0.2 and 0.8, 0 and 1.5, and 0.5 and 1.5. The
// colors are the spec's blend in linear light, worked in f64 for each point's place on its line.
#[test]
fn a_color_line_repeats_over_its_own_stops() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let points = [(9u16, 0.05, [137u8, 0, 224]), (9, 0.95, [224, 0, 137]), (10, 0.5, [213, 0, 156]), (11, 0.9, [203, 0, 170])];
    for (gid, t, want) in points {
        let scene = f.colr_scene(gid, &[], 0).expect("a color glyph");
        let ramp = scene.ops().iter().find_map(|op| match op {
            daegun::paint::Op::Fill { paint: daegun::paint::Paint::Gradient(g), transform, .. } => {
                Some(daegun::paint::gradient::Ramp::new(g, transform))
            }
            _ => None,
        });
        let c = ramp.expect("a gradient fill").at(100.0 + 800.0 * t - 0.5, 249.5).expect("a color");
        let got = [c.r, c.g, c.b];
        assert!(got.iter().zip(want).all(|(&a, b)| a.abs_diff(b) <= 2), "gid {gid} at t {t}: {got:?}, not {want:?}");
    }
}

// One color line twice, over 0..1 and over 0.5..1.5 of geometry stretched to match: each kind samples
// alike under every extend, the stretched radial starting at a radius below zero.
#[test]
fn a_color_line_off_0_to_1_samples_as_its_stretched_geometry() {
    use daegun::paint::{gradient::Ramp, Extend, Gradient, GradientKind, Rgba, Stop, IDENTITY};
    let stops = |a: f32, b: f32| {
        vec![Stop { offset: a, color: Rgba::opaque(255, 0, 0) }, Stop { offset: b, color: Rgba::opaque(0, 0, 255) }]
    };
    let pairs = [
        (GradientKind::Linear { x0: 10.0, y0: 0.0, x1: 110.0, y1: 0.0 },
         GradientKind::Linear { x0: -40.0, y0: 0.0, x1: 60.0, y1: 0.0 }),
        (GradientKind::Radial { x0: 0.0, y0: 0.0, r0: 10.0, x1: 40.0, y1: 0.0, r1: 50.0 },
         GradientKind::Radial { x0: -20.0, y0: 0.0, r0: -10.0, x1: 20.0, y1: 0.0, r1: 30.0 }),
        (GradientKind::Sweep { cx: 0.0, cy: 0.0, start_angle: 30.0, end_angle: 120.0 },
         GradientKind::Sweep { cx: 0.0, cy: 0.0, start_angle: -15.0, end_angle: 75.0 }),
    ];
    for (plain, stretched) in pairs {
        for extend in [Extend::Pad, Extend::Repeat, Extend::Reflect] {
            let ramp = |kind, stops| Ramp::new(&Gradient { kind, stops, extend, transform: IDENTITY }, &IDENTITY);
            let (a, b) = (ramp(plain, stops(0.0, 1.0)), ramp(stretched, stops(0.5, 1.5)));
            for i in -20..20 {
                for j in -20..20 {
                    let (x, y) = (f64::from(i) * 7.3, f64::from(j) * 5.9);
                    let (p, q) = (a.at(x, y), b.at(x, y));
                    let close = match (p, q) {
                        (Some(p), Some(q)) => [p.r.abs_diff(q.r), p.g.abs_diff(q.g), p.b.abs_diff(q.b)].iter().all(|&d| d <= 1),
                        (p, q) => p == q,
                    };
                    assert!(close, "{plain:?} {extend:?} at ({x}, {y}): {p:?} against {q:?}");
                }
            }
        }
    }
}

// A static CFF2 font has no axes, so naming one when subsetting leaves its style as stored: here a
// face that is not Regular stays so.
#[test]
fn a_static_cff2_subset_keeps_its_style() {
    let f = font("source-serif/SourceSerif4Variable-Roman.otf");
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> = f.table_tags().into_iter()
        .filter(|t| *t != "fvar")
        .map(|t| (t.to_string(), f.table(t).expect("listed").to_vec()))
        .collect();
    tables.get_mut("OS/2").expect("OS/2")[62..64].copy_from_slice(&[0, 0]);
    let stat = Font::from_vec(daegun::build_font(&tables)).expect("the static font parses");
    let gid = stat.glyph_id('H' as u32).expect("H");
    let subset = stat.subset(&[gid], &[("wght", 600.0)]).expect("a subset");
    let os2 = Font::from_vec(subset.ttf).expect("the subset parses").table("OS/2").expect("OS/2").to_vec();
    assert_eq!(os2[62..64], [0, 0], "naming an axis the font lacks rewrote fsSelection");
}

// A hard edge, every stop at one offset, past 0..1 on either side: pad still switches colors there.
#[test]
fn a_hard_edge_past_0_to_1_stays_where_its_stops_put_it() {
    use daegun::paint::{gradient::Ramp, Extend, Gradient, GradientKind, Rgba, Stop, IDENTITY};
    let (red, blue) = (Rgba::opaque(255, 0, 0), Rgba::opaque(0, 0, 255));
    let line = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 100.0, y1: 0.0 };
    for (at, before, after) in [(1.5f32, 140.0, 160.0), (-0.5, -60.0, -40.0)] {
        let stops = vec![Stop { offset: at, color: red }, Stop { offset: at, color: blue }];
        let ramp = Ramp::new(&Gradient { kind: line, stops, extend: Extend::Pad, transform: IDENTITY }, &IDENTITY);
        assert_eq!(ramp.at(before - 0.5, 0.0), Some(red), "an edge at {at}: blue before it");
        assert_eq!(ramp.at(after - 0.5, 0.0), Some(blue), "an edge at {at}: red past it");
    }
}

// A ramp takes the scene's space to the device, as daegun_color_scene_ramp does, and applies the
// gradient's own transform first: a line from 0 to 10, doubled, is halfway along at 10, not past its end.
#[test]
fn a_ramp_applies_the_gradients_own_transform() {
    use daegun::paint::{Extend, Gradient, GradientKind, Rgba, Stop, IDENTITY};
    use daegun::paint::gradient::{Interpolation, Ramp};

    let stops = vec![
        Stop { offset: 0.0, color: Rgba::opaque(0, 0, 0) },
        Stop { offset: 1.0, color: Rgba::opaque(200, 200, 200) },
    ];
    let g = Gradient {
        kind: GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 },
        stops,
        extend: Extend::Pad,
        transform: [2.0, 0.0, 0.0, 2.0, 0.0, 0.0],
    };
    let at = Ramp::with_interpolation(&g, &IDENTITY, Interpolation::Srgb).at(9.5, 0.0).expect("a color");
    assert!((90..=110).contains(&at.r), "halfway along the doubled line sampled {}, not about 100", at.r);
}

#[test]
fn a_composed_transform_can_be_undone() {
    use daegun::paint::{concat, invert, IDENTITY};

    let scale = [2.0, 0.0, 0.0, 3.0, 0.0, 0.0];
    let rotate = [0.0, 1.0, -1.0, 0.0, 0.0, 0.0];
    let shift = [1.0, 0.0, 0.0, 1.0, 40.0, -25.0];
    let m = concat(&concat(&scale, &rotate), &shift);
    let back = invert(&m).expect("an invertible composition reported no inverse");

    let apply = |t: &[f64; 6], p: (f64, f64)| {
        (t[0] * p.0 + t[2] * p.1 + t[4], t[1] * p.0 + t[3] * p.1 + t[5])
    };
    for p in [(0.0, 0.0), (100.0, 0.0), (0.0, -250.0), (713.0, 486.0)] {
        let there = apply(&m, p);
        let (x, y) = apply(&back, there);
        assert!(
            (x - p.0).abs() < 1e-9 && (y - p.1).abs() < 1e-9,
            "{p:?} went to {there:?} and came back {:?}", (x, y),
        );
    }

    let round = concat(&m, &back);
    for (got, want) in round.iter().zip(IDENTITY.iter()) {
        assert!((got - want).abs() < 1e-9, "m then m-inverse was {round:?}, not the identity");
    }

    assert!(invert(&[0.0; 6]).is_none(), "the zero matrix reported an inverse");
    assert!(invert(&[2.0, 0.0, 0.0, 0.0, 0.0, 0.0]).is_none(), "scale(2, 0) reported an inverse");
    assert!(invert(&IDENTITY).is_some(), "the identity reported no inverse");
    assert!(invert(&[f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]).is_none(), "a NaN matrix reported an inverse");
    assert!(
        invert(&[1e-200, 0.0, 0.0, 1e-200, 1.0, 1.0]).is_none(),
        "a matrix whose determinant underflowed to zero was reported as invertible",
    );
    assert!(
        invert(&[1e-160, 0.0, 0.0, 1e-160, 1e300, 0.0]).is_none(),
        "a matrix with a finite determinant whose inverse overflows was reported as invertible",
    );
}

#[test]
fn fading_a_color_by_nothing_meaningful_leaves_it_alone() {
    use daegun::paint::Rgba;
    let c = Rgba::opaque(10, 20, 30);

    assert_eq!(c.fade(1.0).a, 255, "a full factor changed the alpha");
    assert_eq!(c.fade(0.5).a, 128, "half of 255 was not 128");
    assert_eq!(c.fade(0.0).a, 0, "a zero factor left the color visible");

    assert_eq!(c.fade(2.0).a, 255, "a factor above one did not clamp");
    assert_eq!(c.fade(-1.0).a, 0, "a factor below zero did not clamp");

    for (name, by) in [("NaN", f64::NAN), ("inf", f64::INFINITY), ("-inf", f64::NEG_INFINITY)] {
        assert_eq!(c.fade(by), c, "fade({name}) changed the color instead of leaving it alone");
    }

    let faded = c.fade(0.25);
    assert_eq!((faded.r, faded.g, faded.b), (10, 20, 30), "fade touched a color channel");
}

#[test]
fn padding_covers_the_filter_reach() {
    use daegun::{StripeOrder::{Bgr, Rgb}, SubpixelLayout};

    // A window can reach before its pixel and past it, and the pad has to cover both.
    let covers = |l: &SubpixelLayout| {
        let ((px, py), (ox, oy), (rx, ry), (tx, ty)) = (l.pad(), l.oversample(), l.origin(), l.taps());
        let fits = |pad: usize, o: u8, r: i8, t: u8| {
            let reach = (-isize::from(r)).max(isize::from(r) + isize::from(t) - isize::from(o)).max(0);
            (pad * usize::from(o)) as isize >= reach
        };
        fits(px, ox, rx, tx) && fits(py, oy, ry, ty)
    };
    let presets = [
        ("grayscale", SubpixelLayout::grayscale()),
        ("horizontal rgb", SubpixelLayout::horizontal(Rgb)),
        ("horizontal bgr", SubpixelLayout::horizontal(Bgr)),
        ("vertical rgb", SubpixelLayout::vertical(Rgb)),
        ("unfiltered h", SubpixelLayout::unfiltered(Rgb, true)),
    ];
    for (name, l) in &presets {
        assert!(covers(l), "{name}: padding {:?} does not cover origin {:?} at oversample {:?}",
                l.pad(), l.origin(), l.oversample());
    }

    assert_eq!(SubpixelLayout::grayscale().pad(), (0, 0), "grayscale asked for padding");
    assert_eq!(SubpixelLayout::horizontal(Rgb).pad(), (1, 0), "horizontal padding is not (1, 0)");
    assert_eq!(SubpixelLayout::vertical(Rgb).pad(), (0, 1), "vertical padding is not (0, 1)");
    assert_eq!(SubpixelLayout::unfiltered(Rgb, true).pad(), (0, 0), "unfiltered asked for padding");

    let three = [0.25f32, 0.5, 0.25];
    let ahead = SubpixelLayout::from_weights((1, 1), (3, 1), (0, 0), [&three, &three, &three]).expect("3 taps");
    assert!(covers(&ahead) && ahead.pad() == (2, 0), "a window 2 samples past its pixel padded {:?}", ahead.pad());
    let four = [0.25f32; 4];
    let wide = SubpixelLayout::from_weights((3, 1), (4, 1), (0, 0), [&four, &four, &four]).expect("4 taps");
    assert!(covers(&wide) && wide.pad() == (1, 0), "a window a sample past its pixel padded {:?}", wide.pad());

    let wide = [0.1f32; 12];
    let l = SubpixelLayout::from_weights((2, 1), (6, 2), (-5, 0), [&wide, &wide, &wide])
        .expect("a six-tap filter within the bounds was refused");
    assert_eq!(l.pad(), (3, 1), "a filter reaching 2.5 pixels across and one down did not pad (3, 1)");
    assert!(covers(&l), "the caller-supplied layout's padding does not cover its reach");

    assert!(SubpixelLayout::grayscale().is_grayscale(), "grayscale did not report itself grayscale");
    for (name, l) in &presets[1..] {
        assert!(!l.is_grayscale(), "{name} reported itself grayscale");
        assert_eq!(l.channels(), 3, "{name} did not resolve three channels");
    }
    assert!(!l.is_grayscale(), "a from_weights layout reported itself grayscale");
}

#[test]
fn mixed_direction_shaping_takes_the_options_the_rest_of_the_family_does() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let text = "السلام hello عليكم";

    let plain = f.shape_bidi(text, &[], None).expect("shape_bidi");
    assert!(plain.len() >= 2, "the text did not split into directional runs");

    assert!(
        plain.iter().all(|r| r.run.unsafe_to_concat.is_empty()),
        "unsafe_to_concat was reported without being asked for",
    );
    let asked = f
        .shape_bidi_with(
            text,
            &[],
            None,
            &daegun::ShapeOptions { report_unsafe_to_concat: true, ..Default::default() },
        )
        .expect("shape_bidi_with");
    assert_eq!(asked.len(), plain.len(), "asking for a flag changed how the text was split");
    assert!(
        asked.iter().any(|r| !r.run.unsafe_to_concat.is_empty()),
        "no run reported unsafe_to_concat, so the options did not reach the runs",
    );
    for (a, b) in asked.iter().zip(plain.iter()) {
        assert_eq!(a.run.glyphs, b.run.glyphs, "a reporting flag changed the glyphs");
        assert_eq!(a.level, b.level, "a reporting flag changed a run's level");
    }

    let joined = "\u{0628}\u{200E}\u{0628}";
    let with_junk = f
        .shape_bidi_with(
            joined,
            &[],
            None,
            &daegun::ShapeOptions { before: "ZZZZ", after: "ZZZZ", ..Default::default() },
        )
        .expect("shape_bidi_with on the joined pair");
    let default_run = f.shape_bidi(joined, &[], None).expect("shape_bidi on the joined pair");
    let g = |v: &Vec<daegun::BidiRun>| v.iter().flat_map(|r| r.run.glyphs.clone()).collect::<Vec<_>>();
    assert_eq!(
        g(&with_junk), g(&default_run),
        "a caller's `before`/`after` reached the runs and changed the joining, which is what this \
         function computes per run precisely so a caller cannot get it wrong",
    );

    let via_default = f
        .shape_bidi_with(text, &[], None, &daegun::ShapeOptions::default())
        .expect("shape_bidi_with under defaults");
    assert_eq!(g(&via_default), g(&plain), "shape_bidi and shape_bidi_with disagree under defaults");
}

#[test]
fn the_raw_tier_can_outline_a_glyph_from_the_bytes_it_is_given() {
    let f = font(GARAMOND);
    let glyf = f.table("glyf").expect("glyf");
    let loca = f.table("loca").expect("loca");
    let head = f.table("head").expect("head");
    let fmt = i16::from_be_bytes([head[50], head[51]]);
    let offsets = daegun::parse_loca(loca, fmt, usize::from(f.num_glyphs()));
    assert_eq!(
        offsets.len(), usize::from(f.num_glyphs()) + 1,
        "loca did not parse to one offset per glyph plus the end sentinel",
    );
    assert!(offsets.windows(2).all(|w| w[0] <= w[1]), "loca offsets are not monotone");
    // maxp counts glyphs in a u16, so a larger count is not a font's and must not size the answer.
    assert_eq!(daegun::parse_loca(loca, fmt, usize::MAX).len(), 65_536, "loca sized itself from usize::MAX");

    let mut compared = 0;
    for ch in ['B', 'o', 'x', 'A', 'g', 'W'] {
        let gid = f.glyph_id(ch as u32).expect("glyph");
        let mut raw = daegun::Path::default();
        if daegun::outline_glyf_bytes(glyf, &offsets, gid, &mut raw).is_err() {
            continue;
        }
        let mut viaapi = daegun::Path::default();
        f.outline_glyph(gid, &mut viaapi).expect("the front door outlined it");
        assert_eq!(
            raw.parts(), viaapi.parts(),
            "{ch}: the raw path and Font::outline_glyph disagree about verbs or coordinates",
        );
        assert!(!raw.is_empty(), "{ch} outlined to nothing");
        compared += 1;
    }
    assert!(compared >= 4, "only {compared} glyphs were compared, so this proves little");

    let mut p = daegun::Path::default();
    assert!(
        daegun::outline_glyf_bytes(glyf, &offsets, f.num_glyphs(), &mut p).is_err(),
        "a glyph id past num_glyphs was outlined instead of refused",
    );
}

#[test]
fn a_vertical_origin_is_asked_for_a_location() {
    let f = font("source-han-sans/SourceHanSansJP-VF.otf");
    let axes = f.axes();
    assert!(!axes.is_empty(), "the fixture is not variable");
    let (tag, lo, hi, def) = (axes[0].tag.clone(), axes[0].min, axes[0].max, axes[0].default);
    let gid = f.glyph_id('\u{6c38}' as u32).or_else(|| f.glyph_id('A' as u32)).expect("a glyph");

    assert_eq!(
        f.vertical_origin(gid, &[]),
        f.vertical_origin(gid, &[(tag.as_str(), def)]),
        "the default location answered differently depending on whether it was named",
    );
    let (o_lo, o_hi) = (f.vertical_origin(gid, &[(tag.as_str(), lo)]), f.vertical_origin(gid, &[(tag.as_str(), hi)]));
    assert_eq!(o_lo.is_some(), o_hi.is_some(), "one end of the axis stated an origin and the other did not");
    assert!(o_lo.is_some(), "the fixture states no vertical origin, so this proves nothing at all");

    assert!(
        f.vertical_advance(gid, &[(tag.as_str(), lo)]) > 0,
        "the fixture reports no vertical advance",
    );
    assert!(f.vertical_origin(f.num_glyphs(), &[]).is_none(), "a glyph id past num_glyphs got an origin");
}

#[test]
fn a_guillemet_after_an_arabic_word_takes_the_arabic_form_without_flipping() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let arab = daegun::script_runs("\u{0628}")[0].script;
    let latn = daegun::script_runs("a")[0].script;

    let seeded = |seed| {
        f.shape_bidi_with(
            "\u{00BB}", &[], Some(false),
            &daegun::ShapeOptions { seed_script: Some(seed), ..Default::default() },
        )
        .expect("shaped")
        .iter()
        .flat_map(|r| r.run.glyphs.clone())
        .collect::<Vec<u16>>()
    };
    let (arabic_form, latin_form) = (seeded(arab), seeded(latn));
    assert_ne!(
        arabic_form, latin_form,
        "the fixture does not distinguish the guillemet by script at a fixed direction, so nothing \
         below can prove the seed arrived",
    );

    let text = "abc \u{0639}\u{0631}\u{0628}\u{064A} \u{00BB} def";
    let opts = daegun::LayoutOptions { max_inline_size: 100_000.0, ..Default::default() };
    let laid = f.layout(text, &[], &opts).expect("laid out");
    let runs: Vec<_> = laid.lines.iter().flat_map(|l| l.runs.iter()).collect();

    let holder = runs
        .iter()
        .find(|r| r.chars.0 <= 9 && 9 < r.chars.1)
        .expect("no run covers the guillemet");
    assert_eq!(
        holder.level % 2, 0,
        "the guillemet's run flipped to right-to-left; the seeded script overrode the bidi level",
    );
    assert!(
        holder.run.glyphs.contains(&arabic_form[0]),
        "the guillemet took {:?}, not the Arabic form {:?} – the itemized script did not reach the \
         run that reaches the caller",
        holder.run.glyphs, arabic_form,
    );
    assert!(
        !holder.run.glyphs.contains(&latin_form[0]),
        "the guillemet kept its Latin form after an Arabic word",
    );
}

#[test]
fn a_seeded_script_does_not_override_the_resolved_level() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let opts = daegun::LayoutOptions { max_inline_size: 100_000.0, ..Default::default() };

    let text = "\u{0633}\u{0644}\u{0627}\u{0645} hello \u{0633}\u{0644}\u{0627}\u{0645}";
    let laid = f.layout(text, &[], &opts).expect("laid out");
    let runs: Vec<_> = laid.lines.iter().flat_map(|l| l.runs.iter()).collect();
    assert!(runs.iter().any(|r| r.level % 2 == 1), "no right-to-left run, so the text is not mixed");
    let even: Vec<_> = runs.iter().filter(|r| r.level % 2 == 0).collect();
    assert!(!even.is_empty(), "no left-to-right run, so this proves nothing");

    let alone = f.shape("hello", &[], false).expect("hello alone").glyphs.clone();
    let found: Vec<u16> = even.iter().flat_map(|r| r.run.glyphs.clone()).collect();
    assert!(
        found.windows(alone.len()).any(|w| w == alone.as_slice()),
        "the even-level run did not come back left-to-right: expected {alone:?} inside {found:?}",
    );

    let all_arabic = f.shape("\u{0633}\u{0644}\u{0627}\u{0645}", &[], false).expect("shaped");
    let reversed = f.shape("\u{0645}\u{0627}\u{0644}\u{0633}", &[], false).expect("shaped");
    assert_ne!(
        all_arabic.glyphs, reversed.glyphs,
        "Font::shape stopped distinguishing an Arabic string from its reverse, which means it \
         stopped guessing the direction",
    );
}

#[test]
fn a_replayed_outline_gives_the_same_quads_as_a_decode() {
    let faces = [
        "eb-garamond/EBGaramond.ttf",
        "inter/InterVariable.ttf",
        "noto-devanagari/NotoSansDevanagari.ttf",
        "source-han-sans/SourceHanSansJP-VF.otf",
    ];
    let mut compared = 0usize;
    for rel in faces {
        let f = font(rel);
        let n = f.num_glyphs().min(300);
        for gid in 0..n {
            let mut direct = daegun::QuadraticPen::new(f32::from(f.upm()));
            if f.outline_glyph_instanced(gid, &[], &mut direct).is_none() {
                continue;
            }
            let mut path = daegun::Path::default();
            f.outline_glyph_instanced(gid, &[], &mut path).expect("the second decode disagreed");
            let mut replayed = daegun::QuadraticPen::new(f32::from(f.upm()));
            path.replay(None, &mut replayed);

            match (direct.finish(), replayed.finish()) {
                (Ok(a), Ok(b)) => assert_eq!(
                    a, b,
                    "{rel} gid {gid}: replaying the cached outline gave different curves",
                ),
                (Err(a), Err(b)) => assert_eq!(
                    a, b, "{rel} gid {gid}: the two routes disagreed about refusing the glyph",
                ),
                (a, b) => panic!("{rel} gid {gid}: one route produced curves and the other did not: {a:?} / {b:?}"),
            }
            compared += 1;
        }
    }
    assert!(compared > 1000, "only {compared} glyphs were compared, so this proves little");
}
