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

impl CollectPen {
    pub fn new() -> CollectPen {
        CollectPen {
            pts: AutoPoints {
                x: Vec::with_capacity(TYPICAL_POINTS),
                y: Vec::with_capacity(TYPICAL_POINTS),
                flags: Vec::with_capacity(TYPICAL_POINTS),
                contour_ends: Vec::with_capacity(TYPICAL_CONTOURS),
            },
            contour_start: None,
            current: (0.0, 0.0),
        }
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

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.open();
        self.push(cx, cy, CONIC);
        self.push(x, y, ON_CURVE);
        self.current = (x, y);
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.open();
        self.push(c1x, c1y, CUBIC);
        self.push(c2x, c2y, CUBIC);
        self.push(x, y, ON_CURVE);
        self.current = (x, y);
    }

    fn close(&mut self) {
        self.end_contour();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // A charstring may draw before it moves, or after it closes: the strays open a contour of their
    // own at the pen rather than joining the next one or none.
    #[test]
    fn drawing_without_a_move_opens_a_contour() {
        let mut pen = CollectPen::new();
        pen.line_to(10.0, 0.0);
        pen.line_to(10.0, 10.0);
        pen.move_to(0.0, 0.0);
        pen.line_to(5.0, 5.0);
        pen.line_to(0.0, 5.0);
        pen.close();
        pen.line_to(20.0, 0.0);
        let pts = pen.finish();
        assert_eq!(pts.contour_ends, [2, 5, 7]);
        assert_eq!((pts.x[6], pts.y[6]), (0.0, 0.0), "the strays after the close start where it closed");
    }
}
