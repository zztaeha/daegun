use alloc::vec::Vec;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daemachine::float::FloatExt;

use super::points::{AutoPoints, ON_CURVE};

// FreeType's Latin blue strings (afblue.dat) and whether each zone is a top one.
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
    pub reference: f32,
    pub overshoot: f32,
    pub is_top: bool,
}

#[derive(Clone, Debug, Default)]
pub struct BlueZones {
    pub zones: Vec<BlueZone>,
}

// FreeType's af_latin_metrics_init_blues: per zone, each character's extremum, flat or round, then the
// median of the flats as the reference and of the rounds as the overshoot.
pub fn compute(
    upm: u16,
    resolve: &mut dyn FnMut(char) -> Option<u16>,
    outline_of: &mut dyn FnMut(u16) -> Option<AutoPoints>,
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
        let (reference, overshoot) = match (median(&mut flats), median(&mut rounds)) {
            (Some(f), Some(r)) => (f, r),
            (Some(v), None) | (None, Some(v)) => (v, v),
            (None, None) => continue,
        };
        // An overshoot on the wrong side of its reference takes the two's mean for both.
        let (reference, overshoot) = if overshoot != reference && is_top != (overshoot > reference) {
            let mean = ((reference + overshoot) / 2.0).trunc();
            (mean, mean)
        } else {
            (reference, overshoot)
        };
        zones.push(BlueZone { reference, overshoot, is_top });
    }
    // Zones must not overlap: sorted by their lower edges, a zone's upper edge (a top zone's
    // overshoot, a bottom zone's reference) never rises past the next zone's.
    let lower = |z: &BlueZone| if z.is_top { z.reference } else { z.overshoot };
    let mut order: Vec<usize> = (0..zones.len()).collect();
    order.sort_by(|&a, &b| lower(&zones[a]).total_cmp(&lower(&zones[b])));
    for w in order.windows(2) {
        let edge = |z: &BlueZone| if z.is_top { z.overshoot } else { z.reference };
        let next = edge(&zones[w[1]]);
        let z = &mut zones[w[0]];
        if edge(z) > next {
            if z.is_top { z.overshoot = next } else { z.reference = next }
        }
    }
    BlueZones { zones }
}

// The highest (lowest) point of contours of two or more, and whether its level run is round, as
// FreeType walks it: flat when on-curve points span over upm / 14, round when it ends off-curve.
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
mod tests {
    use super::*;

    // From the top at 706, the walk goes on past a point 6 units down but 300 across, a shallow
    // angle, and the run ends on that control point: round. Stopping at the drop would make it flat.
    #[test]
    fn a_shallow_drop_stays_in_the_level_run() {
        let pts = AutoPoints {
            x: alloc::vec![0.0, 0.0, 100.0, 400.0, 400.0],
            y: alloc::vec![0.0, 700.0, 706.0, 700.0, 0.0],
            flags: alloc::vec![ON_CURVE, ON_CURVE, ON_CURVE, 0, ON_CURVE],
            contour_ends: alloc::vec![4],
        };
        assert_eq!(extremum(&pts, true, 71.0), Some((706.0, true)));
    }
}
