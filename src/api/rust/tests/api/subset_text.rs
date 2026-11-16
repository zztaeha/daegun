use daegun::{Font, GlyphId, Location};
use daegun::shape::{Direction, ShapeOptions};

use super::aat_subset::{carets_with, glyph_ids, u16s, F, F_F_L, F_I, I, L};

// carets.ttf whose cmap maps each (code, glyph) and nothing else, in one format 12 subtable.
fn mapping(pairs: &[(char, u16)]) -> Font {
    let mut pairs = pairs.to_vec();
    pairs.sort();
    let mut groups = Vec::new();
    for &(c, g) in &pairs {
        groups.extend([c as u32, c as u32, u32::from(g)].iter().flat_map(|w| w.to_be_bytes()));
    }
    let header = [u16s(&[12, 0]), (16 + groups.len() as u32).to_be_bytes().to_vec(), vec![0; 4]].concat();
    let count = (pairs.len() as u32).to_be_bytes().to_vec();
    let cmap = [u16s(&[0, 1, 3, 10]), 12u32.to_be_bytes().to_vec(), header, count, groups].concat();
    carets_with(&[("cmap", cmap)])
}

// The font's glyphs and advances for `text`, which its text subset gives too, glyph for glyph through the gid map.
fn alike(f: &Font, text: &str, vertical: bool, beginning_of_text: bool) -> (Vec<GlyphId>, Vec<f64>) {
    let options = ShapeOptions::default().with_beginning_of_text(beginning_of_text);
    let options = if vertical { options.with_direction(Direction::TopToBottom) } else { options };
    let want = f.shape(text, &Location::default(), &options);
    let out = f.subset_text(text, &Location::default()).expect("a subset");
    let sub = Font::from_vec(out.font.clone(), 0).expect("the subset parses");
    let got = sub.shape(text, &Location::default(), &options);
    let mapped: Vec<Option<GlyphId>> = want.glyphs.iter().map(|&g| out.renumbered(g)).collect();
    assert_eq!(mapped, got.glyphs.iter().map(|&g| Some(g)).collect::<Vec<_>>(), "{text:?} shapes otherwise");
    assert_eq!(want.advances, got.advances, "{text:?} advances otherwise");
    (want.glyphs.to_vec(), want.advances.to_vec())
}

// Decompositions and compositions: a code the font lacks, a base and mark, a decomposition's piece, Hebrew, Khmer's
// split vowels, Hangul both ways, and a letter the font has in Tamil text.
#[test]
fn a_text_subset_keeps_what_its_codes_decompose_and_compose_to() {
    for (pairs, text, want) in [
        (&[('\u{3B1}', F), ('\u{301}', I)][..], "\u{3AC}", &[F, I][..]),
        (&[('a', F), ('\u{301}', I), ('\u{E1}', L)], "a\u{301}", &[L]),
        (&[('a', F), ('\u{323}', I), ('\u{302}', L), ('\u{E2}', F_I)], "\u{1EAD}\u{323}", &[F_I, I, I]),
        (&[('\u{5D5}', F), ('\u{5B9}', I), ('\u{FB4B}', L)], "\u{5D5}\u{5B9}", &[L]),
        (&[('\u{1780}', F), ('\u{17C1}', I), ('\u{17BE}', L)], "\u{1780}\u{17BE}", &[I, F, L]),
        (&[('\u{1100}', F), ('\u{1161}', I), ('\u{AC00}', L)], "\u{1100}\u{1161}", &[L]),
        (&[('\u{AC00}', F), ('\u{11A8}', I), ('\u{AC01}', L)], "\u{AC00}\u{11A8}", &[L]),
        (&[('\u{1100}', F), ('\u{1161}', I), ('\u{11A8}', L)], "\u{AC01}", &[F, I, L]),
        (&[('\u{B95}', F_I), ('a', F), ('\u{301}', I), ('\u{E1}', L)], "\u{B95}\u{E1}", &[F_I, L]),
    ] {
        assert_eq!(alike(&mapping(pairs), text, false, false).0, glyph_ids(want), "{text:?}");
    }
}

// Thai's SARA AM splits in two and a tone mark takes its private-use form where the font has no Thai GSUB, a full
// stop its vertical form, and a parenthesis in right-to-left text its mirror.
#[test]
fn a_text_subset_keeps_the_forms_shaping_turns_its_codes_into() {
    for (pairs, text, vertical, want) in [
        (&[('\u{E19}', F), ('\u{E4D}', I), ('\u{E32}', L)][..], "\u{E19}\u{E33}", false, &[F, I, L][..]),
        (&[('\u{E1B}', F), ('\u{E48}', I), ('\u{F705}', L)], "\u{E1B}\u{E48}", false, &[F, L]),
        (&[('\u{3002}', F), ('\u{FE12}', I)], "\u{3002}", true, &[I]),
        (&[('(', F), (')', I), ('\u{5D0}', L)], "\u{5D0}(", false, &[I, L]),
    ] {
        assert_eq!(alike(&mapping(pairs), text, vertical, false).0, glyph_ids(want), "{text:?}");
    }
}

// A missing space takes the space glyph at its share of the em, or a digit's or comma's width as a figure or
// punctuation space; a hidden zero width space takes it too, and U+2011 the hyphen's glyph.
#[test]
fn a_text_subset_keeps_the_glyphs_shaping_draws_its_spaces_and_hyphens_with() {
    for (pairs, text, want, advances) in [
        (&[(' ', F)][..], "\u{2009}", &[F][..], &[200.0][..]),
        (&[(' ', F), ('7', F_I)], "\u{2007}", &[F], &[900.0]),
        (&[(' ', F), (',', F_F_L)], "\u{2008}", &[F], &[900.0]),
        (&[(' ', F), ('a', I)], "a\u{200B}a", &[I, F, I], &[400.0, 0.0, 400.0]),
        (&[('\u{2010}', F)], "\u{2011}", &[F], &[400.0]),
    ] {
        assert_eq!(alike(&mapping(pairs), text, false, false), (glyph_ids(want), advances.to_vec()), "{text:?}");
    }
}

// A mark opening the text, and a Malayalam dot reph with no consonant, each take a dotted circle when shaped as the
// beginning of text.
#[test]
fn a_text_subset_keeps_the_dotted_circle_its_text_can_take() {
    for (pairs, text, want) in [
        (&[('\u{301}', I), ('\u{25CC}', F)][..], "\u{301}", &[F, I][..]),
        (&[('\u{D4E}', I), ('\u{25CC}', F)], "\u{D4E}", &[F, I]),
    ] {
        assert_eq!(alike(&mapping(pairs), text, false, true).0, glyph_ids(want), "{text:?}");
    }
}

// A letter the font has is decomposed only beside a mark, and one whose mark it lacks not at all, so neither one's
// pieces join the subset; a mark from a decomposition opens no syllable, so it takes no dotted circle.
#[test]
fn a_text_subset_leaves_out_what_shaping_its_letters_never_reads() {
    let subset = |f: &Font, text: &str| {
        Font::from_vec(f.subset_text(text, &Location::default()).expect("a subset").font, 0).expect("the subset parses")
    };
    let f = mapping(&[('E', F)]);
    assert_eq!(alike(&f, "\u{C9}", false, false).0, glyph_ids(&[0]));
    assert_eq!(subset(&f, "\u{C9}").glyph_id('E'), None);
    let f = mapping(&[('a', F), ('\u{301}', I), ('\u{E1}', L)]);
    assert_eq!(alike(&f, "\u{E1}", false, false).0, glyph_ids(&[L]));
    assert_eq!(['a', '\u{301}'].map(|c| subset(&f, "\u{E1}").glyph_id(c)), [None, None]);
    let f = mapping(&[('a', F), ('\u{301}', I), ('\u{25CC}', L)]);
    assert_eq!(alike(&f, "\u{E1}", false, true).0, glyph_ids(&[F, I]));
    assert_eq!(subset(&f, "\u{E1}").glyph_id('\u{25CC}'), None);
}
