use super::*;

// Ink at the ends of the range places a mark where 64-bit arithmetic clamped back to a position puts it, rather than
// wrapping, or panicking where overflow is checked.
#[test]
fn extreme_extents_clamp_rather_than_wrap() {
    let (max, min) = (i32::MAX, i32::MIN);
    let mark = GlyphExtents { x_bearing: min, y_bearing: min, width: min, height: min };
    let base = Room { left: max.into(), width: max.into(), top: max.into(), bottom: i64::from(max) * 2 };
    for (class, want) in [(ccc::ABOVE, (max, max)), (ccc::BELOW, (max, 0)), (ccc::BELOW_LEFT, (max, 0))] {
        let mut room = base;
        assert_eq!(lay(mark, class, &mut room, 128, Direction::LeftToRight), want, "class {class}");
    }
}

// A second mark of one class stacks on the first, its ink's height and the gap further out; one of another class
// starts from the base again, as `place_about` hands it a fresh room.
#[test]
fn marks_of_one_class_stack_out_from_the_base() {
    let mark = GlyphExtents { x_bearing: 0, y_bearing: 50, width: 20, height: -50 };
    let mut room = Room { left: 0, width: 100, top: 500, bottom: 0 };
    let first = lay(mark, ccc::ABOVE, &mut room, 10, Direction::LeftToRight);
    let second = lay(mark, ccc::ABOVE, &mut room, 10, Direction::LeftToRight);
    assert_eq!((first, second), ((40, 510), (40, 570)));
    let mut below = Room { left: 0, width: 100, top: 500, bottom: 0 };
    assert_eq!(lay(mark, ccc::BELOW_RIGHT, &mut below, 10, Direction::LeftToRight), (80, -60));
}

// Classes Unicode gives by script name a position: Hebrew's shin dot above right, Thai's mai han-akat and phinthu,
// and a class that names a position already stays.
#[test]
fn a_script_numbered_class_takes_the_position_its_mark_sits_at() {
    assert_eq!(positional_class(0x05C1, 10), ccc::ABOVE_RIGHT);
    assert_eq!(positional_class(0x0E31, 0), ccc::ABOVE_RIGHT);
    assert_eq!(positional_class(0x0E3A, 9), ccc::BELOW_RIGHT);
    assert_eq!(positional_class(0x0301, ccc::ABOVE), ccc::ABOVE);
    assert_eq!(positional_class(0x0670, 35), ccc::ABOVE);
}
