use crate::daecore::daetype::colr_v1::{ColorStop, Graph, Node, PaintGraph, Shape, Tree, Walk};
use super::{Blend, ClipShape, DisplayList, Extend, GradientKind, Mark, Op, Paint, PathId, Rgba, Stop};
use crate::daecore::daetype::outline::{FillRule, OutlinePen, Path};
use crate::daecore::daekit::draw::matrix::{concat, transform_box, Matrix, IDENTITY};
use crate::daecore::daekit::draw::{normalize_color_line, resolve_in_place, Resolved};
use crate::daecore::{Bounds, GlyphId};

// A level opens at most two layers (a composite), and only these 64 levels are lowered, so a scene nests at most 128
// deep, as the header promises: the depth a paint graph is walked to.
const MAX_DEPTH: usize = 64;

// Adds glyph `g`'s outline to the list once: None where it has none or draws nothing.
type Outline<'a> = dyn FnMut(u32, &mut DisplayList) -> Option<PathId> + 'a;

/// Lowers color glyph `glyph`'s graph into `out` under `ctm`, it and each color glyph it names clipped to its
/// `clip_box`, as [`Font::color_scene`](crate::Font::color_scene) does. `outline` draws a glyph's outline, false
/// where it can't. False, writing nothing, for a glyph the spec says not to draw: unbounded, with no clip box.
pub fn lower(
    glyph: GlyphId,
    graph: &PaintGraph,
    ctm: Matrix,
    clip_box: &dyn Fn(GlyphId) -> Option<Bounds>,
    outline: &mut dyn FnMut(GlyphId, &mut dyn OutlinePen) -> bool,
    foreground: Rgba,
    out: &mut DisplayList,
) -> bool {
    let clip_box = |g| clip_box(GlyphId(g));
    let outline = &mut |g, out: &mut DisplayList| out.draw_path(|pen| outline(GlyphId(g), pen));
    lower_graph(glyph.0, &mut Tree::new(graph), Tree::ROOT, ctm, &clip_box, outline, foreground, out)
}

// As `lower`, from the tree; `outline` adds a glyph's outline to the list itself.
pub(crate) fn lower_tree(
    glyph_id: u32,
    paint: &PaintGraph,
    clip_box: &dyn Fn(u32) -> Option<Bounds>,
    outline: &mut Outline<'_>,
    foreground: Rgba,
    out: &mut DisplayList,
) -> bool {
    lower_graph(glyph_id, &mut Tree::new(paint), Tree::ROOT, IDENTITY, clip_box, outline, foreground, out)
}

// As `lower_tree`, reading the graph in place from its root at `root`; false for a root that doesn't read. None where
// the walk ran out of budget or met an unbounded glyph, as what it left out follows a tree's order: lower the tree.
pub(crate) fn lower_walk(
    glyph_id: u32,
    walk: &mut Walk,
    root: usize,
    clip_box: &dyn Fn(u32) -> Option<Bounds>,
    outline: &mut Outline<'_>,
    foreground: Rgba,
    out: &mut DisplayList,
) -> Option<bool> {
    // A root that doesn't read is a color glyph that draws nothing, as any paint that doesn't read lowers.
    let layers = walk.reads(root).unwrap_or(0);
    // About a fill and an outline a layer, and the clip box's two ops and path. Half of Noto Color Emoji's scenes
    // draw no more than 28 verbs and 48 points a path so counted.
    let paths = layers + 1;
    out.reserve(layers + 2, paths, 32 * paths, 48 * paths);
    let drawn = lower_graph(glyph_id, walk, Some(root), IDENTITY, clip_box, outline, foreground, out);
    (drawn && !walk.exhausted()).then_some(true)
}

#[allow(clippy::too_many_arguments)]
fn lower_graph<G: Graph>(
    glyph_id: u32,
    graph: &mut G,
    root: G::Child,
    ctm: Matrix,
    clip_box: &dyn Fn(u32) -> Option<Bounds>,
    outline: &mut Outline<'_>,
    foreground: Rgba,
    out: &mut DisplayList,
) -> bool {
    let own = clip_box(glyph_id);
    if own.is_none() {
        if !bounded(graph, root, clip_box, 0) {
            return false;
        }
        graph.restart();
    }
    let mark = out.mark();
    let mut l = Lowering {
        graph, outline, clip_box, fg: foreground, out, interned: Interned::new(), boxes: Interned::new(), region: None,
    };
    match own {
        Some(b) => {
            let rect = l.out.add_path(|pen| rectangle(pen, b.corners()));
            l.region = Some((rect, ctm, false));
            l.push_clip(rect, ctm);
            l.lower_at(root, ctm, None, 0);
            l.pop_clip(mark);
        }
        None => {
            l.lower_at(root, ctm, None, 0);
            // A fill outside every glyph lowers into an empty path, made here a rectangle around all the
            // glyph's bounded content, which holds whatever such a fill can show in a bounded graph.
            if let Some((region, _, true)) = l.region
                && let Some(b) = content_bounds(l.out, mark, region)
            {
                l.out.redraw_path(region, |pen| rectangle(pen, b));
            }
        }
    }
    true
}

// Each glyph's path, made once. Searched while a scene names few glyphs, as most name a few dozen, and keyed past
// that: a paint graph can name tens of thousands.
pub(crate) struct Interned {
    len: usize,
    glyphs: [u32; FEW],
    paths: [Option<PathId>; FEW],
    many: alloc::collections::BTreeMap<u32, Option<PathId>>,
}

const FEW: usize = 64;

impl Interned {
    pub(crate) fn new() -> Self {
        Interned { len: 0, glyphs: [0; FEW], paths: [None; FEW], many: alloc::collections::BTreeMap::new() }
    }

    pub(crate) fn get(&self, gid: u32) -> Option<Option<PathId>> {
        match self.glyphs[..self.len].iter().position(|&g| g == gid) {
            Some(i) => Some(self.paths[i]),
            None => self.many.get(&gid).copied(),
        }
    }

    pub(crate) fn insert(&mut self, gid: u32, path: Option<PathId>) {
        if self.len < FEW {
            (self.glyphs[self.len], self.paths[self.len]) = (gid, path);
            self.len += 1;
        } else {
            self.many.insert(gid, path);
        }
    }
}

struct Lowering<'a, G: Graph> {
    graph: &'a mut G,
    outline: &'a mut Outline<'a>,
    clip_box: &'a dyn Fn(u32) -> Option<Bounds>,
    fg: Rgba,
    out: &'a mut DisplayList,
    interned: Interned,
    // Each referenced color glyph's clip box as a path, held once as outlines are.
    boxes: Interned,
    // Where a fill outside every PaintGlyph goes: the clip box, or a rectangle worked out at the end
    // (the flag), with the transform it is drawn under.
    region: Option<(PathId, Matrix, bool)>,
}

impl<G: Graph> Lowering<'_, G> {
    fn path(&mut self, gid: u32) -> Option<PathId> {
        if let Some(id) = self.interned.get(gid) {
            return id;
        }
        let id = (self.outline)(gid, self.out);
        self.interned.insert(gid, id);
        id
    }

    fn clip_path(&mut self, gid: u32) -> Option<PathId> {
        if let Some(id) = self.boxes.get(gid) {
            return id;
        }
        let id = (self.clip_box)(gid).map(|b| self.out.add_path(|pen| rectangle(pen, b.corners())));
        self.boxes.insert(gid, id);
        id
    }

    fn push_clip(&mut self, path: PathId, transform: Matrix) {
        let shapes = self.out.push_shapes(&[ClipShape { path, rule: FillRule::NonZero, transform }]);
        self.out.push(Op::PushClip { shapes });
    }

    // Closes the clip opened at `mark`, and takes it out where nothing inside it is filled.
    fn pop_clip(&mut self, mark: Mark) {
        if self.out.ops()[mark.ops..].iter().any(|o| matches!(o, Op::Fill { .. })) {
            self.out.push(Op::PopClip);
        } else {
            self.out.truncate(mark);
        }
    }

    // `clip` is the innermost PaintGlyph around this paint, with the transform its outline is under:
    // a fill below it fills that outline.
    fn lower_at(&mut self, at: G::Child, ctm: Matrix, clip: Option<(PathId, Matrix)>, depth: usize) {
        if depth >= MAX_DEPTH {
            return self.graph.skip(at);
        }
        let Some(node) = self.graph.open(at) else { return };
        let paint = leaf(&node, self.fg, ctm, self.out);
        let shape = node.shape();
        self.lower_shape(shape, paint, ctm, clip, depth);
        self.graph.close();
    }

    // `paint` is what a fill paints, None for one that paints nothing.
    fn lower_shape(
        &mut self,
        shape: Shape<G::Child, G::Layers>,
        paint: Option<Paint>,
        ctm: Matrix,
        clip: Option<(PathId, Matrix)>,
        depth: usize,
    ) {
        match shape {
            Shape::Transform(child, m) => self.lower_at(child, concat(&m, &ctm), clip, depth + 1),

            Shape::Layers(layers) => {
                for layer in self.graph.layers(layers) {
                    self.lower_at(layer, ctm, clip, depth + 1);
                }
            }

            Shape::Glyph(child, glyph_id) => self.lower_glyph(child, glyph_id, ctm, depth),

            Shape::ColrGlyph(child, glyph_id) => {
                let Some(rect) = self.clip_path(glyph_id) else { return self.lower_at(child, ctm, clip, depth + 1) };
                let mark = self.out.mark();
                self.push_clip(rect, ctm);
                self.lower_at(child, ctm, clip, depth + 1);
                self.pop_clip(mark);
            }

            Shape::Composite { source, mode: blend, backdrop } => {
                let mark = self.out.mark();
                self.out.push(Op::PushLayer { opacity: 1.0, blend: Blend::SrcOver });
                self.lower_at(backdrop, ctm, clip, depth + 1);

                // With one side empty the group is the other side as drawn or nothing, and needs no
                // layer: what a scene draws outside a layer is laid on with SrcOver, which associates.
                if self.out.ops().len() == mark.ops + 1 {
                    self.out.truncate(mark);
                    if keeps_source(blend) {
                        self.lower_at(source, ctm, clip, depth + 1);
                    } else {
                        self.graph.skip(source);
                    }
                    return;
                }
                let inner = self.out.mark();
                self.out.push(Op::PushLayer { opacity: 1.0, blend });
                self.lower_at(source, ctm, clip, depth + 1);
                if self.out.ops().len() == inner.ops + 1 {
                    self.out.truncate(inner);
                    if keeps_backdrop(blend) { self.out.remove(mark.ops) } else { self.out.truncate(mark) }
                    return;
                }
                self.out.push(Op::PopLayer);
                self.out.push(Op::PopLayer);
            }

            Shape::Fill => {
                let target = match clip {
                    Some(c) => Some(c),
                    None => self.region(),
                };
                if let (Some((path, transform)), Some(p)) = (target, paint) {
                    self.out.push(Op::Fill { path, paint: p, rule: FillRule::NonZero, transform });
                }
            }
        }
    }

    // A fill under transforms alone fills the outline directly, the fill carrying them; anything else under them is
    // lowered clipped to the outline, from the paint the transforms lead to.
    fn lower_glyph(&mut self, child: G::Child, glyph_id: u32, ctm: Matrix, depth: usize) {
        let Some(path) = self.path(glyph_id) else { return self.graph.skip(child) };
        let (mut at, mut node_ctm, mut node_depth, mut opened) = (child, ctm, depth + 1, 0);
        let (shape, paint) = loop {
            let Some(node) = self.graph.open(at) else { break (None, None) };
            opened += 1;
            match node.shape() {
                Shape::Transform(c, m) => (at, node_ctm, node_depth) = (c, concat(&m, &node_ctm), node_depth + 1),
                shape => break (Some(shape), leaf(&node, self.fg, node_ctm, self.out)),
            }
        };
        if let Some(p) = paint {
            self.out.push(Op::Fill { path, paint: p, rule: FillRule::NonZero, transform: ctm });
        } else if let Some(shape) = shape.filter(|s| !matches!(s, Shape::Fill)) {
            if node_depth < MAX_DEPTH {
                let mark = self.out.mark();
                self.push_clip(path, ctm);
                self.lower_shape(shape, None, node_ctm, Some((path, ctm)), node_depth);
                self.pop_clip(mark);
            } else {
                skip_below(self.graph, shape);
            }
        }
        (0..opened).for_each(|_| self.graph.close());
    }

    fn region(&mut self) -> Option<(PathId, Matrix)> {
        if self.region.is_none() {
            self.region = Some((self.out.push_path(Path::default().view()), IDENTITY, true));
        }
        self.region.map(|(path, m, _)| (path, m))
    }
}

// Spends on the children of a paint already open what reading them would.
fn skip_below<G: Graph>(graph: &mut G, shape: Shape<G::Child, G::Layers>) {
    match shape {
        Shape::Layers(layers) => graph.layers(layers).for_each(|layer| graph.skip(layer)),
        Shape::Glyph(child, _) | Shape::ColrGlyph(child, _) | Shape::Transform(child, _) => graph.skip(child),
        Shape::Composite { source, backdrop, .. } => {
            graph.skip(source);
            graph.skip(backdrop);
        }
        Shape::Fill => {}
    }
}

// The spec's boundedness: an outline or a clip box bounds what it clips, a fill bounds nothing, and a
// composite is bounded by its operands as its mode lets it be. Past the lowered depth nothing is drawn.
fn bounded<G: Graph>(graph: &mut G, at: G::Child, clip_box: &dyn Fn(u32) -> Option<Bounds>, depth: usize) -> bool {
    if depth >= MAX_DEPTH {
        return true;
    }
    let Some(node) = graph.open(at) else { return true };
    let bounded = match node.shape() {
        Shape::Transform(child, _) => bounded(graph, child, clip_box, depth + 1),
        Shape::Layers(layers) => graph.layers(layers).all(|layer| bounded(graph, layer, clip_box, depth + 1)),
        Shape::Glyph(..) => true,
        Shape::ColrGlyph(child, glyph_id) => clip_box(glyph_id).is_some() || bounded(graph, child, clip_box, depth + 1),
        Shape::Composite { source, mode, backdrop } => match mode {
            Blend::Clear => true,
            Blend::Src | Blend::SrcOut => bounded(graph, source, clip_box, depth + 1),
            Blend::Dest | Blend::DestOut => bounded(graph, backdrop, clip_box, depth + 1),
            Blend::SrcIn | Blend::DestIn => {
                bounded(graph, source, clip_box, depth + 1) || bounded(graph, backdrop, clip_box, depth + 1)
            }
            _ => bounded(graph, source, clip_box, depth + 1) && bounded(graph, backdrop, clip_box, depth + 1),
        },
        Shape::Fill => false,
    };
    graph.close();
    bounded
}

// The box around every outline drawn or clipped to since `mark`, other than `skip`, in the space
// the ops' transforms place them in.
fn content_bounds(out: &DisplayList, mark: Mark, skip: PathId) -> Option<[f64; 4]> {
    let mut acc: Option<[f64; 4]> = None;
    let mut add = |path: PathId, m: &Matrix| {
        let Some(b) = (path != skip).then(|| out.path(path)?.bounds()).flatten() else { return };
        let t = transform_box(m, [b.x_min, b.y_min, b.x_max, b.y_max]);
        let b = acc.get_or_insert(t);
        *b = [b[0].min(t[0]), b[1].min(t[1]), b[2].max(t[2]), b[3].max(t[3])];
    };
    for op in &out.ops()[mark.ops..] {
        match op {
            Op::Fill { path, transform, .. } => add(*path, transform),
            Op::PushClip { shapes } => out.shapes(*shapes).iter().for_each(|s| add(s.path, &s.transform)),
            _ => {}
        }
    }
    acc
}

// The box a color glyph paints in its own units: a fill covers the clips around it, carried as boxes through the
// transforms, an outline its points' box and a color glyph its clip box. None where it paints nothing or isn't drawn.
pub(crate) fn painted_box<G: Graph>(
    graph: &mut G,
    root: G::Child,
    clip_box: &dyn Fn(u32) -> Option<[i32; 4]>,
    outline_box: &mut dyn FnMut(u32) -> Option<[f64; 4]>,
) -> Option<[f64; 4]> {
    match (Covering { clip_box, outline_box }).cover(graph, root, IDENTITY, Cover::Everywhere, 0) {
        Cover::Within(b) => Some(b),
        Cover::Everywhere | Cover::Nothing => None,
    }
}

// The box around these outlines' points: a COLR version 0 glyph's, whose layers each fill their outline.
pub fn layers_box(
    glyphs: impl Iterator<Item = u32>, outline_box: &mut dyn FnMut(u32) -> Option<[f64; 4]>,
) -> Option<[f64; 4]> {
    match glyphs.fold(Cover::Nothing, |acc, g| acc.union(outline_box(g).map_or(Cover::Nothing, Cover::of))) {
        Cover::Within(b) => Some(b),
        Cover::Everywhere | Cover::Nothing => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cover {
    Everywhere,
    Within([f64; 4]),
    Nothing,
}

impl Cover {
    fn of([x0, y0, x1, y1]: [f64; 4]) -> Cover {
        if x0 < x1 && y0 < y1 { Cover::Within([x0, y0, x1, y1]) } else { Cover::Nothing }
    }

    fn union(self, other: Cover) -> Cover {
        match (self, other) {
            (Cover::Everywhere, _) | (_, Cover::Everywhere) => Cover::Everywhere,
            (Cover::Nothing, c) | (c, Cover::Nothing) => c,
            (Cover::Within(a), Cover::Within(b)) => {
                Cover::Within([a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])])
            }
        }
    }

    fn intersect(self, other: Cover) -> Cover {
        match (self, other) {
            (Cover::Nothing, _) | (_, Cover::Nothing) => Cover::Nothing,
            (Cover::Everywhere, c) | (c, Cover::Everywhere) => c,
            (Cover::Within(a), Cover::Within(b)) => {
                Cover::of([a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])])
            }
        }
    }
}

struct Covering<'a> {
    clip_box: &'a dyn Fn(u32) -> Option<[i32; 4]>,
    outline_box: &'a mut dyn FnMut(u32) -> Option<[f64; 4]>,
}

impl Covering<'_> {
    // Past the lowered depth nothing is drawn. Every fill covers its clip, one that paints nothing too: extents are
    // the geometry alone, the same whichever palette or colors a glyph is drawn in.
    fn cover<G: Graph>(&mut self, graph: &mut G, at: G::Child, ctm: Matrix, clip: Cover, depth: usize) -> Cover {
        if depth >= MAX_DEPTH {
            return Cover::Nothing;
        }
        let Some(node) = graph.open(at) else { return Cover::Nothing };
        let clipped =
            |b: Option<[f64; 4]>| clip.intersect(b.map_or(Cover::Nothing, |b| Cover::of(transform_box(&ctm, b))));
        let cover = match node.shape() {
            Shape::Transform(child, m) => self.cover(graph, child, concat(&m, &ctm), clip, depth + 1),
            Shape::Layers(layers) => graph.layers(layers).fold(Cover::Nothing, |acc, layer| {
                acc.union(self.cover(graph, layer, ctm, clip, depth + 1))
            }),
            Shape::Glyph(child, glyph_id) => {
                let clip = clipped((self.outline_box)(glyph_id));
                self.cover(graph, child, ctm, clip, depth + 1)
            }
            Shape::ColrGlyph(child, glyph_id) => {
                let clip = match (self.clip_box)(glyph_id) {
                    Some(b) => clipped(Some(b.map(f64::from))),
                    None => clip,
                };
                self.cover(graph, child, ctm, clip, depth + 1)
            }
            Shape::Composite { source, mode, backdrop } => {
                let backdrop = self.cover(graph, backdrop, ctm, clip, depth + 1);
                let source = self.cover(graph, source, ctm, clip, depth + 1);
                match mode {
                    Blend::Clear => Cover::Nothing,
                    Blend::Src | Blend::SrcOut => source,
                    Blend::Dest | Blend::DestOut => backdrop,
                    Blend::SrcIn | Blend::DestIn => backdrop.intersect(source),
                    _ => backdrop.union(source),
                }
            }
            Shape::Fill => clip,
        };
        graph.close();
        cover
    }
}

fn rectangle(pen: &mut dyn OutlinePen, [x0, y0, x1, y1]: [f64; 4]) {
    pen.move_to(x0 as f32, y0 as f32);
    pen.line_to(x1 as f32, y0 as f32);
    pen.line_to(x1 as f32, y1 as f32);
    pen.line_to(x0 as f32, y1 as f32);
    pen.close();
}

// What a mode leaves of the backdrop under an empty source, and of the source over an empty backdrop:
// all of it unless blend.rs gives it a factor of 0 there, when it leaves nothing.
fn keeps_backdrop(mode: Blend) -> bool {
    !matches!(mode, Blend::Clear | Blend::Src | Blend::SrcIn | Blend::DestIn | Blend::SrcOut | Blend::DestAtop)
}

fn keeps_source(mode: Blend) -> bool {
    !matches!(mode, Blend::Clear | Blend::Dest | Blend::SrcIn | Blend::DestIn | Blend::DestOut | Blend::SrcAtop)
}

// A gradient's stops go into `out`, as the fill it paints follows.
fn leaf<C, L>(node: &Node<'_, C, L>, fg: Rgba, ctm: Matrix, out: &mut DisplayList) -> Option<Paint> {
    match *node {
        Node::Solid { color, foreground } => Some(Paint::Solid(painted(color, foreground, fg))),

        Node::LinearGradient { extend, stops, x0, y0, x1, y1, x2, y2 } => {
            let (px, py) = project((x0, y0), (x1, y1), (x2, y2))?;
            gradient(
                GradientKind::Linear { x0: x0 as f32, y0: y0 as f32, x1: px as f32, y1: py as f32 },
                stops, extend, fg, ctm, out,
            )
        }

        Node::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 } => gradient(
            GradientKind::Radial {
                x0: x0 as f32, y0: y0 as f32, r0: r0 as f32,
                x1: x1 as f32, y1: y1 as f32, r1: r1 as f32,
            },
            stops, extend, fg, ctm, out,
        ),

        Node::SweepGradient { extend, stops, cx, cy, start_angle, end_angle } => gradient(
            GradientKind::Sweep {
                cx: cx as f32, cy: cy as f32,
                start_angle: start_angle as f32, end_angle: end_angle as f32,
            },
            stops, extend, fg, ctm, out,
        ),

        _ => None,
    }
}

fn gradient(
    kind: GradientKind,
    stops: &[ColorStop],
    extend: Extend,
    fg: Rgba,
    ctm: Matrix,
    out: &mut DisplayList,
) -> Option<Paint> {
    let start = out.stops.len();
    out.stops.extend(stops.iter().map(|s| Stop { offset: s.offset as f32, color: painted(s.color, s.foreground, fg) }));
    let line = &mut out.stops[start..];
    let kind = normalize_color_line(kind, line);
    let paint = match resolve_in_place(line, extend) {
        Resolved::Nothing => None,
        Resolved::Solid(c) => Some(Paint::Solid(c)),
        Resolved::Many => return Some(Paint::Gradient(out.add_gradient(kind, start, extend, ctm))),
    };
    out.stops.truncate(start);
    paint
}

// A paint's or a stop's color as drawn: the palette's opaque, or the text's, at its own alpha scaled by `color`'s.
fn painted(color: Rgba, foreground: bool, fg: Rgba) -> Rgba {
    let base = if foreground { fg } else { Rgba { a: 255, ..color } };
    Rgba { a: scale(base.a, color.a), ..base }
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
#[path = "../../../tests/unit/daetype/paint/colr.rs"]
mod tests;
