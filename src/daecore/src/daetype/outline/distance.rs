//! An outline as chords to measure distances to: each segment flattened in f64 until no chord strays from it by more
//! than a tolerance, so a distance to the chords is the curve's within it, with no cube roots. Each segment keeps
//! the tangents at its ends and a color, the channels of a multi-channel field that measure to it, after Chlumský's
//! thesis.

use alloc::vec::Vec;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;
use super::{Path, Point, Verb};

pub(crate) type V = [f64; 2];

/// Red, green and blue together: a segment every channel measures to.
pub(crate) const WHITE: u8 = 0b111;
// Two channels each, so any two share one: where they meet at a corner, the median of three keeps it sharp.
const COLORS: [u8; 3] = [0b110, 0b101, 0b011];

// A corner turns by more than this sine's angle, the thesis' 3 radians between the two directions.
const CORNER: f64 = 0.141_120_008_059_867_2;

/// One segment of the outline: its chords through `pts[first..=last]`, the unit tangents leaving its start and
/// reaching its end, the channels that measure to it, whether what it bounds lies on its left, and which of its
/// ends are its curve's own (`START`, `END`) rather than where another contour cut it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Segment {
    pub(crate) first: usize,
    pub(crate) last: usize,
    pub(crate) start: V,
    pub(crate) end: V,
    pub(crate) color: u8,
    pub(crate) left: bool,
    pub(crate) ends: u8,
}

pub(crate) const START: u8 = 1;
pub(crate) const END: u8 = 2;

// How far to either side of a piece its sides' windings are read, in pixels: off the line it lies on, nearer than
// any other chord but one it touches.
const BESIDE: f64 = 1e-7;

/// An outline's segments as chords, kept between uses.
#[derive(Default)]
pub(crate) struct Chords {
    pub(crate) pts: Vec<V>,
    pub(crate) segments: Vec<Segment>,
    // A contour's curves, their points and how many, its tangents and corners, while it's colored.
    curves: Vec<([V; 4], usize)>,
    tangents: Vec<(V, V)>,
    corners: Vec<bool>,
    colors: Vec<u8>,
    // Where `bound` cuts the chords, the points beside each piece it reads the winding at, and those windings.
    cuts: Vec<(usize, f64)>,
    sides: Vec<V>,
    wound: Vec<i32>,
    /// What `bound` keeps, as `pts` and `segments` hold the whole outline.
    pub(crate) kept: Vec<V>,
    pub(crate) runs: Vec<Segment>,
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1]]
}

fn length(v: V) -> f64 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

pub(crate) fn dot(a: V, b: V) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

pub(crate) fn cross(a: V, b: V) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}

// A curve's unit tangents at its two ends, each from the first control point apart from that end; none for a curve
// that's a point.
fn tangents(c: &[V; 4], n: usize) -> Option<(V, V)> {
    let (a, b) = (c[0], c[n - 1]);
    let start = c[1..n].iter().map(|&p| sub(p, a)).find(|&d| d != [0.0; 2])?;
    let end = c[..n - 1].iter().rev().map(|&p| sub(b, p)).find(|&d| d != [0.0; 2])?;
    Some((start.map(|v| v / length(start)), end.map(|v| v / length(end))))
}

// A curve cut in two halves.
fn halves(c: &[V; 4], n: usize) -> [([V; 4], usize); 2] {
    let mid = |a: V, b: V| [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    match n {
        2 => {
            let m = mid(c[0], c[1]);
            [([c[0], m, m, m], 2), ([m, c[1], c[1], c[1]], 2)]
        }
        3 => {
            let (p, q) = (mid(c[0], c[1]), mid(c[1], c[2]));
            let m = mid(p, q);
            [([c[0], p, m, m], 3), ([m, q, c[2], c[2]], 3)]
        }
        _ => {
            let (p, q, r) = (mid(c[0], c[1]), mid(c[1], c[2]), mid(c[2], c[3]));
            let (s, t) = (mid(p, q), mid(q, r));
            let m = mid(s, t);
            [([c[0], p, s, m], 4), ([m, t, r, c[3]], 4)]
        }
    }
}

impl Chords {
    /// `path` placed by `m`, its segments colored and flattened within `tolerance`; false where the chords would pass
    /// `budget` points.
    pub(crate) fn build(&mut self, path: &Path, m: &[f64; 6], tolerance: f64, budget: usize) -> bool {
        self.pts.clear();
        self.segments.clear();
        self.curves.clear();
        let place = |p: &Point| {
            let (x, y) = (f64::from(p.x), f64::from(p.y));
            [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]]
        };
        let (mut start, mut at, mut fits) = ([0.0; 2], [0.0; 2], true);
        for (verb, points) in path.segments() {
            match verb {
                Verb::Move => {
                    fits &= self.contour(start, at, tolerance, budget);
                    start = place(&points[0]);
                    at = start;
                }
                Verb::Line => {
                    let p = place(&points[0]);
                    self.curves.push(([at, p, p, p], 2));
                    at = p;
                }
                Verb::Quad => {
                    let (c, p) = (place(&points[0]), place(&points[1]));
                    self.curves.push(([at, c, p, p], 3));
                    at = p;
                }
                Verb::Cubic => {
                    let (c, d, p) = (place(&points[0]), place(&points[1]), place(&points[2]));
                    self.curves.push(([at, c, d, p], 4));
                    at = p;
                }
                Verb::Close => {
                    fits &= self.contour(start, at, tolerance, budget);
                    at = start;
                }
            }
        }
        fits & self.contour(start, at, tolerance, budget)
    }

    // The contour read so far closed back to `start`, colored and flattened; its curves cleared.
    fn contour(&mut self, start: V, at: V, tolerance: f64, budget: usize) -> bool {
        if self.curves.is_empty() {
            return true;
        }
        if at != start {
            self.curves.push(([at, start, start, start], 2));
        }
        let mut curves = core::mem::take(&mut self.curves);
        curves.retain(|(c, n)| tangents(c, *n).is_some());
        self.color(&mut curves);
        let mut fits = true;
        for (k, (c, n)) in curves.iter().enumerate() {
            let color = self.colors[k];
            fits &= self.flatten(c, *n, color, tolerance, budget);
        }
        curves.clear();
        self.curves = curves;
        fits
    }

    // Colors a contour's curves into `colors`: one with no corner white throughout; one corner, a teardrop, its
    // curves in thirds, a color either side of the corner and white between; else a color per run between corners,
    // each unlike the last. The curves are turned to start at the first corner, and a teardrop's cut to three.
    fn color(&mut self, curves: &mut Vec<([V; 4], usize)>) {
        self.tangents.clear();
        self.tangents.extend(curves.iter().filter_map(|(c, n)| tangents(c, *n)));
        let m = curves.len();
        self.corners.clear();
        self.corners.extend((0..m).map(|i| {
            let (a, b) = (self.tangents[(i + m - 1) % m].1, self.tangents[i].0);
            dot(a, b) <= 0.0 || cross(a, b).abs() > CORNER
        }));
        self.colors.clear();
        let Some(first) = self.corners.iter().position(|&c| c) else {
            self.colors.resize(m, WHITE);
            return;
        };
        curves.rotate_left(first);
        self.corners.rotate_left(first);
        let count = self.corners.iter().filter(|&&c| c).count();
        if count == 1 {
            while curves.len() < 3 {
                let longest = (0..curves.len()).max_by(|&i, &j| reach(&curves[i]).total_cmp(&reach(&curves[j])));
                let longest = longest.unwrap_or(0);
                let [a, b] = halves(&curves[longest].0, curves[longest].1);
                curves[longest] = a;
                curves.insert(longest + 1, b);
            }
            let m = curves.len();
            self.colors.extend((0..m).map(|k| [COLORS[0], WHITE, COLORS[2]][3 * k / m]));
            return;
        }
        // The last run meets the first at the first corner, so it takes neither's color.
        let mut run = 0;
        for (i, &corner) in self.corners.iter().enumerate() {
            run += usize::from(corner && i > 0);
            self.colors.push(if run == count - 1 && run % 3 == 0 { COLORS[1] } else { COLORS[run % 3] });
        }
    }

    // A curve's chords, as many as keep each within `tolerance`, appended as a segment of `color`.
    fn flatten(&mut self, c: &[V; 4], n: usize, color: u8, tolerance: f64, budget: usize) -> bool {
        let Some((start, end)) = tangents(c, n) else { return true };
        // A chord over a stretch 1/k of the curve strays at most |B''| / 8k², B'' bounded by its second differences.
        let bend = match n {
            2 => 0.0,
            3 => 2.0 * length([c[0][0] - 2.0 * c[1][0] + c[2][0], c[0][1] - 2.0 * c[1][1] + c[2][1]]),
            _ => {
                let d = |a: V, b: V, e: V| length([a[0] - 2.0 * b[0] + e[0], a[1] - 2.0 * b[1] + e[1]]);
                6.0 * d(c[0], c[1], c[2]).max(d(c[1], c[2], c[3]))
            }
        };
        let k = (bend / (8.0 * tolerance)).sqrt().ceil();
        let k = if k >= 1.0 { k as usize } else { 1 };
        if self.pts.len().saturating_add(k).saturating_add(1) > budget {
            return false;
        }
        let first = self.pts.len();
        let (a, b) = (c[0], c[n - 1]);
        self.pts.push(a);
        for i in 1..k {
            let t = i as f64 / k as f64;
            let u = 1.0 - t;
            self.pts.push(match n {
                2 => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                3 => [0, 1].map(|j| u * u * a[j] + 2.0 * u * t * c[1][j] + t * t * b[j]),
                _ => [0, 1].map(|j| u * u * u * a[j] + 3.0 * u * t * (u * c[1][j] + t * c[2][j]) + t * t * t * b[j]),
            });
        }
        self.pts.push(b);
        let last = self.pts.len() - 1;
        self.segments.push(Segment { first, last, start, end, color, left: true, ends: START | END });
        true
    }
}

impl Chords {
    // Each chord, `pts[j]` to `pts[j + 1]`.
    fn chords(&self) -> impl Iterator<Item = usize> + '_ {
        self.segments.iter().flat_map(|s| s.first..s.last)
    }

    /// For an outline whose contours run either way, as one with overlaps left in may: each segment's inside on its
    /// left where the outline's area is counterclockwise.
    pub(crate) fn orient(&mut self) {
        let pts = &self.pts;
        let area: f64 = self.chords().map(|j| cross(pts[j], pts[j + 1])).sum();
        self.segments.iter_mut().for_each(|s| s.left = area >= 0.0);
    }

    /// What bounds the region the chords fill under nonzero, into `kept` and `runs`: each chord cut wherever another
    /// crosses or touches it, of the `pairs` of chords a sweep found meeting, each piece kept where one side is filled
    /// and the other isn't, as `windings` reads the points beside each, and each run of pieces within a segment filled
    /// the same side kept as a segment of its own.
    pub(crate) fn bound(&mut self, pairs: &[(u32, u32)], windings: &mut dyn FnMut(&[V], &mut Vec<i32>)) {
        let pts = &self.pts;
        self.cuts.clear();
        for &(j, k) in pairs {
            let (j, k) = (j as usize, k as usize);
            let ((a, b), (c, d)) = ((pts[j], pts[j + 1]), (pts[k], pts[k + 1]));
            let (ab, cd) = (sub(b, a), sub(d, c));
            let (o1, o2) = (cross(ab, sub(c, a)), cross(ab, sub(d, a)));
            let (o3, o4) = (cross(cd, sub(a, c)), cross(cd, sub(b, c)));
            if o1 * o2 < 0.0 && o3 * o4 < 0.0 {
                self.cuts.push((j, o3 / (o3 - o4)));
                self.cuts.push((k, o1 / (o1 - o2)));
                continue;
            }
            // Touching: an end of one lying on the other, where a piece may change sides.
            let mut on = |j: usize, a: V, ab: V, p: V, o: f64| {
                let t = dot(sub(p, a), ab) / dot(ab, ab);
                if o == 0.0 && t > 0.0 && t < 1.0 {
                    self.cuts.push((j, t));
                }
            };
            on(j, a, ab, c, o1);
            on(j, a, ab, d, o2);
            on(k, c, cd, a, o3);
            on(k, c, cd, b, o4);
        }
        self.cuts.sort_unstable_by(|p, q| p.0.cmp(&q.0).then(p.1.total_cmp(&q.1)));
        self.cuts.dedup();
        let mut sides = core::mem::take(&mut self.sides);
        sides.clear();
        self.pieces(|_, _, _, _, m, n| sides.extend([[m[0] + n[0], m[1] + n[1]], [m[0] - n[0], m[1] - n[1]]]));
        let mut wound = core::mem::take(&mut self.wound);
        windings(&sides, &mut wound);
        self.sides = sides;
        self.kept.clear();
        self.runs.clear();
        let mut open: Option<Segment> = None;
        let mut at = 0;
        let (mut kept, mut runs) = (core::mem::take(&mut self.kept), core::mem::take(&mut self.runs));
        self.pieces(|s, j, (from, to), (p, q), _, _| {
            let (l, r) = (wound[2 * at] != 0, wound[2 * at + 1] != 0);
            at += 1;
            let along = sub(q, p);
            let way = along.map(|v| v / length(along));
            if j == usize::MAX {
                runs.extend(open.take());
                return;
            }
            match open.as_mut() {
                _ if l == r => runs.extend(open.take()),
                Some(run) if run.left == l && kept[run.last] == p => {
                    kept.push(q);
                    run.last += 1;
                }
                _ => {
                    runs.extend(open.take());
                    let (first, own) = (kept.len(), j == s.first && from == 0.0);
                    kept.extend([p, q]);
                    let start = if own { s.start } else { way };
                    let ends = if own { START } else { 0 };
                    let last = first + 1;
                    open = Some(Segment { first, last, start, end: way, color: s.color, left: l, ends });
                }
            }
            if let Some(run) = open.as_mut().filter(|_| l != r) {
                let own = j + 1 == s.last && to == 1.0;
                run.end = if own { s.end } else { way };
                run.ends = run.ends & START | if own { END } else { 0 };
            }
        });
        runs.extend(open);
        (self.kept, self.runs, self.wound) = (kept, runs, wound);
    }

    // Each piece the cuts leave, segment by segment: its segment, chord, span along the chord, ends, middle, and the
    // step to either side of it the windings are read at; then, at each segment's end, its segment with no chord.
    fn pieces(&self, mut each: impl FnMut(&Segment, usize, (f64, f64), (V, V), V, V)) {
        let mut at = 0;
        for s in &self.segments {
            for j in s.first..s.last {
                let (a, b) = (self.pts[j], self.pts[j + 1]);
                // The chord's own ends exactly, so a run joins the next chord's piece where it should.
                let point = |t: f64| match t {
                    0.0 => a,
                    1.0 => b,
                    _ => [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                };
                let mut from = 0.0;
                while from < 1.0 {
                    while self.cuts.get(at).is_some_and(|c| c.0 < j || c.0 == j && c.1 <= from) {
                        at += 1;
                    }
                    let to = self.cuts.get(at).filter(|c| c.0 == j).map_or(1.0, |c| c.1);
                    let (p, q) = (point(from), point(to));
                    let along = sub(q, p);
                    let len = length(along);
                    if len > 0.0 {
                        let (m, n) = (point((from + to) * 0.5), [-along[1] / len * BESIDE, along[0] / len * BESIDE]);
                        each(s, j, (from, to), (p, q), m, n);
                    }
                    from = to;
                }
            }
            each(s, usize::MAX, (0.0, 0.0), ([0.0; 2], [1.0, 0.0]), [0.0; 2], [0.0; 2]);
        }
    }
}

// How far a curve's control polygon runs, to pick the one to cut.
fn reach((c, n): &([V; 4], usize)) -> f64 {
    c[..*n].windows(2).map(|w| length(sub(w[1], w[0]))).sum()
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/outline/distance.rs"]
mod tests;
