#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::vec::Vec;
mod format;
pub(crate) mod paint;
mod color_line;
mod varfield;
mod instance;
pub(crate) mod write;

use core::cmp::Ordering;
use super::decoder::{
    midpoint_search, read_offset24, read_u16_be, read_u32_be, records_fit, search_records,
};
use super::format::ivs::DeltaSource;
use super::format::ivs::DeltaView;
#[cfg(test)]
use crate::daecore::daetype::Tables;
use crate::daecore::daekit::draw::matrix::{concat, Matrix};
use crate::daecore::daekit::draw::Rgba;
use crate::daecore::GlyphId;

pub use paint::{Center, ColorStop, PaintGraph, PaintNode, PaintRef, PaintVisitor};
pub(crate) use paint::{Graph, LayerRun, Node, Shape, Tree};
pub(crate) use format::paint_layout;
pub(crate) use instance::instance_colr_v1;

// How deep a paint graph is walked: a paint past it is left out, as a graph's own drawing expects.
pub(crate) const MAX_PAINT_DEPTH: usize = 64;

// What a glyph's walk may read, each beside one for every 4 bytes of the table: a paint not shared takes 6 bytes at
// least with its offset and a stop 6, so only a graph sharing its paints or color lines can spend them.
pub(crate) const MAX_PAINT_VISITS: usize = 100_000;
pub(crate) const MAX_PAINT_STOPS: usize = 1_000_000;

// A version 1 COLR's header: where its lists and variation data start, None for a NULL offset.
pub(crate) struct Header {
    pub(crate) base_glyph_list: Option<usize>,
    pub(crate) layer_list: Option<usize>,
    pub(crate) clip_list: Option<usize>,
    pub(crate) var_index_map: Option<usize>,
    pub(crate) var_store: Option<usize>,
}

// None for version 0, or a table shorter than the version 1 header; a later version reads as 1.
pub(crate) fn header(colr: &[u8]) -> Option<Header> {
    if colr.len() < 34 || read_u16_be(colr, 0)? < 1 {
        return None;
    }
    // Nothing reads past the table, so an offset that far stops at its end, and no sum after it can overflow.
    let at = |field: usize| read_u32_be(colr, field).filter(|&v| v != 0).map(|v| (v as usize).min(colr.len()));
    Some(Header {
        base_glyph_list: at(14),
        layer_list: at(18),
        clip_list: at(22),
        var_index_map: at(26),
        var_store: at(30),
    })
}

// One glyph's walk: the paints from the root down to the one being read, so a paint that names one
// of them is a cycle, and what is left of the visits and stops a glyph may spend of what its table allows.
pub(crate) struct PaintBudget {
    path: [usize; MAX_PAINT_DEPTH + 1],
    depth: usize,
    visits: usize,
    stops: usize,
    allowed: (usize, usize),
    // Set once the visits or stops run out; from then on, which paints are left out follows the walk's order.
    exhausted: bool,
}

impl PaintBudget {
    // For a table of `bytes`: a visit and a stop for each 4 bytes past the floors.
    pub fn new(bytes: usize) -> Self {
        let allowed = (MAX_PAINT_VISITS + bytes / 4, MAX_PAINT_STOPS + bytes / 4);
        let (visits, stops) = allowed;
        PaintBudget { path: [0; MAX_PAINT_DEPTH + 1], depth: 0, visits, stops, allowed, exhausted: false }
    }

    #[must_use]
    pub(crate) fn spend_stops(&mut self, n: usize) -> bool {
        match self.stops.checked_sub(n) {
            Some(left) => { self.stops = left; true }
            None => { self.exhausted = true; false }
        }
    }

    // False for a paint past the depth limit, already on the path, or past the visit budget.
    #[must_use]
    pub(crate) fn enter(&mut self, off: usize) -> bool {
        if self.depth > MAX_PAINT_DEPTH || self.path[..self.depth].contains(&off) {
            return false;
        }
        if self.visits == 0 {
            self.exhausted = true;
            return false;
        }
        self.path[self.depth] = off;
        self.depth += 1;
        self.visits -= 1;
        true
    }

    pub(crate) fn leave(&mut self) {
        self.depth -= 1;
    }

    // Unspent again; the path past its depth is never read, so it is left as it is.
    fn reset(&mut self) {
        (self.depth, (self.visits, self.stops), self.exhausted) = (0, self.allowed, false);
    }
}

pub type ColrV1VarData = Option<DeltaSource>;

// The deltas a walk reads, at 16.16 scalars and each rounded once to its field's unit: an instance's store decoded, or
// read in place as drawing reads it, the same layout either way; none at a location setting none.
#[derive(Clone, Copy)]
pub(crate) enum Deltas<'a> {
    Measured(Option<&'a DeltaSource>, &'a [i64]),
    Drawn(Option<(&'a DeltaView, &'a [i64])>),
}

pub fn parse_colr_v1_var_data(colr: &[u8]) -> ColrV1VarData {
    let header = header(colr)?;
    // The implicit mapping is for a table with no map; one that is named and cannot be read leaves
    // no way to find the right deltas, so nothing varies.
    DeltaSource::read(colr, header.var_store?, header.var_index_map).ok()
}

pub(crate) struct Colrv1Ctx<'a> {
    pub colr:            &'a [u8],
    pub cpal:             Option<&'a [u8]>,
    pub palette:          Option<super::colr_v0::CpalPalette>,
    pub deltas:           Deltas<'a>,
    pub base_glyph_list_off: usize,
    pub layer_list_off:   Option<usize>,
    // True for a walk that draws. False for extents', which reads no colors, a fill covering its clip whatever it
    // paints.
    pub drawn:            bool,
}

impl Colrv1Ctx<'_> {
    fn color(&self, palette_index: u16, alpha: f64) -> Option<(Rgba, bool)> {
        if !self.drawn {
            return Some((Rgba::default(), false));
        }
        super::colr_v0::palette_color(self.cpal.zip(self.palette.as_ref()), palette_index, alpha)
    }

    // The child `field` bytes into the paint at `off`, Some(None) for one whose offset doesn't read or leaves the
    // table: that child is absent and its paint kept.
    pub(super) fn child(&self, off: usize, field: usize) -> Option<Option<usize>> {
        Some(paint::child_at(self.colr, off, field))
    }
}

// Where a version 1 table's walks start: its BaseGlyphList and LayerList.
#[derive(Clone, Copy)]
pub(crate) struct Lists {
    base_glyph_list: usize,
    layer_list: Option<usize>,
}

impl Lists {
    pub(crate) fn of(colr: &[u8]) -> Option<Lists> {
        let header = header(colr)?;
        Some(Lists { base_glyph_list: header.base_glyph_list?, layer_list: header.layer_list })
    }
}

// The stops a thread's last walk read, lent to its next.
#[cfg(feature = "std")]
std::thread_local! {
    static SPARE_STOPS: core::cell::Cell<Vec<ColorStop>> = const { core::cell::Cell::new(Vec::new()) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE_STOPS.try_with(core::cell::Cell::take);
}

#[cfg(feature = "std")]
const SPARE_STOPS_BYTES: usize = 64 << 10;

// One glyph's walk over the table: the paints it reads, the budget they spend and one buffer for their stops.
pub(crate) struct Walk<'a> {
    ctx: Colrv1Ctx<'a>,
    budget: PaintBudget,
    stops: Vec<ColorStop>,
}

impl<'a> Walk<'a> {
    // `f` on glyph `gid`'s walk and its root paint's offset; None for a glyph with no version 1 paint. The walk
    // runs where it's made, as one returned can be copied out, budget and all, on every glyph.
    #[cfg(test)]
    pub(crate) fn run<R>(
        table_map:      &'a Tables,
        gid:            u32,
        palette_index:  u16,
        deltas:         Deltas<'a>,
        drawn:          bool,
        f:              impl FnOnce(&mut Walk<'a>, usize) -> R,
    ) -> Option<R> {
        let colr = table_map.get("COLR")?;
        let cpal = table_map.get("CPAL").map(|v| v.as_slice());
        Walk::over(colr, cpal, Lists::of(colr)?, gid, palette_index, deltas, drawn, f)
    }

    // As `run`, over tables and lists a caller holds.
    #[allow(clippy::too_many_arguments, reason = "run's inputs, with the tables and lists it reads")]
    pub(crate) fn over<R>(
        colr:           &'a [u8],
        cpal:           Option<&'a [u8]>,
        lists:          Lists,
        gid:            u32,
        palette_index:  u16,
        deltas:         Deltas<'a>,
        drawn:          bool,
        f:              impl FnOnce(&mut Walk<'a>, usize) -> R,
    ) -> Option<R> {
        let base_glyph_list_off = lists.base_glyph_list;
        // Most glyphs have no paint, so the palette is left unread until one does.
        let root = lookup_base_glyph_paint_offset(colr, base_glyph_list_off, gid)?;
        // A COLR with no CPAL is ignored, as the spec says, where it is drawn; extents still read its paints.
        if drawn && cpal.is_none() {
            return None;
        }
        let palette = cpal.and_then(|c| super::colr_v0::CpalPalette::new(c, palette_index));
        let ctx = Colrv1Ctx {
            colr, cpal, palette,
            deltas,
            base_glyph_list_off,
            layer_list_off: lists.layer_list,
            drawn,
        };
        #[cfg(feature = "std")]
        let stops = SPARE_STOPS.try_with(core::cell::Cell::take).unwrap_or_default();
        #[cfg(not(feature = "std"))]
        let stops = Vec::new();
        let mut walk = Walk { ctx, budget: PaintBudget::new(colr.len()), stops };
        let out = f(&mut walk, root);
        // A thread past its locals' teardown drops the stops instead, as it does more than `SPARE_STOPS_BYTES`.
        #[cfg(feature = "std")]
        if crate::daecore::daekit::base::sync::heap_of(&walk.stops) <= SPARE_STOPS_BYTES {
            walk.stops.clear();
            let _ = SPARE_STOPS.try_with(|spare| spare.set(walk.stops));
        }
        Some(out)
    }

    // How many layers the paint at `at` names, though drawing may end the run sooner; 0 for one that isn't
    // PaintColrLayers, None for one that doesn't read. The budget is left unspent.
    pub(crate) fn reads(&mut self, at: usize) -> Option<usize> {
        let layers = self.open(Some(at)).map(|node| if let Node::Layers(run) = node { run.count } else { 0 });
        self.budget.reset();
        layers
    }

    pub(crate) fn exhausted(&self) -> bool {
        self.budget.exhausted
    }

    // The graph under the paint at `at` handed to `visitor` paint by paint, a parent before its children; a paint
    // that doesn't read is empty layers.
    pub(crate) fn visit(&mut self, at: Option<usize>, visitor: &mut (impl PaintVisitor + ?Sized)) {
        let Some(node) = self.open(at) else {
            visitor.enter(PaintNode::Layers);
            visitor.leave();
            return;
        };
        match node {
            Node::Layers(run) => {
                visitor.enter(PaintNode::Layers);
                for layer in self.layers(run) {
                    self.visit(layer, visitor);
                }
            }
            Node::Glyph { child, glyph_id } => {
                visitor.enter(PaintNode::Glyph { glyph: GlyphId(glyph_id) });
                self.visit(child, visitor);
            }
            Node::ColrGlyph { glyph_id, child } => {
                visitor.enter(PaintNode::ColrGlyph { glyph: GlyphId(glyph_id) });
                self.visit(child, visitor);
            }
            Node::Solid { color, foreground } => {
                visitor.enter(PaintNode::Solid { color, foreground });
            }
            Node::LinearGradient { extend, stops, x0, y0, x1, y1, x2, y2 } => {
                visitor.enter(PaintNode::LinearGradient { extend, stops, x0, y0, x1, y1, x2, y2 });
            }
            Node::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 } => {
                visitor.enter(PaintNode::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 });
            }
            Node::SweepGradient { extend, stops, cx, cy, start_angle, end_angle } => {
                visitor.enter(PaintNode::SweepGradient { extend, stops, cx, cy, start_angle, end_angle });
            }
            Node::Transform { child, matrix } => {
                visitor.enter(PaintNode::Transform { matrix });
                self.visit(child, visitor);
            }
            Node::Translate { child, dx, dy } => {
                visitor.enter(PaintNode::Translate { dx, dy });
                self.visit(child, visitor);
            }
            Node::Scale { child, sx, sy, center } => {
                visitor.enter(PaintNode::Scale { sx, sy, center });
                self.visit(child, visitor);
            }
            Node::ScaleUniform { child, s, center } => {
                visitor.enter(PaintNode::ScaleUniform { s, center });
                self.visit(child, visitor);
            }
            Node::Rotate { child, angle, center } => {
                visitor.enter(PaintNode::Rotate { angle, center });
                self.visit(child, visitor);
            }
            Node::Skew { child, x_angle, y_angle, center } => {
                visitor.enter(PaintNode::Skew { x_angle, y_angle, center });
                self.visit(child, visitor);
            }
            Node::Composite { source, mode, backdrop } => {
                visitor.enter(PaintNode::Composite { mode });
                self.visit(source, visitor);
                self.visit(backdrop, visitor);
            }
        }
        self.close();
        visitor.leave();
    }

    // The graph from the root at `root`, built as it's visited; a root that doesn't read is empty layers, as any
    // paint that doesn't is.
    pub(crate) fn paint(&mut self, root: usize) -> PaintGraph {
        let mut paint = PaintGraph::new();
        self.visit(Some(root), &mut paint);
        paint
    }
}

impl<'a> Graph for Walk<'a> {
    type Child = Option<usize>;
    type Layers = LayerRun;

    // Inlined, as `read` is: out of line, wherever two visitors walk each paint would go back through memory and be
    // matched again.
    #[inline(always)]
    fn open(&mut self, at: Option<usize>) -> Option<Node<'_, Option<usize>, LayerRun>> {
        let off = at?;
        if !self.budget.enter(off) {
            return None;
        }
        let node = paint::read(&self.ctx, off, &mut self.budget, &mut self.stops);
        if node.is_none() {
            self.budget.leave();
        }
        node
    }

    fn close(&mut self) {
        self.budget.leave();
    }

    // A layer past the LayerList, or whose offset doesn't read or leaves the table, is None: absent, drawn as
    // nothing, and the run's other layers kept.
    fn layers(&self, run: LayerRun) -> impl Iterator<Item = Option<usize>> + use<'a> {
        let colr = self.ctx.colr;
        (0..run.count).map(move |i| {
            let index = run.first.checked_add(i).filter(|&index| index < run.total);
            let at = index.and_then(|index| index.checked_mul(4)?.checked_add(run.list)?.checked_add(4));
            at.and_then(|at| run.list.checked_add(read_u32_be(colr, at)? as usize)).filter(|&p| p < colr.len())
        })
    }

    fn skip(&mut self, at: Option<usize>) {
        let Some(node) = self.open(at) else { return };
        match node.shape() {
            Shape::Layers(run) => self.layers(run).for_each(|layer| self.skip(layer)),
            Shape::Glyph(child, _) | Shape::ColrGlyph(child, _) | Shape::Transform(child, _) => self.skip(child),
            Shape::Composite { source, backdrop, .. } => {
                self.skip(source);
                self.skip(backdrop);
            }
            Shape::Fill => {}
        }
        self.close();
    }

    fn restart(&mut self) {
        self.budget.reset();
    }
}

// Each region's 16.16 scalar at `location`, none at the default, where every coordinate is 0 and nothing varies.
#[cfg(test)]
pub fn colr_v1_region_scalars(var_data: &ColrV1VarData, location: &[f64]) -> Vec<i64> {
    match var_data {
        Some(source) if location.iter().any(|&c| c != 0.0) => source.scalars(location),
        _ => Vec::new(),
    }
}

// A glyph's graph at a location's region scalars, none at the default; the core's tests read it.
#[cfg(test)]
pub fn colr_v1_paint_graph_with_scalars(
    table_map:      &Tables,
    gid:            u32,
    palette_index:  u16,
    var_data:       &ColrV1VarData,
    region_scalars: &[i64],
) -> Option<PaintGraph> {
    let deltas = Deltas::Measured(var_data.as_ref(), region_scalars);
    Walk::run(table_map, gid, palette_index, deltas, true, |walk, root| walk.paint(root))
}

// The BaseGlyphList's record count where every record lies in the table; none where their count leaves it, which
// makes the list absent to drawing, subsets and instances alike.
pub(crate) fn base_glyph_count(colr: &[u8], list: usize) -> Option<usize> {
    let count = read_u32_be(colr, list)? as usize;
    records_fit(list.checked_add(4)?, count, 6, colr.len()).then_some(count)
}

// Base glyph `gid`'s root paint, by a binary search of the BaseGlyphList's records, which the spec sorts.
pub(super) fn lookup_base_glyph_paint_offset(colr: &[u8], base_glyph_list_off: usize, gid: u32) -> Option<usize> {
    let records_off = base_glyph_list_off.checked_add(4)?;
    let num_records = base_glyph_count(colr, base_glyph_list_off)?;
    let at = |i: usize| i.checked_mul(6).and_then(|o| records_off.checked_add(o));
    let hit = search_records(num_records, gid, |i| read_u16_be(colr, at(i)?).map(u32::from))?.ok()?;
    let rel_off = read_u32_be(colr, at(hit)?.checked_add(2)?)? as usize;
    // An offset of 0 names the null paint, which paints nothing: the table's end, where nothing reads. One past the
    // table is absent.
    if rel_off == 0 { Some(colr.len()) } else { base_glyph_list_off.checked_add(rel_off).filter(|&p| p < colr.len()) }
}

// Where a base glyph's ClipBox starts: the clip holding `gid` as a binary search finds it, the spec's records being
// sorted, the one it meets where clips overlap; none for a null offset. A ClipList of a format but 1 is unknown, and
// one whose records' count leaves the table absent: neither has any.
pub(crate) fn clip_box_offset(colr: &[u8], gid: u32) -> Option<usize> {
    let list = header(colr)?.clip_list?;
    if *colr.get(list)? != 1 { return None; }
    let records = list.checked_add(5)?;
    let count = read_u32_be(colr, list.checked_add(1)?)? as usize;
    if !records_fit(records, count, 7, colr.len()) { return None; }
    let i = midpoint_search(count, |i: usize| {
        let glyph = |at: usize| read_u16_be(colr, records + i * 7 + at).map(u32::from);
        let (first, last) = (glyph(0)?, glyph(2)?);
        Some(if gid < first { Ordering::Less } else if gid > last { Ordering::Greater } else { Ordering::Equal })
    })?;
    list.checked_add(read_offset24(colr, records + i * 7 + 4).filter(|&rel| rel != 0)?)
}

// A color glyph's clip box as (x_min, y_min, x_max, y_max) in font units, its deltas read as `deltas` says.
pub(crate) fn clip_box(colr: &[u8], gid: u32, deltas: Deltas) -> Option<[i32; 4]> {
    let (edges, var) = format::clip_box_at(colr, clip_box_offset(colr, gid)?)?;
    Some(match var {
        None => edges.map(i32::from),
        Some(vib) => varied_box(edges, |k| varfield::resolve_units(deltas, vib, k)),
    })
}

// A variable clip box's edges, each moved by its delta, the minimums floored and the maximums ceiled: rounded
// outward, the box still holds everything it clips.
pub(crate) fn varied_box(edges: [i16; 4], delta: impl Fn(u32) -> f64) -> [i32; 4] {
    core::array::from_fn(|k| {
        let d = delta(k as u32);
        i32::from(edges[k]).saturating_add(if k < 2 { d.floor() } else { d.ceil() } as i32)
    })
}

// The matrices of COLR's transforming paints, x' = a*x + c*y + e as everywhere in daegun, each
// about an optional center. A skew leans x by -tan of its angle, as the spec defines it.
pub(crate) fn scale_matrix(sx: f64, sy: f64, center: Option<Center>) -> Matrix {
    about(center, [sx, 0.0, 0.0, sy, 0.0, 0.0])
}

pub(crate) fn rotate_matrix(degrees: f64, center: Option<Center>) -> Matrix {
    let (sin, cos) = crate::daecore::daekit::float::sin_cos_degrees(degrees);
    about(center, [cos, sin, -sin, cos, 0.0, 0.0])
}

// A right angle's tangent is infinite, a skew no finite matrix holds, so it leans by nothing, drawn or measured.
pub(crate) fn skew_matrix(x_degrees: f64, y_degrees: f64, center: Option<Center>) -> Matrix {
    let tan = |deg: f64| {
        let (s, c) = crate::daecore::daekit::float::sin_cos_degrees(deg);
        if c == 0.0 { 0.0 } else { s / c }
    };
    about(center, [1.0, tan(y_degrees), -tan(x_degrees), 1.0, 0.0, 0.0])
}

fn about(center: Option<Center>, m: Matrix) -> Matrix {
    let Some(Center { x: cx, y: cy }) = center else { return m };
    concat(&concat(&[1.0, 0.0, 0.0, 1.0, -cx, -cy], &m), &[1.0, 0.0, 0.0, 1.0, cx, cy])
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/colr_v1/mod.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../tests/unit/daetype/colr_v1/mod_testing.rs"]
pub(crate) mod testing;
