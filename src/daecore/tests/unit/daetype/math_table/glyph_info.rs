use super::*;
use alloc::string::String;
use crate::daecore::daetype::TableBytes;

// A NULL MathGlyphInfo offset is no subtable, not the MATH header read as one.
#[test]
fn a_null_glyph_info_offset_is_absent() {
    let math: alloc::vec::Vec<u8> = [1u16, 0, 10, 0, 10].iter().flat_map(|v| v.to_be_bytes()).collect();
    let map: Tables = [(String::from("MATH"), TableBytes::from(math))].into_iter().collect();
    assert!(glyph_info_offset(&map).is_none());
}
