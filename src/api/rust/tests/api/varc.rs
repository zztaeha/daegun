use crate::Hint;
use crate::located;
use daegun::{Font, GlyphId, Location};
use daegun::outline::{HintMode, OutlinePen};

fn font(name: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{}/varc/{name}", crate::FONTS)).expect("fixture"), 0).expect("loads")
}

// Each contour's points as drawn, every one of these glyphs' points on the curve.
#[derive(Default)]
struct Points(Vec<Vec<(f32, f32)>>);

impl OutlinePen for Points {
    fn move_to(&mut self, x: f32, y: f32) { self.0.push(vec![(x, y)]); }
    fn line_to(&mut self, x: f32, y: f32) { self.0.last_mut().unwrap().push((x, y)); }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) { self.0.last_mut().unwrap().push((x, y)); }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) { self.0.last_mut().unwrap().push((x, y)); }
    fn close(&mut self) {}
}

fn drawn(font: &Font, gid: u32, axes: &[(&str, f64)]) -> Vec<Vec<(f32, f32)>> {
    let mut pen = Points::default();
    font.outline(GlyphId(gid), &located(font, axes), &mut pen).expect("drawn");
    pen.0
}

// Glyph 2, 각, at wght 650 and 900: each jamo scaled and moved by its component, its stroke widened, and ㅏ moved
// right and the final ㄱ up, by half and then all of their variation.
#[test]
fn a_varc_glyph_draws_by_its_varied_transforms() {
    let font = font("varc-hangul.ttf");
    assert_eq!(drawn(&font, 2, &[("wght", 650.0)]), [
        vec![(50.0, 750.0), (470.0, 750.0), (470.0, 450.0), (350.0, 450.0), (350.0, 700.0), (50.0, 700.0)],
        vec![(590.0, 100.0), (590.0, 800.0), (710.0, 800.0), (710.0, 100.0)],
        vec![(710.0, 380.0), (710.0, 520.0), (860.0, 520.0), (860.0, 380.0)],
        vec![(100.0, 170.0), (625.0, 170.0), (625.0, 20.0), (475.0, 20.0), (475.0, 145.0), (100.0, 145.0)],
    ]);
    assert_eq!(drawn(&font, 2, &[("wght", 900.0)]), [
        vec![(50.0, 750.0), (490.0, 750.0), (490.0, 450.0), (350.0, 450.0), (350.0, 700.0), (50.0, 700.0)],
        vec![(630.0, 100.0), (630.0, 800.0), (770.0, 800.0), (770.0, 100.0)],
        vec![(770.0, 380.0), (770.0, 520.0), (920.0, 520.0), (920.0, 380.0)],
        vec![(100.0, 190.0), (650.0, 190.0), (650.0, 40.0), (475.0, 40.0), (475.0, 165.0), (100.0, 165.0)],
    ]);
}

// The hostile font's glyphs whose records fail, each drawn as its own outline, a triangle left of the origin drawn
// from phantom point 1, where its parent's transform puts it, whether the record was refused unread or failed read.
#[test]
fn a_record_that_fails_draws_the_glyph_s_own_outline() {
    let font = font("varc-hostile.ttf");
    let own = vec![(20.0, -5.0), (0.0, -5.0), (10.0, -30.0)];
    let square = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    // Inside itself: refused before its record is read, so moved 200 across.
    let moved: Vec<_> = own.iter().map(|&(x, y)| (x + 200.0, y)).collect();
    assert_eq!(drawn(&font, 7, &[]), [moved, square.iter().map(|&(x, y)| (x, y + 300.0)).collect()]);
    // A child whose coverage index is past the records: scaled by -1, y too as no y scale is given, and moved 900.
    let mirrored = vec![(880.0, 5.0), (900.0, 5.0), (890.0, 30.0)];
    assert_eq!(drawn(&font, 41, &[]), [mirrored, vec![(-200.0, 10.0), (-120.0, 10.0), (-160.0, 80.0)]]);
    assert_eq!(drawn(&font, 42, &[]), [own]);
    // An empty record draws nothing at all.
    assert!(drawn(&font, 40, &[]).is_empty());
}

// A chain of 20 nested glyphs draws the square at its end, turned 32.5° and moved by the 20 above it.
#[test]
fn a_chain_draws_to_its_end() {
    let font = font("varc-hostile.ttf");
    assert_eq!(drawn(&font, 10, &[]), [vec![(247.0, 68.0), (331.0, 122.0), (277.0, 206.0), (193.0, 152.0)]]);
}

// 66 components of a circle of 1,000 points draw all 66,000, past the 65,535 glyf's own points count to; inside
// another glyph, beside its square.
#[test]
fn a_glyph_past_65_535_points_draws_every_one() {
    let font = font("varc-hostile.ttf");
    let counts = |gid| drawn(&font, gid, &[]).iter().map(Vec::len).collect::<Vec<_>>();
    assert_eq!(counts(31), [1_000; 66]);
    assert_eq!(counts(32), [&[1_000; 66][..], &[4]].concat());
}

// No glyph VARC draws is hinted, whatever the mode.
#[test]
fn a_varc_glyph_is_never_hinted() {
    let font = font("varc-hangul.ttf");
    for mode in [HintMode::Font, HintMode::Auto, HintMode::AutoForce] {
        assert!(font.hinted(GlyphId(2), &Location::default(), 24.0, mode).is_none(), "{mode:?}");
    }
}

// Glyph 3 draws glyph 2, a composite of two squares, at wght 1.0: each square widened by its own deltas, the second
// moved 40 right by the composite's.
#[test]
fn a_composite_leaf_draws_each_glyph_with_its_own_deltas() {
    let font = font("varc-composite-leaf.ttf");
    assert_eq!(drawn(&font, 3, &[]), [
        vec![(0.0, 0.0), (160.0, 0.0), (160.0, 120.0), (0.0, 120.0)],
        vec![(190.0, 0.0), (350.0, 0.0), (350.0, 120.0), (190.0, 120.0)],
    ]);
}

// A CFF2 font's glyph past its CharStrings is VARC's to draw: glyph 3, glyph 1's triangle scaled by 2 and moved 400
// across.
#[test]
fn varc_draws_cff2_glyphs_past_the_charstrings() {
    let font = font("varc-short-cff2.otf");
    assert_eq!(font.glyph_count(), 4);
    assert_eq!(drawn(&font, 1, &[]), [vec![(100.0, 0.0), (200.0, 0.0), (100.0, 100.0)]]);
    assert_eq!(drawn(&font, 3, &[]), [vec![(600.0, 0.0), (800.0, 0.0), (600.0, 200.0)]]);
}
