use super::*;

// Each glyph reads on either side of a page's edge and in the last page, and none past glyf's glyphs; a page is
// charged once, as its first glyph is read.
#[test]
fn glyphs_read_from_their_pages() {
    let path = alloc::format!("{}/assets/test-fonts/inter/InterVariable.ttf", env!("CARGO_MANIFEST_DIR"));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("a fixture"));
    let fc = crate::daecore::daekit::cache::FontCache::new(map.expect("parses")).expect("loads");
    let tables = fc.glyf_tables().expect("glyf");
    let measured = MeasuredGlyphs::new(Shared::clone(tables), &[0.7]);
    let (last, before) = (measured.glyphs() as u32 - 1, measured.bytes());
    for gid in [0, 31, 32, last, 31] {
        assert!(measured.of(gid).is_some(), "glyph {gid}");
    }
    assert_eq!(measured.of(last + 1), None);
    assert_eq!(measured.bytes() - before, 3 * size_of::<Page>());
}
