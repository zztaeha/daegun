use super::*;
use alloc::vec::Vec;

// A 24-bit LigatureSubst of 1 2 3 and of 1 4: it would apply to a ligature's whole components, never to a part of
// them.
#[test]
fn a_24_bit_ligature_would_apply_to_its_components_alone() {
    let u16s = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let u24s = |v: &[u32]| v.iter().flat_map(|v| v.to_be_bytes()[1..].to_vec()).collect::<Vec<u8>>();
    let set = [u16s(&[2]), u24s(&[8, 19, 70_003]), u16s(&[3]), u24s(&[2, 3, 70_004]), u16s(&[2]), u24s(&[4])];
    let head = [u16s(&[1, 0, 0, 0, 10, 1, 4, 4, 0, 1, 8, 2, 0, 12]), u24s(&[1, 20]), u16s(&[3]), u24s(&[1, 1])];
    let gsub = [head.concat(), set.concat()].concat();
    let t = LayoutTable::parse(&gsub).expect("a GSUB");
    let cases = [(&[1, 2, 3][..], true), (&[1, 4], true), (&[1, 2], false), (&[1, 3], false), (&[1], false)];
    for (glyphs, want) in cases {
        assert_eq!(would_apply(&t, 0, glyphs, true, &mut Run::default()), want, "{glyphs:?}");
    }
}

// A LigatureSubst whose 10,925 set offsets all name one set stating 65,535 ligatures: 4.3 GB of index, more than a
// 32-bit size counts. The allowance, refusing all, is never asked for less, and no index is built.
#[test]
fn ligature_sets_sharing_one_past_a_32_bit_size_ask_no_less_of_the_allowance() {
    let sets = 10_925u16;
    let words = [&[1, 0, sets][..], &alloc::vec![6 + 2 * sets; usize::from(sets)], &[u16::MAX]].concat();
    let data: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let needed = u64::from(sets) * (12 + 6 * u64::from(u16::MAX));
    let mut asked = Vec::new();
    let mut reserve = |bytes: usize| {
        asked.push(bytes);
        false
    };
    assert!(LigatureIndex::build(&data, &mut reserve, &mut { usize::MAX }).is_none());
    assert!(asked.iter().all(|&bytes| bytes as u64 >= needed), "asked for {asked:?} of {needed}");
}
