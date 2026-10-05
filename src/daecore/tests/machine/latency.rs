use std::time::{Duration, Instant};

use daegun::daecore::daemachine::float::FloatExt;

fn inputs_f64(n: usize, lo: f64, hi: f64) -> Vec<f64> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    (0..n)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let unit = (state >> 11) as f64 / (1u64 << 53) as f64;
            lo + unit * (hi - lo)
        })
        .collect()
}

fn inputs_f32(n: usize, lo: f32, hi: f32) -> Vec<f32> {
    inputs_f64(n, lo as f64, hi as f64).into_iter().map(|v| v as f32).collect()
}

const N: usize = 4096;
const ROUNDS: usize = 40;
const WARMUP: usize = 60;

fn pass<T: Copy>(input: &[T], f: impl Fn(T) -> f64) -> (Duration, f64) {
    let t = Instant::now();
    let mut acc = [0.0f64; 4];
    for chunk in input.as_chunks::<4>().0 {
        acc[0] += f(chunk[0]);
        acc[1] += f(chunk[1]);
        acc[2] += f(chunk[2]);
        acc[3] += f(chunk[3]);
    }
    let e = t.elapsed();
    core::hint::black_box(&acc);
    (e, acc[0] + acc[1] + acc[2] + acc[3])
}

fn duel<T: Copy>(
    label: &str,
    input: &[T],
    ours: impl Fn(T) -> f64,
    std_: impl Fn(T) -> f64,
) {
    for _ in 0..20 {
        core::hint::black_box(pass(input, &ours));
        core::hint::black_box(pass(input, &std_));
    }

    let (mut a, mut b) = (Vec::with_capacity(ROUNDS), Vec::with_capacity(ROUNDS));
    let (mut sa, mut sb) = (0.0f64, 0.0f64);
    for _ in 0..ROUNDS {
        let (ta, va) = pass(input, &ours);
        let (tb, vb) = pass(input, &std_);
        a.push(ta);
        b.push(tb);
        sa += va;
        sb += vb;
    }
    assert!(sa.is_finite() && sb.is_finite(), "{label}: a pass produced nothing usable");

    a.sort();
    b.sort();
    let (ma, mb) = (a[ROUNDS / 2], b[ROUNDS / 2]);
    let (na, nb) = (
        ma.as_secs_f64() * 1e9 / input.len() as f64,
        mb.as_secs_f64() * 1e9 / input.len() as f64,
    );
    eprintln!(
        "  {label:22}  daemachine {na:>6.3} ns/op   std {nb:>6.3} ns/op   {:>5.2}x",
        na / nb.max(1e-12),
    );
}

fn ratio<T: Copy>(
    label: &str,
    input: &[T],
    a_name: &str,
    a: impl Fn(T) -> f64,
    b_name: &str,
    b: impl Fn(T) -> f64,
) {
    for _ in 0..WARMUP {
        core::hint::black_box(pass(input, &a));
        core::hint::black_box(pass(input, &b));
    }
    let (mut ta, mut tb) = (Vec::with_capacity(ROUNDS), Vec::with_capacity(ROUNDS));
    let (mut sa, mut sb) = (0.0f64, 0.0f64);
    for _ in 0..ROUNDS {
        let (x, va) = pass(input, &a);
        let (y, vb) = pass(input, &b);
        ta.push(x);
        tb.push(y);
        sa += va;
        sb += vb;
    }
    assert!(sa.is_finite() && sb.is_finite(), "{label}: a pass produced nothing usable");
    ta.sort();
    tb.sort();
    let n = |d: Duration| d.as_secs_f64() * 1e9 / input.len() as f64;
    let (na, nb) = (n(ta[ROUNDS / 2]), n(tb[ROUNDS / 2]));
    eprintln!("  {label:22}  {a_name} {na:>6.3} ns/px   {b_name} {nb:>6.3} ns/px   {:>5.2}x", na / nb.max(1e-12));
}

#[test]
#[ignore]
fn float_ext_against_std() {
    let a64 = inputs_f64(N, -1000.0, 1000.0);
    let a32 = inputs_f32(N, -1000.0, 1000.0);

    eprintln!("float_ext_against_std  (ratio > 1 means daemachine is slower)");
    duel("f64 round", &a64, FloatExt::round, |v: f64| v.round());
    duel("f64 floor", &a64, FloatExt::floor, |v: f64| v.floor());
    duel("f64 ceil", &a64, FloatExt::ceil, |v: f64| v.ceil());
    duel("f64 trunc", &a64, FloatExt::trunc, |v: f64| v.trunc());
    duel("f64 abs", &a64, FloatExt::abs, |v: f64| v.abs());
    duel("f64 round_ties_even", &a64, FloatExt::round_ties_even, |v: f64| v.round_ties_even());
    duel("f32 round", &a32, |v| FloatExt::round(v) as f64, |v: f32| v.round() as f64);
    duel("f32 floor", &a32, |v| FloatExt::floor(v) as f64, |v: f32| v.floor() as f64);
    duel("f32 abs", &a32, |v| FloatExt::abs(v) as f64, |v: f32| v.abs() as f64);
    duel("f32 round_ties_even", &a32, |v| FloatExt::round_ties_even(v) as f64, |v: f32| v.round_ties_even() as f64);
}

#[test]
#[ignore]
fn sqrt_against_std() {
    let a64 = inputs_f64(N, 0.0, 1_000_000.0);
    let a32 = inputs_f32(N, 0.0, 1_000_000.0);

    eprintln!("sqrt_against_std  (Newton iteration against one hardware instruction)");
    duel("f64 sqrt", &a64, FloatExt::sqrt, |v: f64| v.sqrt());
    duel("f32 sqrt", &a32, |v| FloatExt::sqrt(v) as f64, |v: f32| v.sqrt() as f64);
}

#[test]
#[ignore]
fn trig_against_std() {
    let ang = inputs_f64(N, -40.0, 40.0);

    eprintln!("trig_against_std");
    duel(
        "f64 sin_cos",
        &ang,
        |v| { let (s, c) = daegun::daecore::daemachine::float::sin_cos(v); s + c },
        |v: f64| { let (s, c) = v.sin_cos(); s + c },
    );
    duel(
        "f64 atan2",
        &ang,
        |v| daegun::daecore::daemachine::float::atan2(v, 1.7),
        |v: f64| v.atan2(1.7),
    );
}

#[test]
#[ignore]
fn daemath_baseline() {
    use daegun::daecore::daemachine::daemath::blend::{blend, composite, Rgb};
    use daegun::daecore::daemachine::daemath::gradient::{Interpolation, Ramp};
    use daegun::daecore::daemachine::daemath::{Blend, Extend, Gradient, GradientKind, Rgba, Stop};

    let px = inputs_f64(N, 0.0, 1.0);
    let pairs: Vec<(Rgb, Rgb)> = px
        .iter()
        .map(|&v| ([0.25, v as f32, 1.0 - v as f32], [v as f32, 1.0 - v as f32, 0.5]))
        .collect();

    let src_over = core::hint::black_box(Blend::SrcOver);
    let base = move |(d, s): (Rgb, Rgb)| f64::from(blend(src_over, d, s)[0]);

    eprintln!("daemath_baseline – blending, anchored on SrcOver");
    for (name, mode) in [
        ("Multiply", Blend::Multiply),
        ("HardLight", Blend::HardLight),
        ("HslSaturation", Blend::HslSaturation),
    ] {
        let m = core::hint::black_box(mode);
        ratio(
            &format!("blend {name}"),
            &pairs,
            "mode",
            move |(d, s): (Rgb, Rgb)| f64::from(blend(m, d, s)[0]),
            "SrcOver",
            base,
        );
    }
    ratio(
        "composite SrcOver",
        &pairs,
        "composite",
        move |(d, s): (Rgb, Rgb)| {
            let (c, a) = composite(src_over, s, 0.75, d, 0.5);
            f64::from(c[0] + a)
        },
        "blend",
        base,
    );

    let stops = vec![
        Stop { offset: 0.0, color: Rgba::opaque(255, 0, 0) },
        Stop { offset: 0.5, color: Rgba::opaque(0, 255, 0) },
        Stop { offset: 1.0, color: Rgba::opaque(0, 0, 255) },
    ];
    let ramp_with = |kind, how| {
        let g = Gradient {
            kind,
            stops: stops.clone(),
            extend: Extend::Pad,
            transform: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        };
        Ramp::with_interpolation(&g, &[1.0, 0.0, 0.0, 1.0, 0.0, 0.0], how)
    };
    let ramp_of = |kind| ramp_with(kind, Interpolation::LinearLight);
    let line = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 100.0, y1: 100.0 };
    let linear = ramp_of(line);
    let linear_srgb = ramp_with(line, Interpolation::Srgb);
    let radial = ramp_of(GradientKind::Radial { x0: 50.0, y0: 50.0, r0: 0.0, x1: 50.0, y1: 50.0, r1: 70.0 });
    let sweep = ramp_of(GradientKind::Sweep { cx: 50.0, cy: 50.0, start_angle: 0.0, end_angle: 360.0 });

    let pts: Vec<(f64, f64)> = px.iter().enumerate().map(|(i, &v)| (v * 100.0, (i % 100) as f64)).collect();
    let hit = |r: &Ramp, (x, y): (f64, f64)| r.at(x, y).map_or(0.0, |c| f64::from(c.r) + 1.0);

    eprintln!("daemath_baseline – gradients, anchored on linear");
    ratio("gradient radial", &pts, "radial", |p| hit(&radial, p), "linear", |p| hit(&linear, p));
    ratio("gradient sweep", &pts, "sweep", |p| hit(&sweep, p), "linear", |p| hit(&linear, p));
    ratio("gradient linear, sRGB", &pts, "srgb", |p| hit(&linear_srgb, p), "linear", |p| hit(&linear, p));
}

#[test]
#[ignore]
fn atan2_cost_breakdown() {
    let ang = inputs_f64(N, -40.0, 40.0);
    let pos = inputs_f64(N, 0.05, 20.0);

    const C: [f64; 20] = [
        -0.333333333333333, 0.199999999999, -0.142857142857, 0.111111111, -0.0909090909,
        0.0769230769, -0.0666666667, 0.0588235294, -0.0526315789, 0.0476190476,
        -0.0434782609, 0.04, -0.037037037, 0.0344827586, -0.0322580645, 0.0303030303,
        -0.0285714286, 0.027027027, -0.025641026, 0.024390244,
    ];

    fn horner20(z: f64) -> f64 {
        let z2 = z * z;
        let mut acc = C[19];
        let mut i = 18;
        loop {
            acc = C[i] + z2 * acc;
            if i == 0 { break; }
            i -= 1;
        }
        z + z * z2 * acc
    }

    fn estrin20(z: f64) -> f64 {
        let z2 = z * z;
        let z4 = z2 * z2;
        let z8 = z4 * z4;
        let z16 = z8 * z8;
        let p01 = C[0] + z2 * C[1];
        let p23 = C[2] + z2 * C[3];
        let p45 = C[4] + z2 * C[5];
        let p67 = C[6] + z2 * C[7];
        let p89 = C[8] + z2 * C[9];
        let pab = C[10] + z2 * C[11];
        let pcd = C[12] + z2 * C[13];
        let pef = C[14] + z2 * C[15];
        let pgh = C[16] + z2 * C[17];
        let pij = C[18] + z2 * C[19];
        let q0 = p01 + z4 * p23;
        let q1 = p45 + z4 * p67;
        let q2 = p89 + z4 * pab;
        let q3 = pcd + z4 * pef;
        let q4 = pgh + z4 * pij;
        let r0 = q0 + z8 * q1;
        let r1 = q2 + z8 * q3;
        let s = r0 + z16 * (r1 + z8 * q4);
        z + z * z2 * s
    }

    eprintln!("atan2_cost_breakdown");
    duel("atan2 vs one divide", &ang,
         |v| daegun::daecore::daemachine::float::atan2(v, 1.7),
         |v: f64| 1.7 / v);
    duel("atan2 vs horner20", &ang,
         |v| daegun::daecore::daemachine::float::atan2(v, 1.7),
         horner20);
    duel("estrin20 vs horner20", &pos, estrin20, horner20);
    duel("horner20 vs one divide", &pos, horner20, |v: f64| 1.7 / v);
}

#[test]
#[ignore]
fn atan_polynomial_shape() {
    const AT: [f64; 11] = [
        0.3333333333333293, -0.19999999999876483, 0.14285714272503466, -0.11111110405462356,
        0.09090887133436507, -0.0769187620504483, 0.06661073137387531, -0.058335701337905735,
        0.049768779946159324, -0.036531572744216916, 0.016285820115365782,
    ];
    let t = inputs_f64(N, -0.4375, 0.4375);

    fn current(t: f64) -> f64 {
        let z = t * t;
        let w = z * z;
        let odd = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
        let even = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
        t - t * (odd + even)
    }

    fn estrin(t: f64) -> f64 {
        let z = t * t;
        let z2 = z * z;
        let z4 = z2 * z2;
        let z8 = z4 * z4;
        let p01 = AT[0] + z * AT[1];
        let p23 = AT[2] + z * AT[3];
        let p45 = AT[4] + z * AT[5];
        let p67 = AT[6] + z * AT[7];
        let p89 = AT[8] + z * AT[9];
        let q0 = p01 + z2 * p23;
        let q1 = p45 + z2 * p67;
        let q2 = p89 + z2 * AT[10];
        let r = q0 + z4 * (q1 + z4 * q2);
        let _ = z8;
        t - t * z * r
    }

    eprintln!("atan_polynomial_shape  (11 coefficients, identical inputs)");
    duel("estrin4 vs current", &t, estrin, current);
}

#[test]
#[ignore]
fn rounding_old_against_new() {
    const LIMIT: f64 = 4_503_599_627_370_496.0;
    const SIGN: u64 = 0x7fff_ffff_ffff_ffff;

    #[allow(clippy::neg_cmp_op_on_partial_ord, reason = "the negated form passes NaN through")]
    fn integral(x: f64) -> bool { !(f64::from_bits(x.to_bits() & SIGN) < LIMIT) || x == 0.0 }

    fn old_trunc(x: f64) -> f64 {
        if integral(x) { return x; }
        let t = (x as i64) as f64;
        if t == 0.0 && x < 0.0 { -0.0 } else { t }
    }
    fn old_floor(x: f64) -> f64 {
        if integral(x) { return x; }
        let t = (x as i64) as f64;
        if x < t { t - 1.0 } else { t }
    }
    fn old_ceil(x: f64) -> f64 {
        if integral(x) { return x; }
        let t = (x as i64) as f64;
        let r = if x > t { t + 1.0 } else { t };
        if r == 0.0 && x < 0.0 { -0.0 } else { r }
    }
    fn old_round(x: f64) -> f64 {
        if integral(x) { return x; }
        let ti = x as i64;
        let t = ti as f64;
        let step = if x < 0.0 { -1.0 } else { 1.0 };
        let r = if f64::from_bits((x - t).to_bits() & SIGN) >= 0.5 { t + step } else { t };
        if r == 0.0 && x < 0.0 { -0.0 } else { r }
    }
    fn old_rte(x: f64) -> f64 {
        if integral(x) { return x; }
        let ti = x as i64;
        let t = ti as f64;
        let step = if x < 0.0 { -1.0 } else { 1.0 };
        let frac = f64::from_bits((x - t).to_bits() & SIGN);
        let r = if frac > 0.5 || (frac == 0.5 && ti % 2 != 0) { t + step } else { t };
        if r == 0.0 && x < 0.0 { -0.0 } else { r }
    }

    let a = inputs_f64(N, -1000.0, 1000.0);
    eprintln!("rounding_old_against_new  (ratio < 1 means the new one is faster)");
    duel("round_ties_even", &a, FloatExt::round_ties_even, old_rte);
    duel("floor", &a, FloatExt::floor, old_floor);
    duel("ceil", &a, FloatExt::ceil, old_ceil);
    duel("trunc", &a, FloatExt::trunc, old_trunc);
    duel("round", &a, FloatExt::round, old_round);
}

#[test]
#[ignore]
fn atan_reduction_shape() {
    const AT: [f64; 11] = [
        0.3333333333333293, -0.19999999999876483, 0.14285714272503466, -0.11111110405462356,
        0.09090887133436507, -0.0769187620504483, 0.06661073137387531, -0.058335701337905735,
        0.049768779946159324, -0.036531572744216916, 0.016285820115365782,
    ];
    const HI: [f64; 4] = [
        0.4636476090008061, core::f64::consts::FRAC_PI_4,
        0.982793723247329, core::f64::consts::FRAC_PI_2,
    ];
    const LO: [f64; 4] = [
        2.2698777452961687e-17, 3.061616997868383e-17,
        1.3903311031230998e-17, 6.123233995736766e-17,
    ];

    #[inline]
    fn poly(t: f64) -> f64 {
        let z = t * t;
        let w = z * z;
        let odd = z * (AT[0] + w * (AT[2] + w * (AT[4] + w * (AT[6] + w * (AT[8] + w * AT[10])))));
        let even = w * (AT[1] + w * (AT[3] + w * (AT[5] + w * (AT[7] + w * AT[9]))));
        odd + even
    }

    fn branchy(x: f64) -> f64 {
        let negative = x.is_sign_negative();
        let ax = f64::from_bits(x.to_bits() & 0x7fff_ffff_ffff_ffff);
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
        let p = poly(t);
        let r = if k < 0 {
            t - t * p
        } else {
            let i = k as usize;
            HI[i] - ((t * p - LO[i]) - t)
        };
        if negative { -r } else { r }
    }

    const RED: [[f64; 6]; 5] = [
        [1.0,  0.0, 0.0, 1.0, 0.0,   0.0],
        [2.0, -1.0, 1.0, 2.0, HI[0], LO[0]],
        [1.0, -1.0, 1.0, 1.0, HI[1], LO[1]],
        [1.0, -1.5, 1.5, 1.0, HI[2], LO[2]],
        [0.0, -1.0, 1.0, 0.0, HI[3], LO[3]],
    ];

    fn branchless(x: f64) -> f64 {
        let negative = x.is_sign_negative();
        let ax = f64::from_bits(x.to_bits() & 0x7fff_ffff_ffff_ffff);
        if ax >= 1.0e300 {
            let r = HI[3] + LO[3];
            return if negative { -r } else { r };
        }
        let i = usize::from(ax >= 0.4375)
            + usize::from(ax >= 0.6875)
            + usize::from(ax >= 1.1875)
            + usize::from(ax >= 2.4375);
        let c = &RED[i];
        let t = (c[0] * ax + c[1]) / (c[2] * ax + c[3]);
        let r = c[4] - ((t * poly(t) - c[5]) - t);
        if negative { -r } else { r }
    }

    let probe = inputs_f64(20_000, -50.0, 50.0);
    let mut diff = 0usize;
    for &v in &probe {
        if branchy(v).to_bits() != branchless(v).to_bits() {
            diff += 1;
        }
    }
    assert_eq!(diff, 0, "the two reductions disagree on {diff} of {} values", probe.len());

    fn hybrid(x: f64) -> f64 {
        let negative = x.is_sign_negative();
        let ax = f64::from_bits(x.to_bits() & 0x7fff_ffff_ffff_ffff);
        if ax < 0.4375 {
            let r = ax - ax * poly(ax);
            return if negative { -r } else { r };
        }
        if ax >= 1.0e300 {
            let r = HI[3] + LO[3];
            return if negative { -r } else { r };
        }
        let i = usize::from(ax >= 0.6875) + usize::from(ax >= 1.1875) + usize::from(ax >= 2.4375);
        let c = &RED[i + 1];
        let t = (c[0] * ax + c[1]) / (c[2] * ax + c[3]);
        let r = c[4] - ((t * poly(t) - c[5]) - t);
        if negative { -r } else { r }
    }
    let mut hdiff = 0usize;
    for &v in &probe {
        if branchy(v).to_bits() != hybrid(v).to_bits() { hdiff += 1; }
    }
    assert_eq!(hdiff, 0, "hybrid disagrees with the current reduction on {hdiff} values");

    let side = 64usize;
    let sweep: Vec<f64> = (0..N)
        .map(|i| {
            let (px, py) = ((i % side) as f64 - 32.0, (i / side % side) as f64 - 32.0);
            let x = if px == 0.0 { 0.5 } else { px };
            py / x
        })
        .collect();

    let scattered = inputs_f64(N, -40.0, 40.0);
    let one_arm = inputs_f64(N, -0.40, 0.40);
    let wide = inputs_f64(N, -3.0, 3.0);
    eprintln!("atan_reduction_shape  (ratio < 1 means the first named is faster)");
    duel("branchless, scattered", &scattered, branchless, branchy);
    duel("branchless, one arm", &one_arm, branchless, branchy);
    duel("hybrid, scattered", &scattered, hybrid, branchy);
    duel("hybrid, one arm", &one_arm, hybrid, branchy);
    duel("hybrid, mid range", &wide, hybrid, branchy);
    duel("branchless, raster sweep", &sweep, branchless, branchy);
    duel("hybrid, raster sweep", &sweep, hybrid, branchy);
}

#[test]
#[ignore]
fn sqrt_iteration_shape() {
    const BIAS: u64 = 0x1ff8_0000_0000_0000;
    const FISR: u64 = 0x5FE6_EB50_C7B5_37A9;

    fn newton_div(x: f64, iters: u32) -> f64 {
        if x.is_nan() || x < 0.0 { return f64::NAN; }
        if x == 0.0 || x == f64::INFINITY { return x; }
        let (v, undo) = if x < f64::MIN_POSITIVE {
            (x * 1.844_674_407_370_955_2e19, 2.328_306_436_538_696_3e-10)
        } else {
            (x, 1.0)
        };
        let mut y = f64::from_bits((v.to_bits() >> 1) + BIAS);
        for _ in 0..iters {
            y = 0.5 * (y + v / y);
        }
        y * undo
    }

    fn newton_rsqrt(x: f64, iters: u32) -> f64 {
        if x.is_nan() || x < 0.0 { return f64::NAN; }
        if x == 0.0 || x == f64::INFINITY { return x; }
        let (v, undo) = if x < f64::MIN_POSITIVE {
            (x * 1.844_674_407_370_955_2e19, 2.328_306_436_538_696_3e-10)
        } else {
            (x, 1.0)
        };
        let h = 0.5 * v;
        let mut y = f64::from_bits(FISR - (v.to_bits() >> 1));
        for _ in 0..iters {
            y = y * (1.5 - h * y * y);
        }
        (v * y) * undo
    }

    let acc = inputs_f64(60_000, 1e-8, 1e8);
    let ulps = |f: &dyn Fn(f64) -> f64| {
        let mut worst = 0.0f64;
        for &x in &acc {
            let (g, w) = (f(x), x.sqrt());
            if w != 0.0 && w.is_finite() {
                let d = ((g.to_bits() as i64) - (w.to_bits() as i64)).unsigned_abs() as f64;
                worst = worst.max(d);
            }
        }
        worst
    };
    eprintln!("sqrt_iteration_shape");
    eprintln!("    accuracy vs std, worst ulp over 60,000 values in [1e-8, 1e8]");
    eprintln!("      newton on root,      4 passes: {:>5.0} ulp", ulps(&|x| newton_div(x, 4)));
    eprintln!("      newton on reciprocal,4 passes: {:>5.0} ulp", ulps(&|x| newton_rsqrt(x, 4)));
    eprintln!("      newton on reciprocal,5 passes: {:>5.0} ulp", ulps(&|x| newton_rsqrt(x, 5)));

    let v = inputs_f64(N, 1e-6, 1e6);
    duel("rsqrt 4 vs current", &v, |x| newton_rsqrt(x, 4), |x| newton_div(x, 4));
    duel("rsqrt 5 vs current", &v, |x| newton_rsqrt(x, 5), |x| newton_div(x, 4));
    duel("current vs std sqrt", &v, |x| newton_div(x, 4), |x: f64| x.sqrt());
}
