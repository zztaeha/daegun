use alloc::vec::Vec;
use super::super::decoder::{offset16_in, read_u16_be, resolve_offset16};
use super::{construction, math_value, Covered, Deltas};
use crate::daecore::daetype::Tables;
use crate::daecore::GlyphId;

/// One ready-made size of a glyph that stretches. Laid out as C reads it.
#[non_exhaustive]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MathGlyphVariant {
    /// The glyph of that size.
    pub glyph: GlyphId,
    /// Its advance along the way it stretches, in font units.
    pub advance: f64,
}

/// One part of a glyph assembled to any size, in font units along the way it stretches. Laid out as C reads it.
#[non_exhaustive]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GlyphPart {
    /// The part's glyph.
    pub glyph: GlyphId,
    /// How much of its start may overlap the part before.
    pub start_connector_length: f64,
    /// How much of its end may overlap the part after.
    pub end_connector_length: f64,
    /// Its whole advance.
    pub full_advance: f64,
    /// Whether it repeats, zero or more times, to fill the length.
    pub extender: bool,
}

/// How to build a glyph to any size from parts, start to end.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct GlyphAssembly {
    /// The italic correction of the built glyph, in font units.
    pub italics_correction: f64,
    /// The parts, from the bottom or the left.
    pub parts: Vec<GlyphPart>,
}

/// How a glyph stretches one way: its ready-made sizes, smallest first, and how to build it larger.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub struct MathGlyphConstruction {
    /// How to build it from parts; None where the font gives only sizes.
    pub assembly: Option<GlyphAssembly>,
    /// Its sizes, smallest first.
    pub variants: Vec<MathGlyphVariant>,
}

pub fn math_min_connector_overlap(table_map: &Tables) -> Option<u16> {
    let math = table_map.get("MATH")?;
    read_u16_be(math, offset16_in(math, 0, 8)?)
}

pub fn math_glyph_construction(
    table_map: &Tables, gid: u32, vertical: bool, deltas: Deltas<'_>,
) -> Option<MathGlyphConstruction> {
    let math = table_map.get("MATH")?;
    let variants = offset16_in(math, 0, 8)?;
    let direction = Covered::variants(math, variants, vertical)?;
    let found =
        construction(math, resolve_offset16(math, direction.records + 2 * direction.index(math, gid)?, variants)?)?;

    let assembly = found.assembly.and_then(|a| {
        let mut parts = Vec::with_capacity(a.part_count);
        for rec in (0..a.part_count).map(|k| a.parts + 10 * k) {
            let field = |i: usize| read_u16_be(math, rec + 2 * i);
            parts.push(GlyphPart {
                glyph: GlyphId(field(0)?.into()),
                start_connector_length: field(1)?.into(),
                end_connector_length: field(2)?.into(),
                full_advance: field(3)?.into(),
                extender: field(4)? & 0x0001 != 0,
            });
        }
        Some(GlyphAssembly { italics_correction: math_value(math, a.at, a.at, deltas)?, parts })
    });
    let mut variants = Vec::with_capacity(found.variant_count);
    for rec in (0..found.variant_count).map(|k| found.variants + 4 * k) {
        let glyph = GlyphId(read_u16_be(math, rec)?.into());
        variants.push(MathGlyphVariant { glyph, advance: read_u16_be(math, rec + 2)?.into() });
    }
    Some(MathGlyphConstruction { assembly, variants })
}
