use alloc::vec::Vec;
use super::super::decoder::{
    f2dot14, fixed, read_i16_be, read_i32_be, read_offset24, read_u16_be, read_u24_be, read_u32_be,
};
use super::color_line::parse_color_line;
use super::varfield::{decode_angle, resolve_var_angle, resolve_var_field, VarField};
use super::{rotate_matrix, scale_matrix, skew_matrix, Colrv1Ctx, PaintBudget};
use crate::daecore::daekit::draw::matrix::Matrix;
use crate::daecore::daekit::draw::{Blend, Extend, Rgba};
use crate::daecore::GlyphId;

/// One stop of a COLR gradient's color line. Laid out as C reads it.
#[non_exhaustive]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorStop {
    /// Where along the line the stop lies: 0 at its start, 1 at its end, and past either for a stop beyond them.
    pub offset:     f64,
    /// The palette's color, its alpha scaled by the stop's; for a foreground stop, black at that alpha.
    pub color:      Rgba,
    /// Whether the stop takes the text's color, its alpha scaled by `color`'s, in place of `color`.
    pub foreground: bool,
}

/// One paint of a COLR version 1 graph as a [`PaintVisitor`] meets it: its own fields, without the paints below it.
/// Positions are in font units, y up, and angles in degrees counterclockwise.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaintNode<'s> {
    /// Its children painted in turn, bottom to top.
    Layers,
    /// Its child painted inside a glyph's outline.
    Glyph {
        /// The glyph whose outline clips the child.
        glyph: GlyphId,
    },
    /// Another color glyph's graph, which is its child.
    ColrGlyph {
        /// The color glyph.
        glyph: GlyphId,
    },
    /// One color over the whole clip.
    Solid {
        /// The palette's color, its alpha scaled by the paint's; for a foreground paint, black at that alpha.
        color: Rgba,
        /// Whether the paint takes the text's color, its alpha scaled by `color`'s, in place of `color`.
        foreground: bool,
    },
    /// A linear gradient from p0 to p1, its lines of one color parallel to p0–p2.
    LinearGradient {
        /// How the color line goes on past its first and last stops.
        extend: Extend,
        /// The color line's stops, in the font's order.
        stops: &'s [ColorStop],
        /// p0's x, where the line starts.
        x0: f64,
        /// p0's y.
        y0: f64,
        /// p1's x, where the line ends.
        x1: f64,
        /// p1's y.
        y1: f64,
        /// p2's x, the point that sets the gradient's rotation.
        x2: f64,
        /// p2's y.
        y2: f64,
    },
    /// A radial gradient between two circles, the line starting on the first.
    RadialGradient {
        /// How the color line goes on past its first and last stops.
        extend: Extend,
        /// The color line's stops, in the font's order.
        stops: &'s [ColorStop],
        /// The first circle's center x.
        x0: f64,
        /// The first circle's center y.
        y0: f64,
        /// The first circle's radius.
        r0: f64,
        /// The second circle's center x.
        x1: f64,
        /// The second circle's center y.
        y1: f64,
        /// The second circle's radius.
        r1: f64,
    },
    /// A sweep gradient around a center, the line running from one angle to another.
    SweepGradient {
        /// How the color line goes on past its first and last stops.
        extend: Extend,
        /// The color line's stops, in the font's order.
        stops: &'s [ColorStop],
        /// The center's x.
        cx: f64,
        /// The center's y.
        cy: f64,
        /// Where the line starts, in degrees.
        start_angle: f64,
        /// Where the line ends, in degrees.
        end_angle: f64,
    },
    /// Its child transformed.
    Transform {
        /// A 2×3 matrix `[xx, yx, xy, yy, dx, dy]`, its offsets in font units.
        matrix: [f64; 6],
    },
    /// Its child moved.
    Translate {
        /// The move along x.
        dx: f64,
        /// The move along y.
        dy: f64,
    },
    /// Its child scaled.
    Scale {
        /// The scale along x.
        sx: f64,
        /// The scale along y.
        sy: f64,
        /// The point the scale keeps fixed; the origin where None.
        center: Option<Center>,
    },
    /// Its child scaled the same both ways.
    ScaleUniform {
        /// The scale.
        s: f64,
        /// The point the scale keeps fixed; the origin where None.
        center: Option<Center>,
    },
    /// Its child rotated.
    Rotate {
        /// The angle, in degrees counterclockwise.
        angle: f64,
        /// The point the rotation turns around; the origin where None.
        center: Option<Center>,
    },
    /// Its child skewed.
    Skew {
        /// The skew in the direction of the x axis, in degrees.
        x_angle: f64,
        /// The skew in the direction of the y axis, in degrees.
        y_angle: f64,
        /// The point the skew keeps fixed; the origin where None.
        center: Option<Center>,
    },
    /// Its first child, the source, composited onto its second, the backdrop.
    Composite {
        /// How the source and the backdrop combine.
        mode: Blend,
    },
}

/// The point a transforming paint keeps fixed, in font units, y up.
#[expect(clippy::exhaustive_structs, reason = "a plain value: its fields are all it is")]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Center {
    /// Across.
    pub x: f64,
    /// Up.
    pub y: f64,
}

/// A paint graph walked as a [`PaintGraph`] holds it, with no tree built: each paint entered, then its children in
/// the graph's order (layers in turn, a composite's source before its backdrop), then left.
///
/// # Examples
///
/// A visitor finding how deep a color glyph's paints nest:
///
/// ```
/// use daegun::color::{PaintNode, PaintVisitor};
/// use daegun::{GlyphId, Location};
///
/// #[derive(Default)]
/// struct Depth {
///     now: usize,
///     deepest: usize,
/// }
///
/// impl PaintVisitor for Depth {
///     fn enter(&mut self, _node: PaintNode<'_>) {
///         self.now += 1;
///         self.deepest = self.deepest.max(self.now);
///     }
///     fn leave(&mut self) {
///         self.now -= 1;
///     }
/// }
///
/// # fn main() -> daegun::Result<()> {
/// # let path = concat!(
/// #     env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/colr-v1-test-glyphs/test_glyphs_variable.ttf"
/// # );
/// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
/// # let font = daegun::Font::from_bytes(&bytes)?;
/// let mut depth = Depth::default();
/// assert!(font.visit_paint_graph(GlyphId(10), &Location::default(), 0, &mut depth));
/// assert!(depth.deepest > 1 && depth.now == 0);
/// # Ok(())
/// # }
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a paint visitor",
    note = "implement `PaintVisitor` for it, or take the graph whole with `Font::paint_graph`"
)]
pub trait PaintVisitor {
    /// A paint, whose children come next, until the `leave` that pairs with this.
    fn enter(&mut self, node: PaintNode<'_>);
    /// The last paint entered and not yet left, its children all met.
    fn leave(&mut self);
}

impl<V: PaintVisitor + ?Sized> PaintVisitor for &mut V {
    #[inline]
    fn enter(&mut self, node: PaintNode<'_>) {
        (**self).enter(node);
    }
    #[inline]
    fn leave(&mut self) {
        (**self).leave();
    }
}

/// A color glyph's paint graph, as [`Font::paint_graph`](crate::Font::paint_graph) reads it. It is a
/// [`PaintVisitor`] itself, so a graph is built by hand by entering and leaving its paints, the first the root.
// Its paints lie in one list, in the order a visitor meets them, each with where the paints below it end, and its
// gradients' stops in another.
#[derive(Clone, PartialEq)]
#[doc(alias("Paint"))]
pub struct PaintGraph {
    nodes: Vec<Entry>,
    stops: Vec<ColorStop>,
    // The innermost paint entered and not yet left, `NONE` where none is.
    open: usize,
}

#[derive(Clone, Copy, PartialEq)]
struct Entry {
    // A gradient's stops left out: they lie in `stops`, `stop_count` of them from `first_stop`.
    node: PaintNode<'static>,
    first_stop: usize,
    stop_count: usize,
    // While the paint is open, the paint it lies in; once it's left, the index past the last paint below it.
    end: usize,
}

const NONE: usize = usize::MAX;

impl PaintGraph {
    /// An empty graph, which reads as empty layers.
    // In the lists the thread's last dropped one held.
    pub fn new() -> PaintGraph {
        #[cfg(feature = "std")]
        let (nodes, stops) = SPARE.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_default();
        #[cfg(not(feature = "std"))]
        let (nodes, stops) = (Vec::new(), Vec::new());
        PaintGraph { nodes, stops, open: NONE }
    }

    /// Paint `node` entered, the paints below it built by `below`, then left: a graph built by hand.
    pub fn with(&mut self, node: PaintNode<'_>, below: impl FnOnce(&mut PaintGraph)) -> &mut PaintGraph {
        self.enter(node);
        below(self);
        self.leave();
        self
    }

    /// The first paint entered; empty layers for a graph with none.
    pub const fn root(&self) -> PaintRef<'_> {
        PaintRef { paint: self, at: 0 }
    }

    // The index past the paints below paint `i`, or past every paint for one never left.
    fn end_of(&self, i: usize) -> usize {
        match self.nodes.get(i) {
            Some(e) if e.end > i && e.end <= self.nodes.len() => e.end,
            _ => self.nodes.len(),
        }
    }
}

impl Default for PaintGraph {
    fn default() -> PaintGraph {
        PaintGraph::new()
    }
}

impl PaintVisitor for PaintGraph {
    // With `leave`, inlined into each of the walk's arms, where the paint's kind is known: out of line, each paint
    // went through the stack and two matches on its kind, and the static COLR row took 13% more cycles.
    #[inline(always)]
    fn enter(&mut self, node: PaintNode<'_>) {
        let stops = node.stops();
        let (first_stop, stop_count) = (self.stops.len(), stops.len());
        let entry = Entry { node: node.with_stops(&[]), first_stop, stop_count, end: self.open };
        self.stops.extend_from_slice(stops);
        self.open = self.nodes.len();
        self.nodes.push(entry);
    }

    // A leave with no paint open is passed by.
    #[inline(always)]
    fn leave(&mut self) {
        let len = self.nodes.len();
        if let Some(entry) = self.nodes.get_mut(self.open) {
            self.open = core::mem::replace(&mut entry.end, len);
        }
    }
}

impl core::fmt::Debug for PaintGraph {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.root().fmt(f)
    }
}

// The lists to the thread's spare, for its next graph; a graph past `SPARE_BYTES` drops its own, as does a build
// checking C's use of freed handles, whose nodes lie in them.
impl Drop for PaintGraph {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        if cfg!(not(daegun_no_spares))
            && crate::daecore::daekit::base::sync::heap_of(&self.nodes)
            + crate::daecore::daekit::base::sync::heap_of(&self.stops)
            <= SPARE_BYTES
        {
            let (mut nodes, mut stops) = (core::mem::take(&mut self.nodes), core::mem::take(&mut self.stops));
            nodes.clear();
            stops.clear();
            let _ = SPARE.try_with(|spare| spare.set(Some((nodes, stops))));
        }
    }
}

#[cfg(feature = "std")]
type Lists = (Vec<Entry>, Vec<ColorStop>);

#[cfg(feature = "std")]
std::thread_local! {
    static SPARE: core::cell::Cell<Option<Lists>> = const { core::cell::Cell::new(None) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE.try_with(core::cell::Cell::take);
}

#[cfg(feature = "std")]
const SPARE_BYTES: usize = 64 << 10;

/// One paint of a [`PaintGraph`]: its own fields, and the paints below it in the graph's order (layers in turn, a
/// composite's source then its backdrop).
#[derive(Clone, Copy)]
pub struct PaintRef<'a> {
    paint: &'a PaintGraph,
    at: usize,
}

impl<'a> PaintRef<'a> {
    /// The paint's own fields.
    pub fn node(&self) -> PaintNode<'a> {
        match self.paint.nodes.get(self.at) {
            Some(e) => e.node.with_stops(&self.paint.stops[e.first_stop..][..e.stop_count]),
            None => PaintNode::Layers,
        }
    }

    /// The paints directly below it, in order.
    pub fn children(&self) -> impl core::iter::FusedIterator<Item = PaintRef<'a>> + Clone + use<'a> {
        let (paint, end) = (self.paint, self.paint.end_of(self.at));
        let first = Some(self.at.wrapping_add(1)).filter(|&c| c < end);
        let next = move |&c: &usize| Some(paint.end_of(c)).filter(|&n| n < end);
        core::iter::successors(first, next).map(move |at| PaintRef { paint, at })
    }
}

// Printed as nested paints derive it: each paint's fields with its children in place, a missing one as empty layers.
impl core::fmt::Debug for PaintRef<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        struct Below<'a>(Option<PaintRef<'a>>);
        impl core::fmt::Debug for Below<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self.0 {
                    Some(paint) => paint.fmt(f),
                    None => f.debug_tuple("Layers").field(&[(); 0]).finish(),
                }
            }
        }
        struct List<'a>(PaintRef<'a>);
        impl core::fmt::Debug for List<'_> {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.debug_list().entries(self.0.children()).finish()
            }
        }
        let mut children = self.children();
        let mut child = || Below(children.next());
        match self.node() {
            PaintNode::Layers => f.debug_tuple("Layers").field(&List(*self)).finish(),
            PaintNode::Glyph { glyph } => {
                f.debug_struct("Glyph").field("child", &child()).field("glyph", &glyph).finish()
            }
            PaintNode::ColrGlyph { glyph } => {
                f.debug_struct("ColrGlyph").field("glyph", &glyph).field("child", &child()).finish()
            }
            PaintNode::Solid { color, foreground } => {
                f.debug_struct("Solid").field("color", &color).field("foreground", &foreground).finish()
            }
            PaintNode::LinearGradient { extend, stops, x0, y0, x1, y1, x2, y2 } => f
                .debug_struct("LinearGradient")
                .field("extend", &extend)
                .field("stops", &stops)
                .field("x0", &x0)
                .field("y0", &y0)
                .field("x1", &x1)
                .field("y1", &y1)
                .field("x2", &x2)
                .field("y2", &y2)
                .finish(),
            PaintNode::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 } => f
                .debug_struct("RadialGradient")
                .field("extend", &extend)
                .field("stops", &stops)
                .field("x0", &x0)
                .field("y0", &y0)
                .field("r0", &r0)
                .field("x1", &x1)
                .field("y1", &y1)
                .field("r1", &r1)
                .finish(),
            PaintNode::SweepGradient { extend, stops, cx, cy, start_angle, end_angle } => f
                .debug_struct("SweepGradient")
                .field("extend", &extend)
                .field("stops", &stops)
                .field("cx", &cx)
                .field("cy", &cy)
                .field("start_angle", &start_angle)
                .field("end_angle", &end_angle)
                .finish(),
            PaintNode::Transform { matrix } => {
                f.debug_struct("Transform").field("child", &child()).field("matrix", &matrix).finish()
            }
            PaintNode::Translate { dx, dy } => {
                f.debug_struct("Translate").field("child", &child()).field("dx", &dx).field("dy", &dy).finish()
            }
            PaintNode::Scale { sx, sy, center } => f
                .debug_struct("Scale")
                .field("child", &child())
                .field("sx", &sx)
                .field("sy", &sy)
                .field("center", &center)
                .finish(),
            PaintNode::ScaleUniform { s, center } => {
                f.debug_struct("ScaleUniform").field("child", &child()).field("s", &s).field("center", &center).finish()
            }
            PaintNode::Rotate { angle, center } => f
                .debug_struct("Rotate")
                .field("child", &child())
                .field("angle", &angle)
                .field("center", &center)
                .finish(),
            PaintNode::Skew { x_angle, y_angle, center } => f
                .debug_struct("Skew")
                .field("child", &child())
                .field("x_angle", &x_angle)
                .field("y_angle", &y_angle)
                .field("center", &center)
                .finish(),
            PaintNode::Composite { mode } => f
                .debug_struct("Composite")
                .field("source", &child())
                .field("mode", &mode)
                .field("backdrop", &child())
                .finish(),
        }
    }
}

impl PartialEq for PaintRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.node() == other.node() && self.children().eq(other.children())
    }
}

impl<'s> PaintNode<'s> {
    // A gradient's stops, none for another paint.
    fn stops(&self) -> &'s [ColorStop] {
        match *self {
            PaintNode::LinearGradient { stops, .. } | PaintNode::RadialGradient { stops, .. } => stops,
            PaintNode::SweepGradient { stops, .. } => stops,
            _ => &[],
        }
    }

    // The paint with `stops` for a gradient's, borrowed for as long as they are.
    fn with_stops<'t>(self, stops: &'t [ColorStop]) -> PaintNode<'t> {
        match self {
            PaintNode::Layers => PaintNode::Layers,
            PaintNode::Glyph { glyph } => PaintNode::Glyph { glyph },
            PaintNode::ColrGlyph { glyph } => PaintNode::ColrGlyph { glyph },
            PaintNode::Solid { color, foreground } => PaintNode::Solid { color, foreground },
            PaintNode::LinearGradient { extend, x0, y0, x1, y1, x2, y2, .. } => {
                PaintNode::LinearGradient { extend, stops, x0, y0, x1, y1, x2, y2 }
            }
            PaintNode::RadialGradient { extend, x0, y0, r0, x1, y1, r1, .. } => {
                PaintNode::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 }
            }
            PaintNode::SweepGradient { extend, cx, cy, start_angle, end_angle, .. } => {
                PaintNode::SweepGradient { extend, stops, cx, cy, start_angle, end_angle }
            }
            PaintNode::Transform { matrix } => PaintNode::Transform { matrix },
            PaintNode::Translate { dx, dy } => PaintNode::Translate { dx, dy },
            PaintNode::Scale { sx, sy, center } => PaintNode::Scale { sx, sy, center },
            PaintNode::ScaleUniform { s, center } => PaintNode::ScaleUniform { s, center },
            PaintNode::Rotate { angle, center } => PaintNode::Rotate { angle, center },
            PaintNode::Skew { x_angle, y_angle, center } => PaintNode::Skew { x_angle, y_angle, center },
            PaintNode::Composite { mode } => PaintNode::Composite { mode },
        }
    }
}

fn parse_affine(ctx: &Colrv1Ctx, off: usize, is_var: bool) -> Option<[f64; 6]> {
    let colr = ctx.colr;
    let mut raw = [0i32; 6];
    for (i, r) in raw.iter_mut().enumerate() {
        *r = read_i32_be(colr, off + i * 4)?;
    }
    if !is_var {
        return Some(raw.map(fixed));
    }
    let var_index_base = read_u32_be(colr, off + 24)?;
    let mut out = [0.0f64; 6];
    for i in 0..6 {
        out[i] = resolve_var_field(raw[i], VarField::Fixed1616, var_index_base, i as u32, ctx);
    }
    Some(out)
}

fn read_center(colr: &[u8], off: usize) -> Option<Center> {
    Some(Center { x: read_i16_be(colr, off)? as f64, y: read_i16_be(colr, off + 2)? as f64 })
}

fn resolve_var_center(ctx: &Colrv1Ctx, off: usize, var_index_base: u32, base_position: u32) -> Option<Center> {
    let cx = read_i16_be(ctx.colr, off)? as i32;
    let cy = read_i16_be(ctx.colr, off + 2)? as i32;
    Some(Center {
        x: resolve_var_field(cx, VarField::Raw, var_index_base, base_position, ctx),
        y: resolve_var_field(cy, VarField::Raw, var_index_base, base_position + 1, ctx),
    })
}

// The paint at `off` plus a 24-bit offset read at `off + at`; none for a null offset or one that leaves the table.
pub(super) fn child_at(colr: &[u8], off: usize, at: usize) -> Option<usize> {
    let rel = read_offset24(colr, off.checked_add(at)?)?;
    off.checked_add(rel).filter(|&child| rel != 0 && child < colr.len())
}

// One paint as read: its own fields resolved, its children named by `C` and its layers by `L`, a tree's nodes or
// offsets into the table to read in turn. A gradient's stops are borrowed.
#[derive(Clone, Copy)]
pub(crate) enum Node<'s, C, L> {
    Layers(L),
    Glyph { child: C, glyph_id: u32 },
    ColrGlyph { glyph_id: u32, child: C },
    Solid { color: Rgba, foreground: bool },
    LinearGradient { extend: Extend, stops: &'s [ColorStop], x0: f64, y0: f64, x1: f64, y1: f64, x2: f64, y2: f64 },
    RadialGradient { extend: Extend, stops: &'s [ColorStop], x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64 },
    SweepGradient { extend: Extend, stops: &'s [ColorStop], cx: f64, cy: f64, start_angle: f64, end_angle: f64 },
    Transform { child: C, matrix: [f64; 6] },
    Translate { child: C, dx: f64, dy: f64 },
    Scale { child: C, sx: f64, sy: f64, center: Option<Center> },
    ScaleUniform { child: C, s: f64, center: Option<Center> },
    Rotate { child: C, angle: f64, center: Option<Center> },
    Skew { child: C, x_angle: f64, y_angle: f64, center: Option<Center> },
    Composite { source: C, mode: Blend, backdrop: C },
}

// What a walk needs of a paint to go on: where its children are, with the transform onto them, and every fill alike.
#[derive(Clone, Copy)]
pub(crate) enum Shape<C, L> {
    Layers(L),
    Glyph(C, u32),
    ColrGlyph(C, u32),
    Transform(C, Matrix),
    Composite { source: C, mode: Blend, backdrop: C },
    Fill,
}

impl<C: Copy, L: Copy> Node<'_, C, L> {
    pub(crate) fn shape(&self) -> Shape<C, L> {
        match *self {
            Node::Layers(layers) => Shape::Layers(layers),
            Node::Glyph { child, glyph_id } => Shape::Glyph(child, glyph_id),
            Node::ColrGlyph { glyph_id, child } => Shape::ColrGlyph(child, glyph_id),
            Node::Transform { child, matrix } => Shape::Transform(child, matrix),
            Node::Translate { child, dx, dy } => Shape::Transform(child, [1.0, 0.0, 0.0, 1.0, dx, dy]),
            Node::Scale { child, sx, sy, center } => Shape::Transform(child, scale_matrix(sx, sy, center)),
            Node::ScaleUniform { child, s, center } => Shape::Transform(child, scale_matrix(s, s, center)),
            Node::Rotate { child, angle, center } => Shape::Transform(child, rotate_matrix(angle, center)),
            Node::Skew { child, x_angle, y_angle, center } => {
                Shape::Transform(child, skew_matrix(x_angle, y_angle, center))
            }
            Node::Composite { source, mode, backdrop } => Shape::Composite { source, mode, backdrop },
            Node::Solid { .. } | Node::LinearGradient { .. } | Node::RadialGradient { .. } => Shape::Fill,
            Node::SweepGradient { .. } => Shape::Fill,
        }
    }
}

// A paint graph read one paint at a time: a tree already built, or the table walked where it lies.
pub(crate) trait Graph {
    type Child: Copy;
    type Layers: Copy;
    // The paint at `child`, held open until `close`; None for one that draws nothing.
    fn open(&mut self, child: Self::Child) -> Option<Node<'_, Self::Child, Self::Layers>>;
    fn close(&mut self);
    // A run's layers, in order, until it ends; the iterator borrows nothing of the graph, so each opens as it comes.
    fn layers(&self, layers: Self::Layers) -> impl Iterator<Item = Self::Child> + use<Self>;
    // Spends what reading the paint at `child` and all below it would, so a walk that passes them by unread
    // still runs out of budget where reading them does.
    fn skip(&mut self, child: Self::Child);
    // A fresh walk from the root, its budget unspent.
    fn restart(&mut self);
}

// A graph already built, read with no budget. A paint is its index, past the list for one that isn't there, which
// reads as empty layers, as a paint that doesn't read is built.
pub(crate) struct Tree<'t>(&'t PaintGraph);

impl<'t> Tree<'t> {
    pub(crate) fn new(paint: &'t PaintGraph) -> Self {
        Tree(paint)
    }

    pub(crate) const ROOT: usize = 0;
}

impl<'t> Graph for Tree<'t> {
    type Child = usize;
    // The span of a paint's children: the first's index, and the index past the last.
    type Layers = (usize, usize);

    fn open(&mut self, at: usize) -> Option<Node<'_, usize, (usize, usize)>> {
        let paint = self.0;
        let end = paint.end_of(at);
        let child = at.checked_add(1).filter(|&c| c < end).unwrap_or(NONE);
        let second = Some(paint.end_of(child)).filter(|&c| child != NONE && c < end).unwrap_or(NONE);
        Some(match (PaintRef { paint, at }).node() {
            PaintNode::Layers => Node::Layers((child.min(end), end)),
            PaintNode::Glyph { glyph } => Node::Glyph { child, glyph_id: glyph.0 },
            PaintNode::ColrGlyph { glyph } => Node::ColrGlyph { glyph_id: glyph.0, child },
            PaintNode::Solid { color, foreground } => Node::Solid { color, foreground },
            PaintNode::LinearGradient { extend, stops, x0, y0, x1, y1, x2, y2 } => {
                Node::LinearGradient { extend, stops, x0, y0, x1, y1, x2, y2 }
            }
            PaintNode::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 } => {
                Node::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 }
            }
            PaintNode::SweepGradient { extend, stops, cx, cy, start_angle, end_angle } => {
                Node::SweepGradient { extend, stops, cx, cy, start_angle, end_angle }
            }
            PaintNode::Transform { matrix } => Node::Transform { child, matrix },
            PaintNode::Translate { dx, dy } => Node::Translate { child, dx, dy },
            PaintNode::Scale { sx, sy, center } => Node::Scale { child, sx, sy, center },
            PaintNode::ScaleUniform { s, center } => Node::ScaleUniform { child, s, center },
            PaintNode::Rotate { angle, center } => Node::Rotate { child, angle, center },
            PaintNode::Skew { x_angle, y_angle, center } => Node::Skew { child, x_angle, y_angle, center },
            PaintNode::Composite { mode } => Node::Composite { source: child, mode, backdrop: second },
        })
    }

    fn close(&mut self) {}

    fn layers(&self, (first, end): (usize, usize)) -> impl Iterator<Item = usize> + use<'t> {
        let paint = self.0;
        core::iter::successors(Some(first).filter(|&c| c < end), move |&c| Some(paint.end_of(c)).filter(|&n| n < end))
    }

    fn skip(&mut self, _: usize) {}

    fn restart(&mut self) {}
}

// A PaintColrLayers' run of `count` layers from `first` in the LayerList at `list`, which holds `total`.
#[derive(Clone, Copy)]
pub(crate) struct LayerRun {
    pub(crate) first: usize,
    pub(crate) count: usize,
    pub(crate) list: usize,
    pub(crate) total: usize,
}

// The paint at `off`, fields resolved, children as offsets, stops into `stops`; None if unreadable or of an unknown
// format. Inlined, as returning it stalls the build.
#[inline(always)]
pub(super) fn read<'s>(
    ctx: &Colrv1Ctx, off: usize, budget: &mut PaintBudget, stops: &'s mut Vec<ColorStop>,
) -> Option<Node<'s, Option<usize>, LayerRun>> {
    let colr = ctx.colr;
    let format = *colr.get(off)?;

    Some(match format {
        1 => {
            let count = *colr.get(off + 1)? as usize;
            let first = read_u32_be(colr, off + 2)? as usize;
            let list = ctx.layer_list_off?;
            let total = read_u32_be(colr, list)? as usize;
            Node::Layers(LayerRun { first, count, list, total })
        }
        2 | 3 => {
            let palette_index = read_u16_be(colr, off + 1)?;
            let alpha_raw = read_i16_be(colr, off + 3)?;
            let alpha = if format == 3 {
                let var_index_base = read_u32_be(colr, off + 5)?;
                resolve_var_field(alpha_raw as i32, VarField::Alpha, var_index_base, 0, ctx)
            } else {
                f2dot14(alpha_raw)
            };
            let (color, foreground) = ctx.color(palette_index, alpha)?;
            Node::Solid { color, foreground }
        }
        4 | 5 => {
            let is_var = format == 5;
            let extend = parse_color_line(ctx, ctx.child(off, 1)??, is_var, budget, stops)?;
            let coord_off = off + 4;
            let raws = read_i16_pair3(colr, coord_off)?;
            let coords = if is_var {
                let var_index_base = read_u32_be(colr, coord_off + 12)?;
                resolve_coords6(ctx, &raws, var_index_base)
            } else {
                raws.map(|v| v as f64)
            };
            Node::LinearGradient {
                extend, stops,
                x0: coords[0], y0: coords[1], x1: coords[2], y1: coords[3], x2: coords[4], y2: coords[5],
            }
        }
        6 | 7 => {
            let is_var = format == 7;
            let extend = parse_color_line(ctx, ctx.child(off, 1)??, is_var, budget, stops)?;
            let c = off + 4;
            let x0 = read_i16_be(colr, c)?;
            let y0 = read_i16_be(colr, c + 2)?;
            let r0 = read_u16_be(colr, c + 4)?;
            let x1 = read_i16_be(colr, c + 6)?;
            let y1 = read_i16_be(colr, c + 8)?;
            let r1 = read_u16_be(colr, c + 10)?;
            let (x0, y0, r0, x1, y1, r1) = if is_var {
                let vib = read_u32_be(colr, c + 12)?;
                (
                    resolve_var_field(x0 as i32, VarField::Raw, vib, 0, ctx),
                    resolve_var_field(y0 as i32, VarField::Raw, vib, 1, ctx),
                    resolve_var_field(r0 as i32, VarField::Raw, vib, 2, ctx),
                    resolve_var_field(x1 as i32, VarField::Raw, vib, 3, ctx),
                    resolve_var_field(y1 as i32, VarField::Raw, vib, 4, ctx),
                    resolve_var_field(r1 as i32, VarField::Raw, vib, 5, ctx),
                )
            } else {
                (x0 as f64, y0 as f64, r0 as f64, x1 as f64, y1 as f64, r1 as f64)
            };
            Node::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 }
        }
        8 | 9 => {
            let is_var = format == 9;
            let extend = parse_color_line(ctx, ctx.child(off, 1)??, is_var, budget, stops)?;
            let c = off + 4;
            let cx_raw = read_i16_be(colr, c)?;
            let cy_raw = read_i16_be(colr, c + 2)?;
            let start_raw = read_i16_be(colr, c + 4)?;
            let end_raw   = read_i16_be(colr, c + 6)?;
            let (cx, cy, start_angle, end_angle) = if is_var {
                let vib = read_u32_be(colr, c + 8)?;
                (
                    resolve_var_field(cx_raw as i32, VarField::Raw, vib, 0, ctx),
                    resolve_var_field(cy_raw as i32, VarField::Raw, vib, 1, ctx),
                    resolve_var_angle(start_raw, 1.0, vib, 2, ctx),
                    resolve_var_angle(end_raw, 1.0, vib, 3, ctx),
                )
            } else {
                (cx_raw as f64, cy_raw as f64, decode_angle(start_raw, 1.0), decode_angle(end_raw, 1.0))
            };
            Node::SweepGradient { extend, stops, cx, cy, start_angle, end_angle }
        }
        10 => Node::Glyph { child: ctx.child(off, 1)?, glyph_id: read_u16_be(colr, off + 4)?.into() },
        33 => Node::Glyph { child: ctx.child(off, 1)?, glyph_id: read_u24_be(colr, off + 4)? },
        11 => {
            let glyph_id = u32::from(read_u16_be(colr, off + 1)?);
            let child = super::lookup_base_glyph_paint_offset(colr, ctx.base_glyph_list_off, glyph_id);
            Node::ColrGlyph { glyph_id, child }
        }
        12 | 13 => {
            let child = ctx.child(off, 1)?;
            let matrix = parse_affine(ctx, ctx.child(off, 4)??, format == 13)?;
            Node::Transform { child, matrix }
        }
        14 | 15 => {
            let child = ctx.child(off, 1)?;
            let dx_raw = read_i16_be(colr, off + 4)?;
            let dy_raw = read_i16_be(colr, off + 6)?;
            let (dx, dy) = if format == 15 {
                let vib = read_u32_be(colr, off + 8)?;
                (
                    resolve_var_field(dx_raw as i32, VarField::Raw, vib, 0, ctx),
                    resolve_var_field(dy_raw as i32, VarField::Raw, vib, 1, ctx),
                )
            } else {
                (dx_raw as f64, dy_raw as f64)
            };
            Node::Translate { child, dx, dy }
        }
        16..=23 => {
            let child = ctx.child(off, 1)?;
            let uniform = matches!(format, 20..=23);
            let around_center = matches!(format, 18 | 19 | 22 | 23);
            let is_var = format % 2 == 1;

            let (sx_raw, sy_raw, after_scale_off) = if uniform {
                let s = read_i16_be(colr, off + 4)?;
                (s, s, off + 6)
            } else {
                (read_i16_be(colr, off + 4)?, read_i16_be(colr, off + 6)?, off + 8)
            };

            let (sx, sy, center) = if is_var {
                let vib_off = if around_center { after_scale_off + 4 } else { after_scale_off };
                let vib = read_u32_be(colr, vib_off)?;
                let sx = resolve_var_field(sx_raw as i32, VarField::F2Dot14, vib, 0, ctx);
                let sy = if uniform { sx } else { resolve_var_field(sy_raw as i32, VarField::F2Dot14, vib, 1, ctx) };
                let base_pos = if uniform { 1 } else { 2 };
                let center =
                    if around_center { Some(resolve_var_center(ctx, after_scale_off, vib, base_pos)?) } else { None };
                (sx, sy, center)
            } else {
                let center = if around_center { Some(read_center(colr, after_scale_off)?) } else { None };
                (f2dot14(sx_raw), f2dot14(sy_raw), center)
            };

            if uniform { Node::ScaleUniform { child, s: sx, center } } else { Node::Scale { child, sx, sy, center } }
        }
        24..=27 => {
            let child = ctx.child(off, 1)?;
            let around_center = matches!(format, 26 | 27);
            let is_var = format % 2 == 1;
            let angle_raw = read_i16_be(colr, off + 4)?;

            let (angle, center) = if is_var {
                let center_off = off + 6;
                let vib_off = if around_center { center_off + 4 } else { center_off };
                let vib = read_u32_be(colr, vib_off)?;
                let angle = resolve_var_angle(angle_raw, 0.0, vib, 0, ctx);
                (angle, if around_center { Some(resolve_var_center(ctx, center_off, vib, 1)?) } else { None })
            } else {
                (decode_angle(angle_raw, 0.0), if around_center { Some(read_center(colr, off + 6)?) } else { None })
            };
            Node::Rotate { child, angle, center }
        }
        28..=31 => {
            let child = ctx.child(off, 1)?;
            let around_center = matches!(format, 30 | 31);
            let is_var = format % 2 == 1;
            let x_raw = read_i16_be(colr, off + 4)?;
            let y_raw = read_i16_be(colr, off + 6)?;

            let (x_angle, y_angle, center) = if is_var {
                let center_off = off + 8;
                let vib_off = if around_center { center_off + 4 } else { center_off };
                let vib = read_u32_be(colr, vib_off)?;
                let x_angle = resolve_var_angle(x_raw, 0.0, vib, 0, ctx);
                let y_angle = resolve_var_angle(y_raw, 0.0, vib, 1, ctx);
                let center = if around_center { Some(resolve_var_center(ctx, center_off, vib, 2)?) } else { None };
                (x_angle, y_angle, center)
            } else {
                let center = if around_center { Some(read_center(colr, off + 8)?) } else { None };
                (decode_angle(x_raw, 0.0), decode_angle(y_raw, 0.0), center)
            };
            Node::Skew { child, x_angle, y_angle, center }
        }
        32 => {
            let mode = Blend::from_colr(*colr.get(off + 4)?);
            Node::Composite { source: ctx.child(off, 1)?, mode, backdrop: ctx.child(off, 5)? }
        }
        _ => return None,
    })
}

fn read_i16_pair3(colr: &[u8], off: usize) -> Option<[i16; 6]> {
    let mut out = [0i16; 6];
    for (i, o) in out.iter_mut().enumerate() {
        *o = read_i16_be(colr, off + i * 2)?;
    }
    Some(out)
}

fn resolve_coords6(ctx: &Colrv1Ctx, raws: &[i16; 6], var_index_base: u32) -> [f64; 6] {
    let mut out = [0.0f64; 6];
    for i in 0..6 {
        out[i] = resolve_var_field(raws[i] as i32, VarField::Raw, var_index_base, i as u32, ctx);
    }
    out
}
