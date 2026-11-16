use daegun::{Font, Location};
use daegun::shape::ShapeOptions;
use daegun::unicode::Script;

use super::aat_subset::{base_with, carets_with};
use super::cmap_reading::{cmap, format4};
use super::FONTS;

fn shaped(f: &Font, text: &str, opts: &ShapeOptions) -> (Vec<u32>, Vec<u32>, Vec<bool>) {
    let run = f.shape(text, &Location::default(), opts);
    (run.glyphs.iter().map(|g| g.0).collect(), run.clusters.to_vec(), run.unsafe_to_break.to_vec())
}

fn noto_devanagari() -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/noto-devanagari/NotoSansDevanagari.ttf")).expect("the font"), 0)
        .expect("it parses")
}

// colr-clip.ttf's glyphs with no GSUB, so each script goes to its own shaper, and some Gujarati and Devanagari
// letters and U+25CC mapped.
fn letters() -> Font {
    let map = [
        (0x905, 5), (0x907, 10), (0x930, 8), (0x93E, 6), (0x94D, 9), (0xA85, 1), (0xABE, 3), (0xAC5, 2), (0x25CC, 4),
    ];
    base_with("test-fixtures/colr-clip.ttf", &[("cmap", cmap(&[(3, 1, &format4(&map))]))])
}

// An i matra with no base takes a dotted circle of the control category, so the KA and i matra after it start a word
// and are safe to break there.
#[test]
fn an_inserted_dotted_circle_ends_a_word() {
    let run = shaped(&noto_devanagari(), "\u{93F}\u{915}\u{93F}", &ShapeOptions::default());
    assert_eq!(run, (vec![32, 789, 545, 56], vec![0, 0, 3, 3], vec![false; 4]));
}

// Each broken syllable takes a circle, the 15th syllable after another too, whose serial repeats the first's.
#[test]
fn a_broken_syllable_takes_a_circle_wherever_it_falls() {
    let f = noto_devanagari();
    let circle = f.glyph_id('\u{25CC}').expect("the font maps U+25CC").0;
    for spaces in 10..=18 {
        let (glyphs, ..) = shaped(&f, &format!("\u{93F}{}\u{93F}", " ".repeat(spaces)), &ShapeOptions::default());
        assert_eq!(glyphs.iter().filter(|&&g| g == circle).count(), 2, "{spaces} spaces between");
    }
}

// Gujarati A, candra e and aa take one circle inside the first pair, as the pass takes it whole, and RA, virama and I
// one before I.
#[test]
fn a_vowel_sequence_takes_one_dotted_circle() {
    let run = shaped(&letters(), "\u{A85}\u{AC5}\u{ABE}", &ShapeOptions::default());
    assert_eq!(run, (vec![1, 4, 2, 3], vec![0; 4], vec![false; 4]));
    let run = shaped(&letters(), "\u{930}\u{94D}\u{907}", &ShapeOptions::default());
    assert_eq!(run, (vec![8, 9, 4, 10], vec![0, 0, 6, 6], vec![false, false, true, true]));
}

// A font without U+25CC still gets the circle, as the character it is, which the font draws as .notdef.
#[test]
fn a_vowel_sequence_takes_a_circle_the_font_lacks() {
    let run = shaped(&carets_with(&[]), "\u{905}\u{93E}", &ShapeOptions::default());
    assert_eq!(run, (vec![0; 3], vec![0; 3], vec![false; 3]));
}

// Devanagari A and aa in a run the caller set to Bengali: only Bengali's sequences take a circle there.
#[test]
fn a_vowel_sequence_counts_only_in_its_own_script() {
    const BENGALI: ShapeOptions = ShapeOptions::DEFAULT.with_seed_script(Script::BENGALI);
    let run = shaped(&letters(), "\u{905}\u{93E}", &BENGALI);
    assert_eq!(run, (vec![5, 6], vec![0, 0], vec![false; 2]));
}
