// sRGB levels to linear light and back, from tables `scripts/tools/gen/srgb-tables.py` writes, since
// `no_std` has no `pow`. Both directions match the exact curve rounded to the nearest level.
mod tables;

use tables::{LINEAR, STEP, THRESHOLD};

// 2^-12 as f32 bits. Below it only levels 0 and 1 occur, and above it each bucket of 2^15 values
// crosses at most one threshold, which the generator checks.
const LOW: u32 = 0x3980_0000;

#[inline]
pub fn to_linear(level: u8) -> f32 {
    f32::from_bits(LINEAR[usize::from(level)])
}

// The level a bucket of the input's top bits starts at, and one threshold to say whether the input
// has passed the next. NaN and anything below zero come out 0.
#[allow(clippy::neg_cmp_op_on_partial_ord, reason = "the negated form is what sends NaN to level 0")]
#[inline]
pub fn from_linear(v: f32) -> u8 {
    if !(v >= f32::from_bits(LOW)) {
        return u8::from(v >= f32::from_bits(THRESHOLD[1]));
    }
    if v >= 1.0 {
        return 255;
    }
    let k = STEP[((v.to_bits() - LOW) >> 15) as usize];
    k + u8::from(v >= f32::from_bits(THRESHOLD[usize::from(k) + 1]))
}

// A linear light's level, between whole levels where it falls between them, so a table built on it has no steps
// of its own.
pub(crate) fn encoded(l: f32) -> f32 {
    let k = from_linear(l);
    let at = to_linear(k);
    if l >= at {
        match k.checked_add(1) {
            Some(n) => f32::from(k) + (l - at) / (to_linear(n) - at),
            None => 255.0,
        }
    } else {
        match k.checked_sub(1) {
            Some(p) => f32::from(k) - (at - l) / (at - to_linear(p)),
            None => 0.0,
        }
    }
}

/// Each coverage level made the alpha whose plain sRGB blend of a text level `text` over its opposite lands where
/// coverage would in linear light, coverage first thickened by `contrast`, 0 to 1, as the backdrop is lighter.
pub(crate) fn coverage_table(text: u8, contrast: f32) -> [u8; 256] {
    let back = 255 - text;
    let (lt, lb) = (to_linear(text), to_linear(back));
    let k = if contrast > 0.0 { contrast.min(1.0) * lb } else { 0.0 };
    let (t, b) = (f32::from(text), f32::from(back));
    core::array::from_fn(|i| {
        let a = i as f32 / 255.0;
        let a = a + k * a * (1.0 - a);
        let out = encoded(lt * a + lb * (1.0 - a));
        (((out - b) / (t - b)).clamp(0.0, 1.0) * 255.0 + 0.5) as u8
    })
}

/// The documented stem-darkening curve: a stem `stem` pixels wide widens by this many pixels, 0.4 up to half a
/// pixel, 0.275 from one to five thirds, none from seven thirds on, straight between.
pub(crate) fn darkening(stem: f32) -> f32 {
    const CURVE: [(f32, f32); 4] = [(0.5, 0.4), (1.0, 0.275), (5.0 / 3.0, 0.275), (7.0 / 3.0, 0.0)];
    if stem.is_nan() {
        return 0.0;
    }
    if stem <= CURVE[0].0 {
        return CURVE[0].1;
    }
    for w in CURVE.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if stem < x1 {
            return y0 + (y1 - y0) * (stem - x0) / (x1 - x0);
        }
    }
    0.0
}

#[cfg(test)]
#[path = "../../../../tests/unit/daekit/draw/srgb.rs"]
mod tests;
