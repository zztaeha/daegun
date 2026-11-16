use super::aat_shaping::{gdef, kern, kerx};
use super::layout_lookups::layout_lookup;
use super::aat_subset::{carets_with, lookup, shapes_alike, stx, u16s, u32s, F, I, L};
use daegun::Location;
use daegun::shape::ShapeOptions;

// Classes 4, 5 and 6 for f, i and l, after the four every state table has.
const CLASSES: [(u16, u16); 3] = [(F, 4), (I, 5), (L, 6)];
const PUSH: u16 = 0x8000;
const MARK: u16 = 0x8000;

fn pairs(pairs: &[(u16, u16, i16)]) -> Vec<u8> {
    u16s(&pairs.iter().flat_map(|&(l, r, v)| [l, r, v as u16]).collect::<Vec<_>>())
}

// A kerx of version 4 and one subtable of `tuples` values a tuple; before version 4 the count is ignored.
fn kerx_tuples(coverage: u32, tuples: u32, body: Vec<u8>) -> Vec<u8> {
    let mut table = kerx3(coverage, body);
    table[..2].copy_from_slice(&4u16.to_be_bytes());
    table[16..20].copy_from_slice(&tuples.to_be_bytes());
    table
}

fn advances(f: &daegun::Font, cases: &[(&str, &[f64])]) {
    for &(text, want) in cases {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).advances, want, "{text}");
        shapes_alike(f, text);
    }
}

// An OpenType `kern` subtable flagged Minimum holds limits, not kerning, so fi keeps its advances; a kerx subtable
// flagged as a variation holds kerning values and runs.
#[test]
fn a_minimum_subtable_kerns_nothing_and_a_kerx_variation_one_kerns() {
    let fi = pairs(&[(F, I, -40)]);
    let body = [u16s(&[1, 0, 0, 0]), fi.clone()].concat();
    let minimum = [u16s(&[0, 1, 0, (6 + body.len()) as u16, 0x0003]), body].concat();
    let variation = kerx(0x2000_0000, [u32s(&[1, 0, 0, 0]), fi].concat());
    advances(&carets_with(&[("kern", minimum)]), &[("fi", &[400.0; 2])]);
    advances(&carets_with(&[("kerx", variation)]), &[("fi", &[360.0, 400.0])]);
}

// The second of two OpenType kern subtables flagged override replaces what the first added to a pair it holds: fi's
// -40 becomes -10, and fl, which it doesn't hold, keeps -20. Unflagged, fi adds both, -50.
#[test]
fn an_override_subtable_replaces_what_kerned_a_pair_before_it() {
    let sub = |coverage: u16, held: &[(u16, u16, i16)]| {
        let body = [u16s(&[held.len() as u16, 0, 0, 0]), pairs(held)].concat();
        [u16s(&[0, (6 + body.len()) as u16, coverage]), body].concat()
    };
    let first = sub(0x0001, &[(F, I, -40), (F, L, -20)]);
    let kern = |coverage| [u16s(&[0, 2]), first.clone(), sub(coverage, &[(F, I, -10)])].concat();
    advances(&carets_with(&[("kern", kern(0x0009))]), &[("fi", &[390.0, 400.0]), ("fl", &[380.0, 400.0])]);
    advances(&carets_with(&[("kern", kern(0x0001))]), &[("fi", &[350.0, 400.0]), ("fl", &[380.0, 400.0])]);
}

// With tuples a format 0 value is a byte offset from the subtable to its tuple, whose first value kerns: fi's, at
// 40, holds -60, and fl's, at 42, lies past the subtable.
#[test]
fn a_format_0_tuple_kerns_by_its_first_value() {
    let body = [u32s(&[2, 0, 0, 0]), pairs(&[(F, I, 40), (F, L, 42)]), u16s(&[(-60i16) as u16])].concat();
    let f = carets_with(&[("kerx", kerx_tuples(0, 1, body))]);
    advances(&f, &[("fi", &[340.0, 400.0]), ("fl", &[400.0; 2])]);
}

// Format 2's array holds byte offsets from the subtable to tuples: f's class is byte 0, i's 2 and l's 4, so fi reads
// the tuple at 78, -60, and fl the one at 80, -20.
#[test]
fn a_format_2_tuple_kerns_by_its_first_value() {
    let body = [
        u32s(&[6, 28, 48, 72]),
        lookup(&[(F, 0)]),
        lookup(&[(I, 2), (L, 4)]),
        u16s(&[0, 78, 80]),
        u16s(&[(-60i16) as u16, (-20i16) as u16]),
    ];
    let f = carets_with(&[("kerx", kerx_tuples(2, 1, body.concat()))]);
    advances(&f, &[("fi", &[340.0, 400.0]), ("fl", &[380.0, 400.0])]);
}

// Format 6's array holds byte offsets from its vector to tuples: fi reads the one at the vector's start, -60, and
// fl the next, -20.
// kerx format 2's classes are byte offsets, as Apple's spec stores them: f's row 1 at 6 bytes a row and i's column 2
// at 2 bytes a value sum to 10, the sixth value, -30. Read as an index, 10 would land past the array.
#[test]
fn kerx_format_2_classes_are_byte_offsets_into_its_array() {
    let body = [
        u32s(&[6, 28, 48, 68]),
        lookup(&[(F, 6)]),
        lookup(&[(I, 4)]),
        u16s(&[0, 0, 0, 0, (-50i16) as u16, (-30i16) as u16]),
    ];
    let f = carets_with(&[("kerx", kerx(2, body.concat()))]);
    advances(&f, &[("fi", &[370.0, 400.0])]);
}

#[test]
fn a_format_6_tuple_kerns_by_its_first_value() {
    let body = [
        u32s(&[0]),
        u16s(&[1, 2]),
        u32s(&[36, 56, 80, 86]),
        lookup(&[(F, 0)]),
        lookup(&[(I, 1), (L, 2)]),
        u16s(&[0, 0, 2]),
        u16s(&[(-60i16) as u16, (-20i16) as u16]),
    ];
    let f = carets_with(&[("kerx", kerx_tuples(6, 1, body.concat()))]);
    advances(&f, &[("fi", &[340.0, 400.0]), ("fl", &[380.0, 400.0])]);
}

// A format 1 machine with two values a tuple: f and i push and l pops them, i taking the first value, -40, and f
// the third, -60, which empties the stack; the second and fourth go unread.
#[test]
fn a_format_1_machine_steps_through_its_tuples() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF], &[2, PUSH, 0xFFFF], &[0, 0, 0]];
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 0];
    let rows: [&[u16]; 3] = [start, start, &[0, 0, 0, 0, 1, 1, 2]];
    let values = u16s(&[(-40i16) as u16, 7, (-60i16) as u16, 7]);
    let f = carets_with(&[("kerx", kerx_tuples(1, 2, stx(7, &CLASSES, &rows, &entries, &[values])))]);
    advances(&f, &[("fil", &[340.0, 360.0, 400.0]), ("fl", &[360.0, 400.0])]);
}

// The same machine at 2^31 values a tuple: fil's second value lies 4 GiB past its first, past the subtable, so
// nothing kerns, while fl reads one value; a 32-bit target, whose sizes stop short of 4 GiB, reads them alike.
#[test]
fn a_format_1_value_4_gib_past_the_first_kerns_nothing() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF], &[2, PUSH, 0xFFFF], &[0, 0, 0]];
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 0];
    let rows: [&[u16]; 3] = [start, start, &[0, 0, 0, 0, 1, 1, 2]];
    let values = u16s(&[(-40i16) as u16, 7, (-60i16) as u16, 7]);
    let f = carets_with(&[("kerx", kerx_tuples(1, 1 << 31, stx(7, &CLASSES, &rows, &entries, &[values])))]);
    advances(&f, &[("fil", &[400.0, 400.0, 400.0]), ("fl", &[360.0, 400.0])]);
}

// A kerx format 1 action names its first value by index, as Apple's fonts store it: f pushes, i acts from value 0,
// -60, and l from value 2, -20. Halved as a byte offset, l's 2 would name the end of i's list and kern nothing.
#[test]
fn a_kerx_format_1_action_names_its_first_value_by_index() {
    let entries: [&[u16]; 4] = [&[0, 0, 0xFFFF], &[0, PUSH, 0xFFFF], &[0, 0, 0], &[0, 0, 2]];
    let start: &[u16] = &[0, 0, 0, 0, 1, 2, 3];
    let values = u16s(&[(-60i16) as u16, 0xFFFF, (-20i16) as u16, 0xFFFF]);
    let f = carets_with(&[("kerx", kerx_tuples(1, 1, stx(7, &CLASSES, &[start, start], &entries, &[values])))]);
    advances(&f, &[("fi", &[340.0, 400.0]), ("fl", &[380.0, 400.0])]);
}

// A cross-stream shift carries down the line: f's 300, set as i reads, lifts i and l with it. "l" shaped apart from
// "fi" stays down, so the break before l can't be safe.
#[test]
fn a_break_a_cross_stream_shift_carries_across_is_unsafe() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF], &[2, PUSH, 0xFFFF], &[0, 0, 0]];
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 0];
    let rows: [&[u16]; 3] = [start, start, &[0, 0, 0, 0, 1, 2, 0]];
    let table = kerx_tuples(0x4000_0001, 1, stx(7, &CLASSES, &rows, &entries, &[u16s(&[300])]));
    let f = carets_with(&[("kerx", table)]);
    let at = Location::default();
    let run = f.shape("fil", &at, &ShapeOptions::default());
    assert_eq!(run.offsets.iter().map(|o| o.y).collect::<Vec<_>>(), [300.0; 3]);
    assert_eq!(f.shape("l", &at, &ShapeOptions::default().with_context("fi", "")).offsets[0].y, 0.0);
    assert!(run.unsafe_to_break[2], "the break before l is called safe: {:?}", run.unsafe_to_break);
}

// f pushes and the machine stays at its start, so i reads as a text's first glyph would, but l's value kerns f: "f"
// shaped apart from "il" keeps its advance, so the break before i can't be safe.
#[test]
fn a_break_before_a_value_kerning_a_glyph_back_is_unsafe() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF], &[0, PUSH, 0xFFFF], &[0, 0, 0]];
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 2];
    let values = u16s(&[(-60i16) as u16]);
    let f = carets_with(&[("kerx", kerx_tuples(1, 1, stx(7, &CLASSES, &[start, start], &entries, &[values])))]);
    let at = Location::default();
    let run = f.shape("fil", &at, &ShapeOptions::default());
    assert_eq!(run.advances, [340.0, 400.0, 400.0]);
    assert_eq!(f.shape("f", &at, &ShapeOptions::default().with_context("", "il")).advances, [400.0]);
    assert!(run.unsafe_to_break[1], "the break before i is called safe: {:?}", run.unsafe_to_break);
}

// Each f pushes, the machine staying at its start, and l's value kerns the glyph on top. Nine f push past the stack's
// eight, which empties it, so l kerns nothing; the eight after the first don't, and l kerns the last of them. The break
// before the second f can't be safe.
#[test]
fn a_break_among_pushes_that_overflow_the_stack_is_unsafe() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF], &[0, PUSH, 0xFFFF], &[0, 0, 0]];
    let start: &[u16] = &[0, 0, 0, 0, 1, 0, 2];
    let values = u16s(&[(-60i16) as u16, 0xFFFF]);
    let f = carets_with(&[("kerx", kerx_tuples(1, 1, stx(7, &CLASSES, &[start, start], &entries, &[values])))]);
    let at = Location::default();
    let run = f.shape(&format!("{}l", "f".repeat(9)), &at, &ShapeOptions::default());
    assert_eq!(run.advances, [400.0; 10]);
    let tail = f.shape(&format!("{}l", "f".repeat(8)), &at, &ShapeOptions::default().with_context("f", ""));
    assert_eq!(tail.advances[7], 340.0);
    assert!(run.unsafe_to_break[1], "the break before the second f is called safe: {:?}", run.unsafe_to_break);
}

// Every subtable a kerx names runs, as many as its bytes hold: 65,537 format 0 subtables, the first kerning fl, the
// last fi, and none of those between anything.
#[test]
fn every_kerx_subtable_runs() {
    let format0 = |pairs: Vec<u8>| [u32s(&[28 + pairs.len() as u32, 0, 0, (pairs.len() / 6) as u32, 0, 0, 0]), pairs];
    let mut kerx = [u16s(&[2, 0]), u32s(&[65_537])].concat();
    kerx.extend(format0(pairs(&[(F, L, -20)])).concat());
    (0..65_535).for_each(|_| kerx.extend(format0(Vec::new()).concat()));
    kerx.extend(format0(pairs(&[(F, I, -60)])).concat());
    advances(&carets_with(&[("kerx", kerx)]), &[("fi", &[340.0, 400.0]), ("fl", &[380.0, 400.0])]);
}

// kerx format 4 of action type `kind`: f marks, i attaches to the mark by action 0, and l attaches by action 1 and
// marks itself.
fn attach(kind: u8, actions: Vec<u8>) -> Vec<u8> {
    let entries: [&[u16]; 4] = [&[0, 0, 0xFFFF], &[0, MARK, 0xFFFF], &[0, 0, 0], &[0, MARK, 1]];
    let row: &[u16] = &[0, 0, 0, 0, 1, 2, 3];
    let mut body = stx(7, &CLASSES, &[row, row], &entries, &[actions]);
    body[16] |= kind << 6;
    kerx(4, body)
}

fn offsets(f: &daegun::Font, cases: &[(&str, &[(f64, f64)])]) {
    for &(text, want) in cases {
        assert_eq!(crate::xy(&f.shape(text, &Location::default(), &ShapeOptions::default()).offsets), want, "{text}");
        let (subset, _) = shapes_alike(f, text);
        let shaped = subset.shape(text, &Location::default(), &ShapeOptions::default());
        assert_eq!(crate::xy(&shaped.offsets), want, "{text}, subset");
    }
}

// Points (type 0) attach as coordinates (type 2) do: f's point 2, (320, 700), meets i's point 0, (20, 0), less f's 400
// advance. A point past the outline attaches nothing, nor an action past the subtable, but l still marks: i takes l.
#[test]
fn kerx_format_4_attaches_only_where_it_can_read_the_action() {
    let points = attach(0, u16s(&[2, 0, 1, 3]));
    let past = attach(0, u16s(&[4, 0, 0, 0]));
    let coordinates = attach(2, u16s(&[100, 50, 0, 0, 30, 20, 0, 0]));
    let short = attach(2, u16s(&[100, 50, 0, 0, 30, 20]));
    offsets(&carets_with(&[("kerx", points)]), &[
        ("fi", &[(0.0, 0.0), (-100.0, 700.0)]),
        ("fli", &[(0.0, 0.0), (-100.0, -700.0), (-300.0, 700.0)]),
    ]);
    offsets(&carets_with(&[("kerx", past)]), &[("fi", &[(0.0, 0.0); 2])]);
    offsets(&carets_with(&[("kerx", coordinates)]), &[
        ("fi", &[(0.0, 0.0), (-300.0, 50.0)]),
        ("fli", &[(0.0, 0.0), (-370.0, 20.0), (-670.0, 50.0)]),
    ]);
    offsets(&carets_with(&[("kerx", short)]), &[("fli", &[(0.0, 0.0), (0.0, 0.0), (-300.0, 50.0)])]);
}

// f marks and i, after an a that does nothing, attaches to it: "ai" shaped apart from "f" has no mark and leaves i
// where it is, so the break before a can't be safe.
#[test]
fn a_break_between_an_attachment_and_its_mark_is_unsafe() {
    let f = carets_with(&[("kerx", attach(2, u16s(&[100, 50, 0, 0, 30, 20, 0, 0])))]);
    let at = Location::default();
    let run = f.shape("fai", &at, &ShapeOptions::default());
    assert_ne!(run.offsets[2].y, 0.0);
    assert_eq!(f.shape("ai", &at, &ShapeOptions::default().with_context("f", "")).offsets[1].y, 0.0);
    assert!(run.unsafe_to_break[1], "the break before a is called safe: {:?}", run.unsafe_to_break);
}

// An attachment reaches 16 bits of glyphs: i, 32,769 past f, or before it in a subtable run backwards, attaches
// nothing and keeps its place.
#[test]
fn kerx_format_4_attaches_nothing_past_16_bits() {
    let pad = "a".repeat(32_768);
    let kerx = attach(2, u16s(&[100, 50, 0, 0, 30, 20, 0, 0]));
    let mut backwards = kerx.clone();
    backwards[12] |= 0x10;
    let (forward, backward) = (carets_with(&[("kerx", kerx)]), carets_with(&[("kerx", backwards)]));
    let unmoved = vec![(0.0, 0.0); 32_770];
    offsets(&forward, &[(&format!("f{pad}i"), &unmoved)]);
    offsets(&backward, &[(&format!("i{pad}f"), &unmoved)]);
}

// The subtable run backwards, so the glyph a later one marks comes after it: i attaches forward to f, moved on past
// its own advance.
#[test]
fn kerx_format_4_attaches_a_glyph_to_one_after_it() {
    let mut backwards = attach(2, u16s(&[100, 50, 0, 0, 30, 20, 0, 0]));
    backwards[12] |= 0x10;
    offsets(&carets_with(&[("kerx", backwards)]), &[
        ("if", &[(500.0, 50.0), (0.0, 0.0)]),
        ("lif", &[(830.0, 20.0), (500.0, 50.0), (0.0, 0.0)]),
        ("iif", &[(900.0, 50.0), (500.0, 50.0), (0.0, 0.0)]),
    ]);
}

// ankr's anchor arrays for these glyphs, in order, and the offsets a lookup gives them by.
fn anchors(glyphs: &[(u16, &[(i16, i16)])]) -> (Vec<(u16, u16)>, Vec<u8>) {
    let (mut entries, mut data) = (Vec::new(), Vec::new());
    for &(glyph, points) in glyphs {
        entries.push((glyph, data.len() as u16));
        data.extend(u32s(&[points.len() as u32]));
        data.extend(u16s(&points.iter().flat_map(|&(x, y)| [x as u16, y as u16]).collect::<Vec<_>>()));
    }
    (entries, data)
}

// An ankr of `lookup` over `data`, its lookup offset 0 where `lookup` is empty.
fn ankr(lookup: &[u8], data: &[u8]) -> Vec<u8> {
    let lookup_at = if lookup.is_empty() { 0 } else { 12 };
    [u16s(&[0, 0]), u32s(&[lookup_at, 12 + lookup.len() as u32]), lookup.to_vec(), data.to_vec()].concat()
}

// Anchor actions (type 1): f's anchor 0 to i's 0, and f's 1 to l's 0, which l lacks and reads as (0, 0), through
// ankr's lookup in any format Apple's lookup tables define, 10 too. An ankr with no lookup gives every anchor (0, 0).
#[test]
fn kerx_format_4_reads_anchors_through_ankr_s_lookup() {
    let kerx = attach(1, u16s(&[0, 0, 1, 0]));
    let (entries, data) = anchors(&[(F, &[(100, 200), (300, 400)]), (I, &[(10, 20)])]);
    let trimmed = u16s(&[10, 2, F, 2, entries[0].1, entries[1].1]);
    for table in [ankr(&lookup(&entries), &data), ankr(&trimmed, &data)] {
        let read = carets_with(&[("kerx", kerx.clone()), ("ankr", table)]);
        offsets(&read, &[
            ("fi", &[(0.0, 0.0), (-310.0, 180.0)]),
            ("fl", &[(0.0, 0.0), (-100.0, 400.0)]),
            ("fli", &[(0.0, 0.0), (-100.0, 400.0), (-510.0, -20.0)]),
        ]);
    }
    let f = carets_with(&[("kerx", kerx), ("ankr", ankr(&[], &data))]);
    offsets(&f, &[("fi", &[(0.0, 0.0), (-400.0, 0.0)])]);
}

// A format 6 lookup of 32-bit values.
fn lookup32(entries: &[(u16, u32)]) -> Vec<u8> {
    let n = entries.len() as u16;
    let selector = 15 - n.max(1).leading_zeros() as u16;
    let mut out = u16s(&[6, 6, n, 6 << selector, selector, 6 * n - (6 << selector)]);
    entries.iter().for_each(|&(g, v)| out.extend([u16s(&[g]), u32s(&[v])].concat()));
    out.extend([u16s(&[0xFFFF]), u32s(&[0])].concat());
    out
}

// Format 6 with long values reads 32-bit rows, columns and values: fi -60 and fl -20.
#[test]
fn format_6_reads_long_values() {
    let body = [
        u32s(&[1]),
        u16s(&[1, 2]),
        u32s(&[36, 60, 90, 0]),
        lookup32(&[(F, 0)]),
        lookup32(&[(I, 1), (L, 2)]),
        u32s(&[0, (-60i32) as u32, (-20i32) as u32]),
    ];
    let f = carets_with(&[("kerx", kerx(6, body.concat()))]);
    advances(&f, &[("fi", &[340.0, 400.0]), ("fl", &[380.0, 400.0])]);
}

// Format 6 kerns a pair only where the row lookup lists its left glyph and the column lookup its right one: l is in
// neither, so fl and li kern nothing where row and column 0 would read -20.
#[test]
fn format_6_kerns_only_the_glyphs_its_lookups_list() {
    let body = [
        u32s(&[0]),
        u16s(&[1, 2]),
        u32s(&[36, 60, 80, 0]),
        lookup(&[(F, 0), (I, 0)]),
        lookup(&[(I, 1)]),
        u16s(&[(-20i16) as u16, (-60i16) as u16]),
    ];
    let f = carets_with(&[("kerx", kerx(6, body.concat()))]);
    advances(&f, &[("fi", &[340.0, 400.0]), ("fl", &[400.0; 2]), ("li", &[400.0; 2])]);
}

// Format 0's pairs out of order: lf comes first, so a search for f's pairs from the start finds none. Every pair's
// left glyph still lets the subtable run, and its midpoint search finds fi and fl.
#[test]
fn an_out_of_order_format_0_runs_for_every_left_glyph() {
    let listed = pairs(&[(L, F, -20), (F, I, -40), (F, L, -60)]);
    let kern_table = kern(&[(0, 0, [u16s(&[3, 0, 0, 0]), listed.clone()].concat())]);
    let kerx_table = kerx(0, [u32s(&[3, 0, 0, 0]), listed].concat());
    for table in [("kern", kern_table), ("kerx", kerx_table)] {
        advances(&carets_with(&[table]), &[("fi", &[360.0, 400.0]), ("fl", &[340.0, 400.0])]);
    }
}

// i a mark GPOS attaches to f, beside a cross-stream kern or kerx pairing f with l, which a subset of fi keeps, as a
// subtable starts at its pairs' left glyphs. kern puts i at (200, 500); with kerx, GPOS doesn't run.
#[test]
fn a_subset_keeps_a_pairs_left_glyph_when_its_right_glyph_goes() {
    let mark_base = u16s(&[1, 12, 18, 1, 24, 36, 1, 1, I, 1, 1, F, 1, 0, 6, 1, 100, 0, 1, 4, 1, 300, 500]);
    let gpos = layout_lookup(b"mark", 4, &[mark_base]);
    let pair = pairs(&[(F, L, -50)]);
    let kern_table = kern(&[(0x40, 0, [u16s(&[1, 6, 0, 0]), pair.clone()].concat())]);
    let kerx_table = kerx(0x4000_0000, [u32s(&[1, 6, 0, 0]), pair].concat());
    for (tag, table, i) in [("kern", kern_table, (200.0, 500.0)), ("kerx", kerx_table, (0.0, 0.0))] {
        let f = carets_with(&[("GDEF", gdef(&[(F, F, 1), (I, I, 3)])), ("GPOS", gpos.clone()), (tag, table)]);
        offsets(&f, &[("fi", &[(0.0, 0.0), i])]);
    }
}

// A kerx of version 3, its subtable followed by the table's coverage word, 0xFFFFFFFF.
fn kerx3(coverage: u32, body: Vec<u8>) -> Vec<u8> {
    let mut table = kerx(coverage, body);
    table[..2].copy_from_slice(&3u16.to_be_bytes());
    [table, u32s(&[u32::MAX])].concat()
}

// Formats 2 and 6 read the value two classes sum to wherever it lands in the subtable, as each subset does: the next
// lookup's unit size or l's own column. Past the subtable, a version 3 table's coverage word or beyond, it reads none.
#[test]
fn format_2_and_6_sums_read_within_their_subtable() {
    let words = |values: &[i16]| u16s(&values.iter().map(|&v| v as u16).collect::<Vec<_>>());
    let (f0, il3, il6) = (lookup(&[(F, 0)]), lookup(&[(I, 1), (L, 3)]), lookup(&[(I, 2), (L, 6)]));
    let x2_part = [u32s(&[4, 32, 32 + f0.len() as u32, 28]), words(&[0, -40]), f0.clone(), il6].concat();
    let x6_part = [u32s(&[0]), u16s(&[1, 2]), u32s(&[40, 40 + f0.len() as u32, 36, 0]), words(&[0, -40]), f0, il3];
    let (rows, cols) = (lookup32(&[(F, 0)]), lookup32(&[(I, 1), (L, 13)]));
    let at = [44, 44 + rows.len() as u32, 36, 0];
    let x6_long_part = [u32s(&[1]), u16s(&[1, 2]), u32s(&at), u32s(&[0, (-40i32) as u32]), rows, cols].concat();
    let (left, right) = (lookup(&[(F, 0), (I, 4)]), lookup(&[(F, 6), (I, 2), (L, 4)]));
    let at = [28, 28 + left.len(), 28 + left.len() + right.len()].map(|o| o as u32);
    let x2_trail = [u32s(&[4]), u32s(&at), left, right, words(&[0, -40])].concat();
    let (rows, cols) = (lookup32(&[(F, 0)]), lookup32(&[(I, 1), (L, 2)]));
    let at = [36, 36 + rows.len(), 36 + rows.len() + cols.len(), 0].map(|o| o as u32);
    let x6_trail = [u32s(&[1]), u16s(&[1, 2]), u32s(&at), rows, cols, u32s(&[0, (-40i32) as u32])].concat();

    let part: &[(&str, &[f64])] = &[("fi", &[360.0, 400.0]), ("fl", &[404.0, 400.0])];
    advances(&carets_with(&[("kerx", kerx(2, x2_part))]), part);
    advances(&carets_with(&[("kerx", kerx(6, x6_part.concat()))]), part);
    advances(&carets_with(&[("kerx", kerx(6, x6_long_part))]), &[("fi", &[360.0, 400.0]), ("fl", &[413.0, 400.0])]);
    let trail: &[(&str, &[f64])] = &[("fi", &[360.0, 400.0]), ("fl", &[400.0; 2]), ("ii", &[400.0; 2])];
    let past: &[(&str, &[f64])] = &[("if", &[400.0; 2]), ("il", &[400.0; 2])];
    advances(&carets_with(&[("kerx", kerx3(2, x2_trail))]), &[trail, past].concat());
    advances(&carets_with(&[("kerx", kerx3(6, x6_trail))]), &trail[..2]);
}

// Apple `kern` format 2, left classes counting the array's offset: below the array or past a subset's tables, each
// still kerns in a subset, and laid out as Apple Chancery sums land in a class table or past the end.
#[test]
fn kern_format_2_subsets_keep_every_class_and_sum() {
    let words = |values: &[i16]| u16s(&values.iter().map(|&v| v as u16).collect::<Vec<_>>());
    let format2 = |left: &[u16], right: &[u16], array: u16| {
        let (lt, rt) = (u16s(left), u16s(right));
        let mut body = [u16s(&[0, 16, 16 + lt.len() as u16, array]), lt, rt].concat();
        body.resize(usize::from(array) - 8, 0);
        [body, words(&[-40, -20])].concat()
    };
    let fi_fl: &[(&str, &[f64])] = &[("fi", &[360.0, 400.0]), ("fl", &[380.0, 400.0])];
    advances(&carets_with(&[("kern", kern(&[(0, 2, format2(&[F, 1, 0], &[I, 2, 200, 202], 200))]))]), fi_fl);
    advances(&carets_with(&[("kern", kern(&[(0, 2, format2(&[F, 1, 3], &[I, 2, 29, 31], 32))]))]), fi_fl);

    let (lt, rt) = (u16s(&[F, 2, 16, 20]), u16s(&[I, 2, 0, 12]));
    let chancery = [u16s(&[0, 20, 20 + lt.len() as u16, 16]), words(&[-40, -20]), lt, rt].concat();
    let (lt, rt) = (u16s(&[I, 1, 16]), u16s(&[L, 1, 100]));
    let first = [u16s(&[0, 20, 20 + lt.len() as u16, 16]), words(&[-10, 0]), lt, rt].concat();
    let f = carets_with(&[("kern", kern(&[(0, 2, first), (0, 2, chancery)]))]);
    advances(&f, &[("fi", &[360.0, 400.0]), ("fl", &[402.0, 400.0]), ("ii", &[401.0, 400.0]), ("il", &[400.0; 2])]);
}
