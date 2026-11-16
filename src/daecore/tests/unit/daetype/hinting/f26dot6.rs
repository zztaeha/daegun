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

// f64::max passes over a NaN, so it has to be refused on its own, or it reaches the hinters.
#[test]
fn a_coordinate_that_is_not_finite_does_not_fit() {
    let size = Size::from_px(16.0).expect("a size");
    assert!(fits([0.0, 10.0], size, 1000));
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(!fits([0.0, v, 10.0], size, 1000), "{v} fits");
    }
}

// The quotient worked in 64 bits against 128, at the rails and over a spread of operands: to the nearest with halves
// away from 0, or cut toward 0, saturated; over 0 a product saturates with its sign, and 0 stays 0.
#[test]
fn a_ratio_divides_exactly_in_64_bits_and_saturates_over_zero() {
    let wide = |a: i32, b: i32, c: i32, nearest: bool| {
        let (n, d) = (i128::from(a) * i128::from(b), i128::from(c));
        let q = if d == 0 {
            n.signum() * i128::MAX
        } else {
            let (q, r) = (n / d, n % d);
            if nearest && 2 * r.abs() >= d.abs() { q + n.signum() * d.signum() } else { q }
        };
        q.clamp(i32::MIN.into(), i32::MAX.into()) as i32
    };
    assert_eq!((ratio(7, 1, 0), ratio(-7, 1, 0), ratio(0, 5, 0)), (i32::MAX, i32::MIN, 0));
    assert_eq!((ratio(3, 1, 2), ratio(-3, 1, 2), ratio_truncated(-3, 1, 2)), (2, -2, -1));
    let mut values = alloc::vec![i32::MIN, i32::MIN + 1, -0x10000, -64, -1, 0, 1, 63, 64, 0x10000, i32::MAX];
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    values.extend((0..40).map(|_| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 32) as i32 >> (state % 31)
    }));
    for &a in &values {
        for &b in &values {
            for &c in &values {
                assert_eq!(ratio(a, b, c), wide(a, b, c, true), "{a} * {b} / {c}");
                assert_eq!(ratio_truncated(a, b, c), wide(a, b, c, false), "{a} * {b} / {c} unrounded");
            }
        }
    }
}

// A slice scaled at once reads as each value scaled alone: halves, signs, NaN, infinity and the rails, at three sizes
// and with an em of 0; and a value `fits_scale` passed scales alike without the clamp.
#[test]
fn values_scale_alike_alone_and_together() {
    let values = [0.0, -0.0, 0.5, -0.5, 1.5, 2.5, -2.5, 999.75, -1e9, 1e9, f32::NAN, f32::INFINITY, f32::MIN, 3e-41];
    for (px, upm) in [(11.0, 1000), (16.0, 2048), (900.0, 16), (12.0, 0)] {
        let size = Size::from_px(px).expect("a size");
        let mut together = alloc::vec::Vec::new();
        scale_f32_into(&values, size, upm, &mut together);
        let alone: alloc::vec::Vec<i32> = values.iter().map(|&v| scale_f32(v, size, upm)).collect();
        assert_eq!(together, alone, "{px} px, {upm} units");
    }
    for scale16 in [1, 0x10000, 67_109, 1 << 20, 1 << 33] {
        let reach = (((SCALE_LIMIT << 16) - 1) / scale16).min(i64::from(i32::MAX)) as i32;
        for v in [0, 1, -1, 31, -32_768, reach, -reach, reach / 2].map(|v| v.clamp(-reach, reach)) {
            assert!(fits_scale([v].into_iter(), scale16));
            assert_eq!(scale_fitting(v, scale16), scale_units(v, scale16), "{v} at {scale16}");
        }
    }
}

// A product's magnitude rounds, halves away from 0, so negating either factor negates the result; past i32 it
// saturates, and a scale past SCALE_LIMIT pins there.
#[test]
fn products_round_halves_away_from_zero_and_saturate() {
    assert_eq!([times_16_16(1, 0x8000), times_16_16(-1, 0x8000), times_16_16(1, 0x7FFF)], [1, -1, 0]);
    assert_eq!([times_2_14(3, 0x2000), times_2_14(-3, 0x2000), times_2_14(-1, 0x1FFF)], [2, -2, 0]);
    assert_eq!([times_16_16(i32::MAX, i32::MAX), times_16_16(i32::MIN, i32::MAX)], [i32::MAX, i32::MIN]);
    assert_eq!([dot_2_14(1, -2, 0x2000, 0x2000), dot_2_14(i64::MAX, i64::MAX, 0x4000, 1)], [-1, i32::MAX]);
    assert_eq!([scale_units(1, 0x8000), scale_units(-1, 0x8000), scale_units(i32::MIN, 1 << 40)], [1, -1, -(1 << 29)]);
    assert_eq!([scale_f26dot6(96, 0x10000), scale_f26dot6(-96, 0x10000)], [2, -1], "half a 64th rounds up");
}

// Each 2.14 component c of a unit vector is the nearest to 2^14 v / L: (2c - 1)² L² <= 2^30 v² < (2c + 1)² L², checked
// in 128 bits over vectors near the axes and diagonals, at the rails, and spread by a xorshift.
#[test]
fn a_unit_vector_s_components_are_exactly_rounded() {
    assert_eq!(unit_vector(0, 0), None);
    let known = [((1, 0), (16384, 0)), ((1, 1), (11585, 11585)), ((3, -4), (9830, -13107))];
    for ((dx, dy), want) in known.into_iter().chain([((-480, 1), (-16384, 34))]) {
        assert_eq!(unit_vector(dx, dy), Some(want), "({dx}, {dy})");
    }
    let rounds_right = |v: i64, c: i32, length2: u128| {
        let (v2, c) = (u128::from(v.unsigned_abs()).pow(2) << 30, u128::from(c.unsigned_abs()));
        let below = c == 0 || (2 * c - 1).pow(2) * length2 <= v2;
        below && v2 < (2 * c + 1).pow(2) * length2
    };
    let mut vectors = alloc::vec![(i64::from(i32::MAX) * 2, 1), (1, -(1 << 32)), (-(1 << 32), 1 << 32), (7, 7)];
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    vectors.extend((0..20_000).map(|_| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        ((state as i32 >> (state % 29)) as i64, ((state >> 32) as i32 >> (state % 23)) as i64)
    }));
    for (dx, dy) in vectors.into_iter().filter(|&v| v != (0, 0)) {
        let (x, y) = unit_vector(dx, dy).expect("a direction");
        let length2 = u128::from(dx.unsigned_abs()).pow(2) + u128::from(dy.unsigned_abs()).pow(2);
        assert!(rounds_right(dx, x, length2) && rounds_right(dy, y, length2), "({dx}, {dy}) gave ({x}, {y})");
        let sign_kept = |c: i32, d: i64| c == 0 || (c < 0) == (d < 0);
        assert!(sign_kept(x, dx) && sign_kept(y, dy), "({dx}, {dy}) gave ({x}, {y})");
    }
}
