#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;

pub(crate) const ONE: i32 = 64;
pub(crate) const HALF: i32 = 32;

pub(crate) const F2DOT14_ONE: i32 = 0x4000;

// The size a glyph is hinted at, in 26.6 pixels per em. Hinting scales by it exactly; MPPEM reads it
// in whole pixels and MPS as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size(i32);

impl Size {
    // None where the whole-pixel size is outside 1 to 65,535 ppem, which MPPEM has to hold.
    pub fn from_px(px: f32) -> Option<Size> {
        const MAX: i32 = u16::MAX as i32 * ONE + HALF - 1;
        let size = (px * ONE as f32).round();
        (HALF as f32..=MAX as f32).contains(&size).then_some(Size(size as i32))
    }

    pub fn ppem(self) -> u16 {
        ((self.0 + HALF) / ONE) as u16
    }

    pub fn px(self) -> f32 {
        self.0 as f32 / ONE as f32
    }

    pub(crate) fn units(self) -> i32 {
        self.0
    }

    // FreeType's scale, font units to 26.6 in 16.16. Wider than i32: a small em at the largest
    // sizes passes 2^31.
    pub(crate) fn scale16(self, upm: u16) -> i64 {
        if upm == 0 { return 0; }
        (i64::from(self.0) * 0x10000 + i64::from(upm) / 2) / i64::from(upm)
    }
}

// A font-unit value at a scale from `Size::scale16`, rounded as FT_MulFix rounds.
pub(crate) fn scale_fix(value: i32, scale16: i64) -> i32 {
    let p = i128::from(value) * i128::from(scale16);
    let r = (p + 0x8000 + if p < 0 { -1 } else { 0 }) >> 16;
    r.clamp(-i128::from(SCALE_LIMIT), i128::from(SCALE_LIMIT)) as i32
}

impl From<u16> for Size {
    fn from(ppem: u16) -> Size {
        Size(i32::from(ppem) * ONE)
    }
}

// Pinned at 2^23 pixels, a quarter of what 26.6 holds, so the hinters can add or subtract any two
// scaled values without overflowing. Only an em under 256 units, at the largest sizes, gets near it.
const SCALE_LIMIT: i64 = 1 << 29;

// Whether a glyph's coordinates scale inside SCALE_LIMIT. Past it `scale` pins them and the glyph
// collapses, so such a glyph is left unhinted.
pub fn fits(coords: impl IntoIterator<Item = f64>, size: Size, upm: u16) -> bool {
    let reach = coords.into_iter().fold(0.0f64, |m, v| m.max(v.abs()));
    upm != 0 && reach * f64::from(size.0) / f64::from(upm) < SCALE_LIMIT as f64
}

pub fn scale(value: i32, size: Size, upm: u16) -> i32 {
    if upm == 0 { return 0; }
    let num = value as i64 * size.0 as i64;
    let den = upm as i64;
    let half = den / 2;
    let adjusted = if num >= 0 { num + half } else { num - half };
    (adjusted / den).clamp(-SCALE_LIMIT, SCALE_LIMIT) as i32
}

// A font-unit value with a fraction, as a CFF2 instance writes it, scaled and rounded once.
pub fn scale_f32(value: f32, size: Size, upm: u16) -> i32 {
    if upm == 0 { return 0; }
    let v = (f64::from(value) * f64::from(size.0) / f64::from(upm)).round();
    v.clamp(-(SCALE_LIMIT as f64), SCALE_LIMIT as f64) as i32
}

// Clamped rather than cast: `as i32` truncates the high bits, so a coordinate past the format
// re-enters it as a small in-range value rather than being pinned at the rail.
pub(crate) fn clamp_i32(v: i64) -> i32 {
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

pub(crate) fn round_to_grid(v: i32) -> i32 {
    if v >= 0 {
        floor_pixel(v.saturating_add(HALF))
    } else {
        -floor_pixel(v.saturating_neg().saturating_add(HALF))
    }
}

pub(crate) fn floor_pixel(v: i32) -> i32 {
    v & !(ONE - 1)
}

pub(crate) fn ceil_pixel(v: i32) -> i32 {
    v.saturating_add(ONE - 1) & !(ONE - 1)
}

// The bytecode interpreter matches FreeType to the 64th, so each helper below rounds as its FreeType
// namesake does: worked in 64 bits, ties away from zero, pinned back to i32.

// FT_MulFix: a * b / 2^16.
pub(crate) fn mul_fix(a: i32, b: i32) -> i32 {
    let ab = i64::from(a) * i64::from(b);
    clamp_i32((ab + 0x8000 + (ab >> 63)) >> 16)
}

// TT_MulFix14: a * b / 2^14, b a 2.14 vector component.
pub(crate) fn mul_fix14(a: i32, b: i32) -> i32 {
    let ab = i64::from(a) * i64::from(b);
    clamp_i32((ab + 0x2000 + (ab >> 63)) >> 14)
}

// TT_DotFix14: (ax, ay) . (bx, by) / 2^14, the difference already taken in 64 bits.
pub(crate) fn dot_fix14(ax: i64, ay: i64, bx: i32, by: i32) -> i32 {
    let c = ax.saturating_mul(i64::from(bx)).saturating_add(ay.saturating_mul(i64::from(by)));
    clamp_i32((c.saturating_add(0x2000) + (c >> 63)) >> 14)
}

// FT_MulDiv and FT_MulDiv_No_Round: on magnitudes with the sign put back; a zero divisor gives
// 0x7FFFFFFF, as FreeType's does.
pub(crate) fn mul_div(a: i32, b: i32, c: i32) -> i32 {
    mul_div_by(a, b, c, true)
}

pub(crate) fn mul_div_no_round(a: i32, b: i32, c: i32) -> i32 {
    mul_div_by(a, b, c, false)
}

fn mul_div_by(a: i32, b: i32, c: i32, round: bool) -> i32 {
    let negative = (a < 0) ^ (b < 0) ^ (c < 0);
    let (a, b, c) = (u128::from(a.unsigned_abs()), u128::from(b.unsigned_abs()), u128::from(c.unsigned_abs()));
    let d = (a * b + if round { c / 2 } else { 0 }).checked_div(c).unwrap_or(0x7FFF_FFFF);
    let d = d.min(i64::MAX as u128) as i64;
    clamp_i32(if negative { -d } else { d })
}

// FT_DivFix: a / b in 16.16.
pub(crate) fn div_fix(a: i32, b: i32) -> i32 {
    mul_div_by(a, 0x10000, b, true)
}

// FT_Vector_NormLen's unit vector, then cut to 2.14 as SPVTL and its kin cut it. None for (0, 0),
// which FreeType leaves unnormalized.
pub(crate) fn normalize(x: i32, y: i32) -> Option<(i32, i32)> {
    if x == 0 && y == 0 {
        return None;
    }
    let (mut ux, mut uy) = (x.unsigned_abs(), y.unsigned_abs());
    let sign = |v: i32, negative: bool| if negative { -v } else { v };
    if ux == 0 {
        return Some((0, sign(0x4000, y < 0)));
    }
    if uy == 0 {
        return Some((sign(0x4000, x < 0), 0));
    }
    // Prenormalized by a shift so the estimated length lies between 2/3 and 4/3, then refined by
    // Newton's iterations on the reciprocal length.
    let estimate = |ux: u32, uy: u32| if ux > uy { ux + (uy >> 1) } else { uy + (ux >> 1) };
    let mut l = estimate(ux, uy);
    let mut shift = l.leading_zeros() as i32;
    shift -= 15 + i32::from(l >= (0xAAAA_AAAAu32 >> shift));
    if shift > 0 {
        ux <<= shift;
        uy <<= shift;
        l = estimate(ux, uy);
    } else {
        ux >>= -shift;
        uy >>= -shift;
        l >>= -shift;
    }
    let mut b = 0x10000 - l as i32;
    let (x0, y0) = (ux as i32, uy as i32);
    let (mut u, mut v);
    loop {
        u = (x0 + ((i64::from(x0) * i64::from(b)) >> 16) as i32) as u32;
        v = (y0 + ((i64::from(y0) * i64::from(b)) >> 16) as i32) as u32;
        let z = i64::from((u.wrapping_mul(u).wrapping_add(v.wrapping_mul(v)) as i32).wrapping_neg()) / 0x200;
        let z = (z * i64::from((0x10000 + b) >> 8) / 0x10000) as i32;
        b += z;
        if z <= 0 {
            break;
        }
    }
    Some((sign(u as i32, x < 0) / 4, sign(v as i32, y < 0) / 4))
}

#[cfg(test)]
mod tests {
    use super::*;

    // MPPEM has to hold the whole-pixel size, so the limits are 1 and 65,535 after rounding.
    #[test]
    fn a_size_is_refused_where_its_whole_pixels_leave_mppem_range() {
        for px in [0.49, 65_535.5, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(Size::from_px(px), None, "{px} px was accepted");
        }
        for (px, ppem) in [(0.5, 1), (16.4, 16), (16.5, 17), (29.7, 30), (65_535.49, 65_535)] {
            assert_eq!(Size::from_px(px).map(Size::ppem), Some(ppem), "{px} px");
        }
        assert_eq!(Size::from_px(16.4).map(Size::px), Some(1050.0 / 64.0), "16.4 px is not 1050 in 26.6");
    }
}
