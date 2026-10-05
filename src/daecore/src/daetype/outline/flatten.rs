use alloc::vec::Vec;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daemachine::float::FloatExt;

use super::pen::OutlinePen;
use super::path::Path;

const MIN_SEGMENT_SPAN: f32 = 1.0 / 4096.0;

// A curve can become 4,097 points, so one glyph of 65,534 curves could otherwise come to 268 million.
pub const MAX_FLATTEN_POINTS: usize = 1_048_576;

// Closing points and contours too short to keep are read but not returned, so reading stops at twice
// the limit: a path that reads more is refused, however little it would keep.
const MAX_READ: usize = 2 * MAX_FLATTEN_POINTS;

// The `max_area` 1.1.7's CPU rasterizer flattened with, for a path in font units drawn at `px`. A px so
// small the area overflows, or one below zero, asks for the coarsest, which flatten would read as finest.
pub fn max_area_for(px: f32, units_per_em: f32) -> f32 {
    let area = 3.0 * 2.0 * (units_per_em / px);
    if area == f32::INFINITY || area.is_sign_negative() && !area.is_nan() { f32::MAX } else { area }
}

// Closed polygons, none under three points or repeating its first: `None` past `MAX_FLATTEN_POINTS`
// points kept or twice that read, or a coordinate past `LIMIT`. A bad `max_area` halves all the way.
pub fn flatten(path: &Path, max_area: f32) -> Option<Vec<Vec<(f32, f32)>>> {
    let max_area = if max_area.is_finite() && max_area > 0.0 { max_area } else { 0.0 };
    let mut pen = Flattener {
        max_area, start: (0.0, 0.0), previous: (0.0, 0.0), points: Vec::new(), read: 0, contours: Vec::new(), in_range: true,
    };
    path.replay(None, &mut pen);
    pen.end_contour();
    if pen.read > MAX_READ || !pen.in_range {
        return None;
    }
    (pen.contours.iter().map(Vec::len).sum::<usize>() <= MAX_FLATTEN_POINTS).then_some(pen.contours)
}

type Pt = (f32, f32);

// A point on a curve is a weighted mean of its controls, and the weights' rounding can carry it past the
// largest of them, so near f32::MAX it could come out infinite. A NaN fails this too.
const LIMIT: f32 = f32::MAX / 16.0;

fn fits(v: &f32) -> bool {
    v.abs() <= LIMIT
}

// `points` is the contour being read; each is kept as it ends, `read` counting every point read.
struct Flattener {
    max_area: f32,
    start: Pt,
    previous: Pt,
    points: Vec<Pt>,
    read: usize,
    contours: Vec<Vec<Pt>>,
    in_range: bool,
}

// A NaN counts as too curved: a curve past 1e19 across overflows its cross products to inf - inf, and
// splits until they fit rather than flattening to its chord.
fn too_curved(a: Pt, b: Pt, c: Pt, max_area: f32) -> bool {
    let area = ((b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)).abs();
    area > max_area || area.is_nan()
}

// The span⁴ under which a piece of a cubic cannot fail its quarter checks once its midpoint passes: its
// quarters sit within 3/64 span³ |Δ³| of the parabola through its ends and midpoint, whose quarter
// triangles are an eighth of its own, and its chord is under 3 span times its longest edge. Half of
// max_area is margin, and f64 keeps a tiny curve's product from underflowing to an infinite limit.
fn quarter_limit(p0: Pt, p1: Pt, p2: Pt, p3: Pt, max_area: f32) -> f32 {
    let w = |p: Pt| (f64::from(p.0), f64::from(p.1));
    let (p0, p1, p2, p3) = (w(p0), w(p1), w(p2), w(p3));
    let len2 = |p: (f64, f64), q: (f64, f64)| (q.0 - p.0) * (q.0 - p.0) + (q.1 - p.1) * (q.1 - p.1);
    let edge2 = len2(p0, p1).max(len2(p1, p2)).max(len2(p2, p3));
    let d3 = (p3.0 - 3.0 * p2.0 + 3.0 * p1.0 - p0.0, p3.1 - 3.0 * p2.1 + 3.0 * p1.1 - p0.1);
    (0.5 * f64::from(max_area) / (9.0 / 64.0 * (edge2 * (d3.0 * d3.0 + d3.1 * d3.1)).sqrt())) as f32
}

impl Flattener {
    // A contour is kept without a closing point repeating its first, and only with three points left.
    fn end_contour(&mut self) {
        self.read += self.points.len();
        if self.points.len() > 1 && self.points.first() == self.points.last() {
            self.points.pop();
        }
        if self.points.len() >= 3 {
            let next = Vec::with_capacity(self.points.len());
            self.contours.push(core::mem::replace(&mut self.points, next));
        } else {
            self.points.clear();
        }
    }

    // A contour drawn without `move_to` starts where the pen is: after `close`, the last one's start.
    fn begin(&mut self) {
        if self.points.is_empty() {
            self.points.push(self.previous);
            self.start = self.previous;
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
                self.points.push(c);
            }
        }
    }
}

impl OutlinePen for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.in_range &= fits(&x) && fits(&y);
        self.end_contour();
        self.points.push((x, y));
        self.start = (x, y);
        self.previous = (x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.in_range &= fits(&x) && fits(&y);
        self.begin();
        self.points.push((x, y));
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
            self.points.push(self.start);
        }
        self.previous = self.start;
        self.end_contour();
    }
}
