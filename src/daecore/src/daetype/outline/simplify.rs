use alloc::collections::{BTreeMap, BTreeSet, BinaryHeap};
use alloc::vec::Vec;
use core::cmp::{Ordering, Reverse};

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;
use super::path::{Path, Point, Verb};
use super::OutlinePen;

pub type Pt = (f32, f32);

// A stroke's pieces overlap, which non-zero hides and even-odd turns into holes; this resolves them so both rules
// agree. Contours whose overlaps are too close to call, or that meet more often than their curves allow, come back
// as they are.
pub fn union(contours: &[Vec<Pt>]) -> Vec<Vec<Pt>> {
    match resolve(read(contours)) {
        Ok(Resolved { removed: Some(removed), .. }) => contours_of(&removed),
        _ => contours.to_vec(),
    }
}

/// One counterclockwise boundary, whatever the input's winding, in place of contours that overlap, for a
/// rasterizer that sums coverage. None means keep the contours: they don't overlap, where they meet they're too close
/// to call, touching without crossing or running together along a curve, or they meet more often than their number
/// of curves allows, as no font does.
pub fn resolve_overlaps(contours: &[Vec<Point>]) -> Option<Vec<Vec<Point>>> {
    let removed = resolve(read(contours)).ok().filter(|r| r.overlapped)?.removed?;
    Some(contours_of(&removed).into_iter().map(|c| c.into_iter().map(Point::from).collect()).collect())
}

// Contours of worked-out points, each on a grid four f32 steps wide at their farthest reach, so points apart by
// rounding alone are one and edges along one line by rounding lie along it.
fn read<P: Copy + Into<Pt>>(contours: &[Vec<P>]) -> Reader {
    let reach = contours.iter().flatten().fold(0.0f32, |m, &p| {
        let (x, y) = p.into();
        m.max(x.abs()).max(y.abs())
    });
    let exponent = (reach.to_bits() >> 23) as i32 - 21;
    let step = if (1..255).contains(&exponent) { f32::from_bits((exponent as u32) << 23) } else { f32::MIN_POSITIVE };
    let snap = |v: f32| (v / step).round() * step;
    let mut reader = Reader::default();
    for c in contours {
        for (k, &p) in c.iter().enumerate() {
            let (x, y) = p.into();
            if k == 0 { reader.move_to(snap(x), snap(y)) } else { reader.line_to(snap(x), snap(y)) }
        }
        reader.close();
    }
    reader
}

// A path of lines as its contours' points, each once.
fn contours_of(path: &Path) -> Vec<Vec<Pt>> {
    let mut out: Vec<Vec<Pt>> = Vec::new();
    for (verb, points) in path.segments() {
        match (verb, out.last_mut()) {
            (Verb::Move, _) => out.push(Vec::new()),
            (Verb::Close, Some(c)) if c.len() > 1 && c.first() == c.last() => drop(c.pop()),
            _ => {}
        }
        if let (Some(c), Some(p)) = (out.last_mut(), points.last()) {
            c.push((p.x, p.y));
        }
    }
    out
}

// Overlaps removed from the curves themselves, so one result in font units serves a glyph at every size, offset and
// affine transform. Only + - * / are used, so every build and CPU splits and keeps alike.

/// What removing an outline's overlaps found.
pub(crate) enum Overlaps {
    /// None: every point is wound at most once, all one way, which summed coverage fills exactly.
    None,
    /// The outline without them, each contour filled on its left.
    Removed(Path),
    /// Too close to call: curves tangent or running together, or meeting more often than their number allows.
    Unsure,
}

type V = [f64; 2];

// Two curves of degree three or less meet nine times at most unless they run together, and each meeting takes about
// 70 halvings to pin down to f64's precision, at most a few pairs of halves alive on each: so a pair needing more than
// this runs together over a stretch, too close to call. Real fonts take 623 at most.
const HALVINGS_PER_PAIR: usize = 1 << 13;
// The cheap check's halvings for two curves no contour runs between: past these it leaves them to the sweep.
const FAST_HALVINGS: usize = 12;

// The work an outline may take in instructions, past which its overlaps are left in: a part for any outline, more for
// each of its first 65,536 curves (all a glyf glyph can number) than past them, so a million curves take seconds.
const WORK: u64 = 1 << 25;
const GLYPH_CURVES: u64 = 1 << 16;
const WORK_PER_GLYPH_CURVE: u64 = 80_000;
const WORK_PER_CURVE: u64 = 12_000;
// Each step's price, about the instructions it takes at a million curves, what follows from it included: a step on
// the line or a cheap check, a tested pair looked up, a halving, a pair met, a cut, an event, a flat stretch wound.
const STEP: u64 = 100;
const LOOKUP: u64 = 600;
const HALVING: u64 = 600;
const PAIR: u64 = 2_000;
const CUT: u64 = 3_000;
const EVENT: u64 = 1_500;
const FLAT_STRETCH: u64 = 4_000;

// Parameters within this of a curve's end are its end, and two within it of each other are one.
const END: f64 = 1e-9;

// A line, quadratic or cubic: its first `n` points.
#[derive(Clone, Copy)]
struct Bez {
    p: [V; 4],
    n: usize,
}

fn lerp(a: V, b: V, t: f64) -> V {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1]]
}

// Twice the area a curve sweeps about the origin, x dy - y dx over it, from its control points.
fn swept(b: &Bez) -> f64 {
    let c = |i: usize, j: usize| b.p[i][0] * b.p[j][1] - b.p[j][0] * b.p[i][1];
    match b.n {
        2 => c(0, 1),
        3 => (2.0 * (c(0, 1) + c(1, 2)) + c(0, 2)) / 3.0,
        _ => (6.0 * (c(0, 1) + c(2, 3)) + 3.0 * (c(0, 2) + c(1, 2) + c(1, 3)) + c(0, 3)) / 10.0,
    }
}

fn apart(a: [f64; 4], b: [f64; 4]) -> bool {
    a[2] < b[0] || b[2] < a[0] || a[3] < b[1] || b[3] < a[1]
}

impl Bez {
    fn end(&self) -> V {
        self.p[self.n - 1]
    }

    fn at(&self, t: f64) -> V {
        let mut q = self.p;
        for k in (1..self.n).rev() {
            for i in 0..k {
                q[i] = lerp(q[i], q[i + 1], t);
            }
        }
        q[0]
    }

    fn tangent(&self, t: f64) -> V {
        let d = (self.n - 1) as f64;
        let mut q = [[0.0; 2]; 4];
        for (q, w) in q.iter_mut().zip(self.p.windows(2)).take(self.n - 1) {
            *q = [(w[1][0] - w[0][0]) * d, (w[1][1] - w[0][1]) * d];
        }
        Bez { p: q, n: self.n - 1 }.at(t)
    }

    // Both halves keep the curve's own ends exactly, so a shared end stays shared however deep the halving goes.
    fn split(&self, t: f64) -> (Bez, Bez) {
        let (mut left, mut right) = (*self, *self);
        let mut q = self.p;
        for k in 0..self.n {
            left.p[k] = q[0];
            right.p[self.n - 1 - k] = q[self.n - 1 - k];
            for i in 0..self.n - 1 - k {
                q[i] = lerp(q[i], q[i + 1], t);
            }
        }
        (left, right)
    }

    fn part(&self, t0: f64, t1: f64) -> Bez {
        let left = if t1 < 1.0 { self.split(t1).0 } else { *self };
        if t0 > 0.0 { left.split(t0 / t1).1 } else { left }
    }

    fn bounds(&self) -> [f64; 4] {
        self.p[1..self.n].iter().fold([self.p[0][0], self.p[0][1], self.p[0][0], self.p[0][1]], |b, q| {
            [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])]
        })
    }

    // Whether its chord stands for it: a line, a speck, or every control point within `tol` of the chord and
    // between its ends.
    fn flat(&self, b: [f64; 4], tol: f64) -> bool {
        if self.n == 2 || (b[2] - b[0]).max(b[3] - b[1]) <= tol {
            return true;
        }
        let (a, e) = (self.p[0], self.end());
        let (dx, dy) = (e[0] - a[0], e[1] - a[1]);
        let len2 = dx * dx + dy * dy;
        len2 > 0.0
            && self.p[1..self.n - 1].iter().all(|q| {
                let (qx, qy) = (q[0] - a[0], q[1] - a[1]);
                let (across, along) = (qx * dy - qy * dx, qx * dx + qy * dy);
                across * across <= tol * tol * len2 && (0.0..=len2).contains(&along)
            })
    }
}

// Whether every point of a curve lies within `tol` of the line from its start to its farthest point.
fn straight(b: &Bez, tol: f64) -> bool {
    let a = b.p[0];
    let off = |q: &V| [q[0] - a[0], q[1] - a[1]];
    let size = |d: &V| d[0].abs().max(d[1].abs());
    let far = b.p[1..b.n].iter().map(off).fold([0.0; 2], |f, d| if size(&d) > size(&f) { d } else { f });
    let across = |d: V| d[0] * far[1] - d[1] * far[0];
    b.p[1..b.n].iter().map(off).all(|d| across(d) * across(d) <= tol * tol * (far[0] * far[0] + far[1] * far[1]))
}

// Where chords a-b and c-d meet, as a parameter along each, ends included; a shared end answers exactly, as two
// chords that nearly continue each other would put it anywhere.
fn chords_meet(a: V, b: V, c: V, d: V) -> Option<(f64, f64)> {
    for (s, p) in [(0.0, a), (1.0, b)] {
        for (u, q) in [(0.0, c), (1.0, d)] {
            if p == q {
                return Some((s, u));
            }
        }
    }
    let (r, e) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * e[1] - r[1] * e[0];
    if den == 0.0 {
        return None;
    }
    let (fx, fy) = (c[0] - a[0], c[1] - a[1]);
    let (s, u) = ((fx * e[1] - fy * e[0]) / den, (fx * r[1] - fy * r[0]) / den);
    let within = |v: f64| (-1e-6..=1.0 + 1e-6).contains(&v);
    (within(s) && within(u)).then(|| (s.clamp(0.0, 1.0), u.clamp(0.0, 1.0)))
}

// Whether stretches sharing one of the curves' own ends meet only there: a line through it parts the rest of their
// control points, tried along the difference of the legs leaving it, each scaled to its larger coordinate.
fn parted(a: &Bez, ends_a: [bool; 2], b: &Bez, ends_b: [bool; 2]) -> bool {
    let ends = |c: &Bez, e: [bool; 2]| [(e[0], c.p[0], c.p[1]), (e[1], c.end(), c.p[c.n - 2])];
    for (own_a, j, leg_a) in ends(a, ends_a) {
        for (own_b, k, leg_b) in ends(b, ends_b) {
            if !(own_a && own_b && j == k) {
                continue;
            }
            let unit = |q: V| {
                let v = [q[0] - j[0], q[1] - j[1]];
                let m = v[0].abs().max(v[1].abs());
                if m > 0.0 { [v[0] / m, v[1] / m] } else { v }
            };
            let (u, v) = (unit(leg_a), unit(leg_b));
            let axis = [v[0] - u[0], v[1] - u[1]];
            let side = |q: &V| (q[0] - j[0]) * axis[0] + (q[1] - j[1]) * axis[1];
            return a.p[..a.n].iter().filter(|q| **q != j).all(|q| side(q) < 0.0)
                && b.p[..b.n].iter().filter(|q| **q != j).all(|q| side(q) > 0.0);
        }
    }
    false
}

// Whether b's control points all lie past the band a's span across a's chord, on one side, so the curves can't meet.
fn beside_band(a: &Bez, b: &Bez) -> bool {
    let (p, e) = (a.p[0], a.end());
    let (dx, dy) = (e[0] - p[0], e[1] - p[1]);
    if dx == 0.0 && dy == 0.0 {
        return false;
    }
    let off = |q: &V| (q[0] - p[0]) * dy - (q[1] - p[1]) * dx;
    let span = |q: &[V], from: (f64, f64)| q.iter().map(off).fold(from, |(l, h), v| (l.min(v), h.max(v)));
    // a's own ends lie on its chord, at 0.
    let ((lo, hi), (b_lo, b_hi)) = (span(&a.p[1..a.n - 1], (0.0, 0.0)), span(&b.p[..b.n], (f64::MAX, f64::MIN)));
    b_hi < lo || b_lo > hi
}

enum Meeting {
    At(f64, f64),
    Apart,
    Tangent,
}

// Newton's method from where the chords met to where the curves do; at a tangent the point is too close to call.
fn refine(a: &Bez, b: &Bez, mut t: f64, mut u: f64, tol: f64) -> Meeting {
    for _ in 0..8 {
        let (p, q) = (a.at(t), b.at(u));
        let f = [p[0] - q[0], p[1] - q[1]];
        let (da, db) = (a.tangent(t), b.tangent(u));
        let det = db[0] * da[1] - da[0] * db[1];
        if det * det <= 1e-12 * (da[0] * da[0] + da[1] * da[1]) * (db[0] * db[0] + db[1] * db[1]) {
            return Meeting::Tangent;
        }
        let next = (
            (t + (f[0] * db[1] - db[0] * f[1]) / det).clamp(0.0, 1.0),
            (u + (f[0] * da[1] - da[0] * f[1]) / det).clamp(0.0, 1.0),
        );
        if next == (t, u) {
            break;
        }
        (t, u) = next;
    }
    let (p, q) = (a.at(t), b.at(u));
    if (p[0] - q[0]).abs().max((p[1] - q[1]).abs()) <= tol { Meeting::At(t, u) } else { Meeting::Apart }
}

#[derive(Clone, Copy)]
struct Curve {
    bez: Bez,
    from: u32,
    to: u32,
}

// The outline read into curves, each end a node; a contour under three points encloses nothing and is dropped, as
// a fill drops it.
#[derive(Default)]
struct Reader {
    curves: Vec<Curve>,
    nodes: Vec<V>,
    first: V,
    first_node: u32,
    last: V,
    last_node: u32,
    start: usize,
    points: usize,
    open: bool,
}

impl Reader {
    fn node(&mut self, p: V) -> u32 {
        self.nodes.push(p);
        (self.nodes.len() - 1) as u32
    }

    fn begin(&mut self, p: V) {
        self.first = p;
        self.first_node = self.node(p);
        (self.last, self.last_node) = (p, self.first_node);
        (self.start, self.points, self.open) = (self.curves.len(), 1, true);
    }

    fn add(&mut self, mut p: [V; 4], n: usize) {
        if !self.open {
            self.begin(self.last);
        }
        p[0] = self.last;
        if p[1..n].iter().all(|v| *v == self.last) {
            return;
        }
        let to = self.node(p[n - 1]);
        self.curves.push(Curve { bez: Bez { p, n }, from: self.last_node, to });
        (self.last, self.last_node) = (p[n - 1], to);
        self.points += n - 1;
    }

    fn end(&mut self) {
        if !self.open {
            return;
        }
        if self.points < 3 {
            self.curves.truncate(self.start);
        } else {
            if self.last != self.first {
                self.add([self.last, self.first, [0.0; 2], [0.0; 2]], 2);
            }
            let first = self.first_node;
            if let Some(c) = self.curves[self.start..].last_mut() {
                c.to = first;
            }
        }
        (self.open, self.last) = (false, self.first);
    }
}

impl OutlinePen for Reader {
    fn move_to(&mut self, x: f32, y: f32) {
        self.end();
        self.begin([f64::from(x), f64::from(y)]);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.add([[0.0; 2], [f64::from(x), f64::from(y)], [0.0; 2], [0.0; 2]], 2);
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.add([[0.0; 2], [f64::from(cx), f64::from(cy)], [f64::from(x), f64::from(y)], [0.0; 2]], 3);
    }

    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        let (a, b, e) = ([f64::from(ax), f64::from(ay)], [f64::from(bx), f64::from(by)], [f64::from(x), f64::from(y)]);
        self.add([[0.0; 2], a, b, e], 4);
    }

    fn close(&mut self) {
        self.end();
    }
}

// A stretch of a curve between two cuts.
struct Piece {
    curve: u32,
    t0: f64,
    t1: f64,
    from: u32,
    to: u32,
}

struct Removal {
    curves: Vec<Curve>,
    nodes: Vec<V>,
    parent: Vec<u32>,
    cuts: Vec<(u32, f64, u32)>,
    found: Vec<(f64, f64)>,
    // Points within `tol` are one.
    tol: f64,
    // Halvings left for the pair being met, and the work left for the whole outline.
    budget: usize,
    work: u64,
}

// Too close to call, or past the work the curves allow.
struct Unsure;

// What removing found: whether some point is wound twice or more, and the outline without its overlaps where it
// differs from the input.
#[derive(Default)]
struct Resolved {
    overlapped: bool,
    removed: Option<Path>,
}

/// The outline's overlaps under nonzero removed: curves split where they meet, each piece kept where exactly one
/// side is filled and turned to have it on the left, the pieces joined into contours. Every meeting is one point
/// both curves share, so the contours close exactly.
pub(crate) fn remove_overlaps(path: &Path) -> Overlaps {
    let mut reader = Reader::default();
    path.replay(None, &mut reader);
    match resolve(reader) {
        Ok(Resolved { removed: Some(removed), .. }) => Overlaps::Removed(removed),
        Ok(_) => Overlaps::None,
        Err(Unsure) => Overlaps::Unsure,
    }
}

// Meetings are found by a sweep from the bottom up, testing only curves side by side on it, as they are just below any
// meeting, and reading off each stretch's winding: O((n + k) log n) for n curves meeting k times, as the work allows.
fn resolve(reader: Reader) -> Result<Resolved, Unsure> {
    let Reader { curves, nodes, .. } = reader;
    if curves.is_empty() {
        return Ok(Resolved::default());
    }
    if !curves.iter().all(|c| c.bez.p[..c.bez.n].iter().flatten().all(|v| v.is_finite())) {
        return Err(Unsure);
    }
    let b = curves.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, c| {
        let q = c.bez.bounds();
        [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[2]), b[3].max(q[3])]
    });
    let scale = (b[2] - b[0]).max(b[3] - b[1]).max(b[0].abs()).max(b[1].abs()).max(b[2].abs()).max(b[3].abs());
    let (tol, mut nodes) = (scale * 1e-10, nodes);
    let line = |a: V, b: V| Bez { p: [a, b, [0.0; 2], [0.0; 2]], n: 2 };
    let curved = |c: &Curve| c.bez.n == 2 || !straight(&c.bez, tol);
    // A curve along one line is that line, run back where it turns, so it lies along lines as they do.
    let lines = if curves.iter().all(curved) {
        curves
    } else {
        let mut lines = Vec::with_capacity(curves.len());
        for c in curves {
            if curved(&c) {
                lines.push(c);
                continue;
            }
            let (turns, n) = turns(&c.bez);
            let (mut from, mut at) = (c.from, c.bez.p[0]);
            for p in turns[..n].iter().map(|&t| c.bez.at(t)) {
                nodes.push(p);
                let to = (nodes.len() - 1) as u32;
                lines.push(Curve { bez: line(at, p), from, to });
                (from, at) = (to, p);
            }
            lines.push(Curve { bez: line(at, c.bez.end()), from, to: c.to });
        }
        lines
    };
    let parent = (0..nodes.len() as u32).collect();
    let n = lines.len() as u64;
    let work = WORK + WORK_PER_GLYPH_CURVE * n.min(GLYPH_CURVES) + WORK_PER_CURVE * n.saturating_sub(GLYPH_CURVES);
    let mut r = Removal {
        curves: lines, nodes, parent, cuts: Vec::new(), found: Vec::new(), tol, budget: 0, work: work / 16,
    };
    // Pairs are met in the order of an x-sorted pass, the earlier curve first, which numbers the nodes as it does.
    let boxes: Vec<[f64; 4]> = r.curves.iter().map(|c| c.bez.bounds()).collect();
    let mut order: Vec<u32> = (0..r.curves.len() as u32).collect();
    order.sort_unstable_by(|&i, &j| boxes[i as usize][0].total_cmp(&boxes[j as usize][0]));
    // The cheap check takes a sixteenth of the outline's work at most, so an outline it can't settle keeps the rest.
    if r.plain(&boxes, &order) {
        return Ok(Resolved::default());
    }
    r.work += work - work / 16;
    let read = r.nodes.len();
    r.unite_ends();
    let united = r.parent.clone();
    let mut rank = alloc::vec![0u32; order.len()];
    order.iter().enumerate().for_each(|(k, &i)| rank[i as usize] = k as u32);
    let mut sweep = Sweep::new(&r, boxes, rank);
    sweep.run(&mut r)?;

    // The sweep's meetings cut again from a fresh start, pair by pair in that order, so the pieces are its stretches.
    r.nodes.truncate(read);
    r.parent = united;
    r.cuts.clear();
    let rank = |c: u32| sweep.rank[c as usize];
    let mut pairs: Vec<&Met> = sweep.met.iter().collect();
    pairs.sort_unstable_by_key(|&&(i, j, _)| (rank(i), rank(j)));
    for &(i, j, ref met) in pairs {
        r.meet_at(i, j, met);
    }

    let pieces = r.pieces();
    let (mut overlapped, mut dropped, mut forward, mut backward) = (false, false, false, false);
    let mut kept: Vec<(usize, bool)> = Vec::with_capacity(pieces.len());
    // Each piece's stretch, the one holding its middle, walked to in step as both run in order.
    let (mut along, mut at) = (sweep.along.iter().peekable(), 0);
    for (k, p) in pieces.iter().enumerate() {
        let middle = (p.curve, (0.5 * (p.t0 + p.t1)).to_bits());
        while let Some((_, &next)) = along.next_if(|(key, _)| **key <= middle) {
            at = next;
        }
        let item = &sweep.items[at as usize];
        let w = sweep.items[item.lead as usize].w;
        let (left, right) = if item.up { (w[0], w[1]) } else { (w[1], w[0]) };
        overlapped |= left.abs() >= 2 || right.abs() >= 2;
        // Of stretches running together, the first stands for all.
        match (left != 0, right != 0, item.lead == at) {
            (true, false, true) => (kept.push((k, false)), forward = true).1,
            (false, true, true) => (kept.push((k, true)), backward = true).1,
            _ => dropped = true,
        }
    }
    // Kept whole and all one way round, the outline already fills as a sum does.
    if !dropped && !(forward && backward) {
        return Ok(Resolved { overlapped, removed: None });
    }
    let removed = r.join(&pieces, &kept).ok_or(Unsure)?;
    Ok(Resolved { overlapped, removed: Some(removed) })
}

impl Removal {
    // A step's price spent, or None once the outline's work is.
    fn spend(&mut self, price: u64) -> Option<()> {
        self.work = self.work.checked_sub(price)?;
        Some(())
    }

    fn find(&mut self, mut n: u32) -> u32 {
        while self.parent[n as usize] != n {
            let up = self.parent[self.parent[n as usize] as usize];
            self.parent[n as usize] = up;
            n = up;
        }
        n
    }

    // The lower node stands for both, so a curve's own end outranks a crossing found on it.
    fn union(&mut self, a: u32, b: u32) {
        let (a, b) = (self.find(a), self.find(b));
        self.parent[a.max(b) as usize] = a.min(b);
    }

    // Ends at one point are one node, whether or not the curves cross there.
    fn unite_ends(&mut self) {
        let mut ends: Vec<(u64, u64, u32)> =
            self.nodes.iter().zip(0..).map(|(p, n)| (p[0].to_bits(), p[1].to_bits(), n)).collect();
        ends.sort_unstable();
        for w in ends.windows(2) {
            if (w[0].0, w[0].1) == (w[1].0, w[1].1) {
                self.union(w[0].2, w[1].2);
            }
        }
    }

    // Where curves i and j meet, each meeting cut into both, or their ends made one node; also the meetings, for
    // `meet_at` to cut again. None where they're too close to call.
    fn pair(&mut self, i: u32, j: u32) -> Option<Vec<(f64, f64)>> {
        let met = self.meetings(i, j)?;
        self.meet_at(i, j, &met);
        Some(met)
    }

    // Where curves i and j meet, refined, once each, in the order halving finds them. Lines along one another meet
    // where either ends inside the other, which `meet_at` works out.
    fn meetings(&mut self, i: u32, j: u32) -> Option<Vec<(f64, f64)>> {
        let (a, b) = (self.curves[i as usize].bez, self.curves[j as usize].bez);
        if self.together(&a, &b) {
            return Some(Vec::new());
        }
        let (mut found, mut met) = (core::mem::take(&mut self.found), Vec::new());
        found.clear();
        self.budget = HALVINGS_PER_PAIR;
        let halved = self.meet(&a, (0.0, 1.0), &b, (0.0, 1.0), &mut found);
        self.found = found;
        halved?;
        let found = &self.found;
        let at_end = |v: f64| v <= END || v >= 1.0 - END;
        for &(t, u) in found {
            let (t, u) = if at_end(t) && at_end(u) {
                (t, u)
            } else {
                match refine(&a, &b, t, u, self.tol) {
                    Meeting::At(t, u) => (t, u),
                    Meeting::Apart => continue,
                    Meeting::Tangent => return None,
                }
            };
            if !met.iter().any(|&(s, v): &(f64, f64)| (s - t).abs() <= END && (v - u).abs() <= END) {
                met.push((t, u));
            }
        }
        Some(met)
    }

    // Curves i and j cut where they meet, at `met` as `meetings` found it.
    fn meet_at(&mut self, i: u32, j: u32, met: &[(f64, f64)]) {
        let (a, b) = (self.curves[i as usize], self.curves[j as usize]);
        if a.bez.n == 2 && b.bez.n == 2 && self.along(&a.bez, &b.bez) {
            for (c, d, k) in [(a, b, i), (b, a, j)] {
                for (node, q) in [(d.from, d.bez.p[0]), (d.to, d.bez.end())] {
                    let (s, e) = (c.bez.p[0], c.bez.end());
                    let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
                    let t = ((q[0] - s[0]) * dx + (q[1] - s[1]) * dy) / (dx * dx + dy * dy);
                    if END < t && t < 1.0 - END {
                        self.cuts.push((k, t, node));
                    }
                }
            }
            return;
        }
        let node_at = |c: &Curve, v: f64| {
            if v <= END {
                Some(c.from)
            } else if v >= 1.0 - END {
                Some(c.to)
            } else {
                None
            }
        };
        for &(t, u) in met {
            match (node_at(&a, t), node_at(&b, u)) {
                (Some(x), Some(y)) => {
                    let (p, q) = (self.nodes[x as usize], self.nodes[y as usize]);
                    if (p[0] - q[0]).abs().max((p[1] - q[1]).abs()) <= self.tol && self.find(x) != self.find(y) {
                        self.union(x, y);
                    }
                }
                (Some(x), None) => self.cuts.push((j, u, x)),
                (None, Some(y)) => self.cuts.push((i, t, y)),
                (None, None) => {
                    // A meeting on a flat lies at its height, which the sweep reaches it at.
                    let mut at = a.bez.at(t);
                    if let Some(flat) = [a.bez, b.bez].iter().find(|c| c.n == 2 && c.p[0][1] == c.p[1][1]) {
                        at[1] = flat.p[0][1];
                    }
                    self.nodes.push(at);
                    let n = (self.nodes.len() - 1) as u32;
                    self.parent.push(n);
                    self.cuts.extend([(i, t, n), (j, u, n)]);
                }
            }
        }
    }

    // Curves of the same points, either way round, and lines along one line lie along one another, meeting only at
    // their ends.
    fn together(&self, a: &Bez, b: &Bez) -> bool {
        let same = a.n == b.n && (a.p[..a.n] == b.p[..b.n] || a.p[..a.n].iter().eq(b.p[..b.n].iter().rev()));
        same || (a.n == 2 && b.n == 2 && self.along(a, b))
    }

    // Whether the outline already fills as a sum does, cheaply: no curves meet but where a contour runs from one to the
    // next, none leave a node alike, and every contour is wound zero on one side only, all one way round.
    fn plain(&mut self, boxes: &[[f64; 4]], order: &[u32]) -> bool {
        let n = self.curves.len();
        let starts: Vec<usize> = (0..n).filter(|&k| k == 0 || self.curves[k].from != self.curves[k - 1].to).collect();
        let span = |s: usize| starts[s]..starts.get(s + 1).copied().unwrap_or(n);
        let mut turn = Vec::with_capacity(starts.len());
        for (s, &start) in starts.iter().enumerate() {
            let mut area = 0.0;
            for k in span(s) {
                let next = if k + 1 < span(s).end { k + 1 } else { start };
                let (c, d) = (&self.curves[k].bez, &self.curves[next].bez);
                area += swept(c);
                let back = c.p[..c.n - 1].iter().rev().find(|q| **q != c.end()).map_or([0.0; 2], |q| sub(*q, c.end()));
                let on = d.p[1..d.n].iter().find(|q| **q != d.p[0]).map_or([0.0; 2], |q| sub(*q, d.p[0]));
                let (cross, dot) = (back[0] * on[1] - back[1] * on[0], back[0] * on[0] + back[1] * on[1]);
                let lengths = (back[0] * back[0] + back[1] * back[1]) * (on[0] * on[0] + on[1] * on[1]);
                if dot >= 0.0 && cross * cross <= 1e-12 * lengths {
                    return false;
                }
            }
            if area == 0.0 {
                return false;
            }
            turn.push(if area > 0.0 { 1 } else { -1 });
        }
        // Curves a contour runs through from one to the next meet only at that node where a line through it parts
        // them; any other two meet nowhere where halving finds no chords meeting.
        let mut found = Vec::new();
        let node = |c: &Curve, t: f64| if t <= END { Some(c.from) } else if t >= 1.0 - END { Some(c.to) } else { None };
        for (a, &i) in order.iter().enumerate() {
            for &j in &order[a + 1..] {
                let (bi, bj) = (boxes[i as usize], boxes[j as usize]);
                if bj[0] > bi[2] {
                    break;
                }
                if self.spend(STEP).is_none() {
                    return false;
                }
                if apart(bi, bj) {
                    continue;
                }
                let (ci, cj) = (self.curves[i as usize], self.curves[j as usize]);
                let linked = ci.to == cj.from || cj.to == ci.from;
                if linked && parted(&ci.bez, [true; 2], &cj.bez, [true; 2]) {
                    continue;
                }
                if self.together(&ci.bez, &cj.bez) {
                    return false;
                }
                if linked {
                    let Some(met) = self.meetings(i, j) else { return false };
                    if !met.iter().all(|&(t, u)| node(&ci, t).is_some() && node(&ci, t) == node(&cj, u)) {
                        return false;
                    }
                    continue;
                }
                self.budget = FAST_HALVINGS;
                if self.meet(&ci.bez, (0.0, 1.0), &cj.bez, (0.0, 1.0), &mut found).is_none() || !found.is_empty() {
                    return false;
                }
            }
        }
        let (mut forward, mut backward) = (false, false);
        for s in 0..starts.len() {
            let q = self.nodes[self.curves[starts[s]].from as usize];
            let mut outside = 0;
            for k in (0..n).filter(|k| !span(s).contains(k)) {
                let Some(w) = self.crossings(self.curves[k].bez, q) else { return false };
                outside += w;
            }
            let inside = outside + turn[s];
            let (left, right) = if turn[s] > 0 { (inside, outside) } else { (outside, inside) };
            match (left != 0, right != 0) {
                (true, false) => forward = true,
                (false, true) => backward = true,
                _ => return false,
            }
        }
        !(forward && backward)
    }

    // A rightward ray from q against one curve, counted upward positive; a stretch wholly right of q crosses as its
    // ends say, one across q is halved until it isn't or is flat, and None where q lies within `tol` of it.
    fn crossings(&mut self, b: Bez, q: V) -> Option<i32> {
        self.spend(STEP)?;
        let bb = b.bounds();
        if bb[3] <= q[1] || bb[1] > q[1] || bb[2] < q[0] - self.tol {
            return Some(0);
        }
        let (a, e) = (b.p[0], b.end());
        let sign = match (a[1] <= q[1], e[1] <= q[1]) {
            (true, false) => 1,
            (false, true) => -1,
            _ => 0,
        };
        if bb[0] > q[0] + self.tol {
            return Some(sign);
        }
        if b.flat(bb, self.tol) {
            if sign == 0 {
                return Some(0);
            }
            let x = a[0] + (q[1] - a[1]) * (e[0] - a[0]) / (e[1] - a[1]);
            return if (x - q[0]).abs() <= self.tol { None } else { Some(if x > q[0] { sign } else { 0 }) };
        }
        let (l, r) = b.split(0.5);
        Some(self.crossings(l, q)? + self.crossings(r, q)?)
    }

    // Whether two lines lie along one line, each end of either within `tol` of the other's.
    fn along(&self, a: &Bez, b: &Bez) -> bool {
        let off = |l: &Bez, q: V| {
            let (s, e) = (l.p[0], l.end());
            let (dx, dy) = (e[0] - s[0], e[1] - s[1]);
            let cross = (q[0] - s[0]) * dy - (q[1] - s[1]) * dx;
            cross * cross <= self.tol * self.tol * (dx * dx + dy * dy)
        };
        off(a, b.p[0]) && off(a, b.end()) && off(b, a.p[0]) && off(b, a.end())
    }

    // Both curves halved, the larger first, while their boxes meet; where both are flat their chords say where.
    fn meet(&mut self, a: &Bez, ta: (f64, f64), b: &Bez, tb: (f64, f64), found: &mut Vec<(f64, f64)>) -> Option<()> {
        self.budget = self.budget.checked_sub(1)?;
        self.spend(HALVING)?;
        let (ba, bb) = (a.bounds(), b.bounds());
        if apart(ba, bb)
            || beside_band(a, b)
            || beside_band(b, a)
            || parted(a, [ta.0 == 0.0, ta.1 == 1.0], b, [tb.0 == 0.0, tb.1 == 1.0])
        {
            return Some(());
        }
        let (fa, fb) = (a.flat(ba, self.tol), b.flat(bb, self.tol));
        if fa && fb {
            if let Some((s, u)) = chords_meet(a.p[0], a.end(), b.p[0], b.end()) {
                found.push((ta.0 + (ta.1 - ta.0) * s, tb.0 + (tb.1 - tb.0) * u));
            }
            return Some(());
        }
        // The larger is halved, a line too: one never halved keeps a box that meets every half of the other.
        let size = |b: [f64; 4]| (b[2] - b[0]).max(b[3] - b[1]);
        if size(ba) >= size(bb) {
            let ((l, r), m) = (a.split(0.5), 0.5 * (ta.0 + ta.1));
            self.meet(&l, (ta.0, m), b, tb, found)?;
            self.meet(&r, (m, ta.1), b, tb, found)
        } else {
            let ((l, r), m) = (b.split(0.5), 0.5 * (tb.0 + tb.1));
            self.meet(a, ta, &l, (tb.0, m), found)?;
            self.meet(a, ta, &r, (m, tb.1), found)
        }
    }

    // Each curve cut at its sorted cuts; cuts within END of each other or of an end are one node.
    fn pieces(&mut self) -> Vec<Piece> {
        let mut cuts = core::mem::take(&mut self.cuts);
        cuts.sort_unstable_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
        let mut out = Vec::with_capacity(self.curves.len() + cuts.len());
        let mut next = cuts.iter().peekable();
        for i in 0..self.curves.len() {
            let c = self.curves[i];
            let (mut t0, mut from) = (0.0, c.from);
            while let Some(&(_, t, node)) = next.next_if(|x| x.0 as usize == i) {
                if t - t0 <= END {
                    self.union(from, node);
                } else if t >= 1.0 - END {
                    self.union(node, c.to);
                } else {
                    out.push(Piece { curve: i as u32, t0, t1: t, from, to: node });
                    (t0, from) = (t, node);
                }
            }
            out.push(Piece { curve: i as u32, t0, t1: 1.0, from, to: c.to });
        }
        out
    }

    // The kept pieces walked into contours; every node must be left as often as it's entered.
    fn join(&mut self, pieces: &[Piece], kept: &[(usize, bool)]) -> Option<Path> {
        let mut edges: Vec<(u32, u32, u32, bool)> = Vec::with_capacity(kept.len());
        for &(k, back) in kept {
            let (a, b) = (self.find(pieces[k].from), self.find(pieces[k].to));
            edges.push(if back { (b, a, k as u32, true) } else { (a, b, k as u32, false) });
        }
        let mut balance = alloc::vec![0i32; self.nodes.len()];
        for e in &edges {
            balance[e.0 as usize] += 1;
            balance[e.1 as usize] -= 1;
        }
        if balance.iter().any(|&b| b != 0) {
            return None;
        }
        edges.sort_unstable_by_key(|e| (e.0, e.2));
        let mut cursor: Vec<usize> = (0..self.nodes.len() as u32).map(|n| edges.partition_point(|e| e.0 < n)).collect();
        let mut taken = alloc::vec![false; edges.len()];
        let mut out = Path::new();
        for s in 0..edges.len() {
            if taken[s] {
                continue;
            }
            let start = edges[s].0;
            let p = self.nodes[start as usize];
            out.move_to(p[0] as f32, p[1] as f32);
            let mut e = s;
            loop {
                taken[e] = true;
                self.emit(&mut out, pieces, edges[e]);
                let at = edges[e].1;
                if at == start {
                    break;
                }
                let c = &mut cursor[at as usize];
                while *c < edges.len() && edges[*c].0 == at && taken[*c] {
                    *c += 1;
                }
                if *c >= edges.len() || edges[*c].0 != at {
                    return None;
                }
                e = *c;
            }
            out.close();
        }
        Some(out)
    }

    fn emit(&self, out: &mut Path, pieces: &[Piece], (from, to, k, back): (u32, u32, u32, bool)) {
        let p = &pieces[k as usize];
        let mut b = self.curves[p.curve as usize].bez.part(p.t0, p.t1);
        if back {
            b.p[..b.n].reverse();
        }
        b.p[0] = self.nodes[from as usize];
        b.p[b.n - 1] = self.nodes[to as usize];
        let f = |v: V| (v[0] as f32, v[1] as f32);
        let (c, d, e) = (f(b.p[1]), f(b.p[2]), f(b.p[3]));
        match b.n {
            2 => out.line_to(c.0, c.1),
            3 => out.quad_to(c.0, c.1, d.0, d.1),
            _ => out.curve_to(c.0, c.1, d.0, d.1, e.0, e.1),
        }
    }
}

const NIL: u32 = u32::MAX;
// Directions at a point closer than this, as `slope` measures them, leave the curves side by side there.
const ANGLE: f64 = 1e-9;

// A stretch of a curve monotone in x and y, run from its lower end to its upper: lower y first, then lower x.
#[derive(Clone, Copy)]
struct Item {
    curve: u32,
    // Where it starts and ends along the curve, the start first.
    t: [f64; 2],
    bez: Bez,
    // Whether the curve runs lower end to upper, and the sum of that, +1 or -1, over the stretches lying along it.
    up: bool,
    dirs: i32,
    flat: bool,
    // The winding left and right of it as it runs up; for a flat one, above and below it.
    w: [i32; 2],
    // Which of its leaving events counts, raised when it's cut short; the stretch standing for those lying along it.
    end: u32,
    lead: u32,
    live: bool,
}

#[derive(Clone, Copy)]
struct Event {
    // Its point's y and x as `ordered` keys, which compare as the numbers do.
    key: [u64; 2],
    kind: u8,
    item: u32,
    version: u32,
}

impl Event {
    fn new(at: V, kind: u8, item: u32, version: u32) -> Event {
        Event { key: [ordered(at[1]), ordered(at[0])], kind, item, version }
    }

    fn at(&self) -> V {
        let number = |k: u64| f64::from_bits(if k >> 63 == 1 { k ^ 1 << 63 } else { !k });
        [number(self.key[1]), number(self.key[0])]
    }
}

// The kinds in the order a point takes them: a height's flat stretches first, then at each point those leaving
// before those entering.
const FLAT: u8 = 0;
const LEAVE: u8 = 1;
const ENTER: u8 = 2;

impl Ord for Event {
    fn cmp(&self, o: &Event) -> Ordering {
        self.key[0].cmp(&o.key[0])
            .then_with(|| (self.kind != FLAT).cmp(&(o.kind != FLAT)))
            .then_with(|| self.key[1].cmp(&o.key[1]))
            .then_with(|| self.kind.cmp(&o.kind))
            .then_with(|| self.item.cmp(&o.item))
    }
}

impl PartialOrd for Event {
    fn partial_cmp(&self, o: &Event) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl PartialEq for Event {
    fn eq(&self, o: &Event) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for Event {}

// Two curves tested, in the order they were met, and where they meet along each.
type Met = (u32, u32, Vec<(f64, f64)>);

// The sweep upward over the curves' monotone stretches. A stretch enters at its lower end, between those already on
// the line as they lie there; each pair it comes to lie beside is tested once, and a meeting cuts both stretches, so
// each leaves there and its rest enters, between the others in the order of their directions.
struct Sweep {
    items: Vec<Item>,
    // Each curve's stretches by where they start along it.
    along: BTreeMap<(u32, u64), u32>,
    line: Line,
    events: BinaryHeap<Reverse<Event>>,
    // Each pair of curves tested whose boxes meet, by their numbers, and of those that meet or lie along one another,
    // the two in the order they were met, with where they meet.
    paired: BTreeSet<(u32, u32)>,
    met: Vec<Met>,
    // Each curve's box, and its place in an x-sorted pass.
    boxes: Vec<[f64; 4]>,
    rank: Vec<u32>,
    at: V,
    tol: f64,
    // A height's flat stretches, kept from height to height.
    flat: Vec<u32>,
    // The stretches placed at the sweep's point, wound again once it's done, and the point each was last wound at.
    placed: Vec<u32>,
    wound: Vec<u32>,
    points: u32,
}


impl Sweep {
    fn new(r: &Removal, boxes: Vec<[f64; 4]>, rank: Vec<u32>) -> Sweep {
        let mut s = Sweep {
            items: Vec::with_capacity(r.curves.len()),
            along: BTreeMap::new(),
            line: Line::new(),
            events: BinaryHeap::new(),
            paired: BTreeSet::new(),
            met: Vec::new(),
            boxes,
            rank,
            at: [f64::MIN; 2],
            tol: r.tol,
            flat: Vec::new(),
            placed: Vec::new(),
            wound: Vec::new(),
            points: 0,
        };
        let (mut along, mut events) = (Vec::with_capacity(r.curves.len()), Vec::with_capacity(r.curves.len()));
        for (i, c) in r.curves.iter().enumerate() {
            let (turns, n) = turns(&c.bez);
            let mut from = (0.0, c.bez.p[0]);
            for &t in turns[..n].iter().chain([&1.0]) {
                let at = if t == 1.0 { c.bez.end() } else { c.bez.at(t) };
                let up = lower(from.1, at);
                let k = s.items.len() as u32;
                let (lo, hi) = if up { (from.1, at) } else { (at, from.1) };
                s.items.push(Item {
                    curve: i as u32, t: [from.0, t], bez: stretch(&c.bez, [from.0, t], up, lo, hi), up,
                    dirs: if up { 1 } else { -1 }, flat: lo[1] == hi[1], w: [0; 2], end: 0, lead: k, live: false,
                });
                along.push(((i as u32, from.0.to_bits()), k));
                events.push(Reverse(Event::new(lo, if lo[1] == hi[1] { FLAT } else { ENTER }, k, 0)));
                from = (t, at);
            }
        }
        // Built from the stretches in order, which a map and a heap take faster than one at a time.
        (s.along, s.events) = (along.into_iter().collect(), BinaryHeap::from(events));
        s
    }

    fn enter(&mut self, k: u32) {
        let it = &self.items[k as usize];
        let at = if self.passed(it.bez.p[0]) { self.at } else { it.bez.p[0] };
        self.events.push(Reverse(Event::new(at, if it.flat { FLAT } else { ENTER }, k, 0)));
    }

    // The stretch of `curve` holding `t`.
    fn item_at(&self, curve: u32, t: f64) -> u32 {
        self.along.range(..=(curve, t.to_bits())).next_back().map_or(0, |(_, &k)| k)
    }

    // Whether the sweep is past `p`, by more than `tol`.
    fn passed(&self, p: V) -> bool {
        p[1] < self.at[1] - self.tol || (p[1] <= self.at[1] + self.tol && p[0] < self.at[0] - self.tol)
    }

    fn run(&mut self, r: &mut Removal) -> Result<(), Unsure> {
        let (mut batch, mut entering, mut beside) = (Vec::new(), Vec::new(), Vec::new());
        while let Some(Reverse(e)) = self.events.pop() {
            if e.kind == FLAT || e.at() != self.at {
                self.wind_again();
            }
            if e.kind == FLAT {
                self.flats(r, e)?;
                continue;
            }
            self.at = e.at();
            batch.clear();
            batch.push(e);
            while let Some(&Reverse(n)) = self.events.peek()
                && n.kind != FLAT
                && n.key == e.key
            {
                batch.push(n);
                self.events.pop();
            }
            r.spend(EVENT * batch.len() as u64).ok_or(Unsure)?;
            self.point(r, &batch, &mut entering, &mut beside)?;
        }
        self.wind_again();
        Ok(())
    }

    // What leaves at a point, then what enters there, each stretch placed; then every pair newly side by side tested.
    fn point(
        &mut self, r: &mut Removal, batch: &[Event], entering: &mut Vec<u32>, beside: &mut Vec<(u32, u32)>,
    ) -> Result<(), Unsure> {
        beside.clear();
        for e in batch.iter().filter(|e| e.kind == LEAVE) {
            let it = self.items[e.item as usize];
            if !it.live || it.end != e.version {
                continue;
            }
            let [prev, next] = self.line.side(e.item);
            self.line.remove(e.item);
            self.items[e.item as usize].live = false;
            if prev != NIL && next != NIL {
                beside.push((prev, next));
            }
        }
        entering.clear();
        entering.extend(
            batch
                .iter()
                .filter(|e| e.kind == ENTER)
                .map(|e| e.item)
                .filter(|&k| !self.items[k as usize].live && self.items[k as usize].lead == k),
        );
        entering.sort_unstable();
        entering.dedup();
        // Stretches leaving the point alike are tested first, which cuts lines along one another to one length.
        let slope_of = |i: u32| slope(start(&self.items[i as usize]));
        entering.sort_by(|&a, &b| slope_of(b).total_cmp(&slope_of(a)));
        for w in 1..entering.len() {
            let (a, b) = (entering[w - 1], entering[w]);
            if (slope(start(&self.items[a as usize])) - slope(start(&self.items[b as usize]))).abs() <= ANGLE {
                self.test(r, a, b)?;
            }
        }
        let tol = self.tol;
        let alike = |a: &Bez, b: &Bez| {
            let near = |(p, q): (&V, &V)| (p[0] - q[0]).abs().max((p[1] - q[1]).abs()) <= tol;
            let ends = a.n == b.n && a.p[0] == b.p[0] && a.end() == b.end();
            ends && a.p[1..a.n - 1].iter().zip(&b.p[1..b.n - 1]).all(near)
        };
        for w in 1..entering.len() {
            let (a, b) = (&self.items[entering[w - 1] as usize], &self.items[entering[w] as usize]);
            if alike(&a.bez, &b.bez) {
                let (lead, dirs) = (a.lead, b.dirs);
                self.items[entering[w] as usize].lead = lead;
                self.items[lead as usize].dirs += dirs;
            }
        }
        for &q in entering.iter() {
            if self.items[q as usize].lead == q {
                self.place(r, q, beside)?;
            }
        }
        self.placed.extend_from_slice(entering);
        for &(a, b) in beside.iter() {
            if self.items[a as usize].live && self.items[b as usize].live {
                self.test(r, a, b)?;
            }
        }
        Ok(())
    }

    // Each run of stretches leaving the sweep's point wound again left to right, as they lie once all are placed there:
    // one placed first may have had another placed to its left since. Each is walked once, a run wound whole.
    fn wind_again(&mut self) {
        if self.placed.is_empty() {
            return;
        }
        let starts_here = |s: &Sweep, k: u32| k != NIL && s.items[k as usize].bez.p[0] == s.at;
        let placed = core::mem::take(&mut self.placed);
        self.points += 1;
        self.wound.resize(self.items.len(), 0);
        for &q in &placed {
            if !self.items[q as usize].live || self.items[q as usize].lead != q {
                continue;
            }
            let mut k = q;
            while self.wound[k as usize] != self.points && starts_here(self, self.line.side(k)[0]) {
                k = self.line.side(k)[0];
            }
            if self.wound[k as usize] == self.points {
                continue;
            }
            let prev = self.line.side(k)[0];
            let mut left = if prev == NIL { 0 } else { self.items[prev as usize].w[1] };
            while starts_here(self, k) {
                self.wound[k as usize] = self.points;
                let it = &mut self.items[k as usize];
                it.w = [left, left - it.dirs];
                left = it.w[1];
                k = self.line.side(k)[1];
            }
        }
        self.placed = placed;
        self.placed.clear();
    }

    // A stretch onto the line where it lies at the sweep's point, or, beside one it can't yet be told from, the two
    // tested and it tried again once they're cut there.
    fn place(&mut self, r: &mut Removal, q: u32, beside: &mut Vec<(u32, u32)>) -> Result<(), Unsure> {
        self.line.grow(self.items.len());
        let mut spent = 0;
        let placed = {
            let (items, paired, p, tol) = (&self.items, &self.paired, self.at, self.tol);
            self.line.insert(q, |a| left_of(items, paired, q, a, p, tol, &mut spent))
        };
        r.spend(spent).ok_or(Unsure)?;
        match placed {
            Ok(()) => {
                let [prev, next] = self.line.side(q);
                let left = if prev == NIL { 0 } else { self.items[prev as usize].w[1] };
                let it = &mut self.items[q as usize];
                (it.w, it.live) = ([left, left - it.dirs], true);
                let at = it.bez.end();
                self.events.push(Reverse(Event::new(at, LEAVE, q, it.end)));
                beside.extend([(prev, q), (q, next)].into_iter().filter(|&(a, b)| a != NIL && b != NIL));
            }
            Err(a) => {
                let made = self.items.len();
                self.test(r, a, q)?;
                if self.items.len() == made {
                    return Err(Unsure);
                }
                let at = self.at;
                self.events.push(Reverse(Event::new(at, ENTER, q, 0)));
            }
        }
        Ok(())
    }

    // A height's flat stretches: each tested with what crosses or touches it there, then wound above and below by
    // the line as it lies just below. Flat stretches lying along one another are one.
    fn flats(&mut self, r: &mut Removal, first: Event) -> Result<(), Unsure> {
        let (y, height) = (first.at()[1], first.key[0]);
        let (mut flats, mut here) = (core::mem::take(&mut self.flat), BTreeMap::new());
        flats.clear();
        flats.push(first.item);
        let gather = |s: &mut Sweep, flats: &mut Vec<u32>, here: &mut BTreeMap<(u64, usize), Event>| {
            while let Some(&Reverse(e)) = s.events.peek()
                && e.key[0] == height
            {
                s.events.pop();
                match e.kind {
                    FLAT => flats.push(e.item),
                    _ => _ = here.insert((e.key[1], here.len()), e),
                }
            }
        };
        gather(self, &mut flats, &mut here);
        // Flats overlapping one another, taken by their left ends, each tested with those reaching it. Cuts only
        // split these flats, and pairs are tested once a curve, so the pieces need no more.
        let span = |f: u32| (self.items[f as usize].bez.p[0][0], self.items[f as usize].bez.end()[0], f);
        let mut spans: Vec<(f64, f64, u32)> = flats.iter().map(|&f| span(f)).collect();
        spans.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
        let mut reaching: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::new();
        for (x0, x1, f) in spans {
            while reaching.peek().is_some_and(|&Reverse((end, _))| end < ordered(x0 - self.tol)) {
                reaching.pop();
            }
            for &Reverse((_, g)) in reaching.iter() {
                self.test(r, f, g)?;
            }
            reaching.push(Reverse((ordered(x1), f)));
        }
        let (mut k, mut touching) = (0, Vec::new());
        while k < flats.len() {
            r.spend(FLAT_STRETCH).ok_or(Unsure)?;
            let (f, h) = (flats[k], self.items[flats[k] as usize]);
            k += 1;
            let (x0, x1) = (h.bez.p[0][0] - self.tol, h.bez.end()[0] + self.tol);
            let left = self.line.seek(|a| x_at(&self.items[a as usize], y) < x0);
            let mut a = if left == NIL { self.line.first() } else { self.line.side(left)[1] };
            touching.clear();
            while a != NIL && x_at(&self.items[a as usize], y) <= x1 {
                touching.push(a);
                a = self.line.side(a)[1];
            }
            let entering = here.range((ordered(x0), 0)..=(ordered(x1), usize::MAX)).map(|(_, e)| e);
            touching.extend(entering.filter(|e| e.kind == ENTER).map(|e| e.item));
            for &a in &touching {
                self.test(r, f, a)?;
            }
            gather(self, &mut flats, &mut here);
        }
        flats.sort_by(|&a, &b| {
            let (a, b) = (&self.items[a as usize].bez, &self.items[b as usize].bez);
            a.p[0][0].total_cmp(&b.p[0][0]).then(a.end()[0].total_cmp(&b.end()[0]))
        });
        let mut lead = NIL;
        for &f in &flats {
            let it = self.items[f as usize];
            let held = (lead != NIL).then(|| self.items[lead as usize].bez);
            if held.is_some_and(|h| h.p[0] == it.bez.p[0] && h.end() == it.bez.end()) {
                self.items[f as usize].lead = lead;
                self.items[lead as usize].dirs += it.dirs;
                continue;
            }
            if lead != NIL && it.bez.p[0][0] < self.items[lead as usize].bez.end()[0] - self.tol {
                return Err(Unsure);
            }
            lead = f;
        }
        for &f in &flats {
            if self.items[f as usize].lead != f {
                continue;
            }
            let b = self.items[f as usize].bez;
            let mid = 0.5 * (b.p[0][0] + b.end()[0]);
            let left = self.line.seek(|a| x_at(&self.items[a as usize], y) < mid);
            let below = if left == NIL { 0 } else { self.items[left as usize].w[1] };
            let it = &mut self.items[f as usize];
            it.w = [below + it.dirs, below];
        }
        here.into_values().for_each(|e| self.events.push(Reverse(e)));
        self.flat = flats;
        Ok(())
    }

    // Two stretches' curves tested where they meet, once a pair, and every stretch holding a meeting cut there.
    fn test(&mut self, r: &mut Removal, a: u32, b: u32) -> Result<(), Unsure> {
        r.spend(LOOKUP).ok_or(Unsure)?;
        let (ca, cb) = (self.items[a as usize].curve, self.items[b as usize].curve);
        let key = (ca.min(cb), ca.max(cb));
        if ca == cb || apart(self.boxes[ca as usize], self.boxes[cb as usize]) || self.paired.contains(&key) {
            return Ok(());
        }
        r.spend(PAIR).ok_or(Unsure)?;
        let (i, j) = if self.rank[ca as usize] < self.rank[cb as usize] { (ca, cb) } else { (cb, ca) };
        let made = r.cuts.len();
        let met = r.pair(i, j).ok_or(Unsure)?;
        r.spend(CUT * (r.cuts.len() - made) as u64).ok_or(Unsure)?;
        self.paired.insert(key);
        // The replay cuts only where a pair met or lies along one another.
        if !met.is_empty() || r.together(&r.curves[i as usize].bez, &r.curves[j as usize].bez) {
            self.met.push((i, j, met));
        }
        for c in made..r.cuts.len() {
            let (curve, t, node) = r.cuts[c];
            self.cut(&r.curves[curve as usize].bez, curve, t, r.nodes[node as usize])?;
        }
        Ok(())
    }

    // The stretch holding `t` cut at `p`: it keeps the part from its lower end, and the rest enters at `p`. A cut the
    // sweep has passed is a meeting found too late to wind what lies beside it.
    fn cut(&mut self, bez: &Bez, curve: u32, t: f64, p: V) -> Result<(), Unsure> {
        let k = self.item_at(curve, t);
        let it = self.items[k as usize];
        if t - it.t[0] <= END || it.t[1] - t <= END {
            return Ok(());
        }
        if self.passed(p) {
            return Err(Unsure);
        }
        let (keep, rest) = if it.up { ([it.t[0], t], [t, it.t[1]]) } else { ([t, it.t[1]], [it.t[0], t]) };
        let new = self.items.len() as u32;
        let (lo, hi) = (it.bez.p[0], it.bez.end());
        self.items[k as usize].t = keep;
        self.items[k as usize].bez = stretch(bez, keep, it.up, lo, p);
        self.items.push(Item {
            t: rest, bez: stretch(bez, rest, it.up, p, hi), w: [0; 2], end: 0, lead: new, live: false, ..it
        });
        self.along.insert((curve, keep[0].to_bits()), k);
        self.along.insert((curve, rest[0].to_bits()), new);
        if it.live {
            let version = self.items[k as usize].end + 1;
            self.items[k as usize].end = version;
            let at = if self.passed(p) { self.at } else { p };
            self.events.push(Reverse(Event::new(at, LEAVE, k, version)));
        }
        self.enter(new);
        Ok(())
    }
}

// Where `q`, entering at the sweep's point `p`, goes against `a` on the line: left of it or not, or `a` where the two
// can't yet be told apart, leaving `p` alike.
fn left_of(
    items: &[Item], paired: &BTreeSet<(u32, u32)>, q: u32, a: u32, p: V, tol: f64, spent: &mut u64,
) -> Result<bool, u32> {
    *spent += STEP;
    let (qi, ai) = (&items[q as usize], &items[a as usize]);
    // A stretch monotone in x lies between its ends' x, so one far enough to a side needs no point on it worked out.
    let (x0, x1) = (ai.bez.p[0][0], ai.bez.end()[0]);
    if p[0] < x0.min(x1) - 2.0 * tol || p[0] > x0.max(x1) + 2.0 * tol {
        return Ok(p[0] < x0);
    }
    let xa = x_at(ai, p[1]);
    if (xa - p[0]).abs() > tol {
        return Ok(xa > p[0]);
    }
    let (sq, sa) = (slope(start(qi)), slope(tangent_at(ai, p[1])));
    if (sq - sa).abs() > ANGLE {
        return Ok(sq > sa);
    }
    // Leaving the point alike, stretches of curves already tested don't cross, so they lie as they do higher up.
    *spent += LOOKUP;
    let pair = (qi.curve.min(ai.curve), qi.curve.max(ai.curve));
    if qi.curve == ai.curve || paired.contains(&pair) {
        let top = qi.bez.end()[1].min(ai.bez.end()[1]);
        for y in [top, 0.5 * (p[1] + top)] {
            let (xq, xa) = (x_at(qi, y), x_at(ai, y));
            if (xq - xa).abs() > tol {
                return Ok(xq < xa);
            }
        }
        // Alike as far as both go, they lie along one another, and nothing lies between them to wind.
        return Ok(q < a);
    }
    Err(a)
}

// A coordinate as a key ordered as the numbers are, zeros as one.
fn ordered(v: f64) -> u64 {
    let bits = (v + 0.0).to_bits();
    if bits >> 63 == 0 { bits | 1 << 63 } else { !bits }
}

// Where a stretch's curve, rising along it, reaches height y: Newton's steps from the chord's guess, halving where a
// step would leave the bounds the steps so far have set.
fn param_at(b: &Bez, y: f64) -> f64 {
    let (lo, hi) = (b.p[0], b.end());
    if y <= lo[1] {
        return 0.0;
    }
    if y >= hi[1] {
        return 1.0;
    }
    // The height and its rate of change, as `Bez::at` works the height out.
    let height = |t: f64| {
        let mut q: [f64; 4] = core::array::from_fn(|k| b.p[k][1]);
        let mut rate = 0.0;
        for k in (1..b.n).rev() {
            if k == 1 {
                rate = (b.n - 1) as f64 * (q[1] - q[0]);
            }
            for i in 0..k {
                q[i] += (q[i + 1] - q[i]) * t;
            }
        }
        (q[0], rate)
    };
    let (mut s0, mut s1) = (0.0, 1.0);
    let mut t = (y - lo[1]) / (hi[1] - lo[1]);
    for _ in 0..64 {
        let (h, rate) = height(t);
        if h == y {
            break;
        }
        if h < y { s0 = t } else { s1 = t }
        let step = t - (h - y) / rate;
        let next = if s0 < step && step < s1 { step } else { 0.5 * (s0 + s1) };
        if next == t || next <= s0 || next >= s1 {
            break;
        }
        t = next;
    }
    t
}

fn x_at(it: &Item, y: f64) -> f64 {
    let (b, (lo, hi)) = (&it.bez, (it.bez.p[0], it.bez.end()));
    if b.n == 2 && lo[1] < y && y < hi[1] {
        return lo[0] + (y - lo[1]) / (hi[1] - lo[1]) * (hi[0] - lo[0]);
    }
    b.at(param_at(b, y))[0]
}

fn tangent_at(it: &Item, y: f64) -> V {
    let d = it.bez.tangent(param_at(&it.bez, y));
    if d == [0.0, 0.0] { start(it) } else { d }
}

// The direction a stretch leaves its lower end: toward its first control point elsewhere, as a curve does.
fn start(it: &Item) -> V {
    let b = &it.bez;
    let q = b.p[1..b.n].iter().find(|q| **q != b.p[0]).unwrap_or(&b.p[0]);
    [q[0] - b.p[0][0], q[1] - b.p[0][1]]
}

// A direction upward as a number rising from -1 pointing right to 1 pointing left.
fn slope(d: V) -> f64 {
    -d[0] / (d[0].abs() + d[1].abs())
}

// Whether `a` comes no later than `b` going up: lower y, then lower x.
fn lower(a: V, b: V) -> bool {
    a[1] < b[1] || (a[1] == b[1] && a[0] <= b[0])
}

// The part of a curve between `t`, run from `lo` to `hi`, its ends those points exactly.
fn stretch(bez: &Bez, t: [f64; 2], up: bool, lo: V, hi: V) -> Bez {
    let mut b = bez.part(t[0], t[1]);
    if !up {
        b.p[..b.n].reverse();
    }
    (b.p[0], b.p[b.n - 1]) = (lo, hi);
    b
}

// Where a curve turns in x or y, inside it, in order: there it splits into stretches monotone in both.
fn turns(b: &Bez) -> ([f64; 4], usize) {
    let (mut ts, mut n) = ([0.0; 4], 0);
    for axis in 0..2 {
        let d: [f64; 3] = core::array::from_fn(|k| if k + 1 < b.n { b.p[k + 1][axis] - b.p[k][axis] } else { 0.0 });
        let mut root = |t: f64| {
            if END < t && t < 1.0 - END {
                ts[n] = t;
                n += 1;
            }
        };
        match b.n {
            3 if d[0] != d[1] => root(d[0] / (d[0] - d[1])),
            4 => {
                let (a, bq, c) = (d[0] - 2.0 * d[1] + d[2], 2.0 * (d[1] - d[0]), d[0]);
                let f = |t: f64| (a * t + bq) * t + c;
                // Each root found by halving, on either side of the derivative's own turn where it runs one way: sqrt
                // works out differently in a build without std.
                let turn = if a == 0.0 { 0.0 } else { (-bq / (2.0 * a)).clamp(0.0, 1.0) };
                for (mut s0, mut s1) in [(0.0, turn), (turn, 1.0)] {
                    // A zero at either end is a root the derivative only touches, which turns nothing.
                    let below = f(s0) < 0.0;
                    if f(s0) * f(s1) >= 0.0 {
                        continue;
                    }
                    for _ in 0..64 {
                        let m = 0.5 * (s0 + s1);
                        if (f(m) < 0.0) == below { s0 = m } else { s1 = m }
                    }
                    root(0.5 * (s0 + s1));
                }
            }
            _ => {}
        }
    }
    ts[..n].sort_by(f64::total_cmp);
    let mut kept = 0;
    for k in 0..n {
        if kept == 0 || ts[k] - ts[kept - 1] > END {
            ts[kept] = ts[k];
            kept += 1;
        }
    }
    (ts, kept)
}

// The stretches on the sweep line, left to right: a treap over their numbers, each linked to its neighbors.
struct Line {
    nodes: Vec<Node>,
    root: u32,
    seed: u32,
}

#[derive(Clone, Copy)]
struct Node {
    kid: [u32; 2],
    up: u32,
    side: [u32; 2],
    prio: u32,
}

impl Default for Node {
    fn default() -> Node {
        Node { kid: [NIL; 2], up: NIL, side: [NIL; 2], prio: 0 }
    }
}

impl Line {
    fn new() -> Line {
        Line { nodes: Vec::new(), root: NIL, seed: 0x9E37_79B9 }
    }

    fn grow(&mut self, n: usize) {
        if self.nodes.len() < n {
            self.nodes.resize(n, Node::default());
        }
    }

    fn side(&self, k: u32) -> [u32; 2] {
        self.nodes[k as usize].side
    }

    fn first(&self) -> u32 {
        let mut at = self.root;
        while at != NIL && self.nodes[at as usize].kid[0] != NIL {
            at = self.nodes[at as usize].kid[0];
        }
        at
    }

    // The rightmost stretch `left` holds for, which holds for all left of it.
    fn seek(&self, mut left: impl FnMut(u32) -> bool) -> u32 {
        let (mut at, mut found) = (self.root, NIL);
        while at != NIL {
            let go = left(at);
            if go {
                found = at;
            }
            at = self.nodes[at as usize].kid[usize::from(go)];
        }
        found
    }

    fn insert<E>(&mut self, k: u32, mut left_of: impl FnMut(u32) -> Result<bool, E>) -> Result<(), E> {
        let (mut up, mut s, mut at) = (NIL, 0, self.root);
        while at != NIL {
            s = usize::from(!left_of(at)?);
            up = at;
            at = self.nodes[at as usize].kid[s];
        }
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        let side = match (up, s) {
            (NIL, _) => [NIL; 2],
            (_, 0) => [self.nodes[up as usize].side[0], up],
            _ => [up, self.nodes[up as usize].side[1]],
        };
        self.nodes[k as usize] = Node { kid: [NIL; 2], up, side, prio: self.seed };
        if side[0] != NIL {
            self.nodes[side[0] as usize].side[1] = k;
        }
        if side[1] != NIL {
            self.nodes[side[1] as usize].side[0] = k;
        }
        match up {
            NIL => self.root = k,
            _ => self.nodes[up as usize].kid[s] = k,
        }
        while let up = self.nodes[k as usize].up
            && up != NIL
            && self.nodes[up as usize].prio < self.nodes[k as usize].prio
        {
            self.rotate(k);
        }
        Ok(())
    }

    fn remove(&mut self, k: u32) {
        loop {
            let [l, r] = self.nodes[k as usize].kid;
            let child = match (l, r) {
                (NIL, NIL) => break,
                (_, NIL) => l,
                (NIL, _) => r,
                _ if self.nodes[l as usize].prio > self.nodes[r as usize].prio => l,
                _ => r,
            };
            self.rotate(child);
        }
        match self.nodes[k as usize].up {
            NIL => self.root = NIL,
            up => {
                let s = usize::from(self.nodes[up as usize].kid[1] == k);
                self.nodes[up as usize].kid[s] = NIL;
            }
        }
        let [prev, next] = self.nodes[k as usize].side;
        if prev != NIL {
            self.nodes[prev as usize].side[1] = next;
        }
        if next != NIL {
            self.nodes[next as usize].side[0] = prev;
        }
    }

    // `x` raised over its parent, the order kept.
    fn rotate(&mut self, x: u32) {
        let p = self.nodes[x as usize].up;
        let g = self.nodes[p as usize].up;
        let s = usize::from(self.nodes[p as usize].kid[1] == x);
        let c = self.nodes[x as usize].kid[1 - s];
        self.nodes[p as usize].kid[s] = c;
        if c != NIL {
            self.nodes[c as usize].up = p;
        }
        self.nodes[x as usize].kid[1 - s] = p;
        self.nodes[p as usize].up = x;
        self.nodes[x as usize].up = g;
        match g {
            NIL => self.root = x,
            _ => {
                let gs = usize::from(self.nodes[g as usize].kid[1] == p);
                self.nodes[g as usize].kid[gs] = x;
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/outline/simplify.rs"]
mod tests;
