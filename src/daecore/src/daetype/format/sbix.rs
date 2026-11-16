use super::super::decoder::read_u32_be;

// sbix: strikes of glyph images, each a ppem and ppi, then an offset per glyph to its record.

// Where each strike starts, those whose ppem and ppi the table holds. None when the strike offsets
// overrun the table.
pub fn strikes(sbix: &[u8]) -> Option<impl Iterator<Item = usize> + Clone + '_> {
    let count = read_u32_be(sbix, 4)? as usize;
    let offsets = sbix.get(8..)?.as_chunks::<4>().0.get(..count)?;
    let fits = |strike: usize| strike.checked_add(4).is_some_and(|end| end <= sbix.len());
    Some(offsets.iter().map(|&o| u32::from_be_bytes(o) as usize).filter(move |&strike| fits(strike)))
}

// A glyph's record in a strike: its origin and graphic type, then its data. Offsets are read glyph by
// glyph, and a record shorter than its 8-byte header is none.
pub fn record(sbix: &[u8], strike: usize, num_glyphs: usize, gid: u32) -> Option<&[u8]> {
    let g = gid as usize;
    if g >= num_glyphs {
        return None;
    }
    let offset = |g: usize| read_u32_be(sbix, strike.checked_add(4 + 4 * g)?).map(|o| o as usize);
    let (start, end) = (offset(g)?, offset(g + 1)?);
    if end <= start {
        return None;
    }
    sbix.get(strike.checked_add(start)?..strike.checked_add(end)?).filter(|r| r.len() >= 8)
}
