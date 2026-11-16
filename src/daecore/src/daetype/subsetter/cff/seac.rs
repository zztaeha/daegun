use alloc::vec::Vec;
use super::expert_charsets::predefined_charset_sid;
use super::parse::walk_charset;
use crate::daecore::daetype::format::cff::Charset;
use crate::daecore::daetype::outline::{CffOutlines, SubrsGathered};
use crate::daecore::daetype::subsetter::GlyphSet;

pub(crate) fn standard_encoding_sid(code: u8) -> u16 {
    match code {
        32..=126 => (code - 31) as u16,
        161 => 96, 162 => 97, 163 => 98, 164 => 99, 165 => 100, 166 => 101,
        167 => 102, 168 => 103, 169 => 104, 170 => 105, 171 => 106, 172 => 107,
        173 => 108, 174 => 109, 175 => 110, 177 => 111, 178 => 112, 179 => 113,
        180 => 114, 182 => 115, 183 => 116, 184 => 117, 185 => 118, 186 => 119,
        187 => 120, 188 => 121, 189 => 122, 191 => 123, 193 => 124, 194 => 125,
        195 => 126, 196 => 127, 197 => 128, 198 => 129, 199 => 130, 200 => 131,
        202 => 132, 203 => 133, 205 => 134, 206 => 135, 207 => 136, 208 => 137,
        225 => 138, 227 => 139, 232 => 140, 233 => 141, 234 => 142, 235 => 143,
        241 => 144, 245 => 145, 248 => 146, 249 => 147, 250 => 148, 251 => 149,
        _ => 0,
    }
}

// Glyph by SID through any charset: one the font lays out, in any format, or a predefined one
// (0 ISOAdobe, 1 Expert, 2 ExpertSubset). Built once and searched; the first glyph naming a SID wins.
pub(crate) struct SidMap(Vec<(u16, u16)>);

impl SidMap {
    pub(crate) fn heap_bytes(&self) -> usize {
        crate::daecore::daekit::base::sync::heap_of(&self.0)
    }

    pub(crate) fn new(cff: &[u8], charset: Charset, n_glyphs: usize) -> SidMap {
        let mut pairs: Vec<(u16, u16)> = alloc::vec![(0, 0)];
        match charset {
            Charset::At(off) => {
                let _ = walk_charset(cff, off, n_glyphs, |gid, sid| pairs.push((sid, gid)));
            }
            Charset::Predefined(id) => pairs.extend((1..=u16::MAX).take(n_glyphs.saturating_sub(1))
                .map_while(|gid| Some((predefined_charset_sid(id, gid.into(), n_glyphs)?, gid)))),
        }
        pairs.sort_by_key(|&(sid, gid)| (sid, gid));
        pairs.dedup_by_key(|&mut (sid, _)| sid);
        SidMap(pairs)
    }

    pub(crate) fn gid(&self, sid: u16) -> Option<u16> {
        self.0.binary_search_by_key(&sid, |&(s, _)| s).ok().map(|i| self.0[i].1)
    }
}

// Adds to `set` the base and accent glyphs the frontier's seacs draw, and theirs in turn, each glyph's subroutine
// calls gathered.
pub(crate) fn close_over_seacs(
    outlines: &CffOutlines, cff: &[u8], mut frontier: Vec<u32>, set: &mut GlyphSet, gathered: &mut SubrsGathered,
) {
    while !frontier.is_empty() {
        frontier = frontier.iter()
            .filter_map(|&g| outlines.seac_glyphs(cff, g, gathered))
            .flatten()
            .flatten()
            .map(u32::from)
            .filter(|&c| set.insert(c))
            .collect();
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/cff/seac.rs"]
mod tests;
