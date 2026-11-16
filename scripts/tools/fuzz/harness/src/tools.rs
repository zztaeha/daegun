//! What a caller does with values of its own rather than a font's: paths through pens, flattened and stroked, a
//! hinted outline's points, paint graphs walked and lowered to display lists, gradients, blends and matrices, packing
//! and subpixel filters. Each takes the input's choices, numbers that aren't finite included.

use daegun::color::gradient::{Interpolation, Ramp};
use daegun::color::{
    concat, invert, lower, Blend, DisplayList, Extend, Gradient, GradientKind, Op, PaintGraph, PaintNode, Rgba, Stop,
    Stops, IDENTITY,
};
use daegun::outline::{max_area_for, normalize_winding, HintedOutline, OutlinePen, Path, Point, QuadraticPen};
use daegun::outline::{StrokeStyle, TransformPen};
use daegun::raster::{Rect, ShelfPacker, SubpixelLayout};
use daegun::{Bounds, Font, GlyphId, Location};

use crate::checks::{check_ops, finite};
use crate::input::Choices;

// Numbers a caller may pass where a size, tolerance, width or angle goes.
const ANY: [f64; 8] = [0.25, 1.0, 3.0, 100.0, 0.0, -2.0, f64::NAN, f64::INFINITY];

fn any(c: &Choices, at: usize) -> f64 {
    ANY[usize::from(c.0[at]) % ANY.len()]
}

// A transform: none, a scale, a turn, a flip, one that folds the plane flat, or one that isn't finite.
fn transform(b: u8) -> [f64; 6] {
    match b % 6 {
        0 => IDENTITY,
        1 => [2.5, 0.0, 0.0, 0.5, 10.0, -3.0],
        2 => [0.8, 0.6, -0.6, 0.8, -40.0, 7.0],
        3 => [-1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        4 => [1.0, 2.0, 0.5, 1.0, 0.0, 0.0],
        _ => [f64::NAN, 0.0, 0.0, 1.0, f64::INFINITY, 0.0],
    }
}

/// A glyph's path as a caller handles one: copied out of its view, replayed under a transform both ways in, turned
/// to quads with their winding made one, flattened for a size, stroked simplified, and cleared.
pub fn check_path_tools(f: &Font, g: GlyphId, at: &Location, c: &Choices) {
    let mut path = Path::with_capacity(16, 64);
    if f.outline(g, at, &mut path).is_err() {
        return;
    }
    let what = format!("glyph {g}'s path");
    must!(path.view().to_path() == path, "{what}: its view copied out is another path");
    let _ = path.cost();
    let t = transform(c.0[22]);
    let (mut replayed, mut through) = (Path::new(), Path::new());
    path.replay(Some(&t), &mut replayed);
    path.view().replay(None, &mut TransformPen::new(&mut through, t));
    must!(replayed.parts().0 == through.parts().0, "{what}: under {t:?} replayed and through a pen, verbs differ");

    let tolerance = any(c, 21) as f32;
    let mut pen = QuadraticPen::new(tolerance);
    path.replay(None, &mut pen);
    if let Ok(mut quads) = pen.finish() {
        let fresh = quads.clone();
        must!(quads.iter().all(|q| finite(&[q.start, q.control, q.end])), "{what}: a quad isn't finite at {tolerance}");
        normalize_winding(&mut quads);
        let mut again = QuadraticPen::reusing(tolerance, quads);
        path.replay(None, &mut again);
        must!(again.finish().ok() == Some(fresh), "{what}: a pen reusing curves made other quads than a new one");
    }

    let area = max_area_for(any(c, 23) as f32, f32::from(f.units_per_em()));
    let mut contours = vec![vec![Point::default(); 3]];
    let (into, fresh) = (path.flatten_into(area, &mut contours), path.flatten(area));
    let alike = match &fresh {
        Ok(fresh) => into.is_ok() && contours == *fresh,
        Err(_) => into.is_err() && contours.is_empty(),
    };
    must!(alike, "{what}: flatten_into at area {area}, over a list holding a contour, isn't flatten");
    must!(contours.iter().all(|c| finite(c)), "{what}: flattened at area {area}, a point isn't finite");

    let mut stroked = Path::new();
    if path.stroke_simplified(&StrokeStyle::new(any(c, 23) as f32 * 10.0), tolerance, &mut stroked).is_ok() {
        must!(finite(stroked.parts().1), "{what}: stroked simplified, a point isn't finite");
    }
    path.clear();
    must!(path.is_empty(), "{what}: cleared, it still holds a contour");
}

/// A hinted outline read point by point and drawn: its contours end in order within its points, all finite.
pub fn check_hinted(h: &HintedOutline, what: &str) {
    let (n, ends) = (h.points().len(), h.contours());
    must!(ends.windows(2).all(|w| w[0] < w[1]) && ends.last().is_none_or(|&e| e < n),
          "{what}: contours ending at {ends:?} over {n} points");
    must!(h.points().all(|(p, _)| p.x.is_finite() && p.y.is_finite()), "{what}: a point isn't finite");
    let mut drawn = Path::new();
    h.draw(&mut drawn);
    must!(finite(drawn.parts().1), "{what}: drawn, a point isn't finite");
}

/// The font's paint graph walked node by node and lowered by the caller's hand, and a graph of the caller's lowered
/// too: numbers that aren't finite, glyphs past the font's, a clip box of the input's.
pub fn check_paint_tools(f: &Font, g: GlyphId, at: &Location, c: &Choices) {
    let ctm = transform(c.0[22]);
    let font_box = |g| f.colr_clip_box(g, at);
    if let Some(graph) = f.paint_graph(g, at, 0) {
        // A graph may name one paint from many places, so the walk stops at a bound rather than at every path.
        let (mut stack, mut walked) = (vec![graph.root()], 0);
        while let Some(r) = stack.pop() && walked < 1 << 16 {
            let _ = r.node();
            walked += 1;
            stack.extend(r.children());
        }
        lowered(f, at, g, &graph, ctm, &font_box, ctm.iter().all(|v| v.is_finite()));
    }
    let (k, h) = (c.0[23], any(c, 24));
    let solid = PaintNode::Solid { color: Rgba::opaque(9, 99, 199), foreground: k & 1 != 0 };
    let beyond = GlyphId(f.glyph_count().saturating_add(u32::from(k)));
    let painted = move |s: &mut PaintGraph| {
        s.with(solid, |_| {});
    };
    let mut graph = PaintGraph::new();
    graph.with(PaintNode::Layers, |layers| {
        layers.with(PaintNode::Composite { mode: Blend::from_colr(k) }, |mix| {
            mix.with(PaintNode::Glyph { glyph: g }, painted);
            mix.with(PaintNode::Rotate { angle: h, center: None }, |r| {
                r.with(PaintNode::ColrGlyph { glyph: g }, |c| {
                    c.with(PaintNode::Glyph { glyph: g }, painted);
                });
            });
        });
        layers.with(PaintNode::Scale { sx: h, sy: -h, center: None }, |s| {
            s.with(PaintNode::Glyph { glyph: beyond }, painted);
        });
        layers.with(PaintNode::Transform { matrix: transform(k) }, |t| {
            t.with(PaintNode::Glyph { glyph: GlyphId(u32::MAX) }, painted);
        });
    });
    let input_box = |_| Some(Bounds { x_min: -h, y_min: -100.0, x_max: h * 4.0, y_max: 900.0 });
    let finite = h.is_finite() && ctm.iter().chain(&transform(k)).all(|v| v.is_finite());
    lowered(f, at, g, &graph, ctm, &input_box, finite);
}

// A graph lowered with the font's outlines and the clip boxes given, the list it makes sound, and finite where all it
// was made of is; then a path, gradients and shapes the list holds pushed again and named, and the list cleared.
fn lowered(
    f: &Font, at: &Location, g: GlyphId, graph: &PaintGraph, ctm: [f64; 6], clip: &dyn Fn(GlyphId) -> Option<Bounds>,
    finite: bool,
) {
    let what = format!("glyph {g} lowered under {ctm:?}");
    let mut list = DisplayList::default();
    let mut outline = |g, pen: &mut dyn OutlinePen| f.outline(g, at, pen).is_ok();
    let drew = lower(g, graph, ctm, clip, &mut outline, Rgba::opaque(0, 0, 0), &mut list);
    must!(drew || list.is_empty(), "{what}: refused, it wrote {} ops", list.ops().len());
    check_ops(&list, &what, finite);
    let first = list.paths().next().map(|p| p.to_path());
    if let Some(path) = first {
        let id = list.push_path(path.view());
        // By bits, as a caller's numbers that aren't finite stay as given.
        let bits = |p: &[Point]| p.iter().map(|p| (p.x.to_bits(), p.y.to_bits())).collect::<Vec<_>>();
        let (verbs, points) = path.parts();
        let named = list.path(id).is_some_and(|p| p.parts().0 == verbs && bits(p.parts().1) == bits(points));
        must!(named, "{what}: a path pushed isn't the one named");
    }
    let gradients: Vec<_> = list.gradients().map(|r| (r.kind, r.stops.to_vec(), r.extend, r.transform)).collect();
    for (kind, stops, extend, transform) in &gradients {
        let id = list.push_gradient(Gradient::new(*kind, stops, *extend, *transform));
        let named = list.gradient(id).is_some_and(|r| r.stops == &stops[..]);
        must!(named, "{what}: a gradient pushed isn't the one named");
    }
    let clips: Vec<_> = list.ops().iter().filter_map(|op| match op {
        Op::PushClip { shapes } => Some(list.shapes(*shapes).to_vec()),
        _ => None,
    }).collect();
    for shapes in clips {
        let range = list.push_shapes(&shapes);
        let key = |s: &daegun::color::ClipShape| (s.path, s.rule, s.transform.map(f64::to_bits));
        let named = list.shapes(range).iter().map(key).eq(shapes.iter().map(key));
        must!(named, "{what}: shapes pushed aren't the ones named");
    }
    list.clear();
    must!(list.is_empty() && list.paths().len() == 0 && list.gradients().len() == 0, "{what}: cleared, it keeps some");
}

/// Stops resolved, gradients ramped and sampled, blends and composites, matrices inverted, colors faded, all from
/// the input's choices.
pub fn check_color_values(c: &Choices) {
    let unit = |i: usize| f32::from(c.0[21 + i % 5]) / 255.0;
    let (cb, cs) = ([unit(0), unit(1), unit(2)], [unit(3), unit(4), unit(0)]);
    // Two of COLR's composite modes, or a number past them.
    for mode in [c.0[21], c.0[21] >> 3] {
        let blend = Blend::from_colr(mode);
        let mixed = blend.blend(cb, cs);
        must!(mixed.iter().all(|x| (-1e-4..=1.0001).contains(x)), "{blend:?} blends {cb:?} and {cs:?} to {mixed:?}");
        let p = blend.composite(cs, unit(1), cb, unit(3));
        must!((0.0..=1.0001).contains(&p.alpha) && p.color.iter().all(|&x| x >= -1e-4 && x <= p.alpha + 1e-4),
              "{blend:?} composites {cs:?} over {cb:?} to {p:?}");
        let _ = blend.composite([f32::NAN, 2.0, -1.0], f32::INFINITY, cb, -0.5);
    }

    let t = transform(c.0[22]);
    let det = t[0] * t[3] - t[1] * t[2];
    if t.iter().all(|v| v.is_finite()) && det == 0.0 {
        must!(invert(&t).is_none(), "{t:?} folds the plane flat, yet inverts");
    }
    if let Some(inverse) = invert(&t) && det.abs() > 1e-3 && t.iter().all(|v| v.is_finite()) {
        let back = concat(&t, &inverse);
        must!(back.iter().zip(IDENTITY).all(|(a, b)| (a - b).abs() < 1e-6), "{t:?} by its inverse is {back:?}");
    }

    let color = Rgba::from([c.0[23], c.0[24], c.0[25], c.0[22]]);
    let by = any(c, 25);
    let faded = color.fade(by);
    must!(!(0.0..=1.0).contains(&by) || faded.a <= color.a, "{color:?} faded by {by} is {faded:?}");

    let offsets = [0.0, 0.5, 1.0, -0.25, 1.75, f32::NAN, 0.5, 0.25];
    let stops: Vec<Stop> = (0..usize::from(c.0[25] % 6))
        .map(|i| Stop { offset: offsets[(usize::from(c.0[21]) + i * 3) % 8], color: color.fade(i as f64 / 5.0) })
        .collect();
    let extend = [Extend::Pad, Extend::Repeat, Extend::Reflect][usize::from(c.0[25] >> 6) % 3];
    if let Stops::Many(many) = Stops::resolve(stops.clone(), extend) {
        must!(many.iter().all(|s| s.offset.is_finite()) && many.windows(2).all(|w| w[0].offset <= w[1].offset),
              "stops {stops:?} resolve out of order, {many:?}");
    }
    let h = any(c, 24) as f32;
    let kind = match c.0[23] % 3 {
        0 => GradientKind::Linear { x0: 0.0, y0: 0.0, x1: h, y1: 10.0 },
        1 => GradientKind::Radial { x0: 0.0, y0: 0.0, r0: h, x1: 5.0, y1: 5.0, r1: 20.0 },
        _ => GradientKind::Sweep { cx: 0.0, cy: 0.0, start_angle: h, end_angle: 270.0 },
    };
    let gradient = Gradient::new(kind, &stops, extend, t);
    for ramp in [Ramp::new(&gradient, &IDENTITY), Ramp::with_interpolation(&gradient, &t, Interpolation::LinearLight)] {
        for (dx, dy) in [(0.0, 0.0), (5.5, -3.0), (1e9, 1e9), (f64::NAN, 0.0)] {
            let _ = ramp.at(dx, dy);
        }
    }
}

/// Rectangles of these sizes packed into a page of the input's size, twice with a reset between: each placed whole
/// inside the page and clear of the others.
pub fn check_packing(c: &Choices, sizes: &[(usize, usize)]) {
    let side = |b: u8| [0, 1, 7, 64, 256, 4096, 1 << 20, usize::MAX][usize::from(b & 7)];
    let (w, h) = (side(c.0[25]), side(c.0[25] >> 3));
    let mut packer = ShelfPacker::new(w, h);
    for _ in 0..2 {
        let mut placed: Vec<Rect> = Vec::new();
        for &(rw, rh) in sizes {
            let Some(r) = packer.insert(rw, rh) else { continue };
            let inside = r.x.checked_add(r.w).is_some_and(|e| e <= w) && r.y.checked_add(r.h).is_some_and(|e| e <= h);
            must!(r.w == rw && r.h == rh && inside, "{rw}x{rh} placed as {r:?} in {w}x{h}");
            let apart = |o: &Rect| r.x + r.w <= o.x || o.x + o.w <= r.x || r.y + r.h <= o.y || o.y + o.h <= r.y;
            must!(placed.iter().all(apart), "{r:?} overlaps a rectangle placed before it");
            placed.push(r);
        }
        packer.reset();
    }
}

/// A subpixel filter's parts read back: as many weights as taps in each of three channels and none in a fourth,
/// all finite.
pub fn check_subpixel(l: &SubpixelLayout, what: &str) {
    let (tx, ty) = l.taps();
    for ch in 0..3 {
        if let Some(w) = l.weights(ch) {
            must!(w.len() == usize::from(tx) * usize::from(ty) && w.iter().all(|v| v.is_finite()),
                  "{what}: channel {ch} has {} weights for {tx}x{ty} taps", w.len());
        }
    }
    must!(l.weights(3).is_none(), "{what}: a fourth channel has weights");
    let _ = (l.oversample(), l.origin(), l.pad(), l.key(), l.is_grayscale());
}
