// Each kind of draw counts what it would hold before it allocates: past MAX_DRAW_BYTES it's refused, its buffers left
// as they were, and a smaller one draws.
use super::coverage::font;
use crate::color::{Blend, DisplayList, Op, Paint, Rgba};
use crate::daecore::daetype::outline::{FillRule, OutlinePen, Path};
use crate::raster::{Field, Rasterizer, MAX_DRAW_BYTES};
use crate::rasterizer::fits;
use crate::{ErrorKind, Location};
use alloc::vec::Vec;

fn square(side: f32) -> Path {
    let mut p = Path::new();
    p.move_to(0.0, 0.0);
    for (x, y) in [(side, 0.0), (side, side), (0.0, side)] {
        p.line_to(x, y);
    }
    p.close();
    p
}

// Layers nested `depth` deep over a square `side` pixels across.
fn nested(side: f32, depth: usize) -> DisplayList {
    let mut l = DisplayList::default();
    for _ in 0..depth {
        l.push(Op::PushLayer { opacity: 1.0, blend: Blend::SrcOver });
    }
    let path = l.push_path(square(side).view());
    let paint = Paint::Solid(Rgba::opaque(200, 40, 0));
    l.push(Op::Fill { path, paint, rule: FillRule::NonZero, transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] });
    for _ in 0..depth {
        l.push(Op::PopLayer);
    }
    l
}

// A circle of 1.2 million points, past MAX_FLATTEN_POINTS, draws: its points are held within the bytes a draw
// holds. A point not finite is a bad argument.
#[test]
fn a_path_past_a_million_points_draws_and_a_point_not_finite_is_a_bad_argument() {
    let mut r = Rasterizer::new();
    let mut circle = Path::new();
    for i in 0..1_200_000 {
        let (s, c) = (f64::from(i) * core::f64::consts::TAU / 1.2e6).sin_cos();
        let (x, y) = ((60.0 + 50.0 * c) as f32, (60.0 + 50.0 * s) as f32);
        if i == 0 { circle.move_to(x, y) } else { circle.line_to(x, y) }
    }
    circle.close();
    let kind = |r: &mut Rasterizer, p: &Path| r.path(p, FillRule::NonZero, None).map(|_| ()).map_err(|e| e.kind());
    assert_eq!(kind(&mut r, &circle), Ok(()));
    let mut stray = Path::new();
    stray.move_to(0.0, 0.0);
    stray.line_to(f32::NAN, 0.0);
    stray.line_to(10.0, 10.0);
    stray.close();
    assert_eq!(kind(&mut r, &stray), Err(ErrorKind::InvalidArgument));
    assert!(kind(&mut r, &square(10.0)).is_ok());
}

// A glyph on one vertical line whose heights zigzag inward over 4,000 points meets itself too often for its overlaps to
// be removed, and its chords lie along each other, each end on some 2,000 others, so its field is refused.
#[test]
fn a_tangled_glyph_s_field_is_a_limit() {
    let words = |w: &[i16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let ys: Vec<i16> = (0..4_000).map(|k| if k % 2 == 0 { k / 2 } else { 2_000 - k / 2 }).collect();
    let deltas: Vec<i16> = ys.iter().scan(0, |at, &y| Some(y - core::mem::replace(at, y))).collect();
    let glyph = [words(&[1, 0, 0, 0, 2_000, 3_999, 0]), vec![0x11; 4_000], words(&deltas)].concat();
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = 1;
    let loca = [0u32, 0, glyph.len() as u32].map(u32::to_be_bytes).concat();
    let tables: alloc::collections::BTreeMap<crate::Tag, Vec<u8>> = [
        (*b"head", head), (*b"maxp", vec![0, 0, 0x50, 0, 0, 2]), (*b"loca", loca), (*b"glyf", glyph),
    ]
    .into_iter()
    .map(|(tag, t)| (crate::Tag::new(&tag), t))
    .collect();
    let font = crate::Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the font parses");
    let sdf = Field::Sdf { spread: 4.0 };
    let mut r = Rasterizer::new();
    let field = r.distance_field(&font, crate::GlyphId(1), &Location::default(), 20.0, sdf);
    assert_eq!(field.map(|_| ()).map_err(|e| e.kind()), Err(ErrorKind::LimitExceeded));
}

// 300 curves under a scale of a million reach past any image a rasterizer lends: each is found too wide before it's
// flattened, and the draw refused as too large rather than flattened to millions of points.
#[test]
fn curves_too_wide_to_lend_are_refused_before_they_flatten() {
    let mut wild = Path::new();
    wild.move_to(0.0, 0.0);
    for i in 0..300 {
        wild.quad_to(i as f32 + 0.5, 1000.0, i as f32 + 1.0, 0.0);
    }
    wild.close();
    let m = [1.0e6, 0.0, 0.0, 1.0e6, 0.0, 0.0];
    let mut r = Rasterizer::new();
    let kind = r.path(&wild, FillRule::NonZero, Some(&m)).map(|_| ()).map_err(|e| e.kind());
    assert_eq!(kind, Err(ErrorKind::LimitExceeded));
    assert!(r.edges.points() <= 600, "{} points flattened", r.edges.points());
}

// A star of 1,000 points crosses itself half a million times, each crossing swapped where it falls: within what a
// fill may take, it draws, full at its middle.
#[test]
fn a_star_of_a_thousand_points_draws() {
    let at = |i: usize| {
        let (s, c) = ((i * 499 % 1_000) as f32 * core::f32::consts::TAU / 1_000.0).sin_cos();
        (500.0 + 450.0 * c, 500.0 + 450.0 * s)
    };
    let mut p = Path::new();
    p.move_to(at(0).0, at(0).1);
    (1..1_000).for_each(|i| p.line_to(at(i).0, at(i).1));
    p.close();
    let image = Rasterizer::new().path(&p, FillRule::NonZero, None).expect("the star draws").to_bitmap();
    let (w, h) = (image.width as usize, image.height as usize);
    assert_eq!((image.pixels[h / 2 * w + w / 2], image.pixels[0]), (255, 0));
}

// A zigzag of n points across a 1,000 pixel square crosses 1,000 cells a chord, more than its points and pixels
// allow: refused before it's summed, and with a tenth as many points it draws.
#[test]
fn a_path_whose_cells_pass_its_allowance_is_refused_before_it_is_summed() {
    use crate::rasterizer::coverage::{CELL, FILL_BASE, FILL_PIXEL, FILL_POINT};
    let zigzag = |n: u64| {
        let mut p = Path::new();
        p.move_to(0.0, 0.0);
        (1..n).for_each(|i| p.line_to(if i % 2 == 1 { 1_000.0 } else { 0.0 }, i as f32 * 1_000.0 / n as f32));
        p.close();
        p
    };
    let n = (FILL_BASE + FILL_PIXEL * 1_002 * 1_002) / (CELL * 1_000 - FILL_POINT) + 1;
    let mut r = Rasterizer::new();
    let kind = r.path(&zigzag(n), FillRule::NonZero, None).map(drop).map_err(|e| e.kind());
    assert_eq!(kind, Err(ErrorKind::LimitExceeded));
    assert!(r.path(&zigzag(n / 10), FillRule::NonZero, None).is_ok());
}

#[test]
fn the_budget_is_counted_to_the_byte() {
    assert!(fits(MAX_DRAW_BYTES as u64).is_ok());
    assert_eq!(fits(MAX_DRAW_BYTES as u64 + 1).map_err(|e| e.kind()), Err(ErrorKind::LimitExceeded));
}

#[test]
fn a_draw_past_the_budget_is_refused_before_it_allocates() {
    let mut r = Rasterizer::new();
    // 6,002 sums and marks a row by 6,000 rows at 9 bytes, and the image: 360 MB.
    let refused = r.path(&square(6000.0), FillRule::NonZero, None).map(|_| ()).map_err(|e| e.kind());
    assert_eq!(refused, Err(ErrorKind::LimitExceeded));
    assert_eq!(r.pixels.capacity(), 0, "a refused draw grew the image");
    assert!(r.path(&square(300.0), FillRule::NonZero, None).is_ok());

    // An MSDF texel holds three channels' nearest segments and values: 5 million of them pass 500 MB.
    let inter = font("inter/InterVariable.ttf").expect("Inter is present");
    let h = inter.glyph_id('H').expect("Inter maps H");
    let msdf = Field::Msdf { spread: 4.0 };
    let refused = r.distance_field(&inter, h, &Location::default(), 3000.0, msdf).map(|_| ()).map_err(|e| e.kind());
    assert_eq!(refused, Err(ErrorKind::LimitExceeded));
    assert!(r.distance_field(&inter, h, &Location::default(), 64.0, msdf).is_ok());

    // Each layer pushed holds 16 bytes a pixel: 16 deep over a million pixels pass 272 MB.
    let m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
    let refused = r.compose(&nested(1000.0, 16), &m, None).map(|_| ()).map_err(|e| e.kind());
    assert_eq!(refused, Err(ErrorKind::LimitExceeded));
    assert!(r.compose(&nested(200.0, 16), &m, None).is_ok());
}

// The most two draws of one thing differ by, a level for rounding, where both cover a pixel's bytes.
fn levels(a: &[u8], b: &[u8]) -> u8 {
    a.iter().zip(b).map(|(&x, &y)| x.abs_diff(y)).max().unwrap_or(0)
}

// Glyphs drawn into a target their lent images' size, and one a band of rows smaller than them, match the images to
// a level: the target's outline is cut in f64, the image's placed in f32.
#[test]
fn glyphs_drawn_into_a_target_match_their_lent_images() {
    use crate::raster::{RasterOptions, Target};
    let Some(inter) = font("inter/InterVariable.ttf") else { return };
    let mut r = Rasterizer::new();
    let at = Location::default();
    for c in "AgQ&@%".chars() {
        let g = inter.glyph_id(c).expect("Inter maps it");
        for (px, options) in [(40.0, RasterOptions::DEFAULT), (17.0, RasterOptions::DEFAULT.with_offset(0.3, 0.6))] {
            let image = r.glyph(&inter, g, &at, px, &options).expect("draws").to_bitmap();
            let (w, h) = (image.width + 4, image.height + 4);
            let mut pixels = vec![7u8; (w * h) as usize];
            let target = Target::new(image.left - 2, image.top + 2, w, h, w as usize, &mut pixels);
            r.glyph_into(&inter, g, &at, px, &options, target).expect("draws into the target");
            let inner: Vec<u8> = (0..image.height as usize)
                .flat_map(|y| pixels[(y + 2) * w as usize + 2..][..image.width as usize].to_vec())
                .collect();
            assert!(levels(&inner, &image.pixels) <= 1, "{c} at {px}: {} levels", levels(&inner, &image.pixels));
            assert!(pixels[..w as usize * 2].iter().all(|&v| v == 0), "{c}: the margin was left");
        }
    }
}

// H at 40,000 px is 30,000 pixels tall, past MAX_SIDE: lending it is refused, but a target of a few rows across its
// stem and its counter draws, the stem whole and the counter empty, in a band or in many.
#[test]
fn a_glyph_past_max_side_draws_into_a_target() {
    use crate::raster::{RasterOptions, Target};
    let Some(inter) = font("inter/InterVariable.ttf") else { return };
    let h = inter.glyph_id('H').expect("Inter maps H");
    let (at, o) = (Location::default(), RasterOptions::DEFAULT);
    let mut r = Rasterizer::new();
    let lent = r.glyph(&inter, h, &at, 40_000.0, &o).map(drop).map_err(|e| e.kind());
    assert_eq!(lent, Err(ErrorKind::LimitExceeded));
    let mut path = Path::new();
    inter.prepared_outline(h, &at, 40_000.0, &crate::outline::OutlineOptions::DEFAULT, &mut path).unwrap();
    let b = path.view().bounds().expect("H has points");
    let (x, y) = (b.x_min as i32 + 200, ((b.y_min + b.y_max) / 2.0) as i32);
    let mut pixels = vec![0u8; 64 * 16];
    r.glyph_into(&inter, h, &at, 40_000.0, &o, Target::new(x, y, 64, 16, 64, &mut pixels)).unwrap();
    assert!(pixels.iter().all(|&v| v == 255), "the stem is whole");
    let mid = ((b.x_min + b.x_max) / 2.0) as i32;
    let mut pixels = vec![9u8; 64 * 16];
    r.glyph_into(&inter, h, &at, 40_000.0, &o, Target::new(mid, y + 6_000, 64, 16, 64, &mut pixels)).unwrap();
    assert!(pixels.iter().all(|&v| v == 0), "the counter is empty");
}

// A path in a target wider and taller than a band, drawn band by band, matches its lent image to a level.
#[test]
fn a_path_drawn_band_by_band_matches_its_lent_image() {
    use crate::raster::Target;
    let mut p = Path::new();
    p.move_to(0.0, 0.0);
    p.curve_to(4000.0, 300.0, -200.0, 1200.0, 3900.0, 1500.0);
    p.line_to(100.0, 1500.0);
    p.quad_to(2000.0, 700.0, 0.0, 0.0);
    p.close();
    let mut r = Rasterizer::new();
    let image = r.path(&p, FillRule::EvenOdd, None).expect("draws").to_bitmap();
    let (w, h) = (image.width, image.height);
    assert!(u64::from(w + 2) * 9 * u64::from(h) > crate::rasterizer::BAND_BYTES, "one band holds it");
    let mut pixels = vec![0u8; (w * h) as usize];
    let target = Target::new(image.left, image.top, w, h, w as usize, &mut pixels);
    r.path_into(&p, FillRule::EvenOdd, None, target).unwrap();
    assert!(levels(&pixels, &image.pixels) <= 1, "{} levels", levels(&pixels, &image.pixels));
    let short = Target::new(0, 0, 10, 10, 10, &mut pixels[..99]);
    assert_eq!(r.path_into(&p, FillRule::NonZero, None, short).map_err(|e| e.kind()), Err(ErrorKind::InvalidArgument));
}

// Color glyphs drawn into a target match their lent images to a level, transparent past them.
#[test]
fn color_glyphs_drawn_into_a_target_match_their_lent_images() {
    use crate::color::SceneOptions;
    use crate::raster::{RasterOptions, Target};
    let Some(f) = font("colr-v1-test-glyphs/test_glyphs.ttf") else { return };
    let mut r = Rasterizer::new();
    let (at, o, scene) = (Location::default(), RasterOptions::DEFAULT.with_offset(0.25, 0.5), SceneOptions::default());
    let mut drawn = 0;
    for g in (0..f.glyph_count()).step_by(3) {
        let Some(image) = r.color_glyph(&f, crate::GlyphId(g), &at, 48.0, &o, &scene).unwrap() else { continue };
        let image = image.to_bitmap();
        let (w, h) = (image.width + 2, image.height + 2);
        let mut pixels = vec![3u8; (w * h * 4) as usize];
        let target = Target::new(image.left - 1, image.top + 1, w, h, (w * 4) as usize, &mut pixels);
        assert!(r.color_glyph_into(&f, crate::GlyphId(g), &at, 48.0, &o, &scene, target).unwrap());
        let stride = (w * 4) as usize;
        let inner: Vec<u8> = (0..image.height as usize)
            .flat_map(|y| pixels[(y + 1) * stride + 4..][..image.width as usize * 4].to_vec())
            .collect();
        assert!(levels(&inner, &image.pixels) <= 1, "glyph {g}: {} levels", levels(&inner, &image.pixels));
        assert!(pixels[..stride].iter().all(|&v| v == 0), "glyph {g}: the margin was left");
        drawn += 1;
    }
    assert!(drawn > 20, "{drawn} drawn");
}

// A target no pixels wide or tall takes a color glyph as it takes a gray one: nothing drawn, the pixels as they were.
#[test]
fn a_color_glyph_into_an_empty_target_draws_nothing() {
    use crate::color::SceneOptions;
    use crate::raster::{RasterOptions, Target};
    let Some(f) = font("colr-v1-test-glyphs/test_glyphs.ttf") else { return };
    let mut r = Rasterizer::new();
    let (at, o, scene) = (Location::default(), RasterOptions::DEFAULT, SceneOptions::default());
    let painted = |r: &mut Rasterizer, g| matches!(r.color_glyph(&f, g, &at, 48.0, &o, &scene), Ok(Some(_)));
    let g = (0..f.glyph_count()).map(crate::GlyphId).find(|&g| painted(&mut r, g)).unwrap();
    let mut pixels = [7u8; 64];
    for (w, h) in [(0, 4), (4, 0), (0, 0)] {
        let target = Target::new(0, 0, w, h, 16, &mut pixels);
        assert!(r.color_glyph_into(&f, g, &at, 48.0, &o, &scene, target).unwrap(), "{w} by {h}");
    }
    assert!(pixels.iter().all(|&v| v == 7), "an empty target was written");
}

// A path reaching 10^30 pixels out, and a square placed 3 × 10^9 along, lie past what a lent image's i32 extent and
// corner can say: refused as too large and as too far, nothing wrapped on the way.
#[test]
fn a_path_past_what_an_image_holds_is_refused_not_wrapped() {
    let mut r = Rasterizer::new();
    let kind = |r: &mut Rasterizer, p: &Path, t: Option<&[f64; 6]>| {
        r.path(p, FillRule::NonZero, t).map(|_| ()).map_err(|e| e.kind())
    };
    let mut spike = Path::new();
    spike.move_to(0.0, 0.0);
    spike.line_to(1.0e30, 1.0);
    spike.line_to(0.0, 2.0);
    spike.close();
    assert_eq!(kind(&mut r, &spike, None), Err(ErrorKind::LimitExceeded));
    let along = [1.0, 0.0, 0.0, 1.0, 3.0e9, 0.0];
    assert_eq!(kind(&mut r, &square(1000.0), Some(&along)), Err(ErrorKind::LimitExceeded));
    assert!(kind(&mut r, &square(1000.0), None).is_ok());
}
