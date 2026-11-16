use crate::daecore::daekit::float::FloatExt;
use alloc::vec::Vec;

fn edges_f64() -> Vec<f64> {
    let mut v = vec![
        0.0, -0.0, 0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 1.0, -1.0, 0.7, -0.7,
        0.499_999_999_999_999_94,
        -0.499_999_999_999_999_94,
        4_503_599_627_370_495.5,
        -4_503_599_627_370_495.5,
        4_503_599_627_370_496.0,
        -4_503_599_627_370_496.0,
        9_007_199_254_740_992.0,
        1e300, -1e300, 1e-300, -1e-300,
        f64::MIN_POSITIVE, -f64::MIN_POSITIVE,
        5e-324, -5e-324,
        f64::MAX, f64::MIN,
        f64::INFINITY, f64::NEG_INFINITY, f64::NAN, -f64::NAN,
    ];
    for i in -40i32..=40 {
        v.push(f64::from(i) * 0.5);
        v.push(f64::from(i) * 0.25);
    }
    v
}

fn edges_f32() -> Vec<f32> {
    let mut v = vec![
        0.0f32, -0.0, 0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 1.0, -1.0,
        0.499_999_97, -0.499_999_97,
        8_388_607.5, -8_388_607.5, 8_388_608.0, -8_388_608.0, 16_777_216.0,
        1e30, -1e30, 1e-30, -1e-30,
        f32::MIN_POSITIVE, -f32::MIN_POSITIVE, 1e-45, -1e-45,
        f32::MAX, f32::MIN, f32::INFINITY, f32::NEG_INFINITY, f32::NAN, -f32::NAN,
    ];
    for i in -40i32..=40 {
        v.push(i as f32 * 0.5);
        v.push(i as f32 * 0.25);
    }
    v
}

fn spread_f64(n: usize) -> Vec<f64> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    (0..n)
        .map(|i| {
            let u = (next() >> 11) as f64 / (1u64 << 53) as f64;
            match i % 5 {
                0 => (u - 0.5) * 2000.0,
                1 => u - 0.5,
                2 => ((u * 2e6) as i64 as f64) + [0.0, 0.5, -0.5, 0.25][i % 4],
                3 => (u - 0.5) * 9.007_199_254_740_992e15,
                _ => (u - 0.5) * 1e-3,
            }
        })
        .collect()
}

fn bit_eq_f64(a: f64, b: f64) -> bool {
    if a.is_nan() && b.is_nan() { return true; }
    a.to_bits() == b.to_bits()
}
fn bit_eq_f32(a: f32, b: f32) -> bool {
    if a.is_nan() && b.is_nan() { return true; }
    a.to_bits() == b.to_bits()
}

macro_rules! check_f64 {
    ($name:literal, $ours:expr, $theirs:expr, $vals:expr) => {{
        let ours = $ours;
        let theirs = $theirs;
        let mut bad = 0usize;
        let mut first = None;
        for &x in $vals {
            let (g, w) = (ours(x), theirs(x));
            if !bit_eq_f64(g, w) {
                bad += 1;
                if first.is_none() {
                    first = Some((x, g, w));
                }
            }
        }
        assert_eq!(
            bad, 0,
            "f64 {} diverged from std on {} of {} values; first x={:?} ({:#018x}) ours={:?} std={:?}",
            $name, bad, $vals.len(),
            first.unwrap().0, first.unwrap().0.to_bits(), first.unwrap().1, first.unwrap().2,
        );
    }};
}

macro_rules! check_f32 {
    ($name:literal, $ours:expr, $theirs:expr, $vals:expr) => {{
        let ours = $ours;
        let theirs = $theirs;
        let mut bad = 0usize;
        let mut first = None;
        for &x in $vals {
            let (g, w) = (ours(x), theirs(x));
            if !bit_eq_f32(g, w) {
                bad += 1;
                if first.is_none() {
                    first = Some((x, g, w));
                }
            }
        }
        assert_eq!(
            bad, 0,
            "f32 {} diverged from std on {} of {} values; first x={:?} ours={:?} std={:?}",
            $name, bad, $vals.len(), first.unwrap().0, first.unwrap().1, first.unwrap().2,
        );
    }};
}

#[test]
fn rounding_matches_std_bit_for_bit_f64() {
    let mut vals = edges_f64();
    vals.extend(spread_f64(300_000));
    check_f64!("round", FloatExt::round, |x: f64| x.round(), &vals);
    check_f64!("round_ties_even", FloatExt::round_ties_even, |x: f64| x.round_ties_even(), &vals);
    check_f64!("floor", FloatExt::floor, |x: f64| x.floor(), &vals);
    check_f64!("ceil", FloatExt::ceil, |x: f64| x.ceil(), &vals);
    check_f64!("trunc", FloatExt::trunc, |x: f64| x.trunc(), &vals);
    check_f64!("abs", FloatExt::abs, |x: f64| x.abs(), &vals);
}

#[test]
fn rounding_matches_std_bit_for_bit_f32() {
    let mut vals = edges_f32();
    vals.extend(spread_f64(300_000).into_iter().map(|v| v as f32));
    check_f32!("round", FloatExt::round, |x: f32| x.round(), &vals);
    check_f32!("round_ties_even", FloatExt::round_ties_even, |x: f32| x.round_ties_even(), &vals);
    check_f32!("floor", FloatExt::floor, |x: f32| x.floor(), &vals);
    check_f32!("ceil", FloatExt::ceil, |x: f32| x.ceil(), &vals);
    check_f32!("trunc", FloatExt::trunc, |x: f32| x.trunc(), &vals);
    check_f32!("abs", FloatExt::abs, |x: f32| x.abs(), &vals);
}

#[test]
fn f32_near_exhaustive_over_the_fractional_range() {
    let mut checked = 0u64;
    let mut bad = 0u64;
    let mut first = None;
    let mut bits = 0u32;
    while bits < 0x4B80_0000 {
        for &b in &[bits, bits | 0x8000_0000] {
            let x = f32::from_bits(b);
            for (name, g, w) in [
                ("round", FloatExt::round(x), x.round()),
                ("round_ties_even", FloatExt::round_ties_even(x), x.round_ties_even()),
                ("floor", FloatExt::floor(x), x.floor()),
                ("ceil", FloatExt::ceil(x), x.ceil()),
                ("trunc", FloatExt::trunc(x), x.trunc()),
            ] {
                checked += 1;
                if !bit_eq_f32(g, w) {
                    bad += 1;
                    if first.is_none() {
                        first = Some((name, x, g, w));
                    }
                }
            }
        }
        bits = bits.wrapping_add(37);
    }
    assert_eq!(bad, 0, "{bad} of {checked} f32 comparisons diverged; first {first:?}");
    assert!(checked > 30_000_000, "only {checked} comparisons ran, the sweep is not covering enough");
}
