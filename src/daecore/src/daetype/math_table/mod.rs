mod constants;
mod glyph_info;
mod variants;

use super::decoder::{offset16_in, read_i16_be, read_u16_be, records_fit};
pub(crate) use super::format::ivs::DeviceDeltas;

pub use constants::{parse_math_constants, MathConstants};
pub use glyph_info::{
    math_is_extended_shape, math_italics_correction, math_kern, math_top_accent_attachment,
    MathKernCorner,
};
pub use variants::{
    math_glyph_construction, math_min_connector_overlap, GlyphAssembly, GlyphPart, MathGlyphConstruction,
    MathGlyphVariant,
};

// MATH's layout, read alike by the public readers and the subsetter. An array of independent records
// holds those that lie in the table; a GlyphAssembly or MathKern counts only whole.

// MathConstants: four 16-bit fields, 51 MathValueRecords and one more int16.
pub(crate) const CONSTANTS_LEN: usize = 8 + 51 * 4 + 2;

// GDEF's store at a location, which MATH's variation indices name; none at the default, where no value moves.
pub(crate) type Deltas<'a> = Option<&'a mut DeviceDeltas>;

// The delta at a location of the MathValueRecord at `record`, its device counted from `base`.
fn device_delta(math: &[u8], record: usize, base: usize, deltas: &mut DeviceDeltas) -> f64 {
    match read_u16_be(math, record + 2).map_or(0, usize::from) {
        0 => 0.0,
        device => super::format::ivs::units(deltas.delta(math, base + device)),
    }
}

// A MathValueRecord's value, with its delta at a location.
pub(super) fn math_value(math: &[u8], record: usize, base: usize, deltas: Deltas<'_>) -> Option<f64> {
    let value = f64::from(read_i16_be(math, record)?);
    Some(match deltas {
        Some(deltas) => value + device_delta(math, record, base, deltas),
        None => value,
    })
}

pub(super) fn coverage_index(buf: &[u8], off: usize, gid: u32) -> Option<usize> {
    super::format::coverage::coverage_index(buf.get(off..)?, gid).and_then(|i| usize::try_from(i).ok())
}

// A Coverage and the records parallel to it. A record counts where both the count and the table hold it.
pub(crate) struct Covered {
    pub coverage: usize,
    pub records: usize,
    stated: usize,
    stride: usize,
}

impl Covered {
    // MathItalicsCorrectionInfo, MathTopAccentAttachment or MathKernInfo, its records `stride` bytes.
    pub fn at(math: &[u8], at: usize, stride: usize) -> Option<Self> {
        let stated = usize::from(read_u16_be(math, at + 2)?);
        Some(Covered { coverage: offset16_in(math, at, 0)?, records: at + 4, stated, stride })
    }

    // One direction of a MathVariants: its coverage and construction offsets.
    pub fn variants(math: &[u8], variants: usize, vertical: bool) -> Option<Self> {
        let (coverage, count) = if vertical { (2, 6) } else { (4, 8) };
        let before = if vertical { 0 } else { usize::from(read_u16_be(math, variants + 6)?) };
        let stated = usize::from(read_u16_be(math, variants + count)?);
        Some(Covered {
            coverage: offset16_in(math, variants, coverage)?,
            records: variants + 10 + 2 * before,
            stated,
            stride: 2,
        })
    }

    pub fn count(&self, math: &[u8]) -> usize {
        self.stated.min(math.len().saturating_sub(self.records) / self.stride)
    }

    // The record of `gid`. Coverage and records are sized apart, so one the Coverage lists past the
    // records has none, rather than reading whatever follows them as its value.
    pub fn index(&self, math: &[u8], gid: u32) -> Option<usize> {
        coverage_index(math, self.coverage, gid).filter(|&i| i < self.count(math))
    }
}

// A MathGlyphConstruction: its variant records and its GlyphAssembly, if that reads.
pub(crate) struct Construction {
    pub variants: usize,
    pub variant_count: usize,
    pub assembly: Option<Assembly>,
}

// A GlyphAssembly, only whole, since its parts build one glyph between them.
pub(crate) struct Assembly {
    pub at: usize,
    pub parts: usize,
    pub part_count: usize,
}

pub(crate) fn construction(math: &[u8], at: usize) -> Option<Construction> {
    let stated = usize::from(read_u16_be(math, at + 2)?);
    let assembly = offset16_in(math, at, 0).and_then(|a| {
        let part_count = usize::from(read_u16_be(math, a + 4)?);
        records_fit(a + 6, part_count, 10, math.len()).then_some(Assembly { at: a, parts: a + 6, part_count })
    });
    Some(Construction { variants: at + 4, variant_count: stated.min(math.len().saturating_sub(at + 4) / 4), assembly })
}

// A MathKern's height count, where its heights and the kerns around them all lie in the table: one cut
// short would pair a height with another's kern.
pub(crate) fn kern_heights(math: &[u8], at: usize) -> Option<usize> {
    let heights = usize::from(read_u16_be(math, at)?);
    records_fit(at + 2, 2 * heights + 1, 4, math.len()).then_some(heights)
}
