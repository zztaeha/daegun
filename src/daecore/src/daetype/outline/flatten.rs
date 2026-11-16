#[cfg(doc)]
use crate::ErrorKind::{InvalidArgument, LimitExceeded};
use alloc::vec::Vec;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;

use super::pen::OutlinePen;
use super::path::{Path, Point, Verb};
use crate::daecore::daekit::base::error::Result;
use crate::daecore::Error;

const MIN_SEGMENT_SPAN: f32 = 1.0 / 4096.0;

/// The most points [`Path::flatten`] keeps, and [`Path::stroke_simplified`] holds to resolve: what a call hands back
/// or holds whole keeps a size, while a stroke drawn onto a pen streams at any size.
// A curve can become 4,097 points, so one glyph of 65,534 curves could otherwise come to 268 million.
pub const MAX_FLATTEN_POINTS: usize = 1_048_576;

// Closing points and contours too short to keep are read but not returned, so reading stops at twice
// the limit: a path that reads more is refused, however little it would keep.
const MAX_READ: usize = 2 * MAX_FLATTEN_POINTS;

/// The `max_area` [`Path::flatten`] takes to draw a path in font units at `px` pixels to the em, as 1.1.7's
/// rasterizer flattened.
// A px so small the area overflows, or one below zero, asks for the coarsest, which flatten would read as finest.
pub fn max_area_for(px: f32, units_per_em: f32) -> f32 {
    let area = 3.0 * 2.0 * (units_per_em / px);
    if area == f32::INFINITY || area.is_sign_negative() && !area.is_nan() { f32::MAX } else { area }
}

impl Path {
    /// The path as closed polygons, none under three points or repeating its first. A curve halves until twice the
    /// area of its ends-and-midpoint triangle is within `max_area`, one not finite and above zero halving all the
    /// way.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`] for a point not finite or past `f32::MAX / 16`, and [`LimitExceeded`] past
    /// [`MAX_FLATTEN_POINTS`] points kept or twice that read.
    pub fn flatten(&self, max_area: f32) -> Result<Vec<Vec<Point>>> {
        let mut contours = Vec::with_capacity(self.parts().0.iter().filter(|&&v| v == Verb::Move).count());
        self.flatten_into(max_area, &mut contours)?;
        Ok(contours)
    }

    /// The same into `contours`, each list it held reused, so lists kept between calls allocate only to grow;
    /// emptied on an error.
    ///
    /// # Errors
    ///
    /// As [`flatten`](Path::flatten) gives them.
    pub fn flatten_into(&self, max_area: f32, contours: &mut Vec<Vec<Point>>) -> Result<()> {
        let max_area = if max_area.is_finite() && max_area > 0.0 { max_area } else { 0.0 };
        let mut pen = Flattener {
            max_area,
            start: (0.0, 0.0),
            previous: (0.0, 0.0),
            points: Vec::new(),
            read: 0,
            contours,
            kept: 0,
            // The first contour's room, a guess from the path's own points rather than growing from four.
            room: self.parts().1.len(),
            in_range: true,
        };
        // Through the pen's table: inlined into the replay, the flattener took 0.4% longer.
        self.replay(None, &mut pen as &mut dyn OutlinePen);
        pen.end_contour();
        let (read, kept, in_range) = (pen.read, pen.kept, pen.in_range);
        contours.truncate(kept);
        let failed = if !in_range {
            Some(Error::invalid("a point is not finite or past f32::MAX / 16"))
        } else if read > MAX_READ || contours.iter().map(Vec::len).sum::<usize>() > MAX_FLATTEN_POINTS {
            Some(Error::limit("past MAX_FLATTEN_POINTS points kept or twice that read"))
        } else {
            None
        };
        match failed {
            Some(error) => {
                contours.clear();
                Err(error)
            }
            None => Ok(()),
        }
    }
}

type Pt = (f32, f32);

// A point on a curve is a weighted mean of its controls, and the weights' rounding can carry it past the
// largest of them, so near f32::MAX it could come out infinite. A NaN fails this too.
const LIMIT: f32 = f32::MAX / 16.0;

fn fits(v: &f32) -> bool {
    v.abs() <= LIMIT
}

// `points` is the contour being read, its list the caller's in slot `kept` where there is one, else one of `room`,
// the last kept contour's length; `read` counts every point read.
struct Flattener<'a> {
    max_area: f32,
    start: Pt,
    previous: Pt,
    points: Vec<Point>,
    read: usize,
    contours: &'a mut Vec<Vec<Point>>,
    kept: usize,
    room: usize,
    in_range: bool,
}

// A NaN counts as too curved: a curve past 1e19 across overflows its cross products to inf - inf, and
// splits until they fit rather than flattening to its chord.
fn too_curved(a: Pt, b: Pt, c: Pt, max_area: f32) -> bool {
    let area = ((b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)).abs();
    area > max_area || area.is_nan()
}

// The span⁴ under which a cubic piece past its midpoint check can't fail its quarter checks: its quarters lie within
// 3/64 span³ |Δ³| of the parabola through its ends and midpoint, whose quarter triangles are an eighth of its own, and
// its chord is under 3 span × its longest edge. Half of max_area is margin; f64 keeps a tiny curve's limit finite.
fn quarter_limit(p0: Pt, p1: Pt, p2: Pt, p3: Pt, max_area: f32) -> f32 {
    let w = |p: Pt| (f64::from(p.0), f64::from(p.1));
    let (p0, p1, p2, p3) = (w(p0), w(p1), w(p2), w(p3));
    let len2 = |p: (f64, f64), q: (f64, f64)| (q.0 - p.0) * (q.0 - p.0) + (q.1 - p.1) * (q.1 - p.1);
    let edge2 = len2(p0, p1).max(len2(p1, p2)).max(len2(p2, p3));
    let d3 = (p3.0 - 3.0 * p2.0 + 3.0 * p1.0 - p0.0, p3.1 - 3.0 * p2.1 + 3.0 * p1.1 - p0.1);
    (0.5 * f64::from(max_area) / (9.0 / 64.0 * (edge2 * (d3.0 * d3.0 + d3.1 * d3.1)).sqrt())) as f32
}

impl Flattener<'_> {
    // A contour is kept without a closing point repeating its first, and only with three points left.
    fn end_contour(&mut self) {
        self.read += self.points.len();
        if self.points.len() > 1 && self.points.first() == self.points.last() {
            self.points.pop();
        }
        if self.points.len() >= 3 {
            self.room = self.points.len();
            let kept = core::mem::take(&mut self.points);
            match self.contours.get_mut(self.kept) {
                Some(slot) => *slot = kept,
                None => self.contours.push(kept),
            }
            self.kept += 1;
        } else {
            self.points.clear();
        }
    }

    // A contour drawn without `move_to` starts where the pen is: after `close`, the last one's start.
    fn begin(&mut self) {
        if self.points.is_empty() {
            self.open();
            self.points.push(self.previous.into());
            self.start = self.previous;
        }
    }

    // A list for a contour about to begin, where the last one was kept: the caller's next, emptied, or a new one.
    // Once a contour, so out of line, keeping `begin`, which every segment calls, small.
    #[inline(never)]
    fn open(&mut self) {
        if self.points.capacity() == 0 {
            self.points = match self.contours.get_mut(self.kept) {
                Some(spare) => core::mem::take(spare),
                None => Vec::with_capacity(self.room),
            };
            self.points.clear();
        }
    }

    fn subdivide(&mut self, start: Pt, end: Pt, quarter_limit: f32, point_at: impl Fn(f32) -> Pt) {
        // Later half first, because the stack pops from its end; that keeps the points in curve order.
        let mut stack = [(start, 0.0f32, end, 1.0f32); 16];
        let mut len = 1usize;
        while len > 0 && self.read + self.points.len() <= MAX_READ {
            len -= 1;
            let (a, at, c, ct) = stack[len];
            let bt = (at + ct) * 0.5;
            let b = point_at(bt);
            // The quarters catch what the midpoint misses: an S puts it on the chord, and a loop's ends meet.
            let span2 = (ct - at) * (ct - at);
            let bends = too_curved(a, b, c, self.max_area)
                || (span2 * span2 > quarter_limit || quarter_limit.is_nan())
                    && (too_curved(a, point_at((at + bt) * 0.5), b, self.max_area)
                        || too_curved(b, point_at((bt + ct) * 0.5), c, self.max_area));
            if bends && ct - at > MIN_SEGMENT_SPAN && len <= 11 {
                stack[len] = (b, bt, c, ct);
                stack[len + 1] = (a, at, b, bt);
                len += 2;
            } else {
                self.points.push(c.into());
            }
        }
    }
}

impl OutlinePen for Flattener<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.in_range &= fits(&x) && fits(&y);
        self.end_contour();
        self.open();
        self.points.push(Point { x, y });
        self.start = (x, y);
        self.previous = (x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.in_range &= fits(&x) && fits(&y);
        self.begin();
        self.points.push(Point { x, y });
        self.previous = (x, y);
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.in_range &= [cx, cy, x, y].iter().all(fits);
        self.begin();
        let (a, end) = (self.previous, (x, y));
        // No limit: a quadratic's quarter triangles are an eighth of its own, so they never fail.
        self.subdivide(a, end, f32::INFINITY, |t| {
            let tm = 1.0 - t;
            let (p, q, r) = (tm * tm, 2.0 * tm * t, t * t);
            (p * a.0 + q * cx + r * x, p * a.1 + q * cy + r * y)
        });
        self.previous = end;
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.in_range &= [c1x, c1y, c2x, c2y, x, y].iter().all(fits);
        self.begin();
        let (a, end) = (self.previous, (x, y));
        let limit = quarter_limit(a, (c1x, c1y), (c2x, c2y), end, self.max_area);
        self.subdivide(a, end, limit, |t| {
            let tm = 1.0 - t;
            let (p, q, r, s) = (tm * tm * tm, 3.0 * (tm * tm) * t, 3.0 * tm * (t * t), t * t * t);
            (p * a.0 + q * c1x + r * c2x + s * x, p * a.1 + q * c1y + r * c2y + s * y)
        });
        self.previous = end;
    }

    fn close(&mut self) {
        if self.start != self.previous {
            self.points.push(self.start.into());
        }
        self.previous = self.start;
        self.end_contour();
    }
}
