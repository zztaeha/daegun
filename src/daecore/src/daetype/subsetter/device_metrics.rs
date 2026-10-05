use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, read_u32_be, records_fit};

pub fn subset_hdmx(hdmx: &[u8], num_glyphs: usize, active_sorted: &[u16]) -> Option<Vec<u8>> {
    if read_u16_be(hdmx, 0)? != 0 { return None; }
    let num_records = read_u16_be(hdmx, 2)? as usize;
    let record_size = read_u32_be(hdmx, 4)? as usize;
    if record_size < 2 + num_glyphs { return None; }
    if !records_fit(8, num_records, record_size, hdmx.len()) { return None; }
    // A record holds a width for each of the font's glyphs and no more, which also keeps the subset
    // no larger than its source.
    if active_sorted.last().is_some_and(|&g| usize::from(g) >= num_glyphs) { return None; }

    let n = active_sorted.len();
    let new_size = (2 + n).next_multiple_of(4);
    let mut out: Vec<u8> = Vec::with_capacity(8 + num_records * new_size);
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(num_records as u16).to_be_bytes());
    out.extend_from_slice(&(new_size as u32).to_be_bytes());

    for i in 0..num_records {
        let rec = 8 + i * record_size;
        let widths: Vec<u8> = active_sorted.iter().map(|&g| hdmx[rec + 2 + usize::from(g)]).collect();
        out.push(*hdmx.get(rec)?);
        out.push(widths.iter().copied().max().unwrap_or(0));
        out.extend_from_slice(&widths);
        out.resize(8 + (i + 1) * new_size, 0);
    }
    Some(out)
}

pub fn subset_ltsh(ltsh: &[u8], active_sorted: &[u16]) -> Option<Vec<u8>> {
    if read_u16_be(ltsh, 0)? != 0 { return None; }
    let count = read_u16_be(ltsh, 2)? as usize;
    if !records_fit(4, count, 1, ltsh.len()) { return None; }
    if active_sorted.last().is_some_and(|&g| usize::from(g) >= count) { return None; }

    let mut out: Vec<u8> = Vec::with_capacity(4 + active_sorted.len());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(active_sorted.len() as u16).to_be_bytes());
    out.extend(active_sorted.iter().map(|&g| ltsh[4 + usize::from(g)]));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two records of four glyphs, each row 8 bytes: pixel size, max width, four widths.
    fn hdmx() -> Vec<u8> {
        [&[0u8, 0, 0, 2, 0, 0, 0, 8][..], &[12, 9, 5, 9, 7, 6], &[0, 0], &[24, 18, 10, 18, 14, 12], &[0, 0]].concat()
    }

    #[test]
    fn hdmx_rows_keep_their_glyphs_widths_padded_to_four() {
        let out = subset_hdmx(&hdmx(), 4, &[0, 2]).expect("a subset hdmx");
        assert_eq!(out, [&[0u8, 0, 0, 2, 0, 0, 0, 4][..], &[12, 7, 5, 7], &[24, 14, 10, 14]].concat());
    }

    // A record holds widths for the font's glyphs only: a glyph past them would read the next record.
    #[test]
    fn a_glyph_past_the_font_refuses_hdmx_and_ltsh() {
        assert!(subset_hdmx(&hdmx(), 4, &[0, 4]).is_none());
        let ltsh = [0u8, 0, 0, 3, 1, 5, 9];
        assert_eq!(subset_ltsh(&ltsh, &[0, 2]), Some(vec![0, 0, 0, 2, 1, 9]));
        assert!(subset_ltsh(&ltsh, &[0, 3]).is_none());
    }
}
