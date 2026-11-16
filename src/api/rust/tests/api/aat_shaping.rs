use super::aat_subset::{
    carets_with, lookup, morx, shapes_alike, stx, u16s, u32s, F, F_F_I, F_F_L, F_I, I, L, PERFORM_ACTION, SET_COMPONENT,
};
use daegun::shape::{ClusterLevel, Direction, ShapeOptions};
use daegun::{GlyphId, Location, Tag};

const MARK_FIRST: u16 = 0x8000;
const MARK_LAST: u16 = 0x2000;
pub(super) const SET_MARK: u16 = 0x8000;
const MARKED_INSERT_ONE: u16 = 0x0001;
const CURRENT_INSERT_ONE: u16 = 0x0020;
const CURRENT_INSERT_THREE: u16 = 0x0060;
const DONT_ADVANCE: u16 = 0x4000;
const CURRENT_INSERT_BEFORE_ONE: u16 = 0x0800 | 0x0020;
const DELETED: u16 = 0xFFFF;
const ALL_DIRECTIONS: u32 = 0x2000_0000;
const IGNORE_MARKS: u16 = 0x0008;
const PUSH: u16 = 0x8000;

// Classes 4, 5 and 6 for f, i and l, after the four every state table has.
const CLASSES: [(u16, u16); 3] = [(F, 4), (I, 5), (L, 6)];

fn flags(f: &daegun::Font, text: &str) -> std::sync::Arc<daegun::shape::ShapedRun> {
    let options = daegun::shape::ShapeOptions::default().with_report_unsafe_to_concat(true);
    f.shape(text, &Location::default(), &options)
}

// Actions 0 and 2 each pop two components and store at the second, where `ligatures` is indexed by their
// sum; the component values, by glyph, are i 1, l 2 and f_i 3.
fn stacked(rows: &[&[u16]], entries: &[&[u16]], ligatures: &[u16]) -> daegun::Font {
    let tails = [u32s(&[0, 0x8000_0000, 0, 0x8000_0000]), u16s(&[0, 0, 1, 2, 0, 3, 0]), u16s(ligatures)];
    carets_with(&[("morx", morx(2, &[(2, stx(7, &CLASSES, rows, entries, &tails))]))])
}

// f pushes, then i and l each push and act.
fn builds_on(ligatures: &[u16]) -> daegun::Font {
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 0];
    let entries: [&[u16]; 4] = [
        &[0, 0, 0],
        &[2, SET_COMPONENT, 0],
        &[3, SET_COMPONENT | PERFORM_ACTION, 0],
        &[0, SET_COMPONENT | PERFORM_ACTION, 2],
    ];
    let rows: [&[u16]; 4] = [start, start, &[0, 0, 0, 0, 1, 2, 0], &[0, 0, 0, 0, 1, 0, 3]];
    stacked(&rows, &entries, ligatures)
}

// fi forms f_i, then l forms f_f_l from f_i and l: the ligature stays on the component stack for the
// next action to build on, and only f_i and l sum to f_f_l's 5, which a subset's closure must follow.
#[test]
fn a_ligature_action_builds_on_the_ligature_before_it() {
    let f = builds_on(&[0, F_I, 0, 0, 0, F_F_L]);

    let run = f.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [F_F_L].map(|g| GlyphId(g.into())));
    assert_eq!(run.clusters, [0]);
    shapes_alike(&f, "fil");
}

// f_f_i at 6 takes f_i's 3 twice, so l's push would have to stack nothing, which only a push coming
// back to l after DontAdvance does. A subset of fil leaves it out.
#[test]
fn a_subset_keeps_no_ligature_only_a_skipped_push_could_form() {
    let f = builds_on(&[0, F_I, 0, 0, 0, F_F_L, F_F_I]);

    assert_eq!(
        f.shape("fil", &Location::default(), &ShapeOptions::default()).glyphs,
        [F_F_L].map(|g| GlyphId(g.into()))
    );
    let (_, out) = shapes_alike(&f, "fil");
    assert_eq!(out.renumbered(GlyphId(F_F_I.into())), None);
}

// l pushes, DontAdvance comes back to it and it pushes again, which stacks it once: the action
// then pops l and f_i for f_f_l, which a closure counting two pushes would miss.
#[test]
fn a_push_back_at_the_same_glyph_stacks_nothing() {
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 0];
    let entries: [&[u16]; 5] = [
        &[0, 0, 0],
        &[2, SET_COMPONENT, 0],
        &[3, SET_COMPONENT | PERFORM_ACTION, 0],
        &[4, SET_COMPONENT | DONT_ADVANCE, 0],
        &[0, SET_COMPONENT | PERFORM_ACTION, 2],
    ];
    let rows: [&[u16]; 5] =
        [start, start, &[0, 0, 0, 0, 1, 2, 0], &[0, 0, 0, 0, 1, 0, 3], &[0, 0, 0, 0, 1, 0, 4]];
    let f = stacked(&rows, &entries, &[0, F_I, 0, 0, 0, F_F_L]);

    assert_eq!(
        f.shape("fil", &Location::default(), &ShapeOptions::default()).glyphs,
        [F_F_L].map(|g| GlyphId(g.into()))
    );
    shapes_alike(&f, "fil");
}

// f pushes, l leads where it would from the start without pushing, and i pushes and acts: fli forms f_i with l after
// it, li alone nothing. At the character level l keeps its own cluster, so the break before it must be unsafe.
#[test]
fn a_break_between_a_ligatures_components_is_unsafe_at_the_character_level() {
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 3];
    let entries: [&[u16]; 4] = [
        &[0, 0, 0],
        &[2, SET_COMPONENT, 0],
        &[0, SET_COMPONENT | PERFORM_ACTION, 0],
        &[2, 0, 0],
    ];
    let f = stacked(&[start, start, &[0, 0, 0, 0, 1, 2, 3]], &entries, &[0, F_I]);
    let characters = ShapeOptions::default().with_cluster_level(ClusterLevel::Characters);
    let shape = |text| f.shape(text, &Location::default(), &characters);

    let run = shape("fli");
    assert_eq!(run.glyphs, [F_I, L].map(|g| GlyphId(g.into())));
    assert_eq!((run.clusters.as_slice(), run.unsafe_to_break.as_slice()), (&[0, 1][..], &[false, true][..]));
    assert_eq!(shape("li").glyphs, [L, I].map(|g| GlyphId(g.into())));
}

// f marks the first glyph and l the last, moving f to the end, however many glyphs lie between.
#[test]
fn a_rearrangement_takes_a_span_of_any_length() {
    let row: &[u16] = &[0, 0, 0, 0, 1, 0, 2];
    let entries: [&[u16]; 3] = [&[0, 0], &[0, MARK_FIRST], &[0, MARK_LAST | 1]];
    let f = carets_with(&[("morx", morx(2, &[(0, stx(7, &CLASSES, &[row, row], &entries, &[]))]))]);

    for is in [62, 63, 1000] {
        let text = format!("f{}l", "i".repeat(is));
        let run = f.shape(&text, &Location::default(), &ShapeOptions::default());
        assert_eq!(run.glyphs.last(), Some(&GlyphId(F.into())), "{} glyphs", is + 2);
    }
}

// f is marked and l turns it into f_i: the span from the mark to l is marked unsafe to break.
#[test]
fn a_contextual_mark_substitution_is_unsafe_to_break() {
    let row: &[u16] = &[0, 0, 0, 0, 1, 0, 2];
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF, 0xFFFF], &[0, SET_MARK, 0xFFFF, 0xFFFF], &[0, 0, 0, 0xFFFF]];
    let substitutions = [u32s(&[4]), lookup(&[(F, F_I)])].concat();
    let f = carets_with(&[("morx", morx(2, &[(1, stx(7, &CLASSES, &[row, row], &entries, &[substitutions]))]))]);

    let run = f.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [F_I, I, L].map(|g| GlyphId(g.into())));
    assert_eq!(run.unsafe_to_break, [false, true, true]);
}

// l substitutes at the mark, which nothing set, so at the first glyph: f, which the lookup doesn't map, stays. Shaped
// apart, "il" has i first, which becomes f_i, so the break before i can't be safe though nothing changed there.
#[test]
fn a_contextual_mark_read_without_a_substitution_is_unsafe_to_break() {
    let row: &[u16] = &[0, 0, 0, 0, 0, 0, 1];
    let entries: [&[u16]; 2] = [&[0, 0, 0xFFFF, 0xFFFF], &[0, 0, 0, 0xFFFF]];
    let substitutions = [u32s(&[4]), lookup(&[(I, F_I)])].concat();
    let f = carets_with(&[("morx", morx(2, &[(1, stx(7, &CLASSES, &[row, row], &entries, &[substitutions]))]))]);

    let run = f.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [F, I, L].map(|g| GlyphId(g.into())));
    let apart = f.shape("il", &Location::default(), &ShapeOptions::default());
    assert_eq!(apart.glyphs, [F_I, L].map(|g| GlyphId(g.into())));
    assert!(run.unsafe_to_break[1], "the break before i is called safe: {:?}", run.unsafe_to_break);
}

// f sends the machine to a state past its table, which stops it with i unread; i shaped apart is read and becomes l,
// so the break before it can't be safe.
#[test]
fn a_machine_stopped_short_of_its_glyphs_calls_no_break_safe() {
    let row: &[u16] = &[0, 0, 0, 0, 1, 2, 0];
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF, 0xFFFF], &[99, 0, 0xFFFF, 0xFFFF], &[0, 0, 0xFFFF, 0]];
    let substitutions = [u32s(&[4]), lookup(&[(I, L)])].concat();
    let f = carets_with(&[("morx", morx(2, &[(1, stx(7, &CLASSES, &[row, row], &entries, &[substitutions]))]))]);

    let run = f.shape("fi", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [F, I].map(|g| GlyphId(g.into())));
    assert_eq!(f.shape("i", &Location::default(), &ShapeOptions::default()).glyphs, [GlyphId(L.into())]);
    assert!(run.unsafe_to_break[1], "the break before i is called safe: {:?}", run.unsafe_to_break);
}

// f is marked and l inserts f_f_i after it: the span from the mark to l is marked unsafe to break,
// which takes in the i between them.
#[test]
fn a_marked_insertion_is_unsafe_to_break_back_to_its_mark() {
    let row: &[u16] = &[0, 0, 0, 0, 1, 0, 2];
    let entries: [&[u16]; 3] =
        [&[0, 0, 0xFFFF, 0xFFFF], &[0, SET_MARK, 0xFFFF, 0xFFFF], &[0, MARKED_INSERT_ONE, 0xFFFF, 0]];
    let f = carets_with(&[("morx", morx(2, &[(5, stx(7, &CLASSES, &[row, row], &entries, &[u16s(&[F_F_I])]))]))]);

    let run = f.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [F, F_F_I, I, L].map(|g| GlyphId(g.into())));
    assert_eq!(run.unsafe_to_break, [false, false, true, true]);
}

// f is marked and every i inserts f_f_i after it, l doing nothing: the last i's span back to f is past what's marked
// glyph by glyph, so the run is marked whole as it settles, and the break before l, past which that i's insertion
// still lands by f, isn't safe.
#[test]
fn a_marked_insertion_far_past_its_mark_is_unsafe_to_break() {
    let row: &[u16] = &[0, 0, 0, 0, 1, 2, 0];
    let entries: [&[u16]; 3] =
        [&[0, 0, 0xFFFF, 0xFFFF], &[0, SET_MARK, 0xFFFF, 0xFFFF], &[0, MARKED_INSERT_ONE, 0xFFFF, 0]];
    let f = carets_with(&[("morx", morx(2, &[(5, stx(7, &CLASSES, &[row, row], &entries, &[u16s(&[F_F_I])]))]))]);

    let run = f.shape(&format!("f{}li", "i".repeat(300)), &Location::default(), &ShapeOptions::default());
    assert_eq!((run.glyphs.len(), run.glyphs[602]), (604, GlyphId(L.into())));
    assert!(run.unsafe_to_break[602], "the break before l is called safe");
}

// i asks for three glyphs from a list of one at the table's end; the list is checked whole, so
// nothing is inserted.
#[test]
fn an_insertion_list_past_the_table_inserts_nothing() {
    let row: &[u16] = &[0, 0, 0, 0, 0, 1, 0];
    let entries: [&[u16]; 2] = [&[0, 0, 0xFFFF, 0xFFFF], &[0, CURRENT_INSERT_THREE, 0, 0xFFFF]];
    let f = carets_with(&[("morx", morx(2, &[(5, stx(7, &CLASSES, &[row, row], &entries, &[u16s(&[F_F_I])]))]))]);

    assert_eq!(f.shape("i", &Location::default(), &ShapeOptions::default()).glyphs, [I].map(|g| GlyphId(g.into())));
}

// Every glyph is marked unsafe to concat before morx runs, here a non-contextual f to f_i.
#[test]
fn morx_makes_the_whole_run_unsafe_to_concat() {
    let f = carets_with(&[("morx", morx(2, &[(4, lookup(&[(F, F_I)]))]))]);

    let run = flags(&f, "fil");
    assert_eq!(run.glyphs, [F_I, I, L].map(|g| GlyphId(g.into())));
    assert_eq!(run.unsafe_to_concat, [true; 3]);
}

// Each of A to E turns into the next, E into A, without advancing. Three glyphs' budget, the least of 65,536,
// substitutes the first 65,537 times, to C; the second once, to B; marking it unsafe uses the rest.
#[test]
fn a_machine_that_never_advances_stops_when_its_budget_runs_out() {
    let bytes = std::fs::read(format!("{}/aat/TestMORXTwentyfour.ttf", super::FONTS)).expect("the fixture font");
    let f = daegun::Font::from_vec(bytes, 0).expect("it parses");

    let run = f.shape("AAA", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [4, 3, 2].map(GlyphId));
    assert_eq!(run.clusters, [0, 1, 2]);
    assert_eq!(run.unsafe_to_break, [false, true, false]);
    assert!(!run.complete);
}

// A non-contextual f to f_i, then an insertion that puts f_f_i at the end of text from the start state: no f is left
// to start it, but end of text acts there, so it runs in fil as in fill.
#[test]
fn an_end_of_text_action_at_the_start_state_runs_whatever_the_run_holds() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF, 0xFFFF], &[0, CURRENT_INSERT_ONE, 0, 0xFFFF], &[2, 0, 0xFFFF, 0xFFFF]];
    let rows: [&[u16]; 3] = [&[1, 0, 0, 0, 2], &[1, 0, 0, 0, 2], &[0; 5]];
    let insertion = stx(5, &[(F, 4)], &rows, &entries, &[u16s(&[F_F_I])]);
    let f = carets_with(&[("morx", morx(2, &[(4, lookup(&[(F, F_I)])), (5, insertion)]))]);

    assert_eq!(
        f.shape("fil", &Location::default(), &ShapeOptions::default()).glyphs,
        [F_I, I, L, F_F_I].map(|g| GlyphId(g.into()))
    );
    let run = f.shape("fill", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [F_I, I, L, L, F_F_I].map(|g| GlyphId(g.into())));
    assert_eq!(run.clusters, [0, 1, 2, 3, 3]);
    assert_eq!(run.unsafe_to_break, [false, true, true, true, true]);
}

// A kerx format 1 machine: f pushes, i waits and l pops f, kerning it by 0. It runs through the driver morx
// uses, which marks i and l unsafe to break.
#[test]
fn a_kerx_machine_marks_what_it_spans_unsafe_to_break() {
    let entries: [&[u16]; 4] = [&[0, 0, 0xFFFF], &[2, PUSH, 0xFFFF], &[2, 0, 0xFFFF], &[0, 0, 6]];
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 0];
    let rows: [&[u16]; 3] = [start, start, &[0, 0, 0, 0, 1, 2, 3]];
    // Action 6 halves to the fourth value, 0, which empties the stack.
    let body = stx(7, &CLASSES, &rows, &entries, &[u16s(&[1, 1, 1, 0])]);
    let f = carets_with(&[("kerx", kerx(1, body))]);

    for (text, unsafe_to_break) in [
        ("fil", &[false, true, true][..]),
        ("fiil", &[false, true, true, true]),
        ("fl", &[false, true]),
        ("if", &[false, false]),
    ] {
        let run = f.shape(text, &Location::default(), &ShapeOptions::default());
        assert_eq!(run.advances, vec![400.0; text.len()], "{text}");
        assert_eq!(run.unsafe_to_break, unsafe_to_break, "{text}");
    }
}

// A `kerx` of one subtable: its coverage, the format in the low byte, and its body.
pub(super) fn kerx(coverage: u32, body: Vec<u8>) -> Vec<u8> {
    let mut out = u16s(&[2, 0]);
    out.extend(u32s(&[1, (12 + body.len()) as u32, coverage, 0]));
    out.extend(body);
    out
}

// f and i push and l pops both, but the table ends one value in. Every value an action pops must be there before
// any is taken, so in fil i keeps its advance; in fl f takes the one value.
#[test]
fn a_kerx_machine_kerns_nothing_unless_every_value_is_there() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF], &[2, PUSH, 0xFFFF], &[0, 0, 0]];
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 0];
    let rows: [&[u16]; 3] = [start, start, &[0, 0, 0, 0, 1, 1, 2]];
    let body = stx(7, &CLASSES, &rows, &entries, &[u16s(&[(-40i16) as u16])]);
    let f = carets_with(&[("kerx", kerx(1, body))]);
    assert_eq!(f.shape("fil", &Location::default(), &ShapeOptions::default()).advances, [400.0; 3]);
    assert_eq!(f.shape("fl", &Location::default(), &ShapeOptions::default()).advances, [360.0, 400.0]);
}

// A vertical kerx machine, with a GPOS `vkrn` to give the kerning a mask: f pushes and l kerns it by -60. The
// kerning applies to glyphs already offset to their vertical origin.
#[test]
fn a_vertical_kerx_machine_kerns_glyphs_offset_to_their_origin() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF], &[2, PUSH, 0xFFFF], &[0, 0, 0]];
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 0];
    let rows: [&[u16]; 3] = [start, start, &[0, 0, 0, 0, 1, 0, 2]];
    let body = stx(7, &CLASSES, &rows, &entries, &[u16s(&[(-60i16) as u16])]);
    let f = carets_with(&[("kerx", kerx(0x8000_0001, body)), ("GPOS", gpos_pair(b"vkrn", I, I, 0))]);
    let features = [daegun::shape::Feature::new(Tag::new(b"vkrn"), 1)];
    let vkrn = daegun::shape::ShapeOptions::default().with_features(&features);
    let run = f.shape("fl", &Location::default(), &vkrn.with_direction(Direction::TopToBottom));
    assert_eq!(run.advances, [1060.0, 1000.0]);
    assert_eq!(crate::xy(&run.offsets), [(-200.0, -910.0), (-200.0, -850.0)]);
}

// kerx format 2 lists f on the left in row 1, byte 6, and i and l on the right in columns 1 and 2, bytes 2 and 4, over
// values -10 to -60. Only glyphs both class tables list are kerned, so il and ff, which would read through class 0,
// kern nothing.
#[test]
fn kerx_format_2_kerns_only_the_glyphs_its_class_tables_list() {
    let mut body = u32s(&[6, 28, 48, 72]);
    body.extend(lookup(&[(F, 6)]));
    body.extend(lookup(&[(I, 2), (L, 4)]));
    body.extend(u16s(&[-10i16, -20, -30, -40, -50, -60].map(|v| v as u16)));
    let f = carets_with(&[("kerx", kerx(2, body))]);
    for (text, advances) in [("fil", &[350.0, 400.0, 400.0][..]), ("ff", &[400.0, 400.0])] {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).advances, advances, "{text}");
    }
}

// kern format 2 sums byte offsets into its array at 36: left f 36, i 34 and l 37, right f 0, i 2 and l 4. What lies
// past the array's start is halved into an index, so il (short) and li (odd) both reach the second, -30.
#[test]
fn kern_format_2_halves_its_offsets_into_an_index() {
    let mut body = u16s(&[0, 16, 26, 36, F, 3, 36, 34, 37, F, 3, 0, 2, 4]);
    body.extend(u16s(&[-20i16, -30, -40, -50].map(|v| v as u16)));
    let f = carets_with(&[("kern", kern(&[(0, 2, body)]))]);
    for text in ["fi", "il", "li"] {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).advances, [370.0, 400.0], "{text}");
        shapes_alike(&f, text);
    }
}

// An Apple `kern` of (coverage, format, body) subtables.
pub(super) fn kern(subtables: &[(u8, u8, Vec<u8>)]) -> Vec<u8> {
    let mut out = u32s(&[0x0001_0000, subtables.len() as u32]);
    for (coverage, format, body) in subtables {
        out.extend(u32s(&[(8 + body.len()) as u32]));
        out.extend([*coverage, *format, 0, 0]);
        out.extend(body);
    }
    out
}

// Six format 0 pairs, fi three times at -20, -40 and -60: a lower midpoint search lands on the third, a lower bound on
// the first and an upper midpoint on the second. A subset of fi keeps the third alone, where three copies read the
// second.
#[test]
fn format_0_kerns_a_repeated_pair_by_the_copy_its_midpoint_finds() {
    let pairs = [(F, I, -20), (F, I, -40), (F, I, -60), (F, L, -80), (I, L, -100), (L, L, -120)];
    let pairs = u16s(&pairs.map(|(l, r, v)| [l, r, v as u16]).concat());
    let kern_table = kern(&[(0, 0, [u16s(&[6, 0, 0, 0]), pairs.clone()].concat())]);
    let kerx_table = kerx(0, [u32s(&[6, 0, 0, 0]), pairs].concat());
    for f in [carets_with(&[("kern", kern_table)]), carets_with(&[("kerx", kerx_table)])] {
        for (text, advances) in [
            ("fi", &[340.0, 400.0][..]),
            ("fl", &[320.0, 400.0]),
            ("il", &[300.0, 400.0]),
            ("fil", &[340.0, 300.0, 400.0]),
        ] {
            assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).advances, advances, "{text}");
            shapes_alike(&f, text);
        }
    }
}

// A `kern` format 1 machine over classes 4, 5 and 6 for f, i and l, new states as byte offsets to their rows. f and
// i push and go to state 2; l pops them with `values`, which its entry names `short` bytes before they start.
pub(super) fn kern_machine(short: u16, values: &[i16]) -> Vec<u8> {
    let (class_at, state_at, after_f, entry_at, values_at) = (10, 17, 31, 38, 54);
    let mut body = u16s(&[7, class_at, state_at, entry_at, values_at, F, 3]);
    body.extend([4, 5, 6]);
    body.extend([0, 0, 0, 0, 1, 0, 0].repeat(2));
    body.extend([0, 0, 0, 0, 1, 2, 3]);
    body.extend(u16s(&[state_at, 0, after_f, PUSH, after_f, PUSH, state_at, values_at - short]));
    body.extend(values.iter().flat_map(|v| v.to_be_bytes()));
    body
}

// i takes the first value and f the second, odd, ending the list; in fl, f alone takes the first. Across the stream
// the values raise the glyphs, each chained to the one before; an entry naming a place before the values names none.
#[test]
fn an_apple_kern_machine_kerns_the_glyphs_it_pushed() {
    let values = kern_machine(0, &[-40, 61]);
    let f = carets_with(&[("kern", kern(&[(0, 1, values.clone())]))]);
    let run = f.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.advances, [460.0, 360.0, 400.0]);
    assert_eq!(crate::xy(&run.offsets), [(60.0, 0.0), (-40.0, 0.0), (0.0, 0.0)]);
    assert_eq!(run.unsafe_to_break, [false, true, true]);
    assert_eq!(f.shape("fl", &Location::default(), &ShapeOptions::default()).advances, [360.0, 400.0]);
    assert_eq!(f.shape("if", &Location::default(), &ShapeOptions::default()).advances, [400.0, 400.0]);
    let features = [daegun::shape::Feature::new(Tag::new(b"kern"), 0)];
    let off = daegun::shape::ShapeOptions::default().with_features(&features);
    assert_eq!(f.shape("fil", &Location::default(), &off).advances, [400.0; 3]);

    let across = carets_with(&[("kern", kern(&[(0x40, 1, values)]))]);
    let run = across.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.advances, [400.0; 3]);
    assert_eq!(crate::xy(&run.offsets), [(0.0, 60.0), (0.0, 20.0), (0.0, 20.0)]);

    let short = carets_with(&[("kern", kern(&[(0, 1, kern_machine(2, &[-40, 61]))]))]);
    assert_eq!(short.shape("fil", &Location::default(), &ShapeOptions::default()).advances, [400.0; 3]);
    for text in ["fil", "fl", "ffl"] {
        shapes_alike(&f, text);
        shapes_alike(&across, text);
    }
}

// kern format 3: glyphs 0 to 3 in left classes 0, 1, 0, 1 and right 0, 0, 1, 2 kern fi and li by -50, fl and ll by 30.
// The variation subtable before it, which kerns fi by -300, is skipped, and every glyph is marked unsafe to concat.
#[test]
fn apple_kern_format_3_kerns_by_class_past_a_variation_subtable() {
    let mut classes = u16s(&[4]);
    classes.extend([3, 2, 3, 0]);
    classes.extend(u16s(&[0, (-50i16) as u16, 30]));
    classes.extend([0, 1, 0, 1, 0, 0, 1, 2, 0, 0, 0, 0, 1, 2]);
    let variation = u16s(&[1, 6, 0, 0, F, I, (-300i16) as u16]);
    let f = carets_with(&[("kern", kern(&[(0x20, 0, variation), (0, 3, classes)]))]);

    for (text, advances, offsets) in [
        ("fi", &[350.0, 400.0][..], &[(0.0, 0.0); 2][..]),
        ("fil", &[350.0, 400.0, 400.0], &[(0.0, 0.0); 3]),
        ("lli", &[430.0, 350.0, 400.0], &[(0.0, 0.0); 3]),
        ("ifl", &[400.0, 430.0, 400.0], &[(0.0, 0.0); 3]),
    ] {
        let run = f.shape(text, &Location::default(), &ShapeOptions::default());
        assert_eq!(run.advances, advances, "{text}");
        assert_eq!(crate::xy(&run.offsets), offsets, "{text}");
    }
    assert_eq!(flags(&f, "if").unsafe_to_concat, [true, true]);
    for text in ["fil", "lli", "fl", "li"] {
        shapes_alike(&f, text);
    }
}

// kern format 3 of one class each side, its pair kerning by -50, a glyph past its classes taking class 0; morx turns f
// into glyph 32,000, past the font's, which kerns nothing: the ff of ffi takes no kern, as ff alone doesn't.
#[test]
fn kern_format_3_kerns_no_id_past_the_fonts_glyphs() {
    let mut one_class = u16s(&[4]);
    one_class.extend([2, 1, 1, 0]);
    one_class.extend(u16s(&[0, (-50i16) as u16]));
    one_class.extend([0, 0, 0, 0, 0, 0, 0, 0, 1, 0]);
    let morx = morx(2, &[(4, lookup(&[(F, 32_000)]))]);
    let f = carets_with(&[("morx", morx), ("kern", kern(&[(0, 3, one_class)]))]);
    let advances = |text| f.shape(text, &Location::default(), &ShapeOptions::default()).advances.clone();

    assert_eq!(advances("ffi")[..2], advances("ff"));
    assert_eq!(advances("il"), [350.0, 400.0]);
}

// A format 4 cmap of (first code, last code, first glyph) runs, in order.
pub(super) fn cmap(runs: &[(u16, u16, u16)]) -> Vec<u8> {
    let mut segments = runs.to_vec();
    segments.push((0xFFFF, 0xFFFF, 0));
    let column = |f: fn(&(u16, u16, u16)) -> u16| u16s(&segments.iter().map(f).collect::<Vec<_>>());
    let n = segments.len() as u16;
    let mut sub = u16s(&[4, 0, 0, n * 2, 0, 0, 0]);
    sub.extend(column(|s| s.1));
    sub.extend(u16s(&[0]));
    sub.extend(column(|s| s.0));
    sub.extend(column(|s| if s.0 == 0xFFFF { 1 } else { s.2.wrapping_sub(s.0) }));
    sub.extend(u16s(&vec![0; segments.len()]));
    let len = sub.len() as u16;
    sub[2..4].copy_from_slice(&len.to_be_bytes());
    [u16s(&[0, 1, 3, 1]), u32s(&[12]), sub].concat()
}

// A GDEF of glyph classes alone, as (first glyph, last glyph, class) ranges in order.
pub(super) fn gdef(ranges: &[(u16, u16, u16)]) -> Vec<u8> {
    let mut classes = u16s(&[2, ranges.len() as u16]);
    ranges.iter().for_each(|&(first, last, class)| classes.extend(u16s(&[first, last, class])));
    [u16s(&[1, 0, 12, 0, 0, 0]), classes].concat()
}

// A GPOS whose `tag` feature adds `x` to `first`'s advance before `second`, ignoring marks between them.
pub(super) fn gpos_pair(tag: &[u8; 4], first: u16, second: u16, x: u16) -> Vec<u8> {
    gpos_pair_set(tag, first, 4, &[1, second, x])
}

// As `gpos_pair`, with `first`'s PairSet, of value records in `format`, given whole: its count, its records and the
// devices they point to, whose offsets count from the PairSet.
pub(super) fn gpos_pair_set(tag: &[u8; 4], first: u16, format: u16, set: &[u16]) -> Vec<u8> {
    let pair = [u16s(&[1, 12, format, 0, 1, 18]), u16s(&[1, 1, first]), u16s(set)].concat();
    let lookups = [u16s(&[1, 4, 2, IGNORE_MARKS, 1, 8]), pair].concat();
    let features = [u16s(&[1]), tag.to_vec(), u16s(&[8, 0, 1, 0])].concat();
    let scripts = [u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0])].concat();
    let (at_scripts, at_features) = (10, 10 + scripts.len() as u16);
    let at_lookups = at_features + features.len() as u16;
    [u16s(&[1, 0, at_scripts, at_features, at_lookups]), scripts, features, lookups].concat()
}

// i inserts the deletion glyph before itself. The glyph the insertion copies, i, is marked deleted too, so both go.
#[test]
fn inserting_the_deletion_glyph_before_a_glyph_deletes_it() {
    let row: &[u16] = &[0, 0, 0, 0, 0, 1, 0];
    let entries: [&[u16]; 2] = [&[0, 0, 0xFFFF, 0xFFFF], &[0, CURRENT_INSERT_BEFORE_ONE, 0, 0xFFFF]];
    let f = carets_with(&[("morx", morx(2, &[(5, stx(7, &CLASSES, &[row, row], &entries, &[u16s(&[DELETED])]))]))]);

    let run = f.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [F, L].map(|g| GlyphId(g.into())));
    assert_eq!(run.clusters, [0, 2]);
}

// morx deletes i between f and an acute. The mark fallback keeps the deleted glyph, hidden, in f's cluster, so
// the acute sits over f.
#[test]
fn a_deleted_glyph_leaves_the_mark_after_it_on_the_base_before() {
    let cmap = cmap(&[(0x66, 0x66, F), (0x69, 0x69, I), (0x6C, 0x6C, L), (0x301, 0x301, L)]);
    let f = carets_with(&[("cmap", cmap), ("morx", morx(2, &[(4, lookup(&[(I, DELETED)]))]))]);

    let run = f.shape("fi\u{301}", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [F, L].map(|g| GlyphId(g.into())));
    assert_eq!(crate::xy(&run.offsets), [(0.0, 0.0), (-320.0, 762.0)]);
    assert_eq!(run.advances, [400.0, 0.0]);
}

// morx deletes an acute between f and a circumflex down a line, where GDEF classes only f, so the marks keep their
// advances. The deleted glyph has no combining class, and its advance moves the circumflex on.
#[test]
fn a_deleted_mark_moves_the_next_mark_on_by_its_advance() {
    let cmap = cmap(&[(0x66, 0x66, F), (0x69, 0x69, I), (0x6C, 0x6C, L), (0x301, 0x302, I)]);
    let morx = morx(2, &[(ALL_DIRECTIONS | 4, lookup(&[(I, DELETED)]))]);
    let f = carets_with(&[("cmap", cmap), ("GDEF", gdef(&[(F, F, 1)])), ("morx", morx)]);

    let run = f.shape(
        "f\u{301}\u{302}",
        &Location::default(),
        &ShapeOptions::default().with_direction(Direction::TopToBottom),
    );
    assert_eq!(run.glyphs, [F, L].map(|g| GlyphId(g.into())));
    assert_eq!(crate::xy(&run.offsets), [(-200.0, -850.0), (80.0, 1956.0)]);
}

// morx turns i between f and l into f_f_i, a mark by GDEF. It takes that class, so the mark's advance is
// zeroed and GPOS kerns f and l across it.
#[test]
fn a_glyph_morx_puts_in_takes_its_gdef_class() {
    let tables = [
        ("GDEF", gdef(&[(F_F_I, F_F_I, 3)])),
        ("GPOS", gpos_pair(b"kern", F, L, 100)),
        ("morx", morx(2, &[(4, lookup(&[(I, F_F_I)]))])),
    ];
    let f = carets_with(&tables);

    let run = f.shape("fil", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs, [F, F_F_I, L].map(|g| GlyphId(g.into())));
    assert_eq!(run.advances, [500.0, 0.0, 400.0]);
}
