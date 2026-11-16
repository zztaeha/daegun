use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be, write_u32_be, Placed};
use super::super::super::format::aat::Lookup;
use super::super::remap_gid16;
use super::lookup::{build_aat_lookup, build_aat_offset_lookup, kept_entries};

const CARET_HEADER: usize = 6;

// lcar and opbd: a lookup from glyph to a record, its value the record's offset from the table's
// start. A record many glyphs name, or two that match byte for byte, is written once.
fn subset_offset_lookup(
    table: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32,
    record_len: impl Fn(&[u8], usize) -> Option<usize>,
) -> Option<Vec<u8>> {
    let lookup = Lookup::parse(table.get(CARET_HEADER..)?, num_glyphs).ok()?;
    let mut kept: Vec<(u16, &[u8])> = Vec::new();
    for (g, value) in kept_entries(&lookup, |g| remap_gid16(active, gid_map, g)) {
        let from = usize::from(value);
        kept.push((g, table.get(from..from.checked_add(record_len(table, from)?)?)?));
    }
    if kept.is_empty() { return None; }

    let glyphs: Vec<(u16, u16)> = kept.iter().map(|&(g, _)| (g, 0)).collect();
    let mut records = Placed::new(CARET_HEADER + build_aat_offset_lookup(&glyphs)?.len());
    let entries: Vec<(u16, u16)> =
        kept.into_iter().map(|(g, bytes)| Some((g, records.place(bytes)?))).collect::<Option<_>>()?;

    let mut out = table.get(..CARET_HEADER)?.to_vec();
    out.extend_from_slice(&build_aat_offset_lookup(&entries)?);
    out.extend_from_slice(&records.bytes);
    Some(out)
}

pub fn subset_lcar(
    lcar: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32,
) -> Option<Vec<u8>> {
    subset_offset_lookup(lcar, active, gid_map, num_glyphs, |d, at| Some(2 + read_u16_be(d, at)? as usize * 2))
}

pub fn subset_opbd(
    opbd: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32,
) -> Option<Vec<u8>> {
    subset_offset_lookup(opbd, active, gid_map, num_glyphs, |_, _| Some(8))
}

pub fn subset_ankr(
    ankr: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32,
) -> Option<Vec<u8>> {
    // Every anchor of an ankr without a lookup reads as (0, 0), as it does where a subset has no ankr.
    let lookup_off = read_u32_be(ankr, 4).filter(|&o| o != 0)? as usize;
    let data_off = read_u32_be(ankr, 8)? as usize;

    let header_len = 12usize;
    let lookup = Lookup::parse(ankr.get(lookup_off..)?, num_glyphs).ok()?;
    let kept = kept_entries(&lookup, |g| remap_gid16(active, gid_map, g));
    if kept.is_empty() { return None; }

    // Values count from the glyph data's own start, so the lookup's size cannot move them.
    let mut data = Placed::new(0);
    let mut entries: Vec<(u16, u16)> = Vec::with_capacity(kept.len());
    for (g, value) in kept {
        let from = data_off.checked_add(value as usize)?;
        let len = (read_u32_be(ankr, from)? as usize).checked_mul(4)?.checked_add(4)?;
        entries.push((g, data.place(ankr.get(from..from.checked_add(len)?)?)?));
    }

    let lookup_bytes = build_aat_lookup(&entries)?;
    let mut out = ankr.get(..4)?.to_vec();
    out.resize(header_len, 0);
    write_u32_be(&mut out, 4, header_len as u32);
    write_u32_be(&mut out, 8, u32::try_from(header_len + lookup_bytes.len()).ok()?);
    out.extend_from_slice(&lookup_bytes);
    out.extend_from_slice(&data.bytes);
    Some(out)
}

pub fn subset_bsln(
    bsln: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32,
) -> Option<Vec<u8>> {
    let format = read_u16_be(bsln, 4)?;
    let (fixed_len, std_glyph_at) = match format {
        0 | 1 => (6 + 2 + 32 * 2, None),
        2 | 3 => (6 + 2 + 2 + 32 * 2, Some(8usize)),
        _ => return None,
    };

    let mut out = bsln.get(..fixed_len)?.to_vec();
    if let Some(at) = std_glyph_at {
        let g = read_u16_be(bsln, at)?;
        let new = remap_gid16(active, gid_map, g)?;
        out.get_mut(at..at + 2)?.copy_from_slice(&new.to_be_bytes());
    }
    if format == 0 || format == 2 { return Some(out); }

    let lookup = Lookup::parse(bsln.get(fixed_len..)?, num_glyphs).ok()?;
    let kept = kept_entries(&lookup, |g| remap_gid16(active, gid_map, g));
    if kept.is_empty() {
        out.get_mut(4..6)?.copy_from_slice(&(format - 1).to_be_bytes());
        return Some(out);
    }
    out.extend_from_slice(&build_aat_lookup(&kept)?);
    Some(out)
}

pub fn subset_fmtx(fmtx: &[u8], active: &GlyphSet, gid_map: &[u32]) -> Option<Vec<u8>> {
    let glyph = read_u32_be(fmtx, 4)?;
    let new = remap_gid16(active, gid_map, u16::try_from(glyph).ok()?)?;
    let mut out = fmtx.to_vec();
    write_u32_be(&mut out, 4, new as u32);
    Some(out)
}

// The glyph bsln formats 2 and 3 and fmtx each take their measures from, which the closure keeps:
// without it the table is dropped from the subset.
pub fn metrics_glyphs<'a>(table: impl Fn(&str) -> Option<&'a [u8]>) -> Vec<u16> {
    let bsln = table("bsln").filter(|t| matches!(read_u16_be(t, 4), Some(2 | 3))).and_then(|t| read_u16_be(t, 8));
    let fmtx = table("fmtx").and_then(|t| read_u32_be(t, 4)).and_then(|g| u16::try_from(g).ok());
    bsln.into_iter().chain(fmtx).collect()
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/aat/simple.rs"]
mod tests;
