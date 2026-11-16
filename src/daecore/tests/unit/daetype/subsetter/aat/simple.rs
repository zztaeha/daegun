use super::*;
use crate::daecore::daetype::decoder::read_i16_be;

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// Glyphs 0 to 8 kept as 0, 1 and 2 from 0, 5 and 7.
fn kept() -> (GlyphSet, Vec<u32>) {
    let mut active = GlyphSet::new();
    [0, 5, 7].iter().for_each(|&g| { active.insert(g); });
    let mut gid_map = alloc::vec![0u32; 9];
    (gid_map[5], gid_map[7]) = (1, 2);
    (active, gid_map)
}

// A caret table: its header, a lookup from glyph to record offset, then the records.
fn caret_table(records: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let glyphs: Vec<_> = records.iter().map(|&(g, _)| (g, 0)).collect();
    let lookup_len = build_aat_offset_lookup(&glyphs).expect("a lookup").len();
    let mut at = CARET_HEADER + lookup_len;
    let mut entries = Vec::new();
    for (g, r) in records {
        entries.push((*g, at as u16));
        at += r.len();
    }
    let records: Vec<u8> = records.iter().flat_map(|(_, r)| r.clone()).collect();
    [u16s(&[1, 0, 0]), build_aat_offset_lookup(&entries).expect("a lookup"), records].concat()
}

fn record_of(table: &[u8], glyph: u16, len: usize) -> &[u8] {
    let lookup = Lookup::parse(&table[CARET_HEADER..], u16::MAX.into()).expect("a lookup");
    let at = usize::from(lookup.value(glyph.into()).expect("an entry"));
    &table[at..at + len]
}

#[test]
fn caret_and_bound_records_follow_their_glyphs() {
    let (active, gid_map) = kept();
    let caret = |v: u16| u16s(&[2, v, v + 1]);
    let lcar = caret_table(&[(5, caret(10)), (6, caret(20)), (7, caret(30))]);
    let out = subset_lcar(&lcar, &active, &gid_map, 9).expect("a subset lcar");
    assert_eq!((record_of(&out, 1, 6), record_of(&out, 2, 6)), (&caret(10)[..], &caret(30)[..]));

    let bounds = |v: u16| u16s(&[v, v, v, v]);
    let opbd = caret_table(&[(5, bounds(1)), (7, bounds(1)), (8, bounds(2))]);
    let out = subset_opbd(&opbd, &active, &gid_map, 9).expect("a subset opbd");
    assert_eq!(record_of(&out, 1, 8), &bounds(1)[..]);
    let shared = Lookup::parse(&out[CARET_HEADER..], 3).expect("a lookup");
    assert_eq!(shared.value(1), shared.value(2), "a record two glyphs share was written twice");
}

#[test]
fn anchor_points_follow_their_glyphs() {
    let (active, gid_map) = kept();
    let points = |n: u32, x: i16| {
        let each = (0..n).flat_map(|i| [(x + i as i16).to_be_bytes(), 0i16.to_be_bytes()].concat());
        [n.to_be_bytes().to_vec(), each.collect()].concat()
    };
    let data = [points(1, 5), points(2, 7)].concat();
    let lookup = build_aat_lookup(&[(5, 0), (7, 8)]).expect("a lookup");
    let ankr = [u16s(&[0, 0, 0, 12]), (12 + lookup.len() as u32).to_be_bytes().to_vec(), lookup, data].concat();
    let out = subset_ankr(&ankr, &active, &gid_map, 9).expect("a subset ankr");
    let (table, data) = (read_u32_be(&out, 4).expect("lookup") as usize, read_u32_be(&out, 8).expect("data") as usize);
    let at = data + usize::from(Lookup::parse(&out[table..], 3).expect("a lookup").value(2).expect("glyph 7"));
    assert_eq!((read_u32_be(&out, at), read_i16_be(&out, at + 8)), (Some(2), Some(8)));
}

#[test]
fn baselines_follow_their_glyphs() {
    let (active, gid_map) = kept();
    let deltas = alloc::vec![0u8; 64];
    let lookup = build_aat_lookup(&[(5, 2), (8, 1)]).expect("a lookup");
    let format1 = [u16s(&[1, 0, 1, 0]), deltas.clone(), lookup].concat();
    let out = subset_bsln(&format1, &active, &gid_map, 9).expect("a subset bsln");
    assert_eq!(Lookup::parse(&out[72..], 3).expect("a lookup").pairs(), [(1, 2)]);

    let format2 = [u16s(&[1, 0, 2, 0, 7]), deltas].concat();
    let out = subset_bsln(&format2, &active, &gid_map, 9).expect("a subset bsln");
    assert_eq!(read_u16_be(&out, 8), Some(2), "the standard glyph was not renumbered");
}

#[test]
fn the_font_metrics_glyph_follows() {
    let (active, gid_map) = kept();
    let fmtx = [0x0002_0000u32, 7, 0, 0].map(u32::to_be_bytes).concat();
    let out = subset_fmtx(&fmtx, &active, &gid_map).expect("a subset fmtx");
    assert_eq!(read_u32_be(&out, 4), Some(2));
}

// bsln's standard glyph and fmtx's glyph join the closure: without them both tables are dropped.
#[test]
fn the_glyphs_bsln_and_fmtx_measure_by_are_named_to_the_closure() {
    let bsln = [u16s(&[1, 0, 3, 0, 7]), alloc::vec![0; 64]].concat();
    let fmtx = [0x0002_0000u32, 4, 0, 0].map(u32::to_be_bytes).concat();
    let found = metrics_glyphs(|tag| match tag { "bsln" => Some(&bsln[..]), "fmtx" => Some(&fmtx[..]), _ => None });
    assert_eq!(found, [7, 4]);
    let format1 = [u16s(&[1, 0, 1, 0]), alloc::vec![0; 64]].concat();
    assert!(metrics_glyphs(|tag| (tag == "bsln").then_some(&format1[..])).is_empty());
}
