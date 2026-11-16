use daegun::{Font, Location, Orientation};
use daegun::raw::bytes::read_u16_be;
use daegun::Tag;
use crate::located;

const FACES: &[&str] = &[
    "inter/InterVariable.ttf",
    "eb-garamond/EBGaramond.ttf",
    "noto-devanagari/NotoSansDevanagari.ttf",
    "noto-khmer/NotoSansKhmer.ttf",
    "colr-v1-test-glyphs/test_glyphs.ttf",
    "stix-two-math/STIX2Math.otf",
];

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{rel} did not parse: {e}"))
}

#[test]
fn cached_scalars_agree_with_the_tables() {
    for rel in FACES {
        let font = font(rel);
        let want_upm = font.table(Tag::new(b"head")).and_then(|h| read_u16_be(h, 18)).expect("a unitsPerEm");
        assert_eq!(font.units_per_em(), want_upm, "{rel}: cached upm disagrees with head");
        let want_glyphs =
            font.table(Tag::new(b"maxp")).and_then(|m| read_u16_be(m, 4)).map(u32::from).expect("a glyph count");
        assert_eq!(font.glyph_count(), want_glyphs, "{rel}: cached num_glyphs disagrees with maxp");

        assert_eq!(
            font.os2(&Location::default()).is_some(),
            font.has_table(Tag::new(b"OS/2")),
            "{rel}: OS/2 presence disagrees with the table map"
        );

        assert_eq!(
            font.cap_height(&Location::default()),
            font.cap_height(&Location::default()),
            "{rel}: cap_height is not stable"
        );
        assert_eq!(
            font.ascender(&Location::default()),
            font.ascender(&Location::default()),
            "{rel}: ascender is not stable"
        );
        let a = font.line_metrics(&Location::default(), Orientation::Horizontal);
        let b = font.line_metrics(&Location::default(), Orientation::Horizontal);
        assert_eq!(a.ascent, b.ascent, "{rel}: line_metrics is not stable");
        assert_eq!(a.descent, b.descent, "{rel}: line_metrics is not stable");
    }
}

#[test]
fn an_instance_caches_its_own_tables() {
    let font = font("inter/InterVariable.ttf");
    let instanced = font.instance(&located(&font, &[("wght", 700.0)])).expect("instanced");
    let sub = Font::from_bytes(&instanced).expect("the instance parses");

    let want = sub.table(Tag::new(b"head")).and_then(|h| read_u16_be(h, 18)).expect("a unitsPerEm");
    assert_eq!(sub.units_per_em(), want, "the instance's cached upm disagrees with its own head");
    assert_eq!(sub.units_per_em(), font.units_per_em(), "instancing changed the em, which it must not");
}
