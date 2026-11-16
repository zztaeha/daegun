use std::collections::BTreeMap;

use daegun::{ErrorKind, Font, GlyphId, Location, Orientation};

use super::FONTS;
use daegun::Tag;
use crate::located;
use daegun::shape::ShapeOptions;

const SOURCE_HAN: &str = "source-han-sans/SourceHanSansJP-VF.otf";

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font"), 0).expect("it parses")
}

fn tables(f: &Font) -> BTreeMap<Tag, Vec<u8>> {
    f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect()
}

fn baselines(f: &Font, script: &[u8; 4], orientation: Orientation, axes: &[(&str, f64)]) -> Vec<(Tag, f64)> {
    f.base_script(Tag::new(script), &located(f, axes), orientation).expect("baselines").baselines.into_iter().collect()
}

fn named(values: [f64; 4]) -> Vec<(Tag, f64)> {
    [b"icfb", b"icft", b"ideo", b"romn"].map(Tag::new).into_iter().zip(values).collect()
}

// The table's stored values: icfb and icft are format 3, moved by BASE's store, and read with the rest.
#[test]
fn baselines_are_their_stored_values_moved_by_the_store() {
    let f = font(SOURCE_HAN);
    let heavy = [("wght", 900.0)];
    let (horizontal, vertical) = (Orientation::Horizontal, Orientation::Vertical);
    assert_eq!(baselines(&f, b"hani", horizontal, &[]), named([-67.0, 827.0, -120.0, 0.0]));
    assert_eq!(baselines(&f, b"hani", horizontal, &heavy), named([-94.0, 854.0, -120.0, 0.0]));
    assert_eq!(baselines(&f, b"hani", vertical, &[]), named([53.0, 947.0, 0.0, 120.0]));
    assert_eq!(baselines(&f, b"hani", vertical, &heavy), named([26.0, 974.0, 0.0, 120.0]));
}

// Arabic has no record of its own, so DFLT's answers, its default baseline ideo where Latin's is romn.
#[test]
fn a_script_without_baselines_takes_dflts() {
    let f = font(SOURCE_HAN);
    let default = |script| {
        f.base_script(Tag::new(script), &Location::default(), Orientation::Horizontal).and_then(|i| i.default_baseline)
    };
    assert_eq!(default(b"arab"), Some(Tag::new(b"ideo")));
    assert_eq!(default(b"latn"), Some(Tag::new(b"romn")));
}

// At 2,000 units per em, baselines are in font units as every other measure is: what BASE holds, not scaled to an em.
#[test]
fn baselines_are_in_font_units_whatever_the_em() {
    let mut t = tables(&font(SOURCE_HAN));
    t.get_mut(&Tag::new(b"head")).expect("a head")[18..20].copy_from_slice(&2000u16.to_be_bytes());
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses");
    assert_eq!(baselines(&f, b"hani", Orientation::Horizontal, &[]), named([-67.0, 827.0, -120.0, 0.0]));
}

// Source Serif 4's STAT, the table's stored values, every value and not just its shape.
#[test]
fn stat_values_are_the_table_s_stored_values() {
    use daegun::metrics::StatAxisValue;
    let info = font("source-serif/SourceSerif4Variable-Roman.otf").stat(&Location::default()).expect("a STAT");
    let axes: Vec<_> = info.axes.iter().map(|a| (a.tag, a.name.as_deref(), a.ordering)).collect();
    let named = |tag, name| (Tag::new(tag), Some(name));
    let [opsz, wght, ital] = [named(b"opsz", "Optical Size"), named(b"wght", "Weight"), named(b"ital", "Italic")];
    assert_eq!(axes, [(opsz.0, opsz.1, 0), (wght.0, wght.1, 1), (ital.0, ital.1, 2)]);
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

// gvar-points's composite E has point 1 at x 500, and 520 at wght 900 once its left phantom point's 30 shift is taken
// as the origin; a simple glyph's default points alone would give no caret.
#[test]
fn a_caret_on_a_point_follows_it_through_composition_and_variation() {
    let mut t = tables(&font("test-fixtures/gvar-points.ttf"));
    let mut gdef = words(&[1, 0, 0, 0, 12, 0]);
    gdef.extend(lig_caret_list(5, &[2, 1]));
    t.insert(Tag::new(b"GDEF"), gdef);
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses");
    assert_eq!(f.ligature_carets(GlyphId(5), &Location::default(), Orientation::Horizontal), [Some(500.0)]);
    assert_eq!(f.ligature_carets(GlyphId(5), &located(&f, &[("wght", 900.0)]), Orientation::Horizontal), [Some(520.0)]);
}

// The same E with its left side bearing 50 past where it was: drawing sets its left phantom point 50 further left, so
// the caret, measured from there as the drawn point is, lands 50 further right.
#[test]
fn a_caret_on_a_point_is_measured_from_the_drawn_origin() {
    let mut t = tables(&font("test-fixtures/gvar-points.ttf"));
    let mut gdef = words(&[1, 0, 0, 0, 12, 0]);
    gdef.extend(lig_caret_list(5, &[2, 1]));
    t.insert(Tag::new(b"GDEF"), gdef);
    let long = usize::from(u16::from_be_bytes([t[&Tag::new(b"hhea")][34], t[&Tag::new(b"hhea")][35]]));
    let at = if 5 < long { 4 * 5 + 2 } else { 4 * long + 2 * (5 - long) };
    let hmtx = t.get_mut(&Tag::new(b"hmtx")).expect("an hmtx");
    let lsb = i16::from_be_bytes([hmtx[at], hmtx[at + 1]]) + 50;
    hmtx[at..at + 2].copy_from_slice(&lsb.to_be_bytes());
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses");
    assert_eq!(f.ligature_carets(GlyphId(5), &Location::default(), Orientation::Horizontal), [Some(550.0)]);
}

// Down, E's point 1 at y 0 sits below where drawing puts the vertical origin: without vmtx (or with none counted),
// the typo height centered on its 0 to 700, -201's odd one landing on a half; with it, yMax 700 plus 100.
#[test]
fn a_caret_down_is_measured_from_the_drawn_vertical_origin() {
    let mut t = tables(&font("test-fixtures/gvar-points.ttf"));
    let mut gdef = words(&[1, 0, 0, 0, 12, 0]);
    gdef.extend(lig_caret_list(5, &[2, 1]));
    t.insert(Tag::new(b"GDEF"), gdef);
    let down = |t: &BTreeMap<Tag, Vec<u8>>, axes: &[(&str, f64)]| {
        let f = Font::from_vec(daegun::raw::build_font(t).unwrap(), 0).expect("the patched font parses");
        f.ligature_carets(GlyphId(5), &located(&f, axes), Orientation::Vertical)
    };
    assert_eq!(down(&t, &[]), [Some(-350.0)]);
    let mut odd = t.clone();
    odd.get_mut(&Tag::new(b"OS/2")).expect("an OS/2")[70..72].copy_from_slice(&(-201i16).to_be_bytes());
    assert_eq!(down(&odd, &[]), [Some(-450.5)]);
    let mut vhea = words(&[1, 0x1000]);
    vhea.resize(34, 0);
    vhea.extend(words(&[6]));
    t.insert(Tag::new(b"vhea"), vhea);
    t.insert(Tag::new(b"vmtx"), words(&[1000, 0, 1000, 0, 1000, 0, 1000, 0, 1000, 0, 1000, 100]));
    assert_eq!(down(&t, &[]), [Some(-800.0)]);
    assert_eq!(down(&t, &[("wght", 900.0)]), [Some(-800.0)]);
    t.get_mut(&Tag::new(b"vhea")).expect("the vhea")[34..36].copy_from_slice(&[0, 0]);
    assert_eq!(down(&t, &[]), [Some(-350.0)]);
}

// Scheherazade's lam-alef (1,318 of 2,048 units) with a caret at 400: its lam is on the right, so the
// boundary is 400 from the left edge, where the right edge would give 918.
#[test]
fn a_right_to_left_ligature_splits_at_its_caret() {
    let source = font("scheherazade-new/ScheherazadeNew-Regular.ttf");
    let lam_alef = source.shape("لا", &Location::default(), &ShapeOptions::default()).glyphs.clone();
    assert_eq!(lam_alef.len(), 1, "lam and alef did not ligate");
    let mut t = tables(&source);
    let gdef = t.get_mut(&Tag::new(b"GDEF")).expect("a GDEF");
    let at = gdef.len() as u16;
    gdef[8..10].copy_from_slice(&at.to_be_bytes());
    gdef.extend(lig_caret_list(lam_alef[0].0 as u16, &[1, 400]));
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses");
    let positions = f.caret_positions("لا", &Location::default(), Orientation::Horizontal);
    assert_eq!(positions, [1318.0, 1318.0, 400.0, 400.0, 0.0], "a caret for each byte, two to a letter");
}

const STIX: &str = "stix-two-math/STIX2Math.otf";

fn with_math(patch: impl FnOnce(&mut Vec<u8>)) -> Font {
    let mut t = tables(&font(STIX));
    patch(t.get_mut(&Tag::new(b"MATH")).expect("a MATH"));
    Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses")
}

// STIX's A, top right: correction heights 252 and 352, kerns 0, -18 and -66. A height under 252 by
// any amount takes the first, though 251.6 rounded to a whole unit would take the second. NaN has none.
#[test]
fn a_math_kern_compares_the_height_unrounded() {
    let f = font(STIX);
    let kern = |height| f.math_kern(GlyphId(3), &Location::default(), daegun::math::MathKernCorner::TopRight, height);
    assert_eq!(kern(251.6), Ok(0.0));
    assert_eq!(kern(252.0), Ok(-18.0));
    assert_eq!(kern(f64::NAN).map_err(|e| e.kind()), Err(ErrorKind::InvalidArgument));
}

// delimitedSubFormulaMinHeight is a UFWORD: 40,000 read as a signed 16-bit value would be -25,536.
#[test]
fn unsigned_math_heights_stay_unsigned() {
    let f = with_math(|math| {
        let constants = usize::from(u16::from_be_bytes([math[4], math[5]]));
        math[constants + 4..constants + 6].copy_from_slice(&40_000u16.to_be_bytes());
    });
    assert_eq!(f.math_constants(&Location::default()).map(|c| c.delimited_sub_formula_min_height), Some(40_000.0));
}

// STIX's A attaches its top accent at 360 font units, whatever the em.
#[test]
fn the_top_accent_attachment_is_in_font_units_whatever_the_em() {
    let mut t = tables(&font(STIX));
    t.get_mut(&Tag::new(b"head")).expect("a head")[18..20].copy_from_slice(&2048u16.to_be_bytes());
    let f = Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses");
    assert_eq!(f.math_top_accent_attachment(GlyphId(3), &Location::default()), 360.0);
}

// math-var.ttf's values at a location, each stored value and its delta there: half its delta at 900 at wght 650, and
// all its delta at 100 at wght 100. Coverage alone doesn't vary, nor do sizes.
#[test]
fn math_values_vary_at_a_location() {
    use daegun::math::MathKernCorner::TopRight;
    let f = font("test-fixtures/math-var.ttf");
    let (x, paren) = (f.glyph_id('x').expect("x"), f.glyph_id('(').expect("("));
    let wght = |v: f64| f.location(&[(Tag::new(b"wght"), v)]);
    let read = |at: &Location| {
        let constants = f.math_constants(at).expect("constants");
        let assembly = f.math_glyph_variants(paren, at, Orientation::Vertical).and_then(|c| c.assembly).expect("parts");
        [
            constants.math_leading,
            constants.axis_height,
            f.math_italics_correction(x, at).expect("a correction"),
            f.math_top_accent_attachment(x, at),
            f.math_kern(x, at, TopRight, 0.0).expect("a kern"),
            assembly.italics_correction,
        ]
    };
    assert_eq!(read(&Location::default()), [100.0, 250.0, 10.0, 150.0, 30.0, 40.0]);
    assert_eq!(read(&wght(650.0)), [120.0, 265.0, 20.0, 135.0, 55.0, 48.0]);
    assert_eq!(read(&wght(100.0)), [80.0, 240.0, 2.0, 162.0, 16.0, 34.0]);
    // The correction height moves too: 500 at the default, 550 at wght 650, 460 at wght 100.
    let kern = |at: &Location, height: f64| f.math_kern(x, at, TopRight, height).expect("a kern");
    assert_eq!((kern(&Location::default(), 499.0), kern(&Location::default(), 500.0)), (30.0, 0.0));
    assert_eq!((kern(&wght(650.0), 549.0), kern(&wght(650.0), 550.0)), (55.0, 0.0));
    assert_eq!((kern(&wght(100.0), 459.0), kern(&wght(100.0), 460.0)), (16.0, 0.0));
    let constructed = f.math_glyph_variants(paren, &wght(650.0), Orientation::Vertical).expect("a construction");
    assert_eq!(constructed.variants[0].advance, 700.0);
    assert_eq!(f.math_min_connector_overlap(), Some(20.0));
    assert!(!f.math_is_extended_shape(x));
}

// A glyph MATH gives no accent for centers on half its advance, at the location asked: Inter's, as it varies.
#[test]
fn a_default_accent_follows_the_advance_at_the_location() {
    let f = font("inter/InterVariable.ttf");
    let h = f.glyph_id('H').expect("H");
    let heavy = f.location(&[(Tag::new(b"wght"), 900.0)]);
    for at in [Location::default(), heavy] {
        assert_eq!(f.math_top_accent_attachment(h, &at), f.advance(h, &at, Orientation::Horizontal) / 2.0);
    }
    let accent = |at: &Location| f.math_top_accent_attachment(h, at);
    assert_ne!(accent(&Location::default()), accent(&f.location(&[(Tag::new(b"wght"), 900.0)])));
}

// A NULL offset in MATH's header is no subtable; followed to offset 0, the header would read as one.
#[test]
fn null_math_subtables_are_absent() {
    let zero = |at: usize| with_math(move |math| math[at..at + 2].fill(0));
    assert!(zero(4).math_constants(&Location::default()).is_none());
    assert!(zero(8).math_min_connector_overlap().is_none());
}

// A trak of tracks -1, 0 and 1 over sizes 9, 12 and 24, the normal track giving 20, 0 and -30. The last track's
// values are the table's final 6 bytes.
fn trak() -> Vec<u8> {
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
    trak
}

// EB Garamond with `trak()`, after `change` edits its tables.
fn tracked_garamond_with(change: impl FnOnce(&mut BTreeMap<Tag, Vec<u8>>)) -> Font {
    let mut t = tables(&font("eb-garamond/EBGaramond.ttf"));
    t.insert(Tag::new(b"trak"), trak());
    change(&mut t);
    Font::from_vec(daegun::raw::build_font(&t).unwrap(), 0).expect("the patched font parses")
}

fn tracked_garamond() -> Font {
    tracked_garamond_with(|_| ())
}

fn advance_at(f: &Font, text: &str, point_size: Option<f64>) -> f64 {
    let opts = point_size.map_or(ShapeOptions::default(), |size| ShapeOptions::default().with_point_size(size));
    f.shape(text, &Location::default(), &opts).advances[0]
}

// What tracking adds to the advance of 'a' (399) at each point size:
// the end sizes' values past the range, and between sizes a straight line.
#[test]
fn tracking_follows_the_point_size() {
    let f = tracked_garamond();
    let sizes = [6.0, 9.0, 12.0, 18.0, 24.0, 30.0];
    assert_eq!(sizes.map(|p| f.tracking(p, Orientation::Horizontal)), [20.0, 20.0, 0.0, -15.0, -30.0, -30.0]);
    let advance = |point_size| advance_at(&f, "a", point_size);
    assert_eq!([None, Some(6.0), Some(18.0)].map(advance), [399.0, 419.0, 384.0]);
    // A size that isn't one reads at the default; a finite one past the last size takes that size's value.
    for size in [0.0, -6.0, f64::NAN, f64::INFINITY] {
        assert_eq!(f.tracking(size, Orientation::Horizontal), 0.0, "at {size}");
        assert_eq!(advance(Some(size)), 399.0, "shaped at {size}");
    }
    assert_eq!((f.tracking(1e300, Orientation::Horizontal), advance(Some(1e300))), (-30.0, 369.0));
}

// Halfway from 12 pt's 0 to 18 pt's -15, 15 pt tracks by -7.5, which a shaped advance rounds half up to -7.
#[test]
fn shaped_tracking_rounds_half_up() {
    let f = tracked_garamond();
    assert_eq!(f.tracking(15.0, Orientation::Horizontal), -7.5);
    assert_eq!(advance_at(&f, "a", Some(15.0)), 392.0);
}

// Shaping tracks only a font with a STAT, a 1.0 one as much as 1.1; without one the table still states its tracking.
#[test]
fn shaping_tracks_only_a_font_with_a_stat() {
    let without = tracked_garamond_with(|t| { t.remove(&Tag::new(b"STAT")); });
    let first = tracked_garamond_with(|t| t.get_mut(&Tag::new(b"STAT")).expect("a STAT")[2..4].fill(0));
    for (f, advance) in [(&without, 399.0), (&first, 419.0)] {
        assert_eq!(f.tracking(6.0, Orientation::Horizontal), 20.0);
        assert_eq!(advance_at(f, "a", Some(6.0)), advance);
    }
}

// A trak of another major version tracks nothing. Cut at 72 bytes, its last track's values leave the table, so that
// track is absent and the normal one still adds 20 at 6 pt.
#[test]
fn a_trak_tracks_by_the_parts_it_holds() {
    let second = tracked_garamond_with(|t| t.get_mut(&Tag::new(b"trak")).expect("a trak")[1] = 2);
    let cut = tracked_garamond_with(|t| t.get_mut(&Tag::new(b"trak")).expect("a trak").truncate(72));
    for (f, tracking) in [(&second, 0.0), (&cut, 20.0)] {
        assert_eq!(f.tracking(6.0, Orientation::Horizontal), tracking);
        assert_eq!(advance_at(f, "a", Some(6.0)), 399.0 + tracking);
    }
}

// STAT names no glyph, so a subset keeps it and tracks as its font: EB Garamond, and STIX Two Math's CFF with the
// same trak and Garamond's STAT. 'a' at 6 pt: 419 and 500.
#[test]
fn a_subset_keeps_stat_and_tracks_as_its_font() {
    let mut stix = tables(&font(STIX));
    stix.insert(Tag::new(b"trak"), trak());
    stix.insert(
        Tag::new(b"STAT"),
        tables(&font("eb-garamond/EBGaramond.ttf")).remove(&Tag::new(b"STAT")).expect("a STAT"),
    );
    let stix = Font::from_vec(daegun::raw::build_font(&stix).unwrap(), 0).expect("the patched font parses");
    for (f, want) in [(tracked_garamond(), 419.0), (stix, 500.0)] {
        let subset = Font::from_vec(f.subset_text("a", &Location::default()).expect("subsets").font, 0)
            .expect("the subset loads");
        assert!(subset.has_table(Tag::new(b"STAT")), "the subset dropped STAT");
        assert_eq!([advance_at(&f, "a", Some(6.0)), advance_at(&subset, "a", Some(6.0))], [want, want]);
    }
}

// A matra the Indic model moves before its consonant continues their grapheme, so it takes no tracking at a run's
// start, as after a space: "कि" alone tracks as it does in " कि", its consonant alone further along. Without GSUB,
// whose scripts don't include Devanagari, the Indic model shapes them.
#[test]
fn a_reordered_matra_opening_a_run_takes_no_tracking() {
    let f = tracked_garamond_with(|t| { t.remove(&Tag::new(b"GSUB")); });
    let options = ShapeOptions::default().with_point_size(6.0);
    let whole = f.shape(" \u{915}\u{93F}", &Location::default(), &options);
    let alone = f.shape("\u{915}\u{93F}", &Location::default(), &options);
    assert_eq!(whole.advances[1..], alone.advances[..]);
    assert_eq!(alone.advances[1] - alone.advances[0], 20.0);
}

// Tracking applies while positioning, before marks are zeroed, so a lone mark ends with no advance.
#[test]
fn a_lone_mark_is_zeroed_after_its_tracking() {
    assert_eq!(advance_at(&tracked_garamond(), "\u{301}", Some(6.0)), 0.0);
}

// fuzz-reach.ttf answers every reader the fuzzers walk, whatever they pick: each glyph a three-part assembly both
// ways, each script its two baselines both ways and two justification levels, and a PostScript name for its instance.
#[test]
fn the_fuzz_fixture_answers_every_reader() {
    let f = font("test-fixtures/fuzz-reach.ttf");
    let at = Location::default();
    for gid in 0..f.glyph_count() {
        for orientation in [Orientation::Horizontal, Orientation::Vertical] {
            let parts = f.math_glyph_variants(GlyphId(gid), &at, orientation).and_then(|c| c.assembly);
            assert_eq!(parts.map(|a| a.parts.len()), Some(3), "glyph {gid} {orientation:?}");
        }
    }
    for script in [b"DFLT", b"arab", b"latn"] {
        let horizontal = [(Tag::new(b"ideo"), -120.0), (Tag::new(b"romn"), 0.0)];
        assert_eq!(baselines(&f, script, Orientation::Horizontal, &[]), horizontal);
        let vertical = [(Tag::new(b"ideo"), 0.0), (Tag::new(b"romn"), 120.0)];
        assert_eq!(baselines(&f, script, Orientation::Vertical, &[]), vertical);
        assert_eq!(f.justification_priorities(Tag::new(script), None).map(|l| l.len()), Some(2));
    }
    let instances = f.named_instances();
    assert_eq!(instances[0].postscript_name.as_deref(), Some("DaegunFuzzReach-Bold"));
}
