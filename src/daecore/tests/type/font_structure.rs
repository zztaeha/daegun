use crate::outline::OutlinePen;
use crate::shape::ShapeOptions;
use crate::{Font, GlyphId, Location};
use alloc::vec::Vec;

const BOTH_OUTLINES: &str = "structure/TestSFNTTwo.ttf";
const MAC_TURKISH: &str = "structure/TestCMAPMacTurkish.ttf";
const SHORT_AXIS_TAGS: &str = "structure/TestGVAREight.ttf";

fn with_font<R>(rel: &str, body: impl FnOnce(&Font) -> R) -> R {
    let path = format!("{}/{}", super::FONTS, rel);
    let bytes = std::fs::read(&path).expect("font opens");
    let font = Font::from_bytes(&bytes).expect("font parses");
    body(&font)
}

#[derive(Default)]
struct Verbs {
    quads: usize,
    cubics: usize,
}

impl OutlinePen for Verbs {
    fn move_to(&mut self, _: f32, _: f32) {}
    fn line_to(&mut self, _: f32, _: f32) {}
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.quads += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.cubics += 1;
    }
    fn close(&mut self) {}
}

// This font carries both outline tables, which is malformed, and sfntVersion is the only thing that
// says which one wins. 0x00010000 means the glyf outlines, so the curves come back quadratic.
#[test]
fn sfnt_version_picks_the_outline_table() {
    with_font(BOTH_OUTLINES, |font| {
        let gid = font.glyph_id('A').expect("font has A");
        let mut verbs = Verbs::default();
        font.outline(gid, &Location::default(), &mut verbs).expect("A outlines");
        assert!(verbs.quads > 0, "expected glyf quadratics, got none");
        assert_eq!(verbs.cubics, 0, "cubics mean the CFF table was read instead");
    });
}

// MacOS Turkish reassigns seven bytes away from MacRoman. Read as MacRoman these six resolve to
// .notdef, because the letters Turkish needs sit where MacRoman keeps punctuation and ligatures.
#[test]
fn mac_turkish_cmap_resolves_turkish_letters() {
    with_font(MAC_TURKISH, |font| {
        for (ch, byte) in [
            ('Ğ', 0xDAu8),
            ('ğ', 0xDB),
            ('İ', 0xDC),
            ('ı', 0xDD),
            ('Ş', 0xDE),
            ('ş', 0xDF),
        ] {
            let gid = font.glyph_id(ch);
            assert!(
                gid.is_some_and(|g| g.0 != 0),
                "{ch} (byte 0x{byte:02X}) resolved to {gid:?}, expected a real glyph"
            );
        }
    });
}

// The same subtable still has to answer for plain ASCII, which Turkish and MacRoman agree on.
#[test]
fn mac_turkish_cmap_keeps_the_shared_range() {
    with_font(MAC_TURKISH, |font| {
        for ch in ['A', 'z', '0'] {
            assert!(
                font.glyph_id(ch).is_some_and(|g| g.0 != 0),
                "{ch} lost its glyph"
            );
        }
    });
}

#[derive(Default, PartialEq, Debug)]
struct Points(Vec<(f32, f32)>);

impl OutlinePen for Points {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn close(&mut self) {}
}

// `fvar` stores a tag shorter than four bytes space-padded, so an unpadded tag from a caller matches
// nothing and the axis is silently ignored rather than refused.
#[test]
fn short_axis_tags_are_padded_to_four_bytes() {
    with_font(SHORT_AXIS_TAGS, |font| {
        let gid = font.glyph_id('H').expect("font has H");
        let outline = |axes: &[(&str, f64)]| {
            let mut pen = Points::default();
            font.outline(gid, &super::located(font, axes), &mut pen).expect("H outlines");
            pen
        };

        let default = outline(&[]);
        let short = outline(&[("TC", 1.0)]);
        let padded = outline(&[("TC  ", 1.0)]);

        assert_ne!(default, short, "the TC axis did not move the outline, so it was ignored");
        assert_eq!(short, padded, "a short tag must reach the same axis as its padded form");
    });
}

const TAI_THAM: &str = "structure/TestShapeLana.ttf";

fn shaped(font: &Font, text: &str) -> Vec<GlyphId> {
    font.shape(text, &Location::default(), &ShapeOptions::default()).glyphs.clone()
}

// The USE pattern orders a cluster's vowels by position and Tai Tham does not, so the
// specification marks ordinary words broken. See scripts/data/grammars/lana.grammar.
#[test]
fn tai_tham_vowel_orders_are_not_broken_clusters() {
    with_font(TAI_THAM, |font| {
        let dotted = font.glyph_id('\u{25CC}').expect("font has a dotted circle");
        // U+1A20 HIGH KA, U+1A6E VOWEL SIGN E, U+1A6C VOWEL SIGN OA BELOW, U+1A68 VOWEL SIGN UUE:
        // a Top vowel after a Bottom one, which the specification's pattern forbids.
        for text in ["\u{1A20}\u{1A6E}\u{1A6C}\u{1A68}", "\u{1A36}\u{1A76}\u{1A63}\u{1A74}"] {
            let glyphs = shaped(font, text);
            assert!(
                !glyphs.contains(&dotted),
                "{text:?} shaped to {glyphs:?}, which includes the dotted circle {dotted}"
            );
        }
    });
}

// The relaxation must not accept everything: a mark with no base before it is still broken.
#[test]
fn tai_tham_still_marks_a_baseless_cluster() {
    with_font(TAI_THAM, |font| {
        let dotted = font.glyph_id('\u{25CC}').expect("font has a dotted circle");
        let glyphs = shaped(font, "\u{1A6C}");
        assert!(
            glyphs.contains(&dotted),
            "a lone vowel sign shaped to {glyphs:?} with no dotted circle"
        );
    });
}
