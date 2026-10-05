use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use alloc::collections::{BTreeMap, BTreeSet};
use super::super::super::decoder::{read_u16_be, read_u32_be, records_fit, write_u32_be};
use super::super::otl::remap_gid;

const HEADER: usize = 8;
const NONE: u32 = 0xFFFF_FFFF;
const GROUP_HAS_FLAGS: u16 = 0x8000;
const GROUP_IS_ARRAY: u16 = 0x4000;
const GROUP_COUNT: u16 = 0x3FFF;
const SUBGROUP_IS_ALIGNED: u16 = 0x8000;

const MAX_GROUP_DEPTH: u8 = 8;

// Each glyph's GlyphInfo offset, NONE for a glyph without one. Version 1 lists them after the
// header; version 2, the spec's current one, keeps them in a lookup with 32-bit values.
fn info_offsets(zapf: &[u8], num_glyphs: usize) -> Option<Vec<u32>> {
    match read_u16_be(zapf, 0)? {
        1 if records_fit(HEADER, num_glyphs, 4, zapf.len()) => {
            Some((0..num_glyphs).map(|g| read_u32_be(zapf, HEADER + g * 4).unwrap_or(NONE)).collect())
        }
        2 => lookup_u32(zapf.get(HEADER..)?, num_glyphs),
        _ => None,
    }
}

// An AAT lookup with 32-bit values, in any format, each glyph taking its first value.
fn lookup_u32(data: &[u8], num_glyphs: usize) -> Option<Vec<u32>> {
    let mut out = alloc::vec![NONE; num_glyphs];
    let mut set = |g: usize, v: Option<u32>| {
        if let (Some(slot), Some(v)) = (out.get_mut(g), v) && *slot == NONE { *slot = v; }
    };
    match read_u16_be(data, 0)? {
        0 => (0..num_glyphs).for_each(|g| set(g, read_u32_be(data, 2 + 4 * g))),
        8 => {
            let (first, count) = (usize::from(read_u16_be(data, 2)?), usize::from(read_u16_be(data, 4)?));
            (0..count).for_each(|i| set(first + i, read_u32_be(data, 6 + 4 * i)));
        }
        format @ (2 | 4 | 6) => {
            let unit = usize::from(read_u16_be(data, 2)?);
            if unit == 0 { return None; }
            let n = usize::from(read_u16_be(data, 4)?).min(data.len().saturating_sub(12) / unit);
            // Units are sorted, so a glyph an earlier one covered is passed over, not visited again.
            let mut next = 0usize;
            for rec in (0..n).map(|i| 12 + i * unit) {
                let (last, first) = if format == 6 {
                    let g = usize::from(read_u16_be(data, rec)?);
                    (g, g)
                } else {
                    (usize::from(read_u16_be(data, rec)?), usize::from(read_u16_be(data, rec + 2)?))
                };
                if first == 0xFFFF || last < first.max(next) { continue; }
                for g in first.max(next)..=last.min(num_glyphs.saturating_sub(1)) {
                    set(g, match format {
                        2 => read_u32_be(data, rec + 4),
                        4 => read_u32_be(data, usize::from(read_u16_be(data, rec + 4)?) + 4 * (g - first)),
                        _ => read_u32_be(data, rec + 2),
                    });
                }
                next = last + 1;
            }
        }
        _ => return None,
    }
    Some(out)
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

#[allow(clippy::too_many_arguments, reason = "recursive: three of these are accumulators it threads through itself")]
fn place_group(
    zapf: &[u8], extra_info: usize, value: u32, active: &GlyphSet, gid_map: &[u16],
    extra: &mut Vec<u8>, placed: &mut BTreeMap<u32, u32>, open: &mut BTreeSet<u32>, depth: u8,
) -> Option<u32> {
    if let Some(&at) = placed.get(&value) { return Some(at); }
    if depth >= MAX_GROUP_DEPTH { return None; }
    open.insert(value);

    let at = extra_info.checked_add(value as usize)?;
    let num_groups = read_u16_be(zapf, at)?;
    let count = (num_groups & GROUP_COUNT) as usize;

    let mut bytes = num_groups.to_be_bytes().to_vec();
    if num_groups & GROUP_IS_ARRAY != 0 {
        let mut children = Vec::with_capacity(count);
        for i in 0..count {
            let child = read_u32_be(zapf, at.checked_add(4 + i * 4)?)?;
            children.push(if child == NONE || open.contains(&child) {
                NONE
            } else {
                place_group(zapf, extra_info, child, active, gid_map, extra, placed, open, depth + 1)?
            });
        }
        bytes.extend_from_slice(&read_u16_be(zapf, at + 2)?.to_be_bytes());
        for c in children { bytes.extend_from_slice(&c.to_be_bytes()); }
    } else {
        // With the top bit set, each subgroup follows a flag word; an aligned one pads to four bytes
        // within the extra info, where groups start four-aligned here as in the source.
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
                .filter_map(|k| read_u16_be(zapf, p + 4 + k * 2))
                .filter_map(|g| remap_gid(active, gid_map, g))
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
    }
    bytes.resize(bytes.len().next_multiple_of(4), 0);

    let landed = u32::try_from(extra.len()).ok()?;
    extra.extend_from_slice(&bytes);
    placed.insert(value, landed);
    open.remove(&value);
    Some(landed)
}

// Written as version 1, the flat array every shipped Zapf uses. A GlyphInfo many glyphs share is
// written once, and so is each group and feature.
pub fn subset_zapf(
    zapf: &[u8], num_glyphs: usize, active_sorted: &[u16], active: &GlyphSet, gid_map: &[u16],
) -> Option<Vec<u8>> {
    let extra_info = read_u32_be(zapf, 4)? as usize;
    let offsets = info_offsets(zapf, num_glyphs)?;
    let budget = super::super::subset_budget(zapf.len());

    let mut extra: Vec<u8> = Vec::new();
    let mut group_at: BTreeMap<u32, u32> = BTreeMap::new();
    let mut open: BTreeSet<u32> = BTreeSet::new();
    let mut feat_at: BTreeMap<u32, u32> = BTreeMap::new();
    let mut records: Vec<u8> = Vec::new();
    let mut record_at: BTreeMap<u32, u32> = BTreeMap::new();
    let mut glyphs: Vec<(u16, u32)> = Vec::new();

    for &orig in active_sorted {
        let Some(new_gid) = remap_gid(active, gid_map, orig) else { continue };
        let Some(&source) = offsets.get(usize::from(orig)).filter(|&&o| o != NONE) else { continue };
        if let Some(&at) = record_at.get(&source) {
            glyphs.push((new_gid, at));
            continue;
        }
        let from = source as usize;
        let mut record = zapf.get(from..from.checked_add(info_len(zapf, from)?)?)?.to_vec();
        record.resize(record.len().next_multiple_of(4), 0);

        let group = read_u32_be(&record, 0)?;
        if group != NONE {
            let at = place_group(zapf, extra_info, group, active, gid_map, &mut extra, &mut group_at, &mut open, 0)?;
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
mod tests {
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

    fn set(glyphs: &[u16]) -> GlyphSet {
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
        let all = [0u16, 1, 2, 3];
        let same = subset_zapf(&zapf, 4, &all, &set(&all), &all).expect("a subset");
        assert_eq!(group_of(&same, 1)[..24], group[..]);
        let fewer = subset_zapf(&zapf, 4, &[0, 1, 3], &set(&[0, 1, 3]), &[0, 1, 0, 2]).expect("a subset");
        assert_eq!(group_of(&fewer, 1)[..20], u16s(&[0x8002, 0x8000, 7, 1, 1, 0, 0, 9, 1, 2]));
    }

    // Version 2, the spec's current one, keeps the offsets in a lookup with 32-bit values. Its subset
    // is written as version 1's flat array.
    #[test]
    fn version_2_offsets_come_from_a_lookup() {
        let (one, three) = (info(NONE, NONE), [NONE.to_be_bytes(), NONE.to_be_bytes()].concat().into_iter().chain([0x80, 1, 0, 0x41, 0, 0, 0, 0]).collect::<Vec<u8>>());
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
        let all = [0u16, 1, 2, 3];
        let out = subset_zapf(&zapf, 4, &all, &set(&all), &all).expect("a subset");
        assert_eq!(read_u16_be(&out, 0), Some(1));
        let at = read_u32_be(&out, HEADER + 12).expect("glyph 3's offset") as usize;
        assert_eq!(out[at..at + three.len()], three[..], "glyph 3's GlyphInfo");
        assert_eq!(read_u32_be(&out, HEADER + 8), Some(NONE));
    }

    // 0xFFFFFFFF is a glyph with no GlyphInfo, not a table to drop.
    #[test]
    fn a_glyph_without_glyph_info_is_passed_over() {
        let zapf = zapf_of(&[None, Some(info(NONE, NONE)), None], &[]);
        let all = [0u16, 1, 2];
        let out = subset_zapf(&zapf, 3, &all, &set(&all), &all).expect("a subset");
        assert_eq!(read_u32_be(&out, HEADER), Some(NONE));
        assert_eq!(read_u32_be(&out, HEADER + 4), Some(8 + 2 * 4), "glyph 2 has no info, so the array ends at glyph 1");
    }

    // Papyrus keeps its features ahead of its records. A feature runs to its own end, not to the next
    // kept glyph's feature, which would carry the rest of the table along.
    #[test]
    fn a_feature_is_copied_to_its_own_end() {
        let feature = |tag: u8| [u16s(&[0, 1, 1, 2]), 1u32.to_be_bytes().to_vec(), alloc::vec![b's', b's', b'0', tag]].concat();
        let extra = [feature(b'1'), feature(b'2'), alloc::vec![9; 400]].concat();
        let zapf = zapf_of(&[None, Some(info(NONE, 0)), Some(info(NONE, 16))], &extra);
        let out = subset_zapf(&zapf, 3, &[0, 1], &set(&[0, 1]), &[0, 1]).expect("a subset");
        assert_eq!(out.len(), 8 + 4 * 2 + 12 + 16);
        assert_eq!(out[out.len() - 16..], feature(b'1')[..]);
    }
}
