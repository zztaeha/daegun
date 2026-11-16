use super::*;

// Charsets in each format, and the predefined ones: SID to glyph, the first glyph naming a SID.
#[test]
fn a_sid_finds_its_glyph_through_every_charset() {
    let format0 = [0u8, 0, 34, 0, 125, 0, 34];
    let map = SidMap::new(&format0, Charset::At(0), 4);
    assert_eq!((map.gid(34), map.gid(125), map.gid(0), map.gid(35)), (Some(1), Some(2), Some(0), None));

    let format1 = [1u8, 1, 0x87, 2];
    assert_eq!(SidMap::new(&format1, Charset::At(0), 4).gid(0x189), Some(3));
    let format2 = [2u8, 1, 0x87, 0, 2];
    assert_eq!(SidMap::new(&format2, Charset::At(0), 4).gid(0x188), Some(2));

    assert_eq!(SidMap::new(&[], Charset::Predefined(0), 40).gid(34), Some(34), "ISOAdobe is SID for glyph");
    let expert = SidMap::new(&[], Charset::Predefined(1), 6);
    assert_eq!(expert.gid(229), Some(2), "Expert names exclamsmall second after space");
    let subset = SidMap::new(&[], Charset::Predefined(2), 6);
    assert_eq!(subset.gid(231), Some(2), "ExpertSubset starts dollaroldstyle there");
}

// A font of more glyphs than its predefined charset has is named by none of it, for a seac as for
// glyph names.
#[test]
fn a_predefined_charset_smaller_than_its_font_names_no_glyph() {
    assert_eq!(SidMap::new(&[], Charset::Predefined(0), 300).gid(34), None, "ISOAdobe");
    assert_eq!(SidMap::new(&[], Charset::Predefined(1), 200).gid(229), None, "Expert");
    assert_eq!(SidMap::new(&[], Charset::Predefined(2), 100).gid(231), None, "Expert Subset");
}
