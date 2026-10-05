use alloc::vec::Vec;
use super::OutlinePen;
#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
    Move,
    Line,
    Quad,
    Cubic,
    Close,
}

impl Verb {
    fn points(self) -> usize {
        match self {
            Verb::Move | Verb::Line => 1,
            Verb::Quad => 2,
            Verb::Cubic => 3,
            Verb::Close => 0,
        }
    }
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Path {
    verbs: Vec<Verb>,
    points: Vec<(f32, f32)>,
}

impl Path {
    pub fn is_empty(&self) -> bool {
        self.verbs.is_empty()
    }

    pub fn parts(&self) -> (&[Verb], &[(f32, f32)]) {
        (&self.verbs, &self.points)
    }

    pub fn cost(&self) -> usize {
        self.verbs.len() + self.points.len() * core::mem::size_of::<(f32, f32)>()
    }

    pub(crate) fn shrink_to_fit(&mut self) {
        self.verbs.shrink_to_fit();
        self.points.shrink_to_fit();
    }

    pub fn replay(&self, transform: Option<&[f64; 6]>, pen: &mut dyn OutlinePen) {
        let mut at = 0usize;
        let mut open = false;
        let map = |(x, y): (f32, f32)| -> (f32, f32) {
            match transform {
                None => (x, y),
                Some([a, b, c, d, dx, dy]) => {
                    let (x, y) = (f64::from(x), f64::from(y));
                    ((x * a + y * c + dx) as f32, (x * b + y * d + dy) as f32)
                }
            }
        };
        for &verb in &self.verbs {
            let p = &self.points[at..at + verb.points()];
            at += verb.points();
            match verb {
                // A path replays as it fills: every contour closed, each before the next begins.
                Verb::Move => {
                    if open { pen.close(); }
                    let (x, y) = map(p[0]);
                    pen.move_to(x, y);
                }
                Verb::Line => { let (x, y) = map(p[0]); pen.line_to(x, y); }
                Verb::Quad => {
                    let (cx, cy) = map(p[0]);
                    let (x, y) = map(p[1]);
                    pen.quad_to(cx, cy, x, y);
                }
                Verb::Cubic => {
                    let (ax, ay) = map(p[0]);
                    let (bx, by) = map(p[1]);
                    let (x, y) = map(p[2]);
                    pen.curve_to(ax, ay, bx, by, x, y);
                }
                Verb::Close => { pen.close(); open = false; }
            }
            if verb == Verb::Move {
                open = true;
            }
        }
        if open {
            pen.close();
        }
    }

    pub fn bounds(&self) -> Option<(f64, f64, f64, f64)> {
        let mut b = Bounds::default();
        let mut at = 0usize;
        for &verb in &self.verbs {
            let p = &self.points[at..at + verb.points()];
            at += verb.points();
            match verb {
                Verb::Move => b.move_to(p[0].0, p[0].1),
                Verb::Line => b.line_to(p[0].0, p[0].1),
                Verb::Quad => b.quad_to(p[0].0, p[0].1, p[1].0, p[1].1),
                Verb::Cubic => b.curve_to(p[0].0, p[0].1, p[1].0, p[1].1, p[2].0, p[2].1),
                Verb::Close => b.close(),
            }
        }
        b.finish()
    }
}

// The tight box of an outline as it is drawn: each segment's end points and its curve's extremes
// between them. A pen, so a box needs no path held.
#[derive(Default)]
pub(crate) struct Bounds {
    e: Extent,
    cur: (f64, f64),
    start: (f64, f64),
}

impl Bounds {
    pub(crate) fn finish(self) -> Option<(f64, f64, f64, f64)> {
        self.e.finish()
    }
}

impl OutlinePen for Bounds {
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
        self.points.push((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.verbs.push(Verb::Line);
        self.points.push((x, y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.verbs.push(Verb::Quad);
        self.points.push((cx, cy));
        self.points.push((x, y));
    }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.verbs.push(Verb::Cubic);
        self.points.push((a, b));
        self.points.push((c, d));
        self.points.push((x, y));
    }
    fn close(&mut self) {
        self.verbs.push(Verb::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // x runs 0, 50, 150, 100: one control between the ends and one past them, so the curve bulges
    // to about 113.7 before it comes back to 100. y stays between its ends and adds nothing.
    #[test]
    fn a_curve_with_one_control_past_its_ends_is_bounded_by_its_bulge() {
        let mut path = Path::default();
        path.move_to(0.0, 0.0);
        path.curve_to(50.0, 10.0, 150.0, 20.0, 100.0, 30.0);
        let (x0, y0, x1, y1) = path.bounds().expect("a box");
        assert_eq!((x0, y0, y1), (0.0, 0.0, 30.0));
        assert!((113.0..114.0).contains(&x1), "the bulge reached {x1}");
    }
}
