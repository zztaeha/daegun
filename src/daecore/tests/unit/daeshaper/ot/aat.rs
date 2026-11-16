use super::*;
use super::super::morx::DELETED_GLYPH;
use crate::daecore::daetype::format::aat::StateTable;
use alloc::vec::Vec;

// A state table of `n_classes` classes, glyph 1 listed in class 4 and glyph 2 in class 1, out of bounds, whose
// start state reads `start` and whose next state goes nowhere.
fn table(n_classes: u32, start: [u16; 5], entries: &[[u16; 2]]) -> Vec<u8> {
    let words = |w: &[u16]| w.iter().flat_map(|w| w.to_be_bytes()).collect::<Vec<u8>>();
    let mut t: Vec<u8> = [n_classes, 16, 26, 46].iter().flat_map(|v| v.to_be_bytes()).collect();
    t.extend(words(&[8, 1, 2, 4, 1]));
    t.extend(words(&start));
    t.extend(words(&[0; 5]));
    entries.iter().for_each(|e| t.extend(words(e)));
    t
}

// `left` edits left in the run's budget.
fn with_edits(run: &mut Run, left: i64) {
    run.begin();
    let spent = run.ops_left() - left;
    run.take_ops(u32::try_from(spent).expect("a small run's budget"));
}

fn glyphs(ids: &[u32]) -> Run {
    let mut buffer = Run::default();
    ids.iter().enumerate().for_each(|(i, &g)| buffer.push(g, i as u32));
    buffer
}

// A machine starts on glyphs whose class leaves the start state or acts there (glyph 1 by class 4); where end of text,
// an unlisted glyph's class 1, or more classes than the filter reads do, any run starts it.
#[test]
fn a_start_set_holds_the_glyphs_whose_class_leaves_the_start_state() {
    let starts = |n_classes: u32, start: [u16; 5], entries: &[[u16; 2]]| {
        let bytes = table(n_classes, start, entries);
        let t = StateTable::parse(&bytes, 0, 10).unwrap();
        let starts = start_glyphs(&t, |_| false);
        [1, 2, 3, DELETED_GLYPH].map(|g| starts.contains(g))
    };
    let leaves = [[0, 0], [1, 0]];
    assert_eq!(starts(5, [0, 0, 1, 0, 1], &leaves), [true, false, false, true]);
    assert_eq!(starts(5, [0, 0, 0, 0, 1], &[[0, 0], [0, 0x8000]]), [true, false, false, false]);
    assert_eq!(starts(5, [0, 1, 0, 0, 0], &leaves), [true; 4]);
    assert_eq!(starts(5, [1, 0, 0, 0, 0], &leaves), [true; 4]);
    assert_eq!(starts(600, [0; 5], &[[0, 0]]), [true; 4]);
}

#[test]
fn the_driver_stops_once_the_buffer_fails() {
    let bytes = table(5, [0; 5], &[[0, 0]]);
    let t = StateTable::parse(&bytes, 0, 10).unwrap();
    let mut buffer = glyphs(&[1; 5]);
    let mut seen = Vec::new();
    run_machine(&t, &mut buffer, false, |_| false, None, |b, _| {
        seen.push(b.cursor);
        b.failed = b.cursor >= 1;
    });
    assert_eq!(seen, [0, 1]);
}

// Glyph 1's class, 4, is past the count of 4, so it goes as class 1, which doesn't advance.
#[test]
fn a_class_past_the_count_goes_as_out_of_bounds() {
    let bytes = table(4, [0, 1, 0, 0, 0], &[[0, 0], [0, DONT_ADVANCE]]);
    let t = StateTable::parse(&bytes, 0, 10).unwrap();
    let mut buffer = glyphs(&[1]);
    with_edits(&mut buffer, 2);
    let mut seen = Vec::new();
    run_machine(&t, &mut buffer, false, |_| false, None, |b, _| seen.push(b.cursor));
    assert_eq!(seen, [0, 0, 0, 1]);
}

// DontAdvance spends one each time; at 0 the glyph moves on, and every one after it at once.
#[test]
fn a_glyph_that_never_advances_moves_on_when_the_budget_runs_out() {
    let bytes = table(5, [0, 0, 0, 0, 1], &[[0, 0], [0, DONT_ADVANCE]]);
    let t = StateTable::parse(&bytes, 0, 10).unwrap();
    let mut buffer = glyphs(&[1; 3]);
    with_edits(&mut buffer, 10);
    let mut seen = Vec::new();
    run_machine(&t, &mut buffer, false, |_| false, None, |b, _| seen.push(b.cursor));
    assert_eq!(seen.iter().filter(|&&i| i == 0).count(), 11);
    assert_eq!(&seen[11..], [1, 2, 3]);
    assert_eq!(buffer.ops_left(), -3);
    assert!(!buffer.failed);
}
