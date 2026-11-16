use alloc::vec::Vec;
use core::ops::Range;

use super::super::HintedOutline;
use super::dict::{operand, Decimal};
use super::hinter::GLYPH_REACH;
use super::{div_round, round_shift, saturate, ONE};

// A placement past 32,768 times, or an offset past 2^31 units, moves every point past a device's range.
const MOST_SCALE: i64 = 1 << 31;
const MOST_OFFSET: i64 = 1 << 47;
// The 26.6 range the other hinters keep, 2^23 px, so any two points add without overflowing.
const REACH: i128 = 1 << 29;

// FontMatrix (TN 5176, table 9), glyph space to text space: xx, yx, xy, yy, then the offset, as written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FontMatrix([Decimal; 6]);

impl FontMatrix {
    // A DICT's FontMatrix from where its operands lie; None with fewer than six, or one that doesn't read.
    pub(crate) fn read(dict: &[u8], operands: Range<usize>) -> Option<FontMatrix> {
        let (mut m, mut at) = ([Decimal::ONE; 6], operands.start);
        for v in &mut m {
            if at >= operands.end {
                return None;
            }
            let (value, len) = operand(dict, at)?;
            (*v, at) = (value, at + len);
        }
        Some(FontMatrix(m))
    }

    // The matrix in units of its own y scale (|yy|, or |yx| for one that turns y onto x), in 32.32, each value
    // divided exactly; None where that scale is 0.
    fn in_own_units(&self) -> Option<[i64; 6]> {
        let [_, yx, _, yy, ..] = self.0;
        let k = if yy.is_zero() { yx } else { yy }.abs();
        (!k.is_zero()).then(|| self.0.map(|v| v.over(k)))
    }
}

// `first`, then `after`, as 32.32 matrices: a CIDFont's FD matrix, then the Top DICT's, as CIDFontType 0 concatenates
// them.
fn then(first: [i64; 6], after: [i64; 6]) -> [i64; 6] {
    let ([a, b, c, d, e, f], [p, q, r, s, t, u]) = (first, after);
    let sum = |x: i64, y: i64, z: i64, w: i64, add: i64| {
        let product = i128::from(x) * i128::from(y) + i128::from(z) * i128::from(w);
        saturate(round_shift(product, 32) + i128::from(add))
    };
    let (xx, yx, xy, yy) = (sum(p, a, r, b, 0), sum(q, a, s, b, 0), sum(p, c, r, d, 0), sum(q, c, s, d, 0));
    [xx, yx, xy, yy, sum(p, e, r, f, t), sum(q, e, s, f, u)]
}

// How a glyph's outline is placed in the unit it's drawn in: `matrix` (16.16) then `offset` (16.16 font units), and
// the matrix's y scale, which the hinter hints at so the matrix leaves hinted edges on their pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    matrix: [i64; 4],
    offset: [i64; 2],
    y_scale: i64,
}

const IDENTITY: Placement = Placement { matrix: [ONE, 0, 0, ONE], offset: [0, 0], y_scale: ONE };

impl Placement {
    // A 32.32 matrix in the units outlines are drawn in; None where it's singular, or scales or moves past what a
    // device holds.
    fn new(m: [i64; 6]) -> Option<Placement> {
        let [xx, yx, xy, yy, x, y] = m.map(|v| saturate(round_shift(i128::from(v), 16)));
        let scale_ok = [xx, yx, xy, yy].iter().all(|v| v.abs() <= MOST_SCALE);
        if !scale_ok || i128::from(xx) * i128::from(yy) == i128::from(xy) * i128::from(yx) {
            return None;
        }
        if x.abs() > MOST_OFFSET || y.abs() > MOST_OFFSET {
            return None;
        }
        let y_scale = if yy != 0 { yy } else { yx }.abs();
        Some(Placement { matrix: [xx, yx, xy, yy], offset: [x, y], y_scale })
    }

    // Each FD's placement, or the Top DICT's alone without FDs; None where it changes nothing. Outlines are drawn in
    // the Top DICT's units, or head's em where it states no usable FontMatrix, as a CIDFont often doesn't while its
    // FDs do; an FD whose matrix is missing or unusable takes the Top DICT's placement.
    pub(crate) fn of(top: Option<FontMatrix>, fds: Option<&[Option<FontMatrix>]>, em: u16) -> Vec<Option<Placement>> {
        let top = top.and_then(|t| {
            let m = t.in_own_units()?;
            Some((m, Placement::new(m)?))
        });
        let base = top.map_or(IDENTITY, |(_, p)| p);
        let em = i64::from(em.max(1)) << 32;
        let unit = top.map_or([em, 0, 0, em, 0, 0], |(m, _)| m);
        let kept = |p: Placement| (p != IDENTITY).then_some(p);
        let Some(fds) = fds else { return alloc::vec![kept(base)] };
        let fd = |fd: FontMatrix| Placement::new(then(fd.0.map(Decimal::fixed), unit));
        fds.iter().map(|&m| kept(m.and_then(fd).unwrap_or(base))).collect()
    }

    // The scale the hinter hints at: the size's, 26.6 per font unit in 16.16, times the matrix's y scale.
    pub(crate) fn hint_scale(&self, scale16: i64) -> i64 {
        saturate(round_shift(i128::from(scale16) * i128::from(self.y_scale), 16))
    }

    // A y in the units outlines are drawn in, back in glyph space and held within ±2^31 units as drawn points are;
    // None where drawn y doesn't follow glyph y alone, or flips it.
    pub(crate) fn glyph_y(&self, y: i64) -> Option<i64> {
        let [_, yx, _, yy] = self.matrix;
        let unplaced = (i128::from(y) - i128::from(self.offset[1])) << 16;
        (yx == 0 && yy > 0).then(|| {
            saturate(div_round(unplaced, i128::from(yy))).clamp(-GLYPH_REACH, GLYPH_REACH)
        })
    }

    // A drawing pen's transform in font units: the matrix, then the offset.
    pub(crate) fn pen_transform(&self) -> [f64; 6] {
        let [xx, yx, xy, yy] = self.matrix.map(|v| v as f64 / ONE as f64);
        [xx, yx, xy, yy, self.offset[0] as f64 / ONE as f64, self.offset[1] as f64 / ONE as f64]
    }

    // An outline hinted at `hint_scale`, through the matrix with its y scale taken out, then moved by the offset at
    // `scale16`; None where a point leaves the 26.6 range.
    pub(crate) fn place(&self, mut out: HintedOutline, scale16: i64) -> Option<HintedOutline> {
        let [xx, yx, xy, yy] = self.matrix.map(|v| div_round(i128::from(v) << 16, i128::from(self.y_scale)));
        let [ox, oy] = self.offset.map(|o| round_shift(i128::from(o) * i128::from(scale16), 32));
        let fits = |v: i128| (-REACH..=REACH).contains(&v).then_some(v as i32);
        for (x, y) in out.x.iter_mut().zip(&mut out.y) {
            let (px, py) = (i128::from(*x), i128::from(*y));
            *x = fits(round_shift(px * xx + py * xy, 16) + ox)?;
            *y = fits(round_shift(px * yx + py * yy, 16) + oy)?;
        }
        Some(out)
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/cff/placement.rs"]
mod tests;
