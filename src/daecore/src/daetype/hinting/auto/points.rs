use alloc::vec::Vec;

use crate::daecore::daetype::outline::OutlinePen;

pub const ON_CURVE: u8 = 0x01;
pub const CONIC: u8 = 0x02;
pub const CUBIC: u8 = 0x04;

#[derive(Default, Clone, Debug)]
pub struct AutoPoints {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub flags: Vec<u8>,
    pub contour_ends: Vec<usize>,
}

impl AutoPoints {
    pub fn len(&self) -> usize {
        self.x.len()
    }

    pub fn is_empty(&self) -> bool {
        self.x.is_empty()
    }

    pub fn contour(&self, i: usize) -> Option<(usize, usize)> {
        let end = *self.contour_ends.get(i)?;
        let start = if i == 0 { 0 } else { self.contour_ends[i - 1] + 1 };
        (start <= end && end < self.len()).then_some((start, end + 1))
    }
}

#[derive(Default)]
pub struct CollectPen {
    pts: AutoPoints,
    contour_start: Option<usize>,
    current: (f32, f32),
}

const TYPICAL_POINTS: usize = 64;
const TYPICAL_CONTOURS: usize = 8;

// The lists the thread's last points dropped, emptied, lent to its next pen; one past `SPARE_BYTES` is dropped.
#[cfg(feature = "std")]
std::thread_local! {
    static SPARE: core::cell::Cell<Option<AutoPoints>> = const { core::cell::Cell::new(None) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE.try_with(core::cell::Cell::take);
}

#[cfg(feature = "std")]
const SPARE_BYTES: usize = 64 << 10;

// Into an empty slot only: one that holds points keeps them, so no points' drop sets it again.
impl Drop for AutoPoints {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        let _ = SPARE.try_with(|slot| {
            use crate::daecore::daekit::base::sync::heap_of;
            let held = slot.take();
            let AutoPoints { x, y, flags, contour_ends } = self;
            if held.is_some() || heap_of(x) + heap_of(y) + heap_of(flags) + heap_of(contour_ends) > SPARE_BYTES {
                return slot.set(held);
            }
            x.clear();
            y.clear();
            flags.clear();
            contour_ends.clear();
            slot.set(Some(core::mem::take(self)));
        });
    }
}

impl CollectPen {
    pub fn new() -> CollectPen {
        let typical = || AutoPoints {
            x: Vec::with_capacity(TYPICAL_POINTS),
            y: Vec::with_capacity(TYPICAL_POINTS),
            flags: Vec::with_capacity(TYPICAL_POINTS),
            contour_ends: Vec::with_capacity(TYPICAL_CONTOURS),
        };
        #[cfg(feature = "std")]
        let pts = SPARE.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_else(typical);
        #[cfg(not(feature = "std"))]
        let pts = typical();
        CollectPen { pts, contour_start: None, current: (0.0, 0.0) }
    }

    pub fn finish(mut self) -> AutoPoints {
        self.end_contour();
        self.pts
    }

    fn push(&mut self, x: f32, y: f32, flag: u8) {
        self.pts.x.push(x);
        self.pts.y.push(y);
        self.pts.flags.push(flag);
    }

    // Drawing without a move starts a contour where the pen is, as a move there would.
    fn open(&mut self) {
        if self.contour_start.is_none() {
            self.contour_start = Some(self.pts.len());
            self.push(self.current.0, self.current.1, ON_CURVE);
        }
    }

    fn end_contour(&mut self) {
        let Some(start) = self.contour_start.take() else { return };
        if let (Some(&x), Some(&y)) = (self.pts.x.get(start), self.pts.y.get(start)) {
            self.current = (x, y);
        }
        let last = self.pts.len();
        if last <= start {
            return;
        }
        if last - start > 1
            && self.pts.flags[last - 1] & ON_CURVE != 0
            && self.pts.x[last - 1] == self.pts.x[start]
            && self.pts.y[last - 1] == self.pts.y[start]
        {
            self.pts.x.pop();
            self.pts.y.pop();
            self.pts.flags.pop();
        }
        if self.pts.len() > start {
            self.pts.contour_ends.push(self.pts.len() - 1);
        } else {
            self.pts.x.truncate(start);
            self.pts.y.truncate(start);
            self.pts.flags.truncate(start);
        }
    }
}

impl OutlinePen for CollectPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.end_contour();
        self.contour_start = Some(self.pts.len());
        self.push(x, y, ON_CURVE);
        self.current = (x, y);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.open();
        self.push(x, y, ON_CURVE);
        self.current = (x, y);
    }

    // A curve's points go in together, each list checking its room once.
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.open();
        self.pts.x.extend_from_slice(&[cx, x]);
        self.pts.y.extend_from_slice(&[cy, y]);
        self.pts.flags.extend_from_slice(&[CONIC, ON_CURVE]);
        self.current = (x, y);
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.open();
        self.pts.x.extend_from_slice(&[c1x, c2x, x]);
        self.pts.y.extend_from_slice(&[c1y, c2y, y]);
        self.pts.flags.extend_from_slice(&[CUBIC, CUBIC, ON_CURVE]);
        self.current = (x, y);
    }

    fn close(&mut self) {
        self.end_contour();
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/auto/points.rs"]
mod tests;
