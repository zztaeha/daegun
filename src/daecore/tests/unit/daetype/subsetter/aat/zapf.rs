use super::*;

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// A GlyphInfo naming a group and a feature, with no code units and no identifiers.
fn info(group: u32, feature: u32) -> Vec<u8> {
    [group.to_be_bytes(), feature.to_be_bytes()].concat().into_iter().chain([0, 0, 0, 0]).collect()
}

// A version 1 Zapf: per glyph its GlyphInfo or none, then the extra info its offsets count from.
fn zapf_of(glyphs: &[Option<Vec<u8>>], extra: &[u8]) -> Vec<u8> {
    let records_at = 8 + 4 * glyphs.len();
    let records: Vec<u8> = glyphs.iter().flatten().flatten().copied().collect();
    let mut out = u16s(&[1, 0]);
    out.extend(((records_at + records.len()) as u32).to_be_bytes());
    let mut at = records_at;
    for g in glyphs {
        out.extend(g.as_ref().map_or(NONE, |r| { at += r.len(); (at - r.len()) as u32 }).to_be_bytes());
    }
    [out, records, extra.to_vec()].concat()
}

fn set(glyphs: &[u32]) -> GlyphSet {
    let mut s = GlyphSet::new();
    glyphs.iter().for_each(|&g| { s.insert(g); });
    s
}

fn group_of(out: &[u8], glyph: usize) -> &[u8] {
    let record = read_u32_be(out, HEADER + 4 * glyph).expect("an offset") as usize;
    let extra = read_u32_be(out, 4).expect("extra info") as usize;
    &out[extra + read_u32_be(out, record).expect("a group") as usize..]
}

// Lucida Grande's flagged groups: a flag word before each subgroup, here two empty ones and then
// three glyphs, one of which the subset drops.
#[test]
fn a_flag_word_precedes_each_subgroup() {
    let group = u16s(&[0x8003, 0x4000, 0, 0, 0, 0, 0, 0, 0x0129, 3, 1, 2, 3, 0]);
    let zapf = zapf_of(&[None, Some(info(0, NONE)), None, None], &group);
    let out = subset_zapf(&zapf, 4, &[0, 1, 3], &set(&[0, 1, 3]), &[0, 1, 0, 2]).expect("a subset");
    assert_eq!(group_of(&out, 1)[..24], u16s(&[0x8003, 0x4000, 0, 0, 0, 0, 0, 0, 0x0129, 2, 1, 2]));
}

// An aligned subgroup pads to four bytes within the extra info, in the source and the subset.
#[test]
fn an_aligned_subgroup_pads_to_four_bytes() {
    let group = u16s(&[0x8002, 0x8000, 7, 1, 1, 0, 0, 9, 2, 2, 3, 0]);
    let zapf = zapf_of(&[None, Some(info(0, NONE)), None, None], &group);
    let all = [0u32, 1, 2, 3];
    let same = subset_zapf(&zapf, 4, &all, &set(&all), &all).expect("a subset");
    assert_eq!(group_of(&same, 1)[..24], group[..]);
    let fewer = subset_zapf(&zapf, 4, &[0, 1, 3], &set(&[0, 1, 3]), &[0, 1, 0, 2]).expect("a subset");
    assert_eq!(group_of(&fewer, 1)[..20], u16s(&[0x8002, 0x8000, 7, 1, 1, 0, 0, 9, 1, 2]));
}

// Version 2, the spec's current one, keeps the offsets in a lookup with 32-bit values. Its subset
// is written as version 1's flat array.
#[test]
fn version_2_offsets_come_from_a_lookup() {
    let one = info(NONE, NONE);
    let three: Vec<u8> =
        [NONE.to_be_bytes(), NONE.to_be_bytes()].concat().into_iter().chain([0x80, 1, 0, 0x41, 0, 0, 0, 0]).collect();
    let records_at = 8 + 12 + 2 * 6 + 6;
    let mut lookup = u16s(&[6, 6, 2, 12, 1, 0]);
    lookup.extend(u16s(&[1]));
    lookup.extend((records_at as u32).to_be_bytes());
    lookup.extend(u16s(&[3]));
    lookup.extend(((records_at + one.len()) as u32).to_be_bytes());
    lookup.extend(u16s(&[0xFFFF, 0, 0]));
    let mut zapf = u16s(&[2, 0]);
    zapf.extend(((records_at + one.len() + three.len()) as u32).to_be_bytes());
    zapf.extend(lookup);
    assert_eq!(zapf.len(), records_at);
    zapf.extend([one, three.clone()].concat());
    let all = [0u32, 1, 2, 3];
    let out = subset_zapf(&zapf, 4, &all, &set(&all), &all).expect("a subset");
    assert_eq!(read_u16_be(&out, 0), Some(1));
    let at = read_u32_be(&out, HEADER + 12).expect("glyph 3's offset") as usize;
    assert_eq!(out[at..at + three.len()], three[..], "glyph 3's GlyphInfo");
    assert_eq!(read_u32_be(&out, HEADER + 8), Some(NONE));
}

// A version 2 Zapf whose offsets lookup is `lookup`.
fn version_2(lookup: &[u16]) -> Vec<u8> {
    [u16s(&[2, 0, 0, 0]), u16s(lookup)].concat()
}

// Format 2 units of six bytes hold a 16-bit value, so a 32-bit offset read from one would take
// half its bytes from the next unit. The lookup names no GlyphInfo, as the shaper's reader has it.
#[test]
fn a_lookup_whose_units_cannot_hold_32_bit_values_names_no_glyph_info() {
    let zapf = version_2(&[2, 6, 3, 12, 1, 6, 1, 1, 0, 3, 3, 0, 0xFFFF, 0xFFFF, 0]);
    assert_eq!(info_offsets(&zapf, 4), Some(alloc::vec![NONE; 4]));
}

// Format 10, a trimmed array with its own value size, reads like any other format.
#[test]
fn a_format_10_lookup_names_glyph_info() {
    let zapf = version_2(&[10, 4, 1, 1, 0, 40]);
    assert_eq!(info_offsets(&zapf, 4), Some(alloc::vec![NONE, 40, NONE, NONE]));
}

// 0xFFFFFFFF is a glyph with no GlyphInfo, not a table to drop.
#[test]
fn a_glyph_without_glyph_info_is_passed_over() {
    let zapf = zapf_of(&[None, Some(info(NONE, NONE)), None], &[]);
    let all = [0u32, 1, 2];
    let out = subset_zapf(&zapf, 3, &all, &set(&all), &all).expect("a subset");
    assert_eq!(read_u32_be(&out, HEADER), Some(NONE));
    assert_eq!(read_u32_be(&out, HEADER + 4), Some(8 + 2 * 4), "glyph 2 has no info, so the array ends at glyph 1");
}

// Papyrus keeps its features ahead of its records. A feature runs to its own end, not to the next
// kept glyph's feature, which would carry the rest of the table along.
#[test]
fn a_feature_is_copied_to_its_own_end() {
    let feature =
        |tag: u8| [u16s(&[0, 1, 1, 2]), 1u32.to_be_bytes().to_vec(), alloc::vec![b's', b's', b'0', tag]].concat();
    let extra = [feature(b'1'), feature(b'2'), alloc::vec![9; 400]].concat();
    let zapf = zapf_of(&[None, Some(info(NONE, 0)), Some(info(NONE, 16))], &extra);
    let out = subset_zapf(&zapf, 3, &[0, 1], &set(&[0, 1]), &[0, 1]).expect("a subset");
    assert_eq!(out.len(), 8 + 4 * 2 + 12 + 16);
    assert_eq!(out[out.len() - 16..], feature(b'1')[..]);
}

// `groups` glyphs past the first 256, each naming a group six bytes on from the last: one subgroup whose list runs
// to the table's end, through every group after it. Past the subset budget the table is refused.
#[test]
fn groups_read_past_the_budget_refuse_the_table() {
    let subset = |groups: u32| {
        let infos = (0..256 + groups).map(|g| (g >= 256).then(|| info(6 * (g - 256), NONE)));
        let zapf = zapf_of(&infos.collect::<Vec<_>>(), &u16s(&[1, 0x7777, 0xFFFF]).repeat(groups as usize));
        let kept: Vec<u32> = core::iter::once(0).chain(256..256 + groups).collect();
        let gid_map: Vec<u32> = (0..256 + groups).collect();
        subset_zapf(&zapf, 256 + groups as usize, &kept, &set(&kept), &gid_map)
    };
    assert!(subset(100).is_some());
    assert!(subset(400).is_none());
}
