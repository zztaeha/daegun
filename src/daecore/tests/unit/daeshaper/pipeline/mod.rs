use super::*;
use alloc::vec::Vec;
use crate::daecore::daeshaper::run::{text, Placement};
use crate::daecore::daeshaper::unicode::Script;

fn advances(run: &Run, horizontal: bool) -> Vec<i32> {
    run.places[..run.len].iter().map(|p| if horizontal { p.advance_x } else { p.advance_y }).collect()
}

// A base and its mark make one grapheme, so only the base is tracked, across the line and down it.
#[test]
fn tracking_moves_each_grapheme_once_saturating() {
    for direction in [Direction::LeftToRight, Direction::TopToBottom] {
        let horizontal = direction.is_horizontal();
        let mut run = Run::default();
        for (codepoint, cluster) in [(0x61, 0), (0x301, 0), (0x62, 1)] {
            run.push(codepoint, cluster);
        }
        run.glyphs[1].text |= text::CONTINUES;
        run.start_positions();
        run.direction = direction;
        let set = |p: &mut Placement, v| if horizontal { p.advance_x = v } else { p.advance_y = v };
        set(&mut run.places[0], i32::MAX - 5);
        set(&mut run.places[2], i32::MIN + 5);

        position::track(&mut run, 10);
        assert_eq!(advances(&run, horizontal), [i32::MAX, 0, i32::MIN + 15], "{direction:?}");
        position::track(&mut run, -20);
        assert_eq!(advances(&run, horizontal), [i32::MAX - 20, 0, i32::MIN], "{direction:?}");
    }
}

// Digits and spaces belong to every script, so the first letter decides; a script given is kept.
#[test]
fn a_run_takes_the_script_of_its_first_letter_and_its_direction() {
    let mut run = Run::default();
    run.push_str("12 سلام");
    assert_eq!(settle_script(&mut run), Direction::RightToLeft);
    assert_eq!(run.script, Some(Script::ARABIC));

    let mut run = Run::default();
    run.push_str("سلام");
    run.script = Some(Script::LATIN);
    assert_eq!(settle_script(&mut run), Direction::LeftToRight);

    let mut run = Run::default();
    run.push_str("123");
    assert_eq!((settle_script(&mut run), run.script), (Direction::LeftToRight, None));
}
