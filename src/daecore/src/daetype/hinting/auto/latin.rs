use alloc::vec::Vec;

use super::blues::{BlueZone, BlueZones};
use super::points::AutoPoints;

use crate::daecore::daetype::hinting::f26dot6;

#[derive(Clone, Debug)]
struct Segment {
    pos: f32,
    points: (u32, u32),
    dir: i8,
    x_min: f32,
    x_max: f32,
    round: bool,
    link: Option<usize>,
}

#[derive(Clone, Debug)]
struct Edge {
    pos: f32,
    dir: i8,
    head: u32,
    tail: u32,
    count: u32,
    round: bool,
    // The position of the last segment it took, the highest, since segments come in rising order.
    reach: f32,
    link: Option<usize>,
}

// A segment links to the nearest facing one; past this many others in the way it looks no further,
// which only a crafted outline reaches and which bounds linking at segments times this.
const LINK_SCAN: usize = 256;

const NIL: u32 = u32::MAX;

#[derive(Default)]
pub struct Scratch {
    seg_arena: Vec<usize>,
    segments: Vec<Segment>,
    seg_next: Vec<u32>,
    owner: Vec<u32>,
    edges: Vec<Edge>,
    order: Vec<usize>,
    live: Vec<usize>,
    chosen: Vec<Option<usize>>,
    targets: Vec<i32>,
    blue_matched: Vec<bool>,
    done: Vec<bool>,
    touched: Vec<bool>,
    covered: Vec<bool>,
    anchors: Vec<usize>,
    y: Vec<i32>,
}

pub fn fit<'s>(pts: &AutoPoints, blues: &BlueZones, size: f26dot6::Size, upm: u16, s: &'s mut Scratch) -> &'s [i32] {
    let scale = |v: f32| f26dot6::scale_f32(v, size, upm);

    let n = pts.len();
    s.y.clear();
    s.y.extend((0..n).map(|i| scale(pts.y[i])));
    if n == 0 {
        return &s.y;
    }

    let seg_tolerance = upm as f32 / 100.0;
    let edge_tolerance = upm as f32 / 50.0;
    let flat_threshold = f32::from(upm / 14);

    compute_segments(pts, seg_tolerance, flat_threshold, s);
    if s.segments.is_empty() {
        return &s.y;
    }
    link_segments(&mut s.segments, upm as f32 / 3.0, &mut s.order, &mut s.chosen);
    compute_edges(edge_tolerance, s);

    let zones = fitted_zones(blues, &scale);
    // An edge snaps to the nearest zone within upm / 40, or half a pixel at most: to the reference,
    // or, for a round edge past the reference, to the overshoot (FreeType's compute_blue_edges).
    let reach = scale(f32::from(upm / 40)).min(f26dot6::HALF);
    s.targets.clear();
    s.blue_matched.clear();
    for edge in &s.edges {
        let mut best = (reach, None);
        for (z, fit) in &zones {
            if z.is_top != (edge.dir > 0) {
                continue;
            }
            let d = scale((edge.pos - z.reference).abs());
            if d < best.0 {
                best = (d, Some(fit.0));
            }
            if edge.round && d != 0 && z.is_top != (edge.pos < z.reference) {
                let d = scale((edge.pos - z.overshoot).abs());
                if d < best.0 {
                    best = (d, Some(fit.1));
                }
            }
        }
        s.targets.push(best.1.unwrap_or_else(|| f26dot6::round_to_grid(scale(edge.pos))));
        s.blue_matched.push(best.1.is_some());
    }

    s.done.clear();
    s.done.resize(s.edges.len(), false);
    for i in 0..s.edges.len() {
        let Some(j) = s.edges[i].link else { continue };
        if s.done[i] || s.done[j] {
            continue;
        }
        let anchor = if s.blue_matched[i] || !s.blue_matched[j] { i } else { j };
        let other = if anchor == i { j } else { i };
        let width = fit_stem_width((scale(s.edges[other].pos) - scale(s.edges[anchor].pos)).abs());
        let signed = if s.edges[other].pos > s.edges[anchor].pos { width } else { -width };
        s.targets[other] = s.targets[anchor] + signed;
        s.done[i] = true;
        s.done[j] = true;
    }

    s.touched.clear();
    s.touched.resize(n, false);
    for e in 0..s.edges.len() {
        let delta = s.targets[e] - scale(s.edges[e].pos);
        let mut seg = s.edges[e].head;
        while seg != NIL {
            let sd = &s.segments[seg as usize];
            let (off, len) = (sd.points.0 as usize, sd.points.1 as usize);
            for k in off..off + len {
                let i = s.seg_arena[k];
                if delta != 0 {
                    s.y[i] += delta;
                }
                s.touched[i] = true;
            }
            seg = s.seg_next[seg as usize];
        }
    }

    interpolate_untouched(pts, &mut s.y, &s.touched, size, upm, &mut s.anchors);
    &s.y
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
    let mut widest = 0.0f32;
    let mut sign = -1.0f32;
    for c in 0..pts.contour_ends.len() {
        let Some((start, end)) = pts.contour(c) else { continue };
        let n = end - start;
        let mut area = 0.0f32;
        for i in 0..n {
            let (a, b) = (start + i, start + (i + 1) % n);
            area += pts.x[a] * pts.y[b] - pts.x[b] * pts.y[a];
        }
        if area.abs() > widest {
            widest = area.abs();
            sign = if area >= 0.0 { 1.0 } else { -1.0 };
        }
    }
    -sign
}

// The zones active at a size, a zone only while its overshoot is under 3/4 pixel, each with its
// reference rounded and its overshoot 0, 1/2 or 1 pixel past it (af_latin_metrics_scale_dim).
fn fitted_zones<'z>(blues: &'z BlueZones, scale: &impl Fn(f32) -> i32) -> Vec<(&'z BlueZone, (i32, i32))> {
    blues
        .zones
        .iter()
        .filter_map(|z| {
            let dist = scale(z.reference - z.overshoot);
            if dist.abs() > 48 {
                return None;
            }
            let step = match dist.abs() {
                d if d < 32 => 0,
                d if d < 48 => 32,
                _ => 64,
            };
            let reference = f26dot6::round_to_grid(scale(z.reference));
            Some((z, (reference, reference - step * dist.signum())))
        })
        .collect()
}

fn compute_segments(pts: &AutoPoints, tolerance: f32, flat_threshold: f32, s: &mut Scratch) {
    let convention = winding_convention(pts);
    s.segments.clear();
    s.seg_arena.clear();
    for c in 0..pts.contour_ends.len() {
        let Some((start, end)) = pts.contour(c) else { continue };
        let n = end - start;
        if n < 2 {
            continue;
        }
        let at = |k: usize| start + k % n;

        let Some(break_at) = (0..n).find(|&k| (pts.y[at(k)] - pts.y[at(k + n - 1)]).abs() > tolerance)
        else {
            continue;
        };

        let mut k = 0usize;
        while k < n {
            let first = break_at + k;
            let mut len = 1usize;
            while len < n - k && (pts.y[at(first + len)] - pts.y[at(first)]).abs() <= tolerance {
                len += 1;
            }
            if len >= 2 {
                let mut sum = 0.0f32;
                for m in 0..len {
                    sum += pts.y[at(first + m)];
                }
                let pos = sum / len as f32;
                let off = s.seg_arena.len() as u32;
                s.seg_arena.extend((0..len).map(|m| at(first + m)));
                let (mut x_min, mut x_max) = (f32::MAX, f32::MIN);
                for &i in &s.seg_arena[off as usize..] {
                    x_min = x_min.min(pts.x[i]);
                    x_max = x_max.max(pts.x[i]);
                }
                let dx = pts.x[at(first + len - 1)] - pts.x[at(first)];
                let neighbors = (pts.y[at(first + n - 1)] + pts.y[at(first + len)]) * 0.5;
                let round = is_round(pts, &s.seg_arena[off as usize..], flat_threshold);
                s.segments.push(Segment {
                    pos,
                    points: (off, len as u32),
                    dir: side(dx, convention, if neighbors < pos { 1 } else { -1 }),
                    x_min,
                    x_max,
                    round,
                    link: None,
                });
            }
            k += len;
        }
    }
    round_extrema(pts, convention, s);
}

// FreeType's round segment: an end of its run, by x, is a control point and its on-curve points
// span less than upm / 14.
fn is_round(pts: &AutoPoints, run: &[usize], flat_threshold: f32) -> bool {
    let by_x = |a: &&usize, b: &&usize| pts.x[**a].total_cmp(&pts.x[**b]);
    let (Some(&lo), Some(&hi)) = (run.iter().min_by(by_x), run.iter().max_by(by_x)) else { return false };
    let control = |i: usize| pts.flags[i] & super::points::ON_CURVE == 0;
    let on = run.iter().filter(|&&i| !control(i)).map(|&i| pts.x[i]);
    let span = on.clone().fold(f32::MIN, f32::max) - on.fold(f32::MAX, f32::min);
    (control(lo) || control(hi)) && span < flat_threshold
}

// Each segment links to the nearest facing it across a stem (other direction, inward, overlapping in
// x, within `max_distance`, lower index on a tie), searched outward by position, not pairwise.
fn link_segments(segments: &mut [Segment], max_distance: f32, order: &mut Vec<usize>, chosen: &mut Vec<Option<usize>>) {
    order.clear();
    order.extend(0..segments.len());
    order.sort_by(|&a, &b| segments[a].pos.total_cmp(&segments[b].pos).then(a.cmp(&b)));
    let mut rank = alloc::vec![0usize; segments.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    for i in 0..segments.len() {
        let a = &segments[i];
        let candidates: &mut dyn Iterator<Item = &usize> =
            if a.dir > 0 { &mut order[..rank[i]].iter().rev() } else { &mut order[rank[i] + 1..].iter() };
        let mut best: Option<(f32, usize)> = None;
        for &j in candidates.take(LINK_SCAN) {
            let b = &segments[j];
            let d = (a.pos - b.pos).abs();
            if d == 0.0 {
                continue;
            }
            if d > max_distance || best.is_some_and(|(bd, _)| d > bd) {
                break;
            }
            let overlap = a.x_max.min(b.x_max) - a.x_min.max(b.x_min);
            if b.dir != a.dir && overlap > 0.0 && best.is_none_or(|(bd, bj)| d < bd || j < bj) {
                best = Some((d, j));
            }
        }
        segments[i].link = best.map(|(_, j)| j);
    }
    chosen.clear();
    chosen.extend(segments.iter().map(|s| s.link));
    for (i, seg) in segments.iter_mut().enumerate() {
        if seg.link.is_some_and(|j| chosen[j] != Some(i)) {
            seg.link = None;
        }
    }
}

fn fit_stem_width(scaled: i32) -> i32 {
    f26dot6::round_to_grid(scaled).max(f26dot6::ONE)
}

fn round_extrema(pts: &AutoPoints, convention: f32, s: &mut Scratch) {
    s.covered.clear();
    s.covered.resize(pts.len(), false);
    for seg in s.segments.iter() {
        let (off, len) = (seg.points.0 as usize, seg.points.1 as usize);
        for k in off..off + len {
            if let Some(slot) = s.covered.get_mut(s.seg_arena[k]) {
                *slot = true;
            }
        }
    }
    for c in 0..pts.contour_ends.len() {
        let Some((start, end)) = pts.contour(c) else { continue };
        let n = end - start;
        if n < 3 {
            continue;
        }
        for i in start..end {
            if s.covered.get(i).copied().unwrap_or(false) {
                continue;
            }
            let prev = start + (i - start + n - 1) % n;
            let next = start + (i - start + 1) % n;
            let (y, yp, yn) = (pts.y[i], pts.y[prev], pts.y[next]);
            let is_min = y < yp && y < yn;
            let is_max = y > yp && y > yn;
            if !is_min && !is_max {
                continue;
            }
            let off = s.seg_arena.len() as u32;
            s.seg_arena.push(i);
            let control = |p: usize| pts.flags[p] & super::points::ON_CURVE == 0;
            s.segments.push(Segment {
                pos: y,
                points: (off, 1),
                dir: side(pts.x[next] - pts.x[prev], convention, if (yp + yn) * 0.5 < y { 1 } else { -1 }),
                x_min: pts.x[i],
                x_max: pts.x[i],
                round: control(i) || control(prev) || control(next),
                link: None,
            });
        }
    }
}

fn compute_edges(tolerance: f32, s: &mut Scratch) {
    s.order.clear();
    s.order.extend(0..s.segments.len());
    s.order.sort_by(|&a, &b| s.segments[a].pos.total_cmp(&s.segments[b].pos));

    s.edges.clear();
    s.live.clear();
    s.seg_next.clear();
    s.seg_next.resize(s.segments.len(), NIL);
    s.owner.clear();
    s.owner.resize(s.segments.len(), NIL);

    // Segments come in rising order, so an edge whose last segment lies more than the tolerance below
    // the current one can take no more: only the live edges are searched.
    for oi in 0..s.order.len() {
        let idx = s.order[oi];
        let (spos, sdir) = (s.segments[idx].pos, s.segments[idx].dir);
        let edges = &s.edges;
        s.live.retain(|&e| edges[e].reach >= spos - tolerance);
        let found = s
            .live
            .iter()
            .map(|&e| (e, &s.edges[e]))
            .filter(|(_, e)| e.dir == sdir && (e.pos - spos).abs() <= tolerance)
            .min_by(|(_, a), (_, b)| (a.pos - spos).abs().total_cmp(&(b.pos - spos).abs()))
            .map(|(i, _)| i);
        match found {
            Some(ei) => {
                let e = &mut s.edges[ei];
                let n = e.count as f32;
                e.pos = (e.pos * n + spos) / (n + 1.0);
                e.reach = spos;
                let tail = e.tail as usize;
                e.tail = idx as u32;
                e.count += 1;
                s.seg_next[tail] = idx as u32;
                s.owner[idx] = ei as u32;
            }
            None => {
                s.owner[idx] = s.edges.len() as u32;
                s.live.push(s.edges.len());
                s.edges.push(Edge {
                    pos: spos,
                    dir: sdir,
                    head: idx as u32,
                    tail: idx as u32,
                    count: 1,
                    round: false,
                    reach: spos,
                    link: None,
                });
            }
        }
    }

    // Round when its round segments are at least as many as its straight ones.
    for e in s.edges.iter_mut() {
        let (mut round, mut straight) = (0u32, 0u32);
        let mut seg = e.head;
        while seg != NIL {
            if s.segments[seg as usize].round { round += 1 } else { straight += 1 }
            seg = s.seg_next[seg as usize];
        }
        e.round = round > 0 && round >= straight;
    }

    for ei in 0..s.edges.len() {
        let mut seg = s.edges[ei].head;
        let mut link = None;
        while seg != NIL {
            if let Some(l) = s.segments[seg as usize].link {
                let target = s.owner[l];
                if target != NIL && target as usize != ei {
                    link = Some(target as usize);
                    break;
                }
            }
            seg = s.seg_next[seg as usize];
        }
        s.edges[ei].link = link;
    }
}

fn interpolate_untouched(
    pts: &AutoPoints,
    out: &mut [i32],
    touched: &[bool],
    size: f26dot6::Size,
    upm: u16,
    anchors: &mut Vec<usize>,
) {
    let scale = |v: f32| f26dot6::scale_f32(v, size, upm);

    for c in 0..pts.contour_ends.len() {
        let Some((start, end)) = pts.contour(c) else { continue };
        let n = end - start;
        if n == 0 {
            continue;
        }
        anchors.clear();
        anchors.extend((start..end).filter(|&i| touched[i]));
        if anchors.is_empty() {
            continue;
        }
        if anchors.len() == 1 {
            let a = anchors[0];
            let delta = out[a] - scale(pts.y[a]);
            for (i, o) in out[start..end].iter_mut().enumerate() {
                let i = start + i;
                if i != a {
                    *o = scale(pts.y[i]) + delta;
                }
            }
            continue;
        }

        for w in 0..anchors.len() {
            let a = anchors[w];
            let b = anchors[(w + 1) % anchors.len()];
            let mut i = start + (a - start + 1) % n;
            while i != b {
                out[i] = interpolate_one(pts, out, i, a, b, scale);
                i = start + (i - start + 1) % n;
            }
        }
    }
}

fn interpolate_one(
    pts: &AutoPoints,
    out: &[i32],
    i: usize,
    a: usize,
    b: usize,
    scale: impl Fn(f32) -> i32,
) -> i32 {
    let (ya, yb, yi) = (pts.y[a], pts.y[b], pts.y[i]);
    let (lo, hi, lo_out, hi_out) = if ya <= yb { (ya, yb, out[a], out[b]) } else { (yb, ya, out[b], out[a]) };

    if yi <= lo {
        lo_out + (scale(yi) - scale(lo))
    } else if yi >= hi {
        hi_out + (scale(yi) - scale(hi))
    } else if hi > lo {
        let t = (yi - lo) / (hi - lo);
        lo_out + ((hi_out - lo_out) as f32 * t) as i32
    } else {
        lo_out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A zone is active at a size only while its overshoot is at most 3/4 pixel: 50 units of 1000 are
    // 0.75 px at 15 px and 0.77 px at 15.4, so only the exact size turns it off.
    #[test]
    fn a_zone_is_judged_at_the_exact_size() {
        let blues = BlueZones { zones: alloc::vec![BlueZone { reference: 500.0, overshoot: 550.0, is_top: true }] };
        let active = |px| {
            let size = f26dot6::Size::from_px(px).expect("a size");
            fitted_zones(&blues, &|v| f26dot6::scale_f32(v, size, 1000)).len()
        };
        assert_eq!((active(15.0), active(15.4)), (1, 0));
    }

    // The last run starts at 10 and comes back round past 0 to the 20s, all within 10 of its start.
    // Bounded by the contour rather than by what is left of it, it would take points 0 and 1 again.
    #[test]
    fn no_point_falls_in_two_segments() {
        let pts = AutoPoints {
            x: alloc::vec![0.0, 100.0, 100.0, 0.0, 0.0, 100.0],
            y: alloc::vec![20.0, 20.0, 60.0, 60.0, 10.0, 0.0],
            flags: alloc::vec![super::super::points::ON_CURVE; 6],
            contour_ends: alloc::vec![5],
        };
        let mut s = Scratch::default();
        compute_segments(&pts, 10.0, 1000.0, &mut s);
        let mut taken = s.seg_arena.clone();
        taken.sort_unstable();
        assert!(taken.windows(2).all(|w| w[0] != w[1]), "segments took {:?}", s.seg_arena);
    }

    // Two facing segments with runs between them, off to the side: they link while LINK_SCAN - 1
    // stand in the way and not past that, so a crafted outline costs segments times LINK_SCAN.
    #[test]
    fn linking_looks_past_at_most_link_scan_segments() {
        let seg = |pos, dir, x_min| Segment { pos, points: (0, 0), dir, x_min, x_max: x_min + 100.0, round: false, link: None };
        let linked = |between: usize| {
            let mut segments = alloc::vec![seg(0.0, -1, 0.0)];
            segments.extend((0..between).map(|i| seg(1.0 + i as f32 * 0.1, -1, 200.0)));
            segments.push(seg(100.0, 1, 0.0));
            link_segments(&mut segments, 333.0, &mut Vec::new(), &mut Vec::new());
            segments[0].link.is_some()
        };
        assert_eq!((linked(LINK_SCAN - 1), linked(LINK_SCAN)), (true, false));
    }
}
