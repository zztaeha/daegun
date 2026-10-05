use alloc::vec::Vec;

use crate::daecore::daetype::colr_v1::{rotate_matrix, scale_matrix, skew_matrix, ColorStop, Paint as Colr};
use super::{
    resolve_stops, Blend, ClipShape, DisplayList, Extend, Gradient, GradientKind, Op, Paint, PathId,
    Rgba, Stop, Stops,
};
use crate::daecore::daetype::outline::{FillRule, OutlinePen, Path};
use crate::daecore::daemachine::daemath::matrix::{concat, Matrix, IDENTITY};
use crate::daecore::daemachine::daemath::normalize_color_line;

// A level opens at most two layers (a composite), and only these 64 levels are lowered, so a scene nests
// at most 128 deep, as the header promises, whatever graph it is handed.
const MAX_DEPTH: usize = 64;

// Lowers a color glyph into `out`, clipped to its clip box if any. False, writing nothing, for a
// glyph the spec says not to draw: an unbounded graph with no clip box.
pub fn lower(
    paint: &Colr,
    ctm: Matrix,
    clip_box: Option<[i32; 4]>,
    outline: &mut dyn FnMut(u16) -> Option<Path>,
    foreground: Rgba,
    out: &mut DisplayList,
) -> bool {
    if clip_box.is_none() && !bounded(paint, 0) {
        return false;
    }
    let mark = out.ops().len();
    let mut l = Lowering { outline, fg: foreground, out, interned: Interned::new(), region: None };
    match clip_box {
        Some(b) => {
            let rect = l.out.push_path(rectangle([b[0], b[1], b[2], b[3]].map(f64::from)));
            l.region = Some((rect, ctm, false));
            l.out.push(Op::PushClip { shapes: alloc::vec![ClipShape { path: rect, rule: FillRule::NonZero, transform: ctm }] });
            l.lower_at(paint, ctm, None, 0);
            l.out.push(Op::PopClip);
            if !l.out.ops()[mark..].iter().any(|o| matches!(o, Op::Fill { .. })) {
                l.out.truncate(mark);
            }
        }
        None => {
            l.lower_at(paint, ctm, None, 0);
            // A fill outside every glyph lowers into an empty path, made here a rectangle around all the
            // glyph's bounded content, which holds whatever such a fill can show in a bounded graph.
            if let Some((region, _, true)) = l.region {
                let bounds = content_bounds(l.out, mark, region);
                if let (Some(b), Some(path)) = (bounds, l.out.path_mut(region)) {
                    *path = rectangle(b);
                }
            }
        }
    }
    true
}

// Keyed rather than searched: a paint graph can name tens of thousands of glyphs.
type Interned = alloc::collections::BTreeMap<u16, Option<PathId>>;

struct Lowering<'a> {
    outline: &'a mut dyn FnMut(u16) -> Option<Path>,
    fg: Rgba,
    out: &'a mut DisplayList,
    interned: Interned,
    // Where a fill outside every PaintGlyph goes: the clip box, or a rectangle worked out at the end
    // (the flag), with the transform it is drawn under.
    region: Option<(PathId, Matrix, bool)>,
}

impl Lowering<'_> {
    fn path(&mut self, gid: u16) -> Option<PathId> {
        if let Some(&id) = self.interned.get(&gid) {
            return id;
        }
        let id = match (self.outline)(gid) {
            Some(path) if !path.is_empty() => Some(self.out.push_path(path)),
            _ => None,
        };
        self.interned.insert(gid, id);
        id
    }

    // `clip` is the innermost PaintGlyph around this paint, with the transform its outline is under:
    // a fill below it fills that outline.
    fn lower_at(&mut self, paint: &Colr, ctm: Matrix, clip: Option<(PathId, Matrix)>, depth: usize) {
        if depth >= MAX_DEPTH {
            return;
        }
        if let Some((child, m)) = transform_of(paint) {
            return self.lower_at(child, concat(&m, &ctm), clip, depth + 1);
        }
        match paint {
            Colr::Layers(children) => {
                for c in children {
                    self.lower_at(c, ctm, clip, depth + 1);
                }
            }

            Colr::Glyph { child, glyph_id } => {
                let Some(path) = self.path(*glyph_id) else { return };
                // A fill under transforms alone fills the outline directly, the fill carrying them.
                let (mut node, mut node_ctm) = (&**child, ctm);
                while let Some((c, m)) = transform_of(node) {
                    (node, node_ctm) = (c, concat(&m, &node_ctm));
                }
                if let Some(p) = leaf(node, self.fg, node_ctm) {
                    self.out.push(Op::Fill { path, paint: p, rule: FillRule::NonZero, transform: ctm });
                    return;
                }
                let mark = self.out.ops().len();
                self.out.push(Op::PushClip {
                    shapes: alloc::vec![ClipShape { path, rule: FillRule::NonZero, transform: ctm }],
                });
                self.lower_at(child, ctm, Some((path, ctm)), depth + 1);
                self.out.push(Op::PopClip);
                if !self.out.ops()[mark..].iter().any(|o| matches!(o, Op::Fill { .. })) {
                    self.out.truncate(mark);
                }
            }

            Colr::ColrGlyph { child, .. } => self.lower_at(child, ctm, clip, depth + 1),

            Colr::Composite { source, mode, backdrop } => {
                let blend = Blend::from_colr(*mode);
                let mark = self.out.ops().len();
                self.out.push(Op::PushLayer { opacity: 1.0, blend: Blend::SrcOver });
                self.lower_at(backdrop, ctm, clip, depth + 1);

                // With one side empty the group is the other side as drawn or nothing, and needs no
                // layer: what a scene draws outside a layer is laid on with SrcOver, which associates.
                if self.out.ops().len() == mark + 1 {
                    self.out.truncate(mark);
                    if keeps_source(blend) {
                        self.lower_at(source, ctm, clip, depth + 1);
                    }
                    return;
                }
                let inner = self.out.ops().len();
                self.out.push(Op::PushLayer { opacity: 1.0, blend });
                self.lower_at(source, ctm, clip, depth + 1);
                if self.out.ops().len() == inner + 1 {
                    self.out.truncate(inner);
                    if keeps_backdrop(blend) { self.out.remove(mark) } else { self.out.truncate(mark) }
                    return;
                }
                self.out.push(Op::PopLayer);
                self.out.push(Op::PopLayer);
            }

            _ => {
                let target = match clip {
                    Some(c) => Some(c),
                    None => self.region(),
                };
                if let (Some((path, transform)), Some(p)) = (target, leaf(paint, self.fg, ctm)) {
                    self.out.push(Op::Fill { path, paint: p, rule: FillRule::NonZero, transform });
                }
            }
        }
    }

    fn region(&mut self) -> Option<(PathId, Matrix)> {
        if self.region.is_none() {
            self.region = Some((self.out.push_path(Path::default()), IDENTITY, true));
        }
        self.region.map(|(path, m, _)| (path, m))
    }
}

// A transforming paint's child and its own transform, in the order `concat` takes them.
fn transform_of(paint: &Colr) -> Option<(&Colr, Matrix)> {
    Some(match paint {
        Colr::Transform { child, matrix } => (child, *matrix),
        Colr::Translate { child, dx, dy } => (child, [1.0, 0.0, 0.0, 1.0, *dx, *dy]),
        Colr::Scale { child, sx, sy, center } => (child, scale_matrix(*sx, *sy, *center)),
        Colr::ScaleUniform { child, s, center } => (child, scale_matrix(*s, *s, *center)),
        Colr::Rotate { child, angle, center } => (child, rotate_matrix(*angle, *center)),
        Colr::Skew { child, x_angle, y_angle, center } => (child, skew_matrix(*x_angle, *y_angle, *center)),
        _ => return None,
    })
}

// The spec's boundedness: an outline bounds what it clips, a fill bounds nothing, and a composite
// is bounded by its operands as its mode lets it be. Past the lowered depth nothing is drawn.
fn bounded(paint: &Colr, depth: usize) -> bool {
    if depth >= MAX_DEPTH {
        return true;
    }
    if let Some((child, _)) = transform_of(paint) {
        return bounded(child, depth + 1);
    }
    match paint {
        Colr::Layers(children) => children.iter().all(|c| bounded(c, depth + 1)),
        Colr::Glyph { .. } => true,
        Colr::ColrGlyph { child, .. } => bounded(child, depth + 1),
        Colr::Composite { source, mode, backdrop } => {
            let source = || bounded(source, depth + 1);
            let backdrop = || bounded(backdrop, depth + 1);
            match Blend::from_colr(*mode) {
                Blend::Clear => true,
                Blend::Src | Blend::SrcOut => source(),
                Blend::Dest | Blend::DestOut => backdrop(),
                Blend::SrcIn | Blend::DestIn => source() || backdrop(),
                _ => source() && backdrop(),
            }
        }
        _ => false,
    }
}

// The box around every outline drawn or clipped to since `mark`, other than `skip`, in the space
// the ops' transforms place them in.
fn content_bounds(out: &DisplayList, mark: usize, skip: PathId) -> Option<[f64; 4]> {
    let mut acc: Option<[f64; 4]> = None;
    let mut add = |path: PathId, m: &Matrix| {
        let Some((x0, y0, x1, y1)) = (path != skip).then(|| out.path(path)?.bounds()).flatten() else { return };
        for (x, y) in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            let (px, py) = (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
            let b = acc.get_or_insert([px, py, px, py]);
            *b = [b[0].min(px), b[1].min(py), b[2].max(px), b[3].max(py)];
        }
    };
    for op in &out.ops()[mark..] {
        match op {
            Op::Fill { path, transform, .. } => add(*path, transform),
            Op::PushClip { shapes } => shapes.iter().for_each(|s| add(s.path, &s.transform)),
            _ => {}
        }
    }
    acc
}

fn rectangle([x0, y0, x1, y1]: [f64; 4]) -> Path {
    let mut p = Path::default();
    p.move_to(x0 as f32, y0 as f32);
    p.line_to(x1 as f32, y0 as f32);
    p.line_to(x1 as f32, y1 as f32);
    p.line_to(x0 as f32, y1 as f32);
    p.close();
    p
}

// What a mode leaves of the backdrop under an empty source, and of the source over an empty backdrop:
// all of it unless blend.rs gives it a factor of 0 there, when it leaves nothing.
fn keeps_backdrop(mode: Blend) -> bool {
    !matches!(mode, Blend::Clear | Blend::Src | Blend::SrcIn | Blend::DestIn | Blend::SrcOut | Blend::DestAtop)
}

fn keeps_source(mode: Blend) -> bool {
    !matches!(mode, Blend::Clear | Blend::Dest | Blend::SrcIn | Blend::DestIn | Blend::DestOut | Blend::SrcAtop)
}

fn leaf(paint: &Colr, fg: Rgba, ctm: Matrix) -> Option<Paint> {
    match paint {
        Colr::Solid { is_foreground, r, g, b, alpha } => {
            let base = if *is_foreground { fg } else { Rgba::opaque(*r, *g, *b) };
            Some(Paint::Solid(Rgba { a: scale(base.a, *alpha), ..base }))
        }

        Colr::LinearGradient { extend, stops, x0, y0, x1, y1, x2, y2 } => {
            let (px, py) = project((*x0, *y0), (*x1, *y1), (*x2, *y2))?;
            gradient(
                GradientKind::Linear { x0: *x0 as f32, y0: *y0 as f32, x1: px as f32, y1: py as f32 },
                stops, *extend, fg, ctm,
            )
        }

        Colr::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 } => gradient(
            GradientKind::Radial {
                x0: *x0 as f32, y0: *y0 as f32, r0: *r0 as f32,
                x1: *x1 as f32, y1: *y1 as f32, r1: *r1 as f32,
            },
            stops, *extend, fg, ctm,
        ),

        Colr::SweepGradient { extend, stops, cx, cy, start_angle, end_angle } => gradient(
            GradientKind::Sweep {
                cx: *cx as f32, cy: *cy as f32,
                start_angle: *start_angle as f32, end_angle: *end_angle as f32,
            },
            stops, *extend, fg, ctm,
        ),

        _ => None,
    }
}

fn gradient(
    kind: GradientKind,
    stops: &[ColorStop],
    extend: u8,
    fg: Rgba,
    ctm: Matrix,
) -> Option<Paint> {
    let converted: Vec<Stop> = stops
        .iter()
        .map(|s| {
            let base = if s.is_foreground { fg } else { Rgba::opaque(s.r, s.g, s.b) };
            Stop { offset: s.offset as f32, color: Rgba { a: scale(base.a, s.alpha), ..base } }
        })
        .collect();
    let (kind, converted) = normalize_color_line(kind, converted);
    match resolve_stops(converted) {
        Stops::Nothing => None,
        Stops::Solid(c) => Some(Paint::Solid(c)),
        Stops::Many(stops) => {
            Some(Paint::Gradient(Gradient { kind, stops, extend: spread(extend), transform: ctm }))
        }
    }
}

// COLR's extend is 0 pad, 1 repeat, 2 reflect, a different order from daemath's `Extend`, which
// follows SVG's `spreadMethod`: casting one to the other would swap repeat and reflect.
fn spread(extend: u8) -> Extend {
    match extend {
        1 => Extend::Repeat,
        2 => Extend::Reflect,
        _ => Extend::Pad,
    }
}

fn scale(a: u8, by: u8) -> u8 {
    ((u16::from(a) * u16::from(by) + 127) / 255) as u8
}

// A COLR linear gradient is three points, not two: `p2` is a rotation point, and the spec's own
// reduction is to project p0->p1 onto the line through p0 perpendicular to p0p2.
fn project(p0: (f64, f64), p1: (f64, f64), p2: (f64, f64)) -> Option<(f64, f64)> {
    let v = (p1.0 - p0.0, p1.1 - p0.1);
    let r = (p2.0 - p0.0, p2.1 - p0.1);
    if (v.0 == 0.0 && v.1 == 0.0) || (r.0 == 0.0 && r.1 == 0.0) {
        return None;
    }
    let n = (-r.1, r.0);
    let denom = n.0 * n.0 + n.1 * n.1;
    let t = (v.0 * n.0 + v.1 * n.1) / denom;
    let p3 = (p0.0 + t * n.0, p0.1 + t * n.1);
    let len = (p3.0 - p0.0).abs() + (p3.1 - p0.1).abs();
    (len > 1e-12).then_some(p3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daecore::daemachine::daemath::blend::composite;

    // As blend.rs composites each mode: an empty source leaves the backdrop or nothing, and a source
    // over an empty backdrop leaves itself or nothing.
    #[test]
    fn what_a_mode_keeps_is_what_it_composites() {
        let (color, alpha) = ([0.2f32, 0.5, 0.9], 0.6f32);
        let alone = (color.map(|v| v * alpha), alpha);
        for mode in (0..28).map(Blend::from_colr) {
            let under_empty = composite(mode, [0.0; 3], 0.0, color, alpha);
            let over_empty = composite(mode, color, alpha, [0.0; 3], 0.0);
            for (kept, result, what) in [(keeps_backdrop(mode), under_empty, "backdrop"), (keeps_source(mode), over_empty, "source")] {
                let want = if kept { alone } else { ([0.0; 3], 0.0) };
                assert_eq!(result, want, "{mode:?} keeps the {what}: {kept}");
            }
        }
    }
}
