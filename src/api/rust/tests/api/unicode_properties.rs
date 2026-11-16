use daegun::{Font, GlyphId, Location};
use daegun::shape::{Direction, ShapeOptions};
use daegun::unicode::vertical_form;

use super::aat_subset::{carets_with, glyph_ids, u16s, u32s, F, F_F_I, F_I, I, L};
use super::cmap_reading::{cmap, format14};

// A format 12 of (first code, last code, first glyph) groups.
fn format12(groups: &[(u32, u32, u16)]) -> Vec<u8> {
    let body: Vec<u8> = groups.iter().flat_map(|&(a, b, glyph)| u32s(&[a, b, u32::from(glyph)])).collect();
    [u16s(&[12, 0]), u32s(&[16 + body.len() as u32, 0, groups.len() as u32]), body].concat()
}

pub(super) fn mapping(groups: &[(u32, u32, u16)]) -> Vec<(&'static str, Vec<u8>)> {
    vec![("cmap", cmap(&[(3, 10, &format12(groups))]))]
}

fn shaped(f: &Font, text: &str, vertical: bool) -> (Vec<GlyphId>, Vec<u32>) {
    let options = ShapeOptions::default();
    let options = if vertical { options.with_direction(Direction::TopToBottom) } else { options };
    let run = f.shape(text, &Location::default(), &options);
    (run.glyphs.clone(), run.clusters.clone())
}

// The Hangul fillers and Duployan's shorthand format controls are drawn, where ZWSP, a default ignorable like them,
// is not.
#[test]
fn a_filler_or_shorthand_control_is_drawn() {
    let groups = [(0x115F, 0x1160, F), (0x200B, 0x200B, L), (0x3164, 0x3164, L), (0xFFA0, 0xFFA0, F_F_I),
        (0x1BCA0, 0x1BCA3, F)];
    let text = "\u{115F}\u{1160}\u{3164}\u{FFA0}\u{1BCA0}\u{1BCA1}\u{1BCA2}\u{1BCA3}\u{200B}";
    let want = (glyph_ids(&[F, I, L, F_F_I, F, I, L, F_F_I]), vec![0, 0, 6, 9, 12, 16, 20, 24]);
    assert_eq!(shaped(&carets_with(&mapping(&groups)), text, false), want);
}

// Telugu's length marks sort as classes 4 and 5, so U+0C55 goes before U+0C56; Tai Viet's vowels take the positions
// the positional data gives, so two of them make no broken cluster.
#[test]
fn marks_take_the_classes_and_positions_of_their_data() {
    let telugu = carets_with(&mapping(&[(0x0C2A, 0x0C2A, F), (0x0C55, 0x0C55, I), (0x0C56, 0x0C56, L)]));
    assert_eq!(shaped(&telugu, "\u{0C2A}\u{0C56}\u{0C55}", false).0, glyph_ids(&[F, I, L]));
    let tai_viet = carets_with(&mapping(&[(0x25CC, 0x25CC, F_I), (0xAA80, 0xAA80, F), (0xAAB3, 0xAAB4, I)]));
    assert_eq!(shaped(&tai_viet, "\u{AA80}\u{AAB3}\u{AAB4}", false).0, glyph_ids(&[F, L, I]));
}

// A Mongolian free variation selector doesn't pick a cmap variation: its base keeps its own glyph, and it is hidden.
// A halfwidth katakana voicing mark joins the cluster before it.
#[test]
fn a_selector_or_sound_mark_stays_with_its_base() {
    let selectors = cmap(&[(0, 5, &format14(0x180B, 0x41, (0x1820, L))),
        (3, 10, &format12(&[(0x180B, 0x180B, I), (0x1820, 0x1820, F)]))]);
    assert_eq!(shaped(&carets_with(&[("cmap", selectors)]), "\u{1820}\u{180B}", false), (glyph_ids(&[F]), vec![0]));
    let katakana = carets_with(&mapping(&[(0xFF76, 0xFF76, F), (0xFF9E, 0xFF9E, I)]));
    assert_eq!(shaped(&katakana, "\u{FF76}\u{FF9E}", false), (glyph_ids(&[F, I]), vec![0, 0]));
}

// Fullwidth punctuation has vertical forms: a font without vert takes the form down a line.
#[test]
fn fullwidth_punctuation_has_vertical_forms() {
    let pairs = [
        ('\u{FF01}', '\u{FE15}'), ('\u{FF08}', '\u{FE35}'), ('\u{FF09}', '\u{FE36}'), ('\u{FF0C}', '\u{FE10}'),
        ('\u{FF1A}', '\u{FE13}'), ('\u{FF1B}', '\u{FE14}'), ('\u{FF1F}', '\u{FE16}'), ('\u{FF3B}', '\u{FE47}'),
        ('\u{FF3D}', '\u{FE48}'), ('\u{FF3F}', '\u{FE33}'), ('\u{FF5B}', '\u{FE37}'), ('\u{FF5D}', '\u{FE38}'),
    ];
    for (c, form) in pairs {
        assert_eq!(vertical_form(c), Some(form), "{c}");
    }
    let f = carets_with(&mapping(&[(0xFE15, 0xFE15, I), (0xFF01, 0xFF01, F)]));
    assert_eq!(shaped(&f, "\u{FF01}", true).0, glyph_ids(&[I]));
}

// The Indic shapers class each character Unicode's Indic properties list for their scripts, a newer block's too: a
// mark of theirs with no base takes a dotted circle, and a pre-base matra moves before a newer block's letter.
#[test]
fn indic_properties_class_their_scripts_characters_in_any_block() {
    let f = carets_with(&mapping(&[(0x0915, 0x0915, F), (0x093F, 0x093F, F_I), (0x0995, 0x0995, F),
        (0x09BF, 0x09BF, F_I), (0x1DFB, 0x1DFB, L), (0x20F0, 0x20F0, L), (0x25CC, 0x25CC, I),
        (0x11B0A, 0x11B0A, F_F_I), (0x11DF0, 0x11DF0, L), (0x11DF1, 0x11DF1, F_F_I)]));
    for text in ["\u{0915} \u{1DFB}", "\u{0915} \u{20F0}", "\u{0995} \u{11DF0}"] {
        assert_eq!(shaped(&f, text, false), (glyph_ids(&[F, 0, I, L]), vec![0, 3, 3, 3]), "{text:?}");
    }
    for text in ["\u{11B0A}\u{093F}", "\u{11DF1}\u{09BF}"] {
        assert_eq!(shaped(&f, text, false), (glyph_ids(&[F_I, F_F_I]), vec![0, 0]), "{text:?}");
    }
}
