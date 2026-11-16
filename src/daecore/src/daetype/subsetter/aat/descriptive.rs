use alloc::vec::Vec;
use alloc::collections::{BTreeMap, BTreeSet};
use super::super::super::decoder::{read_u16_be, read_u32_be, records_fit};
use super::super::super::format::aat::morx_chains;
use super::super::super::format::eblc::{bitmap_sizes, ppem};

const EBSC_HEADER: usize = 8;
const BITMAP_SCALE: usize = 12 + 12 + 4;

const XREF_HEADER: usize = 16;
const XREF_ENTRY: usize = 16;

pub fn strike_sizes(eblc: &[u8]) -> BTreeSet<(u8, u8)> {
    bitmap_sizes(eblc).unwrap_or_default().iter().map(ppem).collect()
}

pub fn subset_ebsc(ebsc: &[u8], surviving: &BTreeSet<(u8, u8)>) -> Option<Vec<u8>> {
    let num_sizes = read_u32_be(ebsc, 4)? as usize;
    if !records_fit(EBSC_HEADER, num_sizes, BITMAP_SCALE, ebsc.len()) { return None; }

    let mut kept: Vec<&[u8]> = Vec::new();
    for i in 0..num_sizes {
        let at = EBSC_HEADER + i * BITMAP_SCALE;
        let record = ebsc.get(at..at + BITMAP_SCALE)?;
        let (&sub_x, &sub_y) = (record.get(26)?, record.get(27)?);
        if surviving.contains(&(sub_x, sub_y)) { kept.push(record); }
    }
    if kept.is_empty() { return None; }

    let mut out = ebsc.get(..4)?.to_vec();
    out.extend_from_slice(&(kept.len() as u32).to_be_bytes());
    for r in kept { out.extend_from_slice(r); }
    Some(out)
}

// How many subtables kerx holds, or each morx chain: xref names them by index, so its entries for a
// table stay only while the subset keeps that table's subtables where they were.
pub fn subtable_counts(tag: &[u8], table: Option<&[u8]>) -> Option<Vec<u32>> {
    let table = table?;
    match tag {
        b"kerx" => Some(alloc::vec![read_u32_be(table, 4)?]),
        b"morx" => Some(morx_chains(table).map(|chain| chain.subtable_count).collect()),
        _ => None,
    }
}

pub fn subset_xref(xref: &[u8], stable: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    let num_entries = read_u32_be(xref, 8)? as usize;
    let string_base = read_u32_be(xref, 12)? as usize;
    if !records_fit(XREF_HEADER, num_entries, XREF_ENTRY, xref.len()) { return None; }

    // Borrowed until written, as every entry may name one long string: the write stops once strings pass 16 bits.
    // A table many entries name is asked after once.
    let mut entries: Vec<(&[u8], &[u8])> = Vec::new();
    let mut stays: BTreeMap<&[u8], bool> = BTreeMap::new();
    for i in 0..num_entries {
        let at = XREF_HEADER + i * XREF_ENTRY;
        let record = xref.get(at..at + XREF_ENTRY)?;
        let tag = record.get(..4)?;
        if !*stays.entry(tag).or_insert_with(|| stable(tag)) { continue; }
        let off = read_u16_be(record, 12)? as usize;
        let len = read_u16_be(record, 14)? as usize;
        let name = xref.get(string_base.checked_add(off)?..string_base.checked_add(off)?.checked_add(len)?)?;
        entries.push((record, name));
    }
    if entries.is_empty() { return None; }

    let new_string_base = XREF_HEADER + entries.len() * XREF_ENTRY;
    let mut strings: Vec<u8> = Vec::new();
    let mut out = xref.get(..8)?.to_vec();
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    out.extend_from_slice(&(new_string_base as u32).to_be_bytes());

    for (record, name) in entries {
        out.extend_from_slice(record.get(..12)?);
        out.extend_from_slice(&u16::try_from(strings.len()).ok()?.to_be_bytes());
        out.extend_from_slice(record.get(14..)?);
        strings.extend_from_slice(name);
    }
    out.extend_from_slice(&strings);
    Some(out)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/aat/descriptive.rs"]
mod tests;
