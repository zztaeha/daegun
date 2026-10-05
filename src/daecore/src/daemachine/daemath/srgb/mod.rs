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
