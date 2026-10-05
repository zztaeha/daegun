use alloc::vec::Vec;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daemachine::float::FloatExt;

pub type Pt = (f32, f32);

type Edge = (Pt, Pt);

const EPS: f32 = 1e-4;

const PROBE: f32 = EPS * 10.0;

// A stroke's pieces overlap, which a non-zero fill hides and even-odd turns into holes; this resolves
// them so both rules agree. Contours too large to resolve, or whose union fails `union_from`'s checks,
// come back as they are.
pub fn union(contours: &[Vec<Pt>]) -> Vec<Vec<Pt>> {
    if contours.iter().map(Vec::len).sum::<usize>() > MAX_UNION_EDGES {
        return contours.to_vec();
    }
    let mut budget = Budget(MAX_VISITS);
    boundary_of(Arrangement::new(edges_of(contours)), &mut budget)
        .and_then(|(before, kept, _)| union_from(before, &kept, &mut budget))
        .unwrap_or_else(|| contours.to_vec())
}

// The edge count past which `resolve_overlaps` declines. Every pair of edges is tested for a crossing,
// and a font is untrusted input.
pub const MAX_RESOLVE_EDGES: usize = 512;

// The pieces crossings may split the edges into, which bounds the memory a resolve takes.
const MAX_SPLIT_EDGES: usize = 1 << 17;

// A stroke is not capped by `MAX_RESOLVE_EDGES`; the largest measured, STIX at 512 px, had 5,412.
const MAX_UNION_EDGES: usize = 1 << 14;

// The edges one resolve may visit in pair tests, inside tests and lookups, which bounds its time
// whatever the arrangement's shape. Past it the contours come back as they are.
const MAX_VISITS: usize = 1 << 27;

struct Budget(usize);

impl Budget {
    fn spend(&mut self, n: usize) -> Option<()> {
        self.0 = self.0.checked_sub(n)?;
        Some(())
    }
}

// One counterclockwise boundary, whatever the input's winding, in place of contours that overlap. `None`
// means keep the input; `EPS` is absolute, so it suits font units of an em up to about 2048.
pub fn resolve_overlaps(contours: &[Vec<Pt>]) -> Option<Vec<Vec<Pt>>> {
    if contours.iter().map(Vec::len).sum::<usize>() > MAX_RESOLVE_EDGES {
        return None;
    }
    union_if_overlapping(contours)
}

// The union of contours that overlap, adopted only when it closes, keeps its box and agrees with the
// input on which side of every edge is filled, which catches a spurious hole.
pub fn union_if_overlapping(contours: &[Vec<Pt>]) -> Option<Vec<Vec<Pt>>> {
    if contours.is_empty() {
        return None;
    }
    let mut budget = Budget(MAX_VISITS);
    let (before, kept, overlapped) = boundary_of(Arrangement::new(edges_of(contours)), &mut budget)?;
    if !overlapped {
        return None;
    }
    union_from(before, &kept, &mut budget)
}

fn union_from(before: Arrangement, kept: &[Edge], budget: &mut Budget) -> Option<Vec<Vec<Pt>>> {
    let (out, closed) = chain(kept, budget)?;
    if !closed || out.is_empty() {
        return None;
    }

    let mut after: Vec<Edge> = Vec::with_capacity(out.iter().map(Vec::len).sum());
    for c in &out {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if !same(a, b) {
                after.push((a, b));
            }
        }
    }

    let after = Arrangement::new(after);
    if !same_bounds(before.edges(), after.edges()) {
        return None;
    }
    // Both sides of every edge, in both arrangements. An edge on the boundary has one side filled
    // in each; a seam left inside the union has both sides filled in each. Either way they agree.
    for list in [before.edges(), after.edges()] {
        for &(a, b) in list {
            let mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
            let n = normal(a, b);
            for side in [PROBE, -PROBE] {
                let p = (mid.0 + n.0 * side, mid.1 + n.1 * side);
                if before.covered(p, budget)? != after.covered(p, budget)? {
                    return None;
                }
            }
        }
    }
    Some(out)
}

fn edges_of(contours: &[Vec<Pt>]) -> Vec<Edge> {
    let mut edges: Vec<Edge> = Vec::with_capacity(contours.iter().map(Vec::len).sum());
    for c in contours {
        for i in 0..c.len() {
            let (a, b) = (c[i], c[(i + 1) % c.len()]);
            if !same(a, b) {
                edges.push((a, b));
            }
        }
    }
    edges
}

// The pieces that bound the filled region, and whether some point is wound twice or more, the one case
// a coverage accumulator gets wrong. Every region borders a piece, so both sides of each find any.
fn boundary_of(before: Arrangement, budget: &mut Budget) -> Option<(Arrangement, Vec<Edge>, bool)> {
    if before.edges().len() < 3 {
        return None;
    }

    let split = split_at_crossings(&before, budget)?;
    let mut kept: Vec<Edge> = Vec::with_capacity(split.len());
    let mut overlapped = false;
    for &(a, b) in &split {
        let mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
        let n = normal(a, b);
        let left = before.winding((mid.0 + n.0 * PROBE, mid.1 + n.1 * PROBE), budget)?;
        let right = before.winding((mid.0 - n.0 * PROBE, mid.1 - n.1 * PROBE), budget)?;
        overlapped |= left.abs() >= 2 || right.abs() >= 2;
        match (left != 0, right != 0) {
            (true, false) => kept.push((a, b)),
            (false, true) => kept.push((b, a)),
            _ => {}
        }
    }
    let kept = dedup(kept, budget)?;
    Some((before, kept, overlapped))
}

// Contours sharing part of an edge leave it in `kept` twice: identical copies are one boundary, so one
// survives; a back to back pair faces away from each other and bounds nothing, so neither does. Either
// starts near one of the edge's ends, so only those are visited, in the order a full scan meets them.
fn dedup(kept: Vec<Edge>, budget: &mut Budget) -> Option<Vec<Edge>> {
    let starts = Starts::new(&kept);
    let mut drop = alloc::vec![false; kept.len()];
    let mut later: Vec<u32> = Vec::new();
    for i in 0..kept.len() {
        if drop[i] {
            continue;
        }
        let (a, b) = kept[i];
        later.clear();
        later.extend(starts.near(a).chain(starts.near(b)).filter(|&j| j as usize > i));
        budget.spend(later.len())?;
        later.sort_unstable();
        later.dedup();
        for &j in &later {
            let j = j as usize;
            if drop[j] {
                continue;
            }
            let (c, d) = kept[j];
            if same(a, c) && same(b, d) {
                drop[j] = true;
            } else if same(a, d) && same(b, c) {
                drop[i] = true;
                drop[j] = true;
                break;
            }
        }
    }
    Some(kept.into_iter().zip(drop).filter(|&(_, d)| !d).map(|(e, _)| e).collect())
}

// Edges by the EPS-wide cell their start lies in. Points `same` takes for one are at most a cell
// apart on each axis, so a point's matches lie in the nine cells about it.
struct Starts(Vec<(i64, i64, u32)>);

impl Starts {
    fn new(edges: &[Edge]) -> Starts {
        let mut keys: Vec<(i64, i64, u32)> = edges
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let (x, y) = cell(e.0);
                (x, y, i as u32)
            })
            .collect();
        keys.sort_unstable();
        Starts(keys)
    }

    fn near(&self, p: Pt) -> impl Iterator<Item = u32> + '_ {
        let (x, y) = cell(p);
        let (y0, y1) = (y.saturating_sub(1), y.saturating_add(1));
        (x.saturating_sub(1)..=x.saturating_add(1)).flat_map(move |cx| {
            let lo = self.0.partition_point(|k| (k.0, k.1) < (cx, y0));
            let hi = self.0.partition_point(|k| (k.0, k.1) <= (cx, y1));
            self.0[lo..hi].iter().map(|k| k.2)
        })
    }
}

fn cell(p: Pt) -> (i64, i64) {
    let w = f64::from(EPS);
    ((f64::from(p.0) / w).floor() as i64, (f64::from(p.1) / w).floor() as i64)
}

fn same_bounds(a: &[Edge], b: &[Edge]) -> bool {
    let box_of = |e: &[Edge]| {
        let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
        for &(p, q) in e {
            for v in [p, q] {
                r[0] = r[0].min(v.0);
                r[1] = r[1].min(v.1);
                r[2] = r[2].max(v.0);
                r[3] = r[3].max(v.1);
            }
        }
        r
    };
    let (x, y) = (box_of(a), box_of(b));
    x.iter().zip(&y).all(|(p, q)| (p - q).abs() < EPS * 10.0)
}

fn split_at_crossings(before: &Arrangement, budget: &mut Budget) -> Option<Vec<Edge>> {
    let edges = before.edges();
    // Boxes first: almost no pair of edges is near the other. The margin is the tolerance the tests
    // below use.
    let boxes: Vec<[f32; 4]> = edges
        .iter()
        .map(|&(a, b)| {
            [
                a.0.min(b.0) - EPS,
                a.1.min(b.1) - EPS,
                a.0.max(b.0) + EPS,
                a.1.max(b.1) + EPS,
            ]
        })
        .collect();

    let lens: Vec<f64> = edges
        .iter()
        .map(|&(a, b)| {
            let (dx, dy) = (f64::from(b.0) - f64::from(a.0), f64::from(b.1) - f64::from(a.1));
            (dx * dx + dy * dy).sqrt()
        })
        .collect();

    // Swept by left edge, so a pair whose boxes are apart in x is never visited.
    let mut order: Vec<u32> = (0..edges.len() as u32).collect();
    order.sort_unstable_by(|&i, &j| boxes[i as usize][0].total_cmp(&boxes[j as usize][0]));

    // Each cut is found once per pair and given to both edges as the same point, so their pieces meet
    // exactly; computed per edge, the two can land apart and break the chain.
    let mut cuts: Vec<(u32, f32, Pt)> = Vec::new();
    for (n, &i) in order.iter().enumerate() {
        let (i, p) = (i as usize, boxes[i as usize]);
        let (a, b) = edges[i];
        let mut visited = 0;
        for &j in &order[n + 1..] {
            let (j, q) = (j as usize, boxes[j as usize]);
            if q[0] > p[2] {
                break;
            }
            visited += 1;
            if p[3] < q[1] || q[3] < p[1] {
                continue;
            }
            let (c, d) = edges[j];
            if beside((a, b), lens[i], (c, d)) || beside((c, d), lens[j], (a, b)) {
                continue;
            }
            for (k, (s, e), ends) in [(i, (a, b), [c, d]), (j, (c, d), [a, b])] {
                for v in ends {
                    if let Some(t) = cut_at(s, e, v) {
                        cuts.push((k as u32, t, v));
                    }
                }
            }
            if let Some((t, u, x)) = crossing(a, b, c, d) {
                cuts.push((i as u32, t, x));
                cuts.push((j as u32, u, x));
            }
        }
        budget.spend(visited)?;
        if edges.len() + cuts.len() > MAX_SPLIT_EDGES {
            return None;
        }
    }
    cuts.sort_unstable_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));

    let mut out = Vec::with_capacity(edges.len() + cuts.len());
    let mut cuts = cuts.iter().peekable();
    for (i, &(a, b)) in edges.iter().enumerate() {
        let mut from = a;
        while let Some(&(_, _, x)) = cuts.next_if(|c| c.0 as usize == i) {
            if !near(from, x) {
                out.push((from, x));
                from = x;
            }
        }
        if !same(from, b) {
            out.push((from, b));
        }
    }
    Some(out)
}

// Whether `other` lies wholly to one side of the line through `edge`, further than EPS: then neither
// crosses the other or ends on it, which settles most pairs whose boxes meet without a divide.
fn beside(edge: Edge, len: f64, other: Edge) -> bool {
    let (s, e) = (wide(edge.0), wide(edge.1));
    let r = (e.0 - s.0, e.1 - s.1);
    let off = |p: Pt| {
        let p = wide(p);
        r.0 * (p.1 - s.1) - r.1 * (p.0 - s.0)
    };
    let (m, x, y) = (f64::from(EPS) * len, off(other.0), off(other.1));
    x > m && y > m || x < -m && y < -m
}

// Where `v` lies on the edge `s` to `e` short of both ends, the parameter that orders it along the edge.
fn cut_at(s: Pt, e: Pt, v: Pt) -> Option<f32> {
    if near(v, s) || near(v, e) {
        return None;
    }
    let (s64, e64, v64) = (wide(s), wide(e), wide(v));
    let r = (e64.0 - s64.0, e64.1 - s64.1);
    let len2 = r.0 * r.0 + r.1 * r.1;
    let t = ((v64.0 - s64.0) * r.0 + (v64.1 - s64.1) * r.1) / len2;
    if !(0.0..=1.0).contains(&t) {
        return None;
    }
    near(((s64.0 + r.0 * t) as f32, (s64.1 + r.1 * t) as f32), v).then_some(t as f32)
}

// Where two edges cross away from all four ends, in f64: in f32 the products lose the point.
fn crossing(a: Pt, b: Pt, c: Pt, d: Pt) -> Option<(f32, f32, Pt)> {
    let (a64, b64, c64, d64) = (wide(a), wide(b), wide(c), wide(d));
    let (r, s) = ((b64.0 - a64.0, b64.1 - a64.1), (d64.0 - c64.0, d64.1 - c64.1));
    let denom = r.0 * s.1 - r.1 * s.0;
    if denom == 0.0 {
        return None;
    }
    let (dx, dy) = (c64.0 - a64.0, c64.1 - a64.1);
    let (t, u) = ((dx * s.1 - dy * s.0) / denom, (dx * r.1 - dy * r.0) / denom);
    if !(0.0..=1.0).contains(&t) || !(0.0..=1.0).contains(&u) {
        return None;
    }
    let x = ((a64.0 + r.0 * t) as f32, (a64.1 + r.1 * t) as f32);
    (![a, b, c, d].iter().any(|&v| near(x, v))).then_some((t as f32, u as f32, x))
}

fn wide(p: Pt) -> (f64, f64) {
    (f64::from(p.0), f64::from(p.1))
}

// Edges bucketed by y: the inside test is a ray cast asked twice per edge, which scanning every edge
// would make quadratic. A bucket holds the edges whose y range reaches it, a handful for a glyph.
struct Arrangement {
    edges: Vec<Edge>,
    // One flat list with an offset per bucket rather than a vector each: the buckets live for one
    // glyph, and a hundred small allocations would cost more than the scan they shorten.
    index: Vec<u32>,
    start: Vec<u32>,
    ymin:  f32,
    inv:   f32,
}

impl Arrangement {
    fn new(edges: Vec<Edge>) -> Arrangement {
        let (mut ymin, mut ymax) = (f32::MAX, f32::MIN);
        for &(a, b) in &edges {
            ymin = ymin.min(a.1).min(b.1);
            ymax = ymax.max(a.1).max(b.1);
        }
        let n = edges.len().clamp(1, 256);
        let span = ymax - ymin;
        let inv = if span > 1e-9 { n as f32 / span } else { 0.0 };

        let range = |a: Pt, b: Pt| {
            let last = n as isize - 1;
            let lo = (((a.1.min(b.1) - ymin) * inv) as isize).clamp(0, last);
            let hi = (((a.1.max(b.1) - ymin) * inv) as isize).clamp(0, last);
            (lo as usize, hi as usize)
        };

        let mut start = alloc::vec![0u32; n + 1];
        for &(a, b) in &edges {
            let (lo, hi) = range(a, b);
            for slot in &mut start[lo + 1..=hi + 1] {
                *slot += 1;
            }
        }
        for k in 0..n {
            start[k + 1] += start[k];
        }
        let mut fill = start.clone();
        let mut index = alloc::vec![0u32; start[n] as usize];
        for (i, &(a, b)) in edges.iter().enumerate() {
            let (lo, hi) = range(a, b);
            for k in lo..=hi {
                index[fill[k] as usize] = i as u32;
                fill[k] += 1;
            }
        }
        Arrangement { edges, index, start, ymin, inv }
    }

    fn edges(&self) -> &[Edge] {
        &self.edges
    }

    fn winding(&self, p: Pt, budget: &mut Budget) -> Option<i32> {
        let k = (((p.1 - self.ymin) * self.inv) as isize).clamp(0, self.start.len() as isize - 2);
        let (k, mut w) = (k as usize, 0i32);
        let bucket = &self.index[self.start[k] as usize..self.start[k + 1] as usize];
        budget.spend(bucket.len())?;
        // The crossing test multiplied through by the y span, which takes a divide out of the
        // innermost loop here and flips the comparison when that span is negative.
        for &i in bucket {
            let (a, b) = self.edges[i as usize];
            if (a.1 <= p.1) != (b.1 <= p.1) {
                let dy = b.1 - a.1;
                let lhs = (p.1 - a.1) * (b.0 - a.0);
                let rhs = (p.0 - a.0) * dy;
                if if dy > 0.0 { lhs > rhs } else { lhs < rhs } {
                    w += if dy > 0.0 { 1 } else { -1 };
                }
            }
        }
        Some(w)
    }

    fn covered(&self, p: Pt, budget: &mut Budget) -> Option<bool> {
        Some(self.winding(p, budget)? != 0)
    }
}

// The kept edges walked into closed contours. The flag says whether all were consumed into contours
// that closed, which separates an arrangement that resolved from one that ran out partway round.
fn chain(edges: &[Edge], budget: &mut Budget) -> Option<(Vec<Vec<Pt>>, bool)> {
    let starts = Starts::new(edges);
    let mut used = alloc::vec![false; edges.len()];
    let mut out: Vec<Vec<Pt>> = Vec::new();
    let mut closed = true;

    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        // Grown, not reserved for every edge left: the contours are handed back, capacity and all.
        let mut contour = alloc::vec![edges[start].0];
        let mut at = edges[start].1;
        let first = edges[start].0;

        for _ in 0..edges.len() {
            if same(at, first) {
                break;
            }
            contour.push(at);
            // The lowest-numbered unused edge that starts here.
            let mut next: Option<usize> = None;
            for k in starts.near(at) {
                budget.spend(1)?;
                let k = k as usize;
                if !used[k] && same(edges[k].0, at) && next.is_none_or(|n| k < n) {
                    next = Some(k);
                }
            }
            match next {
                Some(next) => {
                    used[next] = true;
                    at = edges[next].1;
                }
                None => {
                    closed = false;
                    break;
                }
            }
        }
        if !same(at, first) {
            closed = false;
        }
        if contour.len() >= 3 {
            out.push(contour);
        } else {
            closed = false;
        }
    }
    Some((out, closed && used.iter().all(|&u| u)))
}

fn same(a: Pt, b: Pt) -> bool {
    (a.0 - b.0).abs() < EPS && (a.1 - b.1).abs() < EPS
}

// Within EPS by distance. It implies `same`, so two cuts merged on one edge still chain.
fn near(a: Pt, b: Pt) -> bool {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    dx * dx + dy * dy < EPS * EPS
}

fn normal(a: Pt, b: Pt) -> Pt {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len > 1e-9 { (-dy / len, dx / len) } else { (0.0, 1.0) }
}
