use super::*;
use crate::daecore::daeshaper::unicode::GeneralCategory;

fn glyphs(n: u32) -> Run {
    let mut run = Run::default();
    (0..n).for_each(|i| run.push(i + 1, i));
    run
}

// A variation selector made a plain mark again keeps no flag that would read as a mark class.
#[test]
fn a_selector_fallback_undone_leaves_no_mark_class() {
    let mut selector = Glyph::of_char(0xFE0F, 0);
    selector.set_selector_fallback(true);
    selector.set_selector_fallback(false);
    assert_eq!(selector.mark_class(), 0);
    assert!(!selector.char_is(text::SELECTOR));
}

#[test]
fn a_deleted_glyph_is_a_hidden_format_character_with_no_mark_class() {
    let mut acute = Glyph::of_char(0x301, 0);
    acute.set_category(GeneralCategory::NonspacingMark);
    acute.set_mark_class(230);
    acute.mark_deleted();
    assert!(acute.char_is(text::DELETED) && acute.char_is(text::HIDDEN));
    assert_eq!((acute.category, acute.mark_class()), (GeneralCategory::Format, 0));
    assert!(!acute.char_is(text::ZWNJ) && !acute.char_is(text::SELECTOR));
}

// A run of n glyphs may grow to max(256n, 65,536) and spend max(4,096n, 65,536) edits, in 64 bits, so no length
// turns the budget negative.
#[test]
fn the_budget_scales_with_the_length() {
    let cases = [
        (3, 65_536, 65_536),
        (1000, 256_000, 4_096_000),
        (524_288, 134_217_728, 2_147_483_648),
        (1_048_576, 268_435_456, 4_294_967_296),
    ];
    for (len, most_glyphs, edits) in cases {
        let mut run = Run { len, ..Run::default() };
        run.begin();
        assert_eq!((run.budget.edits, run.budget.most_glyphs), (edits, most_glyphs), "{len} glyphs");
    }
}

// Each edit spends what it touches: the span it joins or marks, the glyphs it moves or shifts, or the whole run for
// masks. Rewrites start with four glyphs of six written.
#[test]
fn edits_spend_the_budget_by_what_they_touch() {
    let spent = |rewriting: bool, edit: fn(&mut Run)| {
        let mut run = glyphs(6);
        if rewriting {
            run.start_rewrite();
            run.keep_n(4);
        }
        run.budget.edits = 1000;
        edit(&mut run);
        1000 - run.budget.edits
    };
    assert_eq!(spent(false, |r| r.join_clusters(1..4)), 3);
    assert_eq!(spent(false, |r| r.mask_clusters(0..2, 1, 1)), 6);
    assert_eq!(spent(false, |r| r.unsafe_to_break(1..4)), 3);
    assert_eq!(spent(true, |r| r.join_out_clusters(1..4)), 3);
    assert_eq!(spent(true, |r| r.unsafe_to_break_across(1, 6)), 5);
    assert_eq!(spent(true, |r| assert!(r.seek(5))), 1);
    assert_eq!(spent(true, |r| assert!(r.seek(1))), 3);
    // Two read and three written, so going back to the start first moves the four unread forward.
    let rewind = |r: &mut Run| {
        r.seek(2);
        r.insert(9);
        r.budget.edits = 1000;
        assert!(r.seek(0));
    };
    assert_eq!(spent(true, rewind), 4 + 3);
}

// A rewrite the budget stops keeps what it wrote and what it hadn't read: here glyphs 1 and 2 became 9 and 3 was
// kept, so the run is 9 and 3 to 6, with no glyph read kept again as it stood.
#[test]
fn a_failed_rewrite_keeps_its_written_and_unread_glyphs() {
    let mut run = glyphs(6);
    run.budget.edits = 1000;
    run.start_rewrite();
    run.replace_span(2, &[9]);
    assert!(run.keep());
    run.failed = true;
    run.finish_rewrite();
    assert_eq!(run.glyphs[..run.len].iter().map(|g| g.id).collect::<Vec<_>>(), [9, 3, 4, 5, 6]);
}

// A run of 1,048,576 glyphs has 4,294,967,296 edits, more than a 32-bit count holds: as room that reads as the
// largest count rather than none, and a budget spent past zero as none.
#[test]
fn a_budget_past_a_32_bit_count_reads_as_the_most_room() {
    let mut run = Run { len: 1_048_576, ..Run::default() };
    run.begin();
    assert!(run.ops_room() >= u32::MAX as usize, "{} of {} edits", run.ops_room(), run.ops_left());
    run.budget.edits = -5;
    assert_eq!(run.ops_room(), 0);
}

#[test]
fn spending_past_the_budget_fails_the_run() {
    let mut run = glyphs(6);
    run.budget.edits = 2;
    run.join_clusters(0..4);
    assert!(run.failed);

    let mut run = glyphs(6);
    run.start_rewrite();
    run.budget.edits = 2;
    assert!(!run.seek(3));
    assert!(run.failed);
}

// The glyph array grows by half again plus 32 from empty, whatever sizes the steps ask for on the way; the placements
// and the rewrite's own array keep room alongside and fill none of it until they're used.
#[test]
fn the_arrays_grow_together_along_one_chain() {
    let mut run = Run::default();
    let room = [1, 31, 32, 100, 152, 153].map(|size| {
        assert!(run.room_for(size));
        run.glyphs.len()
    });
    assert_eq!(room, [32, 32, 80, 152, 260, 260]);
    assert!([run.places.capacity(), run.apart.capacity()].iter().all(|&c| c >= 260));
    assert!(run.places.is_empty() && run.apart.is_empty());
}

#[test]
fn growing_past_the_most_glyphs_fails_the_run() {
    let mut run = glyphs(3);
    run.budget.most_glyphs = usize::MAX;
    assert!(!run.room_for(MOST_GLYPHS));
    assert_eq!((run.failed, run.glyphs.len()), (true, 32));
}

// No span past 255 glyphs is marked, nor one ending before its start, unless it covers the whole run.
#[test]
fn a_flag_span_past_255_glyphs_marks_nothing_unless_it_covers_the_run() {
    let marked = |mark: fn(&mut Run)| {
        let mut run = glyphs(300);
        run.settings.reported |= flag::UNSAFE_TO_CONCAT;
        mark(&mut run);
        run.glyphs[..300].iter().filter(|g| g.mask & flag::UNSAFE_TO_CONCAT != 0).count()
    };
    assert_eq!(marked(|r| r.unsafe_to_concat(0..255)), 255);
    assert_eq!(marked(|r| r.unsafe_to_concat(0..256)), 0);
    assert_eq!(marked(Run::unsafe_to_concat_all), 300);

    let mut run = glyphs(6);
    run.start_rewrite();
    run.keep_n(4);
    run.unsafe_to_break_across(2, 1);
    assert!(run.out()[..4].iter().all(|g| g.mask & flag::UNSAFE_TO_BREAK == 0));
}

// A zeroed mark's advance moves into its offset, saturating at the ends of i32 rather than wrapping to the far side.
#[test]
fn a_zeroed_advance_moves_into_the_offset_saturating() {
    let (offset_x, offset_y) = (i32::MIN + 5, i32::MAX - 5);
    let at = Placement { advance_x: 10, advance_y: -10, offset_x, offset_y, ..Default::default() };
    let (mut moved, mut kept) = (at, at);
    moved.zero_advance(true);
    kept.zero_advance(false);
    assert_eq!((moved.offset_x, moved.offset_y, moved.advance_x, moved.advance_y), (i32::MIN, i32::MAX, 0, 0));
    assert_eq!((kept.offset_x, kept.offset_y, kept.advance_x, kept.advance_y), (at.offset_x, at.offset_y, 0, 0));
}

// Ids count up from 1 through a byte and start over at 1, as 0 names no ligature.
#[test]
fn ligature_ids_cycle_through_1_to_255() {
    let mut run = glyphs(1);
    run.begin();
    let ids: Vec<u8> = (0..257).map(|_| run.next_ligature_id()).collect();
    assert_eq!((ids[0], ids[254], ids[255], ids[256]), (1, 255, 1, 2));
}
