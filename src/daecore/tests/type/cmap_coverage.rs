use daegun::Font;

// A subset must be able to address every codepoint it was asked to carry.
// Format 4 stops at the BMP, so anything above it needs a format 12 subtable;
// without one the glyph is embedded but unreachable through the cmap.
fn roundtrip(rel: &str, text: &str) -> (usize, usize) {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).expect("font opens");
    let font = Font::from_bytes(&bytes).expect("font parses");

    let wanted: Vec<char> = text.chars().filter(|&c| font.glyph_id(c as u32).is_some()).collect();
    let subset = font.subset_text(text, &[]).expect("subsets");
    let out = Font::from_bytes(&subset.ttf).expect("subset parses");

    let addressable = wanted.iter().filter(|&&c| out.glyph_id(c as u32).is_some()).count();
    (addressable, wanted.len())
}

#[test]
fn supplementary_plane_codepoints_survive_subsetting() {
    // U+1D400 MATHEMATICAL BOLD CAPITAL A and neighbours, all above the BMP
    let math: String = (0x1D400u32..0x1D40Bu32).filter_map(char::from_u32).collect();
    let text = format!("Az09 {math}");
    let (got, want) = roundtrip("stix-two-math/STIX2Math.otf", &text);
    assert!(want > 0, "fixture addressed nothing");
    assert_eq!(got, want, "STIX2Math: only {got}/{want} codepoints addressable in the subset");
}

#[test]
fn private_use_plane_codepoints_survive_subsetting() {
    // these COLR fixtures put every glyph in plane 15 PUA
    for rel in ["colr-v1-test-glyphs/test_glyphs.ttf", "colr-v1-test-glyphs/test_glyphs_variable.ttf"] {
        let pua: String = (0xF0100u32..0xF0130u32).filter_map(char::from_u32).collect();
        let (got, want) = roundtrip(rel, &pua);
        assert!(want > 0, "{rel}: fixture addressed nothing");
        assert_eq!(got, want, "{rel}: only {got}/{want} codepoints addressable in the subset");
    }
}

#[test]
fn bmp_only_fonts_keep_working() {
    let (got, want) = roundtrip("inter/InterVariable.ttf", "Hamburgefonstiv 0123 &@#");
    assert!(want > 0);
    assert_eq!(got, want, "Inter: only {got}/{want} codepoints addressable");
}
