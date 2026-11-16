use std::collections::BTreeMap;

use daegun::{Font, GlyphId, Location};

use super::FONTS;
use daegun::Tag;
use daegun::shape::ShapeOptions;

pub(super) fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font"), 0).expect("it parses")
}

pub(super) fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

pub(super) fn u32s(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// The glyph constants below as the ids the API takes and returns.
pub(super) fn glyph_ids(glyphs: &[u16]) -> Vec<GlyphId> {
    glyphs.iter().map(|&g| GlyphId(g.into())).collect()
}

// carets.ttf without its layout tables: .notdef, f, i, l, f_f_i, f_i and f_f_l, with f, i and l mapped.
pub(super) const F: u16 = 1;
pub(super) const I: u16 = 2;
pub(super) const L: u16 = 3;
pub(super) const F_F_I: u16 = 4;
pub(super) const F_I: u16 = 5;
pub(super) const F_F_L: u16 = 6;

pub(super) fn carets_with(extra: &[(&str, Vec<u8>)]) -> Font {
    base_with("test-fixtures/carets.ttf", extra)
}

// A fixture without its layout and kerning tables, plus `extra`.
pub(super) fn base_with(rel: &str, extra: &[(&str, Vec<u8>)]) -> Font {
    Font::from_vec(base_bytes(rel, extra), 0).expect("the patched font parses")
}

pub(super) fn base_bytes(rel: &str, extra: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let source = font(rel);
    let mut tables: BTreeMap<Tag, Vec<u8>> = source
        .table_tags()
        .filter(|t| ![Tag::new(b"GSUB"), Tag::new(b"GDEF"), Tag::new(b"kern")].contains(t))
        .map(|t| (t, source.table(t).expect("listed").to_vec()))
        .collect();
    tables.extend(extra.iter().map(|(t, b)| (t.parse().expect("a tag"), b.clone())));
    daegun::raw::build_font(&tables).unwrap()
}

// The font with `tag`'s stated length cut to `len`, its bytes left where they lay.
pub(super) fn cut(mut font: Vec<u8>, tag: &[u8; 4], len: u32) -> Vec<u8> {
    let tables = usize::from(u16::from_be_bytes([font[4], font[5]]));
    let entry = (0..tables).map(|i| 12 + 16 * i).find(|&e| &font[e..e + 4] == tag).expect("the table");
    font[entry + 12..entry + 16].copy_from_slice(&len.to_be_bytes());
    font
}

// A collection of the one face, its tables lying as they did, after the collection's header.
pub(super) fn one_face_collection(font: &[u8]) -> Vec<u8> {
    let mut face = font.to_vec();
    let tables = usize::from(u16::from_be_bytes([font[4], font[5]]));
    for at in (0..tables).map(|i| 20 + 16 * i) {
        let offset = u32::from_be_bytes(face[at..at + 4].try_into().expect("4 bytes")) + 16;
        face[at..at + 4].copy_from_slice(&offset.to_be_bytes());
    }
    [&b"ttcf"[..], &u16s(&[1, 0]), &u32s(&[1, 16]), &face].concat()
}

// The subset of `text` shapes `text` as the source does, glyph for glyph through the gid map.
pub(super) fn shapes_alike(f: &Font, text: &str) -> (Font, daegun::tables::Subset) {
    let want = f.shape(text, &Location::default(), &ShapeOptions::default());
    let out = f.subset_text(text, &Location::default()).expect("a subset");
    let sub = Font::from_vec(out.font.clone(), 0).expect("the subset parses");
    let got = sub.shape(text, &Location::default(), &ShapeOptions::default());
    let mapped: Vec<Option<GlyphId>> = want.glyphs.iter().map(|&g| out.renumbered(g)).collect();
    assert_eq!(mapped, got.glyphs.iter().map(|&g| Some(g)).collect::<Vec<_>>(), "{text:?} shapes otherwise");
    assert_eq!(want.advances, got.advances, "{text:?} advances otherwise");
    (sub, out)
}

fn glyphs(f: &Font, text: &str) -> Vec<GlyphId> {
    f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs.clone()
}

// A format 6 lookup with its 0xFFFF terminator.
pub(super) fn lookup(pairs: &[(u16, u16)]) -> Vec<u8> {
    let n = pairs.len() as u16;
    let selector = 15 - n.max(1).leading_zeros() as u16;
    let mut out = u16s(&[6, 4, n, 4 << selector, selector, 4 * n - (4 << selector)]);
    pairs.iter().for_each(|&(g, v)| out.extend(u16s(&[g, v])));
    out.extend(u16s(&[0xFFFF, 0]));
    out
}

// A morx of one chain holding these (type, body) subtables; version 3 ends the chain with a coverage
// offset per subtable, all 0.
pub(super) fn morx(version: u16, subtables: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut chain = Vec::new();
    for (kind, body) in subtables {
        chain.extend(u32s(&[(12 + body.len()) as u32, *kind, 1]));
        chain.extend(body);
    }
    if version >= 3 { chain.extend(vec![0; 4 * subtables.len()]); }
    let mut out = u16s(&[version, 0]);
    out.extend(u32s(&[1, 1, (16 + chain.len()) as u32, 0, subtables.len() as u32]));
    out.extend(chain);
    out
}

// An extended state table: its class lookup, the same row of entry indices for the two start states
// and any `more` states, the entries, then `tails` whose offsets follow the four header words.
pub(super) fn stx(
    n_classes: u16, classes: &[(u16, u16)], rows: &[&[u16]], entries: &[&[u16]], tails: &[Vec<u8>],
) -> Vec<u8> {
    stx_classed(n_classes, lookup(classes), rows, entries, tails)
}

// The same, its class lookup given as bytes.
pub(super) fn stx_classed(
    n_classes: u16, classes: Vec<u8>, rows: &[&[u16]], entries: &[&[u16]], tails: &[Vec<u8>],
) -> Vec<u8> {
    let class_at = 16 + 4 * tails.len();
    let state_at = class_at + classes.len();
    let states: Vec<u8> = rows.iter().flat_map(|r| {
        assert_eq!(r.len(), usize::from(n_classes));
        u16s(r)
    }).collect();
    let entry_at = state_at + states.len();
    let entries: Vec<u8> = entries.iter().flat_map(|e| u16s(e)).collect();
    let mut at = entry_at + entries.len();
    let mut header = u32s(&[u32::from(n_classes), class_at as u32, state_at as u32, entry_at as u32]);
    for t in tails {
        header.extend(u32s(&[at as u32]));
        at += t.len();
    }
    [header, classes, states, entries, tails.concat()].concat()
}

// Every string of up to three letters over each fixture's alphabet, subset to itself, shapes as the
// whole font shapes it: one font for each of the five morx subtable types.
#[test]
fn each_morx_fixture_shapes_alike_after_subsetting() {
    for (file, alphabet) in [
        ("aat/TestMORXTen.ttf", "ABX"),
        ("aat/TestMORXTwentyfour.ttf", "ABCDE"),
        ("aat/TestMORXFourtyone.ttf", "abc"),
        ("aat/TestMORXOne.ttf", "ABC"),
        ("aat/TestMORXThirtythree.ttf", "ah"),
    ] {
        let f = font(file);
        let letters: Vec<char> = alphabet.chars().collect();
        let mut texts: Vec<String> = letters.iter().map(char::to_string).collect();
        for _ in 0..2 {
            let longer: Vec<String> =
                texts.iter().flat_map(|t| letters.iter().map(move |c| format!("{t}{c}"))).collect();
            texts.extend(longer);
        }
        texts.sort();
        texts.dedup();
        for text in &texts {
            shapes_alike(&f, text);
        }
    }
}

// A subtable left with no glyph is written empty; the chain keeps it and the subtables beside it.
#[test]
fn an_emptied_subtable_keeps_its_chain() {
    let f = carets_with(&[("morx", morx(2, &[(4, lookup(&[(F, I)])), (4, lookup(&[(F_I, F_F_L)]))]))]);
    assert_eq!(glyphs(&f, "f"), glyph_ids(&[I]));
    let (sub, _) = shapes_alike(&f, "f");
    assert!(sub.table(Tag::new(b"morx")).is_some(), "the subset lost morx");
}

// Lookup 0, which glyph l's class names, changes no kept glyph. The offsets have no null value, so
// it must name an empty lookup, not offset 0, which reads the offset array as a lookup.
#[test]
fn a_contextual_lookup_left_empty_names_an_empty_lookup() {
    let substitutions =
        [u32s(&[8, 8 + lookup(&[(F_I, F_F_L)]).len() as u32]), lookup(&[(F_I, F_F_L)]), lookup(&[(L, F)])].concat();
    let row: &[u16] = &[0, 0, 0, 0, 1];
    let body = stx(
        5,
        &[(L, 4)],
        &[row, row],
        &[&[0, 0, 0xFFFF, 0xFFFF], &[0, 0, 0xFFFF, 0], &[0, 0, 1, 0xFFFF]],
        &[substitutions],
    );
    let f = carets_with(&[("morx", morx(2, &[(1, body)]))]);
    assert_eq!(glyphs(&f, "l"), glyph_ids(&[L]));
    shapes_alike(&f, "l");
}

// A substitution to 0xFFFF deletes the glyph; the deleted glyph is never kept, but the pair must be.
#[test]
fn a_substitution_to_the_deleted_glyph_still_deletes() {
    let f = carets_with(&[("morx", morx(2, &[(4, lookup(&[(F, 0xFFFF), (I, L)]))]))]);
    assert_eq!(glyphs(&f, "fi"), glyph_ids(&[L]));
    shapes_alike(&f, "fi");
}

// Zapfino hands glyph ids past its last glyph from one subtable to the next. Here f becomes 32000,
// which a later subtable turns into i.
#[test]
fn a_placeholder_glyph_past_the_font_carries_through_a_subset() {
    let f = carets_with(&[("morx", morx(2, &[(4, lookup(&[(F, 32_000)])), (4, lookup(&[(32_000, I)]))]))]);
    assert_eq!(glyphs(&f, "f"), glyph_ids(&[I]));
    shapes_alike(&f, "f");
}

pub(super) const SET_COMPONENT: u16 = 0x8000;
pub(super) const PERFORM_ACTION: u16 = 0x2000;

// f then i or l forms f_i or f_f_l: fi sums to 1 and fl to 2. The ligature list comes first, so a
// closure reading it to the subtable's end would take the actions and an unused f_f_i for glyphs.
fn ligatures() -> Font {
    let start = [0, 0, 0, 0, 1, 0];
    let after_f = [0, 0, 0, 0, 1, 2];
    let entries: [&[u16]; 3] = [&[0, 0, 0], &[2, SET_COMPONENT, 0], &[0, SET_COMPONENT | PERFORM_ACTION, 0]];
    let tails = [
        u32s(&[0x3FFF_FFFE, 0x8000_0000]),
        u16s(&[0, 1, 1, F_F_I]),
        u16s(&[0, F_I, F_F_L]),
    ];
    let mut body = stx(6, &[(F, 4), (I, 5), (L, 5)], &[&start, &start, &after_f], &entries, &tails);
    // Move the ligature list ahead of the actions and components, keeping each offset right.
    let (action, component, ligature) = (be32(&body, 16), be32(&body, 20), be32(&body, 24));
    let lists = body.split_off(action);
    let (actions, rest) = lists.split_at(component - action);
    let (components, ligs) = rest.split_at(ligature - component);
    body.extend(ligs);
    body.extend(actions);
    body.extend(components);
    let at = |v: usize| (v as u32).to_be_bytes();
    body[24..28].copy_from_slice(&at(action));
    body[16..20].copy_from_slice(&at(action + ligs.len()));
    body[20..24].copy_from_slice(&at(action + ligs.len() + actions.len()));
    carets_with(&[("morx", morx(2, &[(2, body)]))])
}

fn be32(b: &[u8], at: usize) -> usize {
    u32::from_be_bytes(b[at..at + 4].try_into().expect("four bytes")) as usize
}

// The closure keeps only what kept glyphs can form: fi reaches f_i, never f_f_l or f_f_i.
#[test]
fn a_ligature_closure_keeps_only_the_ligatures_kept_glyphs_form() {
    let f = ligatures();
    assert_eq!(glyphs(&f, "fi"), glyph_ids(&[F_I]));
    assert_eq!(glyphs(&f, "fl"), glyph_ids(&[F_F_L]));
    assert_eq!(
        f.glyph_closure(glyph_ids(&[F, I]), &Location::default()).expect("a closure"),
        glyph_ids(&[0, F, I, F_I])
    );
    shapes_alike(&f, "fi");
    shapes_alike(&f, "fl");
    shapes_alike(&f, "fil");
}

// f then i forms f_i through two words; l is never pushed, so no text forms f_f_l or f_f_i, and a subset keeps neither
// yet shapes every text of up to four letters as the font does.
#[test]
fn a_ligature_closure_reads_only_what_the_machine_pushes_at_each_depth() {
    let start = [0, 0, 0, 0, 1, 0, 0];
    let after_f = [0, 0, 0, 0, 1, 2, 0];
    let entries: [&[u16]; 3] = [&[0, 0, 0], &[2, SET_COMPONENT, 0], &[0, SET_COMPONENT | PERFORM_ACTION, 0]];
    let tails = [u32s(&[0, 0x8000_0004]), u16s(&[0, 0, 0, 1, 0, 0, 0, 2]), u16s(&[F_I, F_F_L, F_F_I])];
    let body = stx(7, &[(F, 4), (I, 5), (L, 6)], &[&start, &start, &after_f], &entries, &tails);
    let f = carets_with(&[("morx", morx(2, &[(2, body)]))]);
    let want: [(&str, &[u16]); 5] =
        [("fi", &[F_I]), ("fil", &[F_I, L]), ("ffi", &[F, F_I]), ("lfi", &[L, F_I]), ("fli", &[F, L, I])];
    for (text, ids) in want {
        assert_eq!(glyphs(&f, text), glyph_ids(ids), "{text}");
    }
    assert_eq!(
        f.glyph_closure(glyph_ids(&[F, I, L]), &Location::default()).expect("a closure"),
        glyph_ids(&[0, F, I, L, F_I])
    );
    for n in 1..=4 {
        for k in 0..3usize.pow(n) {
            let text: String = (0..n).map(|d| ['f', 'i', 'l'][k / 3usize.pow(d) % 3]).collect();
            shapes_alike(&f, &text);
        }
    }
    // The one subtable's body follows the morx header, the chain's and its own; i lands at 2 and f at 1 + 4.
    let (sub, _) = shapes_alike(&f, "fil");
    let body = &sub.table(Tag::new(b"morx")).expect("morx stays")[8 + 16 + 12..];
    assert_eq!((be32(body, 24) - be32(body, 20)) / 2, 6, "components placed past i's and f's");
}

// As `ligatures`, but l is missing from the class table and reads class 1, whose entry after f pushes it and acts.
#[test]
fn a_glyph_the_class_table_leaves_out_keeps_its_ligature_in_a_subset() {
    let start = [0, 0, 0, 0, 1, 0];
    let after_f = [0, 2, 0, 0, 1, 2];
    let entries: [&[u16]; 3] = [&[0, 0, 0], &[2, SET_COMPONENT, 0], &[0, SET_COMPONENT | PERFORM_ACTION, 0]];
    let tails = [u32s(&[0x3FFF_FFFE, 0x8000_0000]), u16s(&[0, 1, 1, F_F_I]), u16s(&[0, F_I, F_F_L])];
    let body = stx(6, &[(F, 4), (I, 5)], &[&start, &start, &after_f], &entries, &tails);
    let f = carets_with(&[("morx", morx(2, &[(2, body)]))]);
    assert_eq!(glyphs(&f, "fl"), glyph_ids(&[F_F_L]));
    assert_eq!(glyphs(&f, "ffl"), glyph_ids(&[F, F_F_L]));
    for text in ["fl", "ffl", "fil", "lfl"] {
        shapes_alike(&f, text);
    }
}

// l's class inserts f_f_i before it. With l out of the subset no entry reaches that insertion.
#[test]
fn an_insertion_only_an_absent_glyph_reaches_stays_out_of_the_closure() {
    const CURRENT_INSERT_ONE: u16 = 0x0020 | 0x0800;
    let row: &[u16] = &[0, 0, 0, 0, 1];
    let body = stx(
        5,
        &[(L, 4)],
        &[row, row],
        &[&[0, 0, 0xFFFF, 0xFFFF], &[0, CURRENT_INSERT_ONE, 0, 0xFFFF]],
        &[u16s(&[F_F_I])],
    );
    let f = carets_with(&[("morx", morx(2, &[(5, body)]))]);
    assert_eq!(glyphs(&f, "l"), glyph_ids(&[F_F_I, L]));
    assert_eq!(f.glyph_closure(glyph_ids(&[F, I]), &Location::default()).expect("a closure"), glyph_ids(&[0, F, I]));
    shapes_alike(&f, "l");
    shapes_alike(&f, "fi");
}

// Each subtable and the chain stay a multiple of four bytes, and version 3 keeps its coverage array:
// one offset per subtable after the last, 0 for none.
#[test]
fn a_subset_morx_keeps_its_alignment_and_version_3_coverage() {
    let row: &[u16] = &[0, 0, 0, 0, 1];
    let insertion =
        stx(5, &[(L, 4)], &[row, row], &[&[0, 0, 0xFFFF, 0xFFFF], &[0, 0x0020 | 0x0800, 0, 0xFFFF]], &[u16s(&[F])]);
    let f = carets_with(&[("morx", morx(3, &[(5, insertion), (4, lookup(&[(F, I)]))]))]);
    let (sub, _) = shapes_alike(&f, "lf");
    let m = sub.table(Tag::new(b"morx")).expect("morx");
    let chain_len = be32(m, 12);
    assert_eq!(chain_len % 4, 0, "chainLength is not a multiple of 4");
    let mut at = 8 + 16;
    for _ in 0..be32(m, 20) {
        assert_eq!(be32(m, at) % 4, 0, "a subtable's length is not a multiple of 4");
        at += be32(m, at);
    }
    assert_eq!(&m[at..8 + chain_len], [0; 8], "the coverage array is missing");
}

// A kerning pair for f and i in formats 0, 2 and 6, each of which must survive a subset.
#[test]
fn kerx_class_and_pair_formats_survive_a_subset() {
    let classes = |pairs: &[(u16, u16)]| lookup(pairs);
    // Format 2 measures its offsets from the subtable's start, header included.
    let left = classes(&[(F, 4)]);
    let right = classes(&[(I, 2)]);
    let array = u16s(&[0, 0, 0, (-50i16) as u16]);
    let format2 = [
        u32s(&[4, 28, 28 + left.len() as u32, (28 + left.len() + right.len()) as u32]),
        left.clone(),
        right.clone(),
        array,
    ]
    .concat();
    let rows = classes(&[(F, 2)]);
    let cols = classes(&[(I, 1)]);
    let values = u16s(&[0, 0, 0, (-70i16) as u16]);
    let format6 = [
        u32s(&[0]),
        u16s(&[2, 2]),
        u32s(&[32, 32 + rows.len() as u32, (32 + rows.len() + cols.len()) as u32]),
        rows,
        cols,
        values,
    ]
    .concat();
    let format0 = [u32s(&[1, 6, 0, 0]), u16s(&[F, L, (-30i16) as u16])].concat();
    for (format, body, kern) in [(2u32, format2, -50.0), (6, format6, -70.0), (0, format0, -30.0)] {
        let sub = [u32s(&[(12 + body.len()) as u32, format, 0]), body].concat();
        let kerx = [u16s(&[2, 0]), u32s(&[1]), sub].concat();
        let f = carets_with(&[("kerx", kerx)]);
        let text = if format == 0 { "fl" } else { "fi" };
        // The shaper splits a pair's kerning between its two glyphs, so only the run's total advance carries it whole.
        let total =
            |f: &Font| f.shape(text, &Location::default(), &ShapeOptions::default()).advances.iter().sum::<f64>();
        let (plain, kerned) = (total(&carets_with(&[])), total(&f));
        assert!(
            (kerned - plain - kern * 1000.0 / f64::from(f.units_per_em())).abs() < 1e-6,
            "format {format} kerned {kerned} against {plain}",
        );
        shapes_alike(&f, text);
    }
}

// Version 3 and later end with a coverage bitfield offset per subtable, 0xFFFFFFFF for none.
#[test]
fn a_subset_kerx_keeps_its_version_3_coverage() {
    let body = [u32s(&[1, 6, 0, 0]), u16s(&[F, L, (-30i16) as u16])].concat();
    let sub = [u32s(&[(12 + body.len()) as u32, 0, 0]), body].concat();
    let kerx = [u16s(&[3, 0]), u32s(&[1]), sub.clone(), u32s(&[u32::MAX])].concat();
    let f = carets_with(&[("kerx", kerx)]);
    let (subset, _) = shapes_alike(&f, "fl");
    let k = subset.table(Tag::new(b"kerx")).expect("kerx");
    assert_eq!(&k[k.len() - 4..], [0xFF; 4], "the coverage array is missing");
    assert_eq!(be32(k, 8) + 8 + 4, k.len(), "something other than one subtable and its offset");
}

// bsln format 2 and fmtx each take their measures from one glyph, here l and f_f_l, which a subset
// of "f" never asks for. The closure keeps both, so both tables survive.
#[test]
fn the_glyphs_bsln_and_fmtx_measure_by_stay_in_a_subset() {
    let bsln = [u16s(&[1, 0, 2, 0, L]), vec![0; 64]].concat();
    let fmtx = [0x0002_0000u32, u32::from(F_F_L), 0, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
    let f = carets_with(&[("bsln", bsln), ("fmtx", fmtx)]);
    assert_eq!(
        f.glyph_closure(glyph_ids(&[F]), &Location::default()).expect("a closure"),
        glyph_ids(&[0, F, L, F_F_L])
    );
    let (sub, out) = shapes_alike(&f, "f");
    let new = |g| out.renumbered(g).expect("kept");
    assert_eq!(
        sub.table(Tag::new(b"bsln")).map(|b| GlyphId(u16::from_be_bytes([b[8], b[9]]).into())),
        Some(new(GlyphId(L.into())))
    );
    assert_eq!(
        sub.table(Tag::new(b"fmtx")).map(|t| GlyphId(u32::from_be_bytes([t[4], t[5], t[6], t[7]]))),
        Some(new(GlyphId(F_F_L.into())))
    );
}

// xref names subtables by index. A subset that drops one of morx's (here a type it cannot rebuild)
// must drop morx's names with it, and keep the rest.
#[test]
fn xref_drops_the_names_of_a_table_whose_subtables_moved() {
    let entry = |tag: &[u8; 4], at: u16, len: u16| [&tag[..], &[0; 8], &at.to_be_bytes(), &len.to_be_bytes()].concat();
    let xref = [
        vec![0, 1, 0, 0, 0, 0, 0, 0],
        u32s(&[2, 48]),
        entry(b"morx", 0, 4),
        entry(b"feat", 4, 4),
        b"SwshBold".to_vec(),
    ]
    .concat();
    let glyf = carets_with(&[("morx", morx(2, &[(4, lookup(&[(F, I)])), (3, vec![0; 8])])), ("xref", xref.clone())]);
    // TestKERNOne is a CFF font, whose subsets take the other path: T, dotlessi, u and space.
    let cff = base_with(
        "aat/TestKERNOne.otf",
        &[("morx", morx(2, &[(4, lookup(&[(1, 3)])), (3, vec![0; 8])])), ("xref", xref)],
    );
    for (f, text) in [(glyf, "f"), (cff, "T")] {
        let (sub, _) = shapes_alike(&f, text);
        let x = sub.table(Tag::new(b"xref")).expect("xref keeps its feat name");
        assert_eq!((be32(x, 8), &x[x.len() - 4..]), (1, &b"Bold"[..]));
    }
}

// A chain names two subtables but its length holds only the first. The shaper walks them by their own lengths, past
// the chain's end, so f still becomes f_i, and a subset keeps the second too.
#[test]
fn a_subtable_past_its_chains_end_runs_in_a_subset_too() {
    let (first, second) = (lookup(&[(L, I)]), lookup(&[(F, F_I)]));
    let mut t = u16s(&[2, 0]);
    t.extend(u32s(&[1, 1, (16 + 12 + first.len()) as u32, 0, 2]));
    for body in [first, second] {
        t.extend([u32s(&[(12 + body.len()) as u32, 4, 1]), body].concat());
    }
    let f = carets_with(&[("morx", t)]);
    assert_eq!(glyphs(&f, "f"), glyph_ids(&[F_I]));
    assert_eq!(glyphs(&f, "fl"), glyph_ids(&[F_I, I]));
    shapes_alike(&f, "f");
    shapes_alike(&f, "fl");
}

// A morx of one chain whose one subtable of `kind` counts only its first `own` bytes of `body` as its own: the
// rest lies past the subtable's length, still within the chain.
fn morx_past_own(kind: u32, body: Vec<u8>, own: usize) -> Vec<u8> {
    let mut t = u16s(&[2, 0]);
    t.extend(u32s(&[1, 1, (16 + 12 + body.len()) as u32, 0, 1, (12 + own) as u32, kind, 1]));
    t.extend(body);
    t
}

// A subtable is read against the whole table, so a ligature's arrays, an insertion's glyphs and a contextual
// subtable's substitutions count past its own length here, and a subset keeps each.
#[test]
fn data_past_a_subtables_own_length_is_kept_in_a_subset() {
    let start = [0, 0, 0, 0, 1, 0];
    let after_f = [0, 0, 0, 0, 1, 2];
    let entries: [&[u16]; 3] = [&[0, 0, 0], &[2, SET_COMPONENT, 0], &[0, SET_COMPONENT | PERFORM_ACTION, 0]];
    let tails = [u32s(&[0x3FFF_FFFE, 0x8000_0000]), u16s(&[0, 1, 1, F_F_I]), u16s(&[0, F_I, F_F_L])];
    let body = stx(6, &[(F, 4), (I, 5), (L, 5)], &[&start, &start, &after_f], &entries, &tails);
    let own = be32(&body, 16);
    let ligature = carets_with(&[("morx", morx_past_own(2, body, own))]);
    assert_eq!(glyphs(&ligature, "fi"), glyph_ids(&[F_I]));
    assert_eq!(glyphs(&ligature, "fl"), glyph_ids(&[F_F_L]));

    const CURRENT_INSERT_ONE: u16 = 0x0020 | 0x0800;
    let row: &[u16] = &[0, 0, 0, 0, 1];
    let body = stx(
        5,
        &[(L, 4)],
        &[row, row],
        &[&[0, 0, 0xFFFF, 0xFFFF], &[0, CURRENT_INSERT_ONE, 0, 0xFFFF]],
        &[u16s(&[F_F_I])],
    );
    let own = be32(&body, 16);
    let insertion = carets_with(&[("morx", morx_past_own(5, body, own))]);
    assert_eq!(glyphs(&insertion, "l"), glyph_ids(&[F_F_I, L]));

    let substitutions = [u32s(&[4]), lookup(&[(F, F_I)])].concat();
    let body = stx(5, &[(F, 4)], &[row, row], &[&[0, 0, 0xFFFF, 0xFFFF], &[0, 0, 0xFFFF, 0]], &[substitutions]);
    let own = be32(&body, 16);
    let contextual = carets_with(&[("morx", morx_past_own(1, body, own))]);
    assert_eq!(glyphs(&contextual, "f"), glyph_ids(&[F_I]));

    for (f, texts) in [(&ligature, &["fi", "fl", "fil"][..]), (&insertion, &["l", "fl"]), (&contextual, &["f", "fi"])] {
        texts.iter().for_each(|text| { shapes_alike(f, text); });
    }
}

// A machine reads rows and entries wherever they lie, a contextual state 2 in the substitution table and a ligature
// entry 3 among the components, and a subset keeps them.
#[test]
fn a_subset_keeps_the_rows_and_entries_a_machine_reaches_past_their_parts() {
    let place = |header: usize, parts: &[&[u8]], order: &[usize]| {
        let mut at = header;
        let mut offsets = vec![0u32; parts.len()];
        for &i in order {
            offsets[i] = at as u32;
            at += parts[i].len();
        }
        let body: Vec<u8> = order.iter().flat_map(|&i| parts[i].to_vec()).collect();
        (offsets, body)
    };
    let row = u16s(&[0, 0, 0, 0, 0, 2]);
    let empty = u16s(&[0, 0, 0xFFFF, 0xFFFF]);
    let entries = [empty.clone(), u16s(&[0, 0, 0xFFFF, 0]), u16s(&[2, 0, 0xFFFF, 0xFFFF]), empty.repeat(4)].concat();
    let substitutions = [u32s(&[4]), lookup(&[(F, F_I)])].concat();
    let parts: [&[u8]; 4] = [&lookup(&[(F, 4), (L, 5)]), &row.repeat(2), &entries, &substitutions];
    let (at, body) = place(20, &parts, &[0, 1, 3, 2]);
    let contextual = carets_with(&[("morx", morx(2, &[(1, [u32s(&[6]), u32s(&at), body].concat())]))]);
    assert_eq!(glyphs(&contextual, "lf"), glyph_ids(&[L, F_I]));

    let rows = [u16s(&[0, 0, 0, 0, 1, 0]).repeat(2), u16s(&[0, 0, 0, 0, 1, 3])].concat();
    let entries = u16s(&[0, 0, 0, 2, SET_COMPONENT, 0, 0, SET_COMPONENT | PERFORM_ACTION, 0]);
    let components = u16s(&[0, SET_COMPONENT | PERFORM_ACTION, 0, 0, 1, 1, F_F_I]);
    let (classes, actions) = (lookup(&[(F, 4), (I, 5), (L, 5)]), u32s(&[1, 0x8000_0003]));
    let parts: [&[u8]; 6] = [&classes, &rows, &entries, &actions, &components, &u16s(&[0, F_I, F_F_L])];
    let (at, body) = place(28, &parts, &[0, 1, 2, 4, 3, 5]);
    let ligature = carets_with(&[("morx", morx(2, &[(2, [u32s(&[6]), u32s(&at), body].concat())]))]);
    assert_eq!((glyphs(&ligature, "fi"), glyphs(&ligature, "fl")), (glyph_ids(&[F_I]), glyph_ids(&[F_F_L])));

    for (f, texts) in [(&contextual, &["lf", "f", "fl"][..]), (&ligature, &["fi", "fl", "fil", "ffl"])] {
        texts.iter().for_each(|text| { shapes_alike(f, text); });
    }
}

// A kerx subtable reads only its own length: it holds one pair of the two it counts, so fl's, past it, kerns nothing,
// in the font and a subset.
#[test]
fn a_kerx_subtable_reads_only_its_own_length_in_a_subset() {
    let body = [u32s(&[2, 12, 1, 0]), u16s(&[F, I, (-40i16) as u16])].concat();
    let t = [u16s(&[2, 0]), u32s(&[1, (12 + body.len()) as u32, 0, 0]), body, u16s(&[F, L, (-20i16) as u16])].concat();
    let f = carets_with(&[("kerx", t)]);
    assert_eq!(f.shape("fl", &Location::default(), &ShapeOptions::default()).advances, [400.0; 2]);
    for text in ["fi", "fl", "ffl"] {
        shapes_alike(&f, text);
    }
}

// l inserts f_f_i before it, and i asks for three glyphs from a list that runs past the table, which inserts
// nothing. A subset without i never reaches i's list, so it keeps the subtable for l.
#[test]
fn an_insertion_list_past_the_table_only_an_absent_glyph_reaches_keeps_its_subtable() {
    const CURRENT_INSERT_ONE: u16 = 0x0020 | 0x0800;
    const CURRENT_INSERT_THREE: u16 = 0x0060 | 0x0800;
    let row: &[u16] = &[0, 0, 0, 0, 1, 2];
    let entries: [&[u16]; 3] =
        [&[0, 0, 0xFFFF, 0xFFFF], &[0, CURRENT_INSERT_ONE, 0, 0xFFFF], &[0, CURRENT_INSERT_THREE, 0, 0xFFFF]];
    let body = stx(6, &[(I, 5), (L, 4)], &[row, row], &entries, &[u16s(&[F_F_I])]);
    let f = carets_with(&[("morx", morx(2, &[(5, body)]))]);
    assert_eq!(glyphs(&f, "l"), glyph_ids(&[F_F_I, L]));
    assert_eq!(glyphs(&f, "i"), glyph_ids(&[I]));
    shapes_alike(&f, "l");
    shapes_alike(&f, "fl");
}

// i inserts three glyphs from l's list of one, past the table, so nothing; a subset keeps them after the last chain,
// or, l's list at glyph 65,534, gives i's a count of 0.
#[test]
fn an_insertion_list_past_the_table_stays_past_it_in_a_subset() {
    const CURRENT_INSERT_ONE: u16 = 0x0020 | 0x0800;
    const CURRENT_INSERT_THREE: u16 = 0x0060 | 0x0800;
    let row: &[u16] = &[0, 0, 0, 0, 1, 2];
    for (at, list) in [(0, u16s(&[F_F_I])), (65_534, u16s(&[[0; 65_534].as_slice(), &[F_F_I]].concat()))] {
        let entries: [&[u16]; 3] =
            [&[0, 0, 0xFFFF, 0xFFFF], &[0, CURRENT_INSERT_ONE, at, 0xFFFF], &[0, CURRENT_INSERT_THREE, at, 0xFFFF]];
        let f = carets_with(&[("morx", morx(2, &[(5, stx(6, &[(I, 5), (L, 4)], &[row, row], &entries, &[list]))]))]);
        assert_eq!((glyphs(&f, "il"), glyphs(&f, "li")), (glyph_ids(&[I, F_F_I, L]), glyph_ids(&[F_F_I, L, I])));
        for text in ["il", "li", "fil", "i"] {
            shapes_alike(&f, text);
        }
    }
}

// An OpenType `kern` of (flags, format, body) subtables; flag 1 marks a horizontal one.
pub(super) fn kern_ot(subtables: &[(u8, u8, Vec<u8>)]) -> Vec<u8> {
    let mut out = u16s(&[0, subtables.len() as u16]);
    for (flags, format, body) in subtables {
        out.extend(u16s(&[0, (6 + body.len()) as u16]));
        out.extend([*format, *flags]);
        out.extend(body);
    }
    out
}

// With i a mark, the plan zeroes it unless a `kern` subtable is a machine, and moves it over its base unless one kerns
// across the stream, run or not; a subset of fi keeps such subtables, however emptied.
#[test]
fn a_kern_subset_keeps_every_subtable_its_plan_counts() {
    use super::aat_shaping::{gdef, kern, kern_machine};
    const ACROSS: u8 = 0x40;
    const VARIATION: u8 = 0x20;
    let ll = [u16s(&[1, 6, 0, 0]), u16s(&[L, L, (-50i16) as u16])].concat();
    let f_against_l = u16s(&[2, 16, 22, 28, F, 1, 28, L, 1, 0, (-50i16) as u16]);
    let (kept, zeroed) = (([400.0, 400.0], (0.0, 0.0)), ([400.0, 0.0], (0.0, 0.0)));
    let cases = [
        (kern(&[(VARIATION, 1, kern_machine(0, &[-40, 61]))]), kept),
        (kern(&[(ACROSS, 0, ll.clone())]), zeroed),
        (kern(&[(ACROSS, 2, f_against_l)]), zeroed),
        (kern(&[(ACROSS, 4, Vec::new())]), zeroed),
        (kern_ot(&[(0x05, 0, ll)]), zeroed),
        (kern_ot(&[(0x05, 4, Vec::new())]), zeroed),
    ];
    for (i, (table, (advances, offset))) in cases.into_iter().enumerate() {
        let f = carets_with(&[("GDEF", gdef(&[(I, I, 3)])), ("kern", table)]);
        let run = f.shape("fi", &Location::default(), &ShapeOptions::default());
        assert_eq!(run.advances, advances, "case {i}");
        assert_eq!(crate::xy(&run.offsets), [(0.0, 0.0), offset], "case {i}");
        let (sub, _) = shapes_alike(&f, "fi");
        assert_eq!(
            sub.shape("fi", &Location::default(), &ShapeOptions::default()).offsets,
            run.offsets,
            "case {i}, subset"
        );
    }
}

// Apple's `kern` gives each subtable a 32-bit length, so its last reads only that: fi's pair past it kerns nothing, in
// the font and a subset.
#[test]
fn apples_last_kern_subtable_reads_only_its_own_length_in_a_subset() {
    let mut t = super::aat_shaping::kern(&[(0, 0, u16s(&[2, 12, 1, 0, F, F, (-20i16) as u16]))]);
    t.extend(u16s(&[F, I, (-40i16) as u16]));
    let f = carets_with(&[("kern", t)]);
    assert_eq!(f.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [400.0, 400.0]);
    shapes_alike(&f, "fi");
}

// A `kern` of version 2 is neither OpenType's version 0 nor Apple's 1.0, so it applies nothing and a subset leaves it
// out.
#[test]
fn a_kern_of_another_version_is_left_out_of_a_subset() {
    let f = carets_with(&[("kern", u16s(&[2, 0]))]);
    let (sub, _) = shapes_alike(&f, "fi");
    assert_eq!(sub.table(Tag::new(b"kern")), None);
}
