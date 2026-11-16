use alloc::vec::Vec;

use super::super::decoder::{read_u16_be, read_u24_be};

// Format 2's (start, end, class) records and format 4's, its glyphs in 24 bits, those the table holds.
fn held<const N: usize>(data: &[u8], count: Option<u32>, at: usize) -> &[[u8; N]] {
    let ranges = data.get(at..).unwrap_or_default().as_chunks::<N>().0;
    &ranges[..count.map_or(0, |count| (count as usize).min(ranges.len()))]
}

fn format2(data: &[u8]) -> &[[u8; 6]] {
    held(data, read_u16_be(data, 2).map(u32::from), 4)
}

fn format4(data: &[u8]) -> &[[u8; 8]] {
    held(data, read_u24_be(data, 2), 5)
}

fn range(r: &[u8; 6]) -> (u32, u32, u32) {
    let word = |at: usize| u32::from(u16::from_be_bytes([r[at], r[at + 1]]));
    (word(0), word(2), word(4))
}

fn range24(r: &[u8; 8]) -> (u32, u32, u32) {
    let u24 = |at: usize| u32::from_be_bytes([0, r[at], r[at + 1], r[at + 2]]);
    (u24(0), u24(3), u32::from(u16::from_be_bytes([r[6], r[7]])))
}

// The class of the first range the search's midpoints meet holding `glyph`, which fixes one answer even for ranges
// out of order.
fn search<const N: usize>(ranges: &[[u8; N]], read: fn(&[u8; N]) -> (u32, u32, u32), glyph: u32) -> u32 {
    let (mut lo, mut hi) = (0, ranges.len());
    while lo < hi {
        let mid = lo + (hi - lo - 1) / 2;
        let (start, end, class) = read(&ranges[mid]);
        if glyph < start {
            hi = mid;
        } else if glyph > end {
            lo = mid + 1;
        } else {
            return class;
        }
    }
    0
}

// A glyph's class, 0 for none. Formats 3 and 4 are 1 and 2 with 24-bit glyphs, format 3's classes 24-bit too.
pub fn class_of(data: &[u8], glyph: u32) -> u32 {
    match read_u16_be(data, 0) {
        Some(1) => {
            let (Some(start), Some(count)) = (read_u16_be(data, 2), read_u16_be(data, 4)) else { return 0 };
            match glyph.checked_sub(u32::from(start)) {
                Some(i) if i < u32::from(count) => read_u16_be(data, 6 + 2 * i as usize).map_or(0, u32::from),
                _ => 0,
            }
        }
        Some(2) => search(format2(data), range, glyph),
        Some(format @ (3 | 4)) => medium_class_of(data, format, glyph),
        _ => 0,
    }
}

// Out of line, so the 16-bit formats read as before.
#[inline(never)]
fn medium_class_of(data: &[u8], format: u16, glyph: u32) -> u32 {
    if format == 4 {
        return search(format4(data), range24, glyph);
    }
    let (Some(start), Some(count)) = (read_u24_be(data, 2), read_u24_be(data, 5)) else { return 0 };
    match glyph.checked_sub(start) {
        Some(i) if i < count => read_u24_be(data, 8 + 3 * i as usize).unwrap_or(0),
        _ => 0,
    }
}

// Every glyph with a class, in glyph order, each with the class class_of gives it. None where the table
// is too short for a format and count, which makes it no ClassDef at all.
#[cfg(test)]
pub fn classes(data: &[u8]) -> Option<Vec<(u32, u32)>> {
    Some(class_runs(data)?.into_iter().flat_map(|(start, end, class)| (start..=end).map(move |g| (g, class))).collect())
}

// The glyphs `classes` gives as runs of one class, in glyph order.
pub(crate) fn class_runs(data: &[u8]) -> Option<Vec<(u32, u32, u32)>> {
    let mut runs = Vec::new();
    each_run(data, |start, end, class| runs.push((start, end, class)))?;
    Some(runs)
}

// `class_runs`' runs, each given to `f` in turn: (first glyph, last glyph, class).
pub(crate) fn each_run(data: &[u8], mut f: impl FnMut(u32, u32, u32)) -> Option<()> {
    let format = read_u16_be(data, 0);
    if data.len() < if matches!(format, Some(3 | 4)) { 5 } else { 4 } {
        return None;
    }
    match format {
        Some(1) => {
            let (Some(start), Some(count)) = (read_u16_be(data, 2), read_u16_be(data, 4)) else { return Some(()) };
            let values = data.get(6..).unwrap_or_default().as_chunks::<2>().0.iter().take(usize::from(count));
            let classes = (u32::from(start)..).zip(values).map(|(glyph, v)| (glyph, u16::from_be_bytes(*v).into()));
            classes.filter(|&(_, class)| class != 0).for_each(|(glyph, class)| f(glyph, glyph, class));
        }
        Some(3) => {
            let (Some(start), Some(count)) = (read_u24_be(data, 2), read_u24_be(data, 5)) else { return Some(()) };
            let values = data.get(8..).unwrap_or_default().as_chunks::<3>().0.iter().take(count as usize);
            let classes = (start..).zip(values).map(|(glyph, v)| (glyph, u32::from_be_bytes([0, v[0], v[1], v[2]])));
            classes.filter(|&(_, class)| class != 0).for_each(|(glyph, class)| f(glyph, glyph, class));
        }
        Some(2) => runs(format2(data), range, f),
        Some(4) => runs(format4(data), range24, f),
        _ => {}
    }
    Some(())
}

// Ranges in order give their own runs. Out of order, the search can miss part of a range, but it answers alike
// between two of the ranges' bounds, so each span between them takes the class it gives the span's first glyph.
fn runs<const N: usize>(ranges: &[[u8; N]], read: fn(&[u8; N]) -> (u32, u32, u32), mut f: impl FnMut(u32, u32, u32)) {
    let all = || ranges.iter().map(read);
    if all().all(|r| r.0 <= r.1) && all().zip(all().skip(1)).all(|(a, b)| a.1 < b.0) {
        all().filter(|r| r.2 != 0).for_each(|(start, end, class)| f(start, end, class));
        return;
    }
    let mut bounds: Vec<u32> = all().filter(|r| r.0 <= r.1).flat_map(|r| [r.0, r.1 + 1]).collect();
    bounds.sort_unstable();
    bounds.dedup();
    for span in bounds.windows(2) {
        let class = search(ranges, read, span[0]);
        if class != 0 {
            f(span[0], span[1] - 1, class);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/class_def.rs"]
mod tests;
