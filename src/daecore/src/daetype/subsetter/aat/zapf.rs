use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::super::decoder::{read_u16_be, read_u32_be, records_fit, write_u32_be};
use super::super::remap_gid16;
use super::super::super::format::aat::Lookup;

const HEADER: usize = 8;
const NONE: u32 = 0xFFFF_FFFF;
const GROUP_HAS_FLAGS: u16 = 0x8000;
const GROUP_IS_ARRAY: u16 = 0x4000;
const GROUP_COUNT: u16 = 0x3FFF;
const SUBGROUP_IS_ALIGNED: u16 = 0x8000;

// Each glyph's GlyphInfo offset, NONE for a glyph without one. Version 1 lists them after the
// header; version 2, the spec's current one, keeps them in a lookup with 32-bit values.
fn info_offsets(zapf: &[u8], num_glyphs: usize) -> Option<Vec<u32>> {
    match read_u16_be(zapf, 0)? {
        1 if records_fit(HEADER, num_glyphs, 4, zapf.len()) => {
            Some((0..num_glyphs).map(|g| read_u32_be(zapf, HEADER + g * 4).unwrap_or(NONE)).collect())
        }
        2 => {
            let lookup = Lookup::parse(zapf.get(HEADER..)?, u32::try_from(num_glyphs).unwrap_or(u32::MAX)).ok()?;
            let mut out = alloc::vec![NONE; num_glyphs];
            for (g, at) in lookup.entries_u32() {
                if let Some(slot) = out.get_mut(usize::from(g)) { *slot = at; }
            }
            Some(out)
        }
        _ => None,
    }
}

// A GlyphInfo runs from its two offsets and flags through its UTF-16 code units and identifiers.
// The spec pads it to four bytes, which Geneva's do not, so the padding is not read.
fn info_len(zapf: &[u8], at: usize) -> Option<usize> {
    let units = usize::from(*zapf.get(at.checked_add(9)?)?);
    let mut p = at + 10 + 2 * units;
    let ids = read_u16_be(zapf, p)?;
    p += 2;
    for _ in 0..ids {
        p += match *zapf.get(p)? {
            0..=63 => 2 + usize::from(*zapf.get(p + 1)?),
            64..=127 => 3,
            _ => return None,
        };
    }
    Some(p - at)
}

// A FeatureInfo: its context and AAT feature count, the <type, selector> pairs, then its OpenType tags.
fn feature_len(zapf: &[u8], at: usize) -> Option<usize> {
    let aat = usize::from(read_u16_be(zapf, at.checked_add(2)?)?);
    let tags = read_u32_be(zapf, at + 4 + 4 * aat)? as usize;
    (4usize + 4 * aat).checked_add(4)?.checked_add(tags.checked_mul(4)?)
}

// A GlyphGroup, or an offset array of them, written once; the Zapf chapter has an array name GlyphGroups alone, so
// an array it names is none. Each starts four-aligned, as an aligned subgroup's padding counts from the table.
// Groups can start inside each other and read the same bytes, so what each reads is taken from `left`, None past it.
#[allow(clippy::too_many_arguments, reason = "recursive: three of these are accumulators it threads through itself")]
fn place_group(
    zapf: &[u8], extra_info: usize, value: u32, active: &GlyphSet, gid_map: &[u32],
    extra: &mut Vec<u8>, placed: &mut BTreeMap<u32, u32>, left: &mut usize, nested: bool,
) -> Option<u32> {
    let at = extra_info.checked_add(value as usize)?;
    let num_groups = read_u16_be(zapf, at)?;
    if nested && num_groups & GROUP_IS_ARRAY != 0 { return Some(NONE); }
    if let Some(&landed) = placed.get(&value) { return Some(landed); }
    let count = (num_groups & GROUP_COUNT) as usize;

    let mut bytes = num_groups.to_be_bytes().to_vec();
    if num_groups & GROUP_IS_ARRAY != 0 {
        *left = left.checked_sub(4 + 4 * count)?;
        let mut children = Vec::with_capacity(count);
        for i in 0..count {
            let child = read_u32_be(zapf, at.checked_add(4 + i * 4)?)?;
            children.push(match child {
                NONE => NONE,
                _ => place_group(zapf, extra_info, child, active, gid_map, extra, placed, left, true)?,
            });
        }
        bytes.extend_from_slice(&read_u16_be(zapf, at + 2)?.to_be_bytes());
        for c in children { bytes.extend_from_slice(&c.to_be_bytes()); }
    } else {
        // With the top bit set, each subgroup follows a flag word; an aligned one pads to four bytes.
        let mut p = at + 2;
        for _ in 0..count {
            let flags = if num_groups & GROUP_HAS_FLAGS != 0 {
                let f = read_u16_be(zapf, p)?;
                bytes.extend_from_slice(&f.to_be_bytes());
                p += 2;
                f
            } else {
                0
            };
            let name_index = read_u16_be(zapf, p)?;
            let n_glyphs = read_u16_be(zapf, p + 2)? as usize;
            let kept: Vec<u16> = (0..n_glyphs)
                .map_while(|k| read_u16_be(zapf, p + 4 + k * 2))
                .filter_map(|g| remap_gid16(active, gid_map, g))
                .collect();
            bytes.extend_from_slice(&name_index.to_be_bytes());
            bytes.extend_from_slice(&(kept.len() as u16).to_be_bytes());
            for g in kept { bytes.extend_from_slice(&g.to_be_bytes()); }
            p += 4 + n_glyphs * 2;
            if flags & SUBGROUP_IS_ALIGNED != 0 {
                p = extra_info + (p - extra_info).next_multiple_of(4);
                bytes.resize(bytes.len().next_multiple_of(4), 0);
            }
        }
        *left = left.checked_sub(p.min(zapf.len()).saturating_sub(at))?;
    }
    bytes.resize(bytes.len().next_multiple_of(4), 0);

    let landed = u32::try_from(extra.len()).ok()?;
    extra.extend_from_slice(&bytes);
    placed.insert(value, landed);
    Some(landed)
}

// Written as version 1, the flat array every shipped Zapf uses. A GlyphInfo many glyphs share is
// written once, and so is each group and feature.
pub fn subset_zapf(
    zapf: &[u8], num_glyphs: usize, active_sorted: &[u32], active: &GlyphSet, gid_map: &[u32],
) -> Option<Vec<u8>> {
    let extra_info = read_u32_be(zapf, 4)? as usize;
    let offsets = info_offsets(zapf, num_glyphs)?;
    let budget = super::super::subset_budget(zapf.len());

    let mut extra: Vec<u8> = Vec::new();
    let mut group_at: BTreeMap<u32, u32> = BTreeMap::new();
    let mut feat_at: BTreeMap<u32, u32> = BTreeMap::new();
    let mut records: Vec<u8> = Vec::new();
    let mut record_at: BTreeMap<u32, u32> = BTreeMap::new();
    let mut glyphs: Vec<(u16, u32)> = Vec::new();
    let mut left = budget;

    for &orig in active_sorted {
        let Some(new_gid) = u16::try_from(orig).ok().and_then(|g| remap_gid16(active, gid_map, g)) else { continue };
        let Some(&source) = offsets.get(orig as usize).filter(|&&o| o != NONE) else { continue };
        if let Some(&at) = record_at.get(&source) {
            glyphs.push((new_gid, at));
            continue;
        }
        let from = source as usize;
        let mut record = zapf.get(from..from.checked_add(info_len(zapf, from)?)?)?.to_vec();
        record.resize(record.len().next_multiple_of(4), 0);

        let group = read_u32_be(&record, 0)?;
        if group != NONE {
            let (placed, left) = (&mut group_at, &mut left);
            let at = place_group(zapf, extra_info, group, active, gid_map, &mut extra, placed, left, false)?;
            write_u32_be(&mut record, 0, at);
        }
        let feature = read_u32_be(&record, 4)?;
        if feature != NONE {
            let at = match feat_at.get(&feature) {
                Some(&p) => p,
                None => {
                    let p = u32::try_from(extra.len()).ok()?;
                    let from = extra_info.checked_add(feature as usize)?;
                    extra.extend_from_slice(zapf.get(from..from.checked_add(feature_len(zapf, from)?)?)?);
                    feat_at.insert(feature, p);
                    p
                }
            };
            write_u32_be(&mut record, 4, at);
        }
        let at = u32::try_from(records.len()).ok()?;
        records.extend_from_slice(&record);
        record_at.insert(source, at);
        glyphs.push((new_gid, at));
        if records.len() + extra.len() > budget { return None; }
    }
    if glyphs.is_empty() { return None; }

    let new_count = usize::from(glyphs.iter().map(|&(g, _)| g).max()?) + 1;
    let records_at = HEADER + new_count * 4;
    let mut out = alloc::vec![0, 1, 0, 0];
    out.resize(records_at, 0);
    for i in 0..new_count { write_u32_be(&mut out, HEADER + i * 4, NONE); }
    for (g, at) in glyphs {
        write_u32_be(&mut out, HEADER + usize::from(g) * 4, u32::try_from(records_at).ok()?.checked_add(at)?);
    }
    write_u32_be(&mut out, 4, u32::try_from(records_at + records.len()).ok()?);
    out.extend_from_slice(&records);
    out.extend_from_slice(&extra);
    Some(out)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/aat/zapf.rs"]
mod tests;
