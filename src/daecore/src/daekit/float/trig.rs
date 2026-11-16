#[cfg(all(not(feature = "std"), not(test)))]
use super::FloatExt;

// x less its nearest multiple of π/2, as a head and a tail within about π/4, then kernels that take both.
// Within 2^-26 of 0, sin x is x and cos x is 1 to the last bit.
pub fn sin_cos(radians: f64) -> (f64, f64) {
    if !radians.is_finite() {
        return (0.0, 1.0);
    }
    if radians.abs() < f64::from_bits(0x3E50_0000_0000_0000) {
        return (radians, 1.0);
    }
    let (n, x, tail) = rem_pio2(radians);
    quadrant(n, kernel_sin(x, tail), kernel_cos(x, tail))
}

// An angle in degrees, less its whole turns and then its nearest quarter turn, both exactly: every
// multiple of 90° gives exact zeros and ones, as COLR, which states its angles in degrees, means them.
pub fn sin_cos_degrees(degrees: f64) -> (f64, f64) {
    let turn = degrees % 360.0;
    let quarter = (turn / 90.0).round_ties_even();
    let (sin, cos) = sin_cos((turn - quarter * 90.0).to_radians());
    quadrant(quarter as i32, sin, cos)
}

// n quarter turns on: an odd one swaps sine and cosine, and the sign bits follow the half turns. Without
// branches, which a random quadrant would mispredict.
fn quadrant(n: i32, sin: f64, cos: f64) -> (f64, f64) {
    let (a, b) = if n & 1 == 0 { (sin, cos) } else { (cos, sin) };
    let flip = |v: f64, k: i32| f64::from_bits(v.to_bits() ^ (u64::from(k as u32 & 2) << 62));
    (flip(a, n), flip(b, n.wrapping_add(1)))
}

// π/2 in three 33-bit parts and their remainders, so n·part is exact for |n| up to 2^20.
const PIO2_1: f64 = 1.5707963267341256;
const PIO2_1T: f64 = 6.077100506506192e-11;
const PIO2_2: f64 = 6.077100506303966e-11;
const PIO2_2T: f64 = 2.0222662487959506e-21;
const PIO2_3: f64 = 2.0222662487111665e-21;
const PIO2_3T: f64 = 8.4784276603689e-32;

// x as n·π/2 + head + tail. Cody-Waite while n fits 20 bits, a part more each time cancellation eats the
// head's bits; Payne-Hanek past that.
fn rem_pio2(x: f64) -> (i32, f64, f64) {
    if x.abs() <= core::f64::consts::FRAC_PI_4 {
        return (0, x, 0.0);
    }
    if x.abs() >= 1_048_576.0 * core::f64::consts::FRAC_PI_2 {
        return rem_pio2_large(x);
    }
    let exponent = |v: f64| ((v.to_bits() >> 52) & 0x7FF) as i32;
    let toint = 1.5 / f64::EPSILON;
    let n = x * core::f64::consts::FRAC_2_PI + toint - toint;
    let (mut r, mut w) = (x - n * PIO2_1, n * PIO2_1T);
    let mut head = r - w;
    for (part, rest, bits) in [(PIO2_2, PIO2_2T, 16), (PIO2_3, PIO2_3T, 49)] {
        if exponent(x) - exponent(head) <= bits {
            break;
        }
        let t = r;
        w = n * part;
        r = t - w;
        w = n * rest - ((t - r) - w);
        head = r - w;
    }
    (n as i32, head, (r - head) - w)
}

// 2/π's first 1,216 bits; the largest double reads up to bit 1,161.
const TWO_OVER_PI: [u64; 19] = [
    0xA2F9836E4E441529, 0xFC2757D1F534DDC0, 0xDB6295993C439041, 0xFE5163ABDEBBC561,
    0xB7246E3A424DD2E0, 0x06492EEA09D1921C, 0xFE1DEB1CB129A73E, 0xE88235F52EBB4484,
    0xE99C7026B45F7E41, 0x3991D639835339F4, 0x9C845F8BBDF9283B, 0x1FF897FFDE05980F,
    0xEF2F118B5A0A6D1F, 0x6D367ECF27CB09B7, 0x4F463F669E5FEA2D, 0x7527BAC7EBE5F17B,
    0x3D0739F78A5292EA, 0x6BFB5FB11F8D5D08, 0x56033046FC7B6BAB,
];

// π/2 · 2^126.
const PIO2_FIXED: u128 = 0x6487ED5110B4611A62633145C06E0E68;

// Payne-Hanek in integers. With x = m·2^e, bits of 2/π before bit e - 1 make whole multiples of 4 and go;
// the next 192 times m, mod 2^192, are x·2/π mod 4 with 190 fraction bits.
#[cold]
#[inline(never)]
fn rem_pio2_large(x: f64) -> (i32, f64, f64) {
    let bits = x.to_bits();
    let m = u128::from((bits & ((1 << 52) - 1)) | (1 << 52));
    let first = ((bits >> 52) & 0x7FF) as i32 - 1076;
    let [w0, w1, w2] = [first, first + 64, first + 128].map(two_over_pi_bits);
    let lo = m * u128::from(w2);
    let mid = m * u128::from(w1) + (lo >> 64);
    let hi = (m * u128::from(w0) + (mid >> 64)) as u64;
    let (mid, lo) = (mid as u64, lo as u64);
    let fraction = (u128::from((hi << 2) | (mid >> 62)) << 64) | u128::from((mid << 2) | (lo >> 62));
    // A fraction of a half or more rounds up to the next quarter turn, leaving it negative.
    let n = (hi >> 62) as i32 + (fraction >> 127) as i32;
    let f = fraction as i128;
    let r = mul_shr126(f.unsigned_abs(), PIO2_FIXED);
    let scale = f64::from_bits(0x37F0_0000_0000_0000);
    // r is below 2^128 and its rounding to a double an integer, so their difference is exact and small.
    let head = r as f64;
    let tail = (r.wrapping_sub(head as u128) as i128) as f64 * scale;
    let sign = if (f < 0) != x.is_sign_negative() { -1.0 } else { 1.0 };
    (if x.is_sign_negative() { -n } else { n }, sign * head * scale, sign * tail)
}

// Bits k to k + 63 of 2/π, bit 1 its first after the point; bits before it are 0.
fn two_over_pi_bits(k: i32) -> u64 {
    let word = |i: i32| usize::try_from(i).ok().and_then(|i| TWO_OVER_PI.get(i).copied()).unwrap_or(0);
    let (w, s) = ((k - 1).div_euclid(64), (k - 1).rem_euclid(64));
    if s == 0 { word(w) } else { (word(w) << s) | (word(w + 1) >> (64 - s)) }
}

// (a·b) >> 126 for a product below 2^254.
fn mul_shr126(a: u128, b: u128) -> u128 {
    const LOW: u128 = u64::MAX as u128;
    let (a1, a0, b1, b0) = (a >> 64, a & LOW, b >> 64, b & LOW);
    let (cross1, cross0) = (a1 * b0, a0 * b1);
    let low = a0 * b0;
    let mid = (low >> 64) + (cross1 & LOW) + (cross0 & LOW);
    let high = a1 * b1 + (cross1 >> 64) + (cross0 >> 64) + (mid >> 64);
    (high << 2) | (((mid & LOW) << 64 | (low & LOW)) >> 126)
}

// The kernels, for |x| within about π/4 and a tail below half its ulp: without a tail, one rounding fewer.
#[rustfmt::skip]
fn kernel_sin(x: f64, tail: f64) -> f64 {
    const S1: f64 = -0.16666666666666632;
    const S2: f64 = 0.00833333333332249;
    const S3: f64 = -0.0001984126982985795;
    const S4: f64 = 2.7557313707070068e-06;
    const S5: f64 = -2.5050760253406863e-08;
    const S6: f64 = 1.58969099521155e-10;

    let z = x * x;
    let w = z * z;
    let r = S2 + z * (S3 + z * S4) + z * w * (S5 + z * S6);
    let v = z * x;
    if tail == 0.0 {
        x + v * (S1 + z * r)
    } else {
        x - ((z * (0.5 * tail - v * r) - tail) - v * S1)
    }
}

#[rustfmt::skip]
fn kernel_cos(x: f64, tail: f64) -> f64 {
    const C1: f64 = 0.0416666666666666;
    const C2: f64 = -0.001388888888887411;
    const C3: f64 = 2.480158728947673e-05;
    const C4: f64 = -2.7557314351390663e-07;
    const C5: f64 = 2.087572321298175e-09;
    const C6: f64 = -1.1359647557788195e-11;

    let z = x * x;
    let w = z * z;
    let r = z * (C1 + z * (C2 + z * C3)) + w * w * (C4 + z * (C5 + z * C6));
    let hz = 0.5 * z;
    let one_less = 1.0 - hz;
    one_less + (((1.0 - one_less) - hz) + (z * r - x * tail))
}

pub fn atan2(y: f64, x: f64) -> f64 {
    const PI: f64 = core::f64::consts::PI;

    if !(y.is_finite() && x.is_finite() && y != 0.0 && x != 0.0) {
        return atan2_special(y, x);
    }

    let a = atan((y / x).abs());
    match (x > 0.0, y > 0.0) {
        (true, true) => a,
        (true, false) => -a,
        (false, true) => PI - a,
        (false, false) => a - PI,
    }
}

#[cold]
#[inline(never)]
fn atan2_special(y: f64, x: f64) -> f64 {
    const PI: f64 = core::f64::consts::PI;
    const FRAC_PI_2: f64 = core::f64::consts::FRAC_PI_2;
    const FRAC_PI_4: f64 = core::f64::consts::FRAC_PI_4;

    if y.is_nan() || x.is_nan() {
        return f64::NAN;
    }
    if y == 0.0 {
        return if x.is_sign_negative() {
            if y.is_sign_negative() { -PI } else { PI }
        } else {
            y
        };
    }
    if x == 0.0 {
        return if y < 0.0 { -FRAC_PI_2 } else { FRAC_PI_2 };
    }
    if y.is_infinite() {
        let a = if x.is_infinite() {
            if x > 0.0 { FRAC_PI_4 } else { 3.0 * FRAC_PI_4 }
        } else {
            FRAC_PI_2
        };
        return if y < 0.0 { -a } else { a };
    }
    // Only an infinite x is left.
    if x > 0.0 { if y < 0.0 { -0.0 } else { 0.0 } } else if y < 0.0 { -PI } else { PI }
}

#[rustfmt::skip]
fn atan(x: f64) -> f64 {
    const AT: [f64; 11] = [
        0.3333333333333293, -0.19999999999876483, 0.14285714272503466, -0.11111110405462356,
        0.09090887133436507, -0.0769187620504483, 0.06661073137387531, -0.058335701337905735,
        0.049768779946159324, -0.036531572744216916, 0.016285820115365782,
    ];
    const HI: [f64; 4] = [
        0.4636476090008061,
        core::f64::consts::FRAC_PI_4,
        0.982793723247329,
        core::f64::consts::FRAC_PI_2,
    ];
    const LO: [f64; 4] = [
        2.2698777452961687e-17, 3.061616997868383e-17, 1.3903311031230998e-17, 6.123233995736766e-17,
    ];

    let negative = x.is_sign_negative();
    let ax = x.abs();
    if ax >= 1.0e300 {
        let r = HI[3] + LO[3];
        return if negative { -r } else { r };
    }

    let (k, t) = if ax < 0.4375 {
        (-1i32, ax)
    } else if ax < 0.6875 {
        (0, (2.0 * ax - 1.0) / (2.0 + ax))
    } else if ax < 1.1875 {
        (1, (ax - 1.0) / (ax + 1.0))
    } else if ax < 2.4375 {
        (2, (ax - 1.5) / (1.0 + 1.5 * ax))
    } else {
        (3, -1.0 / ax)
    };

    let z = t * t;
    let w = z * z;
    let odd  = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
    let even = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));

    let r = if k < 0 {
        t - t * (odd + even)
    } else {
        let i = k as usize;
        HI[i] - ((t * (odd + even) - LO[i]) - t)
    };
    if negative { -r } else { r }
}
