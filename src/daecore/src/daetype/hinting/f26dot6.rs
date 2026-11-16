#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;

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

    #[cfg(test)]
    pub fn px(self) -> f32 {
        self.0 as f32 / ONE as f32
    }

    pub(crate) fn units(self) -> i32 {
        self.0
    }

    // The scale, font units to 26.6 in 16.16. Wider than i32: a small em at the largest sizes passes 2^31.
    pub(crate) fn scale16(self, upm: u16) -> i64 {
        if upm == 0 { return 0; }
        (i64::from(self.0) * 0x10000 + i64::from(upm) / 2) / i64::from(upm)
    }
}

// A 26.6 font-unit value at a scale from `Size::scale16`, then rounded to the 64th, as points at a location scale. A
// product past i64 saturates, which the clamp would pin all the same.
pub(crate) fn scale_f26dot6(value: i32, scale16: i64) -> i32 {
    let p = i64::from(value).saturating_mul(scale16).clamp(-WIDE, WIDE);
    ((shift_round(p, 16) + i64::from(HALF)) >> 6).clamp(-SCALE_LIMIT, SCALE_LIMIT) as i32
}

// As `scale_units`, for a value `fits_scale` passed: its product reaches 2^45 at most, so it needs no clamp.
pub(crate) fn scale_fitting(value: i32, scale16: i64) -> i32 {
    shift_round(i64::from(value) * scale16, 16) as i32
}

// A font-unit value at a scale from `Size::scale16`, in 26.6 to the nearest.
pub(crate) fn scale_units(value: i32, scale16: i64) -> i32 {
    let p = i64::from(value).saturating_mul(scale16).clamp(-WIDE, WIDE);
    shift_round(p, 16).clamp(-SCALE_LIMIT, SCALE_LIMIT) as i32
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
// collapses, so such a glyph is left unhinted, as is one with a NaN, which f64::max would pass over.
pub fn fits(coords: impl IntoIterator<Item = f64>, size: Size, upm: u16) -> bool {
    let mut reach = 0.0f64;
    for v in coords {
        if v.is_nan() { return false; }
        reach = reach.max(v.abs());
    }
    upm != 0 && reach * f64::from(size.0) / f64::from(upm) < SCALE_LIMIT as f64
}

// The same for font-unit coordinates at a scale from `Size::scale16`, as the bytecode hinter scales them.
pub(crate) fn fits_scale(coords: impl Iterator<Item = i32>, scale16: i64) -> bool {
    let reach = coords.map(|v| i64::from(v).abs()).max().unwrap_or(0);
    (i128::from(reach) * i128::from(scale16)) >> 16 < i128::from(SCALE_LIMIT)
}


// A font-unit value with a fraction, as a CFF2 instance writes it, scaled and rounded once.
pub fn scale_f32(value: f32, size: Size, upm: u16) -> i32 {
    if upm == 0 { return 0; }
    let v = (f64::from(value) * f64::from(size.0) / f64::from(upm)).round();
    v.clamp(-(SCALE_LIMIT as f64), SCALE_LIMIT as f64) as i32
}

// Each of `values` as `scale_f32` scales it, onto `out`: the em checked once, so the loop vectorizes.
pub(crate) fn scale_f32_into(values: &[f32], size: Size, upm: u16, out: &mut alloc::vec::Vec<i32>) {
    if upm == 0 {
        return out.extend(values.iter().map(|_| 0));
    }
    let (s, u, limit) = (f64::from(size.0), f64::from(upm), SCALE_LIMIT as f64);
    out.extend(values.iter().map(|&v| (f64::from(v) * s / u).round().clamp(-limit, limit) as i32));
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

// To the nearest and the next whole pixel; past the last whole pixel i32 holds, to that pixel.
pub(crate) fn round_pixel(v: i32) -> i32 {
    floor_pixel(v.saturating_add(HALF))
}

pub(crate) fn ceil_pixel(v: i32) -> i32 {
    floor_pixel(v.saturating_add(ONE - 1))
}

// Hinted points depend on every product and quotient rounding alike: each below rounds once, halves away from 0, so
// a distance and its negation land alike, and saturates to i32.

// What `shift_round` takes: a saturated product pinned here rounds to past any limit all the same.
const WIDE: i64 = 1 << 62;

// v / 2^bits to the nearest: half a step added on v's own side, then divided toward 0.
fn shift_round(v: i64, bits: u32) -> i64 {
    let half = 1 << (bits - 1);
    (if v < 0 { v - half } else { v + half }) / (1 << bits)
}

// v times a 16.16 factor.
pub(crate) fn times_16_16(v: i32, factor: i32) -> i32 {
    clamp_i32(shift_round(i64::from(v) * i64::from(factor), 16))
}

// v times a 2.14 factor, a unit vector's component.
pub(crate) fn times_2_14(v: i32, factor: i32) -> i32 {
    clamp_i32(shift_round(i64::from(v) * i64::from(factor), 14))
}

// (dx, dy) projected onto the 2.14 unit vector (x, y).
pub(crate) fn dot_2_14(dx: i64, dy: i64, x: i32, y: i32) -> i32 {
    let sum = dx.saturating_mul(x.into()).saturating_add(dy.saturating_mul(y.into()));
    clamp_i32(shift_round(sum.clamp(-WIDE, WIDE), 14))
}

// a * b / c to the nearest.
pub(crate) fn ratio(a: i32, b: i32, c: i32) -> i32 {
    quotient(i64::from(a) * i64::from(b), c, true)
}

// a * b / c, cut toward 0.
pub(crate) fn ratio_truncated(a: i32, b: i32, c: i32) -> i32 {
    quotient(i64::from(a) * i64::from(b), c, false)
}

// n / d, exact in 64 bits as n, two i32 multiplied, stays within 2^62. Over 0, n saturates with its sign; 0 stays 0.
fn quotient(n: i64, d: i32, nearest: bool) -> i32 {
    let negative = (n < 0) != (d < 0);
    let (n, d) = (n.unsigned_abs(), u64::from(d.unsigned_abs()));
    let q = match d {
        0 if n == 0 => 0,
        0 => u64::MAX,
        _ if nearest => (2 * n + d) / (2 * d),
        _ => n / d,
    };
    let q = i64::try_from(q).unwrap_or(i64::MAX);
    clamp_i32(if negative { -q } else { q })
}

// The unit vector along (dx, dy) in 2.14, each component exactly rounded (halves away from 0); None for (0, 0). For
// a component v of a length L that's (1 + isqrt(⌊2^30 v² / L²⌋)) / 2 floored, in 128 bits for i32 differences.
pub(crate) fn unit_vector(dx: i64, dy: i64) -> Option<(i32, i32)> {
    let (x, y) = (u128::from(dx.unsigned_abs()), u128::from(dy.unsigned_abs()));
    let length2 = x * x + y * y;
    if length2 == 0 {
        return None;
    }
    let component = |v: u128, negative: bool| {
        let c = ((((v * v) << 30) / length2).isqrt() as i32 + 1) >> 1;
        if negative { -c } else { c }
    };
    Some((component(x, dx < 0), component(y, dy < 0)))
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/hinting/f26dot6.rs"]
mod tests;
