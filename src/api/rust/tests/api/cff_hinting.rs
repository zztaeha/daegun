use crate::Hint;
use daegun::{Font, GlyphId, Location};
use daegun::outline::{HintMode, HintedOutline, PointKind};

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"))
}

// FNV-1a over x then y in 26.6 as i64, flags & 3 (1 on the curve, 2 a cubic control), contour ends as i64: the
// layout the recorded hashes were made in.
fn hash(o: &HintedOutline) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    let mut eat = |b: &[u8]| b.iter().for_each(|&x| h = (h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3));
    let units = |px: f32| ((px * 64.0) as i64).to_le_bytes();
    o.points().for_each(|(p, _)| eat(&units(p.x)));
    o.points().for_each(|(p, _)| eat(&units(p.y)));
    let flag = |k| match k {
        PointKind::OnCurve => 1,
        PointKind::Cubic => 2,
        PointKind::Quadratic => 0,
    };
    eat(&o.points().map(|(_, k)| flag(k)).collect::<Vec<_>>());
    o.contours().iter().for_each(|&v| eat(&(v as i64).to_le_bytes()));
    h
}

// Glyphs a step apart hinted under Auto at 9, 16, 24 and 40 px against daegun's recorded hashes: zones (STIX Two Math),
// blended zones and BlueScale (Source Serif 4, default and instance 30), and a CFF2 font with no stem hints whose
// ideographs are hinted in its em box (Source Han Sans, default and 7th). Recorded from daegun's own output with
// Adobe's hint model, flat edges leaning outward where overshoot is suppressed, and the BASE em box.
#[test]
fn cff_hinting_lands_on_the_recorded_points() {
    const STIX: &str = "stix-two-math/STIX2Math.otf";
    let cases: [(&str, usize, u16, [u64; 4]); 5] = [
        (STIX, 0, 23, [0xa4006686930b4cee, 0x93437e14824d80bc, 0x28052e9aa9dd3096, 0xfdebd5c3409b49bf]),
        (SERIF, 0, 7, [0x6876317c4b2dce26, 0x8212aa9a485d79fe, 0x64ad70d97b652225, 0xb1c566d28dbefea5]),
        (SERIF, 30, 7, [0x5a79e93e3be2b303, 0xfb835a308c1354b1, 0x9c187e6bfc5ee1d7, 0x1fa5bb993afa0058]),
        (HAN, 0, 97, [0xff3f914434d4c984, 0xa79fc2bbf4a9350c, 0x13fd9d5ddae0128a, 0xb0b343de1a11c02e]),
        (HAN, 7, 97, [0xf43b299de170454d, 0xd2775bada822bf84, 0x803a23f654d2efed, 0x46bc546450bdfb2f]),
    ];
    for (rel, instance, step, want) in cases {
        let f = font(rel);
        let coords = instance.checked_sub(1).map_or_else(Vec::new, |n| f.named_instances()[n].coords.clone());
        let at = f.location(&coords);
        for (px, want) in [9.0, 16.0, 24.0, 40.0].into_iter().zip(want) {
            let mut digest = 0xcbf2_9ce4_8422_2325u64;
            for gid in (0..f.glyph_count()).step_by(usize::from(step)) {
                let h = f.hinted(GlyphId(gid), &at, px, HintMode::Auto).map_or(0, |o| hash(&o));
                h.to_le_bytes().iter().for_each(|&b| digest = (digest ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
            }
            assert_eq!(digest, want, "{rel} at instance {instance}, {px} px: not the recorded outlines");
        }
    }
}

const HAN: &str = "source-han-sans/SourceHanSansJP-VF.otf";
const SERIF: &str = "source-serif/SourceSerif4Variable-Roman.otf";

// Where BlueScale suppresses overshoot a flat edge rounds a third of a pixel outward: Source Serif's x-height, 475
// units, is 4.275 px at 9 px and lands on 5, the baseline on 0, overshoot and all.
#[test]
fn small_lowercase_keeps_its_x_height() {
    let f = font(SERIF);
    for c in ['x', 'n', 'o'] {
        let gid = f.glyph_id(c).expect("mapped");
        let out = f.hinted(gid, &Location::default(), 9.0, HintMode::Auto).expect("hinted");
        let (low, high) = out.points().fold((f32::MAX, f32::MIN), |(l, h), (p, _)| (l.min(p.y), h.max(p.y)));
        assert_eq!((low, high), (0.0, 5.0), "{c} at 9 px");
    }
}

// Every y a pen is given, a curve's three each.
#[derive(Default)]
struct Ys(Vec<f32>);

impl daegun::outline::OutlinePen for Ys {
    fn move_to(&mut self, _: f32, y: f32) { self.0.push(y); }
    fn line_to(&mut self, _: f32, y: f32) { self.0.push(y); }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, y: f32) { self.0.push(y); }
    fn curve_to(&mut self, _: f32, a: f32, _: f32, b: f32, _: f32, y: f32) { self.0.extend([a, b, y]); }
    fn close(&mut self) {}
}

// Source Han Sans's BASE puts the ideographic em box at -120 units ('ideo') to 880 (no 'idtp', so an em above). At
// 12 px its bottom, -1.44 px, lands on -1, and each point of 一 rises 0.44 px (28/64) with it. Latin's FD isn't
// ideographic, so A keeps the plain scale.
#[test]
fn an_ideograph_is_hinted_in_the_fonts_em_box() {
    let f = font(HAN);
    for (c, rise) in [('一', 28), ('A', 0)] {
        let gid = f.glyph_id(c).expect("mapped");
        let mut drawn = Ys::default();
        f.outline(gid, &Location::default(), &mut drawn).expect("drawn");
        let out = f.hinted(gid, &Location::default(), 12.0, HintMode::Auto).expect("hinted");
        let hinted: Vec<i64> = out.points().map(|(p, _)| (p.y * 64.0) as i64).collect();
        let plain: Vec<i64> = drawn.0.iter().map(|y| (y * 12.0 / 1000.0 * 64.0).round() as i64 + rise).collect();
        assert_eq!(hinted, plain, "{c}");
    }
}

// A glyph with no horizontal stem is hinted all the same, by the scale alone. Hashes recorded at 16 px from daegun's
// own output once the CFF hinter followed Adobe's hint model.
#[test]
fn a_cff_glyph_with_no_horizontal_stem_is_hinted_all_the_same() {
    let cases = [("test-fixtures/cff-subrs.otf", 2, 0x8c6b54dabde8417a), (HAN, 2, 0x4357109256a9bd71)];
    for (rel, gid, want) in cases {
        let f = font(rel);
        let hints = f.cff_hints(GlyphId(gid), &daegun::Location::default());
        let vertical_only = hints.is_some_and(|h| h.stems.iter().all(|s| s.vertical));
        assert!(vertical_only, "{rel}: glyph {gid} has a horizontal stem");
        let out = f.hinted(GlyphId(gid), &Location::default(), 16.0, HintMode::Auto);
        let out = out.unwrap_or_else(|| panic!("{rel}: glyph {gid} went unhinted"));
        assert_eq!(hash(&out), want, "{rel}: glyph {gid} isn't where it was recorded");
    }
}

// cff-operators.otf hinted at 16 px against daegun's recorded hashes, `random` seeded by -5: 16.16 arithmetic, late
// stems and mask operands dropped, 98 stems failing a glyph, a short mask ending it; None for a failed or empty glyph.
// Glyphs 1, 2, 3, 5, 8, 9, 11 and 12 recorded from daegun's own output once the CFF hinter followed Adobe's hint model.
#[test]
fn cff_operators_hint_to_the_recorded_outlines() {
    let f = font("test-fixtures/cff-operators.otf");
    let want: [Option<u64>; 21] = [
        Some(0x83b5425ad119686f), Some(0x961bb14e767dce20), Some(0x54fd37a0e5592424), Some(0xe737a0bae97dbd25),
        Some(0x06e347b53d83fe20), Some(0x93d79fa66ae5826c), Some(0x8da1f553a7cff320), Some(0x9f6e42fe2372c01b),
        Some(0x780237b55a79301e), None, Some(0x9f75676cff04bddd), Some(0x9f75676cff04bddd), Some(0xf3c90ff42e017787),
        Some(0x93d79fa66ae5826c), Some(0xb21a8407f9fccc4a), None, Some(0x898af9ec8154f716), None,
        Some(0x04599564b8ca9ca4), None, Some(0xc8d68dd1d1364e80),
    ];
    for (gid, want) in (1..).zip(want) {
        let got = f.hinted(GlyphId(gid), &Location::default(), 16.0, HintMode::Auto).map(|o| hash(&o));
        assert_eq!(got, want, "glyph {gid}: not the recorded outline");
    }
    // The ninth stem, declared after the first hintmask, is dropped, and the next mask names eight in one byte.
    let late = f.cff_hints(GlyphId(15), &daegun::Location::default()).expect("latestems draws");
    assert_eq!(late.stems.len(), 8);
    assert!(late.masks.iter().all(|mask| mask.bits.len() == 1));
}

// FontMatrix placements hinted at 16 px against recorded hashes: cff-matrix.otf's sheared Top DICT, halved and doubled
// FDs, cff-matrix-name.otf's 3 em offset, cff-cid.otf's FD em of 1 (1,000×). cff-matrix's glyphs 1, 2 and 4 and
// cff-matrix-name's recorded from daegun's own output once the CFF hinter followed Adobe's hint model.
#[test]
fn font_matrices_place_their_hinted_glyphs() {
    let cases: [(&str, &[u64]); 3] = [
        ("test-fixtures/cff-matrix.otf", &[
            0xaf3d2475d61e1dbc, 0x34617da90c023ada, 0xc39ac793e4908694, 0x552e46dd636987e4,
        ]),
        ("test-fixtures/cff-matrix-name.otf", &[0x397ec88d00ae8e66]),
        ("test-fixtures/cff-cid.otf", &[
            0x342954579a4849fd, 0x642d80c8028ed1fd, 0x5aba550d276bb88d, 0x618d68935c5103fd, 0xdfdbe9646df4f7f2,
            0xf75a51b036685fea, 0xa8daded7123a9bec, 0x48e3f30e3351c32a, 0x7785aa5dfd9d05fa, 0x73d1a5b3d4b92d20,
        ]),
    ];
    for (rel, want) in cases {
        let f = font(rel);
        for (gid, &want) in (1..).zip(want) {
            let got = f.hinted(GlyphId(gid), &Location::default(), 16.0, HintMode::Auto).map(|o| hash(&o));
            assert_eq!(got, Some(want), "{rel}: glyph {gid} isn't where it was recorded");
        }
    }
    // cff-cid's glyph 11 is a seac, which fails in a CID-keyed font: its charset holds CIDs, not the SIDs seac names.
    let seac = font("test-fixtures/cff-cid.otf").hinted(GlyphId(11), &Location::default(), 16.0, HintMode::Auto);
    assert!(seac.is_none(), "a CID-keyed font's seac drew");
}

// cff-matrix-name's FontMatrix moves its glyph 6,000 units across and 8,000 up, which a size scales by head's em of
// 1,000 as it does the glyph: hinted at 16 px, it lies within a pixel of its drawn outline at (6,110, 8,000).
#[test]
fn a_font_matrix_offset_moves_a_hinted_glyph_at_its_size() {
    let f = font("test-fixtures/cff-matrix-name.otf");
    let hinted = f.hinted(GlyphId(1), &Location::default(), 16.0, HintMode::Auto).expect("hinted");
    let drawn = [(6110.0, 8000.0), (6400.0, 8000.0), (6400.0, 8300.0), (6110.0, 8300.0)];
    let px: Vec<(f64, f64)> = hinted.points().map(|(p, _)| (f64::from(p.x), f64::from(p.y))).collect();
    assert_eq!(px.len(), drawn.len(), "{px:?}");
    for ((x, y), (dx, dy)) in px.into_iter().zip(drawn) {
        let (wx, wy) = (dx * 16.0 / 1000.0, dy * 16.0 / 1000.0);
        assert!((x - wx).abs() <= 1.0 && (y - wy).abs() <= 1.0, "({x}, {y}) is off ({wx}, {wy})");
    }
}
