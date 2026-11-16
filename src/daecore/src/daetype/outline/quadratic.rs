#[cfg(doc)]
use crate::ErrorKind::{InvalidArgument, NoOutline, Overflow};
use alloc::vec::Vec;
use crate::daecore::daetype::outline::{OutlinePen, Point};
use crate::daecore::daekit::base::error::Result;
use crate::daecore::{Error, ErrorKind};

/// A quadratic curve: its start, its control and its end. Laid out as C reads it.
#[expect(clippy::exhaustive_structs, reason = "a curve's three points are all it is")]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Quad {
    /// Where it starts.
    pub start: Point,
    /// The control point it bends toward.
    pub control: Point,
    /// Where it ends.
    pub end: Point,
}

// How far a quad may sit from the cubic it stands for, in ems, as `Font::quads` splits them.
pub(crate) const CUBIC_TOLERANCE: f32 = 1.0 / 4096.0;

// Not a limit: each halving shrinks the deviation eightfold, and after this many it is under half the f32 spacing at
// the cubic's largest coordinate, where the halving's own rounding lands. So a cubic gives 256 quads at most.
const MAX_CUBIC_DEPTH: u8 = (f32::MANTISSA_DIGITS / 3) as u8;

// Tuned on Inter, whose mean glyph is 30.3 curves. A serif face is not Inter – EB Garamond averages
// 70.7, and 97% of its glyphs realloc past this anyway. Do not re-tune it on one face.
const CURVES_RESERVE: usize = 32;

const NO_OUTLINE: Error = Error::new(ErrorKind::NoOutline, "glyph has no outline");
const NON_FINITE: Error =
    Error::new(ErrorKind::Overflow, "outline has a coordinate that is not finite, or too large to convert");

/// A pen that turns what it's drawn into quadratic curves, as a curve-evaluating GPU shader takes them: a line
/// becomes a curve with its control at the midpoint, and a cubic is split until each piece lies within a
/// tolerance of it, measured at the piece's quarter points, or into 256, past which f32 can't place the pieces any
/// closer. Points stay in the units they're drawn in, y up.
#[derive(Debug)]
pub struct QuadraticPen {
    curves: Vec<Quad>,
    start: Point,
    cur: Point,
    // The tolerance, squared, as the deviation is measured.
    limit: f32,
    rejected: Option<Error>,
}

impl QuadraticPen {
    /// A pen splitting cubics to within `tolerance`, in the units it's drawn in; [`Font::quads`] asks for 1/4096
    /// of an em. One not finite and above zero fails [`QuadraticPen::finish`].
    ///
    /// [`Font::quads`]: crate::Font::quads
    #[must_use]
    pub fn new(tolerance: f32) -> QuadraticPen {
        QuadraticPen::with_curves(tolerance, Vec::with_capacity(CURVES_RESERVE))
    }

    /// As [`QuadraticPen::new`], drawing into `curves`, emptied, so a list kept between outlines allocates only to
    /// grow: [`QuadraticPen::finish`] hands it back.
    #[must_use]
    pub fn reusing(tolerance: f32, mut curves: Vec<Quad>) -> QuadraticPen {
        curves.clear();
        curves.reserve(CURVES_RESERVE);
        QuadraticPen::with_curves(tolerance, curves)
    }

    fn with_curves(tolerance: f32, curves: Vec<Quad>) -> QuadraticPen {
        let usable = tolerance.is_finite() && tolerance > 0.0;
        let rejected = (!usable).then_some(Error::invalid("tolerance must be finite and above zero"));
        QuadraticPen {
            curves,
            start: Point::default(),
            cur: Point::default(),
            limit: tolerance * tolerance,
            rejected,
        }
    }

    // Every midpoint and control is a combination of inputs no more than eight times the largest, so an
    // input past a sixteenth of f32's range is refused rather than allowed to overflow one. NaN fails too.
    fn point(&mut self, x: f32, y: f32) -> Point {
        const LIMIT: f32 = f32::MAX / 16.0;
        if !(x.abs() <= LIMIT && y.abs() <= LIMIT) {
            self.rejected.get_or_insert(NON_FINITE);
        }
        Point { x, y }
    }

    fn line(&mut self, to: Point) {
        self.curves.push(Quad { start: self.cur, control: mid(self.cur, to), end: to });
        self.cur = to;
    }

    /// The curves, in the order they were drawn, an open contour closed.
    ///
    /// # Errors
    ///
    /// [`NoOutline`] where nothing was drawn, [`Overflow`] for a point not finite or past `f32::MAX / 16`, and
    /// [`InvalidArgument`] for the tolerance.
    pub fn finish(mut self) -> Result<Vec<Quad>> {
        // A caller may leave its last contour open, and the winding rule reads straight through that
        // gap, turning a glyph inside out.
        self.close();

        if let Some(why) = self.rejected {
            return Err(why);
        }
        if self.curves.is_empty() {
            return Err(NO_OUTLINE);
        }
        Ok(self.curves)
    }

    // As `finish`, the list coming back on a failure too for a caller keeping its room. Apart from `finish`, which
    // a shared body took 1.4% to 2% longer in three layouts of four.
    pub(crate) fn finish_apart(mut self) -> (Vec<Quad>, Result<()>) {
        self.close();
        let verdict = match self.rejected.take() {
            Some(why) => Err(why),
            None if self.curves.is_empty() => Err(NO_OUTLINE),
            None => Ok(()),
        };
        (self.curves, verdict)
    }
}

// In f64, where no product of two f32 values overflows.
fn signed_area(curves: &[Quad]) -> f64 {
    let mut total = 0.0;
    for &Quad { start, control, end } in curves {
        let [x0, y0, x1, y1, x2, y2] = [start.x, start.y, control.x, control.y, end.x, end.y].map(f64::from);
        total += 2.0 * ((x0 * y1 - x1 * y0) + (x1 * y2 - x2 * y1)) + (x0 * y2 - x2 * y0);
    }
    total
}

/// Rewinds curves so their total signed area isn't positive: clockwise, TrueType's convention, as
/// [`Font::quads`](crate::Font::quads) gives them. A rewound curve keeps its place, so the curves need not chain.
pub fn normalize_winding(curves: &mut [Quad]) {
    if signed_area(curves) > 0.0 {
        for c in curves {
            core::mem::swap(&mut c.start, &mut c.end);
        }
    }
}

fn mid(a: Point, b: Point) -> Point {
    Point { x: (a.x + b.x) * 0.5, y: (a.y + b.y) * 0.5 }
}

fn quad_for(p: &[Point; 4]) -> Quad {
    let control = Point {
        x: (3.0 * p[1].x - p[0].x + 3.0 * p[2].x - p[3].x) * 0.25,
        y: (3.0 * p[1].y - p[0].y + 3.0 * p[2].y - p[3].y) * 0.25,
    };
    Quad { start: p[0], control, end: p[3] }
}

// The squared distance between a cubic and its quad at t = 1/4 and 3/4. The two differ by
// d·t(t - 1/2)(t - 1) for d = p3 - 3p2 + 3p1 - p0, so it is (3/64·|d|)², with no curve evaluated.
fn deviation(p: &[Point; 4]) -> f32 {
    let dx = (p[3].x - 3.0 * p[2].x + 3.0 * p[1].x - p[0].x) * (3.0 / 64.0);
    let dy = (p[3].y - 3.0 * p[2].y + 3.0 * p[1].y - p[0].y) * (3.0 / 64.0);
    dx * dx + dy * dy
}

fn split_cubic(p: &[Point; 4]) -> ([Point; 4], [Point; 4]) {
    let ab = mid(p[0], p[1]);
    let bc = mid(p[1], p[2]);
    let cd = mid(p[2], p[3]);
    let abc = mid(ab, bc);
    let bcd = mid(bc, cd);
    let abcd = mid(abc, bcd);
    ([p[0], ab, abc, abcd], [abcd, bcd, cd, p[3]])
}

impl OutlinePen for QuadraticPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.close();
        let p = self.point(x, y);
        self.cur = p;
        self.start = p;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.point(x, y);
        self.line(p);
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let c = self.point(cx, cy);
        let to = self.point(x, y);
        self.curves.push(Quad { start: self.cur, control: c, end: to });
        self.cur = to;
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let c1 = self.point(c1x, c1y);
        let c2 = self.point(c2x, c2y);
        let to = self.point(x, y);
        if self.rejected.is_some() {
            return;
        }
        let cubic = [self.cur, c1, c2, to];

        let limit = self.limit;
        // A popped node has at most one sibling waiting per level above it, so no split outgrows this.
        let mut stack = [([Point::default(); 4], 0u8); MAX_CUBIC_DEPTH as usize + 2];
        stack[0] = (cubic, 0);
        let mut top = 1usize;

        while top > 0 {
            top -= 1;
            let (seg, depth) = stack[top];
            if depth >= MAX_CUBIC_DEPTH || deviation(&seg) <= limit {
                self.curves.push(quad_for(&seg));
            } else {
                let (lo, hi) = split_cubic(&seg);
                stack[top] = (hi, depth + 1);
                stack[top + 1] = (lo, depth + 1);
                top += 2;
            }
        }
        self.cur = to;
    }

    fn close(&mut self) {
        if self.cur != self.start {
            let s = self.start;
            self.line(s);
        }
        self.cur = self.start;
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/outline/quadratic.rs"]
mod tests;
