use super::*;

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn kerx_of(declared: u32, subtables: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let body: Vec<u8> = subtables.iter()
        .flat_map(|(format, b)| {
            [[(12 + b.len()) as u32, *format, 0].map(u32::to_be_bytes).concat(), b.clone()].concat()
        })
        .collect();
    [u16s(&[2, 0]), declared.to_be_bytes().to_vec(), body].concat()
}

fn everything(n: u16) -> (GlyphSet, Vec<u32>) {
    let mut active = GlyphSet::new();
    (0..n).for_each(|g| { active.insert(g.into()); });
    (active, (0..n).map(u32::from).collect())
}

// A format 1 state table over glyphs 1 to 65,533 in one segment, its values after the entries.
fn state_kerning() -> Vec<u8> {
    let classes = u16s(&[2, 6, 2, 12, 1, 0, 65_533, 1, 4, 0xFFFF, 0xFFFF, 0]);
    let at = 20 + classes.len();
    let header = [5u32, 20, at as u32, (at + 20) as u32, (at + 24) as u32].map(u32::to_be_bytes).concat();
    [header, classes, alloc::vec![0; 20], u16s(&[0, 0, 0xFFFF, 0])].concat()
}

// Rebuilt at four bytes a glyph, 100 such subtables would be 26 MB from 8 KB; each stays one segment.
#[test]
fn a_segment_stays_a_segment() {
    let t = kerx_of(20, &(0..20).map(|_| (1, state_kerning())).collect::<Vec<_>>());
    let (active, gid_map) = everything(u16::MAX);
    let out = subset_kerx(&t, &active, &gid_map, u16::MAX.into()).expect("a subset kerx");
    assert!(out.len() <= t.len() + 400, "{} bytes of kerx became {}", t.len(), out.len());
}

// 500 format 2 subtables of 36 bytes, each kerning by one class lookup on both sides, a trimmed array naming glyphs 0
// to 65,534 that holds one value: each expansion spends the table's shared work on every glyph it names, found or not,
// past which the rest are left out rather than reading 65 million glyphs.
#[test]
fn class_lookups_spend_the_tables_work() {
    let sub = [[0u32, 28, 28, 34].map(u32::to_be_bytes).concat(), u16s(&[8, 0, 0xFFFF]), u16s(&[0])].concat();
    let t = kerx_of(500, &alloc::vec![(2, sub); 500]);
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let out = subset_kerx(&t, &active, &[0, 1], 0xFFFF).expect("a subset kerx");
    let kept = read_u32_be(&out, 4).unwrap_or_default();
    assert!((1..500).contains(&kept), "{kept} of 500 subtables kept");
}

// A table naming one subtable more than it holds keeps the ones it holds.
#[test]
fn a_missing_last_subtable_keeps_the_ones_before() {
    let pairs = [[1u32, 6, 0, 0].map(u32::to_be_bytes).concat(), u16s(&[1, 2, 0xFFCE])].concat();
    let t = kerx_of(2, &[(0, pairs)]);
    let (active, gid_map) = everything(3);
    let out = subset_kerx(&t, &active, &gid_map, 3).expect("the readable subtable stays");
    assert_eq!(read_u32_be(&out, 4), Some(1));
}
