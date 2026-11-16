// Color glyphs, held to a second compositor: f64 throughout, each fill and clip shape's polygons as the rasterizer
// flattens them covered by the truth, the same ops in the same order.
use std::vec::Vec;

use super::coverage::{font, ROUNDING};
use super::truth;
use crate::color::{
    concat, Blend, ClipShape, DisplayList, Extend, Gradient, GradientKind, Op, Paint, Rgba, SceneOptions, Stop,
};
use crate::daecore::daekit::draw::gradient::Ramp;
use crate::daecore::daekit::draw::srgb::to_linear;
use crate::daecore::daetype::outline::{FillRule, OutlinePen, Path, PathView};
use crate::raster::{Bitmap, RasterOptions, Rasterizer};
use crate::{Font, GlyphId, Location};

type Px = [f64; 4];

fn linear(c: Rgba) -> Px {
    let a = f64::from(c.a) / 255.0;
    [f64::from(to_linear(c.r)) * a, f64::from(to_linear(c.g)) * a, f64::from(to_linear(c.b)) * a, a]
}

fn over(d: &mut Px, s: Px, k: f64) {
    let keep = 1.0 - s[3] * k;
    *d = [0, 1, 2, 3].map(|i| s[i] * k + d[i] * keep);
}

fn blended(mode: Blend, s: Px, d: Px, k: f64) -> Px {
    let straight = |p: Px| if p[3] > 0.0 { [p[0] / p[3], p[1] / p[3], p[2] / p[3]] } else { [0.0; 3] };
    let r = if mode == Blend::SrcOver {
        [0, 1, 2, 3].map(|i| s[i] + d[i] * (1.0 - s[3]))
    } else {
        let (cs, cb) = (straight(s).map(|v| v as f32), straight(d).map(|v| v as f32));
        let r = mode.composite(cs, s[3] as f32, cb, d[3] as f32);
        [r.color[0], r.color[1], r.color[2], r.alpha].map(f64::from)
    };
    [0, 1, 2, 3].map(|i| d[i] + k * (r[i] - d[i]))
}

// A path's polygons, flattened as the rasterizer flattens it under `m`.
fn polygons(path: PathView<'_>, m: &[f64; 6]) -> Vec<Vec<[f64; 2]>> {
    let mut r = Rasterizer::new();
    r.edges.begin(m.map(|v| v as f32));
    path.replay(None, &mut r.edges);
    r.edges.finish();
    r.edges.contours().map(|c| c.iter().map(|p| p.map(f64::from)).collect()).collect()
}

// The scene composed over the image's pixels in f64, inside `clip`, a box in the scene's units, where there's one.
fn exact(list: &DisplayList, m: &[f64; 6], clip: Option<[f64; 4]>, image: &Bitmap) -> Vec<Px> {
    let (w, h) = (image.width as usize, image.height as usize);
    let cover = |path, rule, t: &[f64; 6]| {
        truth::coverage(&polygons(list.path(path).unwrap(), &concat(t, m)), rule, image.left, image.top, w, h)
    };
    let device = [m[0], -m[1], m[2], -m[3], m[4] - f64::from(image.left), f64::from(image.top) - m[5]];
    let mut layers: Vec<(Vec<Px>, f64, Blend)> = vec![(vec![[0.0; 4]; w * h], 1.0, Blend::SrcOver)];
    let mut clips: Vec<(usize, Vec<f64>)> = Vec::new();
    if let Some([x0, y0, x1, y1]) = clip {
        let rect = polygons(square(x0 as f32, y0 as f32, x1 as f32, y1 as f32).view(), m);
        let mask = truth::coverage(&rect, FillRule::NonZero, image.left, image.top, w, h);
        clips.push((0, mask.into_iter().map(|v| v.min(1.0)).collect()));
    }
    let clip_of =
        |clips: &[(usize, Vec<f64>)], layer: usize| clips.last().filter(|c| c.0 == layer).map(|c| c.1.clone());
    for op in list.ops() {
        let depth = layers.len() - 1;
        match *op {
            Op::Fill { path, paint, rule, transform } => {
                let c = cover(path, rule, &transform);
                let clip = clip_of(&clips, depth);
                let ramp = match paint {
                    Paint::Gradient(id) => Some(Ramp::new(&list.gradient(id).unwrap(), &device)),
                    _ => None,
                };
                for i in 0..w * h {
                    let k = c[i].min(1.0) * clip.as_ref().map_or(1.0, |m| m[i]);
                    if k <= 0.0 {
                        continue;
                    }
                    // A gradient's colors unrounded, as the compositor paints them: `at` gives bytes, and a byte of
                    // alpha over a channel near black moves its output by levels where sRGB's curve is steep.
                    let color = match (&ramp, paint) {
                        (Some(r), _) => {
                            let mut px = [[0.0f32; 4]];
                            r.fill_row(i % w, i / w, &mut px);
                            px[0].map(f64::from)
                        }
                        (None, Paint::Solid(c)) => linear(c),
                        _ => [0.0; 4],
                    };
                    over(&mut layers[depth].0[i], color, k);
                }
            }
            Op::PushClip { shapes } => {
                let mut mask = vec![0.0f64; w * h];
                for s in list.shapes(shapes) {
                    let ClipShape { path, rule, transform, .. } = *s;
                    for (k, c) in mask.iter_mut().zip(cover(path, rule, &transform)) {
                        *k = k.max(c.min(1.0));
                    }
                }
                if let Some(base) = clip_of(&clips, depth) {
                    mask.iter_mut().zip(base).for_each(|(k, b)| *k *= b);
                }
                clips.push((depth, mask));
            }
            Op::PopClip => drop(clips.pop()),
            Op::PushLayer { opacity, blend } => {
                layers.push((vec![[0.0; 4]; w * h], f64::from(opacity.clamp(0.0, 1.0)), blend));
            }
            Op::PopLayer if depth > 0 => pop(&mut layers, &mut clips),
            _ => {}
        }
    }
    while layers.len() > 1 {
        pop(&mut layers, &mut clips);
    }
    layers.pop().unwrap().0
}

fn pop(layers: &mut Vec<(Vec<Px>, f64, Blend)>, clips: &mut Vec<(usize, Vec<f64>)>) {
    let (source, opacity, mode) = layers.pop().unwrap();
    let depth = layers.len() - 1;
    while clips.last().is_some_and(|c| c.0 > depth) {
        clips.pop();
    }
    let clip = clips.last().filter(|c| c.0 == depth).map(|c| c.1.clone());
    for (i, (d, s)) in layers[depth].0.iter_mut().zip(source).enumerate() {
        let k = clip.as_ref().map_or(1.0, |m| m[i]);
        if k > 0.0 {
            *d = blended(mode, s.map(|v| v * opacity), *d, k);
        }
    }
}

// Premultiplied linear light as premultiplied sRGB bytes, through sRGB's own curve, unrounded.
fn bytes(p: Px) -> Px {
    let a = p[3].clamp(0.0, 1.0);
    if a <= 0.0 {
        return [0.0; 4];
    }
    let encode = |v: f64| if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    let level = |v: f64| encode((v / a).clamp(0.0, 1.0)) * 255.0 * a;
    [level(p[0]), level(p[1]), level(p[2]), a * 255.0]
}

fn worst(image: &Bitmap, want: &[Px]) -> f64 {
    image
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(want)
        .flat_map(|(got, &w)| (0..4).map(move |i| (f64::from(got[i]) - bytes(w)[i]).abs()))
        .fold(0.0, f64::max)
}

fn square(x0: f32, y0: f32, x1: f32, y1: f32) -> Path {
    let mut p = Path::new();
    p.move_to(x0, y0);
    p.line_to(x1, y0);
    p.line_to(x1, y1);
    p.line_to(x0, y1);
    p.close();
    p
}

const ID: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

fn fill(list: &mut DisplayList, path: &Path, paint: Paint) {
    let path = list.push_path(path.view());
    list.push(Op::Fill { path, paint, rule: FillRule::NonZero, transform: ID });
}

fn scenes() -> Vec<(&'static str, DisplayList)> {
    let mut out = Vec::new();
    let mut l = DisplayList::default();
    fill(&mut l, &square(1.0, 1.0, 9.0, 9.0), Paint::Solid(Rgba::opaque(255, 0, 0)));
    out.push(("an opaque square", l));
    let mut l = DisplayList::default();
    fill(&mut l, &square(0.5, 0.5, 7.25, 7.75), Paint::Solid(Rgba { r: 20, g: 200, b: 90, a: 128 }));
    fill(&mut l, &square(3.3, 2.6, 9.5, 9.1), Paint::Solid(Rgba { r: 250, g: 10, b: 40, a: 200 }));
    out.push(("two translucent squares overlapping", l));
    let mut l = DisplayList::default();
    let clip = l.push_path(square(2.5, 2.5, 6.5, 6.5).view());
    let shapes = l.push_shapes(&[ClipShape { path: clip, rule: FillRule::NonZero, transform: ID }]);
    l.push(Op::PushClip { shapes });
    fill(&mut l, &square(0.0, 0.0, 10.0, 10.0), Paint::Solid(Rgba::opaque(0, 0, 255)));
    l.push(Op::PopClip);
    out.push(("a square clipped", l));
    let mut l = DisplayList::default();
    let (a, b) = (l.push_path(square(1.0, 1.0, 6.3, 6.3).view()), l.push_path(square(4.1, 3.2, 9.4, 8.6).view()));
    let shapes = l.push_shapes(&[
        ClipShape { path: a, rule: FillRule::NonZero, transform: ID },
        ClipShape { path: b, rule: FillRule::NonZero, transform: ID },
    ]);
    l.push(Op::PushClip { shapes });
    fill(&mut l, &square(0.0, 0.0, 10.0, 10.0), Paint::Solid(Rgba { r: 0, g: 120, b: 255, a: 230 }));
    l.push(Op::PopClip);
    out.push(("a clip of two shapes overlapping", l));
    let mut l = DisplayList::default();
    fill(&mut l, &square(0.0, 0.0, 6.0, 10.0), Paint::Solid(Rgba::opaque(240, 200, 30)));
    l.push(Op::PushLayer { opacity: 0.6, blend: Blend::Multiply });
    fill(&mut l, &square(3.2, 1.0, 10.0, 8.6), Paint::Solid(Rgba::opaque(40, 90, 250)));
    l.push(Op::PopLayer);
    out.push(("a layer multiplied at 0.6", l));
    let mut l = DisplayList::default();
    let stops = [
        Stop { offset: 0.0, color: Rgba::opaque(255, 0, 0) },
        Stop { offset: 1.0, color: Rgba { r: 0, g: 0, b: 255, a: 60 } },
    ];
    let kind = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 3.0 };
    let g = l.push_gradient(Gradient::new(kind, &stops, Extend::Reflect, ID));
    fill(&mut l, &square(0.2, 0.4, 9.7, 9.9), Paint::Gradient(g));
    out.push(("a linear gradient", l));
    let mut l = DisplayList::default();
    fill(&mut l, &square(0.0, 0.0, 10.0, 10.0), Paint::Solid(Rgba::opaque(255, 193, 7)));
    let faint = Rgba { r: 95, g: 122, b: 255, a: 0 };
    let stops = [Stop { offset: 0.0, color: faint }, Stop { offset: 1.0, color: Rgba { a: 3, ..faint } }];
    let kind = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 };
    let g = l.push_gradient(Gradient::new(kind, &stops, Extend::Pad, ID));
    fill(&mut l, &square(0.0, 0.0, 10.0, 10.0), Paint::Gradient(g));
    out.push(("a faint gradient over a channel near black", l));
    out
}

#[test]
fn scenes_compose_as_the_exact_model_does() {
    let mut r = Rasterizer::new();
    for (what, list) in scenes() {
        for m in [[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [2.5, 0.3, -0.4, 2.2, 0.37, 0.81]] {
            let image = r.compose(&list, &m, None).map(|d| r.image(&d).to_bitmap()).unwrap();
            assert_eq!(image.channels, 4);
            let gap = worst(&image, &exact(&list, &m, None, &image));
            assert!(gap <= ROUNDING, "{what} under {m:?}: {gap} levels");
        }
    }
}

// A clip box inside the scene cuts it, and one reaching far past it cuts nothing.
#[test]
fn a_clip_box_composes_as_the_exact_model_does_wherever_it_reaches() {
    let mut r = Rasterizer::new();
    for (what, list) in scenes() {
        for clip in [[2.0, 3.0, 7.5, 8.0], [-40.0, -30.0, 50.0, 60.0]] {
            for m in [[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [2.5, 0.3, -0.4, 2.2, 0.37, 0.81]] {
                let image = r.compose(&list, &m, Some(clip)).map(|d| r.image(&d).to_bitmap()).unwrap();
                let gap = worst(&image, &exact(&list, &m, Some(clip), &image));
                assert!(gap <= ROUNDING, "{what} inside {clip:?} under {m:?}: {gap} levels");
            }
        }
    }
}

fn glyphs(r: &mut Rasterizer, font: &Font, px: f32) -> usize {
    let options = RasterOptions::DEFAULT.with_offset(0.3, 0.6);
    let scene = SceneOptions::DEFAULT.with_foreground(Rgba::opaque(30, 60, 200));
    let mut drawn = 0;
    for g in 0..font.glyph_count() {
        let Ok(Some(image)) = r.color_glyph(font, GlyphId(g), &Location::default(), px, &options, &scene) else {
            continue;
        };
        let image = image.to_bitmap();
        let list = font.color_scene(GlyphId(g), &Location::default(), &scene).unwrap().unwrap();
        let s = f64::from(px / f32::from(font.units_per_em()));
        let m = [s, 0.0, 0.0, s, 0.3, 0.6];
        if image.width == 0 {
            continue;
        }
        let clip = font.colr_clip_box(GlyphId(g), &Location::default()).map(|b| [b.x_min, b.y_min, b.x_max, b.y_max]);
        let gap = worst(&image, &exact(&list, &m, clip, &image));
        assert!(gap <= ROUNDING, "glyph {g} at {px} px: {gap} levels");
        for p in image.pixels.as_chunks::<4>().0 {
            assert!(p[..3].iter().all(|&c| c <= p[3]), "glyph {g}: {p:?} isn't premultiplied");
        }
        drawn += 1;
    }
    drawn
}

#[test]
fn color_glyphs_compose_as_the_exact_model_does() {
    let mut r = Rasterizer::new();
    let mut drawn = 0;
    for name in ["colr-v1-test-glyphs/test_glyphs.ttf", "bungee-tint/BungeeTint-Regular.ttf"] {
        let Some(font) = font(name) else { continue };
        drawn += glyphs(&mut r, &font, 24.0);
    }
    assert!(drawn > 100, "only {drawn} color glyphs drawn");
}

#[test]
fn a_glyph_without_a_scene_answers_none_and_a_layout_is_refused() {
    let Some(font) = font("inter/InterVariable.ttf") else { return };
    let mut r = Rasterizer::new();
    let g = font.glyph_id('a').unwrap();
    let scene = SceneOptions::DEFAULT;
    assert!(r.color_glyph(&font, g, &Location::default(), 20.0, &RasterOptions::DEFAULT, &scene).unwrap().is_none());
    let layout = crate::raster::SubpixelLayout::horizontal(crate::raster::StripeOrder::Rgb);
    let striped = RasterOptions::DEFAULT.with_layout(&layout);
    assert!(r.color_glyph(&font, g, &Location::default(), 20.0, &striped, &scene).is_err());
}

// A matrix whose terms cancel, as a PaintTransform (xx 16001) around a PaintTranslate (dx -30000): placed in f32 its
// points land tens of pixels astray, in f64 the square fills its 1,000.06 pixels from 15.5 across.
#[test]
fn a_fill_whose_matrix_s_terms_cancel_draws_where_it_lies() {
    let mut l = DisplayList::default();
    fill(&mut l, &square(30_000.0, 0.0, 30_000.063, 100.0), Paint::Solid(Rgba::opaque(0, 0, 0)));
    let m = [16001.0, 0.0, 0.0, 1.0, -30000.0 * 16001.0 + 15.5, 0.0];
    let mut r = Rasterizer::new();
    let image = r.compose(&l, &m, None).map(|d| r.image(&d).to_bitmap()).unwrap();
    assert_eq!((image.left, image.top, image.width, image.height), (15, 100, 1001, 100));
    assert!(image.pixels.as_chunks::<4>().0.chunks(1001).all(|row| row[1..1000].iter().all(|p| p[3] == 255)));
}

// A curved fill, its controls inside its extent as a glyph's are, is placed in f32 as a lent draw is, and in f64 once a
// square 5,000 px left widens the frame past NEAR from its corner: the pixels it covers come out alike to a level.
#[test]
fn a_fill_placed_in_f32_or_in_f64_covers_alike() {
    let mut p = Path::new();
    p.move_to(1.0, 1.0);
    p.quad_to(30.0, 1.0, 30.0, 20.0);
    p.quad_to(30.0, 30.0, 15.0, 30.0);
    p.curve_to(5.0, 30.0, 1.0, 20.0, 1.0, 1.0);
    p.close();
    let (mut alone, mut wide) = (DisplayList::default(), DisplayList::default());
    fill(&mut alone, &p, Paint::Solid(Rgba::opaque(0, 0, 0)));
    fill(&mut wide, &p, Paint::Solid(Rgba::opaque(0, 0, 0)));
    fill(&mut wide, &square(-5_000.0, 1.0, -4_990.0, 11.0), Paint::Solid(Rgba::opaque(0, 0, 0)));
    let mut r = Rasterizer::new();
    let m = [1.0, 0.0, 0.0, 1.0, 0.3, 0.7];
    let near = r.compose(&alone, &m, None).map(|d| r.image(&d).to_bitmap()).unwrap();
    let far = r.compose(&wide, &m, None).map(|d| r.image(&d).to_bitmap()).unwrap();
    let (across, down) = ((near.left - far.left) as usize, (far.top - near.top) as usize);
    let (w, fw) = (near.width as usize * 4, far.width as usize * 4);
    let worst = (0..near.height as usize)
        .flat_map(|y| near.pixels[y * w..][..w].iter().zip(&far.pixels[(y + down) * fw + across * 4..][..w]))
        .map(|(a, b)| a.abs_diff(*b))
        .max();
    assert!(worst.is_some_and(|w| w <= 1), "{worst:?} levels apart");
}

// A fill 10^15 pixels away lies past where an image's i32 corner can say: refused as too large, whatever it covers.
#[test]
fn a_scene_past_what_an_image_s_corner_holds_is_refused() {
    let mut l = DisplayList::default();
    let mut p = Path::new();
    p.move_to(0.0, 0.0);
    p.line_to(1.0, 1.0);
    fill(&mut l, &p, Paint::Solid(Rgba::opaque(0, 0, 0)));
    let far = [1.0, 0.0, 0.0, 1.0, 1.0e15, 0.0];
    let kind = Rasterizer::new().compose(&l, &far, None).map(|_| ()).map_err(|e| e.kind());
    assert_eq!(kind, Err(crate::ErrorKind::LimitExceeded));
}

// A clip box 20,000 pixels across around a fill of 100 cuts nothing: the fill draws in its own frame, as unclipped,
// where the box's would pass MAX_SIDE.
#[test]
fn a_clip_box_holding_the_fills_leaves_their_frame() {
    let mut l = DisplayList::default();
    fill(&mut l, &square(0.25, 0.5, 100.0, 99.5), Paint::Solid(Rgba { r: 20, g: 200, b: 90, a: 128 }));
    let mut r = Rasterizer::new();
    let mut drawn = |clip| r.compose(&l, &ID, clip).map(|d| r.image(&d).to_bitmap()).unwrap();
    let (plain, clipped) = (drawn(None), drawn(Some([-10000.0, -10000.0, 10000.0, 10000.0])));
    assert_eq!((clipped.left, clipped.top, clipped.width, clipped.height), (0, 100, 100, 100));
    assert_eq!(clipped.pixels, plain.pixels);
}

// A diamond 20,000 units across, a clip shape around a fill of 100 as a PaintGlyph outline can be.
fn diamond(l: &mut DisplayList) {
    let mut p = Path::new();
    polygon(&mut p, &[(50.0, -10000.0), (10000.0, 50.0), (50.0, 10000.0), (-10000.0, 50.0)]);
    let path = l.push_path(p.view());
    let shapes = l.push_shapes(&[ClipShape { path, rule: FillRule::NonZero, transform: ID }]);
    l.push(Op::PushClip { shapes });
}

fn polygon(path: &mut Path, points: &[(f32, f32)]) {
    path.move_to(points[0].0, points[0].1);
    points[1..].iter().for_each(|&(x, y)| path.line_to(x, y));
    path.close();
}

// The frame is what shows: clips past MAX_SIDE (a v1 box pushed as a shape, an outline) leave a 100 px fill drawing as
// unclipped; a box cutting it along y draws its lower half, and a fill reaching as far draws only what the box shows.
#[test]
fn clips_and_fills_reaching_past_max_side_are_cut_to_the_frame() {
    let color = Paint::Solid(Rgba { r: 20, g: 200, b: 90, a: 128 });
    let mut plain = DisplayList::default();
    fill(&mut plain, &square(0.25, 0.5, 100.0, 99.5), color);
    let mut v1 = DisplayList::default();
    let rect = v1.push_path(square(-10000.0, -10000.0, 10000.0, 10000.0).view());
    let shapes = v1.push_shapes(&[ClipShape { path: rect, rule: FillRule::NonZero, transform: ID }]);
    v1.push(Op::PushClip { shapes });
    fill(&mut v1, &square(0.25, 0.5, 100.0, 99.5), color);
    v1.push(Op::PopClip);
    let mut outline = DisplayList::default();
    diamond(&mut outline);
    fill(&mut outline, &square(0.25, 0.5, 100.0, 99.5), color);
    outline.push(Op::PopClip);
    let mut wide = DisplayList::default();
    fill(&mut wide, &square(-10000.0, -10000.0, 10000.0, 10000.0), color);
    let mut r = Rasterizer::new();
    let mut drawn = |l: &DisplayList, clip| r.compose(l, &ID, clip).map(|d| r.image(&d).to_bitmap()).unwrap();
    let want = drawn(&plain, None);
    for got in [drawn(&v1, Some([-10000.0, -10000.0, 10000.0, 10000.0])), drawn(&outline, None)] {
        assert_eq!((got.left, got.top, got.width, got.height), (0, 100, 100, 100));
        assert_eq!(got.pixels, want.pixels);
    }
    let half = drawn(&plain, Some([-10000.0, -10000.0, 10000.0, 50.0]));
    assert_eq!((half.left, half.top, half.width, half.height), (0, 50, 100, 50));
    assert_eq!(half.pixels, want.pixels[50 * 100 * 4..]);
    let boxed = drawn(&wide, Some([0.0, 0.0, 100.0, 100.0]));
    assert_eq!((boxed.left, boxed.top, boxed.width, boxed.height), (0, 100, 100, 100));
    assert!(boxed.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 128));
}

// A frame's corner sets how every point in it rounds, so it's what shows however far a clip reaches: a curve inside
// a clip rectangle or an outline 800 units across, 2,000 pixels and within its limits, draws as alone, byte for byte.
#[test]
fn a_clip_reaching_past_the_fills_leaves_them_drawing_as_alone() {
    let m = [2.5, 0.3, -0.4, 2.2, 0.37, 0.81];
    let mut p = Path::new();
    p.move_to(3.1, 7.3);
    p.quad_to(41.7, -9.2, 33.9, 29.6);
    p.quad_to(17.3, 45.1, 3.1, 7.3);
    p.close();
    let color = Paint::Solid(Rgba { r: 20, g: 200, b: 90, a: 128 });
    let mut alone = DisplayList::default();
    fill(&mut alone, &p, color);
    let mut rect = DisplayList::default();
    let far = rect.push_path(square(-400.0, -400.0, 400.0, 400.0).view());
    let shapes = rect.push_shapes(&[ClipShape { path: far, rule: FillRule::NonZero, transform: ID }]);
    rect.push(Op::PushClip { shapes });
    let mut outline = DisplayList::default();
    let mut d = Path::new();
    polygon(&mut d, &[(20.0, -400.0), (400.0, 20.0), (20.0, 400.0), (-400.0, 20.0)]);
    let far = outline.push_path(d.view());
    let shapes = outline.push_shapes(&[ClipShape { path: far, rule: FillRule::NonZero, transform: ID }]);
    outline.push(Op::PushClip { shapes });
    let mut r = Rasterizer::new();
    let want = r.compose(&alone, &m, None).map(|d| r.image(&d).to_bitmap()).unwrap();
    for (what, mut l) in [("a rectangle", rect), ("an outline", outline)] {
        fill(&mut l, &p, color);
        l.push(Op::PopClip);
        let got = r.compose(&l, &m, None).map(|d| r.image(&d).to_bitmap()).unwrap();
        assert_eq!((got.left, got.top, got.width, got.height), (want.left, want.top, want.width, want.height));
        assert!(got.pixels == want.pixels, "inside {what}, the fill draws other than alone");
    }
}

// Cut to the frame, a clip box along y reaching 20,000 units across, a clip rectangle and an outline as far pushed in
// the scene, and a curve reaching 9,000 units past a small box, compose as the exact model does with them whole.
#[test]
fn clips_and_fills_cut_to_the_frame_compose_as_the_exact_model_does() {
    let mut r = Rasterizer::new();
    let far = [-20000.0, 3.0, 20000.0, 8.0];
    let red = Paint::Solid(Rgba::opaque(255, 0, 0));
    for m in [ID, [2.5, 0.3, -0.4, 2.2, 0.37, 0.81]] {
        let mut lists: Vec<_> = scenes().into_iter().map(|(what, l)| (what, l, far)).collect();
        let mut l = DisplayList::default();
        let rect = l.push_path(square(-20000.0, 2.0, 20000.0, 7.5).view());
        let shapes = l.push_shapes(&[ClipShape { path: rect, rule: FillRule::NonZero, transform: ID }]);
        l.push(Op::PushClip { shapes });
        fill(&mut l, &square(1.0, 1.0, 9.0, 9.0), red);
        l.push(Op::PopClip);
        lists.push(("a clip rectangle pushed", l, far));
        let mut l = DisplayList::default();
        diamond(&mut l);
        fill(&mut l, &square(1.0, 1.0, 9.0, 9.0), red);
        l.push(Op::PopClip);
        lists.push(("an outline pushed", l, far));
        let mut l = DisplayList::default();
        let mut p = Path::new();
        p.move_to(-9000.0, -9000.0);
        p.quad_to(4.0, 30000.0, 9000.0, -9000.0);
        p.close();
        fill(&mut l, &p, red);
        lists.push(("a curve reaching past the box", l, [1.0, 2.0, 9.0, 7.5]));
        for (what, list, clip) in lists {
            let image = r.compose(&list, &m, Some(clip)).map(|d| r.image(&d).to_bitmap()).unwrap();
            let gap = worst(&image, &exact(&list, &m, Some(clip), &image));
            assert!(gap <= ROUNDING, "{what} under {m:?}: {gap} levels");
        }
    }
}

// A zigzag across a clip 16,000 pixels wide and one tall crosses 16,000 cells an edge: past its allowance it's
// refused before it's summed, and with a hundredth as many edges it draws.
#[test]
fn a_scene_priced_past_its_allowance_is_refused_before_its_work() {
    use crate::rasterizer::{compose::{BASE, PASSES}, coverage::CELL};
    let scene = |edges: u64| {
        let mut p = Path::new();
        p.move_to(0.0, 0.0);
        (1..edges).for_each(|i| p.line_to(if i % 2 == 1 { 16_000.0 } else { 0.0 }, i as f32 / edges as f32));
        p.close();
        let mut l = DisplayList::default();
        fill(&mut l, &p, Paint::Solid(Rgba::opaque(0, 0, 0)));
        l
    };
    // The frame is the clip and a pixel spare each way.
    let edges = (PASSES * 16_002 * 3 + BASE) / (CELL * 16_000) + 1;
    let (mut r, clip) = (Rasterizer::new(), Some([0.0, 0.0, 16_000.0, 1.0]));
    let kind = r.compose(&scene(edges), &ID, clip).map(drop).map_err(|e| e.kind());
    assert_eq!(kind, Err(crate::ErrorKind::LimitExceeded));
    assert!(r.compose(&scene(edges / 100), &ID, clip).is_ok());
}

// 600,000 points zigzagging across a clip box's left side: the cut adds a point an edge, 1.2 million in all, past
// MAX_FLATTEN_POINTS. Cut in f64, the zigzag's part outside folds onto the side, and the box fills to the right of it.
#[test]
fn a_path_its_cut_doubles_draws() {
    let mut p = Path::new();
    p.move_to(-10.0, 0.0);
    for i in 1..600_000 {
        p.line_to(if i % 2 == 0 { -10.0 } else { 10.0 }, i as f32 * 1.0e-5);
    }
    p.close();
    let mut l = DisplayList::default();
    fill(&mut l, &p, Paint::Solid(Rgba::opaque(0, 0, 0)));
    let mut r = Rasterizer::new();
    let image = r.compose(&l, &ID, Some([0.0, 0.0, 10.0, 10.0])).map(|d| r.image(&d).to_bitmap()).unwrap();
    assert_eq!((image.left, image.top, image.width, image.height), (0, 6, 10, 6));
}
