use alloc::vec::Vec;
use super::expert_charsets::{EXPERT_CHARSET, EXPERT_SUBSET_CHARSET};
use super::parse::{walk_charset, CharsetFlow};
use crate::daecore::daetype::outline::CffOutlines;
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
    pub(crate) fn new(cff: &[u8], charset_off: Option<usize>, predefined: u8, n_glyphs: usize) -> SidMap {
        let mut pairs: Vec<(u16, u16)> = alloc::vec![(0, 0)];
        match charset_off {
            Some(off) => {
                let _ = walk_charset(cff, off, n_glyphs, |gid, sid| {
                    pairs.push((sid, gid));
                    CharsetFlow::Continue
                });
            }
            None => {
                let table: Option<&[u16]> = match predefined {
                    1 => Some(&EXPERT_CHARSET),
                    2 => Some(&EXPERT_SUBSET_CHARSET),
                    _ => None,
                };
                for gid in 1..n_glyphs.min(usize::from(u16::MAX) + 1) {
                    let sid = match table {
                        Some(t) => match t.get(gid) { Some(&sid) => sid, None => break },
                        None => gid as u16,
                    };
                    pairs.push((sid, gid as u16));
                }
            }
        }
        pairs.sort_by_key(|&(sid, gid)| (sid, gid));
        pairs.dedup_by_key(|&mut (sid, _)| sid);
        SidMap(pairs)
    }

    pub(crate) fn gid(&self, sid: u16) -> Option<u16> {
        self.0.binary_search_by_key(&sid, |&(s, _)| s).ok().map(|i| self.0[i].1)
    }
}

// Adds to `set` the base and accent glyphs the frontier's seacs draw, and theirs in turn.
pub(crate) fn close_over_seacs(outlines: &CffOutlines, cff: &[u8], mut frontier: Vec<u16>, set: &mut GlyphSet) {
    while !frontier.is_empty() {
        frontier = frontier.iter()
            .filter_map(|&g| outlines.seac_glyphs(cff, g))
            .flatten()
            .flatten()
            .filter(|&c| set.insert(c))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Charsets in each format, and the predefined ones: SID to glyph, the first glyph naming a SID.
    #[test]
    fn a_sid_finds_its_glyph_through_every_charset() {
        let format0 = [0u8, 0, 34, 0, 125, 0, 34];
        let map = SidMap::new(&format0, Some(0), 0, 4);
        assert_eq!((map.gid(34), map.gid(125), map.gid(0), map.gid(35)), (Some(1), Some(2), Some(0), None));

        let format1 = [1u8, 1, 0x87, 2];
        assert_eq!(SidMap::new(&format1, Some(0), 0, 4).gid(0x189), Some(3));
        let format2 = [2u8, 1, 0x87, 0, 2];
        assert_eq!(SidMap::new(&format2, Some(0), 0, 4).gid(0x188), Some(2));

        assert_eq!(SidMap::new(&[], None, 0, 40).gid(34), Some(34), "ISOAdobe is SID for glyph");
        assert_eq!(SidMap::new(&[], None, 1, 6).gid(229), Some(2), "Expert names exclamsmall second after space");
        assert_eq!(SidMap::new(&[], None, 2, 6).gid(231), Some(2), "ExpertSubset starts dollaroldstyle there");
    }
}
