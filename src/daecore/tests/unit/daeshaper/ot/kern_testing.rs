use super::*;

// Each format 3 subtable's value for a pair, whatever its coverage, as `format3` reads it.
pub(crate) fn format3_values(table: &[u8], left: u32, right: u32) -> Vec<i32> {
    let kind = match read_u16_be(table, 0) {
        Some(0) => Kind::KernOt,
        Some(1) => Kind::KernAat,
        _ => return Vec::new(),
    };
    kerx::subtables(table, kind).filter(|s| s.format == 3).map(|s| format3(&s, left, right)).collect()
}

// A font of no glyphs covers nothing, and its last glyph isn't worked out: a caller may have overflow checks on.
#[test]
fn format3_sets_of_a_font_with_no_glyphs_are_empty() {
    let kern = [0, 0, 0, 1, 0, 0, 0, 12, 3, 1, 0, 1, 1, 1, 1, 0];
    let sub = kerx::subtables(&kern, Kind::KernOt).next().expect("a subtable");
    assert_eq!(sub.format, 3);
    assert!(!format3_sets(&sub, 0).first.contains(0));
    assert!(format3_sets(&sub, 1).first.contains(0));
}
