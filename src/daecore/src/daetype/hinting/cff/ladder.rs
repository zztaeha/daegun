use alloc::vec::Vec;

use super::{round_shift, saturate};

// Device values are held within ±2^50, past any point an outline keeps, so a sum of a few never overflows.
pub(super) const REACH: i64 = 1 << 50;
// 2^30 px a font unit, past any outline's reach.
const MOST_SCALE: i64 = 1 << 52;

// How far a glyph unit (1/65536 font unit) goes in device units (1/65536 px), with 32 fraction bits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Scale(i64);

impl Scale {
    // A scale in 26.6 per font unit, 16.16, as `Size::scale16` gives it; None where it isn't above 0.
    pub(crate) fn new(scale16: i64) -> Option<Scale> {
        (scale16 > 0).then(|| Scale(scale16.min(MOST_SCALE) << 10))
    }

    pub(super) fn of(self, glyph: i64) -> i64 {
        along(glyph, self.0)
    }

    pub(super) fn slope(self) -> i64 {
        self.0
    }
}

// `glyph` units along a slope, to the nearest device unit.
fn along(glyph: i64, slope: i64) -> i64 {
    saturate(round_shift(i128::from(glyph) * i128::from(slope), 32)).clamp(-REACH, REACH)
}

// An edge's glyph y, the device y it lands on, and the slope up to the next rung.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Rung {
    pub(super) glyph: i64,
    pub(super) device: i64,
    slope: i64,
}

// Rungs rising in glyph space and never falling in device space. A point between two maps on the line through
// them; one beyond the ends moves with the nearer end at the plain scale.
pub(super) struct Ladder {
    rungs: Vec<Rung>,
    plain: Scale,
}

impl Ladder {
    pub(super) fn new(mut rungs: Vec<Rung>, plain: Scale) -> Ladder {
        rungs.clear();
        Ladder { rungs, plain }
    }

    pub(super) fn clear(&mut self) {
        self.rungs.clear();
    }

    pub(super) fn take_rungs(&mut self) -> Vec<Rung> {
        core::mem::take(&mut self.rungs)
    }

    // Whether a rung there would rise from the last, or the ladder is empty.
    pub(super) fn takes(&self, glyph: i64, device: i64) -> bool {
        self.rungs.last().is_none_or(|r| glyph > r.glyph && device >= r.device)
    }

    // A rung on top, which `takes` has allowed; slopes are set once all are in, by `finish`.
    pub(super) fn push(&mut self, glyph: i64, device: i64) {
        self.rungs.push(Rung { glyph, device, slope: 0 });
    }

    // A rung at whichever end it lies beyond, where it keeps the ladder rising; none where the rungs reach it.
    pub(super) fn hold(&mut self, glyph: i64, device: i64) {
        let rung = Rung { glyph, device, slope: 0 };
        match (self.rungs.first(), self.rungs.last()) {
            (None, _) => self.rungs.push(rung),
            (Some(first), _) if first.glyph > glyph && first.device >= device => self.rungs.insert(0, rung),
            (_, Some(last)) if last.glyph < glyph && last.device <= device => self.rungs.push(rung),
            _ => {}
        }
    }

    pub(super) fn finish(&mut self) {
        for i in 1..self.rungs.len() {
            let (low, high) = (self.rungs[i - 1], self.rungs[i]);
            let rise = i128::from(high.device - low.device) << 32;
            self.rungs[i - 1].slope = saturate(rise / i128::from(high.glyph - low.glyph));
        }
        if let Some(last) = self.rungs.last_mut() {
            last.slope = self.plain.slope();
        }
    }

    pub(super) fn map(&self, glyph: i64) -> i64 {
        let above = self.rungs.partition_point(|r| r.glyph <= glyph);
        let next = self.rungs.get(above);
        let Some(at) = above.checked_sub(1).map(|i| &self.rungs[i]) else {
            return next.map_or_else(|| self.plain.of(glyph), |n| n.device + self.plain.of(glyph - n.glyph));
        };
        let device = at.device + along(glyph - at.glyph, at.slope);
        // A point between two rungs stays between them, whatever its slope rounded to.
        next.map_or(device, |n| device.min(n.device))
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/cff/ladder.rs"]
mod tests;
