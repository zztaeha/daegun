use daegun::{Font, Location};

use super::aat_shaping::gdef;
use super::aat_subset::{carets_with, u16s, F, F_F_L, I, L};
use super::cmap_reading::{cmap, format4};
use super::layout_build::{
    aliased, anchor, classes, coverage, layout, list, lookup, maybe, repeated, run, table, to, w, NULL,
};
use daegun::shape::ShapeOptions;

// carets.ttf with i a mark and a GPOS whose `feature` runs one lookup of `kind` over `subtable`.
fn gpos(feature: &[u8; 4], kind: u16, subtable: Vec<u8>) -> Font {
    let gpos = layout(b"DFLT", &[(feature, &[0])], vec![lookup(kind, 0, vec![to(subtable)])]);
    carets_with(&[("GDEF", gdef(&[(I, I, 3)])), ("GPOS", gpos)])
}

fn anchors(rows: &[(i16, i16)]) -> Vec<u8> {
    list(rows.iter().map(|&(x, y)| to(anchor(x, y))).collect())
}

pub(super) type Anchor = Option<(i16, i16)>;

// A mark array, each record its class and anchor.
pub(super) fn marks(records: &[(u16, Anchor)]) -> Vec<u8> {
    let fields = records.iter().flat_map(|&(class, at)| [w(&[class]), maybe(at.map(|(x, y)| anchor(x, y)))]);
    table([w(&[records.len() as u16])].into_iter().chain(fields).collect())
}

// A MarkBasePos of i over the bases `base_coverage` lists, one row of `bases` a base glyph.
fn mark_base(marks: Option<Vec<u8>>, base_coverage: Option<Vec<u8>>, bases: Vec<u8>) -> Font {
    let fields = vec![w(&[1]), to(coverage(&[I])), maybe(base_coverage), w(&[1]), maybe(marks), to(bases)];
    gpos(b"mark", 4, table(fields))
}

// A null mark array or mark anchor, or a mark past the array, leaves i without an anchor, so it attaches nothing, as
// a null base anchor does; a null base coverage covers nothing.
#[test]
fn a_null_mark_table_reads_as_empty() {
    let unattached = [(0.0, 0.0), (0.0, 0.0)];
    let on_f =
        |marks| crate::xy(&run(&mark_base(marks, Some(coverage(&[F])), anchors(&[(300, 500)])), "fi", false).offsets);
    assert_eq!(on_f(None), unattached);
    assert_eq!(on_f(Some(marks(&[(0, None)]))), unattached);
    assert_eq!(on_f(Some(marks(&[]))), unattached);

    let uncovered = mark_base(Some(marks(&[])), None, anchors(&[(300, 500), (100, 200)]));
    let uncovered = run(&uncovered, "fi", false);
    assert_eq!(crate::xy(&uncovered.offsets), [(0.0, 0.0), (0.0, 0.0)]);
    assert_eq!(uncovered.unsafe_to_concat, [true, true]);
}

// A mark anchor of a format GPOS doesn't define is absent, as a base anchor of one is: in format 1, i's (100, 200)
// meets f's (300, 500) less f's 400 advance; in format 9, i attaches nothing.
#[test]
fn a_mark_anchor_of_an_unknown_format_attaches_nothing() {
    let marks = |format: u16| table(vec![w(&[1, 0]), to(u16s(&[format, 100, 200]))]);
    let on_f = |format| {
        let font = mark_base(Some(marks(format)), Some(coverage(&[F])), anchors(&[(300, 500)]));
        crate::xy(&run(&font, "fi", false).offsets)
    };
    assert_eq!(on_f(1), [(0.0, 0.0), (-200.0, 300.0)]);
    assert_eq!(on_f(9), [(0.0, 0.0); 2]);
}

// f before l kerned by -40 from class 0 against class 1, with no first class definition: every glyph is class 0.
#[test]
fn a_null_class_definition_classes_every_glyph_0() {
    let values = [1, 2, 0, (-40i16) as u16];
    let pair = table(vec![w(&[2]), to(coverage(&[F])), w(&[4, 0]), NULL, to(classes(&[(L, L, 1)])), w(&values)]);
    assert_eq!(run(&gpos(b"kern", 2, pair), "fl", false).advances, [360.0, 400.0]);
}

// A PairPos of format 1 whose pair sets, each its (second glyph, x advance) records, are for the glyphs `firsts`
// lists, a null offset for None.
fn pairs(firsts: &[u16], sets: Vec<Option<&[(u16, i16)]>>) -> Font {
    let set = |records: &[(u16, i16)]| {
        let words: Vec<u16> = records.iter().flat_map(|&(g, x)| [g, x as u16]).collect();
        table(vec![w(&[records.len() as u16]), w(&words)])
    };
    let mut fields = vec![w(&[1]), to(coverage(firsts)), w(&[4, 0, sets.len() as u16])];
    fields.extend(sets.into_iter().map(|s| maybe(s.map(set))));
    gpos(b"kern", 2, table(fields))
}

// f and l covered, one pair set: l's coverage index lies past the count, so it reads the empty set and marks the
// pair unsafe to concat.
#[test]
fn a_pair_set_past_the_count_is_empty() {
    let f = pairs(&[F, L], vec![Some(&[(L, -40)])]);
    let run = run(&f, "ll", false);
    assert_eq!(run.advances, [400.0, 400.0]);
    assert_eq!(run.unsafe_to_concat, [true, true]);
}

// Two records for l in f's pair set: the search takes the first midpoint, record 0.
#[test]
fn a_repeated_pair_reads_the_record_the_search_meets_first() {
    let f = pairs(&[F], vec![Some(&[(L, -40), (L, -80)])]);
    assert_eq!(run(&f, "fl", false).advances, [360.0, 400.0]);
}

// f's pair set holds no record for i, and l's set is null: each misses, and marks the pair unsafe to concat.
#[test]
fn a_pair_that_misses_is_unsafe_to_concat() {
    let f = pairs(&[F, L], vec![Some(&[(I, -40)]), None]);
    assert_eq!(run(&f, "fl", false).unsafe_to_concat, [true, true]);
    assert_eq!(run(&f, "lf", false).unsafe_to_concat, [true, true]);
}

// A CursivePos of the glyphs `covered` lists, each record its (entry, exit) anchors.
pub(super) fn cursive_pos(covered: &[u16], records: &[(Anchor, Anchor)]) -> Vec<u8> {
    let at = |a: Anchor| maybe(a.map(|(x, y)| anchor(x, y)));
    let fields = records.iter().flat_map(|&(entry, exit)| [at(entry), at(exit)]);
    let head = vec![w(&[1]), to(coverage(covered)), w(&[records.len() as u16])];
    table(head.into_iter().chain(fields).collect())
}

fn cursive(covered: &[u16], records: &[(Anchor, Anchor)]) -> Font {
    gpos(b"curs", 3, cursive_pos(covered, records))
}

// l has an entry and the glyph before it no record: f uncovered, and f past the records. Each fails as the glyph
// before has no exit, and marks both unsafe to concat.
#[test]
fn a_cursive_join_to_a_glyph_without_a_record_is_unsafe_to_concat() {
    let uncovered = cursive(&[L], &[(Some((0, 100)), None)]);
    assert_eq!(run(&uncovered, "fl", false).unsafe_to_concat, [true, true]);
    let past = cursive(&[F, L], &[(Some((0, 100)), None)]);
    assert_eq!(run(&past, "lf", false).unsafe_to_concat, [true, true]);
}

// i after i, both marks, where the mark-to-mark lookup covers only l as the mark below: the search finds the first
// i and it isn't covered, so the pair is unsafe to concat.
#[test]
fn an_uncovered_mark_below_is_unsafe_to_concat() {
    let (marks, below) = (to(marks(&[(0, Some((0, 0)))])), to(anchors(&[(0, 300)])));
    let subtable = table(vec![w(&[1]), to(coverage(&[I])), to(coverage(&[L])), w(&[1]), marks, below]);
    assert_eq!(run(&gpos(b"mkmk", 6, subtable), "ii", false).unsafe_to_concat, [true, true]);
}

// i after f, a ligature with no attach table: past an empty ligature array and at a null offset, each read as one
// of no components, so the pair is unsafe to concat.
#[test]
fn a_ligature_without_its_attach_table_is_unsafe_to_concat() {
    let ligatures = |array: Vec<u8>| {
        let marks = to(marks(&[(0, Some((0, 0)))]));
        gpos(b"mark", 5, table(vec![w(&[1]), to(coverage(&[I])), to(coverage(&[F])), w(&[1]), marks, to(array)]))
    };
    assert_eq!(run(&ligatures(list(vec![])), "fi", false).unsafe_to_concat, [true, true]);
    assert_eq!(run(&ligatures(list(vec![NULL])), "fi", false).unsafe_to_concat, [true, true]);
}

// i over f and over l, each its own row of the base array: the base coverage's index gives each base its row.
#[test]
fn a_mark_takes_the_anchor_of_its_base_row() {
    let f = mark_base(Some(marks(&[(0, Some((0, 0)))])), Some(coverage(&[F, L])), anchors(&[(300, 500), (700, 100)]));
    let cases = [
        ("fi", &[(0.0, 0.0), (-100.0, 500.0)][..]),
        ("li", &[(0.0, 0.0), (300.0, 100.0)]),
        ("ii", &[(0.0, 0.0), (0.0, 0.0)]),
        ("fli", &[(0.0, 0.0), (0.0, 0.0), (300.0, 100.0)]),
    ];
    for (text, want) in cases {
        assert_eq!(crate::xy(&run(&f, text, false).offsets), want, "{text}");
    }
}

// A cut word keeps its whole shaping where its first letter is shaped apart: in carets.ttf a space drawn as i kerns
// against f, and a right-to-left cursive join sets each f 50 below the next, placing the first by all the rest.
#[test]
fn a_cut_word_keeps_its_whole_cursive_chain() {
    use super::aat_shaping::cmap;
    use super::aat_subset::{carets_with, F, I};
    use super::gpos_lookups::cursive_pos;
    use super::layout_build::{coverage, layout, lookup, table, to, w};
    use daegun::layout::LayoutOptions;
    let kern = table(vec![w(&[1]), to(coverage(&[I])), w(&[4, 0, 1]), to(table(vec![w(&[1, F, (-40i16) as u16])]))]);
    let curs = cursive_pos(&[F], &[(Some((0, 0)), Some((400, 50)))]);
    let lookups = vec![lookup(2, 0, vec![to(kern)]), lookup(3, 1, vec![to(curs)])];
    let gpos = layout(b"DFLT", &[(b"curs", &[1]), (b"kern", &[0])], lookups);
    let f = carets_with(&[("GPOS", gpos), ("cmap", cmap(&[(0x20, 0x20, I), (0x66, 0x66, F)]))]);
    let text = "ffff ffffffffff";
    let whole = f.shape(text, &Location::default(), &ShapeOptions::default());
    let offset_of = |at: usize| whole.offsets[whole.clusters.iter().position(|&c| c as usize == at).expect("shaped")];
    let layout = f.layout(text, &Location::default(), &LayoutOptions::default().with_max_inline_size(1000.0));
    assert!(layout.lines.len() > 4, "the second word should be cut");
    for line in &layout.lines {
        for run in &line.runs {
            for (i, &c) in run.run.clusters.iter().enumerate() {
                let at = run.bytes.start + c as usize;
                assert_eq!(run.run.offsets[i], offset_of(at), "line {:?}, character {at}", line.bytes);
            }
        }
    }
}

// i and an acute mapped to f_f_l are marks, of attachment classes 1 and 2. 6,000 lookups each read past 200 marks to
// a glyph that fails them unmarked, which the run pays for: a pair from f to an f it has no record for, a cursive join
// from f back to an f without an exit, a mark on i back to an i it doesn't cover.
#[test]
fn a_read_past_skipped_glyphs_is_charged_to_the_run() {
    let map = format4(&[(0x66, F), (0x69, I), (0x6C, L), (0x301, F_F_L)]);
    let marks_of = |[i, acute]: [u16; 2]| classes(&[(I, I, i), (F_F_L, F_F_L, acute)]);
    let gdef = table(vec![w(&[1, 0]), to(marks_of([3, 3])), NULL, NULL, to(marks_of([1, 2]))]);
    let pair = table(vec![w(&[1]), to(coverage(&[F])), w(&[4, 0, 1]), to(u16s(&[1, L, 10]))]);
    let mark_mark = table(vec![
        w(&[1]),
        to(coverage(&[I])),
        to(coverage(&[L])),
        w(&[1]),
        to(marks(&[(0, Some((0, 0)))])),
        to(anchors(&[(0, 300)])),
    ]);
    let cases = [
        (2, 8, pair, "f", "i"),
        (3, 8, cursive_pos(&[F], &[(Some((0, 0)), None)]), "f", "i"),
        (6, 0x100, mark_mark, "i", "\u{301}"),
    ];
    for (kind, flags, subtable, ends, skipped) in cases {
        let gpos = aliased(&[(b"kern", 0..6_000)], 6_000, repeated(kind, flags, 1, subtable), vec![]);
        let f = carets_with(&[("cmap", cmap(&[(3, 1, &map)])), ("GDEF", gdef.clone()), ("GPOS", gpos)]);
        let text = format!("{ends}{}{ends}", skipped.repeat(200));
        assert!(!f.shape(&text, &Location::default(), &ShapeOptions::default()).complete, "type {kind}");
    }
}

// i is a mark. The mark feature runs a chain rule of `flags` on two `on`, whose records apply a MarkBasePos of `on`
// onto f at the second, then the first, `pairs` times: each seek back starts the base search over.
fn seeking_back(pairs: u16, on: u16, flags: u16) -> Font {
    let mark_base = table(vec![
        w(&[1]),
        to(coverage(&[on])),
        to(coverage(&[F])),
        w(&[1]),
        to(marks(&[(0, Some((0, 0)))])),
        to(anchors(&[(0, 300)])),
    ]);
    let records = [1, 1, 0, 1].repeat(usize::from(pairs));
    let chain = table(vec![w(&[3, 0, 2]), to(coverage(&[on])), to(coverage(&[on])), w(&[0, 2 * pairs]), w(&records)]);
    let lookups = vec![lookup(8, flags, vec![to(chain)]), lookup(4, 0, vec![to(mark_base)])];
    carets_with(&[("GDEF", gdef(&[(I, I, 3)])), ("GPOS", layout(b"DFLT", &[(b"mark", &[0])], lookups))])
}

// On 100 i each search started over reads to the run's start, 1.2 million glyphs; on 200 i between two l each search
// forward again reads back to the first l, 1 million. The run pays for both; after f, both i still attach to it.
#[test]
fn a_base_search_started_over_by_records_seeking_back_is_charged_to_the_run() {
    let complete = |f: Font, text: &str| f.shape(text, &Location::default(), &ShapeOptions::default()).complete;
    assert!(!complete(seeking_back(500, I, 0), &"i".repeat(100)));
    assert!(!complete(seeking_back(5_000, L, 8), &format!("l{}l", "i".repeat(200))));
    let attached = crate::xy(&run(&seeking_back(1, I, 0), "fii", false).offsets);
    assert_eq!(attached, [(0.0, 0.0), (-400.0, 300.0), (-400.0, 300.0)]);
}
