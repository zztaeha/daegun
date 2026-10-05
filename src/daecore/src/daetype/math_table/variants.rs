use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{read_u16_be, records_fit};
use super::{coverage_index, read_math_value};
use crate::daecore::daetype::TableBytes;

pub struct MathGlyphVariant { pub glyph_id: u16, pub advance: u16 }

pub struct GlyphPart {
    pub glyph_id: u16,
    pub start_connector_length: u16,
    pub end_connector_length: u16,
    pub full_advance: u16,
    pub is_extender: bool,
}

pub struct GlyphAssembly { pub italics_correction: i16, pub parts: Vec<GlyphPart> }

pub struct MathGlyphConstruction {
    pub assembly: Option<GlyphAssembly>,
    pub variants:  Vec<MathGlyphVariant>,
}

fn variants_offset(table_map: &BTreeMap<String, TableBytes>) -> Option<(&[u8], usize)> {
    let math = table_map.get("MATH")?;
    let off = read_u16_be(math, 8).filter(|&o| o != 0)? as usize;
    Some((math, off))
}

pub fn math_min_connector_overlap(table_map: &BTreeMap<String, TableBytes>) -> Option<u16> {
    let (math, variants_off) = variants_offset(table_map)?;
    read_u16_be(math, variants_off)
}

pub fn math_glyph_construction(
    table_map: &BTreeMap<String, TableBytes>, gid: u16, vertical: bool,
) -> Option<MathGlyphConstruction> {
    let (math, variants_off) = variants_offset(table_map)?;

    let cov_field_off  = if vertical { variants_off + 2 } else { variants_off + 4 };
    let count_field_off = if vertical { variants_off + 6 } else { variants_off + 8 };
    let array_off = variants_off + 10 + if vertical { 0 } else {
        read_u16_be(math, variants_off + 6)? as usize * 2
    };

    let cov_off = variants_off + read_u16_be(math, cov_field_off)? as usize;
    let count = read_u16_be(math, count_field_off)? as usize;
    let idx = coverage_index(math, cov_off, gid)?;
    if idx >= count { return None; }

    let construction_rel = read_u16_be(math, array_off + idx * 2)? as usize;
    let construction_off = variants_off + construction_rel;

    let assembly_rel = read_u16_be(math, construction_off)? as usize;
    let assembly = if assembly_rel == 0 {
        None
    } else {
        Some(parse_glyph_assembly(math, construction_off + assembly_rel)?)
    };

    let variant_count = read_u16_be(math, construction_off + 2)? as usize;
    if !records_fit(construction_off + 4, variant_count, 4, math.len()) { return None; }
    let mut variants = Vec::with_capacity(variant_count);
    for i in 0..variant_count {
        let rec = construction_off + 4 + i * 4;
        let glyph_id = read_u16_be(math, rec)?;
        let advance  = read_u16_be(math, rec + 2)?;
        variants.push(MathGlyphVariant { glyph_id, advance });
    }

    Some(MathGlyphConstruction { assembly, variants })
}

fn parse_glyph_assembly(math: &[u8], off: usize) -> Option<GlyphAssembly> {
    let italics_correction = read_math_value(math, off)?;
    let part_count = read_u16_be(math, off + 4)? as usize;
    if !records_fit(off + 6, part_count, 10, math.len()) { return None; }
    let mut parts = Vec::with_capacity(part_count);
    for i in 0..part_count {
        let rec = off + 6 + i * 10;
        let glyph_id = read_u16_be(math, rec)?;
        let start_connector_length = read_u16_be(math, rec + 2)?;
        let end_connector_length   = read_u16_be(math, rec + 4)?;
        let full_advance           = read_u16_be(math, rec + 6)?;
        let part_flags             = read_u16_be(math, rec + 8)?;
        parts.push(GlyphPart {
            glyph_id, start_connector_length, end_connector_length, full_advance,
            is_extender: part_flags & 0x0001 != 0,
        });
    }
    Some(GlyphAssembly { italics_correction, parts })
}
