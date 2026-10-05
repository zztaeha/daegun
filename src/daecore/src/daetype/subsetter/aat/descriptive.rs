use alloc::vec::Vec;
use alloc::collections::BTreeSet;
use super::super::super::decoder::{read_u16_be, read_u32_be, records_fit};

const EBSC_HEADER: usize = 8;
const BITMAP_SCALE: usize = 12 + 12 + 4;

const XREF_HEADER: usize = 16;
const XREF_ENTRY: usize = 16;

pub fn strike_sizes(eblc: &[u8]) -> BTreeSet<(u8, u8)> {
    let mut out = BTreeSet::new();
    let Some(num_sizes) = read_u32_be(eblc, 4).map(|v| v as usize) else { return out };
    if !records_fit(8, num_sizes, 48, eblc.len()) { return out; }
    for i in 0..num_sizes {
        let st = 8 + i * 48;
        if let (Some(&x), Some(&y)) = (eblc.get(st + 44), eblc.get(st + 45)) {
            out.insert((x, y));
        }
    }
    out
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
        b"morx" => Some(super::morx::chains(table).map(|(_, _, n)| n).collect()),
        _ => None,
    }
}

pub fn subset_xref(xref: &[u8], stable: impl Fn(&[u8]) -> bool) -> Option<Vec<u8>> {
    let num_entries = read_u32_be(xref, 8)? as usize;
    let string_base = read_u32_be(xref, 12)? as usize;
    if !records_fit(XREF_HEADER, num_entries, XREF_ENTRY, xref.len()) { return None; }

    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    for i in 0..num_entries {
        let at = XREF_HEADER + i * XREF_ENTRY;
        let record = xref.get(at..at + XREF_ENTRY)?;
        if !stable(record.get(..4)?) { continue; }
        let off = read_u16_be(record, 12)? as usize;
        let len = read_u16_be(record, 14)? as usize;
        let name = xref.get(string_base.checked_add(off)?..string_base.checked_add(off)?.checked_add(len)?)?;
        entries.push((record.to_vec(), name.to_vec()));
    }
    if entries.is_empty() { return None; }

    let new_string_base = XREF_HEADER + entries.len() * XREF_ENTRY;
    let mut strings: Vec<u8> = Vec::new();
    let mut out = xref.get(..8)?.to_vec();
    out.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    out.extend_from_slice(&(new_string_base as u32).to_be_bytes());

    for (record, name) in &entries {
        let mut r = record.clone();
        r.get_mut(12..14)?.copy_from_slice(&u16::try_from(strings.len()).ok()?.to_be_bytes());
        out.extend_from_slice(&r);
        strings.extend_from_slice(name);
    }
    out.extend_from_slice(&strings);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bitmap_scale_stays_only_for_a_strike_that_stays() {
        let scale = |x: u8, y: u8| { let mut r = alloc::vec![0u8; BITMAP_SCALE]; r[26] = x; r[27] = y; r };
        let ebsc = [alloc::vec![0, 2, 0, 0, 0, 0, 0, 2], scale(12, 12), scale(9, 9)].concat();
        let out = subset_ebsc(&ebsc, &[(9u8, 9u8)].into_iter().collect()).expect("a subset EBSC");
        assert_eq!(out, [alloc::vec![0, 2, 0, 0, 0, 0, 0, 1], scale(9, 9)].concat());
        assert!(subset_ebsc(&ebsc, &BTreeSet::new()).is_none());
    }

    // xref names morx and kerx subtables by index: an entry stays only while its table kept them all.
    #[test]
    fn xref_keeps_the_names_of_subtables_that_kept_their_place() {
        let entry = |tag: &[u8; 4], at: u16, len: u16| [&tag[..], &[0; 8], &at.to_be_bytes(), &len.to_be_bytes()].concat();
        let xref = [alloc::vec![0, 1, 0, 0, 0, 0, 0, 0], 2u32.to_be_bytes().to_vec(), 48u32.to_be_bytes().to_vec(), entry(b"morx", 0, 4), entry(b"kerx", 4, 3), b"Swshkrn".to_vec()].concat();
        let out = subset_xref(&xref, |tag| tag != b"kerx").expect("a subset xref");
        assert_eq!(read_u32_be(&out, 8), Some(1));
        assert_eq!(&out[out.len() - 4..], b"Swsh");

        let morx = |n: u32| [alloc::vec![0, 2, 0, 0], 1u32.to_be_bytes().to_vec(), [1, 16, 0, n].map(u32::to_be_bytes).concat()].concat();
        let (before, same, fewer) = (morx(3), morx(3), morx(2));
        assert_eq!(subtable_counts(b"morx", Some(&before)), subtable_counts(b"morx", Some(&same)));
        assert_ne!(subtable_counts(b"morx", Some(&before)), subtable_counts(b"morx", Some(&fewer)));
        assert_ne!(subtable_counts(b"morx", Some(&before)), subtable_counts(b"morx", None), "a dropped morx kept its names");
        assert_eq!(subtable_counts(b"feat", Some(&before)), subtable_counts(b"feat", None));
    }
}
