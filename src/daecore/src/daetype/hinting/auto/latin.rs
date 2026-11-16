use alloc::vec::Vec;

use super::blues::BlueZones;
use super::points::{AutoPoints, ON_CURVE};

use crate::daecore::daetype::hinting::f26dot6;

// A run of points along a contour; which points are its own, `Scratch::seg_of` holds.
#[derive(Clone, Copy, Debug)]
struct Segment {
    pos: f32,
    x_min: f32,
    x_max: f32,
    link: u32,
    dir: i8,
    round: bool,
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    pos: f32,
    // The position of the last segment it took, the highest, since segments come in rising order.
    reach: f32,
    count: u32,
    rounds: u32,
    link: u32,
    dir: i8,
    round: bool,
}

// A segment where it falls in rising position, with what linking and edges read of it.
#[derive(Clone, Copy, Debug)]
struct Placed {
    pos: f32,
    x_min: f32,
    x_max: f32,
    seg: u32,
    dir: i8,
    round: bool,
}

// A zone's flat edge and overshoot, and where they land at the size.
type Fitted = (f32, f32, (i32, i32));

// A segment links to the nearest facing one; past this many others in the way it looks no further,
// which only a crafted outline reaches and which bounds linking at segments times this.
const LINK_SCAN: usize = 256;

const NIL: u32 = u32::MAX;

// Lists of up to this many segments are placed by counting rather than sorted.
const RANKED: usize = 32;

#[derive(Default)]
pub struct Scratch {
    base: Vec<i32>,
    segments: Vec<Segment>,
    // Each point's segment, NIL for none: a point is in one at most.
    seg_of: Vec<u32>,
    singles: Vec<u32>,
    extrema: Vec<(u32, Segment)>,
    keys: Vec<u64>,
    placed: Vec<Placed>,
    chosen: Vec<u32>,
    owner: Vec<u32>,
    edges: Vec<Edge>,
    live: Vec<u32>,
    sided: [Vec<Fitted>; 2],
    scaled: Vec<i32>,
    targets: Vec<i32>,
    blue_matched: Vec<bool>,
    done: Vec<bool>,
    seg_delta: Vec<i32>,
    anchors: Vec<u32>,
}

impl Scratch {
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        let Scratch {
            base, segments, seg_of, singles, extrema, keys, placed, chosen, owner, edges, live, sided, scaled,
            targets, blue_matched, done, seg_delta, anchors,
        } = self;
        heap_of(base) + heap_of(segments) + heap_of(seg_of) + heap_of(singles) + heap_of(extrema) + heap_of(keys)
            + heap_of(placed) + heap_of(chosen) + heap_of(owner) + heap_of(edges) + heap_of(live) + heap_of(&sided[0])
            + heap_of(&sided[1]) + heap_of(scaled) + heap_of(targets) + heap_of(blue_matched) + heap_of(done)
            + heap_of(seg_delta) + heap_of(anchors)
    }
}

// The zones depend on the size alone, so they're fitted again only when it changes. The fitted points go to `y`.
pub fn fit(
    pts: &AutoPoints, blues: &mut BlueZones, fitted_at: &mut Option<core::num::NonZeroI32>, size: f26dot6::Size,
    upm: u16, s: &mut Scratch, y: &mut Vec<i32>,
) {
    let scale = |v: f32| f26dot6::scale_f32(v, size, upm);

    let n = pts.len();
    s.base.clear();
    f26dot6::scale_f32_into(&pts.y[..n], size, upm, &mut s.base);
    y.clear();
    y.extend_from_slice(&s.base);
    if n == 0 {
        return;
    }

    let seg_tolerance = upm as f32 / 100.0;
    let edge_tolerance = upm as f32 / 50.0;
    // A level run past upm / 14, about a stem's width, is a flat stroke end rather than a curve's tangent.
    let flat_threshold = f32::from(upm / 14);

    compute_segments(pts, seg_tolerance, flat_threshold, s);
    if s.segments.is_empty() {
        return;
    }
    link_segments(upm as f32 / 3.0, s);
    compute_edges(edge_tolerance, s);

    let at = core::num::NonZeroI32::new(size.units());
    if *fitted_at != at {
        fit_zones(blues, &scale);
        *fitted_at = at;
    }
    // An edge snaps to the nearest zone within upm / 40, an overshoot's depth well short of the next zone, and half a
    // pixel at most, no more than rounding moves it: to the flat edge, or for a round edge past it the overshoot.
    let reach = scale(f32::from(upm / 40)).min(f26dot6::HALF);
    let beyond = skipped_from(reach, size, upm);
    // The zones fitted at this size, by side, in their order: an edge rising faces the top ones.
    s.sided[0].clear();
    s.sided[1].clear();
    for z in &blues.zones {
        if let Some(fit) = z.fit {
            s.sided[usize::from(z.is_top)].push((z.flat, z.overshoot, fit));
        }
    }
    s.scaled.clear();
    s.targets.clear();
    s.blue_matched.clear();
    for edge in &s.edges {
        let scaled = scale(edge.pos);
        let mut best = (reach, None);
        let top = edge.dir > 0;
        for &(flat, overshoot, fit) in &s.sided[usize::from(top)] {
            let near = (edge.pos - flat).abs();
            let d = if f64::from(near) < beyond { scale(near) } else { i32::MAX };
            if d < best.0 {
                best = (d, Some(fit.0));
            }
            if edge.round && d != 0 && top != (edge.pos < flat) {
                let past = (edge.pos - overshoot).abs();
                if f64::from(past) < beyond {
                    let d = scale(past);
                    if d < best.0 {
                        best = (d, Some(fit.1));
                    }
                }
            }
        }
        s.scaled.push(scaled);
        s.targets.push(best.1.unwrap_or_else(|| f26dot6::round_to_grid(scaled)));
        s.blue_matched.push(best.1.is_some());
    }

    s.done.clear();
    s.done.resize(s.edges.len(), false);
    for i in 0..s.edges.len() {
        let j = s.edges[i].link;
        if j == NIL {
            continue;
        }
        let j = j as usize;
        if s.done[i] || s.done[j] {
            continue;
        }
        let anchor = if s.blue_matched[i] || !s.blue_matched[j] { i } else { j };
        let other = if anchor == i { j } else { i };
        let width = fit_stem_width((s.scaled[other] - s.scaled[anchor]).abs());
        let signed = if s.edges[other].pos > s.edges[anchor].pos { width } else { -width };
        s.targets[other] = s.targets[anchor] + signed;
        s.done[i] = true;
        s.done[j] = true;
    }

    // Each point moved with its segment's edge, in one pass; a point in none takes the last slot's 0.
    let m = s.segments.len();
    s.seg_delta.clear();
    s.seg_delta.extend(s.owner.iter().map(|&e| s.targets[e as usize] - s.scaled[e as usize]));
    s.seg_delta.push(0);
    for (v, &g) in y.iter_mut().zip(&s.seg_of) {
        *v += s.seg_delta[(g as usize).min(m)];
    }

    interpolate_untouched(pts, y, s);
}

// A distance this far or more scales to reach + 1 or more, so a zone there can't be the nearest: skipped unscaled.
fn skipped_from(reach: i32, size: f26dot6::Size, upm: u16) -> f64 {
    f64::from(reach + 1) * f64::from(upm) / f64::from(size.units())
}

fn side(dx: f32, convention: f32, fallback: i8) -> i8 {
    let d = dx * convention;
    if d > 0.0 {
        1
    } else if d < 0.0 {
        -1
    } else {
        fallback
    }
}

fn winding_convention(pts: &AutoPoints) -> f32 {
    let (x, y) = (&pts.x[..], &pts.y[..]);
    let mut widest = 0.0f32;
    let mut sign = -1.0f32;
    for c in 0..pts.contour_ends.len() {
        let Some((start, end)) = pts.contour(c) else { continue };
        let mut area = 0.0f32;
        for a in start..end - 1 {
            area += x[a] * y[a + 1] - x[a + 1] * y[a];
        }
        let (a, b) = (end - 1, start);
        area += x[a] * y[b] - x[b] * y[a];
        if area.abs() > widest {
            widest = area.abs();
            sign = if area >= 0.0 { 1.0 } else { -1.0 };
        }
    }
    -sign
}

// The zones active at a size, each with its flat edge rounded and its overshoot 0, 1/2 or 1 pixel past it. Past 3/4
// pixel (48) an overshoot is big enough to draw as designed, so its zone stops holding round edges to the flat.
fn fit_zones(blues: &mut BlueZones, scale: &impl Fn(f32) -> i32) {
    for z in &mut blues.zones {
        let dist = scale(z.flat - z.overshoot);
        z.fit = (dist.abs() <= 48).then(|| {
            let step = match dist.abs() {
                d if d < 32 => 0,
                d if d < 48 => 32,
                _ => 64,
            };
            let flat = f26dot6::round_to_grid(scale(z.flat));
            (flat, flat - step * dist.signum())
        });
    }
}

// Each contour walked once from its first break in y: a run of two or more points within the tolerance of its first
// is a segment, and a lone point that is a local extremum in y is one too, after every run, in point order.
fn compute_segments(pts: &AutoPoints, tolerance: f32, flat_threshold: f32, s: &mut Scratch) {
    let convention = winding_convention(pts);
    let (x, y, flags) = (&pts.x[..], &pts.y[..], &pts.flags[..]);
    let control = |i: usize| flags[i] & ON_CURVE == 0;
    s.segments.clear();
    s.extrema.clear();
    s.seg_of.clear();
    s.seg_of.resize(pts.len(), NIL);
    for c in 0..pts.contour_ends.len() {
        let Some((start, end)) = pts.contour(c) else { continue };
        let n = end - start;
        if n < 2 {
            continue;
        }
        let next = |i: usize| if i + 1 == end { start } else { i + 1 };
        let prev = |i: usize| if i == start { end - 1 } else { i - 1 };
        s.singles.clear();
        match (start..end).find(|&i| (y[i] - y[prev(i)]).abs() > tolerance) {
            None => s.singles.extend(start as u32..end as u32),
            Some(break_at) => {
                let (mut first, mut k) = (break_at, 0);
                while k < n {
                    let lead = y[first];
                    let after = next(first);
                    // Most runs are a lone point: kept for the extrema without a run's bookkeeping.
                    if k + 1 == n || (y[after] - lead).abs() > tolerance {
                        s.singles.push(first as u32);
                        (first, k) = (after, k + 1);
                        continue;
                    }
                    // Each fold starts from its identity, so a -0.0 or NaN lead folds as any other point.
                    let mut sum = 0.0f32 + lead;
                    let (mut x_min, mut x_max) = (f32::MAX.min(x[first]), f32::MIN.max(x[first]));
                    // By x, the first point of the least and the last of the greatest, and the on-curve span.
                    let ((mut lo, mut lo_x), (mut hi, mut hi_x)) = ((first, x[first]), (first, x[first]));
                    let (mut on_max, mut on_min) = (f32::MIN, f32::MAX);
                    let at = s.segments.len() as u32;
                    s.seg_of[first] = at;
                    if !control(first) {
                        (on_max, on_min) = (on_max.max(x[first]), on_min.min(x[first]));
                    }
                    let (mut len, mut last, mut q) = (1, first, after);
                    while len < n - k && (y[q] - lead).abs() <= tolerance {
                        s.seg_of[q] = at;
                        let xq = x[q];
                        sum += y[q];
                        (x_min, x_max) = (x_min.min(xq), x_max.max(xq));
                        if xq.total_cmp(&lo_x).is_lt() {
                            (lo, lo_x) = (q, xq);
                        }
                        if xq.total_cmp(&hi_x).is_ge() {
                            (hi, hi_x) = (q, xq);
                        }
                        if !control(q) {
                            (on_max, on_min) = (on_max.max(xq), on_min.min(xq));
                        }
                        (len, last, q) = (len + 1, q, next(q));
                    }
                    let pos = sum / len as f32;
                    let neighbors = (y[prev(first)] + y[q]) * 0.5;
                    s.segments.push(Segment {
                        pos,
                        x_min,
                        x_max,
                        link: NIL,
                        dir: side(x[last] - x[first], convention, if neighbors < pos { 1 } else { -1 }),
                        round: (control(lo) || control(hi)) && on_max - on_min < flat_threshold,
                    });
                    (first, k) = (q, k + len);
                }
                // Walked from the break round to it, so the points before it came last.
                let wrapped = s.singles.partition_point(|&i| i as usize >= break_at);
                s.singles.rotate_left(wrapped);
            }
        }
        if n < 3 {
            continue;
        }
        for &i in &s.singles {
            let i = i as usize;
            let (p, q) = (prev(i), next(i));
            let (yi, yp, yq) = (y[i], y[p], y[q]);
            if !(yi < yp && yi < yq) && !(yi > yp && yi > yq) {
                continue;
            }
            s.extrema.push((i as u32, Segment {
                pos: yi,
                x_min: x[i],
                x_max: x[i],
                link: NIL,
                dir: side(x[q] - x[p], convention, if (yp + yq) * 0.5 < yi { 1 } else { -1 }),
                round: control(i) || control(p) || control(q),
            }));
        }
    }
    for &(i, g) in &s.extrema {
        s.seg_of[i as usize] = s.segments.len() as u32;
        s.segments.push(g);
    }
}

// A position's place in `total_cmp`'s order, as an unsigned key.
fn ordered(v: f32) -> u32 {
    let bits = v.to_bits() as i32;
    (bits ^ (((bits >> 31) as u32) >> 1) as i32) as u32 ^ 0x8000_0000
}

// Each segment links to the nearest facing it across a stem (other direction, inward, overlapping in x, within
// `max_distance`, lower index on a tie). Out of line: inlined into `fit`, it took STIX's hinting from 5.83 to 5.95 ms.
#[inline(never)]
fn link_segments(max_distance: f32, s: &mut Scratch) {
    let m = s.segments.len();
    s.keys.clear();
    s.keys.extend(s.segments.iter().enumerate().map(|(i, g)| u64::from(ordered(g.pos)) << 32 | i as u64));
    let place = |g: &Segment, i: usize| {
        Placed { pos: g.pos, x_min: g.x_min, x_max: g.x_max, seg: i as u32, dir: g.dir, round: g.round }
    };
    s.placed.clear();
    if m <= RANKED {
        // Each key's place is how many keys are below it: no branch to mispredict, where a sort of so few has one
        // a step.
        s.placed.resize(m, Placed { pos: 0.0, x_min: 0.0, x_max: 0.0, seg: 0, dir: 0, round: false });
        for (i, &k) in s.keys.iter().enumerate() {
            let below = s.keys.iter().map(|&o| usize::from(o < k)).sum::<usize>();
            s.placed[below] = place(&s.segments[i], i);
        }
    } else {
        s.keys.sort_unstable();
        s.placed.extend(s.keys.iter().map(|&k| place(&s.segments[k as u32 as usize], k as u32 as usize)));
    }
    for r in 0..m {
        let a = s.placed[r];
        let link = match a.dir > 0 {
            true => nearest(&a, s.placed[r.saturating_sub(LINK_SCAN)..r].iter().rev(), max_distance),
            false => nearest(&a, s.placed[r + 1..m.min(r + 1 + LINK_SCAN)].iter(), max_distance),
        };
        s.segments[a.seg as usize].link = link;
    }
    s.chosen.clear();
    s.chosen.extend(s.segments.iter().map(|g| g.link));
    for (i, g) in s.segments.iter_mut().enumerate() {
        if g.link != NIL && s.chosen[g.link as usize] != i as u32 {
            g.link = NIL;
        }
    }
}

// The nearest segment of the other direction overlapping `a`, the lowest index of equals, among `candidates`,
// which run away from `a`'s position.
fn nearest<'a>(a: &Placed, candidates: impl Iterator<Item = &'a Placed>, max_distance: f32) -> u32 {
    let mut best: Option<(f32, u32)> = None;
    for b in candidates {
        let d = (a.pos - b.pos).abs();
        if d == 0.0 {
            continue;
        }
        if d > max_distance || best.is_some_and(|(bd, _)| d > bd) {
            break;
        }
        let overlap = a.x_max.min(b.x_max) - a.x_min.max(b.x_min);
        if b.dir != a.dir && overlap > 0.0 && best.is_none_or(|(bd, bj)| d < bd || b.seg < bj) {
            best = Some((d, b.seg));
        }
    }
    best.map_or(NIL, |(_, j)| j)
}

fn fit_stem_width(scaled: i32) -> i32 {
    f26dot6::round_to_grid(scaled).max(f26dot6::ONE)
}

// Segments in rising position, as `link_segments` placed them.
fn compute_edges(tolerance: f32, s: &mut Scratch) {
    s.edges.clear();
    s.live.clear();
    s.owner.clear();
    s.owner.resize(s.segments.len(), NIL);

    // An edge whose last segment lies more than the tolerance below the current one can take no more:
    // only the live edges are searched.
    for r in 0..s.placed.len() {
        let Placed { pos: spos, seg: idx, dir: sdir, round, .. } = s.placed[r];
        let floor = spos - tolerance;
        let (mut kept, mut found) = (0, None::<(u32, f32)>);
        for k in 0..s.live.len() {
            let e = s.live[k];
            let edge = &s.edges[e as usize];
            if edge.reach < floor {
                continue;
            }
            s.live[kept] = e;
            kept += 1;
            let d = (edge.pos - spos).abs();
            if edge.dir == sdir && d <= tolerance && found.is_none_or(|(_, fd)| d.total_cmp(&fd).is_lt()) {
                found = Some((e, d));
            }
        }
        s.live.truncate(kept);
        match found {
            Some((ei, _)) => {
                let e = &mut s.edges[ei as usize];
                let n = e.count as f32;
                e.pos = (e.pos * n + spos) / (n + 1.0);
                e.reach = spos;
                e.count += 1;
                e.rounds += u32::from(round);
                s.owner[idx as usize] = ei;
            }
            None => {
                let ei = s.edges.len() as u32;
                s.owner[idx as usize] = ei;
                s.live.push(ei);
                let rounds = u32::from(round);
                s.edges.push(Edge { pos: spos, reach: spos, count: 1, rounds, link: NIL, dir: sdir, round: false });
            }
        }
    }

    // Round when its round segments are at least as many as its straight ones.
    for e in s.edges.iter_mut() {
        e.round = e.rounds > 0 && e.rounds >= e.count - e.rounds;
    }

    // An edge links where the first of its segments, in rising position, links to one of another edge.
    for p in &s.placed {
        let (l, ei) = (s.segments[p.seg as usize].link, s.owner[p.seg as usize]);
        if l != NIL && s.edges[ei as usize].link == NIL {
            let target = s.owner[l as usize];
            if target != NIL && target != ei {
                s.edges[ei as usize].link = target;
            }
        }
    }
}

// Each point in no segment, between two of its contour's that are, placed between where those went, or shifted as
// the nearer moved when outside them; a contour with one point in a segment shifts whole.
fn interpolate_untouched(pts: &AutoPoints, out: &mut [i32], s: &mut Scratch) {
    let (y, base) = (&pts.y[..], &s.base[..]);
    for c in 0..pts.contour_ends.len() {
        let Some((start, end)) = pts.contour(c) else { continue };
        // Each point written and kept only when in a segment: a branch here mispredicts on most contours.
        if s.anchors.len() < end - start {
            s.anchors.resize(end - start, 0);
        }
        let mut kept = 0;
        for i in start..end {
            s.anchors[kept] = i as u32;
            kept += usize::from(s.seg_of[i] != NIL);
        }
        let anchors = &s.anchors[..kept];
        match kept {
            0 => {}
            1 => {
                let a = anchors[0] as usize;
                let delta = out[a] - base[a];
                for i in start..end {
                    if i != a {
                        out[i] = base[i] + delta;
                    }
                }
            }
            k => {
                for w in 0..k {
                    let (a, b) = (anchors[w] as usize, anchors[(w + 1) % k] as usize);
                    let (lo, hi, lo_out, hi_out, lo_base, hi_base) = if y[a] <= y[b] {
                        (y[a], y[b], out[a], out[b], base[a], base[b])
                    } else {
                        (y[b], y[a], out[b], out[a], base[b], base[a])
                    };
                    let mut i = if a + 1 == end { start } else { a + 1 };
                    while i != b {
                        let yi = y[i];
                        out[i] = if yi <= lo {
                            lo_out + (base[i] - lo_base)
                        } else if yi >= hi {
                            hi_out + (base[i] - hi_base)
                        } else if hi > lo {
                            let t = (yi - lo) / (hi - lo);
                            lo_out + ((hi_out - lo_out) as f32 * t) as i32
                        } else {
                            lo_out
                        };
                        i = if i + 1 == end { start } else { i + 1 };
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/auto/latin.rs"]
mod tests;
