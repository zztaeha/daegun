use crate::daecore::daekit::float::{atan2, sin_cos, sin_cos_degrees, FloatExt};
use alloc::vec::Vec;

fn spread(n: usize, lo: f64, hi: f64) -> Vec<f64> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let u = (state >> 11) as f64 / (1u64 << 53) as f64;
            lo + u * (hi - lo)
        })
        .collect()
}

fn ulps_f64(a: f64, b: f64) -> u64 {
    a.to_bits().abs_diff(b.to_bits())
}

fn ulps_f32(a: f32, b: f32) -> u32 {
    a.to_bits().abs_diff(b.to_bits())
}

#[test]
fn sqrt_stays_within_an_ulp_of_std() {
    let mut worst64 = (0u64, 0.0f64);
    for (lo, hi) in [(0.0, 1.0), (1.0, 4.0), (0.0, 1e6), (1e-30, 1e-10), (1e10, 1e30), (1e150, 1e300)] {
        for x in spread(20_000, lo, hi) {
            let d = ulps_f64(FloatExt::sqrt(x), x.sqrt());
            if d > worst64.0 {
                worst64 = (d, x);
            }
        }
    }
    assert!(
        worst64.0 <= 1,
        "f64 sqrt drifted {} ulps from std at {:e}, and ext.rs claims within one",
        worst64.0, worst64.1,
    );

    let mut worst32 = (0u32, 0.0f32);
    for (lo, hi) in [(0.0, 1.0), (1.0, 4.0), (0.0, 1e6), (1e-30, 1e-10), (1e10, 1e30)] {
        for x in spread(20_000, lo, hi) {
            let x = x as f32;
            let d = ulps_f32(FloatExt::sqrt(x), x.sqrt());
            if d > worst32.0 {
                worst32 = (d, x);
            }
        }
    }
    assert!(
        worst32.0 <= 1,
        "f32 sqrt drifted {} ulps from std at {:e}, and ext.rs claims within one",
        worst32.0, worst32.1,
    );

    for x in [f64::MIN_POSITIVE / 2.0, f64::MIN_POSITIVE / 1024.0, 5e-324, 1e-310] {
        assert!(
            ulps_f64(FloatExt::sqrt(x), x.sqrt()) <= 1,
            "f64 sqrt of the subnormal {x:e} drifted past an ulp",
        );
    }
    for x in [f32::MIN_POSITIVE / 2.0, f32::MIN_POSITIVE / 512.0, 1e-44f32] {
        assert!(
            ulps_f32(FloatExt::sqrt(x), x.sqrt()) <= 1,
            "f32 sqrt of the subnormal {x:e} drifted past an ulp",
        );
    }
}

#[test]
fn sqrt_answers_the_special_values_exactly_as_std_does() {
    let cases = [0.0f64, -0.0, f64::INFINITY, 1.0, 4.0, 0.25];
    for x in cases {
        assert_eq!(
            FloatExt::sqrt(x).to_bits(), x.sqrt().to_bits(),
            "sqrt({x}) was {} where std gives {}", FloatExt::sqrt(x), x.sqrt(),
        );
    }
    for x in [-1.0f64, -4.0, f64::NEG_INFINITY, f64::NAN] {
        assert!(FloatExt::sqrt(x).is_nan(), "sqrt({x}) was not NaN");
        assert!(x.sqrt().is_nan(), "std's sqrt({x}) was not NaN, so this test is comparing nothing");
    }
}

// Within an ulp of std's wherever the argument lies: near 0, past 2^20·π/2 where the reduction turns
// to Payne-Hanek, at the largest doubles, and beside multiples of π/2, where reduction cancels.
#[test]
fn sine_and_cosine_stay_within_an_ulp_of_std() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut huge = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        f64::from_bits(((1044 + state % 1002) << 52) | (state >> 12))
    };
    let beside = (-10_000..10_000).flat_map(|k| {
        let x = f64::from(k) * core::f64::consts::FRAC_PI_2;
        [x.next_down(), x, x.next_up()]
    });
    let xs =
        spread(200_000, -40.0, 40.0).into_iter().chain(spread(100_000, 40.0, 2e6)).chain((0..100_000).map(|_| huge()));
    for x in xs.chain(beside).chain([f64::MAX, -f64::MAX, 1e300, -1e18]) {
        let (s, c) = sin_cos(x);
        for (got, want, what) in [(s, x.sin(), "sin"), (c, x.cos(), "cos")] {
            assert!(
                got == want || ulps_f64(got, want) <= 1,
                "{what}({x:e} = {:#x}) gave {got:e}, std {want:e}",
                x.to_bits()
            );
        }
    }
    // sin(10^22) = -0.8522008497671888017727..., the classic test of a reduction.
    assert_eq!(sin_cos(1e22).0, -0.8522008497671888);
    for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(sin_cos(x), (0.0, 1.0), "sin_cos({x}) was not the identity");
    }
}

// A whole number of quarter turns in degrees gives exact zeros and ones, however many turns; any other
// angle what its radians give, to within the rounding the conversion to radians carries.
#[test]
fn degrees_are_exact_on_every_quarter_turn() {
    let quarter = [(0.0, 1.0), (1.0, 0.0), (0.0, -1.0), (-1.0, 0.0)];
    for k in -12i32..=12 {
        assert_eq!(sin_cos_degrees(f64::from(k) * 90.0), quarter[k.rem_euclid(4) as usize], "{k} quarter turns");
    }
    assert_eq!(sin_cos_degrees(90.0 * (2f64.powi(47) + 1.0)), quarter[1], "2^47 + 1 quarter turns, exact in a double");
    for d in spread(20_000, -720.0, 720.0) {
        let (s, c) = sin_cos_degrees(d);
        let r = d.to_radians();
        let bound = (r.abs() + 1.0) * f64::EPSILON;
        assert!((s - r.sin()).abs() <= bound && (c - r.cos()).abs() <= bound, "{d}°: ({s}, {c})");
    }
    assert_eq!(sin_cos_degrees(f64::NAN), (0.0, 1.0));
}

#[test]
fn atan2_holds_the_error_its_comment_quotes() {
    let mut worst = (0.0f64, 0.0f64, 0.0f64);
    for y in spread(600, -1000.0, 1000.0) {
        for x in spread(600, -1000.0, 1000.0) {
            let e = (atan2(y, x) - y.atan2(x)).abs();
            if e > worst.0 {
                worst = (e, y, x);
            }
        }
    }
    assert!(
        worst.0 <= 4.0 * f64::EPSILON,
        "atan2 was off by {:e} ({:.2} eps) at ({}, {}); trig.rs quotes 4.4e-16, measured 2.00 eps",
        worst.0, worst.0 / f64::EPSILON, worst.1, worst.2,
    );

    for edge in [0.4375f64, 0.6875, 1.1875, 2.4375] {
        for d in [-1e-12, 0.0, 1e-12] {
            let r = edge + d;
            for (y, x) in [(r, 1.0), (-r, 1.0), (1.0, r), (r, -1.0)] {
                assert!(
                    (atan2(y, x) - y.atan2(x)).abs() <= 4.0 * f64::EPSILON,
                    "atan2({y}, {x}) crossed an interval boundary badly: {} against {}",
                    atan2(y, x), y.atan2(x),
                );
            }
        }
    }
}

#[test]
fn atan2_matches_std_bit_for_bit_on_every_special_pair() {
    let vals = [0.0f64, -0.0, 1.0, -1.0, f64::INFINITY, f64::NEG_INFINITY];
    for &y in &vals {
        for &x in &vals {
            let (got, want) = (atan2(y, x), y.atan2(x));
            assert_eq!(
                got.to_bits(), want.to_bits(),
                "atan2({y}, {x}) gave {got} ({:#x}), std gives {want} ({:#x})",
                got.to_bits(), want.to_bits(),
            );
        }
    }
    for &y in &[f64::NAN, 1.0] {
        for &x in &[f64::NAN, 1.0] {
            if y.is_nan() || x.is_nan() {
                assert!(atan2(y, x).is_nan(), "atan2({y}, {x}) was not NaN");
            }
        }
    }
}
