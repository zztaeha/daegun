use super::*;

// Without maxp the font counts no glyphs, so its CharStrings count differs and the instance refuses the font.
#[test]
fn a_font_without_maxp_has_no_instance() {
    let path =
        alloc::format!("{}/assets/test-fonts/source-serif/SourceSerif4Variable-Roman.otf", env!("CARGO_MANIFEST_DIR"));
    let mut map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(&path).expect("a fixture"))
        .expect("it parses");
    let axes = [(AxisTag::new(b"wght"), 900.0)];
    assert!(instance_cff2_from_map(&map, &Reads::of_tables(&map), &axes, Instancing::Whole, true).is_ok());
    map.remove("maxp");
    let refused = instance_cff2_from_map(&map, &Reads::of_tables(&map), &axes, Instancing::Whole, true).map(drop);
    assert_eq!(refused, Err(crate::daecore::Error::malformed("CFF2: CharStrings count differs from maxp's")));
}
