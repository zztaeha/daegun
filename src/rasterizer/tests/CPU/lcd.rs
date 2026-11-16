use std::vec::Vec;

use super::coverage::{font, sample, FONTS, ROUNDING};
use super::truth;
use crate::daecore::daetype::outline::{FillRule, OutlinePen, Path};
use crate::raster::{Bitmap, RasterOptions, Rasterizer, StripeOrder, SubpixelLayout};
use crate::{Font, GlyphId, Location, Orientation};

fn layouts() -> Vec<(&'static str, SubpixelLayout)> {
    let blur: Vec<f32> = (0..36).map(|i| if i % 7 == 0 { 0.25 } else { 0.75 / 35.0 }).collect();
    let sharpen = [-0.25, 1.5, -0.25];
    let striped =
        |c: usize| (0..5).map(|k| if (c..c + 3).contains(&k) { sharpen[k - c] } else { 0.0 }).collect::<Vec<f32>>();
    let (r, g, b) = (striped(0), striped(1), striped(2));
    std::vec![
        ("horizontal RGB", SubpixelLayout::horizontal(StripeOrder::Rgb)),
        ("horizontal BGR", SubpixelLayout::horizontal(StripeOrder::Bgr)),
        ("vertical RGB", SubpixelLayout::vertical(StripeOrder::Rgb)),
        ("unfiltered across", SubpixelLayout::unfiltered(StripeOrder::Bgr, Orientation::Horizontal)),
        ("unfiltered down", SubpixelLayout::unfiltered(StripeOrder::Rgb, Orientation::Vertical)),
        ("6 by 6 over 4 by 2", SubpixelLayout::from_weights((4, 2), (6, 6), (-1, -2), [&blur, &blur, &blur]).unwrap()),
        ("sharpened", SubpixelLayout::from_weights((3, 1), (3, 1), (0, 0), [&sharpen, &sharpen, &sharpen]).unwrap()),
        ("sharpened stripes", SubpixelLayout::from_weights((3, 1), (5, 1), (-1, 0), [&r, &g, &b]).unwrap()),
    ]
}

// A layout's filter over the truth at its samples, `across` by `down`, in f64, for pixels `width` by `height` from
// the samples' first: what the bytes should hold before rounding.
fn filtered(layout: &SubpixelLayout, samples: &[f64], (across, down): (usize, usize), pixels: [isize; 4]) -> Vec<f64> {
    let (ox, oy) = layout.oversample();
    let (ox, oy) = (isize::from(ox), isize::from(oy));
    let tx = usize::from(layout.taps().0);
    let (gx, gy) = layout.origin();
    let mut out = Vec::new();
    for y in pixels[1]..pixels[3] {
        for x in pixels[0]..pixels[2] {
            for c in 0..3 {
                let mut sum = 0.0;
                for (k, &w) in layout.weights(c).unwrap().iter().enumerate() {
                    let sx = x * ox + isize::from(gx) + (k % tx) as isize;
                    let sy = y * oy + isize::from(gy) + (k / tx) as isize;
                    if (0..across as isize).contains(&sx) && (0..down as isize).contains(&sy) {
                        sum += f64::from(w) * 255.0 * samples[sy as usize * across + sx as usize];
                    }
                }
                out.push(sum);
            }
        }
    }
    out
}

// The most a channel moves when each sample is within `per_sample` of its truth.
fn carried(layout: &SubpixelLayout, per_sample: f64) -> f64 {
    let sum = |c| layout.weights(c).unwrap().iter().map(|w| f64::from(w.abs())).sum::<f64>();
    (0..3).map(sum).fold(0.0, f64::max) * per_sample
}

fn image(r: &mut Rasterizer, font: &Font, g: GlyphId, px: f32, options: &RasterOptions) -> Option<Bitmap> {
    r.glyph(font, g, &Location::default(), px, options).ok().map(|image| image.to_bitmap())
}

// The polygons just filled, in samples.
fn polygons(r: &Rasterizer) -> Vec<Vec<[f64; 2]>> {
    r.edges.contours().map(|c| c.iter().map(|p| p.map(f64::from)).collect()).collect()
}

// Each channel within the samples' rounding, carried through the filter, and the filter's own half a level. A glyph
// summed straight under stripes side by side is filtered before it's read, so it's rounded once.
#[test]
fn subpixel_layouts_filter_exact_coverage() {
    use crate::rasterizer::Shape;
    let (mut r, mut drawn, mut once) = (Rasterizer::new(), 0, 0);
    for name in FONTS.iter().take(4) {
        let Some(font) = font(name) else { continue };
        for (what, layout) in layouts() {
            for g in sample(&font, 6) {
                for (px, o) in [(9.0, (0.0, 0.0)), (16.0, (0.3, 0.6)), (40.0, (0.7, 0.1)), (120.0, (0.4, 0.9))] {
                    let options = RasterOptions::DEFAULT.with_layout(&layout).with_offset(o.0, o.1);
                    let Some(image) = image(&mut r, &font, g, px, &options) else { continue };
                    assert_eq!(image.channels, 3);
                    let (ox, oy) = layout.oversample();
                    let (w, h) = (image.width as usize, image.height as usize);
                    let (across, down) = (w * usize::from(ox), h * usize::from(oy));
                    let (left, top) = (image.left * i32::from(ox), image.top * i32::from(oy));
                    let samples = truth::coverage(&polygons(&r), FillRule::NonZero, left, top, across, down);
                    let want = filtered(&layout, &samples, (across, down), [0, 0, w as isize, h as isize]);
                    let worst = image
                        .pixels
                        .iter()
                        .zip(&want)
                        .map(|(&v, &t)| (f64::from(v) - t.clamp(0.0, 255.0)).abs())
                        .fold(0.0, f64::max);
                    let learned = r.learned.last().and_then(|l| l.glyphs.get(&g.0));
                    let summed = learned.is_some_and(|k| k.1 != Shape::Unsure);
                    let bound = if summed && layout.stripes().is_some() {
                        once += 1;
                        ROUNDING
                    } else {
                        carried(&layout, ROUNDING) + 0.5
                    };
                    assert!(worst <= bound, "{name} glyph {} {what} at {px} px: {worst} levels", g.0);
                    drawn += 1;
                }
            }
        }
    }
    assert!(drawn > 300 && once > 100, "only {drawn} images drawn, {once} rounded once");
}

// A pixel just past the image on any side would filter to nothing: the pad holds all the filter spreads.
#[test]
fn the_pad_holds_the_filter_s_spread() {
    let Some(font) = font(FONTS[0]) else { return };
    let mut r = Rasterizer::new();
    for (what, layout) in layouts() {
        for c in ['W', 'l', 'g'] {
            let g = font.glyph_id(c).unwrap();
            let image = image(&mut r, &font, g, 20.0, &RasterOptions::DEFAULT.with_layout(&layout)).unwrap();
            let (ox, oy) = layout.oversample();
            let (ox, oy) = (usize::from(ox), usize::from(oy));
            let (w, h) = (image.width as usize + 4, image.height as usize + 4);
            let (left, top) = ((image.left - 2) * ox as i32, (image.top + 2) * oy as i32);
            let samples = truth::coverage(&polygons(&r), FillRule::NonZero, left, top, w * ox, h * oy);
            let pixels = filtered(&layout, &samples, (w * ox, h * oy), [0, 0, w as isize, h as isize]);
            for y in 0..h {
                for x in 0..w {
                    let outside = x == 1 || y == 1 || x == w - 2 || y == h - 2;
                    let worst = pixels[(y * w + x) * 3..][..3].iter().fold(0.0f64, |m, v| m.max(v.abs()));
                    assert!(!outside || worst < 1e-9, "{what}, {c}: pixel ({x}, {y}) past the image gets {worst}");
                }
            }
        }
    }
}

// A contour on one vertical line, on a pixel's edge, is 0 pixels wide: each layout, filtered or striped, summed or
// exact, answers an image only its pad wide, of no columns where it has none.
#[test]
fn an_image_no_pixel_wide_filters_to_no_columns() {
    let mut line = Path::new();
    line.move_to(12.0, 0.0);
    line.line_to(12.0, 5.0);
    line.line_to(12.0, 2.0);
    line.close();
    let mut r = Rasterizer::new();
    for (what, layout) in layouts() {
        for exact in [false, true] {
            r.edges.begin([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
            r.edges.path(&line);
            let drawn = r.draw(FillRule::NonZero, &layout, exact).unwrap();
            assert_eq!(drawn.width, 2 * layout.pad().0, "{what}, exact {exact}");
        }
    }
}

// A glyph 9 × 10^8 pixels along fits an image's i32 corner, but not its stripes, three samples a pixel: refused, not
// wrapped.
#[test]
fn a_glyph_whose_samples_lie_past_i32_is_refused() {
    let Some(font) = font("inter/InterVariable.ttf") else { return };
    let g = font.glyph_id('H').expect("Inter maps H");
    let layout = SubpixelLayout::horizontal(StripeOrder::Rgb);
    let mut opts = RasterOptions::DEFAULT;
    (opts.layout, opts.offset) = (&layout, [9.0e8, 0.0]);
    let kind = Rasterizer::new().glyph(&font, g, &Location::default(), 16.0, &opts).map(drop).map_err(|e| e.kind());
    assert_eq!(kind, Err(crate::ErrorKind::LimitExceeded));
    opts.offset = [0.0, 0.0];
    assert!(Rasterizer::new().glyph(&font, g, &Location::default(), 16.0, &opts).is_ok());
}
