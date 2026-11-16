use super::*;

const UNIT: i64 = 1 << 16;

// One pixel a unit, as 26.6 per unit in 16.16.
fn px_per_unit() -> Scale {
    Scale::new(64 << 16).expect("a scale")
}

fn ladder(rungs: &[(i64, i64)]) -> Ladder {
    let mut l = Ladder::new(Vec::new(), px_per_unit());
    for &(glyph, device) in rungs {
        assert!(l.takes(glyph * UNIT, device), "({glyph}, {device}) doesn't rise");
        l.push(glyph * UNIT, device);
    }
    l.finish();
    l
}

// Rungs at 10 → 11 px and 20 → 21 px: 15 maps half way, 30 nine units past the top at the plain scale, 5 five below
// the bottom.
#[test]
fn a_point_between_rungs_maps_on_the_line_through_them() {
    let l = ladder(&[(10, 11 * UNIT), (20, 21 * UNIT)]);
    assert_eq!(l.map(15 * UNIT), 16 * UNIT);
    assert_eq!(l.map(30 * UNIT), 31 * UNIT);
    assert_eq!(l.map(5 * UNIT), 6 * UNIT);
    // 10 units squeezed into 1 px: a point a third of the way up lands a third of a pixel up.
    let l = ladder(&[(0, 0), (30, UNIT)]);
    assert_eq!(l.map(10 * UNIT), UNIT / 3);
}

// No rung below or above the origin: it holds at 0, unless the rungs straddle it.
#[test]
fn the_origin_holds_where_no_rung_reaches_it() {
    let mut l = ladder(&[(10, 12 * UNIT)]);
    l.hold(0, 0);
    l.finish();
    assert_eq!((l.map(0), l.map(5 * UNIT)), (0, 6 * UNIT));
    let mut l = ladder(&[(-10, -11 * UNIT), (10, 12 * UNIT)]);
    l.hold(0, 0);
    assert_eq!(l.rungs.len(), 2);
}

// A held rung beyond the top whose device y would fall below the last rung's is left out, so the ladder keeps rising.
#[test]
fn a_held_rung_that_would_fall_is_left_out() {
    let mut l = ladder(&[(10, 12 * UNIT)]);
    l.hold(20 * 65536, 11 * UNIT);
    assert_eq!(l.rungs.len(), 1);
    l.hold(20 * 65536, 12 * UNIT);
    assert_eq!(l.rungs.len(), 2);
}

// A rung must rise in glyph space and may stay level in device space.
#[test]
fn a_rung_that_falls_or_doesnt_rise_isnt_taken() {
    let l = ladder(&[(10, 5 * UNIT)]);
    assert!(!l.takes(10 * UNIT, 6 * UNIT) && !l.takes(11 * UNIT, 4 * UNIT));
    assert!(l.takes(11 * UNIT, 5 * UNIT));
}

#[test]
fn a_scale_must_be_above_zero() {
    assert_eq!((Scale::new(0), Scale::new(-1)), (None, None));
    assert_eq!(px_per_unit().of(3 * UNIT), 3 * UNIT);
}
