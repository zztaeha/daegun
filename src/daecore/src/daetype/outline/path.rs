use alloc::vec::Vec;
use core::iter::FusedIterator;

use super::{OutlinePen, TransformPen};
use crate::daecore::Bounds;
#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;

/// Which points a path fills.
#[expect(clippy::exhaustive_enums, reason = "outlines fill by one of these two")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum FillRule {
    /// Those its contours wind around a nonzero number of times, as glyph outlines are filled.
    #[default]
    NonZero,
    /// Those its contours wind around an odd number of times.
    EvenOdd,
}

/// One step of a path, taking the points that follow it. One byte, its value fixed, as C reads it.
#[expect(clippy::exhaustive_enums, reason = "every outline format draws with these five")]
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Verb {
    /// Starts a contour at one point.
    Move = 0,
    /// A line to one point.
    Line = 1,
    /// A quadratic curve: a control, then its end.
    Quad = 2,
    /// A cubic curve: two controls, then its end.
    Cubic = 3,
    /// Closes the contour, taking no point.
    Close = 4,
}

/// A point of an outline, in the units it was drawn in, y up.
#[expect(clippy::exhaustive_structs, reason = "a plain value: its fields are all it is")]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Point {
    /// Across.
    pub x: f32,
    /// Up.
    pub y: f32,
}

impl From<(f32, f32)> for Point {
    fn from((x, y): (f32, f32)) -> Point {
        Point { x, y }
    }
}

impl From<Point> for (f32, f32) {
    fn from(p: Point) -> (f32, f32) {
        (p.x, p.y)
    }
}

impl Verb {
    pub(crate) fn points(self) -> usize {
        match self {
            Verb::Move | Verb::Line => 1,
            Verb::Quad => 2,
            Verb::Cubic => 3,
            Verb::Close => 0,
        }
    }
}

/// An outline as its verbs and their points, built as a pen draws into it.
#[derive(Default, PartialEq, Debug)]
pub struct Path {
    verbs: Vec<Verb>,
    points: Vec<Point>,
}

// By hand, so `clone_from` keeps the room of the path it overwrites, as a Vec's does.
impl Clone for Path {
    fn clone(&self) -> Path {
        Path { verbs: self.verbs.clone(), points: self.points.clone() }
    }

    fn clone_from(&mut self, source: &Path) {
        self.verbs.clone_from(&source.verbs);
        self.points.clone_from(&source.points);
    }
}

impl Path {
    /// An empty path.
    #[must_use]
    pub const fn new() -> Path {
        Path { verbs: Vec::new(), points: Vec::new() }
    }

    /// An empty path with room for `verbs` verbs and `points` points, so drawing that many allocates nothing more.
    #[must_use]
    pub fn with_capacity(verbs: usize, points: usize) -> Path {
        Path { verbs: Vec::with_capacity(verbs), points: Vec::with_capacity(points) }
    }

    /// Whether nothing was drawn into it.
    pub const fn is_empty(&self) -> bool {
        self.verbs.is_empty()
    }

    /// Its verbs and, in one list, the points they take.
    pub fn parts(&self) -> (&[Verb], &[Point]) {
        (&self.verbs, &self.points)
    }

    /// Each verb with the points it takes, as [`PathView::segments`] gives them.
    pub fn segments(&self) -> impl ExactSizeIterator<Item = (Verb, &[Point])> + FusedIterator + Clone + '_ {
        self.view().segments()
    }

    /// The path, borrowed.
    pub fn view(&self) -> PathView<'_> {
        PathView { verbs: &self.verbs, points: &self.points }
    }

    /// The bytes it holds: one a verb and eight a point.
    pub const fn cost(&self) -> usize {
        self.verbs.len() + self.points.len() * core::mem::size_of::<Point>()
    }

    pub(crate) fn reserve(&mut self, verbs: usize, points: usize) {
        self.verbs.reserve(verbs);
        self.points.reserve(points);
    }

    pub(crate) fn truncate(&mut self, verbs: usize, points: usize) {
        self.verbs.truncate(verbs);
        self.points.truncate(points);
    }

    pub(crate) fn extend_from(&mut self, path: PathView<'_>) {
        self.verbs.extend_from_slice(path.verbs);
        self.points.extend_from_slice(path.points);
    }

    /// Empties it, keeping its room, so a path drawn into again allocates only to grow.
    pub fn clear(&mut self) {
        self.verbs.clear();
        self.points.clear();
    }

    #[cfg(feature = "std")]
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        heap_of(&self.verbs) + heap_of(&self.points)
    }

    pub(crate) fn shrink_to_fit(&mut self) {
        self.verbs.shrink_to_fit();
        self.points.shrink_to_fit();
    }

    /// Draws it onto a pen, through a 2×3 transform `[a, b, c, d, dx, dy]` if given; every contour reaches the pen
    /// closed, as a fill reads it.
    pub fn replay(&self, transform: Option<&[f64; 6]>, pen: &mut (impl OutlinePen + ?Sized)) {
        self.view().replay(transform, pen);
    }

    /// The tight box it draws, its curves' extremes included; none for a path with no points.
    pub fn bounds(&self) -> Option<Bounds> {
        self.view().bounds()
    }
}

/// A path borrowed from where it's kept, as a display list keeps its own.
// It shows as the path it is.
#[derive(Clone, Copy, PartialEq)]
pub struct PathView<'a> {
    verbs: &'a [Verb],
    points: &'a [Point],
}

impl core::fmt::Debug for PathView<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Path").field("verbs", &self.verbs).field("points", &self.points).finish()
    }
}

impl<'a> PathView<'a> {
    pub(crate) fn new(verbs: &'a [Verb], points: &'a [Point]) -> Self {
        PathView { verbs, points }
    }

    /// Whether nothing was drawn into it.
    pub const fn is_empty(&self) -> bool {
        self.verbs.is_empty()
    }

    /// Its verbs and, in one list, the points they take.
    pub const fn parts(&self) -> (&'a [Verb], &'a [Point]) {
        (self.verbs, self.points)
    }

    /// Each verb with the points it takes, in order: the end point for a move or a line, the control point and
    /// then the end for a quadratic, both control points and then the end for a cubic, and none for a close.
    ///
    /// ```
    /// use daegun::outline::{OutlinePen, Path, Verb};
    ///
    /// let mut path = Path::default();
    /// path.move_to(0.0, 0.0);
    /// path.quad_to(5.0, 10.0, 10.0, 0.0);
    /// path.close();
    /// let ends: Vec<_> = path.segments().filter_map(|(_, points)| points.last()).collect();
    /// assert_eq!(path.segments().map(|(verb, _)| verb).collect::<Vec<_>>(), [Verb::Move, Verb::Quad, Verb::Close]);
    /// assert_eq!((ends[1].x, ends[1].y), (10.0, 0.0));
    /// ```
    pub fn segments(&self) -> impl ExactSizeIterator<Item = (Verb, &'a [Point])> + FusedIterator + Clone + use<'a> {
        let mut points = self.points;
        self.verbs.iter().map(move |&verb| {
            let (taken, rest) = points.split_at_checked(verb.points()).unwrap_or((points, &[]));
            points = rest;
            (verb, taken)
        })
    }

    /// The path, copied.
    pub fn to_path(self) -> Path {
        Path { verbs: self.verbs.to_vec(), points: self.points.to_vec() }
    }

    // The same into a path whose room is kept, as C's spare path is.
    #[cfg(feature = "capi")]
    pub(crate) fn copy_into(&self, path: &mut Path) {
        path.clear();
        path.verbs.extend_from_slice(self.verbs);
        path.points.extend_from_slice(self.points);
    }

    /// Draws it onto a pen, as [`Path::replay`] does.
    pub fn replay(&self, transform: Option<&[f64; 6]>, pen: &mut (impl OutlinePen + ?Sized)) {
        match transform {
            Some(&t) => self.replay_into(&mut TransformPen::new(pen, t)),
            None => self.replay_into(pen),
        }
    }

    fn replay_into(&self, pen: &mut (impl OutlinePen + ?Sized)) {
        let mut points = self.points;
        let mut open = false;
        // Each verb takes its points off the front with one check; a path holds what its verbs take, so none fails.
        for &verb in self.verbs {
            match verb {
                // A path replays as it fills: every contour closed, each before the next begins.
                Verb::Move => {
                    let Some(([p], rest)) = points.split_first_chunk() else { break };
                    if open { pen.close(); }
                    pen.move_to(p.x, p.y);
                    (points, open) = (rest, true);
                }
                Verb::Line => {
                    let Some(([p], rest)) = points.split_first_chunk() else { break };
                    pen.line_to(p.x, p.y);
                    points = rest;
                }
                Verb::Quad => {
                    let Some(([c, p], rest)) = points.split_first_chunk() else { break };
                    pen.quad_to(c.x, c.y, p.x, p.y);
                    points = rest;
                }
                Verb::Cubic => {
                    let Some(([c1, c2, p], rest)) = points.split_first_chunk() else { break };
                    pen.curve_to(c1.x, c1.y, c2.x, c2.y, p.x, p.y);
                    points = rest;
                }
                Verb::Close => { pen.close(); open = false; }
            }
        }
        if open {
            pen.close();
        }
    }

    /// The tight box it draws, as [`Path::bounds`] gives it.
    pub fn bounds(&self) -> Option<Bounds> {
        // The closes a replay adds move no point the box holds.
        let mut b = TightBounds::default();
        self.replay_into(&mut b);
        b.finish()
    }
}

// The box of every point a pen is given, control points included: what glyf's own box holds and what
// a CFF glyph's extents report; and how many points it was given.
#[derive(Default)]
pub(crate) struct PointBounds {
    min: (f32, f32),
    max: (f32, f32),
    any: bool,
    pub(crate) points: usize,
}

impl PointBounds {
    fn add(&mut self, x: f32, y: f32) {
        self.points += 1;
        if !self.any {
            (self.any, self.min, self.max) = (true, (x, y), (x, y));
            return;
        }
        self.min = (self.min.0.min(x), self.min.1.min(y));
        self.max = (self.max.0.max(x), self.max.1.max(y));
    }

    // (x_min, y_min, x_max, y_max), None where no point came.
    pub(crate) fn finish(&self) -> Option<(f32, f32, f32, f32)> {
        self.any.then_some((self.min.0, self.min.1, self.max.0, self.max.1))
    }
}

impl OutlinePen for PointBounds {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.add(cx, cy);
        self.add(x, y);
    }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.add(c1x, c1y);
        self.add(c2x, c2y);
        self.add(x, y);
    }
    fn close(&mut self) {}
}

// The tight box of an outline as it is drawn: each segment's end points and its curve's extremes
// between them. A pen, so a box needs no path held.
#[derive(Default)]
pub(crate) struct TightBounds {
    e: Extent,
    cur: (f64, f64),
    start: (f64, f64),
}

impl TightBounds {
    pub(crate) fn finish(self) -> Option<Bounds> {
        let (x_min, y_min, x_max, y_max) = self.e.finish()?;
        Some(Bounds { x_min, y_min, x_max, y_max })
    }
}

impl OutlinePen for TightBounds {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = (f64::from(x), f64::from(y));
        self.e.point(p);
        self.cur = p;
        self.start = p;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = (f64::from(x), f64::from(y));
        self.e.point(p);
        self.cur = p;
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (c, p) = ((f64::from(cx), f64::from(cy)), (f64::from(x), f64::from(y)));
        self.e.point(p);
        for axis in 0..2 {
            let (v0, v1, v2) = (at_axis(self.cur, axis), at_axis(c, axis), at_axis(p, axis));
            let den = v0 - 2.0 * v1 + v2;
            if den == 0.0 { continue; }
            let t = (v0 - v1) / den;
            if t > 0.0 && t < 1.0 {
                let u = 1.0 - t;
                self.e.axis(axis, u * u * v0 + 2.0 * u * t * v1 + t * t * v2);
            }
        }
        self.cur = p;
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let c1 = (f64::from(c1x), f64::from(c1y));
        let c2 = (f64::from(c2x), f64::from(c2y));
        let p = (f64::from(x), f64::from(y));
        self.e.point(p);
        for axis in 0..2 {
            let v0 = at_axis(self.cur, axis);
            let (v1, v2, v3) = (at_axis(c1, axis), at_axis(c2, axis), at_axis(p, axis));
            // Controls between the ends keep the curve between them too, and both ends are counted.
            let ends = v0.min(v3)..=v0.max(v3);
            if ends.contains(&v1) && ends.contains(&v2) { continue; }
            let (d0, d1, d2) = (v1 - v0, v2 - v1, v3 - v2);
            let (ts, nt) = roots(d0 - 2.0 * d1 + d2, 2.0 * (d1 - d0), d0);
            for &t in &ts[..nt] {
                let u = 1.0 - t;
                self.e.axis(axis, u * u * u * v0 + 3.0 * u * u * t * v1 + 3.0 * u * t * t * v2 + t * t * t * v3);
            }
        }
        self.cur = p;
    }

    fn close(&mut self) {
        self.cur = self.start;
    }
}

#[derive(Default)]
struct Extent {
    seen: bool,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Extent {
    fn axis(&mut self, axis: usize, v: f64) {
        let (lo, hi) = if axis == 0 { (&mut self.x0, &mut self.x1) } else { (&mut self.y0, &mut self.y1) };
        if *lo > v { *lo = v; }
        if *hi < v { *hi = v; }
    }
    fn point(&mut self, (x, y): (f64, f64)) {
        if !self.seen {
            self.seen = true;
            self.x0 = x; self.x1 = x; self.y0 = y; self.y1 = y;
            return;
        }
        self.axis(0, x);
        self.axis(1, y);
    }
    fn finish(self) -> Option<(f64, f64, f64, f64)> {
        self.seen.then_some((self.x0, self.y0, self.x1, self.y1))
    }
}

fn at_axis(p: (f64, f64), axis: usize) -> f64 {
    if axis == 0 { p.0 } else { p.1 }
}

fn roots(a: f64, b: f64, c: f64) -> ([f64; 2], usize) {
    let mut out = [0.0f64; 2];
    let mut n = 0usize;

    if a == 0.0 {
        if b != 0.0 {
            let t = -c / b;
            if t > 0.0 && t < 1.0 {
                out[n] = t;
                n += 1;
            }
        }
        return (out, n);
    }

    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return (out, n);
    }
    let s = disc.sqrt();
    for t in [(-b + s) / (2.0 * a), (-b - s) / (2.0 * a)] {
        if t > 0.0 && t < 1.0 {
            out[n] = t;
            n += 1;
        }
    }
    (out, n)
}

impl OutlinePen for Path {
    fn move_to(&mut self, x: f32, y: f32) {
        self.verbs.push(Verb::Move);
        self.points.push(Point { x, y });
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.verbs.push(Verb::Line);
        self.points.push(Point { x, y });
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.verbs.push(Verb::Quad);
        self.points.extend([Point { x: cx, y: cy }, Point { x, y }]);
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.verbs.push(Verb::Cubic);
        self.points.extend([Point { x: a, y: b }, Point { x: c, y: d }, Point { x, y }]);
    }
    fn close(&mut self) {
        self.verbs.push(Verb::Close);
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/outline/path.rs"]
mod tests;
