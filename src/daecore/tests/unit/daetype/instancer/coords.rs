use super::*;
use alloc::vec;

// A glyph whose one component says more follow keeps the bytes after it as they were, and a point
// number of 200 stays a byte while an offset moved past one widens to a word.
#[test]
fn a_composite_moves_its_offsets_and_keeps_its_point_numbers() {
    let mut glyph = vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
    glyph.extend([0x00, 0x20, 0, 1, 200, 3]);
    glyph.extend([0x00, 0x02, 0, 2, 10, 20, 0xAB]);
    let mut out = Vec::new();
    write_composite_glyph(&glyph, false, &[(0.0, 0.0), (210.0, 15.0)], &mut out);
    assert_eq!(&out[10..16], [0x00, 0x20, 0, 1, 200, 3], "the point numbers moved");
    assert_eq!(&out[16..], [0x00, 0x03, 0, 2, 0, 210, 0, 15, 0xAB], "the offset did not widen, or the tail went");
}

// Two equal flags are written whole and three take a repeat count, as a repeat saves nothing on two; the cubic
// bit carries over in GLYF and is dropped in glyf.
#[test]
fn flags_take_a_repeat_count_from_three() {
    let glyph = [0, 1, 0, 0, 0, 0, 0, 30, 0, 0, 0, 4, 0, 0];
    let points = [(0.0, 0.0), (0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 0.0)];
    let (mut cc, mut flags) = (GlyphCoords::default(), Vec::new());
    for (extended, cubic) in [(true, 0x80), (false, 0)] {
        let mut out = Vec::new();
        write_simple_glyph((&glyph, extended), 1, &points, &[0x81; 5], &mut cc, &mut flags, &mut out);
        assert_eq!(out[14..], [0x31 | cubic, 0x31 | cubic, 0x3B | cubic, 2, 10, 10, 10], "GLYF {extended}");
    }
}
