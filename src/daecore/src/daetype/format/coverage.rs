use core::cmp::Ordering;
use super::super::decoder::{midpoint_search, read_u16_be, read_u24_be};

/// A glyph's index in the Coverage table at the start of `data`; None for one it doesn't cover. Formats 3 and 4 are
/// 1 and 2 with 24-bit glyphs, and a table cut short answers from the records that fit.
// By `midpoint_search`, which picks the record where glyphs repeat or misorder. A range's index is a 32-bit sum
// and can pass 65,535.
pub fn coverage_index(data: &[u8], glyph: u32) -> Option<u32> {
    let format = read_u16_be(data, 0)?;
    let count = usize::from(read_u16_be(data, 2)?);
    let records = data.get(4..)?;
    match format {
        1 => {
            let (glyphs, _) = records[..records.len().min(count * 2)].as_chunks::<2>();
            let i = midpoint_search(glyphs.len(), |i| Some(glyph.cmp(&u32::from(u16::from_be_bytes(glyphs[i])))))?;
            u32::try_from(i).ok()
        }
        2 => {
            let (ranges, _) = records[..records.len().min(count * 6)].as_chunks::<6>();
            let field = |r: &[u8; 6], at: usize| u32::from(u16::from_be_bytes([r[at], r[at + 1]]));
            let place = |r: &[u8; 6]| match glyph {
                g if g < field(r, 0) => Ordering::Less,
                g if g <= field(r, 2) => Ordering::Equal,
                _ => Ordering::Greater,
            };
            let range = &ranges[midpoint_search(ranges.len(), |i| Some(place(&ranges[i])))?];
            Some(field(range, 4) + (glyph - field(range, 0)))
        }
        3 | 4 => medium_index(data, format == 4, glyph),
        _ => None,
    }
}

// Out of line, so the 16-bit formats' search reads as before.
#[inline(never)]
fn medium_index(data: &[u8], ranges: bool, glyph: u32) -> Option<u32> {
    let size = if ranges { 9 } else { 3 };
    let records = data.get(5..)?;
    let held = (records.len() / size).min(read_u24_be(data, 2)? as usize);
    let field = |i: usize, at: usize| read_u24_be(records, i * size + at).unwrap_or(0);
    if !ranges {
        let i = midpoint_search(held, |i| Some(glyph.cmp(&field(i, 0))))?;
        return u32::try_from(i).ok();
    }
    let place = |i: usize| match glyph {
        g if g < field(i, 0) => Ordering::Less,
        g if g <= field(i, 3) => Ordering::Equal,
        _ => Ordering::Greater,
    };
    let i = midpoint_search(held, |i| Some(place(i)))?;
    Some(field(i, 6) + (glyph - field(i, 0)))
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/coverage.rs"]
mod tests;
