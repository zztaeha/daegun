use std::collections::{BTreeMap, BTreeSet};

use daegun::shape::{Direction, Feature, ShapeOptions};
use daegun::{Font, GlyphId, Location, Tag};

use super::aat_shaping::{cmap, gdef};
use super::aat_subset::{shapes_alike, u16s};

fn scheherazade() -> Font {
    let path = format!("{}/scheherazade-new/ScheherazadeNew-Regular.ttf", crate::FONTS);
    Font::from_vec(std::fs::read(&path).expect("the fixture font"), 0).expect("it parses")
}

// Scheherazade New's glyphs and metrics under a cmap of `map` and `extra`, with none of its own layout tables.
fn arabic_with(map: &[(u16, u16)], extra: &[(&str, Vec<u8>)]) -> Font {
    let source = scheherazade();
    let mut tables: BTreeMap<Tag, Vec<u8>> = source
        .table_tags()
        .filter(|t| ![b"GSUB", b"GPOS", b"GDEF", b"JSTF", b"cmap"].map(Tag::new).contains(t))
        .map(|t| (t, source.table(t).expect("listed").to_vec()))
        .collect();
    let runs: BTreeSet<(u16, u16, u16)> = map.iter().map(|&(code, glyph)| (code, code, glyph)).collect();
    tables.insert(Tag::new(b"cmap"), cmap(&runs.into_iter().collect::<Vec<_>>()));
    tables.extend(extra.iter().map(|(t, b)| (t.parse().expect("a tag"), b.clone())));
    Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the font parses")
}

// Scheherazade's Arabic letters and marks, each presentation form of U+FC00..U+FCFF, U+FD88 and U+FE70..U+FEFC on a
// glyph of its own but `missing`, and a GDEF classing the marks.
fn with_forms_but(missing: &[u16], extra: &[(&str, Vec<u8>)]) -> Font {
    let source = scheherazade();
    let own: Vec<(u16, u16)> = (0x0621..=0x0652)
        .filter_map(|c: u16| Some((c, source.glyph_id(char::from_u32(c.into())?)?.0 as u16)))
        .collect();
    let taken: BTreeSet<u16> = own.iter().map(|&(_, g)| g).collect();
    let mut free = (300..).filter(|g| !taken.contains(g));
    let forms = (0xFE70..=0xFEFC).chain(0xFC00..=0xFCFF).chain([0xFD88]).filter(|c| !missing.contains(c));
    let map: Vec<(u16, u16)> = own.iter().copied().chain(forms.map(|c| (c, free.next().expect("a glyph")))).collect();
    let marks: BTreeSet<u16> =
        (0x064B..=0x0652).filter_map(|c| Some(source.glyph_id(char::from_u32(c)?)?.0 as u16)).collect();
    let classes: Vec<(u16, u16, u16)> = marks.into_iter().map(|g| (g, g, 3)).collect();
    let mut tables = vec![("GDEF", gdef(&classes))];
    tables.extend(extra.iter().cloned());
    arabic_with(&map, &tables)
}

fn with_forms(extra: &[(&str, Vec<u8>)]) -> Font {
    with_forms_but(&[], extra)
}

// A GSUB whose `tag` feature, the default script's only one, substitutes glyph 1, which no text here reaches.
fn gsub_with(tag: &[u8; 4]) -> Vec<u8> {
    [
        u16s(&[1, 0, 10, 30, 44, 1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0, 1]), tag.to_vec(),
        u16s(&[8, 0, 1, 0, 1, 4, 1, 0, 1, 8, 1, 6, 1, 1, 1, 1]),
    ]
    .concat()
}

// Beh, teh and theh drawn alike, each with forms of its own.
fn with_shared_letters() -> Font {
    let shared = scheherazade().glyph_id('\u{628}').expect("beh").0 as u16;
    let letters = [(0x0628, [0xFE91, 0xFE92, 0xFE90, 0xFE8F]), (0x062A, [0xFE97, 0xFE98, 0xFE96, 0xFE95]),
        (0x062B, [0xFE9B, 0xFE9C, 0xFE9A, 0xFE99])];
    let mut map = Vec::new();
    for (k, (letter, forms)) in (0..).zip(letters) {
        map.push((letter, shared));
        map.extend((0..).zip(forms).map(|(j, form)| (form, 1000 + 4 * k + j)));
    }
    arabic_with(&map, &[])
}

fn shaped(f: &Font, text: &str, vertical: bool) -> (Vec<GlyphId>, Vec<u32>) {
    let o = ShapeOptions::default();
    let run = f.shape(text, &Location::default(), &if vertical { o.with_direction(Direction::TopToBottom) } else { o });
    (run.glyphs.to_vec(), run.clusters.to_vec())
}

// Without init, medi, fina and isol, Arabic takes the cmap's forms and lam-alef under rlig, across a mark; other
// ligatures are discretionary (Unicode core spec 9.2), so the font's for بم, لمح and shadda with fatha go unused.
#[test]
fn a_font_without_arabic_forms_shapes_through_its_presentation_forms() {
    let f = with_forms(&[]);
    let g = |code| f.glyph_id(char::from_u32(code).expect("a scalar")).expect("mapped");
    let cases = [
        ("بيت", vec![g(0xFE96), g(0xFEF4), g(0xFE91)], vec![4, 2, 0]),
        ("لا", vec![g(0xFEFB)], vec![0]),
        ("بلا", vec![g(0xFEFC), g(0xFE91)], vec![2, 0]),
        ("لَا", vec![g(0x064E), g(0xFEFB)], vec![0, 0]),
        ("لله", vec![g(0xFEEA), g(0xFEE0), g(0xFEDF)], vec![4, 2, 0]),
        ("بم", vec![g(0xFEE2), g(0xFE91)], vec![2, 0]),
        ("لمح", vec![g(0xFEA2), g(0xFEE4), g(0xFEDF)], vec![4, 2, 0]),
        ("\u{628}\u{651}\u{64E}", vec![g(0x064E), g(0x0651), g(0xFE8F)], vec![0, 0, 0]),
    ];
    for (text, glyphs, clusters) in cases {
        assert_eq!(shaped(&f, text, false), (glyphs, clusters), "{text}");
        shapes_alike(&f, text);
    }
}

// A lam-alef forms only when the font maps the lam's form, the alef's and the ligature: lacking the final alef with
// madda, لآ keeps its letters while لا still joins.
#[test]
fn a_lam_alef_forms_only_when_all_three_of_its_forms_are_mapped() {
    let f = with_forms_but(&[0xFE82], &[]);
    let g = |code| f.glyph_id(char::from_u32(code).expect("a scalar")).expect("mapped");
    assert_eq!(shaped(&f, "لآ", false), (vec![g(0x0622), g(0xFEDF)], vec![2, 0]));
    assert_eq!(shaped(&f, "لا", false), (vec![g(0xFEFB)], vec![0]));
    for text in ["لآ", "لا"] {
        shapes_alike(&f, text);
    }
}

// A form turned off leaves the fallback to the others, and off everywhere as off over every letter: بلا keeps its
// lam-alef, and its beh takes no initial form.
#[test]
fn a_form_turned_off_leaves_the_fallback_to_the_others() {
    let f = with_forms(&[]);
    let g = |code| f.glyph_id(char::from_u32(code).expect("a scalar")).expect("mapped");
    let with = |init: Feature| f.shape("بلا", &Location::default(), &ShapeOptions::default().with_features(&[init]));
    let off = Feature::new(Tag::new(b"init"), 0);
    for init in [off, Feature { bytes: (0..6).into(), ..off }] {
        assert_eq!(with(init).glyphs, [g(0xFEFC), g(0x0628)], "{init}");
    }
}

// Beh, teh and theh share a glyph, so the lowest code, beh, gives its forms.
#[test]
fn letters_drawn_alike_take_the_lowest_codes_forms() {
    let f = with_shared_letters();
    let g = |code| f.glyph_id(char::from_u32(code).expect("a scalar")).expect("mapped");
    assert_eq!(shaped(&f, "ببب", false), (vec![g(0xFE90), g(0xFE92), g(0xFE91)], vec![4, 2, 0]));
    shapes_alike(&f, "ببب");
}

// Vertical text takes the default shaper, and a font with init takes no fallback, though one with rlig alone does.
#[test]
fn vertical_text_and_a_font_with_arabic_forms_take_no_fallback() {
    let f = with_forms(&[]);
    let g = |code| f.glyph_id(char::from_u32(code).expect("a scalar")).expect("mapped");
    assert_eq!(shaped(&f, "بيت", true), (vec![g(0x0628), g(0x064A), g(0x062A)], vec![0, 2, 4]));

    let init = with_forms(&[("GSUB", gsub_with(b"init"))]);
    assert_eq!(shaped(&init, "بيت", false), (vec![g(0x062A), g(0x064A), g(0x0628)], vec![4, 2, 0]));
    let rlig = with_forms(&[("GSUB", gsub_with(b"rlig"))]);
    assert_eq!(shaped(&rlig, "لا", false), (vec![g(0xFEFB)], vec![0]));
    for (f, text) in [(&init, "بيت"), (&rlig, "لا")] {
        shapes_alike(f, text);
    }
}
