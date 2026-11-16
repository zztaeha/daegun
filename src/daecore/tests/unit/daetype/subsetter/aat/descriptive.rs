use super::*;

#[test]
fn a_bitmap_scale_stays_only_for_a_strike_that_stays() {
    let scale = |x: u8, y: u8| { let mut r = alloc::vec![0u8; BITMAP_SCALE]; r[26] = x; r[27] = y; r };
    let ebsc = [alloc::vec![0, 2, 0, 0, 0, 0, 0, 2], scale(12, 12), scale(9, 9)].concat();
    let out = subset_ebsc(&ebsc, &[(9u8, 9u8)].into_iter().collect()).expect("a subset EBSC");
    assert_eq!(out, [alloc::vec![0, 2, 0, 0, 0, 0, 0, 1], scale(9, 9)].concat());
    assert!(subset_ebsc(&ebsc, &BTreeSet::new()).is_none());
}

// xref names morx and kerx subtables by index: an entry stays only while its table kept them all.
#[test]
fn xref_keeps_the_names_of_subtables_that_kept_their_place() {
    let entry = |tag: &[u8; 4], at: u16, len: u16| [&tag[..], &[0; 8], &at.to_be_bytes(), &len.to_be_bytes()].concat();
    let xref = [
        alloc::vec![0, 1, 0, 0, 0, 0, 0, 0],
        2u32.to_be_bytes().to_vec(),
        48u32.to_be_bytes().to_vec(),
        entry(b"morx", 0, 4),
        entry(b"kerx", 4, 3),
        b"Swshkrn".to_vec(),
    ]
    .concat();
    let out = subset_xref(&xref, |tag| tag != b"kerx").expect("a subset xref");
    assert_eq!(read_u32_be(&out, 8), Some(1));
    assert_eq!(&out[out.len() - 4..], b"Swsh");

    let morx = |n: u32| {
        [alloc::vec![0, 2, 0, 0], 1u32.to_be_bytes().to_vec(), [1, 16, 0, n].map(u32::to_be_bytes).concat()].concat()
    };
    let (before, same, fewer) = (morx(3), morx(3), morx(2));
    assert_eq!(subtable_counts(b"morx", Some(&before)), subtable_counts(b"morx", Some(&same)));
    assert_ne!(subtable_counts(b"morx", Some(&before)), subtable_counts(b"morx", Some(&fewer)));
    let (kept, dropped) = (subtable_counts(b"morx", Some(&before)), subtable_counts(b"morx", None));
    assert_ne!(kept, dropped, "a dropped morx kept its names");
    assert_eq!(subtable_counts(b"feat", Some(&before)), subtable_counts(b"feat", None));
}

// 1,000 entries naming morx and 1,000 naming kerx, alternating: each table is asked after once, not once an entry.
#[test]
fn xref_asks_after_each_table_once() {
    let entry = |tag: &[u8; 4]| [&tag[..], &[0; 12]].concat();
    let entries: Vec<u8> = (0..1_000).flat_map(|_| [entry(b"morx"), entry(b"kerx")].concat()).collect();
    let xref = [[0x1_0000u32, 0, 2_000, 16 + 16 * 2_000].map(u32::to_be_bytes).concat(), entries].concat();
    let asked = core::cell::Cell::new(0);
    let out = subset_xref(&xref, |tag| {
        asked.set(asked.get() + 1);
        tag != b"kerx"
    });
    assert_eq!((asked.get(), out.and_then(|out| read_u32_be(&out, 8))), (2, Some(1_000)));
}
