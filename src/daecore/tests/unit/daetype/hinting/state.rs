use super::*;

// Each round state on a 32-bit distance, worked by hand: near the rails a distance rounds to the last value of its grid
// i32 holds (0x7FFF_FFC0 the last whole pixel), and its negative, never across the sign.
#[test]
fn rounding_at_the_rails_stops_at_the_last_grid_value() {
    let lattice = |period, phase, threshold| Round::Lattice(Lattice { period, phase, threshold });
    let cases = [
        (Round::Grid, [(95, 64), (-96, -128), (i32::MAX, 0x7FFF_FFC0), (i32::MIN, -0x7FFF_FFC0)]),
        (Round::HalfGrid, [(70, 96), (-70, -96), (i32::MAX, 0x7FFF_FFE0), (i32::MIN, -0x7FFF_FFE0)]),
        (Round::DoubleGrid, [(40, 32), (-48, -64), (i32::MAX, 0x7FFF_FFE0), (i32::MIN, -0x7FFF_FFE0)]),
        (Round::DownToGrid, [(127, 64), (-127, -64), (i32::MAX, 0x7FFF_FFC0), (i32::MIN, -0x7FFF_FFC0)]),
        (Round::UpToGrid, [(1, 64), (-1, -64), (0x7FFF_FFC1, 0x7FFF_FFC0), (i32::MIN, -0x7FFF_FFC0)]),
        (lattice(64, 16, 48), [(100, 144), (-100, -144), (i32::MAX, 0x7FFF_FFD0), (i32::MIN, -0x7FFF_FFD0)]),
        (lattice(45, 0, 22), [(100, 90), (-100, -90), (i32::MAX, 2_147_483_610), (i32::MIN, -2_147_483_610)]),
    ];
    for (state, pairs) in cases {
        for (d, want) in pairs {
            assert_eq!(state.apply(d), want, "{state:?} of {d}");
        }
    }
}

// SROUND's selector: bits 7–6 the period, 5–4 the phase in quarters, 3–0 the threshold in eighths less 4, 0 just
// under the period. S45ROUND's period, half of √2 or 45.25/64 px, is cut to 45, so threshold 0 is 44.
#[test]
fn super_round_selectors_decode_by_the_instruction_set_s_table() {
    let decoded = |selector, grid| match Round::lattice(selector, grid) {
        Round::Lattice(l) => (l.period, l.phase, l.threshold),
        other => panic!("{other:?}"),
    };
    let selector = |period: i32, phase: i32, threshold: i32| period << 6 | phase << 4 | threshold;
    assert_eq!(decoded(selector(1, 0, 8), 0x4000), (64, 0, 32));
    assert_eq!(decoded(selector(0, 1, 5), 0x4000), (32, 8, 4));
    assert_eq!(decoded(selector(2, 3, 0), 0x4000), (128, 96, 127));
    assert_eq!(decoded(selector(1, 0, 1), 0x4000), (64, 0, -24));
    assert_eq!(decoded(selector(1, 0, 0), 0x2D41), (45, 0, 44));
    let ceiling = Round::lattice(selector(1, 0, 0), 0x2D41);
    assert_eq!([0, 1, 45, 46].map(|d| ceiling.apply(d)), [0, 45, 45, 90]);
}

// SPVTL on coincident points, or SPVFS with no direction, falls back to the x-axis the graphics state starts with.
#[test]
fn a_vector_with_no_direction_is_the_x_axis() {
    assert_eq!(Vector::toward(0, 0, false), Vector::X_AXIS);
    assert_eq!(Vector::toward(0, 0, true), Vector::X_AXIS);
    assert_eq!(Vector::toward(0, -5, false), Vector { x: 0, y: -0x4000 });
    assert_eq!(Vector::toward(0, -5, true), Vector::X_AXIS);
    assert_eq!(Vector::toward(3, 0, true), Vector::Y_AXIS);
}

// Resetting a used twilight zone leaves exactly `n` points, all at the origin and untouched, on either side of the
// 64-byte block it zeroes in one store.
#[test]
fn a_reset_twilight_zone_holds_n_points_at_the_origin() {
    let mut zone = Zone::default();
    for n in [0, 3, 16, 17, 100, 5] {
        zone.current.x.extend([7; 70]);
        zone.original.y.extend([7; 70]);
        zone.flags.extend([FLAG_TOUCHED_X; 70]);
        zone.reset_twilight(n);
        let zero = |v: &[i32]| v.len() == n && v.iter().all(|&c| c == 0);
        assert!(zero(&zone.current.x) && zero(&zone.current.y) && zero(&zone.original.x) && zero(&zone.original.y));
        assert!(zone.flags.len() == n && zone.flags.iter().all(|&f| f == 0), "{n} points");
        assert!(zone.unscaled.x.is_empty() && zone.ends.is_empty());
    }
}
