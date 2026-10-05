use alloc::vec::Vec;

use super::f26dot6;
use super::auto::AutoPoints;
use crate::daecore::daetype::outline::cff_pen::{CffHints, CffStem};

fn active(mask: &[u8], stem: usize) -> bool {
    mask.get(stem / 8).is_some_and(|b| b & (0x80 >> (stem % 8)) != 0)
}

// Type 2 allows 96 stem hints, as FreeType holds it. Every point is mapped against every stem, so past
// that the glyph is left to the autohinter rather than costing points times stems.
const MAX_STEM_HINTS: usize = 96;

// A stem's edges in font units with where they land on the grid. An edge (ghost) hint, width -20 or
// -21, marks one edge: the top at its first value, the bottom at its second, as FreeType reads them.
fn edges_of(s: &CffStem, scale: &impl Fn(f32) -> i32) -> ([(f32, i32); 2], usize) {
    let ghost = |edge: f32| ([(edge, f26dot6::round_to_grid(scale(edge))); 2], 1);
    let width = s.max - s.min;
    if width == -20.0 {
        ghost(s.min)
    } else if width == -21.0 {
        ghost(s.max)
    } else {
        let (min, max) = (s.min.min(s.max), s.min.max(s.max));
        let min_fit = f26dot6::round_to_grid(scale(min));
        let width = f26dot6::round_to_grid(scale(max) - scale(min)).max(f26dot6::ONE);
        ([(min, min_fit), (max, min_fit + width)], 2)
    }
}

// The hint map for one mask, edges rising, into `map` (`order` scratch). A stem overlapping one placed
// is dropped, as FreeType drops a conflicting hint, and a fitted edge never falls below the one before.
fn hint_map(stems: &[CffStem], mask: &[u8], scale: &impl Fn(f32) -> i32, order: &mut Vec<usize>, map: &mut Vec<(f32, i32)>) {
    order.clear();
    order.extend((0..stems.len()).filter(|&i| !stems[i].vertical && active(mask, i)));
    order.sort_by(|&a, &b| stems[a].min.min(stems[a].max).total_cmp(&stems[b].min.min(stems[b].max)));
    map.clear();
    for &i in order.iter() {
        let (edges, n) = edges_of(&stems[i], scale);
        if map.last().is_some_and(|&(top, _)| edges[0].0 <= top) {
            continue;
        }
        for &(cs, ds) in &edges[..n] {
            let ds = map.last().map_or(ds, |&(_, prev)| ds.max(prev));
            map.push((cs, ds));
        }
    }
}

// FreeType's cf2_hintmap_map: between two edges by their fitted positions, beyond the outermost by
// the uniform scale from it, so points keep their order whichever stems they fall between.
fn map_one(y: f32, map: &[(f32, i32)], scale: &impl Fn(f32) -> i32) -> i32 {
    let Some(&(cs0, ds0)) = map.first() else { return scale(y) };
    if y < cs0 {
        return ds0 + scale(y - cs0);
    }
    let i = map.partition_point(|&(cs, _)| cs <= y) - 1;
    let (cs, ds) = map[i];
    match map.get(i + 1) {
        Some(&(next_cs, next_ds)) if next_cs > cs => ds + ((next_ds - ds) as f32 * (y - cs) / (next_cs - cs)) as i32,
        Some(_) => ds,
        None => ds + scale(y - cs),
    }
}

pub fn apply(pts: &AutoPoints, hints: &CffHints, size: f26dot6::Size, upm: u16) -> Option<Vec<i32>> {
    if upm == 0 || hints.stems.len() > MAX_STEM_HINTS || hints.stems.iter().all(|s| s.vertical) {
        return None;
    }
    let scale = |v: f32| f26dot6::scale_f32(v, size, upm);

    let all_on = [0xFFu8; MAX_STEM_HINTS.div_ceil(8)];
    let (mut order, mut map) = (Vec::with_capacity(hints.stems.len()), Vec::with_capacity(2 * hints.stems.len()));
    hint_map(&hints.stems, &all_on, &scale, &mut order, &mut map);
    let mut next = 0usize;
    let mut out = Vec::with_capacity(pts.y.len());
    for (i, &y) in pts.y.iter().enumerate() {
        if next < hints.masks.len() && hints.masks[next].0 <= i {
            while next < hints.masks.len() && hints.masks[next].0 <= i {
                next += 1;
            }
            hint_map(&hints.stems, &hints.masks[next - 1].1, &scale, &mut order, &mut map);
        }
        out.push(map_one(y, &map, &scale));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daecore::daetype::outline::cff_pen::CffStem;

    fn points(y: &[f32]) -> AutoPoints {
        AutoPoints { x: alloc::vec![0.0; y.len()], y: y.to_vec(), flags: alloc::vec![1; y.len()], contour_ends: alloc::vec![y.len() - 1] }
    }

    fn stems(s: &[(f32, f32)]) -> CffHints {
        CffHints { stems: s.iter().map(|&(min, max)| CffStem { min, max, vertical: false }).collect(), masks: Vec::new() }
    }

    #[test]
    fn past_96_stem_hints_the_cff_hinter_declines() {
        let pts = points(&[0.0, 10.0, 20.0, 30.0]);
        let size = f26dot6::Size::from_px(16.0).expect("a size");
        let hints = |n: usize| stems(&(0..n).map(|i| (i as f32 * 40.0, i as f32 * 40.0 + 20.0)).collect::<Vec<_>>());
        assert!(apply(&pts, &hints(MAX_STEM_HINTS), size, 1000).is_some(), "96 stems were declined");
        assert!(apply(&pts, &hints(MAX_STEM_HINTS + 1), size, 1000).is_none(), "97 stems were hinted");
    }

    // An edge hint marks one edge, the top (width -20) at its first value and the bottom (-21) at its
    // second: 700 units at 16 px lands on 704, the nearest pixel, not a pixel above as a 20-unit stem's.
    #[test]
    fn an_edge_hint_marks_one_edge() {
        let size = f26dot6::Size::from_px(16.0).expect("a size");
        assert_eq!(apply(&points(&[700.0]), &stems(&[(700.0, 680.0)]), size, 1000), Some(alloc::vec![704]));
        assert_eq!(apply(&points(&[700.0]), &stems(&[(721.0, 700.0)]), size, 1000), Some(alloc::vec![704]));
    }

    // Two stems at 16 px, and points between them: each maps by both edges around it, so 191 stays
    // below 217. By the nearer edge's shift alone they would swap, 267 above 193.
    #[test]
    fn points_between_stems_keep_their_order() {
        let size = f26dot6::Size::from_px(16.0).expect("a size");
        let out = apply(&points(&[191.0, 217.0]), &stems(&[(33.0, 56.0), (341.0, 363.0)]), size, 1000).expect("hinted");
        assert!(out[0] < out[1], "191 and 217 map to {out:?}");
    }

    // A CFF2 instance writes fractional coordinates, which are scaled as they are and rounded once.
    #[test]
    fn a_fractional_coordinate_is_not_truncated() {
        let size = f26dot6::Size::from_px(64.0).expect("a size");
        let out = apply(&points(&[10.7, -10.7]), &stems(&[(0.0, 1.0)]), size, 64).expect("hinted");
        assert_eq!(out, [685, -685]);
    }
}
