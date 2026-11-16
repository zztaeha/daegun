use crate::Hint;
use daegun::outline::HintMode;
use daegun::shape::{Direction, Feature, ShapeOptions, Shaper};
use daegun::unicode::BaseDirection;
use daegun::{Bounds, Font, GlyphId, Location, NameId, Orientation, Tag};

use crate::located;

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
    let tags: Vec<_> = f.table_tags().collect();
    assert!(tags.contains(&Tag::new(b"head")), "no head table in {tags:?}");
    assert!(tags.contains(&Tag::new(b"cmap")), "no cmap table");
    assert!(tags.len() > 5, "only {} tables, which is not a real font", tags.len());

    let head = f.table(Tag::new(b"head")).expect("head is in table_tags but table() declined it");
    assert_eq!(
        daegun::raw::bytes::read_u32_be(head, 12),
        Some(0x5F0F_3CF5),
        "head does not carry its own magic number, so the slice is wrong"
    );

    assert!(f.has_table(Tag::new(b"head")));
    assert!(!f.has_table(Tag::new(b"ZZZZ")), "a tag no font carries reported present");
    assert!(f.table(Tag::new(b"ZZZZ")).is_none());

    for &tag in &tags {
        assert!(f.table(tag).is_some(), "{tag} is listed but does not resolve");
    }
}

#[test]
fn the_byte_readers_decline_past_the_end() {
    let data = [0u8, 1, 0, 2];
    assert_eq!(daegun::raw::bytes::read_u16_be(&data, 0), Some(1));
    assert_eq!(daegun::raw::bytes::read_u16_be(&data, 2), Some(2));
    assert_eq!(daegun::raw::bytes::read_u16_be(&data, 3), None, "a read past the end must decline");
    assert_eq!(daegun::raw::bytes::read_u32_be(&data, 1), None);
    assert!(!daegun::raw::bytes::records_fit(0, 3, 2, 4), "3 records of 2 do not fit in 4 bytes");
    assert!(daegun::raw::bytes::records_fit(0, 2, 2, 4));
}

// The iterator owns what it walks, so it outlives the font it came from.
#[test]
fn coverage_outlives_its_font() {
    let cov = font(GARAMOND).coverage();
    assert!(cov.count() > 100);
}

// Both ends are open to a caller: the highest character the font maps, and a right-to-left run's ids read backward.
#[test]
fn coverage_and_glyph_ids_read_from_either_end() {
    let f = font(GARAMOND);
    let highest = f.coverage().next_back().expect("a text face maps characters");
    assert_eq!(Some(highest), f.coverage().last());
    let ids = f.glyph_ids("Type");
    let backward: Vec<_> = ids.clone().rev().collect();
    let mut forward: Vec<_> = ids.collect();
    forward.reverse();
    assert_eq!(backward, forward);
}

// Each list read backward is the list reversed: tags, names, a hinted outline's points and a script's tags; a
// paint's children clone.
#[test]
fn the_lists_read_from_either_end() {
    fn both<T: PartialEq + core::fmt::Debug>(list: impl DoubleEndedIterator<Item = T> + Clone) {
        let mut forward: Vec<T> = list.clone().collect();
        forward.reverse();
        assert!(forward.len() > 1, "{forward:?}");
        assert_eq!(list.rev().collect::<Vec<_>>(), forward);
    }
    let f = font(GARAMOND);
    both(f.table_tags());
    both(f.glyph_names());
    both(daegun::unicode::Script::DEVANAGARI.opentype_tags());
    let at = Location::default();
    let hinted = f.hinted_outline(GlyphId(40), &at, 16.0, HintMode::AutoForce).expect("hints").expect("an outline");
    both(hinted.points());
    let colr = font("colr-v1-test-glyphs/test_glyphs_variable.ttf");
    let graph = colr.paint_graph(GlyphId(10), &at, 0).expect("a color glyph");
    let children = graph.root().children();
    assert!(children.clone().count() > 0 && children.clone().map(|c| c.node()).eq(children.map(|c| c.node())));
}

#[test]
fn coverage_agrees_with_glyph_id() {
    let f = font(GARAMOND);
    let cov: Vec<_> = f.coverage().collect();
    assert!(cov.len() > 100, "only {} codepoints covered, which is not a text face", cov.len());

    for w in cov.windows(2) {
        assert!(w[0].0 < w[1].0, "coverage is not strictly ascending at {:?}", w[0].0);
    }

    for &(cp, gid) in cov.iter().step_by(cov.len() / 50 + 1) {
        assert_eq!(
            f.glyph_id(cp),
            Some(gid),
            "coverage maps {cp:?} to {gid} but glyph_id disagrees",
        );
    }
    assert!(cov.iter().all(|&(cp, _)| f.has_glyph(cp)));
    assert!(cov.iter().any(|&(cp, _)| cp == 'A'), "EBGaramond does not cover A");
}

#[test]
fn glyph_bounds_are_tight_and_instanced() {
    let f = font(GARAMOND);
    let gid = f.glyph_id('H').expect("EBGaramond has H");
    let Bounds { x_min: x0, y_min: y0, x_max: x1, y_max: y1 } =
        f.glyph_bounds(gid, &Location::default()).expect("H has an outline");
    assert!(x1 > x0 && y1 > y0, "H's box is empty: {x0},{y0},{x1},{y1}");

    let b = f.bounds().expect("head's box");
    assert!(
        x0 >= b.x_min - 1.0 && y0 >= b.y_min - 1.0 && x1 <= b.x_max + 1.0 && y1 <= b.y_max + 1.0,
        "H's box {x0},{y0},{x1},{y1} escapes the font box {b:?}",
    );
    assert!(y0.abs() < 20.0, "H's bottom is at {y0}, not on the baseline");
    assert!(y1 > 500.0, "H's top is at {y1}, which is not a capital");

    assert!(
        f.glyph_bounds(GlyphId(u16::MAX.into()), &Location::default()).is_none(),
        "a gid past the end produced a box"
    );
    let space = f.glyph_id(' ').expect("EBGaramond has a space");
    assert!(f.glyph_bounds(space, &Location::default()).is_none(), "the space glyph produced a box");
    assert!(
        f.glyph_bounds(GlyphId(0), &Location::default()).is_some(),
        "EBGaramond's .notdef drew no box",
    );

    let v = font(INTER);
    let h = v.glyph_id('H').expect("Inter has H");
    let light = v.glyph_bounds(h, &located(&v, &[("wght", 100.0)])).expect("light H");
    let heavy = v.glyph_bounds(h, &located(&v, &[("wght", 900.0)])).expect("heavy H");
    assert!(
        heavy.x_max - heavy.x_min > light.x_max - light.x_min,
        "wght 900 H is not wider than wght 100: {heavy:?} against {light:?}",
    );
}

#[test]
fn os2_info_reads_the_selection_and_the_windows_box() {
    let f = font(GARAMOND);
    let os2 = f.os2(&Location::default()).expect("EBGaramond carries OS/2");
    assert!(os2.version <= 5, "OS/2 version {} is not a real version", os2.version);
    assert!(os2.selection.is_some(), "no fsSelection came back");
    assert!(!os2.is_italic(), "EBGaramond Regular reported italic");
    assert!(!os2.is_oblique());

    let w = os2.win_metrics.expect("EBGaramond states usWinAscent/Descent");
    assert!(w.ascent > 0.0 && w.descent > 0.0, "win metrics are magnitudes: {w:?}");
    assert!(
        w.ascent >= f.ascender(&Location::default()) - 1.0,
        "usWinAscent {} is below the typographic ascender {}, which is the wrong scale",
        w.ascent,
        f.ascender(&Location::default()),
    );
    assert!(w.ascent < 3000.0, "usWinAscent {} was not scaled to 1000 upm", w.ascent);

    assert!(!(os2.is_regular() && os2.is_italic()), "the font claims both regular and italic");
}

#[test]
fn names_agree_with_name() {
    let f = font(GARAMOND);
    let all = f.names(&Location::default());
    assert!(all.len() > 3, "only {} names, which is not a shipping font", all.len());

    for (&id, text) in all.iter() {
        assert_eq!(
            f.name(id, &Location::default()).as_ref(),
            Some(text),
            "names() and name() disagree about name id {id}",
        );
        assert!(!text.is_empty(), "name id {id} came back as an empty string");
    }
    assert!(all.contains_key(&NameId::FAMILY_NAME), "no family name in {:?}", all.keys().collect::<Vec<_>>());
    let mut lent = Vec::new();
    f.visit_names(&Location::default(), |id, text| lent.push((id, text.to_owned())));
    assert!(lent.into_iter().eq(all), "visit_names() lent other names than names() gives");
}

#[test]
fn normalized_coordinates_span_minus_one_to_one() {
    let v = font(INTER);
    let axes = v.axes();
    assert!(!axes.is_empty(), "Inter is variable and should report axes");

    // Empty at the default, as Location::default() is, which knows no font's axes.
    assert!(located(&v, &[]).normalized().is_empty(), "the default location has coordinates");
    assert_eq!(located(&v, &[("wght", 700.0)]).normalized().len(), axes.len(), "one coordinate per fvar axis");

    let wght = axes.iter().position(|a| a.tag == Tag::new(b"wght")).expect("Inter varies weight");
    let heavy = located(&v, &[("wght", axes[wght].max)]).normalized().to_vec();
    assert!(
        (heavy[wght] - 1.0).abs() < 1e-6,
        "the maximum should normalize to 1.0, got {}",
        heavy[wght],
    );
    let light = located(&v, &[("wght", axes[wght].min)]).normalized().to_vec();
    assert!(
        (light[wght] + 1.0).abs() < 1e-6,
        "the minimum should normalize to -1.0, got {}",
        light[wght],
    );
    let past = located(&v, &[("wght", 100_000.0)]).normalized().to_vec();
    assert!(past.iter().all(|&c| (-1.0..=1.0).contains(&c)), "out of range: {past:?}");

    let static_face = font(STIX);
    assert!(!static_face.is_variable(), "STIX2Math gained axes; pick another fixture");
    assert!(located(&static_face, &[]).normalized().to_vec().is_empty());
}

// Normalized in 16.16 as the variations overview requires: SWC1 at -1.8 of ±2 is -117,965 / 131,072 of the way, so
// -58,983 in 16.16 and -14,746 in 2.14; Source Serif's weight 204.55 is -64,045, which its avar maps onto -16,109.
#[test]
fn a_location_normalizes_in_16_16() {
    let colr = font("colr-v1-test-glyphs/test_glyphs_variable.ttf");
    let swc1 = colr.axes().iter().position(|a| a.tag == Tag::new(b"SWC1")).expect("the COLR test font varies SWC1");
    assert_eq!(located(&colr, &[("SWC1", -1.8)]).normalized().to_vec()[swc1] * 16384.0, -14746.0);
    assert_eq!(located(&font(SOURCE_SERIF), &[("wght", 204.55)]).normalized().to_vec()[0] * 16384.0, -16109.0);
}

// The same axes name the same instance everywhere: a value that is not a number is dropped, and of two
// for one tag the last stands, in the location and the metrics as in the advances.
#[test]
fn odd_axis_lists_resolve_as_every_other_call_resolves_them() {
    type Axes<'a> = &'a [(&'a str, f64)];
    let v = font(INTER);
    let cases: [(Axes, Axes); 4] = [
        (&[("wght", f64::NAN)], &[]),
        (&[("wght", f64::INFINITY)], &[]),
        (&[("wght", f64::NEG_INFINITY)], &[]),
        (&[("wght", 900.0), ("wght", 100.0)], &[("wght", 100.0)]),
    ];
    let gid = v.glyph_id('H').expect("Inter has H");
    for (odd, plain) in cases {
        assert_eq!(
            located(&v, odd).normalized().to_vec(),
            located(&v, plain).normalized().to_vec(),
            "normalized({odd:?})"
        );
        assert_eq!(
            v.typographic_metrics(&located(&v, odd)),
            v.typographic_metrics(&located(&v, plain)),
            "typographic_metrics({odd:?})"
        );
        let widths = |axes| crate::advances(&v, &[GlyphId(gid.0)], &located(&v, axes));
        assert_eq!(widths(odd), widths(plain), "advances({odd:?})");
    }
}

#[test]
fn tracking_is_zero_without_a_trak_table() {
    let f = font(GARAMOND);
    assert!(!f.has_table(Tag::new(b"trak")), "EBGaramond gained a trak table; pick another fixture");
    assert_eq!(f.tracking(12.0, Orientation::Horizontal), 0.0, "a face with no trak reported tracking");
    assert_eq!(f.tracking(72.0, Orientation::Vertical), 0.0);
}

#[test]
fn a_path_records_and_replays_an_outline() {
    let f = font(STIX);
    let gid = f.glyph_id('x').expect("STIX has x");

    let mut path = daegun::outline::Path::default();
    f.outline(gid, &Location::default(), &mut path).expect("x has an outline");
    assert!(!path.is_empty(), "the pen recorded nothing");

    let (verbs, points) = path.parts();
    assert!(!verbs.is_empty() && !points.is_empty());
    assert!(verbs.contains(&daegun::outline::Verb::Move), "an outline with no move_to");

    let mut again = daegun::outline::Path::default();
    path.replay(None, &mut again);
    assert_eq!(path.parts().1, again.parts().1, "replay moved the points");

    let mut shifted = daegun::outline::Path::default();
    path.replay(Some(&[1.0, 0.0, 0.0, 1.0, 100.0, 0.0]), &mut shifted);
    let Bounds { x_min: bx0, y_min: _, x_max: _, y_max: _ } = path.bounds().expect("x has bounds");
    let Bounds { x_min: sx0, y_min: _, x_max: _, y_max: _ } = shifted.bounds().expect("the shifted copy has bounds");
    assert!((sx0 - bx0 - 100.0).abs() < 0.01, "the transform did not move the box: {bx0} to {sx0}");
}

#[test]
fn the_variation_store_resolves_the_advance_delta_the_engine_applies() {
    let f = font(INTER);
    let hvar = f.table(Tag::new(b"HVAR")).expect("InterVariable carries HVAR");

    let ivs_off = daegun::raw::bytes::read_u32_be(hvar, 4).expect("HVAR header truncated") as usize;
    assert_ne!(ivs_off, 0, "HVAR names no variation store");
    let store =
        daegun::raw::format::ItemVariationStore::parse(hvar, ivs_off).expect("the store does not parse");
    assert!(!store.regions.is_empty(), "a store with no regions varies nothing");
    assert!(store.axis_count > 0, "the store declares no axes");
    assert!(
        store.ivd_data.iter().any(|ivd| ivd.rows() > 0),
        "every ItemVariationData subtable is empty",
    );

    let map_off = daegun::raw::bytes::read_u32_be(hvar, 8).expect("HVAR header truncated") as usize;
    assert_ne!(map_off, 0, "InterVariable's HVAR grew an implicit mapping; pick another fixture");
    let map = daegun::raw::format::parse_delta_set_index_map(hvar, map_off).expect("the map does not parse");
    assert!(!map.is_empty(), "the delta-set index map is empty");

    let axes = f.axes();
    let wght = axes.iter().find(|a| a.tag == Tag::new(b"wght")).expect("Inter varies weight");
    let heavy = [("wght", wght.max)];
    let scalars = store.region_scalars(located(&f, &heavy).normalized());
    assert!(
        scalars.iter().any(|&s| s != 0.0),
        "no region is live at the heaviest weight, so nothing could vary",
    );
    let at_origin = store.region_scalars(located(&f, &[]).normalized());
    assert!(at_origin.iter().all(|&s| s == 0.0), "a region is live at the default location");

    let (mut checked, mut moved) = (0usize, 0usize);
    for cp in "HOnexamg0123".chars() {
        let Some(gid) = f.glyph_id(cp) else { continue };
        let index = daegun::raw::format::delta_set_index_map_lookup(&map, gid.0 as usize);
        let delta = store.delta(index, &scalars);

        let base = crate::advances(&f, &[gid], &Location::default())[0];
        let bold = crate::advances(&f, &[gid], &located(&f, &heavy))[0];
        assert!(
            (bold - base - delta).abs() < 1e-9,
            "{cp}: the store says the advance moves by {delta} units, the engine moved it by {}",
            bold - base,
        );
        if delta != 0.0 {
            moved += 1;
        }
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} glyphs had ids, which is not a real face");
    assert!(moved >= 4, "only {moved} of {checked} advances varied, so the store went unexercised");
}

// An axis on which a region runs backward, or crosses zero with its peak off it, is ignored wherever the location
// lies, its scalar 1, as the variations overview's interpolation algorithm says (`variation_regions.rs`).
#[test]
fn a_malformed_region_ignores_its_axis() {
    use daegun::raw::format::{ItemVariationStore, RegionAxis};
    let region = |start, peak, end| vec![RegionAxis { start, peak, end }];
    let mut store = ItemVariationStore::default();
    store.regions = vec![region(-1.0, 1.0, 1.0), region(0.5, 0.25, 1.0), region(0.0, 1.0, 0.5)];
    store.axis_count = 1;
    for (at, want) in [(0.0, 1.0), (0.25, 1.0), (0.75, 1.0)] {
        let scalars = store.region_scalars(&[at]);
        assert_eq!(scalars, [want; 3], "at {at}");
    }
}

// Every region index and delta of the store, as the table stores them. Only this font and a fixture
// copy set the long-words flag: subtables 2, 6–12 and 22–27, with 6–12 also holding 16-bit deltas.
#[test]
fn a_long_word_variation_store_decodes_to_its_stored_values() {
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
    let colr = f.table(Tag::new(b"COLR")).expect("the variable COLR fixture carries COLR");
    let ivs_off = daegun::raw::bytes::read_u32_be(colr, 30).expect("COLR v1 header truncated") as usize;
    let store =
        daegun::raw::format::ItemVariationStore::parse(colr, ivs_off).expect("the store does not parse");

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

// An IVD with no regions still holds its items, each a row of no deltas.
#[test]
fn an_ivd_without_regions_keeps_its_rows() {
    let mut buf = vec![0u8; 22];
    daegun::raw::bytes::write_u16_be(&mut buf, 0, 1);
    daegun::raw::bytes::write_u32_be(&mut buf, 2, 12);
    daegun::raw::bytes::write_u16_be(&mut buf, 6, 1);
    daegun::raw::bytes::write_u32_be(&mut buf, 8, 16);
    daegun::raw::bytes::write_u16_be(&mut buf, 16, 3);
    let store = daegun::raw::format::ItemVariationStore::parse(&buf, 0).expect("the store parses");
    let ivd = &store.ivd_data[0];
    assert_eq!(ivd.rows(), 3);
    assert_eq!(ivd.row(2), Some(&[][..]));
    assert_eq!(ivd.row(3), None);
}

// IVD offsets may repeat, and each copy is held: 64 offsets at one IVD of 65,535 regions would hold 50 MB from
// 197 KB. A copy past four values a byte from the store's start, or a million rows, is absent, the store still read.
#[test]
fn a_variation_store_cannot_hold_far_more_than_its_bytes() {
    let store = |copies: usize, items: usize, regions: usize| {
        let ivd = 8 + copies * 4 + 4;
        let mut buf = vec![0u8; ivd + 6 + regions * 2 + items * regions];
        daegun::raw::bytes::write_u16_be(&mut buf, 0, 1);
        daegun::raw::bytes::write_u32_be(&mut buf, 2, (8 + copies * 4) as u32);
        daegun::raw::bytes::write_u16_be(&mut buf, 6, copies as u16);
        for i in 0..copies {
            daegun::raw::bytes::write_u32_be(&mut buf, 8 + i * 4, ivd as u32);
        }
        daegun::raw::bytes::write_u16_be(&mut buf, ivd, items as u16);
        daegun::raw::bytes::write_u16_be(&mut buf, ivd + 4, regions as u16);
        buf
    };
    let held = |buf: &[u8], base: usize| {
        let store = daegun::raw::format::ItemVariationStore::parse(buf, base).expect("the store parses");
        store.ivd_data.iter().filter(|ivd| ivd.rows() > 0).count()
    };
    let one = daegun::raw::format::ItemVariationStore::parse(&store(1, 1, 65_535), 0).expect("one wide IVD parses");
    assert_eq!(one.ivd_data[0].row(0).map(<[i32]>::len), Some(65_535), "the wide IVD lost its row");
    // Each copy of 65,535 values and as many region indices takes 131,070 of 787,516.
    assert_eq!(held(&store(64, 1, 65_535), 0), 6, "64 offsets at one wide IVD");
    // Callers pass the whole table, so the same store after 10 MiB of other data must hold no more.
    let mut table = vec![0u8; 10 << 20];
    table.extend(store(64, 1, 65_535));
    assert_eq!(held(&table, 10 << 20), 6, "the bytes before the store paid for its copies");
    // A row of no regions costs no bytes, so only the row budget bounds 640 copies of 65,535 of them.
    assert_eq!(held(&store(1, 65_535, 0), 0), 1, "one tall IVD");
    assert_eq!(held(&store(640, 65_535, 0), 0), 15, "640 offsets at one tall IVD of empty rows");
    // The base comes from the caller, from C as a size_t, so an offset near the top must decline.
    for base in [usize::MAX, usize::MAX - 1, usize::MAX - 7] {
        assert!(daegun::raw::format::ItemVariationStore::parse(&[0u8; 64], base).is_err(), "base {base:#x} parsed");
    }
}

#[test]
fn ot_round_breaks_ties_toward_positive_infinity() {
    for (v, want) in [(0.5, 1), (1.5, 2), (2.5, 3), (-0.5, 0), (-1.5, -1), (-2.5, -2)] {
        assert_eq!(daegun::raw::format::ot_round(v), want, "ot_round({v})");
    }
    assert_eq!(daegun::raw::format::ot_round(-0.5), 0);
    assert_eq!((-0.5f64).round() as i32, -1);
    for (v, want) in [(0.0, 0), (0.4, 0), (0.6, 1), (-0.4, 0), (-0.6, -1), (7.0, 7)] {
        assert_eq!(daegun::raw::format::ot_round(v), want, "ot_round({v})");
    }
}

#[test]
fn coverage_index_walks_a_real_lookup() {
    use daegun::raw::bytes::read_u16_be;
    let f = font(GARAMOND);
    let gsub = f.table(Tag::new(b"GSUB")).expect("EBGaramond carries GSUB");

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

        let mut seen: Vec<(u32, u32)> = Vec::new();
        for gid in 0..f.glyph_count() {
            if let Some(idx) = daegun::raw::format::coverage_index(cov, gid) {
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

    assert!(daegun::raw::format::coverage_index(&[0, 3, 0, 1, 0, 0], 0).is_none());
    assert!(daegun::raw::format::coverage_index(&[], 0).is_none());
}

#[test]
fn an_aat_lookup_enumerates_what_it_answers() {
    use daegun::raw::bytes::{read_u16_be, read_u32_be};
    let f = font(MORX);
    let morx = f.table(Tag::new(b"morx")).expect("TestMORXOne carries morx");

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

    let lookup = daegun::raw::format::Lookup::parse(body, f.glyph_count()).expect("the lookup does not parse");
    let entries = lookup.entries();
    assert!(!entries.is_empty(), "the lookup substitutes nothing");

    for &daegun::raw::format::GlyphValue { glyph: gid, value: to } in &entries {
        assert_eq!(lookup.value(gid), Some(to), "entries() and value() disagree about glyph {gid}");
        assert_ne!(u32::from(to), gid, "a substitution that changes nothing");
    }
    for gid in 0..f.glyph_count() {
        let listed = entries.iter().find(|e| e.glyph == gid).map(|e| e.value);
        assert_eq!(lookup.value(gid), listed, "value() covers glyph {gid} and entries() does not");
    }
    assert!(daegun::raw::format::Lookup::parse(&[0, 6], f.glyph_count()).ok().is_none_or(|l| l.entries().is_empty()));
    let kind = |r: Result<daegun::raw::format::Lookup<'_>, daegun::Error>| r.err().map(|e| e.kind());
    let parse = |data: &'static [u8]| kind(daegun::raw::format::Lookup::parse(data, 10));
    assert_eq!(parse(&[0]), Some(daegun::ErrorKind::Malformed), "a lookup too short to name its format");
    assert_eq!(parse(&[0, 7]), Some(daegun::ErrorKind::Unsupported), "a lookup of format 7");
    let ankr = daegun::raw::format::Ankr::parse(&[0; 6], 10).err().map(|e| e.kind());
    assert_eq!(ankr, Some(daegun::ErrorKind::Malformed), "an ankr cut short");

    assert!(read_u16_be(morx, 0).is_some());
}

#[test]
fn an_error_kind_reads_as_lowercase_text() {
    use daegun::ErrorKind;
    let kinds = [
        ErrorKind::Malformed, ErrorKind::Unsupported, ErrorKind::Missing, ErrorKind::InvalidArgument,
        ErrorKind::LimitExceeded, ErrorKind::NoOutline, ErrorKind::Overflow,
    ];
    for kind in kinds {
        let text = kind.to_string();
        assert!(!text.is_empty() && text == text.to_lowercase(), "{kind:?} reads {text:?}");
    }
}

#[test]
fn feature_variations_selects_by_axis_range() {
    let mut t = vec![0u8; 100];
    t[..4].copy_from_slice(&0x0001_0001u32.to_be_bytes());
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
    let table = daegun::raw::format::FeatureVariations::parse(&t).expect("a 1.1 header with an offset");

    assert_eq!(table.find(&[0], None), Some(1), "the empty condition set should match at the origin");
    assert_eq!(table.find(&[16384], None), Some(0), "axis 0 at 1.0 is inside [0.5, 1.0]");
    assert_eq!(table.find(&[8192], None), Some(0), "the range is inclusive at its minimum");
    assert_eq!(table.find(&[8191], None), Some(1), "8191 is below the range and must not match record 0");

    assert_eq!(table.substitute(0, 3), Some(daegun::raw::format::Substitution::Feature(100 + 40 + 12)));
    assert_eq!(table.substitute(0, 4), None, "feature 4 has no substitution");
    assert_eq!(table.substitute(1, 3), None, "record 1 declares no substitution table");

    assert!(daegun::raw::format::FeatureVariations::parse(&[0u8; 100]).is_none());
    // `at` can come from C as any size_t, and every offset is added to it.
    for at in [usize::MAX, usize::MAX - 1, usize::MAX - 7] {
        let far = daegun::raw::format::FeatureVariations::at(&t, at);
        assert_eq!((far.find(&[], None), far.substitute(0, 0)), (None, None), "at {at:#x} answered");
    }
    let mut null = t.clone();
    null[10..14].copy_from_slice(&0u32.to_be_bytes());
    assert!(daegun::raw::format::FeatureVariations::parse(&null).is_none());
}

#[test]
fn has_glyph_free_base_answers_for_the_font_that_has_one() {
    let serif = font(SOURCE_SERIF);
    assert!(serif.has_table(Tag::new(b"BASE")), "SourceSerif4Variable lost its BASE; pick another fixture");
    let free = serif.has_glyph_free_base();
    assert!(
        serif.base_script(Tag::new(b"latn"), &Location::default(), Orientation::Horizontal).is_some(),
        "BASE does not describe the Latin script, so nothing here is being read",
    );
    assert!(free, "SourceSerif4Variable's BASE names a glyph, which would be a first for a text face");

    let g = font(GARAMOND);
    assert!(!g.has_table(Tag::new(b"BASE")), "EBGaramond gained a BASE; pick another fixture");
    assert!(!g.has_glyph_free_base());
}

#[test]
fn the_default_point_size_is_what_trak_falls_back_to() {
    assert_eq!(daegun::metrics::DEFAULT_POINT_SIZE, 12.0);
    let f = font(GARAMOND);
    assert_eq!(f.tracking(daegun::metrics::DEFAULT_POINT_SIZE, Orientation::Horizontal), 0.0, "EBGaramond has no trak");
}

#[test]
fn a_font_rebuilt_from_its_tables_survives_an_edit() {
    use std::collections::BTreeMap;
    let f = font(GARAMOND);
    let gid = f.glyph_id('H').expect("EBGaramond has H");

    let mut tables: BTreeMap<Tag, Vec<u8>> = f
        .table_tags()
        .map(|t| (t, f.table(t).expect("listed but absent").to_vec()))
        .collect();

    let rebuilt = daegun::raw::build_font(&tables).unwrap();
    assert!(!rebuilt.is_empty(), "build_font produced nothing from a real font");
    let again = daegun::Font::from_bytes(&rebuilt).expect("the rebuilt font does not parse");
    assert!(again.table_tags().eq(f.table_tags()), "the rebuilt font lost or gained a table");
    assert_eq!(again.glyph_count(), f.glyph_count());
    assert_eq!(again.units_per_em(), f.units_per_em());
    assert_eq!(
        crate::advances(&again, &[GlyphId(gid.0)], &Location::default()),
        crate::advances(&f, &[GlyphId(gid.0)], &Location::default())
    );
    for tag in f.table_tags() {
        let (before, after) = (f.table(tag).expect("listed"), again.table(tag).expect("rebuilt"));
        if tag == Tag::new(b"head") {
            assert_eq!(before.len(), after.len(), "head changed length");
            assert_eq!(before[..8], after[..8], "head changed before checkSumAdjustment");
            assert_eq!(before[12..], after[12..], "head changed after checkSumAdjustment");
            continue;
        }
        assert_eq!(before, after, "{tag} did not survive the rebuild");
    }

    let original_upm = f.units_per_em();
    let head = tables.get_mut(&Tag::new(b"head")).expect("every font has head");
    daegun::raw::bytes::write_u16_be(head, 18, original_upm * 2);
    assert_eq!(
        daegun::raw::bytes::read_u16_be(head, 18),
        Some(original_upm * 2),
        "the writer and the reader disagree about the same two bytes",
    );

    let patched = daegun::raw::build_font(&tables).unwrap();
    let patched = daegun::Font::from_bytes(&patched).expect("the patched font parses");
    assert_eq!(patched.units_per_em(), original_upm * 2, "the edit did not reach the rebuilt font");
    let before = crate::advances(&f, &[gid], &Location::default())[0];
    let after = crate::advances(&patched, &[gid], &Location::default())[0];
    assert_eq!(after, before, "an advance in font units moved with the em: {before} became {after}");

    use daegun::raw::bytes::{write_i16_be, write_offset24, write_u16_be, write_u32_be};
    let mut two = [0u8; 2];
    assert!(!write_u16_be(&mut two, 1, 0xFFFF), "a write past the end said it wrote");
    assert!(!write_i16_be(&mut two, 1, -1) && !write_u32_be(&mut two, 0, 1) && !write_offset24(&mut two, 0, 1));
    assert_eq!(two, [0, 0], "a write past the end changed bytes it does not own");
    let mut four = [0u8; 4];
    assert!(!write_offset24(&mut four, 0, 1 << 24), "an offset past 24 bits was written");
    assert!(write_offset24(&mut four, 1, 0x01_0203) && write_u16_be(&mut four, 0, 0x0A0B));
    assert_eq!(four, [0x0A, 0x0B, 0x02, 0x03]);
}

// Every measurement rests on head's em and every glyph id on maxp's count, so a font missing either is
// refused once, at load, rather than measured on a guess.
#[test]
fn a_font_without_an_em_or_a_glyph_count_is_refused_at_load() {
    type Tables = std::collections::BTreeMap<Tag, Vec<u8>>;
    let f = font(GARAMOND);
    let tables: Tables =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    let loads = |edit: &dyn Fn(&mut Tables)| {
        let mut edited = tables.clone();
        edit(&mut edited);
        let bytes = daegun::raw::build_font(&edited).unwrap();
        let (borrowed, owned) = (Font::from_bytes(&bytes).is_ok(), Font::from_vec(bytes, 0).is_ok());
        assert_eq!(borrowed, owned, "from_bytes and from_vec disagree");
        borrowed
    };
    let em = |upm: u16| move |t: &mut Tables| {
        daegun::raw::bytes::write_u16_be(t.get_mut(&Tag::new(b"head")).expect("head"), 18, upm);
    };

    assert!(loads(&|_| {}), "the untouched font was refused");
    assert!(!loads(&|t| { t.remove(&Tag::new(b"head")); }), "a font with no head loaded");
    assert!(!loads(&em(0)), "a font whose unitsPerEm is 0 loaded");
    assert!(
        !loads(&|t| t.get_mut(&Tag::new(b"head")).expect("head").truncate(19)),
        "a head too short for its em loaded"
    );
    assert!(!loads(&|t| { t.remove(&Tag::new(b"maxp")); }), "a font with no maxp loaded");
    assert!(
        !loads(&|t| t.get_mut(&Tag::new(b"maxp")).expect("maxp").truncate(5)),
        "a maxp too short for its count loaded"
    );
    // Far outside the spec's 16 to 16384, but the em the hinting tests build their tiny fonts with.
    assert!(loads(&em(1)), "a font whose unitsPerEm is 1 was refused");

    // Apple's bitmap-only fonts, NISC18030 among them, keep the same header as bhed.
    let bitmap_only = |t: &mut Tables| {
        let head = t.remove(&Tag::new(b"head")).expect("head");
        t.insert(Tag::new(b"bhed"), head);
    };
    assert!(loads(&bitmap_only), "a font whose header is bhed was refused");
    let mut renamed = tables.clone();
    bitmap_only(&mut renamed);
    let g = Font::from_vec(daegun::raw::build_font(&renamed).unwrap(), 0).expect("loads");
    assert_eq!((g.units_per_em(), g.bounds()), (f.units_per_em(), f.bounds()), "bhed was not read as the header");
}

#[test]
fn instanced_tables_agree_with_the_instanced_metrics() {
    let f = font(INTER);
    let gid = f.glyph_id('H').expect("Inter has H");
    let axes = [("wght", 900.0)];

    let tables = f.instance_tables(&located(&f, &axes)).expect("instanced");
    for gone in [b"fvar", b"gvar", b"HVAR", b"MVAR", b"avar"].map(Tag::new) {
        assert!(!tables.contains_key(&gone), "{gone} survived instancing, so the font is still variable");
    }
    assert!(tables.contains_key(&Tag::new(b"glyf")) && tables.contains_key(&Tag::new(b"hmtx")));

    let cmap = tables.get(&Tag::new(b"cmap")).expect("cmap passes through instancing");
    assert!(matches!(cmap, std::borrow::Cow::Borrowed(_)), "cmap was copied rather than borrowed");
    assert_eq!(cmap.as_ref(), f.table(Tag::new(b"cmap")).expect("cmap"));
    let glyf = tables.get(&Tag::new(b"glyf")).expect("glyf");
    assert!(matches!(glyf, std::borrow::Cow::Owned(_)), "glyf came back borrowed, so nothing varied");
    assert_ne!(glyf.as_ref(), f.table(Tag::new(b"glyf")).expect("glyf"), "the instanced glyf is the stored one");

    let rebuilt = daegun::Font::from_bytes(&daegun::raw::build_font(&tables).unwrap()).expect("the instance parses");
    assert!(!rebuilt.is_variable(), "the rebuilt instance still claims to be variable");
    assert_eq!(rebuilt.units_per_em(), f.units_per_em());
    let direct = crate::advances(&f, &[gid], &located(&f, &axes))[0];
    let through_tables = crate::advances(&rebuilt, &[gid], &Location::default())[0];
    assert!(
        (direct - through_tables).abs() <= 1.0,
        "instance_tables says {through_tables} where advances says {direct}",
    );
    let default_advance = crate::advances(&f, &[gid], &Location::default())[0];
    assert!(
        (through_tables - default_advance).abs() > 1.0,
        "the instanced advance equals the default, so nothing was resolved",
    );
}

#[test]
fn a_hinted_glyph_is_geometry_in_device_space() {
    use daegun::outline::HintMode;
    let f = font(GARAMOND);
    let gid = f.glyph_id('H').expect("EBGaramond has H");

    assert!(
        f.hinted(gid, &Location::default(), 16.0, HintMode::Font).is_none(),
        "EBGaramond gained hinting bytecode; pick another fixture",
    );
    let out = f.hinted(gid, &Location::default(), 16.0, HintMode::AutoForce).expect("the autohinter hints H");
    let points: Vec<_> = out.points().collect();
    assert!(!points.is_empty(), "a hinted outline with no points");
    assert!(!out.contours().is_empty(), "no contours");
    assert!(
        out.contours().iter().all(|&e| e < points.len()),
        "a contour ends past the last point",
    );
    assert!(points.iter().any(|p| p.1 == daegun::outline::PointKind::OnCurve), "no on-curve point");

    let mut path = daegun::outline::Path::default();
    daegun::outline::HintedOutline::draw(&out, &mut path);
    let Bounds { x_min: x0, y_min: y0, x_max: x1, y_max: y1 } = path.bounds().expect("the replayed outline has bounds");
    assert!(x1 > x0 && y1 > y0, "the hinted box is empty: {x0},{y0},{x1},{y1}");
    assert!(
        (y1 - y0) > 4.0 && (y1 - y0) < 20.0,
        "an H at 16px is {} tall, which is not pixels",
        y1 - y0,
    );
    let big = f.hinted(gid, &Location::default(), 64.0, HintMode::AutoForce).expect("hinted at 64px");
    let mut big_path = daegun::outline::Path::default();
    daegun::outline::HintedOutline::draw(&big, &mut big_path);
    let Bounds { x_min: _, y_min: by0, x_max: _, y_max: by1 } = big_path.bounds().expect("bounds");
    assert!(
        (by1 - by0) > (y1 - y0) * 3.0,
        "64px is {} tall against 16px at {}, which is not a scale",
        by1 - by0,
        y1 - y0,
    );

    assert!(f.hinted(gid, &Location::default(), 16.0, HintMode::None).is_none());
    // A px no size has is refused, as prepared_outline refuses it.
    for px in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let refused = f.hinted_outline(gid, &Location::default(), px, HintMode::AutoForce);
        assert!(refused.is_err_and(|e| e.kind() == daegun::ErrorKind::InvalidArgument), "px {px} was hinted");
    }
    assert!(
        f.hinted(GlyphId(u16::MAX.into()), &Location::default(), 16.0, HintMode::AutoForce).is_none(),
        "a gid past the end"
    );

    let hinted = font("test-fixtures/hinted.ttf");
    assert!(
        hinted.hinted(GlyphId(1), &Location::default(), 16.0, HintMode::Font).is_some(),
        "the hinted fixture lost its bytecode",
    );
}

#[test]
fn cff_hints_report_the_declared_stems() {
    let f = font(STIX);
    let (gid, hints) = (0..f.glyph_count())
        .filter_map(|g| f.cff_hints(GlyphId(g), &Location::default()).map(|h| (g, h)))
        .find(|(_, h)| !h.stems.is_empty())
        .expect("no STIX2Math glyph declares a stem, which is not a hinted CFF font");

    // Not min <= max: an edge hint keeps its negative width, as the test below pins.
    for s in &hints.stems {
        assert!(s.min.is_finite() && s.max.is_finite(), "glyph {gid} declares a stem from {} to {}", s.min, s.max);
    }
    for mask in &hints.masks {
        assert!(mask.point <= 100_000, "a hintmask at point {}", mask.point);
        assert_eq!(mask.bits.len(), hints.stems.len().div_ceil(8), "a mask that does not cover the stems");
    }

    let g = font(GARAMOND);
    assert!(!g.has_table(Tag::new(b"CFF ")), "EBGaramond gained a CFF table; pick another fixture");
    assert!(g.cff_hints(g.glyph_id('H').expect("H"), &Location::default()).is_none());
}

// A CFF2 font's stems blend at a location as its outline does: Source Serif's H at its default and at wght 900, each
// edge the charstring's stored value with its stored delta added there.
#[test]
fn cff2_stems_blend_at_a_location() {
    type Stem = (f32, f32, bool);
    const DEFAULT: &[Stem] = &[(0.0, 46.2831, false), (319.0, 366.0, false), (623.7178, 670.0, false),
        (138.0, 234.7235, true), (555.9996, 649.9999, true)];
    const HEAVY: &[Stem] = &[(0.0, 62.0978, false), (304.0, 367.0, false), (589.9635, 652.0, false),
        (96.0, 294.0003, true), (460.5421, 660.0003, true)];
    let serif = font("source-serif/SourceSerif4Variable-Roman.otf");
    let h = serif.glyph_id('H').expect("H");
    for (at, want) in [(Location::default(), DEFAULT), (serif.location(&[(Tag::new(b"wght"), 900.0)]), HEAVY)] {
        let hints = serif.cff_hints(h, &at).expect("a CFF2 font gave no hints");
        let got: Vec<Stem> = hints.stems.iter().map(|s| (s.min, s.max, s.vertical)).collect();
        let near = got.len() == want.len()
            && got.iter().zip(want).all(|(g, w)| (g.0 - w.0).abs() < 1e-3 && (g.1 - w.1).abs() < 1e-3 && g.2 == w.2);
        assert!(near, "at {at:?}: {got:?}, where the charstring's values give {want:?}");
    }
}

// Stem edges as the charstrings store them: within each stem operator the pairs
// run on from the previous edge, and an edge hint keeps its negative width, as A's (662, 642) does.
#[test]
fn cff_stems_decode_to_the_edges_the_charstrings_store() {
    type Stem = (f32, f32, bool);
    const EXPECTED: &[(char, u32, &[Stem])] = &[
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
        assert_eq!(f.glyph_id(ch), Some(GlyphId(gid)), "{ch} maps to a different glyph than the cmap holds");
        let hints = f.cff_hints(GlyphId(gid), &Location::default());
        let hints = hints.unwrap_or_else(|| panic!("{ch} has no CFF hints"));
        let got: Vec<Stem> = hints.stems.iter().map(|s| (s.min, s.max, s.vertical)).collect();
        assert_eq!(got, want, "{ch}: stems");
    }
}

#[test]
fn coverage_glyphs_agrees_with_coverage_index() {
    use daegun::raw::bytes::read_u16_be;
    let f = font(GARAMOND);
    let gsub = f.table(Tag::new(b"GSUB")).expect("EBGaramond carries GSUB");
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

        let Ok(glyphs) = daegun::raw::format::coverage_glyphs(gsub, at) else { continue };
        if glyphs.is_empty() {
            continue;
        }
        for w in glyphs.windows(2) {
            assert!(w[0] < w[1], "coverage is not strictly ascending at {}", w[0]);
        }
        let cov = &gsub[at..];
        for (n, &g) in glyphs.iter().enumerate() {
            assert_eq!(
                daegun::raw::format::coverage_index(cov, g),
                u32::try_from(n).ok(),
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
    use daegun::raw::bytes::read_u32_be;
    use daegun::raw::format::{class, state, Lookup, StateTable};

    assert_eq!(class::END_OF_TEXT, 0);
    assert_eq!(class::DELETED_GLYPH, 2);
    assert_eq!(state::START_OF_TEXT, 0);

    let f = font("aat/TestMORXTwentyfour.ttf");
    let morx = f.table(Tag::new(b"morx")).expect("carries morx");
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

    let table = StateTable::parse(body, 2, f.glyph_count()).expect("the state table parses");
    // `extra_words` comes from C as any size_t, and an entry's stride is 4 + 2 * extra_words bytes.
    for words in [usize::MAX, 1 << (usize::BITS - 1), usize::MAX / 2] {
        let refused = StateTable::parse(body, words, f.glyph_count()).err().map(|e| e.kind());
        assert_eq!(refused, Some(daegun::ErrorKind::InvalidArgument), "{words} extra words were accepted");
    }
    assert_eq!(table.class(0xFFFF), class::DELETED_GLYPH);
    let reachable = (0..f.glyph_count())
        .filter(|&g| table.class(g) > class::DELETED_GLYPH)
        .filter_map(|g| table.entry(state::START_OF_TEXT, table.class(g)))
        .count();
    assert!(reachable > 0, "no glyph's class has an entry in the start state");

    let class_table = u32::from_be_bytes(body[4..8].try_into().unwrap()) as usize;
    let classes = Lookup::parse(&body[class_table..], f.glyph_count()).expect("the class table parses").entries();
    assert!(!classes.is_empty() && classes.len() <= f.glyph_count() as usize);
    assert!(classes.iter().all(|e| table.class(e.glyph) == e.value),
        "the class lookup disagrees with the state table",
    );

    assert_eq!(daegun::raw::format::Ankr::version(&[0, 0, 0, 1]), Some(0));
    assert_eq!(daegun::raw::format::Ankr::version(&[0]), None);
    let point = daegun::raw::format::AnchorPoint { x: -2, y: 5 };
    assert_eq!(daegun::raw::format::control_point(&[0xFF, 0xFE, 0x00, 0x05], 0), Some(point));
    assert_eq!(daegun::raw::format::control_point(&[0, 0], 0), None);
}

// usWeightClass, usWidthClass and fsType read as stored, and at a location as its instance has them: Inter's weight
// follows wght there.
#[test]
fn os2_reads_the_weight_width_and_embedding() {
    let f = font(INTER);
    let os2 = f.os2(&Location::default()).expect("InterVariable carries OS/2");
    assert_eq!((os2.weight_class, os2.width_class, os2.fs_type), (Some(400), Some(5), Some(0)));
    let bold = f.location(&[(Tag::new(b"wght"), 700.0)]);
    let instance = Font::from_bytes(&f.instance(&bold).expect("instanced")).expect("the instance loads");
    let at = f.os2(&bold).expect("OS/2 at wght 700");
    assert_eq!(at.weight_class, Some(700));
    assert_eq!(Some(at), instance.os2(&Location::default()));
}

#[test]
fn os2_reports_the_typographic_line_box() {
    let f = font(INTER);
    let os2 = f.os2(&Location::default()).expect("InterVariable carries OS/2");
    let typo = os2.typo_metrics.expect("Inter states sTypoAscender/Descender/LineGap");

    assert!(typo.ascent > 0.0, "the typographic ascender is above the baseline");
    assert!(typo.descent < 0.0, "the typographic descender is below it, and signed");
    assert!(typo.line_gap >= 0.0);

    let raw = f.table(Tag::new(b"OS/2")).expect("OS/2");
    for (offset, got, name) in [
        (68usize, typo.ascent, "sTypoAscender"),
        (70, typo.descent, "sTypoDescender"),
        (72, typo.line_gap, "sTypoLineGap"),
    ] {
        let stored = daegun::raw::bytes::read_i16_be(raw, offset).expect("OS/2 truncated");
        assert_eq!(got, f64::from(stored), "{name}: {got} where OS/2 stores {stored}");
    }

    let line = f.line_metrics(&Location::default(), Orientation::Horizontal);
    if os2.uses_typo_metrics() {
        assert!(
            (line.ascent - typo.ascent).abs() <= 1.0,
            "the font asks for typo metrics but line_metrics reports {} against {}",
            line.ascent,
            typo.ascent,
        );
    }
}

fn closure_of(f: &Font, gids: &[GlyphId]) -> Vec<GlyphId> {
    let closure = f.glyph_closure(gids, &Location::default()).expect("the closure converges");
    assert!(closure.contains(&GlyphId(0)), "the closure dropped .notdef");
    for w in closure.windows(2) {
        assert!(w[0] < w[1], "the closure is not strictly ascending at {}", w[0]);
    }
    for g in gids {
        assert!(closure.contains(g), "the closure dropped requested glyph {g}");
    }
    closure
}

// A context's nested lookups close only where its rule can match: "किसी भी" keeps 12 glyphs, with the default
// features or every one, and U+25CC, which a text with marks keeps for a broken cluster.
#[test]
fn a_text_subset_closes_contexts_where_their_rules_match() {
    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let out = f.subset_text("किसी भी", &Location::default()).expect("subsets");
    let kept: Vec<u32> = (0..f.glyph_count()).filter(|&g| out.renumbered(GlyphId(g)).is_some()).collect();
    assert_eq!(kept, [0, 3, 32, 33, 56, 79, 87, 545, 546, 549, 550, 634, f.glyph_id('\u{25CC}').expect("U+25CC").0]);
}

#[test]
fn the_glyph_closure_is_exactly_what_a_subset_keeps() {
    for face in [GARAMOND, INTER] {
        let f = font(face);
        let gids: Vec<GlyphId> = "fifl AVTo".chars().filter_map(|c| f.glyph_id(c)).collect();
        assert!(gids.len() > 5, "{face}: only {} of the sample glyphs resolved", gids.len());

        let closure = closure_of(&f, &gids);
        let result = f.subset(&gids, &Location::default()).expect("the subset builds");
        let kept: Vec<GlyphId> =
            (0..f.glyph_count()).map(GlyphId).filter(|&g| result.renumbered(g).is_some()).collect();
        assert_eq!(closure, kept, "{face}: the closure and the subset disagree about what survives");

        assert!(
            closure.len() > gids.len() + 1,
            "{face}: {} requested became {}, so nothing was reached",
            gids.len(),
            closure.len(),
        );
        assert!(closure.len() < f.glyph_count() as usize, "{face}: the closure kept the whole font");

        let built = daegun::Font::from_bytes(&result.font).expect("the subset parses");
        assert_eq!(
            built.glyph_count() as usize,
            closure.len(),
            "{face}: the subset has {} glyphs where the closure named {}",
            built.glyph_count(),
            closure.len(),
        );
    }
}

#[test]
fn the_closure_reaches_components_and_stretchy_variants() {
    use daegun::raw::bytes::{read_i16_be, read_u16_be, read_u32_be};

    let f = font(GARAMOND);
    let (glyf, loca, head) = (
        f.table(Tag::new(b"glyf")).expect("glyf"),
        f.table(Tag::new(b"loca")).expect("loca"),
        f.table(Tag::new(b"head")).expect("head"),
    );
    let long_loca = read_i16_be(head, 50).expect("indexToLocFormat") != 0;
    let offset_of = |g: usize| {
        if long_loca {
            read_u32_be(loca, g * 4).map(|v| v as usize)
        } else {
            read_u16_be(loca, g * 2).map(|v| v as usize * 2)
        }
    };

    let composite = (0..f.glyph_count() as usize)
        .find(|&g| {
            let (Some(s), Some(e)) = (offset_of(g), offset_of(g + 1)) else { return false };
            s < e && read_i16_be(glyf, s) == Some(-1)
        })
        .expect("EBGaramond has no composite glyph, which is not a real text face");

    let mut components: Vec<u32> = Vec::new();
    let mut pos = offset_of(composite).expect("offset") + 10;
    while let (Some(flags), Some(gid)) = (read_u16_be(glyf, pos), read_u16_be(glyf, pos + 2)) {
        components.push(u32::from(gid));
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

    let closure = closure_of(&f, &[GlyphId(composite as u32)]);
    for &c in &components {
        assert!(
            closure.contains(&GlyphId(c)),
            "component {c} of composite {composite} is missing from its own closure {closure:?}",
        );
    }

    let m = font(STIX);
    let stretchy = (0..m.glyph_count())
        .find(|&g| {
            m.math_glyph_variants(GlyphId(g), &Location::default(), Orientation::Vertical)
                .is_some_and(|c| !c.variants.is_empty())
        })
        .expect("STIX2Math has no vertically stretchy glyph");
    let variants: Vec<GlyphId> = m
        .math_glyph_variants(GlyphId(stretchy), &Location::default(), Orientation::Vertical)
        .expect("variants")
        .variants
        .iter()
        .map(|v| v.glyph)
        .collect();
    assert!(variants.len() > 1, "only {} variant, so nothing is pulled in", variants.len());

    let math_closure = closure_of(&m, &[GlyphId(stretchy)]);
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
    assert!(f.has_table(Tag::new(b"CFF ")), "STIX2Math is the CFF fixture");
    let gids: Vec<GlyphId> = "xyz+=".chars().filter_map(|c| f.glyph_id(c)).collect();
    assert!(gids.len() >= 4, "only {} of the sample glyphs resolved", gids.len());

    let result = f.subset(&gids, &Location::default()).expect("the subset builds");
    assert_eq!(
        Font::from_bytes(&result.font).expect("the subset parses").glyph_count(),
        f.glyph_count(),
        "the CFF subset now compacts, so the closure's asymmetry with it is stale",
    );

    let closure = closure_of(&f, &gids);
    assert!(closure.len() > gids.len(), "the closure reached nothing beyond the request");
    assert!(closure.len() < f.glyph_count() as usize, "the closure kept the whole font");
}

// A set of glyphs, an array and a `Vec` go in as they are, by value or by reference, and agree.
#[test]
fn glyphs_go_in_as_any_collection() {
    let f = font(GARAMOND);
    let at = Location::default();
    let gids: Vec<GlyphId> = "Type".chars().filter_map(|c| f.glyph_id(c)).collect();
    let set: std::collections::HashSet<GlyphId> = gids.iter().copied().collect();
    let closure = f.glyph_closure(&gids, &at).expect("a closure");
    assert_eq!(f.glyph_closure(&set, &at).expect("a closure"), closure);
    assert_eq!(f.glyph_closure([gids[0], gids[1], gids[2], gids[3]], &at).expect("a closure"), closure);
    let subset = f.subset(set, &at).expect("a subset");
    assert_eq!(subset, f.subset(gids.clone(), &at).expect("a subset"));
    // Equal subsets hash alike, so one keys a map.
    let mut kept = std::collections::HashSet::new();
    assert!(kept.insert(subset) && !kept.insert(f.subset(gids, &at).expect("a subset")));
}

#[test]
fn the_closure_handles_the_edges() {
    let f = font(GARAMOND);

    let empty = f.glyph_closure(core::iter::empty::<GlyphId>(), &Location::default()).expect("empty request");
    assert_eq!(empty, vec![GlyphId(0)]);

    let h = f.glyph_id('H').expect("H");
    let real = f.glyph_closure([h], &Location::default()).expect("H");
    let padded =
        f.glyph_closure([h, GlyphId(u16::MAX.into())], &Location::default()).expect("H plus a gid past the end");
    assert_eq!(real, padded, "a gid the font does not have changed the closure");

    let v = font(INTER);
    let vh = v.glyph_id('H').expect("Inter has H");
    for wght in [100.0, 900.0] {
        let c = v.glyph_closure([vh], &located(&v, &[("wght", wght)])).expect("closes at every location");
        assert!(c.contains(&vh) && c.contains(&GlyphId(0)), "wght {wght} lost a requested glyph");
    }
}

#[test]
fn an_out_of_range_request_is_ignored_by_both_flavors() {
    for (face, ch) in [(GARAMOND, 'H'), (STIX, 'x'), (INTER, 'H')] {
        let f = font(face);
        let gid = f.glyph_id(ch).unwrap_or_else(|| panic!("{face} has no {ch}"));

        let real = f.glyph_closure([gid], &Location::default()).expect("closes");
        let padded = f.glyph_closure([gid, GlyphId(u16::MAX.into())], &Location::default()).expect("closes");
        assert_eq!(real, padded, "{face}: a gid past the end changed the closure");

        let n = f.glyph_count();
        for g in &padded {
            assert!(*g < GlyphId(n), "{face}: the closure names glyph {g} of {n}");
        }
        assert_eq!(
            f.glyph_closure([GlyphId(u16::MAX.into()), GlyphId(n)], &Location::default()).expect("closes"),
            vec![GlyphId(0)],
            "{face}: an entirely out-of-range request produced more than .notdef",
        );
    }
}

#[test]
fn instance_tables_answers_for_a_static_font_too() {
    for face in [STIX, "test-fixtures/hinted.ttf"] {
        let f = font(face);
        assert!(!f.is_variable(), "{face} is the static fixture");

        let tables = f.instance_tables(&Location::default()).expect("instanced");
        assert_eq!(tables.len(), f.table_tags().len(), "{face}: a table went missing");
        for tag in f.table_tags() {
            let got = tables.get(&tag).unwrap_or_else(|| panic!("{face}: {tag} missing"));
            assert_eq!(got.as_ref(), f.table(tag).expect("listed"), "{face}: {tag} changed");
            assert!(matches!(got, std::borrow::Cow::Borrowed(_)), "{face}: {tag} was copied");
        }
        assert_eq!(
            f.instance_tables(&located(&f, &[("wght", 900.0)])).expect("instanced").len(),
            f.table_tags().len(),
            "{face}: naming an axis it does not have changed the answer",
        );
    }
}

// EB Garamond has no programs. The font's own hints leave its first composite where scaling puts it, and the
// autohinter hints it.
#[test]
fn a_composite_without_programs_is_left_by_its_font_and_hinted_by_the_autohinter() {
    use daegun::raw::bytes::{read_i16_be, read_u16_be, read_u32_be};
    let f = font(GARAMOND);
    let (glyf, loca, head) = (
        f.table(Tag::new(b"glyf")).expect("glyf"),
        f.table(Tag::new(b"loca")).expect("loca"),
        f.table(Tag::new(b"head")).expect("head"),
    );
    let long_loca = read_i16_be(head, 50).expect("indexToLocFormat") != 0;
    let offset_of = |g: usize| {
        if long_loca {
            read_u32_be(loca, g * 4).map(|v| v as usize)
        } else {
            read_u16_be(loca, g * 2).map(|v| v as usize * 2)
        }
    };
    let composite = (0..f.glyph_count() as usize)
        .find(|&g| {
            let (Some(s), Some(e)) = (offset_of(g), offset_of(g + 1)) else { return false };
            s < e && read_i16_be(glyf, s) == Some(-1)
        })
        .expect("EBGaramond has no composite glyph") as u32;

    assert!(
        f.hinted(GlyphId(composite), &Location::default(), 16.0, HintMode::Font).is_none(),
        "the font's own hints moved composite {composite}"
    );
    for mode in [HintMode::Auto, HintMode::AutoForce] {
        let out = f
            .hinted(GlyphId(composite), &Location::default(), 16.0, mode)
            .unwrap_or_else(|| panic!("the autohinter declined composite {composite} under {mode:?}"));
        assert!(out.points().next().is_some(), "{mode:?}: a hinted composite with no points");
    }
}

#[test]
fn each_hinting_mode_tries_its_own_set_of_strategies() {
    use daegun::outline::HintMode;

    let cff = font(STIX);
    let (gid, hints) = (0..cff.glyph_count())
        .filter_map(|g| cff.cff_hints(GlyphId(g), &Location::default()).map(|h| (g, h)))
        .find(|(_, h)| h.stems.iter().any(|s| !s.vertical))
        .expect("STIX2Math declares no horizontal stems anywhere");
    assert!(!hints.stems.is_empty());

    // Font takes the font's own hints, CFF's stems as well as bytecode.
    for mode in [HintMode::Font, HintMode::Auto, HintMode::AutoForce] {
        assert!(
            cff.hinted(GlyphId(gid), &Location::default(), 16.0, mode).is_some(),
            "{mode:?} hinted nothing on a CFF face that declares stems",
        );
    }
    assert!(cff.hinted(GlyphId(gid), &Location::default(), 16.0, HintMode::None).is_none());

    let glyf = font(GARAMOND);
    let h = glyf.glyph_id('H').expect("EBGaramond has H");
    assert!(
        glyf.hinted(h, &Location::default(), 16.0, HintMode::Font).is_none(),
        "EBGaramond gained hinting bytecode; pick another fixture",
    );
    assert!(glyf.hinted(h, &Location::default(), 16.0, HintMode::Auto).is_some());

    let hinted = font("test-fixtures/hinted.ttf");
    assert!(
        hinted.hinted(GlyphId(1), &Location::default(), 16.0, HintMode::Font).is_some(),
        "the hinted fixture lost its bytecode",
    );
}

#[test]
fn the_vertical_advance_agrees_with_shaping() {
    let f = font(GARAMOND);
    assert!(!f.has_table(Tag::new(b"vmtx")), "EBGaramond gained vmtx; pick another fixture");
    let gid = f.glyph_id('H').expect("EBGaramond has H");

    let advance = f.advance(gid, &Location::default(), Orientation::Vertical);
    assert!(advance > 0.0, "a glyph set vertically advanced by nothing");
    let shaped = f.shape("H", &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom));
    assert_eq!(shaped.glyphs.len(), 1, "H shaped to more than one glyph");
    assert!(
        (advance - shaped.advances[0]).abs() <= 1.0,
        "the vertical advance is {advance}, shaping says {}",
        shaped.advances[0],
    );
    assert_eq!(
        advance,
        f.ascender(&Location::default()) - f.descender(&Location::default()),
        "the fallback is not ascender-to-descender",
    );

    let cjk = font("source-han-sans/SourceHanSansJP-VF.otf");
    assert!(cjk.has_table(Tag::new(b"vmtx")), "the CJK fixture lost vmtx");
    let cg = cjk.glyph_id('あ').expect("SourceHanSans has あ");
    let cjk_advance = cjk.advance(cg, &Location::default(), Orientation::Vertical);
    assert!(cjk_advance > 0.0);
    let cjk_shaped =
        cjk.shape("あ", &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom));
    assert!(
        (cjk_advance - cjk_shaped.advances[0]).abs() <= 1.0,
        "the vertical advance is {cjk_advance}, shaping says {}",
        cjk_shaped.advances[0],
    );
    assert_ne!(
        cjk_advance,
        cjk.ascender(&Location::default()) - cjk.descender(&Location::default()),
        "the CJK face's stated advance happens to equal the fallback, so this proves nothing",
    );

    // vmtx gives the vertical form of the Hangul tone mark U+302E an advance of 0, and 0 is what it means,
    // not a gap for the fallback to fill.
    let run = cjk.shape(
        "\u{3042}\u{302E}",
        &Location::default(),
        &ShapeOptions::default().with_direction(Direction::TopToBottom),
    );
    assert_eq!(run.glyphs.len(), 2, "あ and the tone mark shaped to {:?}", run.glyphs);
    let mark = run.glyphs[1];
    assert_eq!(cjk.advance(mark, &Location::default(), Orientation::Vertical), 0.0, "a stated 0 became the fallback");
    assert_eq!(run.advances[1], 0.0, "shaping advanced a stated 0 by a line");
    let mut sink = daegun::outline::Path::default();
    let prepared =
        cjk.prepared_outline(mark, &Location::default(), 16.0, &daegun::outline::OutlineOptions::default(), &mut sink);
    assert_eq!(prepared.map(|p| p.advance_height).ok(), Some(0.0), "prepared_outline disagrees");
}

// A batch of vertical advances is the per-glyph call's, vmtx's or the line's, at the default and off it.
#[test]
fn vertical_advances_in_a_batch_are_each_glyphs() {
    for rel in ["inter/InterVariable.ttf", "source-han-sans/SourceHanSansJP-VF.otf"] {
        let f = font(rel);
        let glyphs: Vec<GlyphId> = (0..f.glyph_count().min(300)).map(GlyphId).collect();
        for at in [Location::default(), located(&f, &[("wght", 700.0)])] {
            let mut batch = Vec::new();
            f.advances(&glyphs, &at, Orientation::Vertical, &mut batch);
            let each: Vec<f64> = glyphs.iter().map(|&g| f.advance(g, &at, Orientation::Vertical)).collect();
            assert_eq!(batch, each, "{rel}");
        }
    }
}

// Without vmtx a glyph advances its font extents' height: TestShapeLana sets USE_TYPO_METRICS, so 1,556 + 1,600
// gives 3,156 where hhea's would give 4,311.
#[test]
fn the_vertical_fallback_is_the_font_extents_height() {
    let f = font("structure/TestShapeLana.ttf");
    let run = f.shape("ab", &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom));
    assert_eq!(run.advances, [3156.0; 2]);
    assert_eq!(f.advance(run.glyphs[0], &Location::default(), Orientation::Vertical), 3156.0);
}

// Extents of 0 advance 0 down a line, with no em standing in for them.
#[test]
fn extents_of_0_advance_0_down_a_line() {
    let mut hhea = font("test-fixtures/carets.ttf").table(Tag::new(b"hhea")).expect("carets.ttf has hhea").to_vec();
    hhea[4..8].fill(0);
    let f = super::aat_subset::carets_with(&[("hhea", hhea)]);
    let run = f.shape("fil", &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom));
    assert_eq!(run.advances, [0.0; 3]);
    assert_eq!(f.advance(run.glyphs[0], &Location::default(), Orientation::Vertical), 0.0);
}

// GDEF classes f a mark (class 3), so shaping zeroes its advance, at any table lengths: GDEF at 188 or 189 bytes,
// GSUB at 248 and GPOS at 3,852 read as any others do.
#[test]
fn a_gdef_reads_at_any_length() {
    use super::aat_shaping::{gdef, gpos_pair};
    use super::aat_subset::{carets_with, shapes_alike, F, L};
    let padded = |mut table: Vec<u8>, len: usize| {
        table.resize(len, 0);
        table
    };
    let gsub =
        padded(font("test-fixtures/carets.ttf").table(Tag::new(b"GSUB")).expect("carets.ttf has GSUB").to_vec(), 248);
    let gpos = padded(gpos_pair(b"kern", F, L, 0), 3852);
    for gdef_len in [188, 189] {
        let gdef = padded(gdef(&[(F, F, 3)]), gdef_len);
        let f = carets_with(&[("GDEF", gdef), ("GSUB", gsub.clone()), ("GPOS", gpos.clone())]);
        assert_eq!(
            f.shape("fl", &Location::default(), &ShapeOptions::default()).advances,
            [0.0, 400.0],
            "GDEF of {gdef_len} bytes"
        );
        assert_eq!(f.glyph_class(GlyphId(F.into())), Some(daegun::GlyphClass::Mark), "GDEF of {gdef_len} bytes");
        shapes_alike(&f, "fl");
    }
}

// carets.ttf with a wght axis, `gpos`, and a GDEF 1.3 classing f as a mark, whose store moves the kern of `gpos`'s
// device by -40 at 900.
fn varied_by_its_gdef(gpos: Vec<u8>) -> Font {
    use super::aat_subset::{carets_with, u16s, u32s, F};
    let fvar = [u16s(&[1, 0, 16, 2, 1, 20, 0, 8]), b"wght".to_vec(), u32s(&[100 << 16, 400 << 16, 900 << 16]),
        u16s(&[0, 256])].concat();
    let store = [u16s(&[1]), u32s(&[12]), u16s(&[1]), u32s(&[22]), u16s(&[1, 1, 0, 0x4000, 0x4000]),
        u16s(&[1, 1, 1, 0, -40i16 as u16])].concat();
    let gdef = [u16s(&[1, 3, 18, 0, 0, 0, 0]), u32s(&[28]), u16s(&[2, 1, F, F, 3]), store].concat();
    carets_with(&[("fvar", fvar), ("GDEF", gdef), ("GPOS", gpos)])
}

// f is a mark at every location and in its instance, so its advance is zeroed whatever the kern's device moves it
// by, in place or from a record that widens GPOS.
#[test]
fn a_gdef_mark_is_zeroed_at_every_location() {
    use super::aat_shaping::gpos_pair_set;
    use super::aat_subset::{F, L};
    for (format, set) in [(0x44, &[1, L, 0, 8, 0, 0, 0x8000][..]), (0x40, &[1, L, 6, 0, 0, 0x8000])] {
        let f = varied_by_its_gdef(gpos_pair_set(b"kern", F, format, set));
        for axes in [&[][..], &[("wght", 900.0)]] {
            assert_eq!(
                f.shape("fl", &located(&f, axes), &ShapeOptions::default()).advances,
                [0.0, 400.0],
                "value format {format:#x} at {axes:?}"
            );
        }
        let instance =
            Font::from_bytes(&f.instance(&located(&f, &[("wght", 900.0)])).expect("an instance")).expect("it parses");
        assert_eq!(
            instance.shape("fl", &Location::default(), &ShapeOptions::default()).advances,
            [0.0, 400.0],
            "value format {format:#x}'s instance"
        );
    }
}

// An instance clears the VariationIndex tables of the store its GDEF loses, at the default too: wght 100 lies outside
// the store's one region, so its deltas are 0 as the default's, and both instances' GPOS are one, as are both subsets'.
#[test]
fn an_instance_at_the_default_clears_its_variation_indexes() {
    use super::aat_shaping::gpos_pair_set;
    use super::aat_subset::{F, L};
    let f = varied_by_its_gdef(gpos_pair_set(b"kern", F, 0x44, &[1, L, 0, 8, 0, 0, 0x8000]));
    let gpos = |wght: f64| {
        let instance =
            Font::from_bytes(&f.instance(&located(&f, &[("wght", wght)])).expect("an instance")).expect("it parses");
        instance.table(Tag::new(b"GPOS")).map(<[u8]>::to_vec)
    };
    assert_eq!(gpos(400.0), gpos(100.0));
    assert_ne!(gpos(400.0).as_deref(), f.table(Tag::new(b"GPOS")));
    let subset = |wght: f64| {
        let all: Vec<GlyphId> = (0..f.glyph_count()).map(GlyphId).collect();
        let sub = Font::from_vec(f.subset(&all, &located(&f, &[("wght", wght)])).expect("a subset").font, 0)
            .expect("it parses");
        sub.table(Tag::new(b"GPOS")).map(<[u8]>::to_vec)
    };
    assert_eq!(subset(400.0), subset(100.0));
}

// The budget grows with the run in 64 bits, so 600,000 glyphs ligate throughout as 200,000 do.
#[test]
fn a_long_run_ligates_throughout() {
    use super::aat_subset::F_I;
    let f = font("test-fixtures/carets.ttf");
    for pairs in [100_000, 300_000] {
        let run = f.shape(&"fi".repeat(pairs), &Location::default(), &ShapeOptions::default());
        assert!(run.complete, "{pairs} pairs");
        assert_eq!(run.glyphs.len(), pairs);
        assert!(run.glyphs.iter().all(|&g| g == GlyphId(F_I.into())), "{pairs} pairs");
    }
}

// Inter, without vmtx, its xhgt record retagged hasc and moved first, where the search by tag finds it: at opsz 32
// its ascender, and so the fallback, is the location's.
#[test]
fn the_vertical_fallback_follows_the_location() {
    let f = font(INTER);
    assert!(!f.has_table(Tag::new(b"vmtx")), "Inter gained vmtx; pick another fixture");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    let mvar = tables.get_mut(&Tag::new(b"MVAR")).expect("Inter has MVAR");
    let [size, count] = [6, 8].map(|at| usize::from(u16::from_be_bytes([mvar[at], mvar[at + 1]])));
    let record = (0..count).map(|i| 12 + i * size).find(|&at| &mvar[at..at + 4] == b"xhgt").expect("Inter varies xhgt");
    mvar[record..record + 4].copy_from_slice(b"hasc");
    mvar[12..record + size].rotate_right(size);
    let g = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("loads");
    for (at, want) in [(&[("opsz", 32.0)][..], 2416.0), (&[], 2478.0)] {
        let run = g.shape("H", &located(&g, at), &ShapeOptions::default().with_direction(Direction::TopToBottom));
        assert_eq!(run.advances, [want], "at {at:?}");
        assert_eq!(g.advance(run.glyphs[0], &located(&g, at), Orientation::Vertical), want, "at {at:?}");
    }
}

#[test]
fn a_shaped_run_places_its_marks() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");

    for (name, text, above) in [
        ("fatha", "\u{0628}\u{064E}", true),
        ("kasra", "\u{0628}\u{0650}", false),
    ] {
        let run = f.shape(text, &Location::default(), &ShapeOptions::default());
        assert_eq!(run.offsets.len(), run.glyphs.len(), "{name}: offsets are not parallel");
        assert_eq!(run.advances.len(), run.glyphs.len());
        assert_eq!(run.clusters.len(), run.glyphs.len());
        assert_eq!(run.glyphs.len(), 2, "{name}: expected a base and a mark");

        let mark = run
            .advances
            .iter()
            .position(|&a| a == 0.0)
            .unwrap_or_else(|| panic!("{name}: no zero-advance mark, so nothing needs placing"));
        let daegun::shape::Offset { x: dx, y: dy } = run.offsets[mark];
        assert!(
            dx != 0.0 || dy != 0.0,
            "{name}: the mark has neither advance nor offset, so it cannot be placed at all",
        );

        let Bounds { y_min: y0, y_max: y1, .. } = f
            .glyph_bounds(run.glyphs[mark], &Location::default())
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
        let run = latin.shape(text, &Location::default(), &ShapeOptions::default());
        assert_eq!(run.offsets.len(), run.glyphs.len());
        assert!(
            run.offsets.iter().all(|o| o.x == 0.0 && o.y == 0.0),
            "{text:?} produced offsets with nothing to place: {:?}",
            run.offsets,
        );
        assert!(run.advances.iter().all(|&a| a > 0.0));
    }

    let vertical =
        latin.shape("AV", &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom));
    assert_eq!(vertical.offsets.len(), vertical.glyphs.len());
    assert!(vertical.advances.iter().all(|&a| a > 0.0), "vertical advances are positive magnitudes");
}

#[test]
fn a_run_says_where_it_may_be_cut() {
    let f = font(GARAMOND);

    let (mut flagged, mut checked) = (0usize, 0usize);
    for text in ["HIn", "nun", "HH", "mm", "AVA", "ox"] {
        let whole = f.shape(text, &Location::default(), &ShapeOptions::default());
        assert_eq!(whole.unsafe_to_break.len(), whole.glyphs.len(), "{text:?}: not parallel");
        assert_eq!(
            whole.glyphs.len(),
            text.chars().count(),
            "{text:?} does not shape one glyph per character, so the cut indices do not line up",
        );
        assert!(!whole.unsafe_to_break[0], "{text:?}: the run's own start is flagged");

        for cut in 1..text.chars().count() {
            let (head, tail) = text.split_at(cut);
            let a = f.shape(head, &Location::default(), &ShapeOptions::default());
            let b = f.shape(tail, &Location::default(), &ShapeOptions::default());
            let spliced: Vec<GlyphId> = a.glyphs.iter().chain(b.glyphs.iter()).copied().collect();
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

    let ava = f.shape("AVA", &Location::default(), &ShapeOptions::default());
    assert!(ava.unsafe_to_break[1], "the A/V pair is no longer flagged; pick another fixture");
    let lost: f64 = f.shape("A", &Location::default(), &ShapeOptions::default()).advances.iter().sum::<f64>()
        + f.shape("VA", &Location::default(), &ShapeOptions::default()).advances.iter().sum::<f64>()
        - ava.advances.iter().sum::<f64>();
    assert!(lost > 1.0, "splitting a flagged cut changed nothing, so the flag means nothing");
}

#[test]
fn advances_are_the_glyph_metric_and_not_the_shaped_width() {
    let f = font(GARAMOND);
    assert_eq!(f.units_per_em(), 1000, "EBGaramond's em changed; the exactness below depends on it");
    let h = f.glyph_id('H').expect("H");
    let single = f.shape("H", &Location::default(), &ShapeOptions::default());
    assert_eq!(single.glyphs.len(), 1);
    assert!(
        (crate::advances(&f, &[h], &Location::default())[0] - single.advances[0]).abs() < 0.01,
        "one unkerned glyph should measure the same both ways",
    );

    let text = "The quick brown fox jumps over the lazy dog";
    let gids: Vec<GlyphId> = text.chars().filter_map(|c| f.glyph_id(c)).collect();
    let summed: f64 = crate::advances(&f, &gids, &Location::default()).iter().sum();
    let run = f.shape(text, &Location::default(), &ShapeOptions::default());
    let shaped: f64 = run.advances.iter().sum();
    assert_eq!(run.width(), shaped, "a run's width is not the sum of its advances");
    assert!(
        (summed - shaped).abs() > 10.0,
        "the two agree over a whole line, so this fixture kerns nothing and proves nothing",
    );

    let v = font(INTER);
    assert_ne!(v.units_per_em(), 1000, "Inter's em changed; this checks an em other than 1,000");
    let o = v.glyph_id('o').expect("Inter has o");
    let metric = crate::advances(&v, &[o], &Location::default())[0];
    let shaped_one = v.shape("o", &Location::default(), &ShapeOptions::default()).advances[0];
    assert_eq!(metric, shaped_one, "a glyph alone shaped to another advance than its metric");
}

// An MVAR delta can be a long word, and one near i32::MAX must saturate the metric as the instancer
// saturates it, not overflow before the clamp.
#[test]
fn typographic_metrics_saturate_an_mvar_delta_as_the_instance_does() {
    let f = font(INTER);
    let axes = f.axes();
    let wght = axes.iter().position(|a| a.tag == Tag::new(b"wght")).expect("Inter varies weight");
    // One value record, xhgt, whose store gives a single long-word delta on a region peaking at the
    // heaviest weight.
    let mut mvar = Vec::new();
    let u16s = |out: &mut Vec<u8>, vs: &[u16]| vs.iter().for_each(|v| out.extend(v.to_be_bytes()));
    u16s(&mut mvar, &[1, 0, 0, 8, 1, 20]);
    mvar.extend(b"xhgt\0\0\0\0");
    let regions = 4 + 6 * axes.len() as u32;
    u16s(&mut mvar, &[1]);
    mvar.extend(12u32.to_be_bytes());
    u16s(&mut mvar, &[1]);
    mvar.extend((12 + regions).to_be_bytes());
    u16s(&mut mvar, &[axes.len() as u16, 1]);
    for a in 0..axes.len() {
        let peak = if a == wght { 0x4000 } else { 0 };
        u16s(&mut mvar, &[0, peak, peak]);
    }
    u16s(&mut mvar, &[1, 0x8001, 1, 0]);
    mvar.extend(i32::MAX.to_be_bytes());

    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"MVAR"), mvar);
    let g = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("loads");
    let heavy = located(&g, &[("wght", axes[wght].max)]);
    let direct = g.typographic_metrics(&heavy).x_height;
    let instance = Font::from_bytes(&g.instance(&heavy).expect("instanced")).expect("the instance loads");
    let through_instance = instance.typographic_metrics(&Location::default()).x_height;
    assert_eq!(direct, through_instance, "typographic_metrics and the instance disagree on a saturated delta");
    assert_eq!(direct, Some(32767.0), "the delta did not saturate");
}

#[test]
fn typographic_metrics_survives_a_missing_os2() {
    let f = font(GARAMOND);
    let intact = f.typographic_metrics(&Location::default());
    let underline = intact.underline.expect("EBGaramond's post states an underline");
    assert!(underline.position < 0.0, "underline sits below the baseline");
    assert!(underline.thickness > 0.0, "an underline of no thickness draws nothing");
    let strikeout = intact.strikeout.map(|s| s.thickness);
    assert!(intact.x_height > Some(0.0) && strikeout > Some(0.0), "the OS/2 block is present here");

    let mut tables = f.instance_tables(&Location::default()).expect("instanced");
    assert!(tables.contains_key(&Tag::new(b"OS/2")) && tables.contains_key(&Tag::new(b"post")), "fixture lost a table");

    tables.remove(&Tag::new(b"OS/2"));
    let g = Font::from_bytes(&daegun::raw::build_font(&tables).unwrap()).expect("parses without OS/2");
    let m = g.typographic_metrics(&Location::default());
    assert_eq!(
        m.underline, intact.underline,
        "dropping OS/2 changed the underline, which post states and OS/2 does not"
    );
    assert_eq!((m.x_height, m.strikeout), (None, None), "an absent OS/2 field must read none rather than carry over");
    assert_eq!((m.subscript, m.superscript), (None, None), "absent sub/superscript boxes are none");

    let mut short = f.instance_tables(&Location::default()).expect("instanced");
    let os2 = short[&Tag::new(b"OS/2")][..28].to_vec();
    short.insert(Tag::new(b"OS/2"), std::borrow::Cow::Owned(os2));
    let h = Font::from_bytes(&daegun::raw::build_font(&short).unwrap()).expect("parses with a short OS/2");
    let m = h.typographic_metrics(&Location::default());
    assert_eq!(m.underline, intact.underline, "short OS/2 lost the underline");
    assert_eq!(m.strikeout, None, "a truncated OS/2 block reported a strikeout anyway");

    let mut bare = f.instance_tables(&Location::default()).expect("instanced");
    bare.remove(&Tag::new(b"OS/2"));
    bare.remove(&Tag::new(b"post"));
    if let Ok(b) = Font::from_bytes(&daegun::raw::build_font(&bare).unwrap()) {
        let m = b.typographic_metrics(&Location::default());
        assert_eq!(
            (m.x_height, m.underline, m.strikeout),
            (None, None, None),
            "a font stating none of these reported some"
        );
    }
}

#[test]
fn the_types_a_caller_debugs_with_can_be_debugged() {
    let f = font(GARAMOND);

    let axes = f.axes();
    assert!(!axes.is_empty(), "EBGaramond is variable; the rest of this needs an axis");
    assert_eq!(axes, axes.clone(), "Axis compares unequal to a clone of itself");
    assert!(
        format!("{axes:?}").contains(&axes[0].tag.to_string()),
        "an axis debug-prints without naming its tag, which is the only part a reader needs",
    );

    let opts = daegun::outline::OutlineOptions::default().with_oblique(0.25);
    let shown = format!("{opts:?}");
    assert!(shown.contains("oblique") && shown.contains("hinting"), "OutlineOptions hid its fields");

    let gray = format!("{:?}", daegun::raster::SubpixelLayout::grayscale());
    let rgb = format!("{:?}", daegun::raster::SubpixelLayout::horizontal(daegun::raster::StripeOrder::Rgb));
    assert!(gray.len() < 200, "the layout dumped its weight table: {gray}");
    assert_ne!(gray, rgb, "two different layouts debug-print identically");

    let gid = f.glyph_id('A').expect("A");
    let hinted = f.hinted(gid, &Location::default(), 16.0, daegun::outline::HintMode::Auto).expect("A hints");
    assert_eq!(hinted, hinted.clone(), "HintedOutline compares unequal to its own clone");
    assert!(!format!("{hinted:?}").is_empty());
}

#[test]
fn shape_bidi_tells_each_run_what_surrounds_it() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");

    let with = f.shape("سل", &Location::default(), &daegun::shape::ShapeOptions::default().with_context("", "ام"));
    let without = f.shape("سل", &Location::default(), &ShapeOptions::default());
    assert_ne!(
        with.glyphs, without.glyphs,
        "a following letter did not change the joining form, so this font cannot show the bug",
    );

    let text = "سلام\u{200E}سلام";
    let runs = f.shape_bidi(text, &Location::default(), BaseDirection::Auto, &ShapeOptions::default());
    assert_eq!(runs.len(), 3, "LRM no longer splits the Arabic; the case below tests nothing");

    for (i, r) in runs.iter().enumerate() {
        let alone = f.shape(
            &text[core::ops::Range::from(r.bytes)],
            &Location::default(),
            &daegun::shape::ShapeOptions::default().with_context(&text[..r.bytes.start], &text[r.bytes.end..]),
        );
        assert_eq!(
            r.run.glyphs, alone.glyphs,
            "run {i} of {text:?} was shaped without knowing its neighbors",
        );
    }
}

#[test]
fn shape_options_composes_features_with_fragment_context() {
    let g = font(GARAMOND);

    let on = g.shape("fi", &Location::default(), &daegun::shape::ShapeOptions::default());
    let off = g.shape(
        "fi",
        &Location::default(),
        &daegun::shape::ShapeOptions::default().with_features(&[Feature::new(Tag::new(b"liga"), 0)]),
    );
    assert_ne!(on.glyphs, off.glyphs, "turning liga off changed nothing, so nothing is proven below");
    assert_eq!(on.glyphs.len(), 1, "EBGaramond should form an fi ligature");

    let m = font(STIX);
    let plain = m.shape("x", &Location::default(), &daegun::shape::ShapeOptions::default());
    let ssty = m.shape(
        "x",
        &Location::default(),
        &daegun::shape::ShapeOptions::default()
            .with_features(&[Feature::new(Tag::new(b"ssty"), 1)])
            .with_script(Tag::new(b"math")),
    );
    assert_ne!(ssty.glyphs, plain.glyphs, "ssty under the math script selected nothing");

    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let joined = f.shape("سل", &Location::default(), &daegun::shape::ShapeOptions::default().with_context("", "ام"));
    let alone = f.shape("سل", &Location::default(), &ShapeOptions::default());
    assert_ne!(joined.glyphs, alone.glyphs, "context changes nothing here; the case below is vacuous");

    let both = f.shape(
        "سل",
        &Location::default(),
        &daegun::shape::ShapeOptions::default()
            .with_features(&[Feature::new(Tag::new(b"liga"), 0)])
            .with_context("", "ام"),
    );
    assert_eq!(
        both.glyphs, joined.glyphs,
        "asking for a feature dropped the fragment context that was asked for alongside it",
    );
}

#[test]
fn layout_shapes_each_run_at_its_own_bidi_level() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let opts = daegun::layout::LayoutOptions::default().with_max_inline_size(f64::INFINITY);

    let text = "سلام (a) سلام";
    let lay = f.layout(text, &Location::default(), &opts);
    let run = lay.lines[0].runs.iter()
        .find(|r| r.bytes == (11..21).into())
        .expect("the `) سلام` piece after the Latin word, its parenthesis in the script it opened in");
    assert_eq!(run.level % 2, 1, "that piece is at an odd level, or the case below is vacuous");
    assert!(
        run.run.glyphs.contains(&GlyphId(9)),
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
        let bidi: Vec<GlyphId> = f.shape_bidi(text, &Location::default(), BaseDirection::Auto, &ShapeOptions::default())
            .iter().flat_map(|r| r.run.glyphs.clone()).collect();
        let laid: Vec<GlyphId> = f.layout(text, &Location::default(), &opts)
            .lines[0].runs.iter().flat_map(|r| r.run.glyphs.clone()).collect();
        assert_eq!(laid, bidi, "layout and shape_bidi disagree on {text:?}");
    }
}

#[test]
fn typographic_metrics_vary_by_axis_the_way_instancing_does() {
    let inter = font(INTER);
    let axis = |tag: &[u8; 4]| inter.axes().into_iter().find(|a| a.tag == Tag::new(tag))
        .unwrap_or_else(|| panic!("Inter varies {tag:?}"));
    let at = |tag: Tag, v: f64| inter.typographic_metrics(&inter.location(&[(tag, v)]));
    let thickness = |m: daegun::metrics::TypographicMetrics| m.underline.expect("Inter states an underline").thickness;

    let (wght, opsz) = (axis(b"wght"), axis(b"opsz"));
    let (thin, black) = (thickness(at(wght.tag, wght.min)), thickness(at(wght.tag, wght.max)));
    assert!(black > thin * 3.0, "the underline barely moved with the weight: {thin} against {black}");

    assert!(
        at(opsz.tag, opsz.max).x_height < at(opsz.tag, opsz.min).x_height,
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
            let coords: Vec<(Tag, f64)> = axes.iter().map(|a| (a.tag, a.min + (a.max - a.min) * t)).collect();
            let at = f.location(&coords);
            let instanced = Font::from_bytes(&f.instance(&at).expect("instanced")).expect("the instance reopens");
            assert_eq!(
                f.typographic_metrics(&at),
                instanced.typographic_metrics(&Location::default()),
                "{rel} at t={t}: asking at the axes disagrees with instancing to them",
            );
        }
    }
}

type Retag = [(&'static [u8; 4], &'static [u8; 4])];

// Inter with its MVAR records retagged (and its major version set, where given).
fn inter_with_mvar(retag: &Retag, version: Option<u16>) -> Font {
    let f = font(INTER);
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    let mvar = tables.get_mut(&Tag::new(b"MVAR")).expect("Inter has MVAR");
    let [size, count] = [6, 8].map(|at| usize::from(u16::from_be_bytes([mvar[at], mvar[at + 1]])));
    for (old, new) in retag {
        let record = (0..count).map(|i| 12 + i * size).find(|&at| &mvar[at..at + 4] == *old).expect("Inter varies it");
        mvar[record..record + 4].copy_from_slice(*new);
    }
    if let Some(major) = version {
        mvar[..2].copy_from_slice(&major.to_be_bytes());
    }
    Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("loads")
}

// A tag's MVAR record is found by a search stopping at the first matching midpoint, and another major version varies
// nothing, here in an instance at opsz 32.
#[test]
fn mvar_records_are_found_by_tag() {
    let cases: [(&Retag, Option<u16>, [i16; 4]); 5] = [
        (&[], None, [1984, 1984, 1056, 634]),
        (&[(b"xhgt", b"hasc")], None, [1984, 1984, 1118, 634]),
        (&[(b"stro", b"hasc")], None, [1947, 1947, 1056, 671]),
        (&[(b"stro", b"hasc"), (b"xhgt", b"hasc")], None, [1947, 1947, 1118, 671]),
        (&[], Some(2), [1984, 1984, 1118, 671]),
    ];
    for (retag, version, want) in cases {
        let g = inter_with_mvar(retag, version);
        let instance = Font::from_bytes(&g.instance(&located(&g, &[("opsz", 32.0)])).expect("instanced"))
            .expect("the instance loads");
        let (hhea, os2) =
            (instance.table(Tag::new(b"hhea")).expect("hhea"), instance.table(Tag::new(b"OS/2")).expect("OS/2"));
        let at = |t: &[u8], o: usize| i16::from_be_bytes([t[o], t[o + 1]]);
        let got = [at(hhea, 4), at(os2, 68), at(os2, 86), at(os2, 28)];
        assert_eq!(got, want, "{retag:?}, version {version:?}");
    }
}

// Inter with its stro record retagged hasc, still in order, so its ascender varies with opsz. Every metric, and
// layout's lines, read at a location as its instance has them there.
#[test]
fn line_metrics_follow_the_location_as_its_instance_does() {
    let g = inter_with_mvar(&[(b"stro", b"hasc")], None);
    let at = located(&g, &[("opsz", 32.0)]);
    let instance = Font::from_bytes(&g.instance(&at).expect("instanced")).expect("the instance loads");

    assert_ne!(g.ascender(&at), g.ascender(&Location::default()), "the retagged record moved nothing");
    assert_eq!(g.ascender(&at), instance.ascender(&Location::default()));
    assert_eq!(g.descender(&at), instance.descender(&Location::default()));
    assert_eq!(g.cap_height(&at), instance.cap_height(&Location::default()));
    for o in [Orientation::Horizontal, Orientation::Vertical] {
        assert_eq!(g.line_metrics(&at, o), instance.line_metrics(&Location::default(), o), "{o:?}");
    }
    assert_eq!(g.os2(&at), instance.os2(&Location::default()));
    assert_eq!(g.typographic_metrics(&at), instance.typographic_metrics(&Location::default()));

    let opts = daegun::layout::LayoutOptions::default();
    let line = &g.layout("Hamburg", &at, &opts).lines[0];
    let ascent = instance.line_metrics(&Location::default(), Orientation::Horizontal).ascent;
    assert_eq!((line.ascent, line.baseline), (ascent, ascent), "layout's line is not the location's");
}

// Inter with opsz retagged slnt in fvar and STAT, so a location slants it: what the instance's style pass rewrites
// (post's angle, OS/2's selection, head's style, the names and STAT) reads at a location as that instance has it.
#[test]
fn accessors_the_style_pass_rewrites_read_at_the_location() {
    let f = font(INTER);
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    for tag in [b"fvar", b"STAT"] {
        let t = tables.get_mut(&Tag::new(tag)).expect("Inter has fvar and STAT");
        let at = t.windows(4).position(|w| w == b"opsz").expect("Inter has an opsz axis");
        t[at..at + 4].copy_from_slice(b"slnt");
    }
    let g = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("loads");
    let read = |f: &Font, at: &[(&str, f64)]| {
        let names = f.names(&located(f, at));
        let each: Vec<Option<String>> = names.keys().map(|&id| f.name(id, &located(f, at))).collect();
        (
            f.italic_angle(&located(f, at)),
            f.pdf_flags(&located(f, at)),
            f.style(&located(f, at)),
            f.family_name(&located(f, at)),
            names,
            each,
            f.stat(&located(f, at)),
        )
    };
    for at in [&[("wght", 900.0)][..], &[("slnt", 20.0)], &[("wght", 300.0), ("slnt", 32.0)]] {
        let instance = Font::from_bytes(&g.instance(&located(&g, at)).expect("instanced")).expect("the instance loads");
        assert_eq!(read(&g, at), read(&instance, &[]), "at {at:?}");
    }
    let (heavy, slanted) = (read(&g, &[("wght", 900.0)]), read(&g, &[("slnt", 20.0)]));
    assert_eq!(
        (slanted.0, slanted.1.contains(daegun::metrics::PdfFlags::ITALIC)),
        (20.0, true),
        "slnt 20 is not slanted"
    );
    assert_eq!(heavy.4[&NameId::FAMILY_NAME], "Inter Variable Black", "wght 900 is not named Black");
    assert_ne!(heavy.6, g.stat(&Location::default()), "STAT is the default's at wght 900");

    // At the default every font reads its own tables, a CFF2 font too, though it draws from its instance there.
    let cff2 = font("source-han-sans/SourceHanSansJP-VF.otf");
    let stored = cff2.table(Tag::new(b"OS/2")).map(|t| u16::from_be_bytes([t[62], t[63]]));
    let instanced =
        Font::from_bytes(&cff2.instance(&Location::default()).expect("instanced")).expect("the instance loads");
    assert_eq!(
        cff2.os2(&Location::default()).and_then(|o| o.selection),
        stored,
        "the default read Source Han Sans's instance"
    );
    assert_ne!(
        instanced.os2(&Location::default()).and_then(|o| o.selection),
        stored,
        "the instance restyles nothing here"
    );
}

// Inter's OS/2 as version 3, with OBLIQUE set and its typographic ascender 200 units over hhea's. Before version 4 the
// bits are reserved and must be 0, so one set is read as version 4 defines it.
#[test]
fn os2_selection_bits_count_before_version_4() {
    let f = font(INTER);
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    let os2 = tables.get_mut(&Tag::new(b"OS/2")).expect("Inter has OS/2");
    os2[0..2].copy_from_slice(&3u16.to_be_bytes());
    let selection = u16::from_be_bytes([os2[62], os2[63]]) | 0x0200;
    os2[62..64].copy_from_slice(&selection.to_be_bytes());
    let typo = i16::from_be_bytes([os2[68], os2[69]]) + 200;
    os2[68..70].copy_from_slice(&typo.to_be_bytes());
    let g = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("loads");
    let info = g.os2(&Location::default()).expect("an OS/2");
    assert_eq!((info.version, info.uses_typo_metrics(), info.is_oblique()), (3, true, true));
    let ascent = g.line_metrics(&Location::default(), Orientation::Horizontal).ascent;
    assert_eq!(ascent, f64::from(typo), "line metrics ignore USE_TYPO_METRICS");
}

#[test]
fn subset_glyph_ids_are_readable_whatever_the_outline_format() {
    let mut saw_renumbered = false;
    let mut saw_kept = false;

    for rel in [
        GARAMOND, INTER,
        STIX,
        "source-serif/SourceSerif4Variable-Roman.otf",
    ] {
        let f = font(rel);
        let gids: Vec<GlyphId> = "Hamburg".chars().filter_map(|c| f.glyph_id(c)).collect();
        assert!(gids.len() >= 5, "{rel} did not map the test text");

        let sub = f.subset(&gids, &Location::default()).expect("subsets");
        let out = Font::from_bytes(&sub.font).expect("the subset reopens");
        if gids.iter().all(|&g| sub.renumbered(g) == Some(g)) { saw_kept = true } else { saw_renumbered = true }

        for &old in &gids {
            let new = sub.renumbered(old).unwrap_or_else(|| panic!("{rel}: requested gid {old} was dropped"));
            assert_eq!(
                f.glyph_bounds(old, &Location::default()),
                out.glyph_bounds(new, &Location::default()),
                "{rel}: gid {old} -> {new} is not the same glyph after subsetting",
            );
        }

        assert_eq!(sub.renumbered(GlyphId(0)), Some(GlyphId(0)), "{rel}: .notdef did not survive");
    }

    assert!(saw_renumbered && saw_kept, "the fixtures no longer cover both subsetting strategies");

    let f = font(GARAMOND);
    let high = f.glyph_id('z').filter(|&g| g > GlyphId(8)).expect("a glyph well past .notdef");
    let sub = f.subset([high], &Location::default()).expect("subsets");
    assert_ne!(sub.renumbered(high), Some(high), "the glyf subset did not compact its glyph ids");
}

// CFF2 with no fvar is a valid static font. Made by dropping a variable font's fvar, its charstrings
// still blend, so it has to draw and subset exactly as that font's default.
#[test]
fn a_static_cff2_font_is_its_default_instance() {
    let variable = font(SOURCE_SERIF);
    let tables: std::collections::BTreeMap<Tag, std::borrow::Cow<[u8]>> = variable
        .table_tags()
        .filter(|&tag| tag != Tag::new(b"fvar"))
        .map(|tag| (tag, std::borrow::Cow::Borrowed(variable.table(tag).expect("listed"))))
        .collect();
    let f = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the static font parses");
    assert!(f.has_table(Tag::new(b"CFF2")) && !f.is_variable(), "the fixture did not become a static CFF2 font");

    let gids: Vec<GlyphId> = "Hamburg".chars().filter_map(|c| f.glyph_id(c)).collect();
    assert!(gids.len() >= 5, "the static font did not map the test text");
    for &g in &gids {
        let (mut got, mut want) = (daegun::outline::Path::default(), daegun::outline::Path::default());
        f.outline(g, &Location::default(), &mut got).expect("a static CFF2 glyph draws");
        variable.outline(g, &Location::default(), &mut want).expect("the variable glyph draws");
        assert_eq!(got, want, "gid {g} is not the variable font's default");
    }

    let sub = f.subset(&gids, &Location::default()).expect("a static CFF2 font subsets");
    let out = Font::from_bytes(&sub.font).expect("the subset reopens");
    for &old in &gids {
        let new = sub.renumbered(old).unwrap_or_else(|| panic!("requested gid {old} was dropped"));
        assert_eq!(
            f.glyph_bounds(old, &Location::default()),
            out.glyph_bounds(new, &Location::default()),
            "gid {old} changed in the subset"
        );
    }
    assert_eq!(
        f.glyph_closure(&gids, &Location::default()).expect("a static CFF2 closure"),
        variable.glyph_closure(&gids, &Location::default()).expect("a variable CFF2 closure"),
    );
    f.subset_text("Hamburg", &Location::default()).expect("a static CFF2 font subsets by text");

    // Subsetting pins the CFF2 inside; a static font asked for its instance is itself.
    let instance = f.instance_tables(&Location::default()).expect("instanced");
    assert!(
        instance.contains_key(&Tag::new(b"CFF2")) && !instance.contains_key(&Tag::new(b"CFF ")),
        "the static font came back converted"
    );
    assert_eq!(
        *f.instance(&Location::default()).expect("instanced"),
        *daegun::raw::build_font(&tables).unwrap(),
        "instance() is not the font as stored"
    );
}

// A .notdef built from another glyph keeps that glyph in a subset, as the closure says it must.
#[test]
fn a_composite_notdef_keeps_its_components() {
    let base = font("test-fixtures/hinted.ttf");
    let (a, f) = (base.glyph_id('A').expect("A"), base.glyph_id('F').expect("F"));
    let mut notdef: Vec<u8> = Vec::new();
    for v in [-1i16, 0, 0, 300, 700, 0x0003, f.0 as i16, 0, 0] {
        notdef.extend(v.to_be_bytes());
    }
    let font = crate::prepared::hinted_with(0, &notdef);
    assert!(closure_of(&font, &[a]).contains(&f), "the closure lost .notdef's component");
    let sub = font.subset([a], &Location::default()).expect("subsets");
    assert!(sub.renumbered(f).is_some(), "the subset dropped .notdef's component");
}

// An fvar whose header claims axes that do not parse (an axis record 10 bytes long, not 20) leaves the
// font static to the instancer, and so it comes back as stored too, CFF2 and fvar both.
#[test]
fn a_cff2_font_whose_axes_do_not_parse_is_its_own_instance() {
    let variable = font(SOURCE_SERIF);
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = variable
        .table_tags()
        .map(|tag| (tag, variable.table(tag).expect("listed").to_vec()))
        .collect();
    daegun::raw::bytes::write_u16_be(tables.get_mut(&Tag::new(b"fvar")).expect("fvar"), 10, 10);
    let f = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    assert!(f.is_variable() && f.axes().is_empty(), "the fixture is not a variable font with unreadable axes");
    let instance = f.instance_tables(&located(&f, &[("wght", 900.0)])).expect("instanced");
    assert!(
        instance.contains_key(&Tag::new(b"CFF2")) && instance.contains_key(&Tag::new(b"fvar")),
        "the font came back converted"
    );
    assert_eq!(
        *f.instance(&located(&f, &[("wght", 900.0)])).expect("instanced"),
        *daegun::raw::build_font(&tables).unwrap(),
        "instance() is not the font as stored"
    );

    // And to draw, hint and subset it is its default instance, as a font with no fvar is.
    let gids: Vec<GlyphId> = "Hamburg".chars().filter_map(|c| f.glyph_id(c)).collect();
    for &g in &gids {
        let (mut got, mut want) = (daegun::outline::Path::default(), daegun::outline::Path::default());
        f.outline(g, &Location::default(), &mut got).expect("the glyph draws");
        variable.outline(g, &Location::default(), &mut want).expect("the variable glyph draws");
        assert_eq!(got, want, "gid {g} is not the variable font's default");
    }
    assert!(
        f.hinted(gids[0], &Location::default(), 16.4, daegun::outline::HintMode::Auto).is_some(),
        "the glyph did not hint"
    );
    f.subset(&gids, &Location::default()).expect("the font subsets");
}

// The default instance, written out as static CFF2 and opened as a font of its own, is a second route to the same
// outline.
#[test]
fn outline_draws_a_cff2_font_as_its_default() {
    for rel in [SOURCE_SERIF, "source-han-sans/SourceHanSansJP-VF.otf"] {
        let f = font(rel);
        let defaults: Vec<(Tag, f64)> = f.axes().iter().map(|a| (a.tag, a.default)).collect();
        let cff =
            Font::from_bytes(&f.instance(&f.location(&defaults)).expect("instanced")).expect("the instance reopens");
        assert!(
            cff.has_table(Tag::new(b"CFF2")) && !cff.has_table(Tag::new(b"CFF ")),
            "{rel}: the instance is not CFF2"
        );
        let mut drawn = 0;
        for gid in 1..40 {
            let (mut got, mut want) = (daegun::outline::Path::default(), daegun::outline::Path::default());
            assert_eq!(
                f.outline(GlyphId(gid), &Location::default(), &mut got),
                cff.outline(GlyphId(gid), &Location::default(), &mut want),
                "{rel} gid {gid}"
            );
            assert_eq!(got, want, "{rel} gid {gid}: outline is not the default instance's");
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
    let gids: Vec<u32> = (1024..1040).chain([2233]).chain((0..f.glyph_count()).step_by(97)).collect();
    let lost: Vec<u32> = gids
        .into_iter()
        .filter(|&g| f.outline(GlyphId(g), &Location::default(), &mut daegun::outline::Path::default()).is_err())
        .collect();
    assert!(lost.is_empty(), "{} glyphs did not draw: {lost:?}", lost.len());
}

// A glyph past the count answers as the tables do: no hmtx advance, the line's height down and the ascender up without
// vmtx (Bungee Tint's gives 0), and no outline, box, name or color.
#[test]
fn a_glyph_past_the_count_answers_as_the_tables_do_everywhere() {
    let fonts = [(GARAMOND, 1305.0, 1007.0), (STIX, 1000.0, 762.0), ("bungee-tint/BungeeTint-Regular.ttf", 0.0, 0.0)];
    for (rel, down, origin) in fonts {
        let f = font(rel);
        let n = f.glyph_count();
        assert!(n > 0, "{rel} has no glyphs");

        for gid in [n, n.saturating_add(1), u32::from(u16::MAX)] {
            let gid = GlyphId(gid);
            assert_eq!(crate::advances(&f, &[gid], &Location::default()), vec![0.0], "{rel}: advances({gid})");
            let vertical = (
                f.advance(gid, &Location::default(), Orientation::Vertical),
                f.vertical_origin(gid, &Location::default()),
            );
            assert_eq!(vertical, (down, origin), "{rel}: the vertical advance and origin of {gid}");
            assert!(f.glyph_bounds(GlyphId(gid.0), &Location::default()).is_none(), "{rel}: glyph_bounds({gid})");
            assert!(f.glyph_name(GlyphId(gid.0)).is_none(), "{rel}: glyph_name({gid})");
            assert_eq!(
                f.quads(gid, &Location::default()).map_err(|e| e.kind()),
                Err(daegun::ErrorKind::NoOutline),
                "{rel}: quads({gid})"
            );
            let mut sink = daegun::outline::Path::default();
            let opts = daegun::outline::OutlineOptions::default();
            assert!(
                f.prepared_outline(GlyphId(gid.0), &Location::default(), 16.0, &opts, &mut sink).is_err(),
                "{rel}: prepared_outline({gid})"
            );
            assert!(sink.is_empty(), "{rel}: prepared_outline({gid}) drew before declining");
            let scene = f.color_scene(gid, &Location::default(), &daegun::color::SceneOptions::default());
            assert!(matches!(scene, Ok(None)), "{rel}: color_scene({gid})");
            assert!(
                f.hinted(GlyphId(gid.0), &Location::default(), 16.0, daegun::outline::HintMode::Auto).is_none(),
                "{rel}: hinted"
            );
            assert!(
                f.ligature_carets(GlyphId(gid.0), &Location::default(), Orientation::Horizontal).is_empty(),
                "{rel}: ligature_carets({gid})"
            );
            assert!(f.colr_layers(gid, 0).is_none(), "{rel}: colr_layers({gid})");
            assert!(f.paint_graph(gid, &Location::default(), 0).is_none(), "{rel}: paint_graph({gid})");
            assert!(
                f.math_glyph_variants(GlyphId(gid.0), &Location::default(), Orientation::Vertical).is_none(),
                "{rel}: math_glyph_variants({gid})"
            );
        }
        assert!(
            f.advance(GlyphId(n - 1), &Location::default(), Orientation::Vertical) > 0.0,
            "{rel}: the last glyph {} has no vertical advance",
            n - 1
        );
    }
}

// Cutting maxp's count changes no glyph id: EB Garamond counting 40 glyphs maps and shapes as it does whole, and only
// a vertical origin past the count moves, to the ascender, as such a glyph has no box.
#[test]
fn a_cut_count_changes_no_glyph_id() {
    let f = font(GARAMOND);
    const KEPT: u32 = 40;
    let mut tables = f.instance_tables(&Location::default()).expect("instanced");
    let mut maxp = tables[&Tag::new(b"maxp")].to_vec();
    maxp[4..6].copy_from_slice(&(KEPT as u16).to_be_bytes());
    tables.insert(Tag::new(b"maxp"), std::borrow::Cow::Owned(maxp));
    let g = Font::from_bytes(&daegun::raw::build_font(&tables).unwrap()).expect("still parses");
    assert_eq!(g.glyph_count(), KEPT, "the maxp edit did not take");

    assert!(
        f.coverage().any(|(_, id)| id >= GlyphId(KEPT)),
        "no codepoint maps past {KEPT}; the case is vacuous"
    );
    assert_eq!(g.coverage().collect::<Vec<_>>(), f.coverage().collect::<Vec<_>>());
    for text in ["Hamburgefonstiv", "fi ffl office", "AV To 123", "\u{0}\u{FFFD}"] {
        let (whole, cut) = (
            f.shape(text, &Location::default(), &ShapeOptions::default()),
            g.shape(text, &Location::default(), &ShapeOptions::default()),
        );
        assert_eq!(cut.glyphs, whole.glyphs, "{text:?}");
        assert_eq!((&cut.advances, &cut.offsets), (&whole.advances, &whole.offsets), "{text:?}");
        let (whole, cut) = (
            f.shape(text, &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom)),
            g.shape(text, &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom)),
        );
        assert_eq!(cut.glyphs, whole.glyphs, "{text:?} down");
        for (i, &id) in cut.glyphs.iter().enumerate() {
            let x = whole.offsets[i].x;
            let expected = if id < GlyphId(KEPT) { whole.offsets[i] } else { (x, -1007.0).into() };
            assert_eq!(cut.offsets[i], expected, "{text:?} down, glyph {id} at {i}");
        }
    }
}

#[test]
fn instancing_does_not_depend_on_the_order_the_axes_were_listed_in() {
    let f = font("source-serif/SourceSerif4Variable-Roman.otf");
    let axes = f.axes();
    assert!(axes.len() >= 2, "the fixture needs two axes to reorder");
    assert!(f.has_table(Tag::new(b"CFF2")), "the fixture is no longer CFF2; this tests the name it generates");

    let mut coords: Vec<(Tag, f64)> = axes.iter().map(|a| (a.tag, a.default)).collect();
    let forward = f.location(&coords);
    coords.reverse();
    let reversed = f.location(&coords);

    assert_eq!(
        f.instance(&forward).expect("instanced"),
        f.instance(&reversed).expect("instanced"),
        "instance depends on the axis order"
    );
    assert_eq!(
        f.instance_tables(&forward).expect("instanced"),
        f.instance_tables(&reversed).expect("instanced"),
        "instance_tables depends on the axis order",
    );

    for at in [&forward, &reversed] {
        let tables = f.instance_tables(at).expect("instanced");
        assert_eq!(
            *daegun::raw::build_font(&tables).unwrap(),
            *f.instance(at).expect("instanced"),
            "build_font(instance_tables).unwrap() is not the font instance() produces",
        );
    }

    let g = font(INTER);
    let ga = g.axes();
    if ga.len() >= 2 {
        let mut coords: Vec<(Tag, f64)> = ga.iter().map(|a| (a.tag, a.default)).collect();
        let fwd = g.location(&coords);
        coords.reverse();
        let rev = g.location(&coords);
        assert_eq!(
            g.instance_tables(&fwd).expect("instanced"),
            g.instance_tables(&rev).expect("instanced"),
            "glyf instancing reordered"
        );
    }
}

#[test]
fn instance_tables_answers_exactly_as_instance_does() {
    let f = font(INTER);
    assert!(!f.axes().is_empty(), "the fixture is not variable; nothing below is meaningful");

    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = f.table_tags()
        .filter_map(|t| f.table(t).map(|d| (t, d.to_vec())))
        .collect();
    let fvar = tables.get_mut(&Tag::new(b"fvar")).expect("Inter carries fvar");
    fvar[8..10].copy_from_slice(&0u16.to_be_bytes());
    let broken = Font::from_bytes(&daegun::raw::build_font(&tables).unwrap()).expect("still parses");
    assert!(broken.has_table(Tag::new(b"fvar")) && broken.axes().is_empty(), "the fvar edit did not take");
    // An fvar of no axes still parses, so the instancer instances it, as 1.1.7 did: its variation tables go.
    let instanced = broken.instance_tables(&Location::default()).expect("instanced");
    assert!(
        !instanced.contains_key(&Tag::new(b"fvar")) && !instanced.contains_key(&Tag::new(b"gvar")),
        "an fvar of no axes came back as stored"
    );

    for probe in [&broken, &f] {
        let tables = probe.instance_tables(&Location::default()).expect("instanced");
        assert_eq!(
            *daegun::raw::build_font(&tables).unwrap(),
            *probe.instance(&Location::default()).expect("instanced"),
            "instance_tables and instance disagree about the same font at the same axes",
        );
    }

    let s = font(STIX);
    assert!(!s.has_table(Tag::new(b"fvar")), "the static fixture gained an fvar");
    let st = s.instance_tables(&Location::default()).expect("instanced");
    assert!(st.contains_key(&Tag::new(b"head")) && st.contains_key(&Tag::new(b"cmap")), "the static path lost tables");
    assert_eq!(
        *daegun::raw::build_font(&st).unwrap(),
        *s.instance(&Location::default()).expect("instanced"),
        "static: the two routes disagree"
    );
}

#[test]
fn a_font_can_be_asked_what_features_it_declares() {
    let ar = font("scheherazade-new/ScheherazadeNew-Regular.ttf");

    let scripts = ar.script_tags();
    assert!(scripts.contains(&Tag::new(b"arab")), "an Arabic face declares arab: {scripts:?}");
    let langs = ar.language_system_tags(Tag::new(b"arab"));
    assert!(langs.contains(&Tag::new(b"SND ")), "Scheherazade declares Sindhi: {langs:?}");

    let feats = ar.feature_tags(Some(Tag::new(b"arab")), None);
    for required in [b"init", b"medi", b"fina", b"rlig"].map(Tag::new) {
        assert!(feats.contains(&required), "no {required} for arab: {feats:?}");
    }
    assert!(feats.windows(2).all(|w| w[0] < w[1]), "not sorted and deduplicated: {feats:?}");

    let latin = font(GARAMOND);
    let offered = latin.feature_tags(None, None);
    let plain = latin.shape("fi", &Location::default(), &ShapeOptions::default()).glyphs.clone();
    assert!(offered.contains(&Tag::new(b"kern")), "EBGaramond declares kern: {offered:?}");
    assert!(!offered.contains(&Tag::new(b"zzzz")), "no font declares zzzz");

    let off = latin
        .shape(
            "fi",
            &Location::default(),
            &ShapeOptions::default().with_features(&[Feature::new(Tag::new(b"liga"), 0)]),
        )
        .glyphs
        .clone();
    assert_ne!(off, plain, "liga is declared and turning it off changed nothing");
    let bogus = latin
        .shape(
            "fi",
            &Location::default(),
            &ShapeOptions::default().with_features(&[Feature::new(Tag::new(b"zzzz"), 1)]),
        )
        .glyphs
        .clone();
    assert_eq!(bogus, plain, "an undeclared tag changed the shaping");

    let dv = font("noto-devanagari/NotoSansDevanagari.ttf");
    let dv_scripts = dv.script_tags();
    assert!(
        dv_scripts.contains(&Tag::new(b"deva")) && dv_scripts.contains(&Tag::new(b"dev2")),
        "both Devanagari models should be listed: {dv_scripts:?}",
    );

    assert!(ar.feature_tags(Some(Tag::new(b"zzzz")), None).is_empty(), "an undeclared script offered features");
    assert!(ar.language_system_tags(Tag::new(b"ab  ")).is_empty(), "a script the font lacks offered languages");
    assert_eq!(
        ar.feature_tags(Some(Tag::new(b"arab")), Some(Tag::new(b"nope"))),
        ar.feature_tags(Some(Tag::new(b"arab")), None),
        "an undeclared language should report the default LangSys, as shaping would use it",
    );

    let emoji = font("noto-color-emoji/NotoColorEmoji.ttf");
    let _ = emoji.script_tags();
    let _ = emoji.feature_tags(None, None);
}

// The font at `rel` with each tag `from` in its `table` spelled `to`, for tags no fixture spells.
fn renamed(rel: &str, table: &[u8; 4], renames: &[(&[u8; 4], &[u8; 4])]) -> Font {
    let f = font(rel);
    let mut map: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    let table = Tag::new(table);
    let data = map.get_mut(&table).expect("the table");
    for (from, to) in renames {
        let at: Vec<usize> = data.windows(4).enumerate().filter(|(_, w)| w == from).map(|(i, _)| i).collect();
        assert!(!at.is_empty(), "no {} in {table}", String::from_utf8_lossy(&from[..]));
        at.into_iter().for_each(|i| data[i..i + 4].copy_from_slice(&to[..]));
    }
    Font::from_vec(daegun::raw::build_font(&map).unwrap(), 0).expect("loads")
}

// A short tag is padded with spaces as fonts store it, and one over four bytes refused. German's language system
// adds locl to Latin's default, so "DEU" can only have found it.
#[test]
fn a_short_tag_is_padded_and_a_long_one_refused() {
    let tag = |s: &str| s.parse::<Tag>();
    assert_eq!([tag("DEU"), tag("lao"), tag("nko")], [b"DEU ", b"lao ", b"nko "].map(|t| Ok(Tag::new(t))));
    assert!(tag("DEUTSCH").is_err() && tag("latin").is_err() && tag("lao  x").is_err());

    let latin = font(GARAMOND);
    let german = latin.feature_tags(Some(Tag::new(b"latn")), Some(Tag::new(b"DEU ")));
    assert!(german.contains(&Tag::new(b"locl")), "DEU offered {german:?}");

    // With DFLT gone, only the script's own record can answer.
    let serif = renamed(SOURCE_SERIF, b"BASE", &[(b"latn", b"lao "), (b"DFLT", b"zzzz")]);
    let lao = serif.base_script(Tag::new(b"lao "), &Location::default(), Orientation::Horizontal);
    assert!(lao.is_some(), "BASE's lao baselines went unfound");

    let nko = renamed("scheherazade-new/ScheherazadeNew-Regular.ttf", b"JSTF", &[(b"arab", b"nko ")]);
    let extenders = nko.justification_glyphs(Tag::new(b"nko "));
    assert!(extenders.is_some_and(|g| !g.is_empty()), "JSTF's nko extenders went unfound");
}

// A feature as tools write one: each documented form, and what isn't one refused.
#[test]
fn a_feature_parses_as_tools_write_it() {
    let f = |tag: &[u8; 4], value: u32, start: usize, end: usize| {
        Ok(Feature { bytes: (start..end).into(), ..Feature::new(Tag::new(tag), value) })
    };
    let all = usize::MAX;
    let forms = [
        ("kern", f(b"kern", 1, 0, all)), ("+kern", f(b"kern", 1, 0, all)), ("-kern", f(b"kern", 0, 0, all)),
        ("kern=0", f(b"kern", 0, 0, all)), ("aalt=2", f(b"aalt", 2, 0, all)), ("aalt 2", f(b"aalt", 2, 0, all)),
        ("kern[3:5]", f(b"kern", 1, 3, 5)), ("kern[3:5]=0", f(b"kern", 0, 3, 5)), ("kern[3]", f(b"kern", 1, 3, 4)),
        ("kern[:5]", f(b"kern", 1, 0, 5)), ("kern[3:]", f(b"kern", 1, 3, all)), ("kern[]", f(b"kern", 1, 0, all)),
        ("kern[3;5]", f(b"kern", 1, 3, 5)), ("'liga' on", f(b"liga", 1, 0, all)),
        ("\"smcp\"=off", f(b"smcp", 0, 0, all)),
        ("  ss01  ", f(b"ss01", 1, 0, all)), ("-kern=2", f(b"kern", 2, 0, all)), ("'lao'", f(b"lao ", 1, 0, all)),
    ];
    for (text, want) in forms {
        assert_eq!(text.parse::<Feature>(), want, "{text:?}");
    }
    for text in ["", "-", "kern=", "kern[3", "kern[a:b]", "toolong", "kern=x", "kern 1 2", "'kern", "ke-rn", "kern]"] {
        let got = text.parse::<Feature>();
        assert!(got.is_err_and(|e| e.kind() == daegun::ErrorKind::InvalidArgument), "{text:?} parsed");
    }
}

// The number a tag's bytes spell big-endian, as C's daegun_tag holds it, and back.
#[test]
fn a_tag_is_the_number_its_bytes_spell() {
    assert_eq!(u32::from(Tag::new(b"wght")), 0x7767_6874);
    assert_eq!(Tag::from(0x7767_6874), Tag::new(b"wght"));
}

#[test]
fn the_other_two_glyph_flags_can_be_asked_for() {
    let ar = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let text = "سلام دنيا";

    let plain = ar.shape(text, &Location::default(), &ShapeOptions::default());
    assert!(plain.unsafe_to_concat.is_empty(), "reported without being asked");
    assert!(plain.safe_to_insert_tatweel.is_empty(), "reported without being asked");
    assert!(!plain.unsafe_to_break.is_empty(), "unsafe_to_break is always reported");

    let both = ar.shape(
        text,
        &Location::default(),
        &daegun::shape::ShapeOptions::default().with_report_unsafe_to_concat(true).with_report_tatweel_positions(true),
    );

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

    let only_concat = ar.shape(
        text,
        &Location::default(),
        &daegun::shape::ShapeOptions::default().with_report_unsafe_to_concat(true),
    );
    assert_eq!(only_concat.unsafe_to_concat.len(), n, "concat alone did not report");
    assert!(only_concat.safe_to_insert_tatweel.is_empty(), "tatweel reported unasked");

    let latin = font(GARAMOND);
    let l = latin.shape(
        "office",
        &Location::default(),
        &daegun::shape::ShapeOptions::default().with_report_tatweel_positions(true),
    );
    assert_eq!(l.safe_to_insert_tatweel.len(), l.glyphs.len(), "not reported for Latin");
    assert!(!l.safe_to_insert_tatweel.iter().any(|&b| b), "Latin offered a kashida position");
}

#[test]
fn a_glyph_can_be_asked_what_gdef_calls_it() {
    let f = font(GARAMOND);
    assert!(f.has_table(Tag::new(b"GDEF")), "the fixture lost its GDEF");

    let mut seen = [0usize; 4];
    let mut marks = Vec::new();
    for gid in 0..f.glyph_count() {
        match f.glyph_class(GlyphId(gid)) {
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
            crate::advances(&f, &[GlyphId(m)], &Location::default())[0], 0.0,
            "gid {m} is classified a mark but advances",
        );
    }

    let none = font("noto-color-emoji/NotoColorEmoji.ttf");
    if !none.has_table(Tag::new(b"GDEF")) {
        assert_eq!(none.glyph_class(GlyphId(1)), None, "a font with no GDEF classified a glyph");
        assert_eq!(none.mark_attachment_class(GlyphId(1)), None, "a font with no GDEF has no attach class");
    }

    let past = f.glyph_count();
    assert_eq!(f.glyph_class(GlyphId(past)), None, "{past} lies past GDEF's classes");
    assert_eq!(f.glyph_class(GlyphId(u16::MAX.into())), None, "u16::MAX lies past GDEF's classes");
    assert_eq!(f.mark_attachment_class(GlyphId(past)), None, "{past} lies past GDEF's attach classes");

    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = f.table_tags()
        .filter_map(|t| f.table(t).map(|d| (t, d.to_vec())))
        .collect();
    const KEPT: u32 = 40;
    tables.get_mut(&Tag::new(b"maxp")).expect("maxp")[4..6].copy_from_slice(&(KEPT as u16).to_be_bytes());
    let shrunk = Font::from_bytes(&daegun::raw::build_font(&tables).unwrap()).expect("parses");

    let outside: Vec<u32> = (KEPT..f.glyph_count())
        .filter(|&g| f.glyph_class(GlyphId(g)).is_some())
        .take(5)
        .collect();
    assert!(!outside.is_empty(), "no classified glyph past {KEPT}; the case below is vacuous");
    // GDEF classes a glyph past the count as it does any other, since its class table is keyed by glyph id alone.
    for g in outside {
        assert_eq!(shrunk.glyph_class(GlyphId(g)), f.glyph_class(GlyphId(g)), "gid {g} past maxp");
        assert_eq!(shrunk.mark_attachment_class(GlyphId(g)), f.mark_attachment_class(GlyphId(g)), "gid {g} past maxp");
        assert!(shrunk.glyph_bounds(GlyphId(g), &Location::default()).is_none(), "gid {g} past maxp has a box");
    }

    let run = f.shape("e\u{0301}", &Location::default(), &ShapeOptions::default());
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

    let by = |ig| f.shape(text, &Location::default(), &daegun::shape::ShapeOptions::default().with_ignorables(ig));
    let hide = by(daegun::shape::Ignorables::Hide);
    let remove = by(daegun::shape::Ignorables::Remove);
    let preserve = by(daegun::shape::Ignorables::Preserve);

    assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs, hide.glyphs, "the default moved");

    assert_eq!(remove.glyphs.len(), hide.glyphs.len() - 1, "Remove did not drop the ZWJ");
    assert_eq!(preserve.glyphs.len(), hide.glyphs.len(), "Preserve changed the run length");
    for r in [&hide, &remove, &preserve] {
        assert_eq!(r.advances.len(), r.glyphs.len(), "arrays fell out of step");
        assert_eq!(r.clusters.len(), r.glyphs.len(), "clusters fell out of step");
    }

    assert_ne!(hide.glyphs, preserve.glyphs, "Hide and Preserve produced the same glyphs");

    let named =
        f.shape(text, &Location::default(), &daegun::shape::ShapeOptions::default().with_invisible_glyph(GlyphId(5)));
    assert!(named.glyphs.contains(&GlyphId(5)), "invisible_glyph was ignored: {:?}", named.glyphs);

    let joined = f.shape("لا", &Location::default(), &ShapeOptions::default());
    let broken_kept = f.shape("ل\u{200C}ا", &Location::default(), &ShapeOptions::default());
    let broken_dropped = f.shape(
        "ل\u{200C}ا",
        &Location::default(),
        &daegun::shape::ShapeOptions::default().with_ignorables(daegun::shape::Ignorables::Remove),
    );

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

// An ignorable's advances and its offset along the run are zeroed but not its offset across, so down the line it
// keeps the x its vertical origin gave it: each sits at x -672 of Inter's 2,048.
#[test]
fn a_vertical_ignorable_keeps_its_offset_across_the_run() {
    let f = font("inter/InterVariable.ttf");
    for text in ["a\u{200D}b", "a\u{FE0F}b", "a\u{2060}b"] {
        let run = f.shape(text, &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom));
        assert_eq!((run.offsets[1].into(), run.advances[1]), ((-672.0, 0.0), 0.0), "{text:?}");
    }
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
            let run = f.shape(text, &Location::default(), &ShapeOptions::default());
            assert!(run.complete, "{rel} did not finish shaping {text:?}");
        }
    }
}

#[test]
fn the_dotted_circle_can_be_turned_off() {
    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let circle = f.glyph_id('\u{25CC}').expect("the fixture has U+25CC");

    let orphan = "\u{093F}";
    let on = f.shape(orphan, &Location::default(), &ShapeOptions::default());
    let off = f.shape(
        orphan,
        &Location::default(),
        &daegun::shape::ShapeOptions::default().with_suppress_dotted_circle(true),
    );

    assert!(on.glyphs.contains(&circle), "no dotted circle for an orphaned matra: {:?}", on.glyphs);
    assert!(!off.glyphs.contains(&circle), "suppressing it left one in: {:?}", off.glyphs);
    assert_eq!(off.glyphs.len(), on.glyphs.len() - 1, "suppressing removed more than the circle");

    let dflt = f.shape(orphan, &Location::default(), &daegun::shape::ShapeOptions::default());
    assert_eq!(dflt.glyphs, on.glyphs, "the default stopped inserting the dotted circle");

    let based = "क\u{093F}";
    assert_eq!(
        f.shape(based, &Location::default(), &ShapeOptions::default()).glyphs,
        f.shape(based, &Location::default(), &daegun::shape::ShapeOptions::default().with_suppress_dotted_circle(true))
            .glyphs,
        "suppressing changed a run that had no dotted circle in it",
    );
}

#[test]
fn an_itemized_script_reaches_the_fonts_feature_list() {
    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let runs = daegun::unicode::script_runs("हिन्दी");
    assert_eq!(runs.len(), 1, "the sample should itemize as one script");
    let script = runs[0].script;
    assert_eq!(script.name(), "Devanagari");

    let declared = f.script_tags();
    let chosen = script.opentype_tags().find(|t| declared.contains(t)).expect("the font declares one of them");
    assert_eq!(chosen, Tag::new(b"dev2"), "Noto Devanagari ships the second-generation model");

    let feats = f.feature_tags(Some(chosen), None);
    assert!(feats.len() > 10, "dev2 should select a full Indic feature set, saw {feats:?}");
    for required in [b"abvs", b"blws", b"akhn", b"nukt"].map(Tag::new) {
        assert!(feats.contains(&required), "no {required} under {chosen}: {feats:?}");
    }

    assert!(
        f.feature_tags(Some(Tag::new(b"dev3")), None).is_empty(),
        "dev3 is not declared by this font and should offer nothing",
    );
}

#[test]
fn a_cluster_level_is_a_trade_and_the_output_shows_which() {
    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let text = "क्षि";
    let at = |lvl| f.shape(text, &Location::default(), &daegun::shape::ShapeOptions::default().with_cluster_level(lvl));

    let mono_g = at(daegun::shape::ClusterLevel::MonotoneGraphemes);
    let mono_c = at(daegun::shape::ClusterLevel::MonotoneCharacters);
    let chars = at(daegun::shape::ClusterLevel::Characters);
    let graph = at(daegun::shape::ClusterLevel::Graphemes);

    for other in [&mono_c, &chars, &graph] {
        assert_eq!(other.glyphs, mono_g.glyphs, "a cluster level changed the glyphs");
        assert_eq!(other.advances, mono_g.advances, "a cluster level changed the advances");
    }

    let ascends = |r: &daegun::shape::ShapedRun| r.clusters.windows(2).all(|w| w[0] <= w[1]);
    assert!(ascends(&mono_g), "MonotoneGraphemes ran backwards: {:?}", mono_g.clusters);
    assert!(ascends(&mono_c), "MonotoneCharacters ran backwards: {:?}", mono_c.clusters);
    assert!(
        !ascends(&chars) || !ascends(&graph),
        "neither non-monotone level reordered, so the fixture no longer shows the trade: \
         Characters {:?}, Graphemes {:?}", chars.clusters, graph.clusters,
    );

    assert!(daegun::shape::ClusterLevel::MonotoneGraphemes.is_monotone());
    assert!(daegun::shape::ClusterLevel::MonotoneCharacters.is_monotone());
    assert!(!daegun::shape::ClusterLevel::Characters.is_monotone());
    assert!(!daegun::shape::ClusterLevel::Graphemes.is_monotone());
    assert!(daegun::shape::ClusterLevel::MonotoneGraphemes.is_graphemes());
    assert!(!daegun::shape::ClusterLevel::MonotoneCharacters.is_graphemes());

    assert_eq!(daegun::shape::ClusterLevel::default(), daegun::shape::ClusterLevel::MonotoneGraphemes);
    assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).clusters, mono_g.clusters);

    let ar = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let rtl = ar.shape("سلام (a) سلام", &Location::default(), &ShapeOptions::default());
    let asc = rtl.clusters.windows(2).all(|w| w[0] <= w[1]);
    let desc = rtl.clusters.windows(2).all(|w| w[0] >= w[1]);
    assert!(desc && !asc, "a right-to-left run should descend: {:?}", rtl.clusters);
    assert!(asc || desc, "monotone means one direction throughout, either one");
}

#[test]
fn a_run_says_whether_the_text_was_well_formed_for_its_script() {
    let dv = font("noto-devanagari/NotoSansDevanagari.ttf");
    let circle = dv.glyph_id('\u{25CC}').expect("the fixture has U+25CC");

    for good in ["हिन्दी", "क", "कि"] {
        let r = dv.shape(good, &Location::default(), &ShapeOptions::default());
        assert!(!r.has_broken_syllable, "{good:?} is well-formed and was reported broken");
        assert!(!r.glyphs.contains(&circle), "{good:?} got a dotted circle");
    }
    for bad in ["\u{093F}", "\u{094D}", "\u{093F}क"] {
        let r = dv.shape(bad, &Location::default(), &ShapeOptions::default());
        assert!(r.has_broken_syllable, "{bad:?} is a broken syllable and was not reported");
        assert!(r.glyphs.contains(&circle), "{bad:?} was called broken and drew no circle");
    }

    let bad = "\u{093F}";
    let quiet =
        dv.shape(bad, &Location::default(), &daegun::shape::ShapeOptions::default().with_suppress_dotted_circle(true));
    assert!(!quiet.glyphs.contains(&circle), "the circle survived suppression");
    assert!(quiet.has_broken_syllable, "suppressing the circle also suppressed the finding");

    for (rel, good, bad) in [
        ("noto-khmer/NotoSansKhmer.ttf", "ខ្មែរ", "\u{17D2}"),
        ("noto-myanmar/NotoSansMyanmar.ttf", "မြန်မာ", "\u{1039}"),
    ] {
        let f = font(rel);
        assert!(!f.shape(good, &Location::default(), &ShapeOptions::default()).has_broken_syllable, "{rel}: {good:?}");
        assert!(f.shape(bad, &Location::default(), &ShapeOptions::default()).has_broken_syllable, "{rel}: {bad:?}");
    }

    for (rel, texts) in [
        (GARAMOND, ["Hamburg", "\u{0301}", "e\u{0301}"]),
        ("scheherazade-new/ScheherazadeNew-Regular.ttf", ["سلام", "\u{064E}", "لا"]),
    ] {
        let f = font(rel);
        for t in texts {
            assert!(
                !f.shape(t, &Location::default(), &ShapeOptions::default()).has_broken_syllable,
                "{rel} has no syllable grammar but reported {t:?} broken",
            );
        }
    }
}

// Inter without GPOS or GSUB, and Scheherazade, each given an empty morx, which shapes as any morx does: a zeroed
// mark keeps its offset and "dumber" replaces the script's shaper, GPOS still running.
#[test]
fn an_empty_morx_shapes_as_any_morx_does() {
    let with_morx = |rel: &str, drop: &[Tag]| {
        let f = font(rel);
        let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = f.table_tags()
            .filter(|t| !drop.contains(t))
            .map(|t| (t, f.table(t).expect("listed").to_vec()))
            .collect();
        tables.insert(Tag::new(b"morx"), vec![0, 2, 0, 0, 0, 0, 0, 0]);
        Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("loads")
    };
    let units = |r: &daegun::shape::ShapedRun| -> Vec<(u32, f64, f64, f64)> {
        (0..r.glyphs.len()).map(|i| (r.glyphs[i].0, r.offsets[i].x, r.offsets[i].y, r.advances[i])).collect()
    };

    let inter = with_morx(INTER, &[Tag::new(b"GPOS"), Tag::new(b"GSUB")]);
    let lone =
        inter.shape("\u{301}", &Location::default(), &ShapeOptions::default().with_direction(Direction::TopToBottom));
    assert_eq!(units(&lone), [(1770, 0.0, -2650.0, 0.0)], "a lone mark, vertical");
    assert_eq!(units(&inter.shape("\u{301}", &Location::default(), &ShapeOptions::default())), [(1770, 0.0, 0.0, 0.0)]);

    let arabic = with_morx("scheherazade-new/ScheherazadeNew-Regular.ttf", &[]);
    let run = arabic.shape("\u{628}\u{64E}", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.shaper, Shaper::Dumber);
    assert_eq!(units(&run), [(1342, 783.0, -631.0, 0.0), (308, 0.0, 0.0, 1960.0)]);
}

#[test]
fn a_run_says_which_shaping_model_it_went_through() {
    let ar = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    assert_eq!(ar.shape("سلام", &Location::default(), &ShapeOptions::default()).shaper, Shaper::Arabic);
    assert_eq!(ar.shape("abc", &Location::default(), &ShapeOptions::default()).shaper, Shaper::Default);

    let dv = font("noto-devanagari/NotoSansDevanagari.ttf");
    assert_eq!(dv.shape("हिन्दी", &Location::default(), &ShapeOptions::default()).shaper, Shaper::Indic);
    assert_eq!(dv.shape("abc", &Location::default(), &ShapeOptions::default()).shaper, Shaper::Default);

    for (rel, text, expect) in [
        (GARAMOND, "Hamburg", Shaper::Default),
        (GARAMOND, "Ελληνικά", Shaper::Default),
        ("noto-khmer/NotoSansKhmer.ttf", "ខ្មែរ", Shaper::Khmer),
        ("noto-myanmar/NotoSansMyanmar.ttf", "မြန်မာ", Shaper::Myanmar),
        ("source-han-sans/SourceHanSansJP-VF.otf", "한국어", Shaper::Hangul),
        ("source-han-sans/SourceHanSansJP-VF.otf", "日本語", Shaper::Default),
    ] {
        let got = font(rel).shape(text, &Location::default(), &ShapeOptions::default()).shaper;
        assert_eq!(got, expect, "{rel} shaped {text:?} through {got:?}");
    }

    for (rel, text) in [
        (GARAMOND, "\u{0301}"),
        ("scheherazade-new/ScheherazadeNew-Regular.ttf", "\u{064E}"),
    ] {
        let r = font(rel).shape(text, &Location::default(), &ShapeOptions::default());
        assert!(!r.has_broken_syllable && r.shaper == Shaper::Default || r.shaper == Shaper::Arabic,
                "{rel}: {text:?} -> {:?}", r.shaper);
    }
    let broken = dv.shape("\u{093F}", &Location::default(), &ShapeOptions::default());
    assert_eq!(broken.shaper, Shaper::Indic, "a broken syllable must come from a syllabic model");
    assert!(broken.has_broken_syllable, "and it must report one");
}

// The names C lends a run's shaper by, each the string `shaper` was before it was a type.
#[test]
fn each_shaping_model_keeps_its_name() {
    let all = [
        Shaper::Default, Shaper::Dumber, Shaper::Arabic, Shaper::Hangul, Shaper::Hebrew, Shaper::Indic,
        Shaper::Khmer, Shaper::Myanmar, Shaper::MyanmarZawgyi, Shaper::Thai, Shaper::Universal,
    ];
    let names = [
        "default", "dumber", "arabic", "hangul", "hebrew", "indic", "khmer", "myanmar", "myanmar_zawgyi", "thai",
        "universal",
    ];
    assert_eq!(all.map(Shaper::name), names);
    assert_eq!(Shaper::default(), Shaper::Default);
}

#[test]
fn an_incomplete_run_reports_itself() {
    use daegun::raw::bytes::{read_u16_be, write_u16_be};

    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = f.table_tags()
        .filter_map(|t| f.table(t).map(|d| (t, d.to_vec())))
        .collect();

    let mut patched = 0usize;
    {
        let g = tables.get_mut(&Tag::new(b"GSUB")).expect("the fixture carries GSUB");
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

    let looping = Font::from_bytes(&daegun::raw::build_font(&tables).unwrap()).expect("still parses");

    let incomplete: Vec<&str> = ["हिन्दी", "क्षि", "नमस्ते"]
        .into_iter()
        .filter(|t| !looping.shape(t, &Location::default(), &ShapeOptions::default()).complete)
        .collect();
    assert!(
        !incomplete.is_empty(),
        "a self-referential lookup produced no incomplete run; the nesting guard may have moved",
    );

    for t in incomplete {
        let r = looping.shape(t, &Location::default(), &ShapeOptions::default());
        assert!(!r.complete, "{t:?}");
        assert_eq!(r.advances.len(), r.glyphs.len(), "{t:?}: arrays fell out of step");
        assert_eq!(r.clusters.len(), r.glyphs.len(), "{t:?}: clusters fell out of step");
        assert!(r.advances.iter().all(|a| a.is_finite()), "{t:?}: non-finite advance");
        assert!(r.glyphs.iter().all(|&g| g < GlyphId(looping.glyph_count())), "{t:?}: glyph past the end");
    }

    for t in ["हिन्दी", "क्षि", "नमस्ते"] {
        assert!(
            f.shape(t, &Location::default(), &ShapeOptions::default()).complete,
            "the sound font reported {t:?} partial"
        );
    }
}

#[test]
fn the_subpixel_layout_bound_is_published() {
    let ok = daegun::raster::SubpixelLayout::from_weights(
        (daegun::raster::MAX_OVERSAMPLE, 1), (1, 1), (0, 0), [&[1.0], &[1.0], &[1.0]],
    );
    assert!(ok.is_ok(), "the published maximum was itself refused");

    let over = daegun::raster::SubpixelLayout::from_weights(
        (daegun::raster::MAX_OVERSAMPLE + 1, 1), (1, 1), (0, 0), [&[1.0], &[1.0], &[1.0]],
    );
    let invalid = |e: daegun::Error| e.kind() == daegun::ErrorKind::InvalidArgument;
    assert!(over.is_err_and(invalid), "one sample past the published maximum was accepted");
    let over_y = daegun::raster::SubpixelLayout::from_weights(
        (1, daegun::raster::MAX_OVERSAMPLE + 1), (1, 1), (0, 0), [&[1.0], &[1.0], &[1.0]],
    );
    assert!(over_y.is_err_and(invalid), "one sample past the published maximum was accepted on y");
    for bad in [f32::NAN, f32::INFINITY] {
        let weight = daegun::raster::SubpixelLayout::from_weights((1, 1), (1, 1), (0, 0), [&[bad], &[1.0], &[1.0]]);
        assert!(weight.is_err_and(invalid), "a weight of {bad} was accepted");
    }

    let zero = daegun::raster::SubpixelLayout::from_weights((0, 1), (1, 1), (0, 0), [&[1.0], &[1.0], &[1.0]]);
    assert!(zero.is_err_and(invalid), "an oversample of zero was accepted");
    for taps in [(0, 0), (0, 5), (5, 0)] {
        let none: [&[f32]; 3] = [&[], &[], &[]];
        assert!(
            daegun::raster::SubpixelLayout::from_weights((1, 1), taps, (0, 0), none).is_err_and(invalid),
            "{taps:?} taps were accepted"
        );
    }

    for layout in [
        daegun::raster::SubpixelLayout::grayscale(),
        daegun::raster::SubpixelLayout::horizontal(daegun::raster::StripeOrder::Rgb),
        daegun::raster::SubpixelLayout::vertical(daegun::raster::StripeOrder::Bgr),
        daegun::raster::SubpixelLayout::unfiltered(daegun::raster::StripeOrder::Rgb, Orientation::Horizontal),
    ] {
        let (ox, oy) = layout.oversample();
        assert!(ox <= daegun::raster::MAX_OVERSAMPLE && oy <= daegun::raster::MAX_OVERSAMPLE,
                "a preset oversamples past the published maximum: {ox}x{oy}");
    }
}

#[test]
fn a_gradient_can_be_sampled_without_engine_internals() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let mut sampled = 0;

    for gid in 0..f.glyph_count() {
        let Some(graph) = f.paint_graph(GlyphId(gid), &Location::default(), 0) else { continue };
        let mut scene = daegun::color::DisplayList::default();
        let mut outline =
            |g, pen: &mut dyn daegun::outline::OutlinePen| f.outline(g, &Location::default(), pen).is_ok();
        daegun::color::lower(
            GlyphId(gid),
            &graph,
            daegun::color::IDENTITY,
            &|g| f.colr_clip_box(g, &Location::default()),
            &mut outline,
            daegun::color::Rgba::default(),
            &mut scene,
        );

        for op in scene.ops() {
            let daegun::color::Op::Fill { paint, transform, .. } = op else { continue };
            let &daegun::color::Paint::Gradient(id) = paint else { continue };
            let g = scene.gradient(id).expect("a fill names a gradient the scene holds");

            let ramp = daegun::color::gradient::Ramp::new(&g, transform);
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
    let taps = u8::try_from(daegun::raster::MAX_TAPS).expect("the tap bound fits a u8");
    let need = usize::from(taps) * usize::from(taps);
    let weights: Vec<f32> = core::iter::repeat_n(1.0 / need as f32, need).collect();

    let layout = daegun::raster::SubpixelLayout::from_weights(
        (daegun::raster::MAX_OVERSAMPLE, 1),
        (taps, taps),
        (-(i8::try_from(taps).expect("taps fit an i8") / 2), 0),
        [&weights, &weights, &weights],
    ).expect("the widest layout the table is sized for was refused");

    assert_eq!(layout.taps(), (taps, taps), "the layout did not keep its taps");
    for c in 0..3 {
        assert_eq!(layout.weights(c).map(<[f32]>::len), Some(need), "channel {c} lost weights");
        assert_eq!(
            layout.weights(c).and_then(<[f32]>::last),
            Some(&weights[need - 1]),
            "channel {c} lost its last weight"
        );
    }

    // As many weights as its taps want, so only the tap bound can refuse it.
    let wider: Vec<f32> = vec![1.0 / f32::from(taps + 1); usize::from(taps) + 1];
    assert!(
        daegun::raster::SubpixelLayout::from_weights(
            (daegun::raster::MAX_OVERSAMPLE, 1), (taps + 1, 1), (0, 0),
            [&wider, &wider, &wider],
        ).is_err_and(|e| e.kind() == daegun::ErrorKind::InvalidArgument),
        "a filter wider than the table was accepted",
    );
    let taller: Vec<f32> = vec![1.0 / f32::from(taps * (taps + 1)); usize::from(taps) * usize::from(taps + 1)];
    assert!(
        daegun::raster::SubpixelLayout::from_weights(
            (1, daegun::raster::MAX_OVERSAMPLE), (taps, taps + 1), (0, 0),
            [&taller, &taller, &taller],
        ).is_err_and(|e| e.kind() == daegun::ErrorKind::InvalidArgument),
        "a filter taller than the table was accepted",
    );
}

#[test]
fn no_two_subpixel_layouts_share_an_identity() {
    use daegun::raster::{StripeOrder::{Bgr, Rgb}, SubpixelLayout};

    let mut layouts = vec![
        ("grayscale", SubpixelLayout::grayscale()),
        ("horizontal rgb", SubpixelLayout::horizontal(Rgb)),
        ("horizontal bgr", SubpixelLayout::horizontal(Bgr)),
        ("vertical rgb", SubpixelLayout::vertical(Rgb)),
        ("vertical bgr", SubpixelLayout::vertical(Bgr)),
        ("unfiltered h rgb", SubpixelLayout::unfiltered(Rgb, Orientation::Horizontal)),
        ("unfiltered h bgr", SubpixelLayout::unfiltered(Bgr, Orientation::Horizontal)),
        ("unfiltered v rgb", SubpixelLayout::unfiltered(Rgb, Orientation::Vertical)),
        ("unfiltered v bgr", SubpixelLayout::unfiltered(Bgr, Orientation::Vertical)),
    ];

    let flat = [0.25f32; 4];
    let tilt = [0.25f32, 0.25, 0.25, 0.2501];
    let build = |name, os, taps, origin, first: &[f32]| {
        let l = SubpixelLayout::from_weights(os, taps, origin, [first, &flat, &flat])
            .unwrap_or_else(|e| panic!("{name} was refused: {e}"));
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
    use daegun::color::{Extend, Gradient, GradientKind, Rgba, Stop, Stops};
    use daegun::color::gradient::Ramp;

    let line = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 };
    let two = vec![
        Stop { offset: 0.0, color: Rgba::opaque(255, 0, 0) },
        Stop { offset: 1.0, color: Rgba::opaque(0, 0, 255) },
    ];
    let grad = |stops| Gradient::new(line, stops, Extend::Pad, daegun::color::IDENTITY);

    let empty = Ramp::new(&grad(&[]), &daegun::color::IDENTITY);
    let collapsed = Ramp::new(&grad(&two), &[0.0; 6]);
    let unbounded = Ramp::new(&grad(&two), &[1.0, 0.0, 0.0, 1.0, f64::INFINITY, 0.0]);
    let undefined = Ramp::new(&grad(&two), &[1.0, 0.0, 0.0, 1.0, 0.0, f64::NAN]);
    for x in [-50.0, 0.0, 1.0, 5.0, 9.5, 60.0] {
        assert_eq!(empty.at(x, 1.0), None, "an empty stop list painted at {x}");
        assert_eq!(collapsed.at(x, 1.0), None, "a singular transform painted at {x}");
        assert_eq!((unbounded.at(x, 1.0), undefined.at(x, 1.0)), (None, None), "a transform not finite painted");
    }

    assert!(matches!(Stops::resolve(Vec::new(), Extend::Pad), Stops::Nothing), "an empty list was not Nothing");
    assert!(
        matches!(Stops::resolve(two.clone(), Extend::Pad), Stops::Many(ref m) if m.len() == 2),
        "two stops did not survive as two",
    );
    assert!(
        matches!(Stops::resolve(vec![two[0]], Extend::Pad), Stops::Solid(c) if c == two[0].color),
        "one stop was not reported as a solid fill",
    );
}

// COLR's color line spans its own stops, so repeat and reflect cycle over them: test_glyphs.ttf's lines run red to blue
// over x 100 to 900, stops at 0.2–0.8, 0–1.5 and 0.5–1.5, the colors the spec's linear-light blend worked in f64.
#[test]
fn a_color_line_repeats_over_its_own_stops() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let points =
        [(9u32, 0.05, [137u8, 0, 224]), (9, 0.95, [224, 0, 137]), (10, 0.5, [213, 0, 156]), (11, 0.9, [203, 0, 170])];
    for (gid, t, want) in points {
        let scene = f.color_scene(GlyphId(gid), &Location::default(), &daegun::color::SceneOptions::default());
        let scene = scene.expect("the scene reads").expect("a color glyph");
        let ramp = scene.ops().iter().find_map(|op| match op {
            daegun::color::Op::Fill { paint: daegun::color::Paint::Gradient(id), transform, .. } => {
                Some(daegun::color::gradient::Ramp::new(&scene.gradient(*id)?, transform))
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
    use daegun::color::{gradient::Ramp, Extend, Gradient, GradientKind, Rgba, Stop, IDENTITY};
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
            let ramp = |kind, stops| Ramp::new(&Gradient::new(kind, stops, extend, IDENTITY), &IDENTITY);
            let (a, b) = (ramp(plain, &stops(0.0, 1.0)), ramp(stretched, &stops(0.5, 1.5)));
            for i in -20..20 {
                for j in -20..20 {
                    let (x, y) = (f64::from(i) * 7.3, f64::from(j) * 5.9);
                    let (p, q) = (a.at(x, y), b.at(x, y));
                    let close = match (p, q) {
                        (Some(p), Some(q)) => {
                            [p.r.abs_diff(q.r), p.g.abs_diff(q.g), p.b.abs_diff(q.b)].iter().all(|&d| d <= 1)
                        }
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
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = f.table_tags()
        .filter(|t| *t != Tag::new(b"fvar"))
        .map(|t| (t, f.table(t).expect("listed").to_vec()))
        .collect();
    tables.get_mut(&Tag::new(b"OS/2")).expect("OS/2")[62..64].copy_from_slice(&[0, 0]);
    let stat = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the static font parses");
    let gid = stat.glyph_id('H').expect("H");
    let subset = stat.subset([gid], &located(&stat, &[("wght", 600.0)])).expect("a subset");
    let os2 = Font::from_vec(subset.font, 0)
        .expect("the subset parses").table(Tag::new(b"OS/2")).expect("OS/2").to_vec();
    assert_eq!(os2[62..64], [0, 0], "naming an axis the font lacks rewrote fsSelection");
}

// A hard edge, every stop at one offset, past 0..1 on either side: pad still switches colors there.
#[test]
fn a_hard_edge_past_0_to_1_stays_where_its_stops_put_it() {
    use daegun::color::{gradient::Ramp, Extend, Gradient, GradientKind, Rgba, Stop, IDENTITY};
    let (red, blue) = (Rgba::opaque(255, 0, 0), Rgba::opaque(0, 0, 255));
    let line = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 100.0, y1: 0.0 };
    for (at, before, after) in [(1.5f32, 140.0, 160.0), (-0.5, -60.0, -40.0)] {
        let stops = vec![Stop { offset: at, color: red }, Stop { offset: at, color: blue }];
        let g = Gradient::new(line, &stops, Extend::Pad, IDENTITY);
        let ramp = Ramp::new(&g, &IDENTITY);
        assert_eq!(ramp.at(before - 0.5, 0.0), Some(red), "an edge at {at}: blue before it");
        assert_eq!(ramp.at(after - 0.5, 0.0), Some(blue), "an edge at {at}: red past it");
    }
}

// A ramp takes the scene's space to the device, as daegun_scene_ramp does, and applies the
// gradient's own transform first: a line from 0 to 10, doubled, is halfway along at 10, not past its end.
#[test]
fn a_ramp_applies_the_gradients_own_transform() {
    use daegun::color::{Extend, Gradient, GradientKind, Rgba, Stop, IDENTITY};
    use daegun::color::gradient::{Interpolation, Ramp};

    let stops = vec![
        Stop { offset: 0.0, color: Rgba::opaque(0, 0, 0) },
        Stop { offset: 1.0, color: Rgba::opaque(200, 200, 200) },
    ];
    let line = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 };
    let g = Gradient::new(line, &stops, Extend::Pad, [2.0, 0.0, 0.0, 2.0, 0.0, 0.0]);
    let at = Ramp::with_interpolation(&g, &IDENTITY, Interpolation::Srgb).at(9.5, 0.0).expect("a color");
    assert!((90..=110).contains(&at.r), "halfway along the doubled line sampled {}, not about 100", at.r);
}

#[test]
fn a_composed_transform_can_be_undone() {
    use daegun::color::{concat, invert, IDENTITY};

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
    use daegun::color::Rgba;
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
    use daegun::raster::{StripeOrder::{Bgr, Rgb}, SubpixelLayout};

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
        ("unfiltered h", SubpixelLayout::unfiltered(Rgb, Orientation::Horizontal)),
    ];
    for (name, l) in &presets {
        assert!(covers(l), "{name}: padding {:?} does not cover origin {:?} at oversample {:?}",
                l.pad(), l.origin(), l.oversample());
    }

    assert_eq!(SubpixelLayout::grayscale().pad(), (0, 0), "grayscale asked for padding");
    assert_eq!(SubpixelLayout::horizontal(Rgb).pad(), (1, 0), "horizontal padding is not (1, 0)");
    assert_eq!(SubpixelLayout::vertical(Rgb).pad(), (0, 1), "vertical padding is not (0, 1)");
    assert_eq!(SubpixelLayout::unfiltered(Rgb, Orientation::Horizontal).pad(), (0, 0), "unfiltered asked for padding");

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

    let plain = f.shape_bidi(text, &Location::default(), BaseDirection::Auto, &ShapeOptions::default());
    assert!(plain.len() >= 2, "the text did not split into directional runs");

    assert!(
        plain.iter().all(|r| r.run.unsafe_to_concat.is_empty()),
        "unsafe_to_concat was reported without being asked for",
    );
    let asked = f.shape_bidi(
        text,
        &Location::default(),
        BaseDirection::Auto,
        &daegun::shape::ShapeOptions::default().with_report_unsafe_to_concat(true),
    );
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
    let with_junk = f.shape_bidi(
        joined,
        &Location::default(),
        BaseDirection::Auto,
        &daegun::shape::ShapeOptions::default().with_context("ZZZZ", "ZZZZ"),
    );
    let default_run = f.shape_bidi(joined, &Location::default(), BaseDirection::Auto, &ShapeOptions::default());
    let g = |v: &Vec<daegun::shape::BidiRun>| v.iter().flat_map(|r| r.run.glyphs.clone()).collect::<Vec<_>>();
    assert_eq!(
        g(&with_junk), g(&default_run),
        "a caller's `before`/`after` reached the runs and changed the joining, which is what this \
         function computes per run precisely so a caller cannot get it wrong",
    );
}

#[test]
fn the_raw_tier_can_outline_a_glyph_from_the_bytes_it_is_given() {
    let f = font(GARAMOND);
    let glyf = f.table(Tag::new(b"glyf")).expect("glyf");
    let loca = f.table(Tag::new(b"loca")).expect("loca");
    let head = f.table(Tag::new(b"head")).expect("head");
    let fmt = i16::from_be_bytes([head[50], head[51]]);
    let offsets = daegun::raw::parse_loca(loca, fmt, f.glyph_count() as usize);
    assert_eq!(
        offsets.len(), f.glyph_count() as usize + 1,
        "loca did not parse to one offset per glyph plus the end sentinel",
    );
    assert!(offsets.windows(2).all(|w| w[0] <= w[1]), "loca offsets are not monotone");
    // maxp counts glyphs in a u16, so a larger count is not a font's and must not size the answer.
    assert_eq!(daegun::raw::parse_loca(loca, fmt, usize::MAX).len(), 65_536, "loca sized itself from usize::MAX");

    let mut compared = 0;
    for ch in ['B', 'o', 'x', 'A', 'g', 'W'] {
        let gid = f.glyph_id(ch).expect("glyph");
        let mut raw = daegun::outline::Path::default();
        if daegun::raw::outline_glyf_bytes(glyf, false, &offsets, gid.0, &mut raw).is_err() {
            continue;
        }
        let mut viaapi = daegun::outline::Path::default();
        f.outline(gid, &Location::default(), &mut viaapi).expect("the front door outlined it");
        assert_eq!(
            raw.parts(), viaapi.parts(),
            "{ch}: the raw path and Font::outline disagree about verbs or coordinates",
        );
        assert!(!raw.is_empty(), "{ch} outlined to nothing");
        compared += 1;
    }
    assert!(compared >= 4, "only {compared} glyphs were compared, so this proves little");

    let mut p = daegun::outline::Path::default();
    assert!(
        daegun::raw::outline_glyf_bytes(glyf, false, &offsets, f.glyph_count(), &mut p).is_err(),
        "a glyph id past num_glyphs was outlined instead of refused",
    );
}

#[test]
fn a_vertical_origin_is_asked_for_a_location() {
    let f = font("source-han-sans/SourceHanSansJP-VF.otf");
    let axes = f.axes();
    assert!(!axes.is_empty(), "the fixture is not variable");
    let (tag, lo, def) = (axes[0].tag, axes[0].min, axes[0].default);
    let gid = f.glyph_id('\u{6c38}').or_else(|| f.glyph_id('A')).expect("a glyph");

    assert_eq!(
        f.vertical_origin(gid, &Location::default()),
        f.vertical_origin(gid, &f.location(&[(tag, def)])),
        "the default location answered differently depending on whether it was named",
    );
    assert!(
        f.advance(gid, &f.location(&[(tag, lo)]), Orientation::Vertical) > 0.0,
        "the fixture reports no vertical advance",
    );
    assert_eq!(
        f.vertical_origin(GlyphId(f.glyph_count()), &Location::default()),
        880.0,
        "a glyph past the count takes VORG's default"
    );
}

// Vertical origins with vertical features off, one for each path of the rule: VORG; glyf with vmtx, a composite
// taking its USE_MY_METRICS component's; else the box centered, in glyf, CFF and CFF2. Shaping alike.
#[test]
fn vertical_origins_take_each_path_of_their_rule() {
    let cases = [
        ("source-han-sans/SourceHanSansJP-VF.otf", '\u{6C38}', 880.0),
        ("bungee-tint/BungeeTint-Regular.ttf", 'H', 860.0),
        ("bungee-tint/BungeeTint-Regular.ttf", '\u{E18C}', 742.0),
        ("inter/InterVariable.ttf", 'A', 1984.0),
        ("eb-garamond/EBGaramond.ttf", 'g', 715.0),
        ("stix-two-math/STIX2Math.otf", 'A', 831.0),
        ("source-serif/SourceSerif4Variable-Roman.otf", 'A', 1023.0),
    ];
    for (rel, c, want) in cases {
        let f = font(rel);
        let gid = f.glyph_id(c).expect("mapped");
        assert_eq!(f.vertical_origin(gid, &Location::default()), want, "{rel} {c:?}");
        let run = f.shape(
            &c.to_string(),
            &Location::default(),
            &ShapeOptions::default().with_features(&NO_VERTICAL_FEATURES).with_direction(Direction::TopToBottom),
        );
        assert_eq!((run.glyphs[0], run.offsets[0].y), (gid, -want), "{rel} {c:?} shaped");
    }
}

const NO_VERTICAL_FEATURES: [Feature; 8] = [
    Feature::new(Tag::new(b"vert"), 0),
    Feature::new(Tag::new(b"vrt2"), 0),
    Feature::new(Tag::new(b"vkna"), 0),
    Feature::new(Tag::new(b"vpal"), 0),
    Feature::new(Tag::new(b"valt"), 0),
    Feature::new(Tag::new(b"vhal"), 0),
    Feature::new(Tag::new(b"vkrn"), 0),
    Feature::new(Tag::new(b"kern"), 0),
];

// A glyph without vertical metrics centers on its box: a color glyph's image (CBDT, sbix) or paint (COLR's clip box,
// else what v1 paints, else v0's layers), else its outline's, flat ones too: cff-cid's B lies along y 20,000.
#[test]
fn color_and_flat_glyphs_center_on_their_boxes() {
    let cases = [
        ("noto-color-emoji/NotoColorEmoji.ttf", 883, Some('\u{1F600}'), 1895.0),
        ("color-fonts-samples/samples-sbix.ttf", 19, None, 948.0),
        ("colr-v1-test-glyphs/test_glyphs.ttf", 10, Some('\u{F0102}'), 1075.0),
        ("colr-v1-test-glyphs/test_glyphs.ttf", 86, Some('\u{F0302}'), 1162.0),
        ("colr-v1-test-glyphs/test_glyphs.ttf", 88, Some('\u{F0304}'), 787.0),
        ("colr-v1-test-glyphs/test_glyphs.ttf", 168, Some('\u{F0E00}'), 1073.0),
        ("test-fixtures/cff-cid.otf", 2, Some('B'), 20_500.0),
    ];
    for (rel, gid, c, want) in cases {
        let f = font(rel);
        assert_eq!(f.vertical_origin(GlyphId(gid), &Location::default()), want, "{rel} glyph {gid}");
        if let Some(c) = c {
            let run = f.shape(
                &c.to_string(),
                &Location::default(),
                &ShapeOptions::default().with_features(&NO_VERTICAL_FEATURES).with_direction(Direction::TopToBottom),
            );
            assert_eq!((run.glyphs[0], run.offsets[0].y), (GlyphId(gid), -want), "{rel} {c:?} shaped");
        }
    }
}

// Each glyph's origin at the run's own location, shaped and asked for: a CFF2 and a glyf glyph centered on
// their boxes, which wght moves.
#[test]
fn vertical_shaping_takes_each_glyph_s_origin_at_its_location() {
    let cases = [
        ("source-serif/SourceSerif4Variable-Roman.otf", '\u{2019}', [(-106.0, -1292.0), (-114.0, -1256.0)]),
        ("noto-devanagari/NotoSansDevanagari.ttf", '\u{0958}', [(-384.0, -940.0), (-434.0, -905.0)]),
    ];
    for (rel, c, want) in cases {
        let f = font(rel);
        for (axes, want) in [&[][..], &[("wght", 900.0)][..]].into_iter().zip(want) {
            let run = f.shape(
                &c.to_string(),
                &located(&f, axes),
                &ShapeOptions::default().with_features(&NO_VERTICAL_FEATURES).with_direction(Direction::TopToBottom),
            );
            assert_eq!(run.offsets[0], want.into(), "{rel} {c:?} at {axes:?}");
            assert_eq!(f.vertical_origin(run.glyphs[0], &located(&f, axes)), -want.1, "{rel} {c:?} at {axes:?}");
        }
    }
}

// A glyph whose record is too short for its header reads as empty: no outline, and its top side bearing
// alone for a vertical origin.
#[test]
fn a_truncated_glyph_keeps_its_top_bearing_as_its_vertical_origin() {
    let rel = "bungee-tint/BungeeTint-Regular.ttf";
    let gid = font(rel).glyph_id('H').expect("H");
    assert_eq!(font(rel).vertical_origin(gid, &Location::default()), 860.0, "yMax 720 and a top bearing of 140");
    let cut = crate::prepared::patched(rel, &[(gid.0, vec![0, 1, 0, 0])]);
    assert!(
        cut.outline(gid, &Location::default(), &mut daegun::outline::Path::default()).is_err(),
        "the cut glyph still decodes"
    );
    assert_eq!(cut.vertical_origin(gid, &Location::default()), 140.0);
}

// A font with tables set or added, as `build_font` writes them.
fn with_tables(f: &Font, set: Vec<(&str, Vec<u8>)>) -> Font {
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.extend(set.into_iter().map(|(tag, data)| (tag.parse().expect("a tag"), data)));
    Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses")
}

fn words(w: &[i16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// A composite topped at `y_max` of `kids`, each taking USE_MY_METRICS at no offset.
fn composite(y_max: i16, kids: &[u32]) -> Vec<u8> {
    let mut g = words(&[-1, 0, 0, 500, y_max]);
    for (i, &kid) in kids.iter().enumerate() {
        let more = if i + 1 < kids.len() { 0x20 } else { 0 };
        g.extend_from_slice(&(0x0202u16 | more).to_be_bytes());
        g.extend_from_slice(&(kid as u16).to_be_bytes());
        g.extend_from_slice(&[0, 0]);
    }
    g
}

// vhea and vmtx giving every glyph an advance of 1000 down and a top bearing of 50.
fn vertical_metrics(f: &Font) -> Vec<(&'static str, Vec<u8>)> {
    let n = f.glyph_count() as u16;
    let mut vhea = 0x0001_1000u32.to_be_bytes().to_vec();
    vhea.extend(words(&[500, -500, 0, 1000, 50, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]));
    vhea.extend_from_slice(&n.to_be_bytes());
    vec![("vhea", vhea), ("vmtx", (0..n).flat_map(|_| words(&[1000, 50])).collect())]
}

// A glyph's origin as vertical shaping offsets it, the same as asked for.
fn origin(f: &Font, c: char, axes: &[(&str, f64)]) -> f64 {
    let run = f.shape(
        &c.to_string(),
        &located(f, axes),
        &ShapeOptions::default().with_features(&NO_VERTICAL_FEATURES).with_direction(Direction::TopToBottom),
    );
    let shaped = -run.offsets[0].y;
    assert_eq!(f.vertical_origin(run.glyphs[0], &located(f, axes)), shaped, "{c:?} asked and shaped");
    shaped
}

// VORG reads at major version 1, or 2 with 24-bit counts and glyphs; records counted past the table's end are absent,
// leaving its default origin, 900. Another version leaves BungeeTint's H its top phantom point: yMax 720, bearing 140.
#[test]
fn an_unreadable_vorg_leaves_the_origin_to_vmtx() {
    let f = font("bungee-tint/BungeeTint-Regular.ttf");
    let h = f.glyph_id('H').expect("H");
    let h16 = h.0 as i16;
    for (vorg, want) in [
        ([1, 0, 900, 1, h16, 100], 100.0),
        ([1, 5, 900, 1, h16, 100], 100.0),
        ([0, 0, 900, 1, h16, 100], 860.0),
        ([2, 0, 900, 1, h16, 100], 900.0),
        ([1, 0, 900, 2, h16, 100], 900.0),
    ] {
        assert_eq!(origin(&with_tables(&f, vec![("VORG", words(&vorg))]), 'H', &[]), want, "{vorg:?}");
    }
}

// A component naming a composite the walk is inside closes a cycle and is passed over, so that composite keeps its
// own top: BungeeTint's H, I and J top at 700, 600 and 650, each bearing 140.
#[test]
fn a_composite_containing_itself_stops_a_glyph_or_two_in() {
    let rel = "bungee-tint/BungeeTint-Regular.ttf";
    let ids = |c: char| font(rel).glyph_id(c).expect("mapped").0;
    let (h, i, j) = (ids('H'), ids('I'), ids('J'));
    let cases = [
        (vec![(h, composite(700, &[h]))], [840.0, 860.0, 860.0]),
        (vec![(h, composite(700, &[i])), (i, composite(600, &[h]))], [740.0, 840.0, 860.0]),
        (vec![(j, composite(650, &[h])), (h, composite(700, &[i])), (i, composite(600, &[h]))], [740.0, 840.0, 740.0]),
        (vec![(h, composite(700, &[i])), (i, composite(600, &[j])), (j, composite(650, &[h]))], [790.0, 840.0, 740.0]),
    ];
    for (glyphs, want) in cases {
        let f = crate::prepared::patched(rel, &glyphs);
        let path: Vec<u32> = glyphs.iter().map(|g| g.0).collect();
        assert_eq!(['H', 'I', 'J'].map(|c| origin(&f, c, &[])), want, "{path:?}");
    }
}

// A simple glyph whose end points or instruction length don't read, or with fewer points than contours, fails the
// walk, which answers the em.
#[test]
fn a_simple_glyph_cut_short_answers_the_em_for_its_origin() {
    let rel = "bungee-tint/BungeeTint-Regular.ttf";
    let h = font(rel).glyph_id('H').expect("H");
    for glyph in [words(&[1, 0, 0, 500, 700, 3]), words(&[2, 0, 0, 500, 700, 0, 0, 0])] {
        assert_eq!(origin(&crate::prepared::patched(rel, &[(h.0, glyph.clone())]), 'H', &[]), 1000.0, "{glyph:?}");
    }
}

// Past 65,535 glyphs read or 65,535 points with the 4 phantoms the walk fails and the em answers, however deep its
// chain: H as a chain of composites down to I, with that many empty glyphs, or with a child declaring that many points.
#[test]
fn a_composite_past_the_walks_limits_answers_the_em_for_its_origin() {
    let rel = "bungee-tint/BungeeTint-Regular.ttf";
    let ids = |c: char| font(rel).glyph_id(c).expect("mapped").0;
    let (h, i, space) = (ids('H'), ids('I'), ids(' '));
    let chain = |n: u32| -> Vec<(u32, Vec<u8>)> {
        let gids: Vec<u32> = std::iter::once(h).chain(200..200 + n - 1).collect();
        gids.iter().enumerate().map(|(k, &g)| (g, composite(700, &[*gids.get(k + 1).unwrap_or(&i)]))).collect()
    };
    let points = |n: i16| vec![(h, composite(700, &[200])), (200, words(&[1, 0, 0, 500, 650, n - 1, 0]))];
    for (glyphs, want, what) in [
        (chain(100), 860.0, "100 levels"),
        (chain(1_900), 860.0, "1,900 levels"),
        (vec![(h, composite(700, &vec![space; 65_534]))], 860.0, "65,534 components"),
        (vec![(h, composite(700, &vec![space; 65_535]))], 1000.0, "65,535 components"),
        (points(-5), 710.0, "a child of 65,531 points"),
        (points(-4), 1000.0, "a child of 65,532 points"),
    ] {
        assert_eq!(origin(&crate::prepared::patched(rel, &glyphs), 'H', &[]), want, "{what}");
    }
}

// Without vertical metrics, a record of 0 contours, outside glyf or running backward draws nothing and centers on the
// line, 1000 tall; one under its 10-byte header fails to draw, and with no box takes the ascender, as past loca.
#[test]
fn an_empty_glyph_has_no_box_to_center() {
    let rel = "test-fixtures/hinted.ttf";
    assert_eq!(origin(&font(rel), 'A', &[]), 851.0, "yMax 703 and a box 703 tall");
    let empty: [&[u8]; 3] =
        [&[], &[0, 0, 0, 0x25, 0, 0, 1, 0x51, 2, 0xbf], &[0, 0, 0, 0x25, 0, 0, 1, 0x51, 2, 0xbf, 0, 0]];
    for record in empty {
        assert_eq!(origin(&crate::prepared::hinted_with(1, record), 'A', &[]), 500.0, "{record:?}");
    }
    let short: [&[u8]; 3] = [&[0, 1], &[0, 1, 0, 0x25], &[0, 1, 0, 0x25, 0, 0, 1, 0x51, 2]];
    for record in short {
        assert_eq!(origin(&crate::prepared::hinted_with(1, record), 'A', &[]), 800.0, "{record:?}");
    }
    let f = font(rel);
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    let loca = tables.get_mut(&Tag::new(b"loca")).expect("loca");
    let last = loca.len() - 2;
    let end = u16::from_be_bytes([loca[last], loca[last + 1]]) + 1;
    loca[last..].copy_from_slice(&end.to_be_bytes());
    let past = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("parses");
    assert_eq!(origin(&past, 'F', &[]), 500.0, "the last glyph running past glyf's end");
    // E's end past glyf's 214 bytes, so F, starting there, runs backward.
    tables.get_mut(&Tag::new(b"loca")).expect("loca")[12..14].copy_from_slice(&200u16.to_be_bytes());
    let cut = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("parses");
    assert_eq!([origin(&cut, 'E', &[]), origin(&cut, 'F', &[])], [500.0; 2], "E past glyf's end, F backward");
    // A glyph past loca's has no box and takes the ascender; so does every glyph where loca holds none, which takes
    // no phantom point though vmtx gives bearings.
    let plain = font(rel);
    let one = with_tables(&plain, vec![("loca", vec![0, 0, 0, 0])]);
    assert_eq!(origin(&one, 'A', &[]), 800.0, "past loca's one glyph");
    let mut none = vertical_metrics(&plain);
    none.push(("loca", vec![0, 0]));
    assert_eq!(origin(&with_tables(&plain, none), 'A', &[]), 800.0, "loca holding no glyph, with vmtx");
}

// A glyph of 0 contours keeps its stored box for its top phantom point, and is empty at a location too, where an
// instance writes it empty: gvar-points.ttf's glyph 0 topped at 900, with a top bearing of 50.
#[test]
fn a_glyph_of_no_contours_is_empty_at_a_location_and_in_its_instance() {
    let rel = "test-fixtures/gvar-points.ttf";
    let base = crate::prepared::patched(rel, &[(0, words(&[0, -200, -300, 500, 900, 0]))]);
    let f = with_tables(&base, vertical_metrics(&base));
    for axes in [&[][..], &[("wght", 900.0)]] {
        assert_eq!(origin(&f, 'z', axes), 950.0, "{axes:?}");
    }
    let instance = Font::from_bytes(&f.instance(&located(&f, &[("wght", 900.0)])).expect("instances")).expect("parses");
    let loca = instance.table(Tag::new(b"loca")).expect("loca");
    let at = |g: usize| match instance.table(Tag::new(b"head")).expect("head")[51] {
        0 => 2 * usize::from(u16::from_be_bytes([loca[2 * g], loca[2 * g + 1]])),
        _ => u32::from_be_bytes(loca[4 * g..4 * g + 4].try_into().expect("four")) as usize,
    };
    assert_eq!(at(1) - at(0), 0, "the instance keeps the empty glyph's stored box");
    assert_eq!(origin(&instance, 'z', &[]), 950.0, "the instance's origin");
    let head = instance.table(Tag::new(b"head")).expect("head");
    let x_min = i16::from_be_bytes([head[36], head[37]]);
    assert!(x_min >= 0, "head's box takes the empty glyph's: xMin {x_min}");
}

// A composite containing itself is measured past the repeat at a location too, gathering no points, but draws
// nothing, so its instance writes it empty: gvar-points.ttf's glyph 0 tops at 700, bearing 50.
#[test]
fn a_composite_containing_itself_is_measured_past_the_repeat_at_a_location() {
    let base = crate::prepared::patched("test-fixtures/gvar-points.ttf", &[(0, composite(700, &[0]))]);
    let f = with_tables(&base, vertical_metrics(&base));
    for axes in [&[][..], &[("wght", 900.0)]] {
        assert_eq!(origin(&f, 'z', axes), 750.0, "{axes:?}");
    }
    let instance = Font::from_bytes(&f.instance(&located(&f, &[("wght", 900.0)])).expect("instances")).expect("parses");
    assert_eq!(origin(&instance, 'z', &[]), 750.0, "the instance's origin");
    let loca = instance.table(Tag::new(b"loca")).expect("loca");
    let width = if instance.table(Tag::new(b"head")).expect("head")[51] == 0 { 2 } else { 4 };
    assert_eq!(loca[..width], loca[width..2 * width], "written empty");
}

// An empty glyph keeps its side bearing in an instance, its box of 0 at its own origin: gvar-points.ttf's glyph 0,
// which gvar doesn't vary, given a bearing of 60, keeps it at wght 900.
#[test]
fn an_empty_glyph_keeps_its_side_bearing_in_an_instance() {
    let base = font("test-fixtures/gvar-points.ttf");
    let mut hmtx = base.table(Tag::new(b"hmtx")).expect("hmtx").to_vec();
    hmtx[2..4].copy_from_slice(&60i16.to_be_bytes());
    let f = with_tables(&base, vec![("hmtx", hmtx)]);
    let instance = Font::from_bytes(&f.instance(&located(&f, &[("wght", 900.0)])).expect("instances")).expect("parses");
    let hmtx = instance.table(Tag::new(b"hmtx")).expect("hmtx");
    assert_eq!(i16::from_be_bytes([hmtx[2], hmtx[3]]), 60);
}

// A glyph of 0 contours with its whole header draws as a space, and a composite holding one as the rest of it; a
// record under the header is refused.
#[test]
fn a_glyph_of_no_contours_draws_nothing_and_a_composite_holding_one_draws() {
    let empty = [0, 0, 0, 0x25, 0, 0, 1, 0x51, 2, 0xbf];
    let f = crate::prepared::patched("test-fixtures/hinted.ttf", &[(1, empty.to_vec()), (3, composite(700, &[1, 2]))]);
    let mut path = daegun::outline::Path::default();
    assert!(
        f.outline(GlyphId(1), &Location::default(), &mut path).is_ok() && path.bounds().is_none(),
        "the empty glyph"
    );
    let mut path = daegun::outline::Path::default();
    assert!(
        f.outline(GlyphId(3), &Location::default(), &mut path).is_ok() && path.bounds().is_some(),
        "the composite holding it"
    );
    let short = crate::prepared::hinted_with(1, &[0, 0, 0, 0]);
    assert!(
        short.outline(GlyphId(1), &Location::default(), &mut daegun::outline::Path::default()).is_err(),
        "a record under its header"
    );
}

#[test]
fn a_guillemet_after_an_arabic_word_takes_the_arabic_form_without_flipping() {
    let f = font("scheherazade-new/ScheherazadeNew-Regular.ttf");

    let seeded = |seed| {
        f.shape_bidi(
            "\u{00BB}",
            &Location::default(),
            BaseDirection::LeftToRight,
            &daegun::shape::ShapeOptions::default().with_seed_script(seed),
        )
        .iter()
        .flat_map(|r| r.run.glyphs.clone())
        .collect::<Vec<GlyphId>>()
    };
    let (arabic_form, latin_form) = (seeded(daegun::unicode::Script::ARABIC), seeded(daegun::unicode::Script::LATIN));
    assert_ne!(
        arabic_form, latin_form,
        "the fixture does not distinguish the guillemet by script at a fixed direction, so nothing \
         below can prove the seed arrived",
    );

    let text = "abc \u{0639}\u{0631}\u{0628}\u{064A} \u{00BB} def";
    let opts = daegun::layout::LayoutOptions::default().with_max_inline_size(100_000.0);
    let laid = f.layout(text, &Location::default(), &opts);
    let runs: Vec<_> = laid.lines.iter().flat_map(|l| l.runs.iter()).collect();

    let holder = runs
        .iter()
        .find(|r| r.bytes.contains(&13))
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
    let opts = daegun::layout::LayoutOptions::default().with_max_inline_size(100_000.0);

    let text = "\u{0633}\u{0644}\u{0627}\u{0645} hello \u{0633}\u{0644}\u{0627}\u{0645}";
    let laid = f.layout(text, &Location::default(), &opts);
    let runs: Vec<_> = laid.lines.iter().flat_map(|l| l.runs.iter()).collect();
    assert!(runs.iter().any(|r| r.level % 2 == 1), "no right-to-left run, so the text is not mixed");
    let even: Vec<_> = runs.iter().filter(|r| r.level % 2 == 0).collect();
    assert!(!even.is_empty(), "no left-to-right run, so this proves nothing");

    let alone = f.shape("hello", &Location::default(), &ShapeOptions::default()).glyphs.clone();
    let found: Vec<GlyphId> = even.iter().flat_map(|r| r.run.glyphs.clone()).collect();
    assert!(
        found.windows(alone.len()).any(|w| w == alone.as_slice()),
        "the even-level run did not come back left-to-right: expected {alone:?} inside {found:?}",
    );

    let all_arabic = f.shape("\u{0633}\u{0644}\u{0627}\u{0645}", &Location::default(), &ShapeOptions::default());
    let reversed = f.shape("\u{0645}\u{0627}\u{0644}\u{0633}", &Location::default(), &ShapeOptions::default());
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
        let n = f.glyph_count().min(300);
        for gid in 0..n {
            let mut direct = daegun::outline::QuadraticPen::new(f32::from(f.units_per_em()) / 4096.0);
            if f.outline(GlyphId(gid), &Location::default(), &mut direct).is_err() {
                continue;
            }
            let mut path = daegun::outline::Path::default();
            f.outline(GlyphId(gid), &Location::default(), &mut path).expect("the second decode disagreed");
            let mut replayed = daegun::outline::QuadraticPen::new(f32::from(f.units_per_em()) / 4096.0);
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

// Calls whose features differ only in where they apply share one plan, and each applies its own range: every call
// matches a font that shaped nothing before it, so no call reads an earlier one's range.
#[test]
fn a_shared_plan_applies_each_calls_ranges() {
    let text = "fi office fi";
    let shaped = |font: &Font, start: usize, end: usize| {
        let features = [Feature { tag: Tag::new(b"liga"), value: 0, bytes: (start..end).into() }];
        font.shape(text, &Location::default(), &daegun::shape::ShapeOptions::default().with_features(&features))
    };
    let shared = font(GARAMOND);
    let ranges = [(0, 2), (3, 9), (10, 12), (0, 12), (0, 2)];
    let runs: Vec<_> = ranges.iter().map(|&(start, end)| shaped(&shared, start, end)).collect();
    for (&(start, end), run) in ranges.iter().zip(&runs) {
        let alone = shaped(&font(GARAMOND), start, end);
        assert_eq!(
            (&run.glyphs, &run.clusters, &run.advances),
            (&alone.glyphs, &alone.clusters, &alone.advances),
            "liga off over {start}..{end} shaped as an earlier call's range",
        );
    }
    assert_ne!(runs[0].glyphs, runs[2].glyphs, "the two ranges shape alike, so nothing is proven");
}
