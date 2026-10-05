use std::collections::BTreeMap;

use daegun::Font;

use super::FONTS;

const SOURCE_HAN: &str = "source-han-sans/SourceHanSansJP-VF.otf";

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font")).expect("it parses")
}

fn tables(f: &Font) -> BTreeMap<String, Vec<u8>> {
    f.table_tags().into_iter().map(|t| (t.to_string(), f.table(t).expect("listed").to_vec())).collect()
}

fn baselines(f: &Font, script: &str, vertical: bool, axes: &[(&str, f64)]) -> Vec<(String, f64)> {
    f.base_info(script, vertical, axes).expect("baselines").baseline_coords.into_iter().collect()
}

fn named(values: [f64; 4]) -> Vec<(String, f64)> {
    ["icfb", "icft", "ideo", "romn"].iter().map(|t| t.to_string()).zip(values).collect()
}

// fontTools' values: icfb and icft are format 3, moved by BASE's store, and read with the rest.
#[test]
fn baselines_vary_as_fonttools_reads_them() {
    let f = font(SOURCE_HAN);
    let heavy = [("wght", 900.0)];
    assert_eq!(baselines(&f, "hani", false, &[]), named([-67.0, 827.0, -120.0, 0.0]));
    assert_eq!(baselines(&f, "hani", false, &heavy), named([-94.0, 854.0, -120.0, 0.0]));
    assert_eq!(baselines(&f, "hani", true, &[]), named([53.0, 947.0, 0.0, 120.0]));
    assert_eq!(baselines(&f, "hani", true, &heavy), named([26.0, 974.0, 0.0, 120.0]));
}

// Arabic has no record of its own, so DFLT's answers, its default baseline ideo where Latin's is romn.
#[test]
fn a_script_without_baselines_takes_dflts() {
    let f = font(SOURCE_HAN);
    let default = |script: &str| f.base_info(script, false, &[]).and_then(|i| i.default_baseline_tag);
    assert_eq!(default("arab").as_deref(), Some("ideo"));
    assert_eq!(default("latn").as_deref(), Some("romn"));
}

// At 2,000 units per em, baselines come in the 1000-unit em as every other measure does; in design
// units they would be twice as far.
#[test]
fn baselines_are_in_the_thousand_unit_em() {
    let mut t = tables(&font(SOURCE_HAN));
    t.get_mut("head").expect("a head")[18..20].copy_from_slice(&2000u16.to_be_bytes());
    let f = Font::from_vec(daegun::build_font(&t)).expect("the patched font parses");
    assert_eq!(baselines(&f, "hani", false, &[]), named([-33.5, 413.5, -60.0, 0.0]));
}

// Source Serif 4's STAT as fontTools reads it, every value and not just its shape.
#[test]
fn stat_values_are_what_fonttools_reads() {
    use daegun::StatAxisValue;
    let info = font("source-serif/SourceSerif4Variable-Roman.otf").stat_info().expect("a STAT");
    let axes: Vec<_> = info.axes.iter().map(|a| (a.tag.as_str(), a.name.as_deref(), a.ordering)).collect();
    assert_eq!(axes, [("opsz", Some("Optical Size"), 0), ("wght", Some("Weight"), 1), ("ital", Some("Italic"), 2)]);
    let range = |axis_index, name: &str, nominal, min, max, elidable| StatAxisValue::Range {
        axis_index, name: Some(name.to_string()), nominal, min, max, elidable, older_sibling: false,
    };
    let want = vec![
        range(1, "ExtraLight", 200.0, 200.0, 250.0, false),
        range(1, "Light", 300.0, 250.0, 350.0, false),
        range(1, "Regular", 400.0, 350.0, 450.0, true),
        range(1, "Medium", 500.0, 450.0, 550.0, false),
        range(1, "Semibold", 600.0, 550.0, 650.0, false),
        range(1, "Bold", 700.0, 650.0, 750.0, false),
        range(1, "ExtraBold", 775.0, 750.0, 800.0, false),
        range(1, "Black", 900.0, 800.0, 900.0, false),
        range(0, "Caption", 8.0, 8.0, 12.0, false),
        range(0, "SmallText", 16.0, 12.0, 18.0, false),
        range(0, "Text", 20.0, 18.0, 26.0, true),
        range(0, "Subhead", 32.0, 26.0, 48.0, false),
        range(0, "Display", 60.0, 48.0, 60.0, false),
        StatAxisValue::Linked {
            axis_index: 2, name: Some("Regular".to_string()), value: 0.0, linked_value: 1.0, elidable: true,
            older_sibling: false,
        },
    ];
    assert_eq!(info.values, want);
    assert_eq!(info.elided_fallback_name.as_deref(), Some("Regular"));
}

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// A LigCaretList covering `gid` with one caret: `caret`'s words.
fn lig_caret_list(gid: u16, caret: &[u16]) -> Vec<u8> {
    let mut t = words(&[10 + 2 * caret.len() as u16, 1, 6, 1, 4]);
    t.extend(words(caret));
    t.extend(words(&[1, 1, gid]));
    t
}

// gvar-points's E is a composite, its point 1 at x 500 by default and 550 at wght 900, as fontTools
// instances it. Read from a simple glyph's default points alone, a format 2 caret would have none.
#[test]
fn a_caret_on_a_point_follows_it_through_composition_and_variation() {
    let mut t = tables(&font("test-fixtures/gvar-points.ttf"));
    let mut gdef = words(&[1, 0, 0, 0, 12, 0]);
    gdef.extend(lig_caret_list(5, &[2, 1]));
    t.insert("GDEF".into(), gdef);
    let f = Font::from_vec(daegun::build_font(&t)).expect("the patched font parses");
    assert_eq!(f.ligature_carets(5, &[], false), [Some(500.0)]);
    assert_eq!(f.ligature_carets(5, &[("wght", 900.0)], false), [Some(550.0)]);
    assert_eq!(f.ligature_carets(5, &[], true), [Some(0.0)]);
}

// Scheherazade's lam-alef (1,318 of 2,048 units) with a caret at 400: its lam is on the right, so the
// boundary is 400 from the left edge, 195.3 in the 1000-unit em, where the right edge would give 448.2.
#[test]
fn a_right_to_left_ligature_splits_at_its_caret() {
    let source = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let lam_alef = source.shape("لا", &[], false).expect("a run").glyphs.clone();
    assert_eq!(lam_alef.len(), 1, "lam and alef did not ligate");
    let mut t = tables(&source);
    let gdef = t.get_mut("GDEF").expect("a GDEF");
    let at = gdef.len() as u16;
    gdef[8..10].copy_from_slice(&at.to_be_bytes());
    gdef.extend(lig_caret_list(lam_alef[0], &[1, 400]));
    let f = Font::from_vec(daegun::build_font(&t)).expect("the patched font parses");
    let positions = f.caret_positions("لا", &[], false).expect("caret positions");
    let em = |units: f64| units * 1000.0 / 2048.0;
    assert_eq!(positions, [em(1318.0), em(400.0), 0.0]);
}

const STIX: &str = "stix-two-math/STIX2Math.otf";

fn with_math(patch: impl FnOnce(&mut Vec<u8>)) -> Font {
    let mut t = tables(&font(STIX));
    patch(t.get_mut("MATH").expect("a MATH"));
    Font::from_vec(daegun::build_font(&t)).expect("the patched font parses")
}

// STIX's A, top right: correction heights 252 and 352, kerns 0, -18 and -66. A height under 252 by
// any amount takes the first, though 251.6 rounded to a whole unit would take the second. NaN has none.
#[test]
fn a_math_kern_compares_the_height_unrounded() {
    let f = font(STIX);
    let kern = |height| f.math_kern(3, daegun::MathKernCorner::TopRight, height);
    assert_eq!(kern(251.6), Some(0.0));
    assert_eq!(kern(252.0), Some(-18.0));
    assert_eq!(kern(f64::NAN), None);
}

// delimitedSubFormulaMinHeight is a UFWORD: 40,000 read as a signed 16-bit value would be -25,536.
#[test]
fn unsigned_math_heights_stay_unsigned() {
    let f = with_math(|math| {
        let constants = usize::from(u16::from_be_bytes([math[4], math[5]]));
        math[constants + 4..constants + 6].copy_from_slice(&40_000u16.to_be_bytes());
    });
    assert_eq!(f.math_constants().map(|c| c.delimited_sub_formula_min_height), Some(40_000.0));
}

// STIX's A attaches its top accent at 360. At 2,048 units per em that is 175.78 in the 1000-unit em,
// as unrounded as every other MATH value, not 176.
#[test]
fn the_top_accent_attachment_is_not_rounded() {
    let mut t = tables(&font(STIX));
    t.get_mut("head").expect("a head")[18..20].copy_from_slice(&2048u16.to_be_bytes());
    let f = Font::from_vec(daegun::build_font(&t)).expect("the patched font parses");
    assert_eq!(f.math_top_accent_attachment(3), 360.0 * 1000.0 / 2048.0);
}

// A NULL offset in MATH's header is no subtable; followed to offset 0, the header would read as one.
#[test]
fn null_math_subtables_are_absent() {
    let zero = |at: usize| with_math(move |math| math[at..at + 2].fill(0));
    assert!(zero(4).math_constants().is_none());
    assert!(zero(8).math_min_connector_overlap().is_none());
}

// EB Garamond with a trak of tracks -1, 0 and 1 over sizes 9, 12 and 24, the normal track giving
// 20, 0 and -30.
fn tracked_garamond() -> Font {
    let mut t = tables(&font("eb-garamond/EBGaramond.ttf"));
    let mut trak = words(&[1, 0, 0, 12, 0, 0, 3, 3, 0, 44]);
    for (track, values) in [(0xFFFFu16, 56u16), (0, 62), (1, 68)] {
        trak.extend(words(&[track, 0, 256, values]));
    }
    for size in [9u16, 12, 24] {
        trak.extend(words(&[size, 0]));
    }
    for v in [40i16, 20, -10, 20, 0, -30, 0, -20, -50] {
        trak.extend(v.to_be_bytes());
    }
    t.insert("trak".into(), trak);
    Font::from_vec(daegun::build_font(&t)).expect("the patched font parses")
}

// What HarfBuzz adds to the advance of 'a' (399) at each point size, the same table built with
// fontTools: the end sizes' values past the range, and between sizes a straight line.
#[test]
fn tracking_follows_the_point_size_as_harfbuzz_applies_it() {
    let f = tracked_garamond();
    let sizes = [6.0, 9.0, 12.0, 18.0, 24.0, 30.0];
    assert_eq!(sizes.map(|p| f.tracking(p, true)), [20.0, 20.0, 0.0, -15.0, -30.0, -30.0]);
    let advance = |point_size| {
        let opts = daegun::ShapeOptions { point_size, ..Default::default() };
        f.shape_with_options("a", &[], false, &opts).expect("a run").advances[0]
    };
    assert_eq!([None, Some(6.0), Some(18.0)].map(advance), [399.0, 419.0, 384.0]);
}
