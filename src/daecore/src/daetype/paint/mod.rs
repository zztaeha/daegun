use alloc::vec::Vec;
use core::fmt;
use core::iter::FusedIterator;
use core::range::Range;
use crate::daecore::daetype::outline::{FillRule, OutlinePen, Path, PathView};

pub mod colr;

pub use crate::daecore::daekit::draw::{
    Blend, Extend, Gradient, GradientKind, Rgba, Stop, Stops,
};

/// What a fill paints.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Paint {
    /// One color.
    Solid(Rgba),
    /// One of the list's gradients.
    Gradient(GradientId),
}

/// One shape of a clip: one of the list's paths, filled by a rule and placed by a transform.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ClipShape {
    /// The path.
    pub path: PathId,
    /// How the path fills.
    pub rule: FillRule,
    /// Where it's placed, a [`Matrix`](crate::color::Matrix).
    pub transform: [f64; 6],
}

/// A path of a [`DisplayList`], by its place in the list.
#[expect(clippy::exhaustive_structs, reason = "a plain value: its index is all it is")]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct PathId(pub usize);

/// A gradient of a [`DisplayList`], by its place in the list.
#[expect(clippy::exhaustive_structs, reason = "a plain value: its index is all it is")]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct GradientId(pub usize);

/// One step of a [`DisplayList`], which draws them in order.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Op {
    /// A path filled.
    Fill {
        /// The path.
        path: PathId,
        /// What it's filled with.
        paint: Paint,
        /// How the path fills.
        rule: FillRule,
        /// Where it's placed, a [`Matrix`](crate::color::Matrix).
        transform: [f64; 6],
    },
    /// A clip to these shapes together, until the `PopClip` that pairs with it.
    PushClip {
        /// The shapes, read through [`DisplayList::shapes`].
        shapes: Range<usize>,
    },
    /// The last clip pushed, lifted.
    PopClip,
    /// A layer drawn on its own, until the `PopLayer` that pairs with it composites it onto what lies below.
    PushLayer {
        /// Its opacity as it composites, 0 to 1.
        opacity: f32,
        /// How it composites.
        blend: Blend,
    },
    /// The last layer pushed, composited.
    PopLayer,
}

/// A scene to draw: ops in order over the paths, clip shapes and gradients it holds, in the units of whatever it was
/// drawn from, font units for a color glyph. Compared and printed as drawn: each path, and each op with what it
/// names.
// Paths, clip shapes and gradient stops each lie in one buffer the list keeps, so a scene allocates per buffer
// rather than per element.
#[derive(Clone, Default)]
pub struct DisplayList {
    ops: Vec<Op>,
    outlines: Path,
    paths: Vec<(Range<usize>, Range<usize>)>,
    shapes: Vec<ClipShape>,
    gradients: Vec<Line>,
    stops: Vec<Stop>,
}

// Its lists to the thread's spare, for `spare`'s next list, as a scene drawn each call would allocate each anew; one
// past `SPARE_BYTES` drops its own, as does a build checking C's use of freed handles, whose views lie in them.
impl Drop for DisplayList {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        if cfg!(not(daegun_no_spares)) && (1..=SPARE_BYTES).contains(&self.heap_bytes()) {
            use core::mem::take;
            self.clear();
            let lists = (take(&mut self.ops), take(&mut self.outlines), take(&mut self.paths),
                         take(&mut self.shapes), take(&mut self.gradients), take(&mut self.stops));
            let _ = SPARE.try_with(|spare| spare.set(Some(lists)));
        }
    }
}

#[cfg(feature = "std")]
type Lists = (Vec<Op>, Path, Vec<(Range<usize>, Range<usize>)>, Vec<ClipShape>, Vec<Line>, Vec<Stop>);

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

#[derive(Clone, Copy)]
struct Line {
    kind: GradientKind,
    stops: Range<usize>,
    extend: Extend,
    transform: [f64; 6],
}

// Where a list stood. Going back drops what came since, other than paths: an op after it may name one.
#[derive(Clone, Copy)]
pub(crate) struct Mark {
    pub(crate) ops: usize,
    shapes: usize,
    gradients: usize,
    stops: usize,
}

impl DisplayList {
    /// The ops, in drawing order.
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    /// Whether it draws nothing: no ops.
    pub const fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// A path; None past the list's.
    pub fn path(&self, id: PathId) -> Option<PathView<'_>> {
        self.paths.get(id.0).map(|&span| self.view(span))
    }

    /// A gradient; None past the list's.
    pub fn gradient(&self, id: GradientId) -> Option<Gradient<'_>> {
        self.gradients.get(id.0).map(|line| self.read(line))
    }

    /// Every path, in id order: the `n`th is [`path`](Self::path)'s for `PathId(n)`.
    pub fn paths(&self) -> impl ExactSizeIterator<Item = PathView<'_>> + DoubleEndedIterator + FusedIterator + Clone {
        self.paths.iter().map(|&span| self.view(span))
    }

    /// Every gradient, in id order: the `n`th is [`gradient`](Self::gradient)'s for `GradientId(n)`.
    pub fn gradients(
        &self,
    ) -> impl ExactSizeIterator<Item = Gradient<'_>> + DoubleEndedIterator + FusedIterator + Clone {
        self.gradients.iter().map(|line| self.read(line))
    }

    /// The clip shapes a `PushClip` names; none for a range past the list's.
    pub fn shapes(&self, shapes: Range<usize>) -> &[ClipShape] {
        self.shapes.get(shapes).unwrap_or_default()
    }

    /// A path added, and its id.
    pub fn push_path(&mut self, path: PathView<'_>) -> PathId {
        let (verbs, points) = self.ends();
        self.outlines.extend_from(path);
        self.span_from(verbs, points)
    }

    /// A gradient added, its stops copied, and its id.
    pub fn push_gradient(&mut self, gradient: Gradient<'_>) -> GradientId {
        let start = self.stops.len();
        self.stops.extend_from_slice(gradient.stops);
        self.add_gradient(gradient.kind, start, gradient.extend, gradient.transform)
    }

    /// Clip shapes added, and the range a `PushClip` names them by.
    pub fn push_shapes(&mut self, shapes: &[ClipShape]) -> Range<usize> {
        let start = self.shapes.len();
        self.shapes.extend_from_slice(shapes);
        (start..self.shapes.len()).into()
    }

    /// An op added after the last.
    pub fn push(&mut self, op: Op) {
        self.ops.push(op);
    }

    // None, leaving the list as it was, where `draw` fails or draws nothing.
    pub(crate) fn draw_path(&mut self, draw: impl FnOnce(&mut dyn OutlinePen) -> bool) -> Option<PathId> {
        let (verbs, points) = self.ends();
        if !draw(&mut self.outlines) || self.ends().0 == verbs {
            self.outlines.truncate(verbs, points);
            return None;
        }
        Some(self.span_from(verbs, points))
    }

    pub(crate) fn add_path(&mut self, draw: impl FnOnce(&mut dyn OutlinePen)) -> PathId {
        let (verbs, points) = self.ends();
        draw(&mut self.outlines);
        self.span_from(verbs, points)
    }

    pub(crate) fn redraw_path(&mut self, id: PathId, draw: impl FnOnce(&mut dyn OutlinePen)) {
        let (verbs, points) = self.ends();
        draw(&mut self.outlines);
        let (v, p) = self.ends();
        if let Some(span) = self.paths.get_mut(id.0) {
            *span = ((verbs..v).into(), (points..p).into());
        }
    }

    // A gradient over the stops added from `stops` on.
    pub(crate) fn add_gradient(
        &mut self,
        kind: GradientKind,
        stops: usize,
        extend: Extend,
        transform: [f64; 6],
    ) -> GradientId {
        let stops = (stops..self.stops.len()).into();
        self.gradients.push(Line { kind, stops, extend, transform });
        GradientId(self.gradients.len() - 1)
    }

    // A list to draw into: the thread's spare, emptied with its room, or a new one.
    pub(crate) fn spare() -> DisplayList {
        #[cfg(feature = "std")]
        if let Ok(Some((ops, outlines, paths, shapes, gradients, stops))) = SPARE.try_with(core::cell::Cell::take) {
            return DisplayList { ops, outlines, paths, shapes, gradients, stops };
        }
        DisplayList::default()
    }

    /// Emptied, keeping its room, so a list drawn into again allocates only to grow.
    pub fn clear(&mut self) {
        self.ops.clear();
        self.outlines.clear();
        self.paths.clear();
        self.shapes.clear();
        self.gradients.clear();
        self.stops.clear();
    }

    #[cfg(feature = "std")]
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        let lists = heap_of(&self.ops) + heap_of(&self.paths) + heap_of(&self.shapes) + heap_of(&self.gradients);
        lists + heap_of(&self.stops) + self.outlines.heap_bytes()
    }

    pub(crate) fn reserve(&mut self, ops: usize, paths: usize, verbs: usize, points: usize) {
        self.ops.reserve(ops);
        self.paths.reserve(paths);
        self.outlines.reserve(verbs, points);
    }

    pub(crate) fn mark(&self) -> Mark {
        let (ops, shapes, gradients, stops) =
            (self.ops.len(), self.shapes.len(), self.gradients.len(), self.stops.len());
        Mark { ops, shapes, gradients, stops }
    }

    pub(crate) fn truncate(&mut self, to: Mark) {
        self.ops.truncate(to.ops);
        self.shapes.truncate(to.shapes);
        self.gradients.truncate(to.gradients);
        self.stops.truncate(to.stops);
    }

    pub(crate) fn remove(&mut self, at: usize) {
        self.ops.remove(at);
    }

    fn view(&self, (verbs, points): (Range<usize>, Range<usize>)) -> PathView<'_> {
        let (v, p) = self.outlines.parts();
        PathView::new(&v[verbs], &p[points])
    }

    fn read(&self, &Line { kind, stops, extend, transform }: &Line) -> Gradient<'_> {
        Gradient { kind, stops: &self.stops[stops], extend, transform }
    }

    fn ends(&self) -> (usize, usize) {
        let (v, p) = self.outlines.parts();
        (v.len(), p.len())
    }

    fn span_from(&mut self, verbs: usize, points: usize) -> PathId {
        let (v, p) = self.ends();
        self.paths.push(((verbs..v).into(), (points..p).into()));
        PathId(self.paths.len() - 1)
    }

    // Each op with what it names read from the list.
    fn drawn(&self) -> impl Iterator<Item = Drawn<'_>> {
        self.ops.iter().map(|&op| match op {
            Op::Fill { path, paint, rule, transform } => {
                let paint = match paint {
                    Paint::Solid(c) => Painted::Solid(c),
                    Paint::Gradient(id) => self.gradient(id).map_or(Painted::Missing(id), Painted::Gradient),
                };
                Drawn::Fill { path, paint, rule, transform }
            }
            Op::PushClip { shapes } => Drawn::PushClip { shapes: self.shapes(shapes) },
            Op::PopClip => Drawn::PopClip,
            Op::PushLayer { opacity, blend } => Drawn::PushLayer { opacity, blend },
            Op::PopLayer => Drawn::PopLayer,
        })
    }
}

#[derive(PartialEq, Debug)]
enum Drawn<'a> {
    Fill { path: PathId, paint: Painted<'a>, rule: FillRule, transform: [f64; 6] },
    PushClip { shapes: &'a [ClipShape] },
    PopClip,
    PushLayer { opacity: f32, blend: Blend },
    PopLayer,
}

#[derive(PartialEq, Debug)]
enum Painted<'a> {
    Solid(Rgba),
    Gradient(Gradient<'a>),
    Missing(GradientId),
}

impl PartialEq for DisplayList {
    fn eq(&self, other: &Self) -> bool {
        self.paths().eq(other.paths()) && self.drawn().eq(other.drawn())
    }
}

impl fmt::Debug for DisplayList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DisplayList")
            .field("paths", &fmt::from_fn(|f| f.debug_list().entries(self.paths()).finish()))
            .field("ops", &fmt::from_fn(|f| f.debug_list().entries(self.drawn()).finish()))
            .finish()
    }
}
