use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::{read_u16_be, read_uint};
use super::{parse_coverage, build_coverage};
use super::super::remap_gid;
use super::lookup_list;
use super::context;
use super::generic::{self, schemas, Work};
use crate::daecore::Error;

// Format 1 of 16-bit glyphs, or 2 with 24-bit glyphs and substitute count, 24-bit offsets to the context's coverages
// and a 32-bit one to its own.
struct ReverseChainSingleSubst {
    width: usize,
    backtrack: Vec<Vec<u32>>,
    lookahead: Vec<Vec<u32>>,
    pairs: Vec<(u32, u32)>,
}

fn parse_reverse_chain_single_subst(
    buf: &[u8], off: usize, work: &mut Work,
) -> Result<ReverseChainSingleSubst, Error> {
    let truncated = || Error::malformed("ReverseChainSingleSubst: truncated");
    let w = match read_u16_be(buf, off) {
        Some(1) => 2,
        Some(2) => 3,
        _ => return Err(Error::unsupported("ReverseChainSingleSubst: unknown format")),
    };
    let l = if w == 2 { 2 } else { 4 };
    let cov_off = read_uint(buf, off + 2, l).ok_or_else(truncated)? as usize;
    let bt_count = usize::from(read_u16_be(buf, off + 2 + l).ok_or_else(truncated)?);
    let backtrack = context::parse_coverage_array(buf, off, off + 4 + l, bt_count, w, work)?;
    let la_off = off + 4 + l + bt_count * w;
    let la_count = usize::from(read_u16_be(buf, la_off).ok_or_else(truncated)?);
    let lookahead = context::parse_coverage_array(buf, off, la_off + 2, la_count, w, work)?;
    let gc_off = la_off + 2 + la_count * w;
    let glyph_count = read_uint(buf, gc_off, w).ok_or_else(truncated)? as usize;
    let coverage = parse_coverage(buf, off + cov_off)?;
    work.spend(coverage.len())?;
    let mut pairs = Vec::with_capacity(glyph_count.min(coverage.len()));
    for (i, &g) in coverage.iter().enumerate().take(glyph_count) {
        let sub = read_uint(buf, gc_off + w + i * w, w)
            .ok_or(Error::malformed("ReverseChainSingleSubst: substitute array truncated"))?;
        pairs.push((g, sub));
    }
    Ok(ReverseChainSingleSubst { width: w, backtrack, lookahead, pairs })
}

// Written in its source's format.
fn subset_reverse_chain_single_subst(
    buf: &[u8], off: usize, active: &GlyphSet, gid_map: &[u32], work: &mut Work,
) -> Option<Vec<u8>> {
    let parsed = parse_reverse_chain_single_subst(buf, off, work).ok()?;
    let new_backtrack = context::filter_coverage_group(&parsed.backtrack, active, gid_map)?;
    let new_lookahead = context::filter_coverage_group(&parsed.lookahead, active, gid_map)?;

    let mut remapped: Vec<(u32, u32)> = parsed.pairs.into_iter()
        .filter_map(|(g, s)| Some((remap_gid(active, gid_map, g)?, remap_gid(active, gid_map, s)?)))
        .collect();
    if remapped.is_empty() { return None; }
    // In the order the rebuilt coverage lists glyphs, one substitute each: a glyph listed twice keeps
    // its first, so the two stay the same length.
    remapped.sort_by_key(|&(g, _)| g);
    remapped.dedup_by_key(|&mut (g, _)| g);

    let (w, l) = (parsed.width, if parsed.width == 2 { 2 } else { 4 });
    let bt_blobs = context::build_coverage_array_blobs(&new_backtrack);
    let la_blobs = context::build_coverage_array_blobs(&new_lookahead);
    let cov_gids: Vec<u32> = remapped.iter().map(|&(g, _)| g).collect();
    let cov_bytes = build_coverage(&cov_gids);

    let header_len = 2 + l + 2 + bt_blobs.len() * w + 2 + la_blobs.len() * w + w + remapped.len() * w;
    let mut out = Vec::with_capacity(header_len);
    let mut put = |v: usize, width: usize| -> Option<()> {
        let v = u32::try_from(v).ok().filter(|&v| width == 4 || v >> (8 * width) == 0)?;
        out.extend_from_slice(&v.to_be_bytes()[4 - width..]);
        Some(())
    };
    put(if w == 2 { 1 } else { 2 }, 2)?;
    let mut pos = header_len;
    let context_at: Vec<usize> = bt_blobs.iter().chain(&la_blobs).map(|blob| {
        let at = pos;
        pos += blob.len();
        at
    }).collect();
    put(pos, l)?;
    put(bt_blobs.len(), 2)?;
    context_at[..bt_blobs.len()].iter().try_for_each(|&at| put(at, w))?;
    put(la_blobs.len(), 2)?;
    context_at[bt_blobs.len()..].iter().try_for_each(|&at| put(at, w))?;
    put(remapped.len(), w)?;
    remapped.iter().try_for_each(|&(_, sub)| put(sub as usize, w))?;
    for blob in bt_blobs.iter().chain(&la_blobs) { out.extend_from_slice(blob); }
    out.extend(cov_bytes);
    Some(out)
}

// A lookup's mark filtering set takes its index in `mark_sets`, each of the font's sets by its index in the subset's
// GDEF. `held` keeps lookups' and features' numbers, as a JSTF names lookups by them.
pub fn subset_gsub(
    gsub: &[u8], active: &GlyphSet, gid_map: &[u32], mark_sets: &[Option<u16>], held: bool,
) -> Option<Vec<u8>> {
    let subtable: lookup_list::SubtableSubsetter = &|effective_type, schema, buf, off, work| {
        if effective_type == 8 {
            return subset_reverse_chain_single_subst(buf, off, active, gid_map, work).map(generic::Parsed::Bytes);
        }
        let kept = generic::Kept::Subset(active, gid_map);
        generic::parse_subtable(buf, off, schema?, kept, generic::Devices::Keep, work).map(generic::Parsed::Value)
    };
    lookup_list::subset_lookup_table(gsub, 7, mark_sets, subtable, schemas::gsub_schema_for_type, held)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/otl/gsub_shared_lookups.rs"]
mod shared_lookups;

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/otl/gsub_layout.rs"]
mod layout;
