use super::*;

fn prop_of(pairs: &[(u16, u16)]) -> Vec<u8> {
    let lookup = build_aat_lookup(pairs).expect("a lookup");
    [0x0002_0000u32.to_be_bytes().to_vec(), alloc::vec![0, 1, 0, 0], lookup].concat()
}

fn values(prop: &[u8]) -> Vec<(u16, u16)> {
    Lookup::parse(&prop[8..], u16::MAX.into()).expect("a lookup").pairs()
}

// The bracket offset counts on its own; 0x1000 only says whether to swap. With glyph 12 dropped,
// 11 and 13 sit one apart, whether or not 0x1000 is set.
#[test]
fn a_bracket_offset_follows_its_partner_whatever_0x1000_says() {
    let mut active = GlyphSet::new();
    [0, 11, 13].iter().for_each(|&g| { active.insert(g); });
    let mut gid_map = alloc::vec![0u32; 14];
    (gid_map[11], gid_map[13]) = (1, 2);
    let out = subset_prop(&prop_of(&[(11, 0x0200), (13, 0x0E00)]), 14, &active, &gid_map).expect("a subset");
    assert_eq!(values(&out), [(1, 0x0100), (2, 0x0F00)]);
    let out = subset_prop(&prop_of(&[(11, 0x1200), (13, 0x1E00)]), 14, &active, &gid_map).expect("a subset");
    assert_eq!(values(&out), [(1, 0x1100), (2, 0x1F00)]);
}

// A partner the subset drops clears the offset and the swap.
#[test]
fn a_dropped_partner_clears_the_bracket() {
    let mut active = GlyphSet::new();
    [0, 11].iter().for_each(|&g| { active.insert(g); });
    let mut gid_map = alloc::vec![0u32; 14];
    gid_map[11] = 1;
    let out = subset_prop(&prop_of(&[(11, 0x1203), (13, 0x1E00)]), 14, &active, &gid_map).expect("a subset");
    assert_eq!(values(&out), [(1, 0x0003)]);
}
