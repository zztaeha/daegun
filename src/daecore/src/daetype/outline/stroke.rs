use alloc::vec::Vec;
use core::f32::consts::PI;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daemachine::float::FloatExt;
use crate::daecore::daemachine::float::{atan2, sin_cos};

use super::flatten::MAX_FLATTEN_POINTS;
use super::{OutlinePen, Path, Verb};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Join {
    Miter { limit: f32 },
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug)]
pub struct StrokeStyle {
    pub width: f32,
    pub join: Join,
    pub cap: Cap,
}

impl Default for StrokeStyle {
    fn default() -> StrokeStyle {
        StrokeStyle { width: 1.0, join: Join::Miter { limit: 4.0 }, cap: Cap::Butt }
    }
}

type Pt = (f32, f32);

const UNION_REACH: f32 = 4096.0;

// `tolerance` is how far the flattened curves may sit from the true ones, in the path's units; one
// not above zero, or NaN, means 0.1. A curve, round join or cap takes at most 256 segments.
pub fn stroke(path: &Path, style: &StrokeStyle, tolerance: f32, pen: &mut dyn OutlinePen) {
    if let Some(s) = Stroked::new(path, style, tolerance)
        && s.finite()
    {
        s.draw(pen);
    }
}

pub fn stroke_simplified(path: &Path, style: &StrokeStyle, tolerance: f32, pen: &mut dyn OutlinePen) {
    if let Some(s) = Stroked::new(path, style, tolerance)
        && s.finite()
    {
        s.draw_simplified(pen);
    }
}

// A stroke built whole before any of it reaches a pen. A join can add 256 points to each flattened
// one, so a hostile outline is refused past MAX_FLATTEN_POINTS rather than drawn in part.
pub(crate) struct Stroked(Vec<Vec<Pt>>);

impl Stroked {
    pub(crate) fn new(path: &Path, style: &StrokeStyle, tolerance: f32) -> Option<Stroked> {
        Stroked::build(path, style, tolerance).map(|(s, _)| s)
    }

    // A ring winds the same way whichever way its contour runs, so an embolden's is turned to wind as
    // the fill does; a non-zero fill cancels where they disagree, and CFF winds against TrueType.
    pub(crate) fn embolden(path: &Path, style: &StrokeStyle, tolerance: f32) -> Option<Stroked> {
        let (mut s, fill) = Stroked::build(path, style, tolerance)?;
        if fill * s.0.iter().map(|l| area(l)).sum::<f64>() < 0.0 {
            s.0.iter_mut().for_each(|l| l.reverse());
        }
        Some(s)
    }

    fn build(path: &Path, style: &StrokeStyle, tolerance: f32) -> Option<(Stroked, f64)> {
        let r = style.width.abs() * 0.5;
        if r <= 0.0 || !r.is_finite() {
            return Some((Stroked(Vec::new()), 0.0));
        }
        let tolerance = if tolerance > 0.0 { tolerance } else { 0.1 };
        loops_of(path, r, style, tolerance).map(|(loops, fill)| (Stroked(loops), fill))
    }

    pub(crate) fn points(&self) -> impl Iterator<Item = &Pt> {
        self.0.iter().flatten()
    }

    // A miter's tip, at the radius over the cosine of half the turn, overflows long before the width.
    pub(crate) fn finite(&self) -> bool {
        self.points().all(|p| p.0.is_finite() && p.1.is_finite())
    }

    pub(crate) fn draw(&self, pen: &mut dyn OutlinePen) {
        for l in &self.0 {
            emit_loop(l, pen);
        }
    }

    // `union`'s tolerances are absolute and suit font units, so a stroke is resolved scaled by a power
    // of two, which f32 undoes exactly, to reach between half UNION_REACH and UNION_REACH.
    pub(crate) fn draw_simplified(&self, pen: &mut dyn OutlinePen) {
        let mut collect = Collect::default();
        self.draw(&mut collect);
        let reach = self.points().fold(0.0f32, |m, p| m.max(p.0.abs()).max(p.1.abs()));
        let mut k = 1.0f32;
        while reach > UNION_REACH * k {
            k *= 2.0;
        }
        while reach > 0.0 && reach <= UNION_REACH * k * 0.5 && k > f32::MIN_POSITIVE {
            k *= 0.5;
        }
        collect.contours.iter_mut().flatten().for_each(|p| *p = (p.0 / k, p.1 / k));
        for c in super::simplify::union(&collect.contours) {
            pen.move_to(c[0].0 * k, c[0].1 * k);
            for p in &c[1..] {
                pen.line_to(p.0 * k, p.1 * k);
            }
            pen.close();
        }
    }
}

#[derive(Default)]
struct Collect {
    contours: Vec<Vec<Pt>>,
}

impl OutlinePen for Collect {
    fn move_to(&mut self, x: f32, y: f32) {
        self.contours.push(alloc::vec![(x, y)]);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        if let Some(c) = self.contours.last_mut() {
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

// The loops, and the signed area the closed contours enclose.
fn loops_of(path: &Path, r: f32, style: &StrokeStyle, tolerance: f32) -> Option<(Vec<Vec<Pt>>, f64)> {
    let mut loops: Vec<Vec<Pt>> = Vec::new();
    let mut room = MAX_FLATTEN_POINTS;
    let mut fill = 0.0;
    for (pts, closed) in flatten(path, tolerance)? {
        let pts = dedup(&pts, closed);
        if pts.len() == 1 && !closed {
            keep(&mut loops, &mut room, dot(pts[0], r, style, tolerance))?;
        } else if pts.len() >= 2 && closed {
            fill += area(&pts);
            let n = pts.len();
            let normals: Vec<Pt> = (0..n).map(|i| normal(pts[i], pts[(i + 1) % n])).collect();
            let front = side(&pts, &normals, false, r, style, tolerance, room)?;
            keep(&mut loops, &mut room, front)?;
            let back = side(&pts, &normals, true, r, style, tolerance, room)?;
            keep(&mut loops, &mut room, back)?;
        } else if pts.len() >= 2 {
            let both = open(&pts, r, style, tolerance, room)?;
            keep(&mut loops, &mut room, both)?;
        }
    }
    Some((loops, fill))
}

fn area(pts: &[Pt]) -> f64 {
    let mut sum = 0.0;
    for (i, &(x0, y0)) in pts.iter().enumerate() {
        let (x1, y1) = pts[(i + 1) % pts.len()];
        sum += f64::from(x0) * f64::from(y1) - f64::from(x1) * f64::from(y0);
    }
    sum
}

fn keep(loops: &mut Vec<Vec<Pt>>, room: &mut usize, l: Vec<Pt>) -> Option<()> {
    *room = room.checked_sub(l.len())?;
    loops.push(l);
    Some(())
}

// One side of a closed contour, walked forward or back, `normals[i]` running from point i to the next.
// A segment walked back has the negated normal, which `normal` gives exactly.
#[allow(clippy::too_many_arguments, reason = "a side is its contour, its direction and the stroke's style")]
fn side(pts: &[Pt], normals: &[Pt], back: bool, r: f32, style: &StrokeStyle, tolerance: f32, room: usize) -> Option<Vec<Pt>> {
    let n = pts.len();
    let at = |i: usize| if back { pts[n - 1 - i] } else { pts[i] };
    let segment = |i: usize| if back { neg(normals[(2 * n - 2 - i) % n]) } else { normals[i] };
    let mut out: Vec<Pt> = Vec::with_capacity(n * 2);
    for i in 0..n {
        let prev = at((i + n - 1) % n);
        let cur = at(i);
        let next = at((i + 1) % n);
        let (n_in, n_out) = (segment((i + n - 1) % n), segment(i));
        push_join(&mut out, mid(prev, cur), cur, mid(cur, next), n_in, n_out, r, style, tolerance);
        if out.len() > room {
            return None;
        }
    }
    Some(out)
}

fn open(pts: &[Pt], r: f32, style: &StrokeStyle, tolerance: f32, room: usize) -> Option<Vec<Pt>> {
    let n = pts.len();
    let mut out: Vec<Pt> = Vec::with_capacity(n * 4);
    let normals: Vec<Pt> = (0..n - 1).map(|i| normal(pts[i], pts[i + 1])).collect();

    // How far an inner corner may trim each segment: all of one that ends in a cap, half of one
    // shared with another corner.
    let reach = |from: usize, to: usize| if from == 0 || from == n - 1 { pts[from] } else { mid(pts[from], pts[to]) };
    out.push(offset(pts[0], normals[0], r));
    for i in 1..n - 1 {
        let (prev, next) = (reach(i - 1, i), reach(i + 1, i));
        push_join(&mut out, prev, pts[i], next, normals[i - 1], normals[i], r, style, tolerance);
        if out.len() > room {
            return None;
        }
    }
    let end_n = normals[n - 2];
    out.push(offset(pts[n - 1], end_n, r));
    push_cap(&mut out, pts[n - 1], end_n, direction(pts[n - 2], pts[n - 1]), r, style, tolerance);

    if style.cap != Cap::Round {
        out.push(offset(pts[n - 1], neg(end_n), r));
    }
    for i in (1..n - 1).rev() {
        let (prev, next) = (reach(i + 1, i), reach(i - 1, i));
        push_join(&mut out, prev, pts[i], next, neg(normals[i]), neg(normals[i - 1]), r, style, tolerance);
        if out.len() > room {
            return None;
        }
    }
    let start_n = normals[0];
    out.push(offset(pts[0], neg(start_n), r));
    push_cap(&mut out, pts[0], neg(start_n), direction(pts[1], pts[0]), r, style, tolerance);
    (out.len() <= room).then_some(out)
}

// `prev` and `next` bound how far back along each segment an inner corner may be trimmed.
#[allow(clippy::too_many_arguments, reason = "a corner is its two neighbors as well as itself")]
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
    // The offsets meet r * tan(half the turn) back along each segment (from the normals: intersecting loses
    // a slight turn). Past that they have crossed, so the side runs through the corner, as Skia's does.
    if turn > 0.0 {
        let trim = r * turn / (1.0 + cos_turn);
        if trim <= dist(prev, at) && trim <= dist(at, next) {
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
fn dot(at: Pt, r: f32, style: &StrokeStyle, tolerance: f32) -> Vec<Pt> {
    let mut out = Vec::new();
    match style.cap {
        Cap::Butt => {}
        Cap::Round => {
            let q = [(r, 0.0), (0.0, -r), (-r, 0.0), (0.0, r)];
            for k in 0..4 {
                let from = (at.0 + q[k].0, at.1 + q[k].1);
                let to = (at.0 + q[(k + 1) % 4].0, at.1 + q[(k + 1) % 4].1);
                arc(&mut out, at, from, to, r, tolerance);
            }
        }
        Cap::Square => out.extend_from_slice(&[
            (at.0 - r, at.1 - r),
            (at.0 - r, at.1 + r),
            (at.0 + r, at.1 + r),
            (at.0 + r, at.1 - r),
        ]),
    }
    out
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

// The loop as `dedup` leaves it, emitted as it is walked rather than copied first.
fn emit_loop(pts: &[Pt], pen: &mut dyn OutlinePen) {
    let kept = || pts.iter().copied().scan(None, |last: &mut Option<Pt>, p| {
        let keep = last.is_none_or(|q| distinct(p, q));
        if keep { *last = Some(p); }
        Some(keep.then_some(p))
    }).flatten();
    let (mut count, mut first, mut end) = (0usize, None, None);
    for p in kept() {
        count += 1;
        first.get_or_insert(p);
        end = Some(p);
    }
    if let (Some(f), Some(l)) = (first, end) && count > 1 && repeats(f, l) {
        count -= 1;
    }
    if count < 3 {
        return;
    }
    for (k, p) in kept().take(count).enumerate() {
        if k == 0 { pen.move_to(p.0, p.1) } else { pen.line_to(p.0, p.1) }
    }
    pen.close();
}

fn distinct(p: Pt, q: Pt) -> bool {
    (p.0 - q.0).abs() > 1e-6 || (p.1 - q.1).abs() > 1e-6
}

fn repeats(p: Pt, q: Pt) -> bool {
    (p.0 - q.0).abs() <= 1e-6 && (p.1 - q.1).abs() <= 1e-6
}

// Repeated points dropped, and on a closed contour a last point that repeats the first.
fn dedup(pts: &[Pt], closed: bool) -> Vec<Pt> {
    let mut out: Vec<Pt> = Vec::with_capacity(pts.len());
    for &p in pts {
        if out.last().is_none_or(|&q| distinct(p, q)) {
            out.push(p);
        }
    }
    if closed
        && out.len() > 1
        && let (Some(&f), Some(&l)) = (out.first(), out.last())
        && repeats(f, l)
    {
        out.pop();
    }
    out
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

fn normal(a: Pt, b: Pt) -> Pt {
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

fn flatten(path: &Path, tolerance: f32) -> Option<Vec<(Vec<Pt>, bool)>> {
    let (verbs, points) = path.parts();
    let mut out: Vec<(Vec<Pt>, bool)> = Vec::new();
    let mut cur: Vec<Pt> = Vec::new();
    let mut at: Pt = (0.0, 0.0);
    let mut i = 0usize;
    let mut kept = 0usize;

    for &v in verbs {
        if kept + cur.len() > MAX_FLATTEN_POINTS {
            return None;
        }
        // A contour drawn without a move starts where the last one ended, as `flatten` reads it.
        if matches!(v, Verb::Line | Verb::Quad | Verb::Cubic) && cur.is_empty() {
            cur.push(at);
        }
        match v {
            Verb::Move => {
                if !cur.is_empty() {
                    kept += cur.len();
                    out.push((core::mem::take(&mut cur), false));
                }
                at = points[i];
                cur.push(at);
                i += 1;
            }
            Verb::Line => {
                at = points[i];
                cur.push(at);
                i += 1;
            }
            Verb::Quad => {
                let (c, e) = (points[i], points[i + 1]);
                let n = steps(dist(at, c) + dist(c, e), tolerance);
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    let u = 1.0 - t;
                    cur.push((
                        u * u * at.0 + 2.0 * u * t * c.0 + t * t * e.0,
                        u * u * at.1 + 2.0 * u * t * c.1 + t * t * e.1,
                    ));
                }
                at = e;
                i += 2;
            }
            Verb::Cubic => {
                let (c1, c2, e) = (points[i], points[i + 1], points[i + 2]);
                let n = steps(dist(at, c1) + dist(c1, c2) + dist(c2, e), tolerance);
                for k in 1..=n {
                    let t = k as f32 / n as f32;
                    let u = 1.0 - t;
                    cur.push((
                        u * u * u * at.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * e.0,
                        u * u * u * at.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * e.1,
                    ));
                }
                at = e;
                i += 3;
            }
            Verb::Close => {
                if !cur.is_empty() {
                    at = cur[0];
                    kept += cur.len();
                    out.push((core::mem::take(&mut cur), true));
                }
            }
        }
    }
    if kept + cur.len() > MAX_FLATTEN_POINTS {
        return None;
    }
    if !cur.is_empty() {
        out.push((cur, false));
    }
    Some(out)
}

fn dist(a: Pt, b: Pt) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    (dx * dx + dy * dy).sqrt()
}

fn steps(len: f32, tolerance: f32) -> usize {
    ((len / tolerance.max(1e-4)).sqrt().ceil() as usize).clamp(1, 256)
}
