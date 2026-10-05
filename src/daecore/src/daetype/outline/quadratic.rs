use alloc::vec::Vec;
use crate::daecore::daetype::outline::OutlinePen;

pub type Quad = [[f32; 2]; 3];

const CUBIC_TOLERANCE: f32 = 1.0 / 4096.0;

const MAX_CUBIC_DEPTH: u8 = 8;

pub const MAX_CURVES_PER_GLYPH: usize = 16_384;

// Tuned on Inter, whose mean glyph is 30.3 curves. A serif face is not Inter – EB Garamond averages
// 70.7, and 97% of its glyphs realloc past this anyway. Do not re-tune it on one face.
const CURVES_RESERVE: usize = 32;

// Em units, y up. A line becomes a quad with its control at the midpoint, and a cubic is split until
// each piece lies within `CUBIC_TOLERANCE` of it, measured at the piece's quarter points.
pub struct QuadraticPen {
    curves: Vec<Quad>,
    start: [f32; 2],
    cur: [f32; 2],
    scale: f32,
    rejected: Option<QuadError>,
}

#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuadError {
    NoOutline,
    TooComplex,
    NonFinite,
    BadUnitsPerEm,
}

impl core::fmt::Display for QuadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            QuadError::NoOutline => write!(f, "glyph has no outline"),
            QuadError::TooComplex => write!(f, "outline exceeds MAX_CURVES_PER_GLYPH"),
            QuadError::NonFinite => write!(f, "outline has a coordinate that is not finite, or too large to convert"),
            QuadError::BadUnitsPerEm => write!(f, "units_per_em must be finite and above zero, with a finite reciprocal"),
        }
    }
}

impl core::error::Error for QuadError {}

impl QuadraticPen {
    pub fn new(units_per_em: f32) -> QuadraticPen {
        // An em so small its reciprocal is infinite is as unusable as zero.
        let scale = 1.0 / units_per_em;
        let usable = units_per_em.is_finite() && units_per_em > 0.0 && scale.is_finite();
        QuadraticPen {
            curves: Vec::with_capacity(CURVES_RESERVE),
            start: [0.0; 2],
            cur: [0.0; 2],
            scale: if usable { scale } else { 0.0 },
            rejected: (!usable).then_some(QuadError::BadUnitsPerEm),
        }
    }

    // Every midpoint and control is a combination of inputs no more than eight times the largest, so an
    // input past a sixteenth of f32's range is refused rather than allowed to overflow one. NaN fails too.
    fn point(&mut self, x: f32, y: f32) -> [f32; 2] {
        const LIMIT: f32 = f32::MAX / 16.0;
        let p = [x * self.scale, y * self.scale];
        if !(p[0].abs() <= LIMIT && p[1].abs() <= LIMIT) {
            self.rejected.get_or_insert(QuadError::NonFinite);
        }
        p
    }

    fn push(&mut self, quad: Quad) {
        if self.curves.len() >= MAX_CURVES_PER_GLYPH {
            self.rejected.get_or_insert(QuadError::TooComplex);
            return;
        }
        self.curves.push(quad);
    }

    fn line(&mut self, to: [f32; 2]) {
        let mid = [(self.cur[0] + to[0]) * 0.5, (self.cur[1] + to[1]) * 0.5];
        self.push([self.cur, mid, to]);
        self.cur = to;
    }

    pub fn finish(mut self) -> Result<Vec<Quad>, QuadError> {
        // A caller may leave its last contour open, and the winding rule reads straight through that
        // gap, turning a glyph inside out.
        self.close();

        if let Some(why) = self.rejected {
            return Err(why);
        }
        if self.curves.is_empty() {
            return Err(QuadError::NoOutline);
        }
        Ok(self.curves)
    }
}

// In f64, where no product of two f32 values overflows.
fn signed_area(curves: &[Quad]) -> f64 {
    let mut total = 0.0;
    for c in curves {
        let [[x0, y0], [x1, y1], [x2, y2]] = c.map(|p| p.map(f64::from));
        total += 2.0 * ((x0 * y1 - x1 * y0) + (x1 * y2 - x2 * y1)) + (x0 * y2 - x2 * y0);
    }
    total
}

// Clockwise, as `glyph_quads` gives them. A rewound curve keeps its place, so the curves need not chain.
pub fn normalize_winding(curves: &mut [Quad]) {
    if signed_area(curves) > 0.0 {
        for c in curves {
            c.swap(0, 2);
        }
    }
}

fn quad_for(p: &[[f32; 2]; 4]) -> Quad {
    let ctrl = [
        (3.0 * p[1][0] - p[0][0] + 3.0 * p[2][0] - p[3][0]) * 0.25,
        (3.0 * p[1][1] - p[0][1] + 3.0 * p[2][1] - p[3][1]) * 0.25,
    ];
    [p[0], ctrl, p[3]]
}

// The squared distance between a cubic and its quad at t = 1/4 and 3/4. The two differ by
// d·t(t - 1/2)(t - 1) for d = p3 - 3p2 + 3p1 - p0, so it is (3/64·|d|)², with no curve evaluated.
fn deviation(p: &[[f32; 2]; 4]) -> f32 {
    let d = |i: usize| (p[3][i] - 3.0 * p[2][i] + 3.0 * p[1][i] - p[0][i]) * (3.0 / 64.0);
    d(0) * d(0) + d(1) * d(1)
}

fn split_cubic(p: &[[f32; 2]; 4]) -> ([[f32; 2]; 4], [[f32; 2]; 4]) {
    let mid = |a: [f32; 2], b: [f32; 2]| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
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
        self.push([self.cur, c, to]);
        self.cur = to;
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        let c1 = self.point(c1x, c1y);
        let c2 = self.point(c2x, c2y);
        let to = self.point(x, y);
        let cubic = [self.cur, c1, c2, to];

        let limit = CUBIC_TOLERANCE * CUBIC_TOLERANCE;
        // A popped node has at most one sibling waiting per level above it, so no split outgrows this.
        let mut stack = [([[0.0f32; 2]; 4], 0u8); MAX_CUBIC_DEPTH as usize + 2];
        stack[0] = (cubic, 0);
        let mut top = 1usize;

        while top > 0 {
            top -= 1;
            let (seg, depth) = stack[top];
            if depth >= MAX_CUBIC_DEPTH || deviation(&seg) <= limit {
                self.push(quad_for(&seg));
            } else {
                let (lo, hi) = split_cubic(&seg);
                stack[top] = (hi, depth + 1);
                stack[top + 1] = (lo, depth + 1);
                top += 2;
            }

            if self.rejected.is_some() {
                return;
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
mod tests {
    use super::*;

    // The closed form is the distance the quad strays from the cubic at t = 1/4 and 3/4, sampled.
    #[test]
    fn the_deviation_is_the_distance_at_the_quarter_points() {
        let at = |p: &[[f32; 2]; 4], q: &Quad, t: f32| {
            let u = 1.0 - t;
            let c = |i: usize| u * u * u * p[0][i] + 3.0 * u * u * t * p[1][i] + 3.0 * u * t * t * p[2][i] + t * t * t * p[3][i];
            let b = |i: usize| u * u * q[0][i] + 2.0 * u * t * q[1][i] + t * t * q[2][i];
            (c(0) - b(0)).powi(2) + (c(1) - b(1)).powi(2)
        };
        for k in 0..200u16 {
            let v = |i: u16| f32::from((k * 37 + i * 101) % 211) - 100.0;
            let p = [[v(0), v(1)], [v(2), v(3)], [v(4), v(5)], [v(6), v(7)]];
            let q = quad_for(&p);
            let sampled = at(&p, &q, 0.25).max(at(&p, &q, 0.75));
            assert!((deviation(&p) - sampled).abs() <= 1e-3 * sampled.max(1.0), "{p:?}: {} against {sampled}", deviation(&p));
        }
    }
}
