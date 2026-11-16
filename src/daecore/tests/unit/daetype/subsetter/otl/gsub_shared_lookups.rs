use super::*;

// 2,000 lookup list entries naming one lookup, which names one SingleSubst (5 to 6) 2,000 times. No feature names
// it, so subsets hold the lookups by number.
fn gsub() -> Vec<u8> {
    const N: u16 = 2_000;
    let mut t = [1u16, 0, 0, 0, 10, N].map(u16::to_be_bytes).concat();
    (0..N).for_each(|_| t.extend((2 + 2 * N).to_be_bytes()));
    t.extend([1u16, 0, N].map(u16::to_be_bytes).concat());
    (0..N).for_each(|_| t.extend((6 + 2 * N).to_be_bytes()));
    t.extend([1u16, 6, 1, 1, 1, 5].map(u16::to_be_bytes).concat());
    t
}

// 2,000 entries naming one lookup of a 1,000-glyph SingleSubst, which copies would make 8 MB from 8 KB of table:
// written once, the subset is no larger than its source. No feature names it, so it's held by number.
#[test]
fn a_lookup_many_entries_name_is_written_once() {
    const N: u16 = 2_000;
    const K: u16 = 1_000;
    let mut t = [1u16, 0, 0, 0, 10, N].map(u16::to_be_bytes).concat();
    (0..N).for_each(|_| t.extend((2 + 2 * N).to_be_bytes()));
    t.extend([1u16, 0, 1, 8].map(u16::to_be_bytes).concat());
    t.extend([2u16, 6 + 2 * K, K].map(u16::to_be_bytes).concat());
    t.extend((1..=K).flat_map(|g| (g + K).to_be_bytes()));
    t.extend([1u16, K].map(u16::to_be_bytes).concat());
    t.extend((1..=K).flat_map(u16::to_be_bytes));
    let mut active = GlyphSet::new();
    (0..=2 * K).for_each(|g| { active.insert(g.into()); });
    let gid_map: Vec<u32> = (0..=2 * K).map(u32::from).collect();
    let out = subset_gsub(&t, &active, &gid_map, &[], true).expect("a subset GSUB");
    assert!(out.len() <= t.len(), "{} bytes of GSUB became {}", t.len(), out.len());
}

#[test]
fn a_shared_subtable_is_rebuilt_once_a_lookup() {
    let t = gsub();
    let mut active = GlyphSet::new();
    (0..8).for_each(|g| { active.insert(g); });
    let out = subset_gsub(&t, &active, &(0..8).collect::<Vec<u32>>(), &[], true).expect("a subset GSUB");
    let list = usize::from(read_u16_be(&out, 8).expect("a lookup list"));
    let lookup = list + usize::from(read_u16_be(&out, list + 2).expect("a lookup"));
    assert_eq!(read_u16_be(&out, lookup + 4), Some(1), "the lookup kept its subtable more than once");
}
