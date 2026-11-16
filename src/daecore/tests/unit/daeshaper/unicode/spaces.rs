use super::*;

// Each space's kind survives the glyph's byte, which is never 0; a byte no kind writes reads as none.
#[test]
fn a_spaces_kind_reads_back_from_its_byte() {
    let spaces = (0..=0x10FFFF).filter_map(char::from_u32).filter_map(space_fallback);
    assert_eq!(spaces.clone().count(), 16);
    for kind in spaces {
        assert_ne!(kind.to_byte(), 0, "{kind:?}");
        assert_eq!(SpaceFallback::from_byte(kind.to_byte()), Some(kind));
    }
    for unwritten in [0, 6, FRACTION - 1, FRACTION] {
        assert_eq!(SpaceFallback::from_byte(unwritten), None, "{unwritten:#x}");
    }
}
