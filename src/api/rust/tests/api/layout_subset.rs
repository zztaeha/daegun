use std::collections::BTreeMap;

use daegun::raw::bytes::read_u16_be;
use daegun::{Font, GlyphId, Location};

use super::FONTS;
use daegun::Tag;
use daegun::shape::ShapeOptions;

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font"), 0).expect("it parses")
}

// How many of a GSUB or GPOS's lookups filter marks by a mark glyph set.
fn filtering(table: &[u8]) -> usize {
    let list = usize::from(read_u16_be(table, 8).expect("a lookup list"));
    let count = read_u16_be(table, list).expect("a lookup count");
    (0..usize::from(count))
        .filter(|i| {
            let at = list + usize::from(read_u16_be(table, list + 2 + i * 2).expect("a lookup"));
            read_u16_be(table, at + 2).expect("a lookup flag") & 0x0010 != 0
        })
        .count()
}

// `base` with Noto Devanagari's GDEF and GSUB, which filter 13 lookups by its 10 mark glyph sets, and
// those sets made unreadable when `broken`; subset to every glyph, through the glyf or CFF path.
fn subset_with_devanagari_layout(base: &str, broken: bool) -> Font {
    let devanagari = font("noto-devanagari/NotoSansDevanagari.ttf");
    let source = font(base);
    let mut tables: BTreeMap<Tag, Vec<u8>> =
        source.table_tags().map(|t| (t, source.table(t).expect("listed").to_vec())).collect();
    let mut gdef = devanagari.table(Tag::new(b"GDEF")).expect("a GDEF").to_vec();
    if broken {
        let sets = usize::from(read_u16_be(&gdef, 12).expect("MarkGlyphSets"));
        gdef[sets..sets + 2].copy_from_slice(&2u16.to_be_bytes());
    }
    tables.insert(Tag::new(b"GDEF"), gdef);
    tables.insert(Tag::new(b"GSUB"), devanagari.table(Tag::new(b"GSUB")).expect("a GSUB").to_vec());
    let patched = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    let gids: Vec<GlyphId> = (0..patched.glyph_count()).map(GlyphId).collect();
    Font::from_vec(patched.subset(&gids, &Location::default()).expect("a subset").font, 0).expect("the subset parses")
}

// MarkGlyphSets of an undefined format cost only themselves: the glyph classes stay, and GSUB's
// lookups stop filtering by sets the subset's GDEF lacks.
#[test]
fn unreadable_mark_glyph_sets_cost_only_themselves() {
    for base in ["noto-devanagari/NotoSansDevanagari.ttf", "stix-two-math/STIX2Math.otf"] {
        let out = subset_with_devanagari_layout(base, true);
        let gdef = out.table(Tag::new(b"GDEF")).unwrap_or_else(|| panic!("{base}: the subset lost its GDEF"));
        assert_ne!(read_u16_be(gdef, 4), Some(0), "{base}: the glyph classes went");
        assert_eq!(mark_glyph_set_count(gdef), 0, "{base}");
        assert_eq!(
            filtering(out.table(Tag::new(b"GSUB")).expect("a GSUB")),
            0,
            "{base}: lookups filter by sets GDEF lacks"
        );
    }
}

#[test]
fn readable_mark_glyph_sets_keep_their_lookups_filtering() {
    for base in ["noto-devanagari/NotoSansDevanagari.ttf", "stix-two-math/STIX2Math.otf"] {
        let out = subset_with_devanagari_layout(base, false);
        assert_eq!(mark_glyph_set_count(out.table(Tag::new(b"GDEF")).expect("a GDEF")), 10, "{base}");
        assert_eq!(filtering(out.table(Tag::new(b"GSUB")).expect("a GSUB")), 13, "{base}");
    }
}

// Each fixture's subset to every glyph writes its GSUB and GPOS in no more bytes than the font has them, children
// shared across the table wherever they're alike.
#[test]
fn a_whole_font_subsets_layout_is_no_larger_than_its_fonts() {
    let fonts = [
        "bungee-tint/BungeeTint-Regular.ttf", "eb-garamond/EBGaramond.ttf", "inter/InterVariable.ttf",
        "noto-devanagari/NotoSansDevanagari.ttf", "noto-khmer/NotoSansKhmer.ttf", "noto-myanmar/NotoSansMyanmar.ttf",
        "scheherazade-new/ScheherazadeNew-Regular.ttf", "structure/TestShapeLana.ttf", "stix-two-math/STIX2Math.otf",
        "source-serif/SourceSerif4Variable-Roman.otf",
    ];
    for rel in fonts {
        let f = font(rel);
        let all: Vec<GlyphId> = (0..f.glyph_count()).map(GlyphId).collect();
        let sub =
            Font::from_vec(f.subset(&all, &Location::default()).expect("a subset").font, 0).expect("the subset parses");
        for tag in [Tag::new(b"GSUB"), Tag::new(b"GPOS")] {
            let [was, now] = [&f, &sub].map(|font| font.table(tag).map_or(0, <[u8]>::len));
            assert!(now <= was, "{rel}: {tag} of {was} bytes became {now}");
        }
    }
}

// Scheherazade New's JSTF names GSUB's and GPOS's lookups by number, so its subset for a text keeps every lookup at
// its number, those no feature reaches hollow, where a font without one numbers its lookups afresh.
#[test]
fn a_fonts_jstf_holds_its_lookups_numbers() {
    let lookups = |table: &[u8]| {
        let list = usize::from(read_u16_be(table, 8).expect("a lookup list"));
        read_u16_be(table, list).expect("a lookup count")
    };
    let fonts = [("scheherazade-new/ScheherazadeNew-Regular.ttf", true), ("noto-khmer/NotoSansKhmer.ttf", false)];
    for (rel, held) in fonts {
        let f = font(rel);
        let sub = Font::from_vec(f.subset_text("abc", &Location::default()).expect("a subset").font, 0)
            .expect("the subset parses");
        assert_eq!(f.has_table(Tag::new(b"JSTF")), held, "{rel}");
        for tag in [Tag::new(b"GSUB"), Tag::new(b"GPOS")] {
            let [was, now] = [&f, &sub].map(|font| font.table(tag).map(lookups));
            assert_eq!(was == now, held, "{rel}: {tag} {was:?} lookups became {now:?}");
        }
    }
}

// Noto Devanagari subset for text whose glyphs fall in 5 of its 10 mark glyph sets keeps those 5, renumbered, and
// its lookups filtering by them shape the text as the font does.
#[test]
fn a_subset_keeps_the_mark_glyph_sets_its_glyphs_fall_in() {
    let f = font("noto-devanagari/NotoSansDevanagari.ttf");
    let text = "र्कं किं कुँ";
    let (sub, _) = super::aat_subset::shapes_alike(&f, text);
    assert_eq!(
        sub.shape(text, &Location::default(), &ShapeOptions::default()).offsets,
        f.shape(text, &Location::default(), &ShapeOptions::default()).offsets
    );
    assert_eq!(mark_glyph_set_count(sub.table(Tag::new(b"GDEF")).expect("a GDEF")), 5);
    assert_eq!([b"GSUB", b"GPOS"].map(|t| filtering(sub.table(Tag::new(t)).expect("the table"))), [10, 1]);
}

// GDEF classes only f_i, so a lone U+0301 on .notdef is no mark and keeps its advance; a subset keeping none of the
// classes still has them, empty, or its marks would come from Unicode.
#[test]
fn a_subset_keeps_its_fonts_glyph_classes_when_none_of_them_are_kept() {
    use super::aat_shaping::gdef;
    use super::aat_subset::{carets_with, shapes_alike, F_I};
    let f = carets_with(&[("GDEF", gdef(&[(F_I, F_I, 2)]))]);
    assert_eq!(f.shape("\u{301}i", &Location::default(), &ShapeOptions::default()).advances, [400.0, 400.0]);
    let (sub, _) = shapes_alike(&f, "\u{301}i");
    assert!(sub.has_table(Tag::new(b"GDEF")), "the subset has no GDEF");
}

// How many mark glyph sets a GDEF defines: none before version 1.2, or where it names no definition of format 1.
fn mark_glyph_set_count(gdef: &[u8]) -> u16 {
    let sets = usize::from(read_u16_be(gdef, 12).filter(|_| read_u16_be(gdef, 2) >= Some(2)).unwrap_or(0));
    match (sets, read_u16_be(gdef, sets), read_u16_be(gdef, sets + 2)) {
        (1.., Some(1), Some(count)) => count,
        _ => 0,
    }
}
