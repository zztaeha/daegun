use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be};
use super::super::otl::remap_gid;
use super::super::super::format::aat::Lookup;
use super::lookup::build_aat_lookup;

const HAS_BRACKET: u16 = 0x1000;
const BRACKET_OFFSET: u16 = 0x0F00;

fn bracket_delta(value: u16) -> i32 {
    let nibble = ((value & BRACKET_OFFSET) >> 8) as i32;
    if nibble >= 8 { nibble - 16 } else { nibble }
}

// The offset to the complementary bracket stands on its own; 0x1000 only says whether to swap in
// right-to-left text, so it is kept as it was while the offset follows the renumbered partner.
fn remap_bracket(value: u16, glyph: u16, active: &GlyphSet, gid_map: &[u16]) -> u16 {
    if value & BRACKET_OFFSET == 0 { return value; }
    let cleared = value & !(HAS_BRACKET | BRACKET_OFFSET);

    let Some(partner) = glyph.checked_add_signed(bracket_delta(value) as i16) else { return cleared };
    let (Some(new_self), Some(new_partner)) = (
        remap_gid(active, gid_map, glyph), remap_gid(active, gid_map, partner),
    ) else { return cleared };

    let delta = new_partner as i32 - new_self as i32;
    if !(-8..=7).contains(&delta) { return cleared; }
    (value & !BRACKET_OFFSET) | (((delta & 0xF) as u16) << 8)
}

pub fn subset_prop(
    prop: &[u8], num_glyphs: usize, active: &GlyphSet, gid_map: &[u16],
) -> Option<Vec<u8>> {
    let version = read_u32_be(prop, 0)?;
    let format = read_u16_be(prop, 4)?;
    let default = read_u16_be(prop, 6)?;

    let mut out = Vec::with_capacity(prop.len() / 4);
    out.extend_from_slice(&version.to_be_bytes());
    if format == 0 {
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&default.to_be_bytes());
        return Some(out);
    }

    let lookup = Lookup::parse(prop.get(8..)?, num_glyphs as u16)?;
    let kept: Vec<(u16, u16)> = lookup.entries().into_iter()
        .filter_map(|(g, v)| {
            remap_gid(active, gid_map, g).map(|ng| (ng, remap_bracket(v, g, active, gid_map)))
        })
        .collect();

    if kept.is_empty() || kept.iter().all(|(_, v)| *v == default) {
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&default.to_be_bytes());
        return Some(out);
    }

    let lookup = build_aat_lookup(&kept)?;
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&default.to_be_bytes());
    out.extend_from_slice(&lookup);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop_of(pairs: &[(u16, u16)]) -> Vec<u8> {
        [0x0002_0000u32.to_be_bytes().to_vec(), alloc::vec![0, 1, 0, 0], build_aat_lookup(pairs).expect("a lookup")].concat()
    }

    fn values(prop: &[u8]) -> Vec<(u16, u16)> {
        Lookup::parse(&prop[8..], u16::MAX).expect("a lookup").entries()
    }

    // The bracket offset counts on its own; 0x1000 only says whether to swap. With glyph 12 dropped,
    // 11 and 13 sit one apart, whether or not 0x1000 is set.
    #[test]
    fn a_bracket_offset_follows_its_partner_whatever_0x1000_says() {
        let mut active = GlyphSet::new();
        [0, 11, 13].iter().for_each(|&g| { active.insert(g); });
        let mut gid_map = alloc::vec![0u16; 14];
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
        let mut gid_map = alloc::vec![0u16; 14];
        gid_map[11] = 1;
        let out = subset_prop(&prop_of(&[(11, 0x1203), (13, 0x1E00)]), 14, &active, &gid_map).expect("a subset");
        assert_eq!(values(&out), [(1, 0x0003)]);
    }
}
