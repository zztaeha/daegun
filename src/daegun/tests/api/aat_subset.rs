use std::collections::BTreeMap;

use daegun::Font;

use super::FONTS;

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font")).expect("it parses")
}

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn u32s(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// carets.ttf without its layout tables: .notdef, f, i, l, f_f_i, f_i and f_f_l, with f, i and l mapped.
const F: u16 = 1;
const I: u16 = 2;
const L: u16 = 3;
const F_F_I: u16 = 4;
const F_I: u16 = 5;
const F_F_L: u16 = 6;

fn carets_with(extra: &[(&str, Vec<u8>)]) -> Font {
    base_with("test-fixtures/carets.ttf", extra)
}

// A fixture without its layout and kerning tables, plus `extra`.
fn base_with(rel: &str, extra: &[(&str, Vec<u8>)]) -> Font {
    let source = font(rel);
    let mut tables: BTreeMap<String, Vec<u8>> = source
        .table_tags()
        .into_iter()
        .filter(|t| !matches!(*t, "GSUB" | "GDEF" | "kern"))
        .map(|t| (t.to_string(), source.table(t).expect("listed").to_vec()))
        .collect();
    tables.extend(extra.iter().map(|(t, b)| (t.to_string(), b.clone())));
    Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses")
}

// The subset of `text` shapes `text` as the source does, glyph for glyph through the gid map.
fn shapes_alike(f: &Font, text: &str) -> (Font, daegun::SubsetResult) {
    let want = f.shape(text, &[], false).expect("the source shapes");
    let out = f.subset_text(text, &[]).expect("a subset");
    let sub = Font::from_vec(out.ttf.clone()).expect("the subset parses");
    let got = sub.shape(text, &[], false).expect("the subset shapes");
    let mapped: Vec<Option<u16>> = want.glyphs.iter().map(|&g| out.new_gid(g)).collect();
    assert_eq!(mapped, got.glyphs.iter().map(|&g| Some(g)).collect::<Vec<_>>(), "{text:?} shapes otherwise");
    assert_eq!(want.advances, got.advances, "{text:?} advances otherwise");
    (sub, out)
}

fn glyphs(f: &Font, text: &str) -> Vec<u16> {
    f.shape(text, &[], false).expect("shapes").glyphs.to_vec()
}

// A format 6 lookup with its 0xFFFF terminator.
fn lookup(pairs: &[(u16, u16)]) -> Vec<u8> {
    let n = pairs.len() as u16;
    let selector = 15 - n.max(1).leading_zeros() as u16;
    let mut out = u16s(&[6, 4, n, 4 << selector, selector, 4 * n - (4 << selector)]);
    pairs.iter().for_each(|&(g, v)| out.extend(u16s(&[g, v])));
    out.extend(u16s(&[0xFFFF, 0]));
    out
}

// A morx of one chain holding these (type, body) subtables; version 3 ends the chain with a coverage
// offset per subtable, all 0.
fn morx(version: u16, subtables: &[(u32, Vec<u8>)]) -> Vec<u8> {
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
fn stx(n_classes: u16, classes: &[(u16, u16)], rows: &[&[u16]], entries: &[&[u16]], tails: &[Vec<u8>]) -> Vec<u8> {
    let class_at = 16 + 4 * tails.len();
    let classes = lookup(classes);
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
            let longer: Vec<String> = texts.iter().flat_map(|t| letters.iter().map(move |c| format!("{t}{c}"))).collect();
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
    assert_eq!(glyphs(&f, "f"), [I]);
    let (sub, _) = shapes_alike(&f, "f");
    assert!(sub.table("morx").is_some(), "the subset lost morx");
}

// Lookup 0, which glyph l's class names, changes no kept glyph. The offsets have no null value, so
// it must name an empty lookup, not offset 0, which reads the offset array as a lookup.
#[test]
fn a_contextual_lookup_left_empty_names_an_empty_lookup() {
    let substitutions = [u32s(&[8, 8 + lookup(&[(F_I, F_F_L)]).len() as u32]), lookup(&[(F_I, F_F_L)]), lookup(&[(L, F)])].concat();
    let row: &[u16] = &[0, 0, 0, 0, 1];
    let body = stx(5, &[(L, 4)], &[row, row], &[&[0, 0, 0xFFFF, 0xFFFF], &[0, 0, 0xFFFF, 0], &[0, 0, 1, 0xFFFF]], &[substitutions]);
    let f = carets_with(&[("morx", morx(2, &[(1, body)]))]);
    assert_eq!(glyphs(&f, "l"), [L]);
    shapes_alike(&f, "l");
}

// A substitution to 0xFFFF deletes the glyph; the deleted glyph is never kept, but the pair must be.
#[test]
fn a_substitution_to_the_deleted_glyph_still_deletes() {
    let f = carets_with(&[("morx", morx(2, &[(4, lookup(&[(F, 0xFFFF), (I, L)]))]))]);
    assert_eq!(glyphs(&f, "fi"), [L]);
    shapes_alike(&f, "fi");
}

// Zapfino hands glyph ids past its last glyph from one subtable to the next. Here f becomes 32000,
// which a later subtable turns into i.
#[test]
fn a_placeholder_glyph_past_the_font_carries_through_a_subset() {
    let f = carets_with(&[("morx", morx(2, &[(4, lookup(&[(F, 32_000)])), (4, lookup(&[(32_000, I)]))]))]);
    assert_eq!(glyphs(&f, "f"), [I]);
    shapes_alike(&f, "f");
}

const SET_COMPONENT: u16 = 0x8000;
const PERFORM_ACTION: u16 = 0x2000;

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
    assert_eq!(glyphs(&f, "fi"), [F_I]);
    assert_eq!(glyphs(&f, "fl"), [F_F_L]);
    assert_eq!(f.glyph_closure(&[F, I], &[]).expect("a closure"), [0, F, I, F_I]);
    shapes_alike(&f, "fi");
    shapes_alike(&f, "fl");
    shapes_alike(&f, "fil");
}

// l's class inserts f_f_i before it. With l out of the subset no entry reaches that insertion.
#[test]
fn an_insertion_only_an_absent_glyph_reaches_stays_out_of_the_closure() {
    const CURRENT_INSERT_ONE: u16 = 0x0020 | 0x0800;
    let row: &[u16] = &[0, 0, 0, 0, 1];
    let body = stx(5, &[(L, 4)], &[row, row], &[&[0, 0, 0xFFFF, 0xFFFF], &[0, CURRENT_INSERT_ONE, 0, 0xFFFF]], &[u16s(&[F_F_I])]);
    let f = carets_with(&[("morx", morx(2, &[(5, body)]))]);
    assert_eq!(glyphs(&f, "l"), [F_F_I, L]);
    assert_eq!(f.glyph_closure(&[F, I], &[]).expect("a closure"), [0, F, I]);
    shapes_alike(&f, "l");
    shapes_alike(&f, "fi");
}

// Each subtable and the chain stay a multiple of four bytes, and version 3 keeps its coverage array:
// one offset per subtable after the last, 0 for none.
#[test]
fn a_subset_morx_keeps_its_alignment_and_version_3_coverage() {
    let row: &[u16] = &[0, 0, 0, 0, 1];
    let insertion = stx(5, &[(L, 4)], &[row, row], &[&[0, 0, 0xFFFF, 0xFFFF], &[0, 0x0020 | 0x0800, 0, 0xFFFF]], &[u16s(&[F])]);
    let f = carets_with(&[("morx", morx(3, &[(5, insertion), (4, lookup(&[(F, I)]))]))]);
    let (sub, _) = shapes_alike(&f, "lf");
    let m = sub.table("morx").expect("morx");
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
    let left = classes(&[(F, 2)]);
    let right = classes(&[(I, 1)]);
    let array = u16s(&[0, 0, 0, (-50i16) as u16]);
    let format2 = [u32s(&[4, 28, 28 + left.len() as u32, (28 + left.len() + right.len()) as u32]), left.clone(), right.clone(), array].concat();
    let rows = classes(&[(F, 2)]);
    let cols = classes(&[(I, 1)]);
    let values = u16s(&[0, 0, 0, (-70i16) as u16]);
    let format6 = [u32s(&[0]), u16s(&[2, 2]), u32s(&[32, 32 + rows.len() as u32, (32 + rows.len() + cols.len()) as u32]), rows, cols, values].concat();
    let format0 = [u32s(&[1, 6, 0, 0]), u16s(&[F, L, (-30i16) as u16])].concat();
    for (format, body, kern) in [(2u32, format2, -50.0), (6, format6, -70.0), (0, format0, -30.0)] {
        let sub = [u32s(&[(12 + body.len()) as u32, format, 0]), body].concat();
        let kerx = [u16s(&[2, 0]), u32s(&[1]), sub].concat();
        let f = carets_with(&[("kerx", kerx)]);
        let text = if format == 0 { "fl" } else { "fi" };
        // The shaper splits a pair's kerning between its two glyphs, as HarfBuzz does.
        let total = |f: &Font| f.shape(text, &[], false).expect("shapes").advances.iter().sum::<f64>();
        let (plain, kerned) = (total(&carets_with(&[])), total(&f));
        assert!(
            (kerned - plain - kern * 1000.0 / f64::from(f.upm())).abs() < 1e-6,
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
    let k = subset.table("kerx").expect("kerx");
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
    assert_eq!(f.glyph_closure(&[F], &[]).expect("a closure"), [0, F, L, F_F_L]);
    let (sub, out) = shapes_alike(&f, "f");
    let new = |g| out.new_gid(g).expect("kept");
    assert_eq!(sub.table("bsln").map(|b| u16::from_be_bytes([b[8], b[9]])), Some(new(L)));
    assert_eq!(sub.table("fmtx").map(|t| u32::from_be_bytes([t[4], t[5], t[6], t[7]])), Some(u32::from(new(F_F_L))));
}

// xref names subtables by index. A subset that drops one of morx's (here a type it cannot rebuild)
// must drop morx's names with it, and keep the rest.
#[test]
fn xref_drops_the_names_of_a_table_whose_subtables_moved() {
    let entry = |tag: &[u8; 4], at: u16, len: u16| [&tag[..], &[0; 8], &at.to_be_bytes(), &len.to_be_bytes()].concat();
    let xref = [vec![0, 1, 0, 0, 0, 0, 0, 0], u32s(&[2, 48]), entry(b"morx", 0, 4), entry(b"feat", 4, 4), b"SwshBold".to_vec()].concat();
    let glyf = carets_with(&[("morx", morx(2, &[(4, lookup(&[(F, I)])), (3, vec![0; 8])])), ("xref", xref.clone())]);
    // TestKERNOne is a CFF font, whose subsets take the other path: T, dotlessi, u and space.
    let cff = base_with("aat/TestKERNOne.otf", &[("morx", morx(2, &[(4, lookup(&[(1, 3)])), (3, vec![0; 8])])), ("xref", xref)]);
    for (f, text) in [(glyf, "f"), (cff, "T")] {
        let (sub, _) = shapes_alike(&f, text);
        let x = sub.table("xref").expect("xref keeps its feat name");
        assert_eq!((be32(x, 8), &x[x.len() - 4..]), (1, &b"Bold"[..]));
    }
}
