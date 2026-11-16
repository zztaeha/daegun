use super::*;

const DEGREE: i64 = 1 << 16;

// The unit vector at `angle` and its tangent, in 16.16.
fn unit(angle: i64) -> (i64, i64) {
    let (s, c) = sin_cos(angle);
    (round_shift(c, Q - 16) as i64, round_shift(s, Q - 16) as i64)
}

fn tangent(angle: i64) -> i64 {
    round_shift(tan(angle), 28) as i64
}

// Each multiple of 90° is exact, and 30° and 45° round from cos 30° = 0.8660254… (56,755.84 units) and
// √2/2 = 0.7071068… (46,340.95). A tangent at ±90° pins at its rail.
#[test]
fn quarter_turns_are_exact_and_common_angles_round_to_the_nearest_unit() {
    let units = [0, 90, 180, 270, -90, 30, 45, 150].map(|d| unit(d * DEGREE));
    let want = [(65536, 0), (0, 65536), (-65536, 0), (0, -65536), (0, -65536), (56756, 32768), (46341, 46341),
        (-56756, 32768)];
    assert_eq!(units, want);
    let tangents = [0, 45, -45, 135, 90, -90, 270].map(|d| tangent(d * DEGREE));
    assert_eq!(tangents, [0, 65536, -65536, -65536, 0x7FFF_FFFF, -0x7FFF_FFFF, -0x7FFF_FFFF]);
}

// Over a sweep of angles, every value lies within its bound of the true one, here a double's, whose own error is
// far under the slack: sine and cosine half a unit, the tangent half a unit where it stays under 1,000.
#[test]
fn every_value_lies_within_half_a_unit_of_the_true_one() {
    for angle in (-(400 * DEGREE)..400 * DEGREE).step_by(997) {
        let radians = angle as f64 / 65536.0 * core::f64::consts::PI / 180.0;
        let (cos, sin) = unit(angle);
        assert!((cos as f64 - radians.cos() * 65536.0).abs() <= 0.5 + 1e-9, "cos at {angle}: {cos}");
        assert!((sin as f64 - radians.sin() * 65536.0).abs() <= 0.5 + 1e-9, "sin at {angle}: {sin}");
        if radians.tan().abs() < 1000.0 {
            let t = tangent(angle);
            assert!((t as f64 - radians.tan() * 65536.0).abs() <= 0.5 + 1e-3, "tan at {angle}: {t}");
        }
    }
}

// A turn far past any a font stores ends as fast as a small one, its whole turns taken off first.
#[test]
fn a_huge_angle_turns_in_bounded_time() {
    assert_eq!(unit(1 << 58), unit((1 << 58) % (360 << 16)));
    assert_eq!(unit(-(1 << 58)), unit(-((1 << 58) % (360 << 16))));
}
