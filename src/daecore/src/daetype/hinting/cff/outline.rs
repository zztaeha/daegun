use alloc::vec::Vec;

use super::super::{HintedOutline, FLAG_CUBIC, FLAG_ON_CURVE};

// 2^23 px in device units, the 26.6 range the other hinters keep, so any two points add without overflowing.
const REACH: i64 = 1 << 39;

// A hinted outline as it's built, in 26.6: contours of on-curve points and cubic controls, each closing back to
// its first point.
#[derive(Default)]
pub(crate) struct Outline {
    x: Vec<i32>,
    y: Vec<i32>,
    flags: Vec<u8>,
    ends: Vec<usize>,
    // Where the open contour starts.
    first: usize,
    over: bool,
}

impl Outline {
    // Room for about the points a charstring of `bytes` draws, so a glyph's outline rarely grows: its points are as
    // many as its steps draw, so only the reservation stops at 64K.
    pub(crate) fn for_charstring(bytes: usize) -> Outline {
        let points = bytes.min(1 << 16);
        Outline {
            x: Vec::with_capacity(points),
            y: Vec::with_capacity(points),
            flags: Vec::with_capacity(points),
            ends: Vec::with_capacity(points / 16 + 1),
            ..Outline::default()
        }
    }

    // A device point (16.16 px) to the nearest 1/64, a half up; one past REACH fails the outline.
    pub(super) fn push(&mut self, (x, y): (i64, i64), on_curve: bool) {
        let fits = |v: i64| (-REACH..=REACH).contains(&v);
        if !fits(x) || !fits(y) {
            self.over = true;
            return;
        }
        self.x.push(((x + 512) >> 10) as i32);
        self.y.push(((y + 512) >> 10) as i32);
        self.flags.push(if on_curve { FLAG_ON_CURVE } else { FLAG_CUBIC });
    }

    // The open contour ended: its last point dropped where the charstring drew back to the start (the close draws
    // that line) or it lies on the first, then the contour if one point is left.
    pub(super) fn close(&mut self, back_at_start: bool) {
        if back_at_start && self.x.len() > self.first + 1 {
            self.pop();
        }
        let (n, first) = (self.x.len(), self.first);
        let on_first = n > first + 1 && (self.x[n - 1], self.y[n - 1]) == (self.x[first], self.y[first]);
        if on_first && self.flags[n - 1] == FLAG_ON_CURVE {
            self.pop();
        }
        match self.x.len() - first {
            0 => {}
            1 => self.pop(),
            _ => self.ends.push(self.x.len() - 1),
        }
        self.first = self.x.len();
    }

    fn pop(&mut self) {
        self.x.pop();
        self.y.pop();
        self.flags.pop();
    }

    // The outline once every contour has closed; None where a point went past the 26.6 range.
    pub(crate) fn finish(self) -> Option<HintedOutline> {
        (!self.over).then_some(HintedOutline { x: self.x, y: self.y, flags: self.flags, contour_ends: self.ends })
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/cff/outline.rs"]
mod tests;
