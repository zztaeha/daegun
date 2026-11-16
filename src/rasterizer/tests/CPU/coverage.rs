use std::format;
use std::vec::Vec;

use super::truth::{self, Fine};
use crate::daecore::daetype::outline::{FillRule, OutlinePen, Path};
use crate::outline::OutlineOptions;
use crate::raster::{Bitmap, RasterOptions, Rasterizer, SubpixelLayout};
use crate::rasterizer::coverage::TOLERANCE;
use crate::{Font, GlyphId, Location, Tag};

pub(super) const FONTS: &[&str] = &[
    "inter/InterVariable.ttf",
    "source-serif/SourceSerif4Variable-Roman.otf",
    "eb-garamond/EBGaramond.ttf",
    "source-han-sans/SourceHanSansJP-VF.otf",
    "stix-two-math/STIX2Math.otf",
    "noto-devanagari/NotoSansDevanagari.ttf",
    "noto-khmer/NotoSansKhmer.ttf",
    "noto-myanmar/NotoSansMyanmar.ttf",
    "scheherazade-new/ScheherazadeNew-Regular.ttf",
];

const SIZES: &[f32] = &[8.0, 12.0, 16.0, 24.0, 48.0, 128.0];
const OFFSETS: &[(f32, f32)] = &[(0.0, 0.0), (0.25, 0.5), (0.5, 0.75), (0.75, 0.25)];

pub(super) fn font(name: &str) -> Option<Font> {
    let bytes = std::fs::read(format!("{}/assets/test-fonts/{name}", env!("CARGO_MANIFEST_DIR"))).ok()?;
    Some(Font::from_bytes(&bytes).expect("a test font opens"))
}

// Glyphs spread over the font, `n` of them.
pub(super) fn sample(font: &Font, n: u32) -> impl Iterator<Item = GlyphId> {
    let count = font.glyph_count();
    (0..n).map(move |i| GlyphId(1 + i * (count - 1) / n))
}

fn as_f64(contours: impl Iterator<Item = impl AsRef<[[f32; 2]]>>) -> Vec<Vec<[f64; 2]>> {
    contours.map(|c| c.as_ref().iter().map(|p| p.map(f64::from)).collect()).collect()
}

// The worst gap between an image and coverage, in levels of 255.
fn gap(image: &Bitmap, truth: &[f64]) -> f64 {
    image.pixels.iter().zip(truth).map(|(&v, &t)| (f64::from(v) - 255.0 * t).abs()).fold(0.0, f64::max)
}

// Within rounding of the truth: half a level, and the fixed point's own, a 2^-16 step for each piece in a pixel,
// kept under a thirty-second of one.
pub(super) const ROUNDING: f64 = 0.5 + 1.0 / 32.0;

// A chord strays at most TOLERANCE from its arc, so the sliver between them in a pixel is at most that times the
// pixel's diagonal.
const CHORDS: f64 = 255.0 * core::f64::consts::SQRT_2 * TOLERANCE + ROUNDING;

fn glyph(r: &mut Rasterizer, font: &Font, g: GlyphId, at: &Location, px: f32, o: (f32, f32)) -> Option<Bitmap> {
    let options = RasterOptions::DEFAULT.with_offset(o.0, o.1);
    r.glyph(font, g, at, px, &options).ok().map(|image| image.to_bitmap())
}

// The image held to the truth over the very polygons it filled.
fn own_gap(r: &mut Rasterizer, image: &Bitmap, rule: FillRule) -> f64 {
    let truth = truth::coverage(
        &as_f64(r.edges.contours()),
        rule,
        image.left,
        image.top,
        image.width as usize,
        image.height as usize,
    );
    gap(image, &truth)
}

#[test]
fn coverage_is_exact_over_the_polygons_it_fills() {
    let mut r = Rasterizer::new();
    for name in FONTS {
        let Some(font) = font(name) else { continue };
        for g in sample(&font, 12) {
            for &px in SIZES {
                for &o in OFFSETS {
                    let Some(image) = glyph(&mut r, &font, g, &Location::default(), px, o) else { continue };
                    let worst = own_gap(&mut r, &image, FillRule::NonZero);
                    assert!(worst <= ROUNDING, "{name} glyph {} at {px} px, offset {o:?}: {worst} levels", g.0);
                }
            }
        }
    }
}

// Inter's contours overlap wherever strokes join, and more so as it gets heavier.
#[test]
fn overlapping_contours_are_exact() {
    let Some(font) = font("inter/InterVariable.ttf") else { return };
    let mut r = Rasterizer::new();
    for weight in [100.0, 400.0, 900.0] {
        let at = font.location(&[(Tag::new(b"wght"), weight)]);
        for c in "AaBbegkKxRQ&@%$ÆŒß8".chars() {
            let Some(g) = font.glyph_id(c) else { continue };
            for &px in SIZES {
                for &o in OFFSETS {
                    let Some(image) = glyph(&mut r, &font, g, &at, px, o) else { continue };
                    let worst = own_gap(&mut r, &image, FillRule::NonZero);
                    assert!(worst <= ROUNDING, "{c} at wght {weight}, {px} px, offset {o:?}: {worst} levels");
                }
            }
        }
    }
}

// With the overlaps removed once, in font units, every size still fills as the font's own curves do.
#[test]
fn removed_overlaps_fill_as_the_curves_do() {
    use crate::rasterizer::Shape;
    let Some(font) = font("inter/InterVariable.ttf") else { return };
    let mut r = Rasterizer::new();
    let (mut worst, mut removed, mut unsure): (f64, usize, usize) = (0.0, 0, 0);
    for weight in [100.0, 400.0, 900.0] {
        let at = font.location(&[(Tag::new(b"wght"), weight)]);
        for c in "AaBbegkKxRQ&@%$ÆŒß8".chars() {
            let Some(g) = font.glyph_id(c) else { continue };
            for &px in SIZES {
                let Some(image) = glyph(&mut r, &font, g, &at, px, (0.25, 0.5)) else { continue };
                let mut fine = Fine::default();
                let options = OutlineOptions::DEFAULT;
                font.prepared_outline(g, &at, px, &options, &mut fine).unwrap();
                let fine: Vec<Vec<[f64; 2]>> =
                    fine.contours.iter().map(|c| c.iter().map(|p| [p[0] + 0.25, p[1] + 0.5]).collect()).collect();
                let (w, h) = (image.width as usize, image.height as usize);
                let truth = truth::coverage(&fine, FillRule::NonZero, image.left, image.top, w, h);
                let gap = gap(&image, &truth);
                assert!(gap <= CHORDS, "{c} at wght {weight}, {px} px: {gap} levels from the curves");
                worst = worst.max(gap);
            }
        }
    }
    let shapes: Vec<Shape> = r.learned.iter().flat_map(|l| l.glyphs.values().map(|g| g.1)).collect();
    for shape in &shapes {
        match shape {
            Shape::Removed => removed += 1,
            Shape::Unsure => unsure += 1,
            Shape::Plain => {}
        }
    }
    std::println!("worst {worst:.2} levels; {removed} removed, {unsure} unsure of {}", shapes.len());
}

fn polygon(path: &mut Path, points: &[(f32, f32)]) {
    path.move_to(points[0].0, points[0].1);
    for &(x, y) in &points[1..] {
        path.line_to(x, y);
    }
    path.close();
}

fn shapes() -> Vec<(&'static str, Path)> {
    let mut out = Vec::new();
    let mut p = Path::new();
    polygon(&mut p, &[(0.3, 0.2), (6.7, 0.2), (6.7, 5.1), (0.3, 5.1)]);
    polygon(&mut p, &[(3.2, 2.4), (9.9, 2.4), (9.9, 7.7), (3.2, 7.7)]);
    out.push(("two squares overlapping", p));
    let mut p = Path::new();
    polygon(&mut p, &[(0.3, 0.2), (6.7, 0.2), (6.7, 5.1), (0.3, 5.1)]);
    polygon(&mut p, &[(3.2, 2.4), (3.2, 7.7), (9.9, 7.7), (9.9, 2.4)]);
    out.push(("two squares wound against each other", p));
    let mut p = Path::new();
    polygon(&mut p, &[(1.0, 1.0), (3.0, 1.0), (3.0, 5.5), (1.0, 5.5)]);
    polygon(&mut p, &[(1.5, 5.5), (1.5, 9.0), (3.5, 9.0), (3.5, 5.5)]);
    out.push(("two squares wound against each other meeting along a row", p));
    let mut p = Path::new();
    polygon(&mut p, &[(5.0, 0.1), (6.2, 9.7), (0.3, 3.5), (9.8, 3.4), (3.6, 9.9)]);
    out.push(("a star crossing itself", p));
    let mut p = Path::new();
    for _ in 0..3 {
        polygon(&mut p, &[(1.3, 1.1), (7.6, 2.2), (4.4, 8.9)]);
    }
    out.push(("one triangle three times", p));
    let mut p = Path::new();
    polygon(&mut p, &[(1.0, 1.0), (8.0, 1.0), (8.0, 1.3), (1.0, 1.3)]);
    polygon(&mut p, &[(1.0, 1.1), (8.0, 1.15), (8.0, 1.4), (1.0, 1.45)]);
    polygon(&mut p, &[(2.0, 0.5), (2.2, 0.5), (2.2, 2.0), (2.0, 2.0)]);
    out.push(("slivers overlapping within a pixel", p));
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    polygon(&mut p, &[(2.5, 2.5), (2.5, 7.5), (7.5, 7.5), (7.5, 2.5)]);
    polygon(&mut p, &[(4.1, 4.1), (5.9, 4.1), (5.9, 5.9), (4.1, 5.9)]);
    out.push(("a square in a hole in a square", p));
    let mut p = Path::new();
    polygon(&mut p, &[(0.2, 0.2), (9.8, 0.2), (9.8, 9.8), (0.2, 9.8)]);
    polygon(&mut p, &[(0.7, 0.7), (9.3, 0.7), (9.3, 9.3), (0.7, 9.3)]);
    out.push(("a square in one wound alike, sharing its pixels", p));
    let mut p = Path::new();
    p.move_to(1.0, 1.0);
    p.quad_to(9.0, 1.0, 9.0, 9.0);
    p.curve_to(5.0, -3.0, 1.0, 13.0, 1.0, 1.0);
    p.close();
    out.push(("curves looping over themselves", p));
    out
}

#[test]
fn shapes_are_exact_under_both_rules() {
    let mut r = Rasterizer::new();
    for (name, path) in shapes() {
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            for scale in [1.0, 3.0, 7.5] {
                for &(dx, dy) in OFFSETS {
                    let m = [scale, 0.0, 0.0, scale, f64::from(dx), f64::from(dy)];
                    let image = r.path(&path, rule, Some(&m)).expect("the shape fills").to_bitmap();
                    let worst = own_gap(&mut r, &image, rule);
                    assert!(worst <= ROUNDING, "{name}, {rule:?}, ×{scale} + {dx},{dy}: {worst} levels");
                }
            }
        }
    }
}

// Each shape cut to frames that take in part of it in f64, every side cutting it somewhere, covers each frame as the
// whole shape does, under both rules, straight and turned.
#[test]
fn a_path_cut_to_its_frame_covers_the_frame_as_the_whole_path() {
    use crate::rasterizer::Frame;
    let mut r = Rasterizer::new();
    let frames = [(-2, 14, 9, 9), (4, 25, 12, 15), (11, 31, 30, 8), (8, 18, 6, 6)];
    for (name, path) in shapes() {
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            for m in [[3.0, 0.0, 0.0, 3.0, 0.3, 0.6], [2.6, 1.1, -0.9, 2.4, 6.2, 1.7]] {
                for (left, top, width, height) in frames {
                    r.edges.begin(m);
                    r.edges.path(&path);
                    r.edges.finish();
                    let whole = as_f64(r.edges.contours());
                    let m = m.map(f64::from);
                    let local = [m[0], m[1], m[2], m[3], m[4] - f64::from(left), m[5] - f64::from(top)];
                    r.edges.begin_cut(local, [0.0, -(height as f64), width as f64, 0.0]);
                    r.edges.path(&path);
                    r.edges.finish();
                    let (mut out, mut all) = (vec![0u8; width * height], u64::MAX);
                    let frame = Frame { left: 0, top: 0, width, height };
                    assert!(r.coverage.draw(&r.edges, rule, frame, Some((usize::MAX, &mut all)), &mut out, width));
                    let truth = truth::coverage(&whole, rule, left, top, width, height);
                    let gaps = out.iter().zip(&truth).map(|(&v, &t)| (f64::from(v) - 255.0 * t).abs());
                    let worst = gaps.fold(0.0, f64::max);
                    let at = format!("{left},{top}, {width} by {height}");
                    assert!(worst <= ROUNDING, "{name}, {rule:?}, {m:?} at {at}: {worst}");
                }
            }
        }
    }
}

// Contours that neither cross nor nest alike, summed straight: chords through pixel corners, along pixel edges, and
// short ones crossing a few pixel edges each way.
#[test]
fn summed_polygons_are_exact_through_pixel_corners() {
    let mut shapes = Vec::new();
    let mut p = Path::new();
    polygon(&mut p, &[(5.0, 0.0), (10.0, 5.0), (5.0, 10.0), (0.0, 5.0)]);
    shapes.push(("a diamond", p));
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)]);
    polygon(&mut p, &[(2.0, 2.0), (2.0, 6.0), (6.0, 6.0), (6.0, 2.0)]);
    shapes.push(("a square with a hole", p));
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (9.0, 3.0), (3.0, 9.0)]);
    shapes.push(("a triangle", p));
    let mut p = Path::new();
    let circle: Vec<(f32, f32)> = (0..48)
        .map(|i| {
            let a = f64::from(i) * core::f64::consts::TAU / 48.0;
            ((5.0 + 4.0 * a.cos()) as f32, (5.0 + 4.0 * a.sin()) as f32)
        })
        .collect();
    polygon(&mut p, &circle);
    shapes.push(("a circle of short chords", p));
    let mut r = Rasterizer::new();
    for (name, path) in shapes {
        for scale in [0.5, 1.0, 2.0, 3.0] {
            for &(dx, dy) in OFFSETS {
                r.edges.begin([scale, 0.0, 0.0, scale, dx, dy]);
                r.edges.path(&path);
                let drawn = r.draw(FillRule::NonZero, &SubpixelLayout::grayscale(), false).expect("the shape fills");
                let image = r.image(&drawn).to_bitmap();
                let worst = own_gap(&mut r, &image, FillRule::NonZero);
                assert!(worst <= ROUNDING, "{name}, ×{scale} + {dx},{dy}: {worst} levels");
            }
        }
    }
}

// A chord ending on a corner of the bottom edge crosses that edge last, never stepping into the row past the image:
// drawn first, so the grid is no larger than the image needs. The fuzzer's sliver, as the font drew it.
#[test]
fn a_chord_ending_on_a_bottom_corner_stays_in_the_grid() {
    let mut p = Path::new();
    polygon(&mut p, &[(0.0, 0.0), (0.900_000_1, 0.0), (0.887_816_6, 0.7155), (-0.462_183_45, 0.7155)]);
    let mut r = Rasterizer::new();
    let image = r.path(&p, FillRule::NonZero, None).expect("the sliver fills").to_bitmap();
    let worst = own_gap(&mut r, &image, FillRule::NonZero);
    assert!(worst <= ROUNDING, "{worst} levels");
}

// A path from outside is filled exactly whether its contours overlap or not: summed straight where they lie apart,
// resolved where they don't.
#[test]
fn paths_fill_exactly_whether_their_contours_overlap() {
    let mut r = Rasterizer::new();
    let mut check = |font: &Font, g: GlyphId, at: &Location, px: f32, what: &str| {
        let mut path = Path::new();
        font.prepared_outline(g, at, px, &OutlineOptions::DEFAULT, &mut path).unwrap();
        for rule in [FillRule::NonZero, FillRule::EvenOdd] {
            let Ok(image) = r.path(&path, rule, Some(&[1.0, 0.0, 0.0, 1.0, 0.25, 0.5])) else { continue };
            let image = image.to_bitmap();
            let worst = own_gap(&mut r, &image, rule);
            assert!(worst <= ROUNDING, "{what} glyph {} at {px} px, {rule:?}: {worst} levels", g.0);
        }
    };
    for name in FONTS {
        let Some(font) = font(name) else { continue };
        for g in sample(&font, 12) {
            for px in [10.0, 40.0] {
                check(&font, g, &Location::default(), px, name);
            }
        }
    }
    let Some(font) = font("inter/InterVariable.ttf") else { return };
    for weight in [100.0, 400.0, 900.0] {
        let at = font.location(&[(Tag::new(b"wght"), weight)]);
        for c in "AaBbegkKxRQ&@%$ÆŒß8".chars() {
            let Some(g) = font.glyph_id(c) else { continue };
            for px in [10.0, 40.0] {
                check(&font, g, &at, px, "Inter");
            }
        }
    }
}

// Against the curves themselves, chords cut far finer: what the flattening's tolerance costs.
#[test]
fn chords_stay_near_the_curves() {
    let mut r = Rasterizer::new();
    let mut worst: f64 = 0.0;
    for name in FONTS {
        let Some(font) = font(name) else { continue };
        for g in sample(&font, 12) {
            for &px in SIZES {
                let Some(image) = glyph(&mut r, &font, g, &Location::default(), px, (0.0, 0.0)) else { continue };
                let mut fine = Fine::default();
                font.prepared_outline(g, &Location::default(), px, &OutlineOptions::DEFAULT, &mut fine).unwrap();
                let (w, h) = (image.width as usize, image.height as usize);
                let truth = truth::coverage(&fine.contours, FillRule::NonZero, image.left, image.top, w, h);
                worst = worst.max(gap(&image, &truth));
            }
        }
    }
    std::println!("worst gap from the curves: {worst:.2} levels");
    assert!(worst <= CHORDS, "{worst} levels from the curves, over {CHORDS}");
}

// Noto Color Emoji's glyph 1778 at 160 px: a hole's edge crosses its outline's twice in a row of pixels, the second
// below the band split at the first, where the gap read as a crossing too near to count would hide it, 20 levels off.
#[test]
fn a_crossing_below_one_that_split_its_band_is_found() {
    let mut p = Path::new();
    polygon(&mut p, &[
        (80.3, 75.6), (80.8469, 75.0531), (81.2375, 74.35), (81.3547, 73.8812), (81.3938, 73.4125), (81.2766, 72.6703),
        (80.925, 72.0062), (81.2375, 71.225), (81.3938, 71.225), (81.7844, 71.9281), (82.0188, 72.7875),
        (81.8527, 73.6176), (81.5109, 74.3891), (80.9934, 75.102), (80.3, 75.7562),
    ]);
    polygon(&mut p, &[
        (81.2375, 74.6625), (81.5847, 74.1069), (81.7931, 73.4819), (81.8625, 72.7875), (81.7453, 72.0453),
        (81.3938, 71.3812), (80.925, 71.85), (81.2722, 72.3014), (81.4806, 72.8222), (81.55, 73.4125),
        (81.5109, 73.9203), (81.3938, 74.5062), (81.2766, 74.5453),
    ]);
    let mut r = Rasterizer::new();
    let image = r.path(&p, FillRule::NonZero, None).expect("the shape fills").to_bitmap();
    let worst = own_gap(&mut r, &image, FillRule::NonZero);
    assert!(worst <= ROUNDING, "{worst} levels");
}

// Noto Color Emoji's glyph 1867 at 160 px: a hole's corner lies on its outline's edge just below where a band starts,
// at their meeting. Ordered that near the top, rounding can put the two backwards for the whole band, 1.2 levels off.
#[test]
fn a_band_s_chains_are_ordered_clear_of_its_top() {
    let mut p = Path::new();
    polygon(&mut p, &[
        (119.3625, 49.50625), (119.32344, 49.6625), (119.20625, 49.81875), (119.05, 49.935936), (118.89375, 49.975),
        (118.7375, 50.053123), (118.58125, 49.975), (118.464066, 49.779686), (118.425, 49.50625),
        (118.47709, 49.24583), (118.63334, 49.08958), (118.89375, 49.0375), (119.20625, 49.0375),
        (119.32344, 49.23281),
    ]);
    polygon(&mut p, &[
        (119.20625, 49.50625), (119.17153, 49.24583), (119.06737, 49.08958), (118.89375, 49.0375),
        (118.7375, 49.07656), (118.58125, 49.19375), (118.58125, 49.50625), (118.58125, 49.740623),
        (118.58125, 49.81875), (118.7375, 49.935936), (118.89375, 49.975), (119.06737, 49.922916),
        (119.17153, 49.766666),
    ]);
    let mut r = Rasterizer::new();
    let image = r.path(&p, FillRule::NonZero, None).expect("the shape fills").to_bitmap();
    let worst = own_gap(&mut r, &image, FillRule::NonZero);
    assert!(worst <= ROUNDING, "{worst} levels");
}

// One contour's chains cross along a level segment, all at one y, as in Noto Color Emoji's glyph 1937 at 160 px.
// Read only arriving there, their gap would keep its sign, and the row sum the two lobes against each other.
#[test]
fn a_level_segment_carrying_one_chain_across_another_is_a_crossing() {
    let mut p = Path::new();
    polygon(&mut p, &[(1.0, 2.9), (1.2, 2.45), (1.8, 2.45), (1.6, 2.1)]);
    let mut r = Rasterizer::new();
    let image = r.path(&p, FillRule::EvenOdd, None).expect("the shape fills").to_bitmap();
    let worst = own_gap(&mut r, &image, FillRule::EvenOdd);
    assert!(worst <= ROUNDING, "{worst} levels");
}

// Two pairs of squares overlap near the top and the bottom, and a zigzag of 400 points runs down between them: the
// rows between are left out of the sweep, which starts again above the bottom pair, and every row is exact.
#[test]
fn a_sweep_started_again_past_a_gap_is_exact() {
    let mut p = Path::new();
    for y in [1.2, 96.3] {
        polygon(&mut p, &[(1.1, y), (6.4, y), (6.4, y + 2.5), (1.1, y + 2.5)]);
        polygon(&mut p, &[(3.3, y + 0.6), (9.2, y + 0.6), (9.2, y + 3.1), (3.3, y + 3.1)]);
    }
    let zig: Vec<_> = (0..200).map(|k| (14.0 + (k % 2) as f32 * 3.0, 0.5 + k as f32 * 0.5)).collect();
    polygon(&mut p, &zig.iter().copied().chain(zig.iter().rev().map(|&(x, y)| (x + 4.0, y))).collect::<Vec<_>>());
    let mut r = Rasterizer::new();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let image = r.path(&p, rule, None).expect("the path fills").to_bitmap();
        let worst = own_gap(&mut r, &image, rule);
        assert!(worst <= ROUNDING, "{rule:?}: {worst} levels");
    }
}

// `path` filled exactly in a frame of its own under `rule` within `allowance`, and the work it paid; none past it.
fn filled(path: &Path, rule: FillRule, allowance: u64) -> Option<(Vec<u8>, u64)> {
    use crate::rasterizer::Frame;
    let mut r = Rasterizer::new();
    r.edges.begin([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    r.edges.path(path);
    r.edges.finish();
    let [x0, y0, x1, y1] = r.edges.bounds().expect("the path has points");
    let (left, top) = (x0.floor() as i32, y1.ceil() as i32);
    let (width, height) = ((x1.ceil() as i32 - left) as usize, (top - y0.floor() as i32) as usize);
    let (mut out, mut rest) = (vec![0u8; width * height], allowance);
    let frame = Frame { left, top, width, height };
    let filled = r.coverage.draw(&r.edges, rule, frame, Some((usize::MAX, &mut rest)), &mut out, width);
    filled.then(|| (out, allowance - rest))
}

// 300 thin squares wound alike, each 0.01 px right of the last, put 600 pieces in each row they share: every row is
// exact, where summing them is 127 levels off along the squares' left edge.
#[test]
fn a_row_of_hundreds_of_overlapping_pieces_is_exact() {
    let mut p = Path::new();
    for k in 0..300 {
        let x = 1.0 + 0.01 * k as f32;
        polygon(&mut p, &[(x, 0.25), (x + 2.0, 0.25), (x + 2.0, 1.75), (x, 1.75)]);
    }
    let mut r = Rasterizer::new();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let image = r.path(&p, rule, None).expect("the squares fill").to_bitmap();
        let worst = own_gap(&mut r, &image, rule);
        assert!(worst <= ROUNDING, "{rule:?}: {worst} levels");
    }
}

// Disjoint triangles are told nested by one sweep over their chains, paying no swaps: four times as many pay under
// six times as much, where testing each pair would take sixteen.
#[test]
fn contours_are_told_nested_in_one_sweep() {
    let work = |n: usize| {
        let mut path = Path::new();
        for i in 0..n {
            let (x, y) = ((i % 40) as f32 * 2.0 + 0.3, (i / 40) as f32 * 2.0 + 0.3);
            polygon(&mut path, &[(x, y), (x + 0.5, y), (x, y + 0.5)]);
        }
        filled(&path, FillRule::NonZero, u64::MAX).expect("the triangles fill").1
    };
    let (few, many) = (work(1_000), work(4_000));
    assert!(many < 6 * few, "{few} then {many}");
}

// 64 zigzags braided in one row cross each other thousands of times: each crossing is a swap of neighbors, and the
// row is exact under both rules.
#[test]
fn braided_zigzags_are_exact() {
    let mut path = Path::new();
    for j in 0..64 {
        let x0 = j as f32 * 0.03;
        let zig = (0..=8).map(|t| (x0 + if (t + j) % 2 == 0 { 0.0 } else { 6.0 }, t as f32 / 9.0));
        polygon(&mut path, &zig.chain([(x0 + 7.0, 0.99)]).collect::<Vec<_>>());
    }
    let mut r = Rasterizer::new();
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let image = r.path(&path, rule, None).expect("the zigzags fill").to_bitmap();
        let worst = own_gap(&mut r, &image, rule);
        assert!(worst <= ROUNDING, "{rule:?}: {worst} levels");
    }
}

// 300 thin strips each crossing every other make some 45,000 swaps, each paid as it's made: an allowance a step short
// of what the fill pays refuses it, and one holding it fills alike.
#[test]
fn a_fill_s_swaps_are_paid_as_they_re_made() {
    let mut path = Path::new();
    for i in 0..300 {
        let y = 0.03 * i as f32;
        polygon(&mut path, &[(0.0, y), (100.0, 10.0 - y), (100.0, 10.05 - y), (0.0, y + 0.05)]);
    }
    let (out, paid) = filled(&path, FillRule::NonZero, u64::MAX).expect("the strips fill");
    assert!(filled(&path, FillRule::NonZero, paid - 1).is_none(), "the strips filled short");
    assert_eq!(filled(&path, FillRule::NonZero, paid).map(|f| f.0), Some(out));
}

// 300 copies of one square put 300 passages in each pixel along its edges, far past the room the walk lists them in:
// every row is resolved instead, so even-odd fills nothing and an odd count or nonzero fills the square.
#[test]
fn passages_past_their_room_are_resolved_whole() {
    let copies = |n: usize| {
        let mut p = Path::new();
        (0..n).for_each(|_| polygon(&mut p, &[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]));
        p
    };
    let mut r = Rasterizer::new();
    let cases = [(300, FillRule::EvenOdd, 0), (301, FillRule::EvenOdd, 100), (300, FillRule::NonZero, 100)];
    for (n, rule, filled) in cases {
        let image = r.path(&copies(n), rule, None).expect("the squares fill").to_bitmap();
        assert!(r.coverage.tags.over, "{n} {rule:?}: the passages fit");
        assert_eq!(image.pixels.iter().filter(|&&v| v == 255).count(), filled, "{n} {rule:?}");
        assert!(own_gap(&mut r, &image, rule) <= ROUNDING, "{n} {rule:?}");
    }
}
