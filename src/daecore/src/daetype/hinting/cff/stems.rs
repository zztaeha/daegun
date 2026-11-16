use alloc::vec::Vec;

use super::ladder::{Ladder, Scale};
use super::zones::Zones;
use super::{HALF, ONE};

// Type 2 allows 96 stem hints, horizontal and vertical together (TN 5177, appendix B), past which drawing refuses a
// glyph; CFF2 sets no limit, and the hinter takes any number.
pub(crate) const MAX_STEMS: usize = 96;

// The edges a stem hint controls: both, or one for an edge hint (TN 5177, 4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Edges {
    Both,
    Bottom,
    Top,
}

// A horizontal stem in 16.16 font units, `bit` its place in a mask, and once placed the device y of its edges,
// which it keeps for the rest of the glyph so a contour crossing a hint replacement doesn't tear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Stem {
    pub(super) bottom: i64,
    pub(super) top: i64,
    pub(super) edges: Edges,
    pub(super) bit: u32,
    pub(super) place: Option<(i64, i64)>,
}

impl Stem {
    // A stem operator's pair: an edge and the width to the other. -21 marks a bottom edge alone at edge + width,
    // -20 a top edge alone at the edge; any other negative width reads as the stem with its edges swapped.
    pub(super) fn declared(edge: i64, width: i64, bit: u32) -> Stem {
        let (bottom, top, edges) = match width {
            -0x15_0000 => (edge + width, edge + width, Edges::Bottom),
            -0x14_0000 => (edge, edge, Edges::Top),
            ..0 => (edge + width, edge, Edges::Both),
            _ => (edge, edge + width, Edges::Both),
        };
        Stem { bottom, top, edges, bit, place: None }
    }

    // Where its middle maps through the anchors, its width the plain scale's; an edge hint's edge as it maps.
    fn natural(&self, anchors: &Ladder, scale: Scale) -> (i64, i64) {
        match self.edges {
            Edges::Bottom => (anchors.map(self.bottom), anchors.map(self.bottom)),
            Edges::Top => (anchors.map(self.top), anchors.map(self.top)),
            Edges::Both => {
                let width = scale.of(self.top - self.bottom);
                let bottom = anchors.map(self.bottom + (self.top - self.bottom) / 2) - width / 2;
                (bottom, bottom + width)
            }
        }
    }
}

// The stems a hint set holds, a bit each in declared order, the first stem the lowest bit of the first word.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Mask(Vec<u64>);

impl Mask {
    // Every one of `stems`.
    pub(super) fn fill(&mut self, stems: usize) {
        self.0.clear();
        self.0.resize(stems.div_ceil(64), u64::MAX);
        self.trim(stems);
    }

    // A hintmask's or cntrmask's bytes, high bit first (TN 5177, 4.3); bits past `stems` are left off.
    pub(super) fn read(&mut self, bytes: &[u8], stems: usize) {
        self.0.clear();
        self.0.resize(stems.div_ceil(64), 0);
        for (i, &b) in bytes.iter().take(stems.div_ceil(8)).enumerate() {
            self.0[i / 8] |= u64::from(b.reverse_bits()) << (8 * (i % 8));
        }
        self.trim(stems);
    }

    fn trim(&mut self, stems: usize) {
        if let Some(last) = self.0.last_mut().filter(|_| !stems.is_multiple_of(64)) {
            *last &= u64::MAX >> (64 - stems % 64);
        }
    }

    pub(super) fn holds(&self, bit: u32) -> bool {
        self.0.get(bit as usize / 64).is_some_and(|w| (w >> (bit % 64)) & 1 != 0)
    }
}

// The stems lowest first, by bottom edge and then as declared, the order every hint set takes them in, fixed with the
// stem list; and the stems and places each hint set takes.
#[derive(Default)]
pub(super) struct Order {
    sorted: Vec<u32>,
    taken: Vec<u32>,
    at: Vec<(i64, i64)>,
}

impl Order {
    pub(super) fn sort(&mut self, stems: &[Stem]) {
        self.sorted.clear();
        self.sorted.extend(0..stems.len() as u32);
        self.sorted.sort_unstable_by_key(|&i| (stems[i as usize].bottom, i));
    }

    // The stems `held` takes, lowest first, one touching or overlapping a stem before it left out, a placed stem
    // winning over a free one.
    fn take(&mut self, stems: &[Stem], held: impl Fn(&Stem) -> bool) {
        self.taken.clear();
        for &i in self.sorted.iter().filter(|&&i| held(&stems[i as usize])) {
            let stem = &stems[i as usize];
            let mut kept = true;
            while let Some(&last) = self.taken.last() {
                let last = &stems[last as usize];
                if stem.bottom > last.top {
                    break;
                }
                if last.place.is_some() || stem.place.is_none() {
                    kept = false;
                    break;
                }
                self.taken.pop();
            }
            if kept {
                self.taken.push(i);
            }
        }
    }
}

// A ladder from placed stems, lowest first, a stem whose bottom would land below the rung before it left out; then
// the em box's edges where they lie beyond the stems.
fn build(stems: &[Stem], taken: &[u32], frame: Option<[(i64, i64); 2]>, ladder: &mut Ladder) {
    ladder.clear();
    for stem in taken.iter().map(|&i| &stems[i as usize]) {
        let Some((bottom, top)) = stem.place else { continue };
        match stem.edges {
            Edges::Top if ladder.takes(stem.top, top) => ladder.push(stem.top, top),
            Edges::Bottom if ladder.takes(stem.bottom, bottom) => ladder.push(stem.bottom, bottom),
            Edges::Both if ladder.takes(stem.bottom, bottom) => {
                ladder.push(stem.bottom, bottom);
                if stem.top > stem.bottom {
                    ladder.push(stem.top, top);
                }
            }
            _ => {}
        }
    }
    for (glyph, device) in frame.into_iter().flatten() {
        ladder.hold(glyph, device);
    }
}

// The anchors free stems are placed by: every aligned stem's edges, taken as one hint set takes them, the em box's,
// and the origin where those don't reach it, which holds glyph y 0 at device 0 unless they move it.
pub(super) fn anchor(stems: &[Stem], order: &mut Order, frame: Option<[(i64, i64); 2]>, anchors: &mut Ladder) {
    order.take(stems, |s| s.place.is_some());
    build(stems, &order.taken, frame, anchors);
    anchors.hold(0, 0);
    anchors.finish();
}

// A hint set placed: its free stems placed in turn, lowest first, each keeping that place, then its ladder built.
// Returns how many stems it took.
pub(super) fn place(
    stems: &mut [Stem], mask: &Mask, zones: &Zones, anchors: &Ladder, ladder: &mut Ladder, order: &mut Order,
) -> usize {
    let scale = zones.scale();
    order.take(stems, |s| mask.holds(s.bit));
    let Order { taken, at, .. } = order;
    at.clear();
    at.extend(taken.iter().map(|&i| {
        let stem = &stems[i as usize];
        stem.place.unwrap_or_else(|| stem.natural(anchors, scale))
    }));
    for (k, &i) in taken.iter().enumerate() {
        let stem = &mut stems[i as usize];
        if stem.place.is_none() {
            let below = k.checked_sub(1).map(|k| at[k].1);
            let above = at.get(k + 1).map(|a| a.0);
            let by = snap(at[k], below, above);
            at[k] = (at[k].0 + by, at[k].1 + by);
            stem.place = Some(at[k]);
        }
    }
    build(stems, taken, zones.frame(), ladder);
    ladder.finish();
    taken.len()
}

// How far a free stem moves: the smaller of the moves that put one of its edges on a pixel, a tie up, or the other
// where that one would leave the counter it narrows under half a pixel; none where both would.
fn snap((bottom, top): (i64, i64), below: Option<i64>, above: Option<i64>) -> i64 {
    let up = |d: i64| (ONE - d.rem_euclid(ONE)) % ONE;
    let rise = up(bottom).min(up(top));
    let fall = -(bottom.rem_euclid(ONE).min(top.rem_euclid(ONE)));
    let rises = above.is_none_or(|a| a - (top + rise) >= HALF);
    let falls = below.is_none_or(|b| bottom + fall - b >= HALF);
    match (rise <= -fall, rises, falls) {
        (true, true, _) | (false, true, false) => rise,
        (false, _, true) | (true, false, true) => fall,
        _ => 0,
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/cff/stems.rs"]
mod tests;
