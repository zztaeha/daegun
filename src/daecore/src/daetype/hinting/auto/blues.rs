use alloc::vec::Vec;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;

use super::points::{AutoPoints, ON_CURVE};

// The Latin blue strings and whether each zone is a top one.
const ZONE_SPECS: &[(&str, bool)] = &[
    ("THEZOCQS", true),
    ("HEZLOCUS", false),
    ("fijkdbh", true),
    ("uvxzoesc", true),
    ("nrxzoesc", false),
    ("pqgjy", false),
];

#[derive(Clone, Copy, Debug)]
pub struct BlueZone {
    pub flat: f32,
    pub overshoot: f32,
    pub is_top: bool,
    // Where the flat edge and overshoot land at the size the hinter last fitted, kept so every glyph at that size
    // reuses them; None where the zone is off there.
    pub(crate) fit: Option<(i32, i32)>,
}

impl BlueZone {
    pub(crate) fn new(flat: f32, overshoot: f32, is_top: bool) -> BlueZone {
        BlueZone { flat, overshoot, is_top, fit: None }
    }
}

#[derive(Clone, Debug, Default)]
pub struct BlueZones {
    pub zones: Vec<BlueZone>,
}

// The blue zones: per zone, each character's extremum, flat or round, then the median of the flats as
// the flat edge and of the rounds as the overshoot.
pub fn compute(
    upm: u16,
    resolve: &mut dyn FnMut(char) -> Option<u32>,
    outline_of: &mut dyn FnMut(u32) -> Option<AutoPoints>,
) -> BlueZones {
    let flat_threshold = f32::from(upm / 14);
    let mut zones = Vec::new();
    for &(chars, is_top) in ZONE_SPECS {
        let (mut flats, mut rounds) = (Vec::new(), Vec::new());
        for ch in chars.chars() {
            let Some(gid) = resolve(ch) else { continue };
            let Some(pts) = outline_of(gid) else { continue };
            let Some((y, round)) = extremum(&pts, is_top, flat_threshold) else { continue };
            if round { rounds.push(y) } else { flats.push(y) }
        }
        let (flat, overshoot) = match (median(&mut flats), median(&mut rounds)) {
            (Some(f), Some(r)) => (f, r),
            (Some(v), None) | (None, Some(v)) => (v, v),
            (None, None) => continue,
        };
        // An overshoot on the wrong side of its flat edge takes the two's mean for both.
        let (flat, overshoot) = if overshoot != flat && is_top != (overshoot > flat) {
            let mean = ((flat + overshoot) / 2.0).trunc();
            (mean, mean)
        } else {
            (flat, overshoot)
        };
        zones.push(BlueZone::new(flat, overshoot, is_top));
    }
    // Zones must not overlap: sorted by their lower edges, a zone's upper edge (a top zone's
    // overshoot, a bottom zone's flat edge) never rises past the next zone's.
    let lower = |z: &BlueZone| if z.is_top { z.flat } else { z.overshoot };
    let mut order: Vec<usize> = (0..zones.len()).collect();
    order.sort_by(|&a, &b| lower(&zones[a]).total_cmp(&lower(&zones[b])));
    for w in order.windows(2) {
        let edge = |z: &BlueZone| if z.is_top { z.overshoot } else { z.flat };
        let next = edge(&zones[w[1]]);
        let z = &mut zones[w[0]];
        if edge(z) > next {
            if z.is_top { z.overshoot = next } else { z.flat = next }
        }
    }
    BlueZones { zones }
}

// The highest (lowest) point of contours of two or more, and whether its level run is round: flat when on-curve
// points span over upm / 14, about a stem's width and so a stroke's end, round when it ends off-curve.
fn extremum(pts: &AutoPoints, is_top: bool, flat_threshold: f32) -> Option<(f32, bool)> {
    if pts.len() <= 2 {
        return None;
    }
    let mut best: Option<(usize, usize, usize)> = None;
    for c in 0..pts.contour_ends.len() {
        let Some((first, end)) = pts.contour(c) else { continue };
        let last = end - 1;
        if last <= first {
            continue;
        }
        for p in first..=last {
            let better = best.is_none_or(|(b, _, _)| if is_top { pts.y[p] > pts.y[b] } else { pts.y[p] < pts.y[b] });
            if better {
                best = Some((p, first, last));
            }
        }
    }
    let (point, first, last) = best?;
    let (bx, by) = (pts.x[point], pts.y[point]);
    let on = |p: usize| pts.flags[p] & ON_CURVE != 0;
    let level = |p: usize| {
        let dist = (pts.y[p] - by).abs();
        dist <= 5.0 || (pts.x[p] - bx).abs() > 20.0 * dist
    };
    let (mut seg_first, mut seg_last) = (point, point);
    let (mut on_first, mut on_last) = if on(point) { (Some(point), Some(point)) } else { (None, None) };
    let mut prev = point;
    loop {
        prev = if prev > first { prev - 1 } else { last };
        if !level(prev) {
            break;
        }
        seg_first = prev;
        if on(prev) {
            on_first = Some(prev);
            on_last.get_or_insert(prev);
        }
        if prev == point {
            break;
        }
    }
    let mut next = point;
    loop {
        next = if next < last { next + 1 } else { first };
        if !level(next) {
            break;
        }
        seg_last = next;
        if on(next) {
            on_last = Some(next);
            on_first.get_or_insert(next);
        }
        if next == point {
            break;
        }
    }
    let flat = matches!((on_first, on_last), (Some(a), Some(b)) if (pts.x[b] - pts.x[a]).abs() > flat_threshold);
    let round = !flat && (!on(seg_first) || !on(seg_last));
    Some((by, round))
}

fn median(v: &mut [f32]) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() / 2])
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/auto/blues.rs"]
mod tests;
