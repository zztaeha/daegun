#[cfg(doc)]
use crate::ErrorKind::{InvalidArgument, LimitExceeded, Overflow};
use alloc::vec::Vec;
use core::f32::consts::PI;
use core::f64::consts::SQRT_2;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;
use crate::daecore::daekit::float::{atan2, sin_cos};

use super::flatten::MAX_FLATTEN_POINTS;
use super::{OutlinePen, Path, Point, Verb};
use crate::daecore::daekit::base::error::Result;
use crate::daecore::{Error, ErrorKind};

/// How a stroke turns a corner.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Join {
    /// A sharp point, beveled where it would reach past `limit` times half the width.
    Miter {
        /// How far the point may reach, in half widths; one not above 0 means 4.
        limit: f32,
    },
    /// Rounded.
    Round,
    /// Cut straight across.
    Bevel,
}

/// How a stroke ends an open contour.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cap {
    /// Square, at the end.
    Butt,
    /// Rounded, past the end by half the width.
    Round,
    /// Square, past the end by half the width.
    Square,
}

/// How a path is stroked.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrokeStyle {
    /// The stroke's width, in the path's units: its sign is ignored, and 0 draws nothing.
    pub width: f32,
    /// How it turns corners.
    pub join: Join,
    /// How it ends open contours.
    pub cap: Cap,
}

impl Default for StrokeStyle {
    fn default() -> StrokeStyle {
        StrokeStyle::new(1.0)
    }
}

impl StrokeStyle {
    /// A stroke this wide, mitered at 4 half widths and butt-capped, as the default is at a width of 1.
    #[must_use]
    pub const fn new(width: f32) -> StrokeStyle {
        StrokeStyle { width, join: Join::Miter { limit: DEFAULT_MITER_LIMIT }, cap: Cap::Butt }
    }

    /// This style with a join.
    #[must_use]
    pub const fn with_join(mut self, join: Join) -> StrokeStyle {
        self.join = join;
        self
    }

    /// This style with a cap.
    #[must_use]
    pub const fn with_cap(mut self, cap: Cap) -> StrokeStyle {
        self.cap = cap;
        self
    }
}

// What a miter limit not above 0, or NaN, stands for.
const DEFAULT_MITER_LIMIT: f32 = 4.0;

type Pt = (f32, f32);

impl Path {
    /// Strokes the path onto a pen as filled contours, each point reaching the pen as it's worked out, so a stroke of
    /// any size draws. `tolerance` is how far the flattened curves may sit from the true ones, in the path's units;
    /// one not above zero, or NaN, means 0.1.
    ///
    /// # Errors
    ///
    /// Drawing nothing: [`InvalidArgument`] for a width not finite, and [`Overflow`] for a point not finite, as a
    /// miter's tip overflows long before the width does.
    pub fn stroke(&self, style: &StrokeStyle, tolerance: f32, pen: &mut dyn OutlinePen) -> Result<()> {
        Stroke::checked(self, style, tolerance)?.draw(pen);
        Ok(())
    }

    /// Strokes the path as [`stroke`](Path::stroke) does, its self-intersections resolved into one boundary, for a
    /// filler that doesn't do nonzero winding: a stroke overlaps itself at every join.
    ///
    /// # Errors
    ///
    /// Drawing nothing: as [`stroke`](Path::stroke) gives them, and [`LimitExceeded`] past [`MAX_FLATTEN_POINTS`]
    /// points, as the stroke is held whole to be resolved.
    pub fn stroke_simplified(&self, style: &StrokeStyle, tolerance: f32, pen: &mut dyn OutlinePen) -> Result<()> {
        let mut whole = Collect::default();
        Stroke::checked(self, style, tolerance)?.draw(&mut whole);
        if whole.points > MAX_FLATTEN_POINTS {
            return Err(Error::limit("the stroke would take more than MAX_FLATTEN_POINTS points"));
        }
        for c in super::simplify::union(&whole.contours) {
            pen.move_to(c[0].0, c[0].1);
            for &(x, y) in &c[1..] {
                pen.line_to(x, y);
            }
            pen.close();
        }
        Ok(())
    }
}

// What a stroke draws: the stroke itself, or the ring an embolden adds around the fill.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ink {
    Stroke,
    Embolden,
}

// A stroke ready to draw, every point it reaches checked to fit. Past HOLD it keeps a point a segment, flattening each
// again as it's walked, so no allocation grows with the stroke.
pub(crate) struct Stroke<'p> {
    path: &'p Path,
    style: StrokeStyle,
    r: f32,
    tolerance: f32,
    turned: bool,
    work: Work,
}

// The contours, the point each segment's walk starts from, the contours' points while they fit HOLD with each one's
// normal to the next, and one segment, or one corner or end, at a time.
#[derive(Default)]
struct Work {
    contours: Vec<Contour>,
    states: Vec<Pt>,
    held: Vec<Pt>,
    normals: Vec<Pt>,
    flat: Vec<Pt>,
    kept: Vec<Pt>,
    piece: Vec<Pt>,
}

// The points a stroke holds with their normals rather than flatten again, all a glyph's at text sizes: 1 MiB.
const HOLD: usize = 1 << 16;

// A contour's start, verbs, points and first state, where its points are held if they are, and how many it keeps;
// `dropped` where a closed one's last repeated its first, `last` the last kept.
#[derive(Clone, Copy)]
struct Contour {
    start: Pt,
    verbs: (usize, usize),
    points: (usize, usize),
    state: usize,
    held: Option<usize>,
    closed: bool,
    kept: usize,
    dropped: bool,
    last: Pt,
}

// The lists the thread's last stroke dropped, emptied, lent to its next; one past `SPARE_BYTES` is dropped.
#[cfg(feature = "std")]
std::thread_local! {
    static SPARE: core::cell::Cell<Option<Work>> = const { core::cell::Cell::new(None) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE.try_with(core::cell::Cell::take);
}

#[cfg(feature = "std")]
const SPARE_BYTES: usize = 64 << 10;

// Into an empty slot only: one that holds lists keeps them, so no stroke's drop sets it again.
impl Drop for Stroke<'_> {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        let _ = SPARE.try_with(|slot| {
            use crate::daecore::daekit::base::sync::heap_of;
            let held = slot.take();
            let Work { contours, states, held: points, normals, flat, kept, piece } = &mut self.work;
            let bytes = heap_of(contours) + heap_of(states) + heap_of(points) + heap_of(normals) + heap_of(flat)
                + heap_of(kept) + heap_of(piece);
            if held.is_some() || bytes > SPARE_BYTES {
                return slot.set(held);
            }
            contours.clear();
            states.clear();
            points.clear();
            normals.clear();
            flat.clear();
            kept.clear();
            piece.clear();
            slot.set(Some(core::mem::take(&mut self.work)));
        });
    }
}

impl<'p> Stroke<'p> {
    // The stroke of `path`, or None where a point it reaches would fail `fits`. The path's reach settles that for any
    // stroke not near f32's own range; one that is gets worked out once without drawing.
    pub(crate) fn new(
        path: &'p Path,
        style: &StrokeStyle,
        tolerance: f32,
        ink: Ink,
        fits: impl Fn(Pt) -> bool,
    ) -> Option<Stroke<'p>> {
        #[cfg(feature = "std")]
        let work = SPARE.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_default();
        #[cfg(not(feature = "std"))]
        let work = Work::default();
        let r = style.width.abs() * 0.5;
        let tolerance = if tolerance > 0.0 { tolerance } else { 0.1 };
        let mut s = Stroke { path, style: *style, r, tolerance, turned: false, work };
        if !(r > 0.0 && r.is_finite()) {
            return Some(s);
        }
        let fill = s.work.scan(path, tolerance, ink == Ink::Embolden);
        // Every loop winds clockwise, the rings by construction and the dots by choice, so an embolden's turn where the
        // fill winds the other way: a non-zero fill cancels where they disagree, and CFF winds against TrueType.
        s.turned = ink == Ink::Embolden && fill > 0.0;
        if !reach(path, style, r).is_some_and(|far| fits((far as f32, far as f32))) {
            let mut check = Check { fits, ok: true };
            s.walk(&mut check);
            if !check.ok {
                return None;
            }
        }
        Some(s)
    }

    pub(crate) fn checked(path: &'p Path, style: &StrokeStyle, tolerance: f32) -> Result<Stroke<'p>> {
        if !style.width.is_finite() {
            return Err(Error::invalid("the stroke width must be finite"));
        }
        // Checked here, as a NaN point reads as a repeat of the one before and would be dropped unseen.
        if !path.parts().1.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
            return Err(Error::new(ErrorKind::Overflow, "a point of the path is not finite"));
        }
        Stroke::new(path, style, tolerance, Ink::Stroke, |p| p.0.is_finite() && p.1.is_finite())
            .ok_or(Error::new(ErrorKind::Overflow, "a point of the stroke is not finite"))
    }

    pub(crate) fn draw(mut self, pen: &mut (impl OutlinePen + ?Sized)) {
        self.walk(&mut Emit { pen, head: [(0.0, 0.0); 3], count: 0, last: (0.0, 0.0), held: (0.0, 0.0) });
    }

    // Each contour's loops into `sink`: a closed one's two sides, an open one's way out and back, a lone point's dot.
    fn walk(&mut self, sink: &mut impl Sink) {
        let (verbs, points) = self.path.parts();
        let Work { contours, states, held, normals, flat, kept, piece } = &mut self.work;
        let mut d = Draw { sink, piece, style: self.style, r: self.r, tolerance: self.tolerance };
        for c in contours.iter() {
            let w = Walk { verbs, points, c, states, held, normals, tolerance: self.tolerance };
            let count = c.kept - usize::from(c.dropped);
            if count == 1 && !c.closed {
                d.sink.begin();
                d.dot(c.start, self.turned);
                d.sink.end();
            } else if count >= 2 && c.closed {
                d.ring(&w, flat, kept, self.turned);
            } else if count >= 2 {
                d.open(&w, flat, kept, count, self.turned);
            }
        }
    }
}

impl Work {
    // The path's contours as it reads them: a move starts one, a segment with none open starts one where the last
    // ended, a close ends one. Gives the closed contours' signed area where `area` asks for it.
    fn scan(&mut self, path: &Path, tolerance: f32, area: bool) -> f64 {
        let (verbs, points) = path.parts();
        let (mut fill, mut at, mut i) = (0.0, (0.0, 0.0), 0);
        let mut open: Option<Open> = None;
        for (v, &verb) in verbs.iter().enumerate() {
            match verb {
                Verb::Move => {
                    if let Some(o) = open.take() {
                        fill += self.finish(o, v, i, false);
                    }
                    at = points[i].into();
                    i += 1;
                    open = Some(self.open(at, v + 1, i));
                }
                Verb::Close => {
                    if let Some(o) = open.take() {
                        at = o.start;
                        fill += self.finish(o, v, i, true);
                    }
                }
                Verb::Line | Verb::Quad | Verb::Cubic => {
                    let o = match &mut open {
                        Some(o) => o,
                        None => open.insert(self.open(at, v, i)),
                    };
                    self.states.push(o.last);
                    let n = arity(verb);
                    flatten(verb, at, &points[i..i + n], tolerance, &mut self.flat);
                    for &p in &self.flat {
                        if o.keep(p, area)
                            && let Some(h) = o.held
                        {
                            if self.held.len() < HOLD {
                                self.held.push(p);
                            } else {
                                self.held.truncate(h);
                                o.held = None;
                            }
                        }
                    }
                    at = points[i + n - 1].into();
                    i += n;
                }
            }
        }
        if let Some(o) = open.take() {
            fill += self.finish(o, verbs.len(), i, false);
        }
        fill
    }

    // A contour opening at `start`, its points held while HOLD has room.
    fn open(&mut self, start: Pt, verbs: usize, points: usize) -> Open {
        let held = (self.held.len() < HOLD).then(|| {
            self.held.push(start);
            self.held.len() - 1
        });
        let state = self.states.len();
        Open { start, verbs, points, state, held, kept: 1, last: start, before: start, sum: 0.0, term: 0.0 }
    }

    fn finish(&mut self, o: Open, verbs: usize, points: usize, closed: bool) -> f64 {
        let dropped = closed && o.kept > 1 && repeats(o.start, o.last);
        let last = if dropped { o.before } else { o.last };
        if let Some(h) = o.held {
            if dropped {
                self.held.pop();
            }
            let end = self.held.len();
            for k in h..end {
                self.normals.push(normal(self.held[k], self.held[if k + 1 < end { k + 1 } else { h }]));
            }
        }
        let c = Contour {
            start: o.start,
            verbs: (o.verbs, verbs),
            points: (o.points, points),
            state: o.state,
            held: o.held,
            closed,
            kept: o.kept,
            dropped,
            last,
        };
        self.contours.push(c);
        if !(closed && o.kept - usize::from(dropped) >= 2) {
            return 0.0;
        }
        let sum = if dropped { o.sum } else { o.sum + o.term };
        sum + cross(last, o.start)
    }
}

// A contour being scanned: the points it keeps, counted, the last two, and its area's terms so far, the latest kept
// back until the next point shows it isn't the last, which a closed contour drops where it repeats the first.
struct Open {
    start: Pt,
    verbs: usize,
    points: usize,
    state: usize,
    held: Option<usize>,
    kept: usize,
    last: Pt,
    before: Pt,
    sum: f64,
    term: f64,
}

impl Open {
    // Whether `p` is kept, as it isn't where it repeats the last.
    fn keep(&mut self, p: Pt, area: bool) -> bool {
        if !distinct(p, self.last) {
            return false;
        }
        if area {
            if self.kept > 1 {
                self.sum += self.term;
            }
            self.term = cross(self.last, p);
        }
        self.kept += 1;
        self.before = self.last;
        self.last = p;
        true
    }
}

fn cross(a: Pt, b: Pt) -> f64 {
    f64::from(a.0) * f64::from(b.1) - f64::from(b.0) * f64::from(a.1)
}

// One contour's kept points, walked either way: as held, or each segment flattened again as it's reached. Backward, a
// segment's points are kept by replaying the walk from its state, so both ways keep the same points.
struct Walk<'a> {
    verbs: &'a [Verb],
    points: &'a [Point],
    c: &'a Contour,
    states: &'a [Pt],
    held: &'a [Pt],
    normals: &'a [Pt],
    tolerance: f32,
}

// Each point comes with the normal of the segment from the point before it in the walk, as it runs forward.
impl Walk<'_> {
    fn forward(&self, flat: &mut Vec<Pt>, mut f: impl FnMut(Pt, Pt)) {
        let c = self.c;
        if let Some(h) = c.held {
            let end = h + c.kept - usize::from(c.dropped);
            f(self.held[h], (0.0, 0.0));
            return (h + 1..end).for_each(|k| f(self.held[k], self.normals[k - 1]));
        }
        let (mut last, mut index) = (c.start, 1);
        f(c.start, (0.0, 0.0));
        let mut before = c.start;
        let mut f = |p: Pt| {
            f(p, normal(before, p));
            before = p;
        };
        let (mut at, mut i) = (c.start, c.points.0);
        for &v in &self.verbs[c.verbs.0..c.verbs.1] {
            let n = arity(v);
            flatten(v, at, &self.points[i..i + n], self.tolerance, flat);
            for &p in flat.iter() {
                if distinct(p, last) {
                    last = p;
                    if !(c.dropped && index == c.kept - 1) {
                        f(p);
                    }
                    index += 1;
                }
            }
            at = self.points[i + n - 1].into();
            i += n;
        }
    }

    fn backward(&self, flat: &mut Vec<Pt>, kept: &mut Vec<Pt>, mut f: impl FnMut(Pt, Pt)) {
        let c = self.c;
        if let Some(h) = c.held {
            let end = h + c.kept - usize::from(c.dropped);
            f(self.held[end - 1], (0.0, 0.0));
            return (h..end - 1).rev().for_each(|k| f(self.held[k], self.normals[k]));
        }
        let mut after = None;
        let mut f = |p: Pt| {
            f(p, after.map_or((0.0, 0.0), |q| normal(p, q)));
            after = Some(p);
        };
        let (mut index, mut end) = (c.kept, c.points.1);
        for (k, &v) in self.verbs[c.verbs.0..c.verbs.1].iter().enumerate().rev() {
            let begin = end - arity(v);
            let at = if k == 0 { c.start } else { self.points[begin - 1].into() };
            flatten(v, at, &self.points[begin..end], self.tolerance, flat);
            kept.clear();
            let mut last = self.states[c.state + k];
            for &p in flat.iter() {
                if distinct(p, last) {
                    last = p;
                    kept.push(p);
                }
            }
            for &p in kept.iter().rev() {
                index -= 1;
                if !(c.dropped && index == c.kept - 1) {
                    f(p);
                }
            }
            end = begin;
        }
        f(c.start);
    }
}

// Where the loops go: drawn, or checked.
trait Sink {
    fn begin(&mut self) {}
    fn piece(&mut self, piece: &[Pt]);
    fn end(&mut self) {}
}

// Each loop onto the pen as it comes, repeats dropped, a closing point that repeats the first left off, and one of
// fewer than three points left out: the first three are held until a fourth shows the loop draws.
struct Emit<'a, P: ?Sized> {
    pen: &'a mut P,
    head: [Pt; 3],
    count: usize,
    last: Pt,
    held: Pt,
}

impl<P: OutlinePen + ?Sized> Sink for Emit<'_, P> {
    fn begin(&mut self) {
        self.count = 0;
    }

    // The state in locals while the pen is called, which could otherwise reload it for every point.
    fn piece(&mut self, piece: &[Pt]) {
        let (mut count, mut last, mut held) = (self.count, self.last, self.held);
        for &p in piece {
            if count > 0 && !distinct(p, last) {
                continue;
            }
            last = p;
            match count {
                0..=2 => self.head[count] = p,
                3 => {
                    self.pen.move_to(self.head[0].0, self.head[0].1);
                    self.pen.line_to(self.head[1].0, self.head[1].1);
                    self.pen.line_to(self.head[2].0, self.head[2].1);
                    held = p;
                }
                _ => {
                    self.pen.line_to(held.0, held.1);
                    held = p;
                }
            }
            count += 1;
        }
        (self.count, self.last, self.held) = (count, last, held);
    }

    fn end(&mut self) {
        if self.count > 3 {
            if !repeats(self.head[0], self.held) {
                self.pen.line_to(self.held.0, self.held.1);
            }
            self.pen.close();
            return;
        }
        let mut count = self.count;
        if count > 1 && repeats(self.head[0], self.head[count - 1]) {
            count -= 1;
        }
        if count == 3 {
            self.pen.move_to(self.head[0].0, self.head[0].1);
            self.pen.line_to(self.head[1].0, self.head[1].1);
            self.pen.line_to(self.head[2].0, self.head[2].1);
            self.pen.close();
        }
    }
}

struct Check<F> {
    fits: F,
    ok: bool,
}

impl<F: Fn(Pt) -> bool> Sink for Check<F> {
    fn piece(&mut self, piece: &[Pt]) {
        self.ok &= piece.iter().all(|&p| (self.fits)(p));
    }
}

// The stroke's loops, held to be resolved together; past MAX_FLATTEN_POINTS it only counts them.
#[derive(Default)]
struct Collect {
    contours: Vec<Vec<Pt>>,
    points: usize,
}

impl OutlinePen for Collect {
    fn move_to(&mut self, x: f32, y: f32) {
        self.points += 1;
        if self.points <= MAX_FLATTEN_POINTS {
            self.contours.push(alloc::vec![(x, y)]);
        }
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.points += 1;
        if self.points <= MAX_FLATTEN_POINTS
            && let Some(c) = self.contours.last_mut()
        {
            c.push((x, y));
        }
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
        self.line_to(x, y);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.line_to(x, y);
    }
    fn close(&mut self) {}
}

// Each corner, end and dot as it's worked out, into the sink, backward where its loop is turned.
struct Draw<'a, S> {
    sink: &'a mut S,
    piece: &'a mut Vec<Pt>,
    style: StrokeStyle,
    r: f32,
    tolerance: f32,
}

impl<S: Sink> Draw<'_, S> {
    // Out of line, so each walk's closure calling it is small enough to inline: a tenth off a stroke at text sizes.
    #[inline(never)]
    fn join(&mut self, prev: Pt, at: Pt, next: Pt, n_in: Pt, n_out: Pt, back: bool) {
        self.piece.clear();
        push_join(self.piece, prev, at, next, n_in, n_out, self.r, &self.style, self.tolerance);
        self.put(back);
    }

    fn offset(&mut self, at: Pt, n: Pt) {
        self.sink.piece(&[offset(at, n, self.r)]);
    }

    fn cap(&mut self, at: Pt, n: Pt, dir: Pt, back: bool) {
        self.piece.clear();
        push_cap(self.piece, at, n, dir, self.r, &self.style, self.tolerance);
        self.put(back);
    }

    fn dot(&mut self, at: Pt, back: bool) {
        self.piece.clear();
        dot(self.piece, at, self.r, &self.style, self.tolerance);
        self.put(back);
    }

    fn put(&mut self, back: bool) {
        if back {
            self.piece.reverse();
        }
        self.sink.piece(self.piece);
    }

    // A closed contour's two loops: the side to the left of its walk forward, corner by corner, then the side to the
    // left of its walk back. Turned, each is drawn from its end, its corners walked the other way.
    fn ring(&mut self, w: &Walk, flat: &mut Vec<Pt>, kept: &mut Vec<Pt>, turned: bool) {
        let (first, last) = (w.c.start, w.c.last);
        self.sink.begin();
        let mut at = None;
        if turned {
            let (mut next, mut n_out) = (first, None);
            w.backward(flat, kept, |p, n_in| {
                if let Some(at) = at {
                    let n = n_out.unwrap_or_else(|| normal(at, next));
                    self.join(mid(p, at), at, mid(at, next), n_in, n, true);
                    (next, n_out) = (at, Some(n_in));
                }
                at = Some(p);
            });
            let n_out = n_out.unwrap_or_else(|| normal(first, next));
            self.join(mid(last, first), first, mid(first, next), normal(last, first), n_out, true);
        } else {
            let (mut prev, mut n_in) = (last, None);
            w.forward(flat, |p, n_out| {
                if let Some(at) = at {
                    let n = n_in.unwrap_or_else(|| normal(prev, at));
                    self.join(mid(prev, at), at, mid(at, p), n, n_out, false);
                    (prev, n_in) = (at, Some(n_out));
                }
                at = Some(p);
            });
            let n_in = n_in.unwrap_or_else(|| normal(prev, last));
            self.join(mid(prev, last), last, mid(last, first), n_in, normal(last, first), false);
        }
        self.sink.end();

        self.sink.begin();
        let mut at = None;
        if turned {
            let (mut next, mut n_out) = (last, None);
            w.forward(flat, |p, n| {
                if let Some(at) = at {
                    let n_in = neg(n);
                    let n = n_out.unwrap_or_else(|| neg(normal(next, at)));
                    self.join(mid(p, at), at, mid(at, next), n_in, n, true);
                    (next, n_out) = (at, Some(n_in));
                }
                at = Some(p);
            });
            let n_out = n_out.unwrap_or_else(|| neg(normal(next, last)));
            self.join(mid(first, last), last, mid(last, next), neg(normal(last, first)), n_out, true);
        } else {
            let (mut prev, mut n_in) = (first, None);
            w.backward(flat, kept, |p, n| {
                if let Some(at) = at {
                    let n_out = neg(n);
                    let n = n_in.unwrap_or_else(|| neg(normal(at, prev)));
                    self.join(mid(prev, at), at, mid(at, p), n, n_out, false);
                    (prev, n_in) = (at, Some(n_out));
                }
                at = Some(p);
            });
            let n_in = n_in.unwrap_or_else(|| neg(normal(first, prev)));
            self.join(mid(prev, first), first, mid(first, last), n_in, neg(normal(last, first)), false);
        }
        self.sink.end();
    }

    // An open contour's one loop: out along its left side, round its end, back along its other side and round its
    // start. Turned, the same pieces from the start's cap back, each drawn backward.
    fn open(&mut self, w: &Walk, flat: &mut Vec<Pt>, kept: &mut Vec<Pt>, count: usize, turned: bool) {
        self.sink.begin();
        let (mut seen, mut before, mut at, mut n_in) = (0, (0.0, 0.0), (0.0, 0.0), (0.0, 0.0));
        w.forward(flat, |p, n| {
            if seen == 1 {
                n_in = n;
                if turned {
                    self.cap(at, neg(n_in), direction(p, at), true);
                    self.offset(at, neg(n_in));
                } else {
                    self.offset(at, n_in);
                }
            } else if seen >= 2 {
                // The corner at `at`, trimmed no further back than halfway along a segment another corner shares.
                let n_out = n;
                let from = if seen == 2 { before } else { mid(before, at) };
                let to = if seen == count - 1 { p } else { mid(p, at) };
                if turned {
                    self.join(to, at, from, neg(n_out), neg(n_in), true);
                } else {
                    self.join(from, at, to, n_in, n_out, false);
                }
                n_in = n_out;
            }
            (before, at, seen) = (at, p, seen + 1);
        });
        let cut = self.style.cap != Cap::Round;
        if turned {
            if cut {
                self.offset(at, neg(n_in));
            }
            self.cap(at, n_in, direction(before, at), true);
            self.offset(at, n_in);
        } else {
            self.offset(at, n_in);
            self.cap(at, n_in, direction(before, at), false);
            if cut {
                self.offset(at, neg(n_in));
            }
        }

        let (mut seen, mut after, mut at, mut n_out) = (0, (0.0, 0.0), (0.0, 0.0), (0.0, 0.0));
        w.backward(flat, kept, |p, n| {
            if seen == 1 {
                n_out = n;
            } else if seen >= 2 {
                let n_in = n;
                let to = if seen == 2 { after } else { mid(after, at) };
                let from = if seen == count - 1 { p } else { mid(p, at) };
                if turned {
                    self.join(from, at, to, n_in, n_out, true);
                } else {
                    self.join(to, at, from, neg(n_out), neg(n_in), false);
                }
                n_out = n_in;
            }
            (after, at, seen) = (at, p, seen + 1);
        });
        if turned {
            self.offset(at, n_out);
        } else {
            self.offset(at, neg(n_out));
            self.cap(at, neg(n_out), direction(after, at), false);
        }
        self.sink.end();
    }
}

// How far from 0 a point of the stroke may lie: past the path's farthest coordinate, a radius for an offset, arc or cut
// corner, √2 for a square end, the limit for a miter's tip; doubled for rounding. None where a step on the way could
// pass f32: a segment's squared length, a miter's r / (2 cos²), a cut corner's r / (1 + cos), within 10^6 of a segment.
fn reach(path: &Path, style: &StrokeStyle, r: f32) -> Option<f64> {
    let mut far = 0.0f64;
    for p in path.parts().1 {
        if !(p.x.is_finite() && p.y.is_finite()) {
            return None;
        }
        far = far.max(f64::from(p.x.abs())).max(f64::from(p.y.abs()));
    }
    let r = f64::from(r);
    let (tip, scale) = match style.join {
        Join::Miter { limit } => {
            let limit = f64::from(if limit > 0.0 { limit } else { DEFAULT_MITER_LIMIT }).max(1.0);
            (limit.min(1e6), r * (limit * limit).min(1e12))
        }
        Join::Round | Join::Bevel => (1.0, 0.0),
    };
    let reach = 2.0 * (far + r * tip.max(SQRT_2));
    let steps = [reach, 2.0 * scale, 6e6 * far, 16.0 * far * far];
    steps.iter().all(|&v| v <= f64::from(f32::MAX)).then_some(reach)
}

// `prev` and `next` bound how far back along each segment an inner corner may be trimmed. Inlined into its one
// caller, `Draw::join`, so a corner's dozen floats are passed once: 9% of a stroke at text sizes.
#[allow(clippy::too_many_arguments, reason = "a corner is its two neighbors as well as itself")]
#[inline(always)]
fn push_join(
    out: &mut Vec<Pt>,
    prev: Pt,
    at: Pt,
    next: Pt,
    n_in: Pt,
    n_out: Pt,
    r: f32,
    style: &StrokeStyle,
    tolerance: f32,
) {
    let a = offset(at, n_in, r);
    let b = offset(at, n_out, r);
    let turn = n_in.0 * n_out.1 - n_in.1 * n_out.0;
    let cos_turn = n_in.0 * n_out.0 + n_in.1 * n_out.1;
    if turn.abs() < 1e-6 && cos_turn >= 0.0 {
        out.push(b);
        return;
    }
    // A turn back on itself has no inside and any miter exceeds its limit, so both sides end flat, or
    // round through the tip as two quarter arcs, since `arc` cannot pick a side at half a turn.
    if turn.abs() < 1e-6 {
        out.push(a);
        if style.join == Join::Round {
            let tip = offset(at, (n_in.1, -n_in.0), r);
            arc(out, at, a, tip, r, tolerance);
            arc(out, at, tip, b, r, tolerance);
        } else {
            out.push(b);
        }
        return;
    }
    // The offsets meet r * tan(half the turn) back along each segment (from the normals: intersecting loses a slight
    // turn). Past that, or where 1 + cos rounds to 0 at a fold, they have crossed and the side runs through the corner.
    if turn > 0.0 {
        let trim = r * turn / (1.0 + cos_turn);
        if 1.0 + cos_turn > 0.0 && trim <= dist(prev, at) && trim <= dist(at, next) {
            let k = r / (1.0 + cos_turn);
            out.push((at.0 + (n_in.0 + n_out.0) * k, at.1 + (n_in.1 + n_out.1) * k));
        } else {
            out.extend_from_slice(&[a, at, b]);
        }
        return;
    }
    out.push(a);
    // An arc ends on `b` itself; the other joins are followed by it.
    match style.join {
        Join::Bevel => {}
        Join::Round => return arc(out, at, a, b, r, tolerance),
        Join::Miter { limit } => {
            // The tip sits r / cos(half the turn) out along (sx, sy), whose length is 2 cos(half).
            let (sx, sy) = (n_in.0 + n_out.0, n_in.1 + n_out.1);
            let cos_half = ((sx * sx + sy * sy) * 0.25).sqrt();
            let limit = if limit > 0.0 { limit } else { DEFAULT_MITER_LIMIT };
            if cos_half > 1e-6 && 1.0 / cos_half <= limit.max(1.0) {
                let scale = r / (2.0 * cos_half * cos_half);
                out.push((at.0 + sx * scale, at.1 + sy * scale));
            }
        }
    }
    out.push(b);
}

fn push_cap(out: &mut Vec<Pt>, at: Pt, n: Pt, dir: Pt, r: f32, style: &StrokeStyle, tolerance: f32) {
    match style.cap {
        Cap::Butt => {}
        Cap::Square => {
            out.push((at.0 + (n.0 + dir.0) * r, at.1 + (n.1 + dir.1) * r));
            out.push((at.0 + (dir.0 - n.0) * r, at.1 + (dir.1 - n.1) * r));
        }
        Cap::Round => {
            let tip = offset(at, dir, r);
            arc(out, at, offset(at, n, r), tip, r, tolerance);
            arc(out, at, tip, offset(at, neg(n), r), r, tolerance);
        }
    }
}

// Clockwise, as every other loop's ink winds, so a dot on a line adds to it rather than canceling.
fn dot(out: &mut Vec<Pt>, at: Pt, r: f32, style: &StrokeStyle, tolerance: f32) {
    match style.cap {
        Cap::Butt => {}
        Cap::Round => {
            let q = [(r, 0.0), (0.0, -r), (-r, 0.0), (0.0, r)];
            for k in 0..4 {
                let from = (at.0 + q[k].0, at.1 + q[k].1);
                let to = (at.0 + q[(k + 1) % 4].0, at.1 + q[(k + 1) % 4].1);
                arc(out, at, from, to, r, tolerance);
            }
        }
        Cap::Square => out.extend_from_slice(&[
            (at.0 - r, at.1 - r),
            (at.0 - r, at.1 + r),
            (at.0 + r, at.1 + r),
            (at.0 + r, at.1 - r),
        ]),
    }
}

fn arc(out: &mut Vec<Pt>, center: Pt, a: Pt, b: Pt, r: f32, tolerance: f32) {
    let a0 = atan2((a.1 - center.1) as f64, (a.0 - center.0) as f64) as f32;
    let a1 = atan2((b.1 - center.1) as f64, (b.0 - center.0) as f64) as f32;
    let mut sweep = a1 - a0;
    while sweep > PI {
        sweep -= 2.0 * PI;
    }
    while sweep < -PI {
        sweep += 2.0 * PI;
    }
    let max_step = (8.0 * tolerance / r).sqrt();
    let steps = ((sweep.abs() / max_step).ceil() as usize).clamp(1, 256);
    for k in 1..steps {
        let t = a0 + sweep * (k as f32 / steps as f32);
        let (sin, cos) = sin_cos(t as f64);
        out.push((center.0 + r * cos as f32, center.1 + r * sin as f32));
    }
    out.push(b);
}


fn distinct(p: Pt, q: Pt) -> bool {
    (p.0 - q.0).abs() > 1e-6 || (p.1 - q.1).abs() > 1e-6
}

fn repeats(p: Pt, q: Pt) -> bool {
    (p.0 - q.0).abs() <= 1e-6 && (p.1 - q.1).abs() <= 1e-6
}


// A segment flattened from `at` into `out`: a line's end, or a curve's points as its length over the tolerance asks,
// 256 at most.
fn flatten(verb: Verb, at: Pt, p: &[Point], tolerance: f32, out: &mut Vec<Pt>) {
    out.clear();
    let pt = |k: usize| -> Pt { p[k].into() };
    match verb {
        Verb::Line => out.push(pt(0)),
        Verb::Quad => {
            let (c, e) = (pt(0), pt(1));
            let n = steps(dist(at, c) + dist(c, e), tolerance);
            for k in 1..=n {
                let t = k as f32 / n as f32;
                let u = 1.0 - t;
                out.push((
                    u * u * at.0 + 2.0 * u * t * c.0 + t * t * e.0,
                    u * u * at.1 + 2.0 * u * t * c.1 + t * t * e.1,
                ));
            }
        }
        Verb::Cubic => {
            let (c1, c2, e) = (pt(0), pt(1), pt(2));
            let n = steps(dist(at, c1) + dist(c1, c2) + dist(c2, e), tolerance);
            for k in 1..=n {
                let t = k as f32 / n as f32;
                let u = 1.0 - t;
                out.push((
                    u * u * u * at.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * e.0,
                    u * u * u * at.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * e.1,
                ));
            }
        }
        Verb::Move | Verb::Close => {}
    }
}

fn arity(verb: Verb) -> usize {
    match verb {
        Verb::Move | Verb::Line => 1,
        Verb::Quad => 2,
        Verb::Cubic => 3,
        Verb::Close => 0,
    }
}

fn direction(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len.is_finite() {
        return if len > 1e-9 { (dx / len, dy / len) } else { (1.0, 0.0) };
    }
    // The square overflows f32 past about 1.8e19, which would leave every normal (0, 0).
    let (dx, dy) = (f64::from(dx), f64::from(dy));
    let len = (dx * dx + dy * dy).sqrt();
    ((dx / len) as f32, (dy / len) as f32)
}

pub(super) fn normal(a: Pt, b: Pt) -> Pt {
    let d = direction(a, b);
    (-d.1, d.0)
}

fn mid(a: Pt, b: Pt) -> Pt {
    ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5)
}

fn neg(p: Pt) -> Pt {
    (-p.0, -p.1)
}

fn offset(at: Pt, n: Pt, r: f32) -> Pt {
    (at.0 + n.0 * r, at.1 + n.1 * r)
}


fn dist(a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    (dx * dx + dy * dy).sqrt()
}

fn steps(len: f32, tolerance: f32) -> usize {
    ((len / tolerance.max(1e-4)).sqrt().ceil() as usize).clamp(1, 256)
}
