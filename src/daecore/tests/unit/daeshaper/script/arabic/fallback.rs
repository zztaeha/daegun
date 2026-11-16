use super::*;

// Glyphs sorted stably from code order: of a glyph three codes share, the first entry, the lowest code's, stands.
#[test]
fn a_glyph_several_codes_share_takes_the_lowest_codes_entry() {
    let entries = |glyphs: &[u16]| resolve(glyphs, |i| i).unwrap_or_default();
    assert_eq!(entries(&[5, 6, 7, 7, 8]), [(5, 0), (6, 1), (7, 2), (8, 4)]);
    assert_eq!(entries(&[7, 7, 7, 7]), [(7, 0)]);
    assert_eq!(entries(&[]), []);
}
