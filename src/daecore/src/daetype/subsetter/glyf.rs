use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, read_u32_be};
use super::super::format::glyf::{is_composite, Components};
use crate::daecore::Error;

// How many of `num_glyphs` loca is read for: a count past 16 bits only as far as the table holds entries, so a
// short table can't size the answer.
fn loca_glyphs(loca_len: usize, format: i16, num_glyphs: usize) -> usize {
    let held = loca_len / if format == 0 { 2 } else { 4 };
    num_glyphs.min(held.max(0x1_0000) - 1)
}

/// The offsets loca holds into glyf, one a glyph and one for the end; `format` is head's indexToLocFormat, 0 short
/// and 1 long. Past 65,535 glyphs only as many as loca holds, and an entry past its end reads 0.
pub fn parse_loca(loca: &[u8], format: i16, num_glyphs: usize) -> Vec<usize> {
    let mut offsets = vec![0usize; loca_len(loca.len(), format, num_glyphs)];
    loca_into(loca, format, &mut offsets);
    offsets
}

// How many offsets `parse_loca` answers, which C's buffer takes them into.
pub(crate) fn loca_len(loca_len: usize, format: i16, num_glyphs: usize) -> usize {
    loca_glyphs(loca_len, format, num_glyphs) + 1
}

// The offsets into every slot of `offsets`, as `parse_loca` reads them.
pub(crate) fn loca_into(loca: &[u8], format: i16, offsets: &mut [usize]) {
    if format == 0 {
        for (i, slot) in offsets.iter_mut().enumerate() {
            *slot = read_u16_be(loca, i * 2).map_or(0, |v| v as usize * 2);
        }
    } else {
        for (i, slot) in offsets.iter_mut().enumerate() {
            *slot = read_u32_be(loca, i * 4).map_or(0, |v| v as usize);
        }
    }
}

// loca read only at the glyphs a subset takes, as `parse_loca` reads it: an entry past the table's end is 0.
#[derive(Clone, Copy)]
pub(crate) struct Loca<'a> {
    bytes: &'a [u8],
    long: bool,
    entries: usize,
}

impl<'a> Loca<'a> {
    pub(crate) fn new(bytes: &'a [u8], format: i16, num_glyphs: usize) -> Loca<'a> {
        Loca { bytes, long: format != 0, entries: loca_len(bytes.len(), format, num_glyphs) }
    }

    fn at(&self, i: usize) -> Option<usize> {
        (i < self.entries).then(|| match self.long {
            true => read_u32_be(self.bytes, i * 4).map_or(0, |v| v as usize),
            false => read_u16_be(self.bytes, i * 2).map_or(0, |v| usize::from(v) * 2),
        })
    }

    pub(crate) fn glyph<'g>(&self, glyf: &'g [u8], gid: u32) -> Option<&'g [u8]> {
        glyf.get(self.at(gid as usize)?..self.at(gid as usize + 1)?)
    }
}

// `extended` where glyf is GLYF. Each composite's bytes are taken from `left`, as loca ranges that overlap read one
// composite again for every glyph naming it; past it the subset is refused, as copying them would be.
pub fn active_gids_into(
    requested: &[u32], glyf: &[u8], extended: bool, loca: Loca, num_glyphs: usize, set: &mut GlyphSet,
    left: &mut usize,
) -> Result<(), Error> {
    let mut new = |gid: u32| (gid as usize) < num_glyphs && set.insert(gid);
    let mut stack: Vec<u32> = requested.iter().copied().filter(|&gid| new(gid)).collect();
    while let Some(gid) = stack.pop() {
        let Some(glyph) = loca.glyph(glyf, gid).filter(|g| is_composite(g)) else { continue };
        *left = left.checked_sub(glyph.len()).ok_or(Error::limit("subset: glyf components past the subset budget"))?;
        let components = Components::new(glyph, extended).map_while(Result::ok);
        stack.extend(components.map(|c| c.gid).filter(|&gid| new(gid)));
    }
    Ok(())
}

// `glyph`'s component ids renumbered in its copy `out`, each in its own width, so nothing moves: a component the
// subset lacks names a glyph past its own, failing as in the source.
pub fn patch_compound_gids(glyph: &[u8], out: &mut [u8], extended: bool, gid_map: &[u32]) {
    for c in Components::new(glyph, extended).map_while(Result::ok) {
        let new = match gid_map.get(c.gid as usize) {
            Some(&g) if g != 0 || c.gid == 0 => g,
            _ => u32::MAX,
        };
        let len = c.gid_len();
        let id = new.min(if c.wide { 0xFF_FFFF } else { 0xFFFF }).to_be_bytes();
        out[c.at + 2..c.at + 2 + len].copy_from_slice(&id[4 - len..]);
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/glyf.rs"]
mod tests;
