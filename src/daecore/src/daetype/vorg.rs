use super::decoder::{read_i16_be, read_u16_be, read_uint, search_records};
use crate::daecore::daetype::Tables;

// VORG's default origin and its records, each a glyph and its origin: version 1's glyphs and count in 16 bits,
// version 2's in 24, for fonts past 65,535 glyphs.
pub(crate) struct Vorg<'a> {
    pub(crate) default: i16,
    pub(crate) width: usize,
    records: &'a [u8],
}

impl<'a> Vorg<'a> {
    // None for a table of another major version, which fonts are read without; records whose count leaves the table
    // are absent, every glyph then taking the default.
    pub(crate) fn read(vorg: &'a [u8]) -> Option<Vorg<'a>> {
        let width = match read_u16_be(vorg, 0)? {
            1 => 2,
            2 => 3,
            _ => return None,
        };
        let default = read_i16_be(vorg, 4)?;
        let count = read_uint(vorg, 6, width)? as usize;
        let at = 6 + width;
        let records = vorg.get(at..at + (width + 2) * count).unwrap_or_default();
        Some(Vorg { default, width, records })
    }

    pub(crate) fn records(&self) -> impl Iterator<Item = (u32, i16)> + '_ {
        let w = self.width;
        self.records.chunks_exact(w + 2).map(move |r| (read_uint(r, 0, w).unwrap_or(0), read_i16_be(r, w).unwrap_or(0)))
    }

    // The glyph's record, by binary search as the spec orders them, or the default.
    #[inline]
    pub(crate) fn origin(&self, gid: u32) -> i16 {
        match self.width {
            2 => self.search::<2>(gid),
            _ => self.search::<3>(gid),
        }
    }

    // The search at a fixed width, inlined where the shaper asks each glyph's origin: at a width read per probe and
    // out of line it took Source Han Sans's vertical shape from 1,334 to 1,542 ns.
    #[inline(always)]
    fn search<const W: usize>(&self, gid: u32) -> i16 {
        match search_records(self.records.len() / (W + 2), gid, |i| read_uint(self.records, (W + 2) * i, W)) {
            Some(Ok(i)) => read_i16_be(self.records, (W + 2) * i + W).unwrap_or(self.default),
            _ => self.default,
        }
    }
}

pub fn vorg_default_origin_y(table_map: &Tables) -> Option<i16> {
    Some(Vorg::read(table_map.get("VORG")?)?.default)
}

#[cfg(test)]
#[path = "../../tests/unit/daetype/vorg.rs"]
mod tests;
