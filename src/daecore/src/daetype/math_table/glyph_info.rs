use super::super::decoder::offset16_in;
use super::{coverage_index, kern_heights, math_value, Covered, Deltas};
use crate::daecore::daetype::Tables;

fn glyph_info_offset(table_map: &Tables) -> Option<(&[u8], usize)> {
    let math = table_map.get("MATH")?;
    Some((math, offset16_in(math, 0, 6)?))
}

// A glyph's value in the MathItalicsCorrectionInfo or MathTopAccentAttachment at `at`.
fn parallel_math_value(math: &[u8], at: usize, gid: u32, deltas: Deltas<'_>) -> Option<f64> {
    let values = Covered::at(math, at, 4)?;
    math_value(math, values.records + 4 * values.index(math, gid)?, at, deltas)
}

pub fn math_italics_correction(table_map: &Tables, gid: u32, deltas: Deltas<'_>) -> Option<f64> {
    let (math, glyph_info) = glyph_info_offset(table_map)?;
    parallel_math_value(math, offset16_in(math, glyph_info, 0)?, gid, deltas)
}

pub fn math_top_accent_attachment(table_map: &Tables, gid: u32, deltas: Deltas<'_>) -> Option<f64> {
    let (math, glyph_info) = glyph_info_offset(table_map)?;
    parallel_math_value(math, offset16_in(math, glyph_info, 2)?, gid, deltas)
}

pub fn math_is_extended_shape(table_map: &Tables, gid: u32) -> bool {
    glyph_info_offset(table_map)
        .and_then(|(math, glyph_info)| coverage_index(math, offset16_in(math, glyph_info, 4)?, gid))
        .is_some()
}

/// Which corner of a glyph a script attaches at, for its kern.
#[expect(clippy::exhaustive_enums, reason = "MATH kerns at these four corners")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MathKernCorner {
    /// A superscript's, after the glyph.
    TopRight,
    /// A superscript's, before the glyph.
    TopLeft,
    /// A subscript's, after the glyph.
    BottomRight,
    /// A subscript's, before the glyph.
    BottomLeft,
}

// The kern at `height` in design units: a height below a correction height by any amount selects the kern before
// it, each height and kern with its delta at a location.
pub fn math_kern(table_map: &Tables, gid: u32, corner: MathKernCorner, height: f64, mut deltas: Deltas<'_>) -> f64 {
    let mut kern = || {
        let (math, glyph_info) = glyph_info_offset(table_map)?;
        let info = offset16_in(math, glyph_info, 6)?;
        let records = Covered::at(math, info, 8)?;
        let corner = match corner {
            MathKernCorner::TopRight => 0,
            MathKernCorner::TopLeft => 2,
            MathKernCorner::BottomRight => 4,
            MathKernCorner::BottomLeft => 6,
        };
        let at = offset16_in(math, info, 4 + 8 * records.index(math, gid)? + corner)?;
        let heights = kern_heights(math, at)?;
        let mut below = |i: usize| {
            math_value(math, at + 2 + 4 * i, at, deltas.as_deref_mut()).is_some_and(|h| height < h)
        };
        let selected = (0..heights).find(|&i| below(i)).unwrap_or(heights);
        math_value(math, at + 2 + 4 * (heights + selected), at, deltas.as_deref_mut())
    };
    kern().unwrap_or(0.0)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/math_table/glyph_info.rs"]
mod tests;
