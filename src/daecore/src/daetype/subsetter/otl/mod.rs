pub mod anchor;
pub mod closure;
pub mod context;
pub mod gdef;
pub mod generic;
pub mod gpos;
pub mod gsub;
pub mod lookup_list;

#[cfg(doc)]
use crate::ErrorKind::{Malformed, Unsupported};
use alloc::vec::Vec;
use core::ops::Range;
use super::super::decoder::{read_u16_be, read_u32_be, read_uint};
use super::super::format::sanitize::Reads;
use super::GlyphSet;
use crate::daecore::daeshaper::ot::LayoutTable;
use crate::daecore::Error;

// A subset's GDEF, GSUB and GPOS, where a table the sanitizer rejects reads as none: such a GDEF goes, its lookups
// filtering by no set as the font's do. One that goes for keeping nothing leaves its emptied sets ignoring every
// mark; one that fails to write names no set.
pub(crate) fn subset_layout<'a>(
    table: impl Fn(&str) -> Option<&'a [u8]> + Copy, reads: &Reads, active: &GlyphSet, gid_map: &[u32],
) -> [Option<Vec<u8>>; 3] {
    let reading = reads.reading(table);
    let (gsub, gpos) = (reading("GSUB"), reading("GPOS"));
    let gdef = reading("GDEF");
    let new_gdef = gdef.and_then(|g| gdef::subset_gdef(g, active, gid_map));
    let sets = gdef.map(|g| gdef::mark_sets(g, active, gid_map).0).unwrap_or_default();
    let sets = if new_gdef.is_some() || sets.iter().all(Option::is_none) { sets } else { Vec::new() };
    let held = table("JSTF").is_some();
    let new_gsub = gsub.and_then(|g| gsub::subset_gsub(g, active, gid_map, &sets, held));
    let varied = gdef.and_then(gdef::var_store).is_some();
    let new_gpos = gpos.and_then(|g| gpos::subset_gpos(g, active, gid_map, &sets, held, varied));
    [new_gdef, new_gsub, new_gpos]
}

/// The glyphs of the Coverage table at `off`, in its order: format 1's or 3's list, or 2's or 4's ranges, which
/// must ascend apart.
///
/// # Errors
///
/// [`Malformed`] for one cut short or out of order, [`Unsupported`] for another format.
pub fn parse_coverage(buf: &[u8], off: usize) -> Result<Vec<u32>, Error> {
    let coverage = Coverage::read(buf, off)?;
    let mut gids = Vec::with_capacity(coverage.len);
    coverage.runs(|first, last| gids.extend(first..=last));
    Ok(gids)
}

// A Coverage table checked whole, so its glyphs' count is known before the first is read: C writes them into its
// caller's buffer, where a list copied out took 11 ns over Rust's.
pub(crate) struct Coverage<'a> {
    buf: &'a [u8],
    at: usize,
    w: usize,
    records: usize,
    ranges: bool,
    pub(crate) len: usize,
}

impl<'a> Coverage<'a> {
    pub(crate) fn read(buf: &'a [u8], off: usize) -> Result<Self, Error> {
        let format = read_u16_be(buf, off).ok_or(Error::malformed("Coverage: truncated"))?;
        let w = match format {
            1 | 2 => 2,
            3 | 4 => 3,
            _ => {
                return Err(Error::owned(
                    crate::daecore::ErrorKind::Unsupported,
                    format!("Coverage: unknown format {format}"),
                ));
            }
        };
        let records = read_uint(buf, off + 2, w).ok_or(Error::malformed("Coverage: truncated"))? as usize;
        let at = off + 2 + w;
        if format % 2 == 1 {
            if buf.len() < at + records * w {
                return Err(Error::malformed("Coverage: glyph array truncated"));
            }
            return Ok(Coverage { buf, at, w, records, ranges: false, len: records });
        }
        let (mut len, mut prev_end) = (0usize, None::<u32>);
        for i in 0..records {
            let r = at + i * 3 * w;
            let (Some(start), Some(end)) = (read_uint(buf, r, w), read_uint(buf, r + w, w)) else {
                return Err(Error::malformed("Coverage: range truncated"));
            };
            if end < start { return Err(Error::malformed("Coverage: range end before start")); }
            if let Some(pe) = prev_end && start <= pe {
                return Err(Error::malformed("Coverage: ranges not ascending/non-overlapping"));
            }
            prev_end = Some(end);
            len += (end - start) as usize + 1;
        }
        Ok(Coverage { buf, at, w, records, ranges: true, len })
    }

    // Each run of its glyphs in order, first to last: a range, or one glyph of a list.
    pub(crate) fn runs(&self, mut run: impl FnMut(u32, u32)) {
        let (buf, w) = (self.buf, self.w);
        let step = if self.ranges { 3 * w } else { w };
        for r in (0..self.records).map(|i| self.at + i * step) {
            let first = read_uint(buf, r, w).unwrap_or(0);
            run(first, if self.ranges { read_uint(buf, r + w, w).unwrap_or(first) } else { first });
        }
    }
}

// Sorted (glyph, value) pairs as runs of consecutive glyphs of one value: (first, last, value).
fn runs<T: Copy + PartialEq>(sorted: &[(u32, T)]) -> impl Iterator<Item = (u32, u32, T)> + '_ {
    let mut i = 0;
    core::iter::from_fn(move || {
        let &(start, value) = sorted.get(i)?;
        let mut end = start;
        i += 1;
        while let Some(&(g, _)) = sorted.get(i).filter(|&&(g, v)| g == end + 1 && v == value) {
            end = g;
            i += 1;
        }
        Some((start, end, value))
    })
}

// Each value, `w` bytes big-endian.
fn put(out: &mut Vec<u8>, w: usize, values: impl IntoIterator<Item = u32>) {
    values.into_iter().for_each(|v| out.extend_from_slice(&v.to_be_bytes()[4 - w..]));
}

// A coverage: the glyph list where it's no longer than the ranges, in 24 bits where a glyph needs them.
pub fn build_coverage<G: Copy + Into<u32>>(gids: &[G]) -> Vec<u8> {
    let mut sorted: Vec<(u32, ())> = gids.iter().map(|&g| (g.into(), ())).collect();
    sorted.sort_unstable_by_key(|e| e.0);
    sorted.dedup_by_key(|e| e.0);
    let wide = sorted.last().is_some_and(|e| e.0 > 0xFFFF);
    let w = if wide { 3 } else { 2 };
    let ranges = runs(&sorted).count();
    let list = sorted.len() <= 3 * ranges;
    let format: u16 = match (list, wide) {
        (true, false) => 1,
        (false, false) => 2,
        (true, true) => 3,
        (false, true) => 4,
    };
    let mut out = format.to_be_bytes().to_vec();
    if list {
        put(&mut out, w, core::iter::once(sorted.len() as u32).chain(sorted.iter().map(|e| e.0)));
    } else {
        put(&mut out, w, [ranges as u32]);
        let mut index = 0;
        for (first, last, ()) in runs(&sorted) {
            put(&mut out, w, [first, last, index]);
            index += last - first + 1;
        }
    }
    out
}

// A class definition, class 0 left out, in the smallest format: of those whose fields hold the glyphs and classes,
// in order 1 to 4, the first no other is strictly smaller than.
pub fn build_classdef<G: Copy + Into<u32>, C: Copy + Into<u32>>(entries: &[(G, C)]) -> Vec<u8> {
    let mut sorted: Vec<(u32, u32)> =
        entries.iter().map(|&(g, c)| (g.into(), c.into())).filter(|&(_, c)| c != 0).collect();
    sorted.sort_unstable_by_key(|e| e.0);
    sorted.dedup_by_key(|e| e.0);
    let (Some(&(first, _)), Some(&(last, _))) = (sorted.first(), sorted.last()) else { return vec![0, 2, 0, 0] };
    let span = (last - first) as usize + 1;
    let ranges = runs(&sorted).count();
    let class_max = sorted.iter().map(|e| e.1).max().unwrap_or(0);
    let narrow = last <= 0xFFFF && class_max <= 0xFFFF;
    let candidates = [
        (1u16, narrow && span <= 0xFFFF, 6 + 2 * span),
        (2, narrow && ranges <= 0xFFFF, 4 + 6 * ranges),
        (3, true, 8 + 3 * span),
        (4, class_max <= 0xFFFF, 5 + 8 * ranges),
    ];
    let (format, ..) = candidates.into_iter().filter(|c| c.1).fold((0, false, usize::MAX), |best, c| {
        if c.2 < best.2 { c } else { best }
    });
    let w = if format <= 2 { 2 } else { 3 };
    let mut out = format.to_be_bytes().to_vec();
    if format % 2 == 1 {
        let mut classes = vec![0u32; span];
        sorted.iter().for_each(|&(g, c)| classes[(g - first) as usize] = c);
        put(&mut out, w, [first, span as u32].into_iter().chain(classes));
    } else {
        put(&mut out, w, [ranges as u32]);
        for (start, end, class) in runs(&sorted) {
            put(&mut out, w, [start, end]);
            put(&mut out, 2, [class]);
        }
    }
    out
}

fn langsys_extent(buf: &[u8], off: usize) -> Option<usize> {
    let feature_count = read_u16_be(buf, off + 4)? as usize;
    Some(off + 6 + feature_count * 2)
}

// A null script or language system holds nothing, so reaches nowhere.
fn script_list_extent(buf: &[u8], off: usize, steps_left: &mut usize) -> Option<usize> {
    let count = read_u16_be(buf, off)? as usize;
    let mut max_reach = off + 2 + count * 6;
    for i in 0..count {
        let rec = off + 2 + i * 6;
        let s_rel = read_u16_be(buf, rec + 4)? as usize;
        if s_rel == 0 {
            continue;
        }
        let s_off = off + s_rel;
        let default_ls_rel = read_u16_be(buf, s_off)? as usize;
        let langsys_count = read_u16_be(buf, s_off + 2)? as usize;
        max_reach = max_reach.max(s_off + 4 + langsys_count * 6);
        if default_ls_rel != 0 {
            max_reach = max_reach.max(langsys_extent(buf, s_off + default_ls_rel)?);
        }
        *steps_left = steps_left.checked_sub(langsys_count)?;
        for j in 0..langsys_count {
            let lrec = s_off + 4 + j * 6;
            let ls_rel = read_u16_be(buf, lrec + 4)? as usize;
            if ls_rel != 0 {
                max_reach = max_reach.max(langsys_extent(buf, s_off + ls_rel)?);
            }
        }
    }
    Some(max_reach)
}

// The registered features whose FeatureParams say what they hold, by tag.
#[derive(Clone, Copy)]
pub(crate) enum FeatureParams {
    Size,
    StylisticSet,
    CharacterVariant,
}

impl FeatureParams {
    pub(crate) fn of(tag: &[u8]) -> Option<FeatureParams> {
        match tag {
            b"size" => Some(FeatureParams::Size),
            [b's', b's', a, b] if a.is_ascii_digit() && b.is_ascii_digit() => Some(FeatureParams::StylisticSet),
            [b'c', b'v', a, b] if a.is_ascii_digit() && b.is_ascii_digit() => Some(FeatureParams::CharacterVariant),
            _ => None,
        }
    }

    // FeatureParams have no length of their own: each kind defines the bytes its parameters at `at` take.
    pub(crate) fn len(self, buf: &[u8], at: usize) -> Option<usize> {
        Some(match self {
            FeatureParams::Size => 10,
            FeatureParams::StylisticSet => 4,
            FeatureParams::CharacterVariant => 14 + 3 * usize::from(read_u16_be(buf, at.checked_add(12)?)?),
        })
    }
}

// Where the FeatureParams at `at` of the feature whose tag is at `tag` end.
pub(crate) fn feature_params_end(buf: &[u8], tag: usize, at: usize) -> Option<usize> {
    at.checked_add(FeatureParams::of(buf.get(tag..tag.checked_add(4)?)?)?.len(buf, at)?)
}

fn feature_list_extent(buf: &[u8], off: usize) -> Option<usize> {
    let count = read_u16_be(buf, off)? as usize;
    let mut max_reach = off + 2 + count * 6;
    for i in 0..count {
        let rec = off + 2 + i * 6;
        let f_rel = read_u16_be(buf, rec + 4)? as usize;
        if f_rel == 0 {
            continue;
        }
        let f_off = off + f_rel;
        let params_rel = read_u16_be(buf, f_off)?;
        if params_rel != 0 {
            max_reach = max_reach.max(feature_params_end(buf, rec, f_off + params_rel as usize)?);
        }
        let lookup_idx_count = read_u16_be(buf, f_off + 2)? as usize;
        max_reach = max_reach.max(f_off + 4 + lookup_idx_count * 2);
    }
    Some(max_reach)
}

// Where a GSUB or GPOS's header, ScriptList and FeatureList end: what stays ahead of a rebuilt
// LookupList. A NULL list offset is an empty list.
pub fn layout_live_prefix_len(buf: &[u8]) -> Option<usize> {
    let header = layout_header_len(buf)?;
    let ([script_off, feature_off, _], _) = layout_lists(buf)?;
    let mut steps_left = buf.len() / 6;
    let script_extent = if script_off == 0 { 0 } else { script_list_extent(buf, script_off, &mut steps_left)? };
    let feature_extent = if feature_off == 0 { 0 } else { feature_list_extent(buf, feature_off)? };
    Some(script_extent.max(feature_extent).max(header))
}

// A GSUB or GPOS header's length: 10 bytes at version 1.0, 14 at 1.1, which adds FeatureVariations, and 26 from
// 1.2, which adds its lists' 32-bit offsets. Another major version holds no lists.
pub(crate) fn layout_header_len(buf: &[u8]) -> Option<usize> {
    let len = match (read_u16_be(buf, 0)?, read_u16_be(buf, 2)?) {
        (1, 0) => 10,
        (1, 1) => 14,
        (1, _) => 26,
        _ => return None,
    };
    (buf.len() >= len).then_some(len)
}

// Where a GSUB or GPOS's ScriptList, FeatureList and LookupList start, 0 for none, and the width of its lookups'
// offsets, as the shaper reads them.
pub(crate) fn layout_lists(buf: &[u8]) -> Option<([usize; 3], usize)> {
    LayoutTable::parse(buf).map(|table| table.lists())
}

// A 1.1 table's FeatureVariations and all it points at, which lie after it, so the bytes move whole: Some(None) for
// none, None where unreadable, and from its own 1.1 taking in each feature's lookup variations.
pub(crate) fn feature_variations(buf: &[u8]) -> Option<Option<Range<usize>>> {
    if layout_header_len(buf)? == 10 {
        return Some(None);
    }
    let start = read_u32_be(buf, 10)? as usize;
    if start == 0 {
        return Some(None);
    }
    let count = read_u32_be(buf, start.checked_add(4)?)? as usize;
    let records = start + 8;
    let mut end = count.checked_mul(8)?.checked_add(records)?;
    let mut steps_left = buf.len() / 2;
    let ([_, feature_list, _], _) = layout_lists(buf)?;
    for rec in (records..end).step_by(8) {
        let set = read_u32_be(buf, rec)? as usize;
        if set != 0 {
            let set = start.checked_add(set)?;
            let n = usize::from(read_u16_be(buf, set)?);
            steps_left = steps_left.checked_sub(n + 1)?;
            end = end.max(set + 2 + n * 4);
            for k in 0..n {
                let rel = read_u32_be(buf, set + 2 + k * 4)? as usize;
                if rel != 0 {
                    end = end.max(condition_end(buf, set.checked_add(rel)?)?);
                }
            }
        }
        let subst = read_u32_be(buf, rec + 4)? as usize;
        if subst != 0 {
            let subst = start.checked_add(subst)?;
            let n = usize::from(read_u16_be(buf, subst.checked_add(4)?)?);
            steps_left = steps_left.checked_sub(n + 1)?;
            end = end.max(subst + 6 + n * 6);
            for k in 0..n {
                let record = subst + 6 + k * 6;
                // A null alternate is the empty feature, and lies nowhere.
                let alternate = read_u32_be(buf, record + 2)? as usize;
                if alternate == 0 {
                    continue;
                }
                let feature = subst.checked_add(alternate)?;
                let lookups = usize::from(read_u16_be(buf, feature.checked_add(2)?)?);
                end = end.max(feature + 4 + lookups * 2);
                let params = usize::from(read_u16_be(buf, feature)?);
                if params != 0 {
                    // The feature's tag, which says what its parameters hold, is on its FeatureList record.
                    let index = usize::from(read_u16_be(buf, record)?);
                    let tag = feature_list.checked_add(2 + index * 6)?;
                    end = end.max(feature_params_end(buf, tag, feature + params)?);
                }
            }
        }
    }
    if read_u16_be(buf, start + 2)? >= 1 {
        let lookups = count.checked_mul(8)?.checked_add(records)?;
        let n = read_u32_be(buf, lookups)? as usize;
        steps_left = steps_left.checked_sub(n.checked_add(1)?)?;
        end = end.max(n.checked_mul(6)?.checked_add(lookups + 4)?);
        for k in 0..n {
            let rel = read_u32_be(buf, lookups + 6 + k * 6)? as usize;
            if rel == 0 {
                continue;
            }
            let table = start.checked_add(rel)?;
            let m = read_u32_be(buf, table.checked_add(6)?)? as usize;
            steps_left = steps_left.checked_sub(m.checked_add(1)?)?;
            end = end.max(m.checked_mul(8)?.checked_add(table + 10)?);
            for j in 0..m {
                let (condition, list) = (read_u32_be(buf, table + 10 + j * 8)?, read_u32_be(buf, table + 14 + j * 8)?);
                if condition != 0 {
                    end = end.max(condition_end(buf, table.checked_add(condition as usize)?)?);
                }
                if list != 0 {
                    let list = table.checked_add(list as usize)?;
                    let lookups = usize::from(read_u16_be(buf, list)?);
                    end = end.max(list + 2 + 2 * lookups);
                }
            }
        }
    }
    (end <= buf.len()).then_some(Some(start..end))
}

// Where a condition ends. Only format 1, an axis range, is read: 8 bytes, where another may reach further.
fn condition_end(buf: &[u8], at: usize) -> Option<usize> {
    (read_u16_be(buf, at)? == 1).then_some(at + 8)
}

pub(crate) fn device_table(buf: &[u8], at: usize) -> Option<&[u8]> {
    let delta_format = read_u16_be(buf, at + 4)?;
    let len = match delta_format {
        0x8000 => 6,
        1..=3 => {
            let start = read_u16_be(buf, at)? as usize;
            let end = read_u16_be(buf, at + 2)? as usize;
            let count = end.checked_sub(start)?.checked_add(1)?;
            let per_word = 16 >> delta_format;
            6 + count.div_ceil(per_word) * 2
        }
        _ => return None,
    };
    buf.get(at..at + len)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/otl/mod.rs"]
mod tests;
