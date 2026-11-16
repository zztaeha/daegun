// Sine, cosine and tangent of angles in 16.16 degrees, worked in integers so a VARC rotation or skew turns out the same
// to the unit on every platform: sine and cosine in Q60 to 2^-50, the tangent in Q44.

// Half a turn, in 16.16 degrees.
pub(crate) const PI: i64 = 180 << 16;
const QUARTER: i64 = 90 << 16;
const EIGHTH: i64 = 45 << 16;
// The kernels' fixed point, 2^60, and π in it.
pub(crate) const Q: u32 = 60;
const ONE_Q: i128 = 1 << Q;
const PI_Q: i128 = 3_622_009_729_038_561_421;

fn mul_q(a: i128, b: i128) -> i128 {
    (a * b + (1 << (Q - 1))) >> Q
}

// sin and cos of `r`, 0 to 45 degrees, in Q60 by their Taylor series, each run until a term falls under 2^-60.
fn kernel(r: i64) -> (i128, i128) {
    let x = (i128::from(r) * PI_Q + i128::from(PI / 2)) / i128::from(PI);
    let x2 = mul_q(x, x);
    let (mut sin, mut term) = (x, x);
    for k in (2..).step_by(2) {
        term = -mul_q(term, x2) / (k * (k + 1));
        if term == 0 {
            break;
        }
        sin += term;
    }
    let (mut cos, mut term) = (ONE_Q, ONE_Q);
    for k in (1..).step_by(2) {
        term = -mul_q(term, x2) / (k * (k + 1));
        if term == 0 {
            break;
        }
        cos += term;
    }
    (sin, cos)
}

// sin and cos of `angle` in Q60, whole turns and quarter turns taken off exactly, so each multiple of 90° is exact.
pub(crate) fn sin_cos(angle: i64) -> (i128, i128) {
    let a = angle.rem_euclid(4 * QUARTER);
    let r = a % QUARTER;
    let (s, c) = if r <= EIGHTH { kernel(r) } else { let (s, c) = kernel(QUARTER - r); (c, s) };
    match a / QUARTER {
        0 => (s, c),
        1 => (c, -s),
        2 => (-s, -c),
        _ => (-c, s),
    }
}

// `v` shifted right by `by`, half a unit rounded away from 0.
pub(crate) fn round_shift(v: i128, by: u32) -> i128 {
    let m = ((v.unsigned_abs() + (1 << (by - 1))) >> by) as i128;
    if v < 0 { -m } else { m }
}

// The tangent's rail in Q44, 16.16's largest value: a skew past it sends a glyph out of 16.16's range anyway.
pub(crate) const TAN_RAIL: i128 = 0x7FFF_FFFF << 28;

// The tangent in Q44, so its product with a Q60 sine fits 128 bits, held to the rail, as at ±90°.
pub(crate) fn tan(angle: i64) -> i128 {
    let (s, c) = sin_cos(angle);
    let (num, den) = (s.unsigned_abs() << 44, c.unsigned_abs());
    let m = (num + den / 2).checked_div(den).map_or(TAN_RAIL, |q| (q as i128).min(TAN_RAIL));
    if (s < 0) != (c < 0) { -m } else { m }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/varc/trig.rs"]
mod tests;
