use alloc::vec::Vec;

use super::{settle, AdvanceSums, Axis, Shift, ShiftTree};
use crate::daecore::daeshaper::ot::apply::NESTING_LIMIT;
use crate::daecore::daeshaper::run::{AttachKind, Placement};

// A xorshift sequence: varied values that repeat from run to run.
fn sequence(mut seed: u64) -> impl FnMut() -> u64 {
    move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    }
}

// Every span of a run of saturating shifts, composed, gives what the shifts give one at a time, for runs of each
// length to 40 whose shifts reach past either end of an i32.
#[test]
fn composed_shifts_saturate_as_single_ones_do() {
    let mut next = sequence(0x2545_F491_4F6C_DD1D);
    let (min, max) = (i64::from(i32::MIN), i64::from(i32::MAX));
    for n in 1..40 {
        let shifts: Vec<i64> = (0..n)
            .map(|_| match next() % 5 {
                0 => max,
                1 => min,
                2 => -min,
                _ => i64::from(next() as i32),
            })
            .collect();
        let tree = ShiftTree::new(shifts.iter().map(|&d| Shift::by(d)));
        for from in 0..=n {
            for to in from..=n {
                let x = next() as i32;
                let one_at_a_time =
                    shifts[from..to].iter().fold(x, |x, &d| (i64::from(x) + d).clamp(min, max) as i32);
                assert_eq!(tree.span(from..to).apply(x), one_at_a_time, "{n} shifts, {from}..{to}");
            }
        }
    }
}

// A walk on either side of the short limit gives what the advances give one at a time, on both axes and signs,
// with advances past either end of an i32.
#[test]
fn walks_short_and_long_saturate_as_single_steps_do() {
    let mut next = sequence(0x9E37_79B9_7F4A_7C15);
    let n = 2 * AdvanceSums::SHORT + 3;
    let pick = |r: u64| match r % 4 {
        0 => i32::MAX,
        1 => i32::MIN,
        _ => r as i32 >> (r % 24),
    };
    let glyph = |advance_x, advance_y| Placement { advance_x, advance_y, ..Default::default() };
    let places: Vec<Placement> = (0..n).map(|_| glyph(pick(next()), pick(next()))).collect();
    let mut sums = AdvanceSums::default();
    for (axis, back) in [(Axis::X, false), (Axis::X, true), (Axis::Y, false), (Axis::Y, true)] {
        for from in 0..=n {
            for to in from..=n {
                let x = next() as i32;
                let one_at_a_time = places[from..to].iter().fold(x, |x, p| {
                    let a = if axis == Axis::Y { p.advance_y } else { p.advance_x };
                    if back { x.saturating_sub(a) } else { x.saturating_add(a) }
                });
                assert_eq!(sums.walk(&places, x, axis, back, from..to), one_at_a_time, "{from}..{to}");
            }
        }
    }
}

// Glyphs hanging from each other in a ring of 0 to 1 to 2 and back: the walk from 0 drops the link that would close
// the ring, so 2 stays a root, each offset across is added once, and every link is cut.
#[test]
fn a_cycle_of_attachments_resolves_once_round() {
    let glyph = |to: i16, kind: AttachKind, offset_y: i32| {
        let mut p = Placement { offset_y, ..Default::default() };
        (p.attach.to, p.attach.kind) = (to, kind);
        p
    };
    let cursive = |to, offset_y| glyph(to, AttachKind::Cursive, offset_y);
    let mut places = [cursive(1, 10), cursive(1, 20), cursive(-2, 40)];
    let (mut links, mut sums) = ([(0, 0); NESTING_LIMIT], AdvanceSums::default());
    for i in 0..places.len() {
        if places[i].attach.to != 0 {
            settle(&mut places, i, (true, false), &mut links, &mut sums);
        }
    }
    assert_eq!(places.map(|p| (p.attach.to, p.offset_y)), [(0, 70), (0, 60), (0, 40)]);

    // A glyph and a mark hanging from each other: the walk from the glyph drops the mark's link back, and the mark
    // keeps its offsets.
    let mut mixed = [cursive(1, 10), glyph(-1, AttachKind::Mark, 20)];
    settle(&mut mixed, 0, (true, false), &mut links, &mut sums);
    assert_eq!(mixed.map(|p| (p.attach.to, p.offset_y)), [(0, 30), (0, 20)]);
}
