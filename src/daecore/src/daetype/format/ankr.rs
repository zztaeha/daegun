#[cfg(doc)]
use crate::ErrorKind::Malformed;
use super::aat::Lookup;
use crate::daecore::daetype::decoder::{read_i16_be, read_u16_be, read_u32_be, window};
use crate::daecore::Error;

/// A point as ankr stores a glyph's anchor and a kerx action a control point, in font units, y up.
#[expect(clippy::exhaustive_structs, reason = "the table's pair, field for field")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct AnchorPoint {
    /// Across.
    pub x: i16,
    /// Up.
    pub y: i16,
}

/// Apple's ankr table: each glyph's anchor points, which kerx and morx attach glyphs by.
#[derive(Clone, Copy)]
pub struct Ankr<'a> {
    data: &'a [u8],
    lookup: Lookup<'a>,
    glyph_data: usize,
}

impl core::fmt::Debug for Ankr<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ankr").field("bytes", &self.data.len()).finish_non_exhaustive()
    }
}

impl<'a> Ankr<'a> {
    /// The version at the start of an ankr table in `data`; None for one too short to hold it.
    #[doc(alias("ankr_version"))]
    pub fn version(data: &[u8]) -> Option<u16> {
        read_u16_be(data, 0)
    }

    /// The table in `data`, for a font of `num_glyphs` glyphs.
    ///
    /// # Errors
    ///
    /// [`Malformed`] for a header cut short or a lookup past the data, and the lookup's own.
    pub fn parse(data: &'a [u8], num_glyphs: u32) -> Result<Self, Error> {
        let h = window::<8>(data, 4).ok_or(Error::malformed("ankr: its header is cut short"))?;
        let lookup_at = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize;
        let glyph_data = u32::from_be_bytes([h[4], h[5], h[6], h[7]]) as usize;
        let lookup = data.get(lookup_at..).ok_or(Error::malformed("ankr: its lookup lies past its end"))?;
        let lookup = Lookup::parse(lookup, num_glyphs)?;
        Ok(Ankr { data, lookup, glyph_data })
    }

    /// A glyph's anchor point by index; None past its points.
    pub fn anchor_point(&self, glyph: u32, index: u16) -> Option<AnchorPoint> {
        let entry = self.lookup.value(glyph)? as usize;
        let at = self.glyph_data.checked_add(entry)?;
        let count = read_u32_be(self.data, at)?;
        if u32::from(index) >= count {
            return None;
        }
        let point = at.checked_add(4)?.checked_add(usize::from(index).checked_mul(4)?)?;
        let p = window::<4>(self.data, point)?;
        Some(AnchorPoint { x: i16::from_be_bytes([p[0], p[1]]), y: i16::from_be_bytes([p[2], p[3]]) })
    }

    /// How many anchor points a glyph has that are there and that a u16 index can name, whatever count the font
    /// states.
    pub fn point_count(&self, glyph: u32) -> u32 {
        let Some(entry) = self.lookup.value(glyph) else { return 0 };
        let Some(at) = self.glyph_data.checked_add(entry as usize) else { return 0 };
        let Some(count) = read_u32_be(self.data, at) else { return 0 };
        let present = (self.data.len() - at - 4) / 4;
        count.min(u32::try_from(present).unwrap_or(u32::MAX)).min(1 << 16)
    }
}

/// The point at `at`, as an ankr point or a kerx control point stores it; None past the end.
pub fn control_point(data: &[u8], at: usize) -> Option<AnchorPoint> {
    Some(AnchorPoint { x: read_i16_be(data, at)?, y: read_i16_be(data, at + 2)? })
}


#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/ankr.rs"]
mod tests;
