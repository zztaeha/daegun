use super::aat_shaping::kerx;
use super::aat_subset::{carets_with, glyph_ids, lookup, morx, shapes_alike, stx_classed, u16s, u32s, F, F_I, I, L};
use daegun::Location;
use daegun::shape::ShapeOptions;

// A lookup of format 2 (last, first, value) or 6 (glyph, value) whose units stay in the order given, ending in a
// terminator unit where `terminated`.
fn units(format: u16, units: &[&[u16]], terminated: bool) -> Vec<u8> {
    let words = units[0].len();
    let mut all: Vec<Vec<u16>> = units.iter().map(|u| u.to_vec()).collect();
    if terminated {
        all.push((0..words).map(|w| if w + 1 < words { 0xFFFF } else { 0 }).collect());
    }
    let (size, n) = (2 * words as u16, all.len() as u16);
    let selector = 15 - n.max(1).leading_zeros() as u16;
    let mut out = u16s(&[format, size, n, size << selector, selector, size * n - (size << selector)]);
    all.iter().for_each(|u| out.extend(u16s(u)));
    out
}

fn glyphs(f: &daegun::Font, cases: &[(&str, &[u16])]) {
    for &(text, want) in cases {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs, glyph_ids(want), "{text}");
        shapes_alike(f, text);
    }
}

fn advances(f: &daegun::Font, cases: &[(&str, &[f64])]) {
    for &(text, want) in cases {
        assert_eq!(f.shape(text, &Location::default(), &ShapeOptions::default()).advances, want, "{text}");
        shapes_alike(f, text);
    }
}

// A contextual subtable substituting f with f_i, f being class 4. Its class lookup lists l, f and i in that order:
// a binary search stops on f at its first midpoint, where a lower-bound search would land on l.
#[test]
fn a_morx_class_lookup_out_of_order_is_searched_by_midpoint() {
    let classes = units(2, &[&[L, L, 6], &[F, F, 4], &[I, I, 5]], true);
    let entries: [&[u16]; 2] = [&[0, 0, 0xFFFF, 0xFFFF], &[0, 0, 0xFFFF, 0]];
    let row: &[u16] = &[0, 0, 0, 0, 1, 0, 0];
    let substitutions = [u32s(&[4]), lookup(&[(F, F_I)])].concat();
    let body = stx_classed(7, classes, &[row, row], &entries, &[substitutions]);
    let f = carets_with(&[("morx", morx(2, &[(1, body)]))]);
    glyphs(&f, &[("f", &[F_I]), ("fi", &[F_I, I])]);
}

// Non-contextual lookups searched by midpoint: of i, f, l out of order the search finds f and l, not i; with a last
// unit of 0xFFFF left unsearched, of l and f before it, it finds only l.
#[test]
fn a_non_contextual_lookup_is_searched_by_midpoint() {
    let unordered = units(6, &[&[I, L], &[F, F_I], &[L, I]], true);
    let terminated = units(6, &[&[L, I], &[F, F_I]], true);
    let (unordered, terminated) =
        (carets_with(&[("morx", morx(2, &[(4, unordered)]))]), carets_with(&[("morx", morx(2, &[(4, terminated)]))]));
    glyphs(&unordered, &[("fil", &[F_I, I, I])]);
    glyphs(&terminated, &[("fl", &[F, I])]);
}

// kerx format 2 reads each glyph's class by search: the left lookup lists l then f out of order, so the search misses
// f, which is then as absent as a glyph no unit lists and isn't kerned; l, found, kerns -20 against i.
#[test]
fn kerx_format_2_kerns_no_glyph_its_search_misses() {
    let body = [
        u32s(&[4, 28, 52, 72]),
        units(6, &[&[L, 4], &[F, 8]], true),
        lookup(&[(I, 2)]),
        u16s(&[0, (-50i16) as u16, 0, (-20i16) as u16, 0, (-90i16) as u16]),
    ];
    let f = carets_with(&[("kerx", kerx(2, body.concat()))]);
    advances(&f, &[("fi", &[400.0; 2]), ("li", &[380.0, 400.0])]);
}

// kerx format 6 likewise: its row lookup lists l then f, the search misses f, and fi isn't kerned where l kerns -20.
#[test]
fn kerx_format_6_kerns_no_glyph_its_search_misses() {
    let body = [
        u32s(&[0]),
        u16s(&[3, 2]),
        u32s(&[36, 60, 80, 0]),
        units(6, &[&[L, 2], &[F, 4]], true),
        lookup(&[(I, 1)]),
        u16s(&[0, (-50i16) as u16, 0, (-20i16) as u16, 0, (-90i16) as u16]),
    ];
    let f = carets_with(&[("kerx", kerx(6, body.concat()))]);
    advances(&f, &[("fi", &[400.0; 2]), ("li", &[380.0, 400.0])]);
}

// f's class 516, past the table's 7, reads as out of bounds, class 1, which leaves the start state for state 2, so
// the machine starts on any glyph; there i turns into l.
#[test]
fn a_class_past_the_count_reads_as_out_of_bounds() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF, 0xFFFF], &[2, 0, 0xFFFF, 0xFFFF], &[0, 0, 0xFFFF, 0]];
    let start: &[u16] = &[0, 1, 0, 0, 1, 0, 0];
    let after: &[u16] = &[0, 1, 0, 0, 1, 2, 0];
    let substitutions = [u32s(&[4]), lookup(&[(I, L)])].concat();
    let body = stx_classed(7, lookup(&[(F, 516), (I, 5)]), &[start, start, after], &entries, &[substitutions]);
    let f = carets_with(&[("morx", morx(2, &[(1, body)]))]);
    glyphs(&f, &[("fi", &[F, L]), ("i", &[I])]);
}

// The search misses f, listed in class 4, so f reads as out of bounds, class 1, which leads to state 2, where i turns
// into l. A subset lists f in class 1, so i turns into l there as well.
#[test]
fn a_subset_keeps_a_glyph_the_search_misses_where_it_starts_the_machine() {
    let entries: [&[u16]; 3] = [&[0, 0, 0xFFFF, 0xFFFF], &[2, 0, 0xFFFF, 0xFFFF], &[0, 0, 0xFFFF, 0]];
    let start: &[u16] = &[0, 1, 0, 0, 1, 0, 0];
    let after: &[u16] = &[0, 1, 0, 0, 1, 2, 0];
    let classes = units(6, &[&[L, 6], &[I, 5], &[F, 4]], true);
    let substitutions = [u32s(&[4]), lookup(&[(I, L)])].concat();
    let body = stx_classed(7, classes, &[start, start, after], &entries, &[substitutions]);
    let f = carets_with(&[("morx", morx(2, &[(1, body)]))]);
    glyphs(&f, &[("fi", &[F, L])]);
}

// f is listed twice and the search finds it in class 5, idle; but the start state inserts l at the end of the text,
// which any text reaches in that state, so f and i alike are followed by l.
#[test]
fn a_start_state_acting_at_the_end_of_text_runs_on_any_text() {
    let entries: [&[u16]; 2] = [&[0, 0, 0xFFFF, 0xFFFF], &[0, 0x0020, 0, 0xFFFF]];
    let start: &[u16] = &[1, 0, 0, 0, 1, 0, 0];
    let classes = units(2, &[&[F, F, 5], &[F, F, 4]], true);
    let body = stx_classed(7, classes, &[start, start], &entries, &[u16s(&[L])]);
    let f = carets_with(&[("morx", morx(2, &[(5, body)]))]);
    glyphs(&f, &[("f", &[F, L]), ("i", &[I, L])]);
}
