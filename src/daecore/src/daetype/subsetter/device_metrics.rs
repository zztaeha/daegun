use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, read_u32_be, records_fit};

pub fn subset_hdmx(hdmx: &[u8], num_glyphs: usize, active_sorted: &[u32]) -> Option<Vec<u8>> {
    if read_u16_be(hdmx, 0)? != 0 { return None; }
    let num_records = read_u16_be(hdmx, 2)? as usize;
    let record_size = read_u32_be(hdmx, 4)? as usize;
    if record_size < 2 + num_glyphs { return None; }
    if !records_fit(8, num_records, record_size, hdmx.len()) { return None; }
    // A record holds a width for each of the font's glyphs and no more, which also keeps the subset
    // no larger than its source.
    if active_sorted.last().is_some_and(|&g| g as usize >= num_glyphs) { return None; }

    let n = active_sorted.len();
    // The hdmx chapter has each device record padded to 32-bit alignment.
    let new_size = (2 + n).next_multiple_of(4);
    let mut out: Vec<u8> = Vec::with_capacity(8 + num_records * new_size);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(num_records as u16).to_be_bytes());
    out.extend_from_slice(&(new_size as u32).to_be_bytes());

    for i in 0..num_records {
        let rec = 8 + i * record_size;
        let widths: Vec<u8> = active_sorted.iter().map(|&g| hdmx[rec + 2 + g as usize]).collect();
        out.push(*hdmx.get(rec)?);
        out.push(widths.iter().copied().max().unwrap_or(0));
        out.extend_from_slice(&widths);
        out.resize(8 + (i + 1) * new_size, 0);
    }
    Some(out)
}

pub fn subset_ltsh(ltsh: &[u8], active_sorted: &[u32]) -> Option<Vec<u8>> {
    if read_u16_be(ltsh, 0)? != 0 { return None; }
    let count = read_u16_be(ltsh, 2)? as usize;
    if !records_fit(4, count, 1, ltsh.len()) { return None; }
    if active_sorted.last().is_some_and(|&g| g as usize >= count) { return None; }

    let mut out: Vec<u8> = Vec::with_capacity(4 + active_sorted.len());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(active_sorted.len() as u16).to_be_bytes());
    out.extend(active_sorted.iter().map(|&g| ltsh[4 + g as usize]));
    Some(out)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/device_metrics.rs"]
mod tests;
