use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, read_u32_be, read_i16_be};
use super::super::format::glyf::{scale_len, ARG_1_AND_2_ARE_WORDS, MORE_COMPONENTS};

// maxp counts glyphs in a u16, so a larger count is not a font's and is clamped rather than allowed to
// size the answer.
pub fn parse_loca(loca: &[u8], format: i16, num_glyphs: usize) -> Vec<usize> {
    let mut offsets = vec![0usize; num_glyphs.min(usize::from(u16::MAX)) + 1];
    if format == 0 {
        for (i, slot) in offsets.iter_mut().enumerate() {
            if let Some(v) = read_u16_be(loca, i * 2) { *slot = v as usize * 2; }
        }
    } else {
        for (i, slot) in offsets.iter_mut().enumerate() {
            if let Some(v) = read_u32_be(loca, i * 4) { *slot = v as usize; }
        }
    }
    offsets
}

fn compound_components(glyf: &[u8], start: usize, end: usize) -> Vec<u16> {
    let mut ids = Vec::new();
    let mut pos = start + 10;
    let limit = end.min(glyf.len());
    loop {
        if pos + 4 > limit { break; }
        let flags = match read_u16_be(glyf, pos) { Some(v) => v, None => break };
        pos += 2;
        let gid = match read_u16_be(glyf, pos) { Some(v) => v, None => break };
        pos += 2;
        ids.push(gid);
        if flags & ARG_1_AND_2_ARE_WORDS  != 0 { pos += 4; } else { pos += 2; }
        pos += scale_len(flags);
        if flags & MORE_COMPONENTS == 0 { break; }
    }
    ids
}

pub fn active_gids_into(requested: &[u16], glyf: &[u8], loca: &[usize], num_glyphs: usize, set: &mut GlyphSet) {
    let mut stack: Vec<u16> = requested.to_vec();
    while let Some(gid) = stack.pop() {
        if gid as usize >= num_glyphs { continue; }
        if !set.insert(gid) { continue; }
        let (s, e) = (loca[gid as usize], loca[gid as usize + 1]);
        if s < e && is_composite(glyf, s) {
            for comp in compound_components(glyf, s, e) {
                if !set.contains(&comp) { stack.push(comp); }
            }
        }
    }
}

// Any negative contour count is a composite, as the spec and every reader take it, not only -1.
pub(crate) fn is_composite(glyf: &[u8], start: usize) -> bool {
    read_i16_be(glyf, start).is_some_and(|n| n < 0)
}

// A component the subset does not hold, one past the source's glyphs, names a glyph past the subset's
// too, so the composite fails to draw as it did in the source rather than drawing .notdef.
pub fn patch_compound_gids(data: &mut [u8], start: usize, end: usize, gid_map: &[u16]) {
    let mut pos = start + 10;
    loop {
        if pos + 4 > end.min(data.len()) { break; }
        let flags   = ((data[pos] as u16) << 8) | data[pos + 1] as u16;
        pos += 2;
        let orig    = ((data[pos] as u16) << 8) | data[pos + 1] as u16;
        let compact = match gid_map.get(usize::from(orig)) {
            Some(&g) if g != 0 || orig == 0 => g,
            _ => u16::MAX,
        };
        data[pos]     = (compact >> 8) as u8;
        data[pos + 1] = compact as u8;
        pos += 2;
        if flags & ARG_1_AND_2_ARE_WORDS  != 0 { pos += 4; } else { pos += 2; }
        pos += scale_len(flags);
        if flags & MORE_COMPONENTS == 0 { break; }
    }
}
