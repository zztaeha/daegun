use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be};
use super::super::remap_gid16;
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
fn remap_bracket(value: u16, glyph: u16, active: &GlyphSet, gid_map: &[u32]) -> u16 {
    if value & BRACKET_OFFSET == 0 { return value; }
    let cleared = value & !(HAS_BRACKET | BRACKET_OFFSET);

    let Some(partner) = glyph.checked_add_signed(bracket_delta(value) as i16) else { return cleared };
    let (Some(new_self), Some(new_partner)) = (
        remap_gid16(active, gid_map, glyph), remap_gid16(active, gid_map, partner),
    ) else { return cleared };

    let delta = new_partner as i32 - new_self as i32;
    if !(-8..=7).contains(&delta) { return cleared; }
    (value & !BRACKET_OFFSET) | (((delta & 0xF) as u16) << 8)
}

pub fn subset_prop(
    prop: &[u8], num_glyphs: usize, active: &GlyphSet, gid_map: &[u32],
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

    let lookup = Lookup::parse(prop.get(8..)?, num_glyphs as u32).ok()?;
    let kept: Vec<(u16, u16)> = lookup.pairs().into_iter()
        .filter_map(|(g, v)| {
            remap_gid16(active, gid_map, g).map(|ng| (ng, remap_bracket(v, g, active, gid_map)))
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
#[path = "../../../../tests/unit/daetype/subsetter/aat/prop.rs"]
mod tests;
