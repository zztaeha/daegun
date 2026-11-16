use super::*;
use crate::daecore::daekit::cache::FontCache;
use crate::daecore::daeshaper::run::text;
use alloc::vec::Vec;

fn carets() -> FontCache {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/carets.ttf");
    let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(path).expect("the fixture"));
    FontCache::new(tables.expect("its tables")).expect("a font")
}

// A glyph of GDEF class `class`, mask 1, attachment class `attach` for a mark.
fn glyph(id: u32, class: u8, attach: u8) -> Glyph {
    let mut g = Glyph::of_char(id, id);
    (g.class, g.attach_class, g.mask) = (class, attach, 1);
    g
}

fn ignorable(id: u32, bits: u8) -> Glyph {
    let mut g = glyph(id, class::BASE, 0);
    g.text = text::IGNORABLE | bits;
    g
}

// IgnoreMarks hides every mark; a mark attachment type hides marks of other classes only; a filtering set the font
// lacks is dropped, so the attachment type decides.
#[test]
fn a_lookup_hides_by_class_then_by_mark_attachment() {
    let cache = carets();
    let face = Face::new(&cache);
    let mark = |attach| glyph(1, class::MARK, attach);
    let ignore_marks = Filter::of_lookup(lookup_flags::IGNORE_MARKS, None, &face);
    assert!(ignore_marks.hides(&mark(0), &face) && !ignore_marks.hides(&glyph(2, class::BASE, 0), &face));
    let of_class_2 = Filter::of_lookup(0x0200, None, &face);
    assert_eq!([1, 2].map(|a| of_class_2.hides(&mark(a), &face)), [true, false]);
    let missing_set = Filter::of_lookup(0x0200 | lookup_flags::USE_MARK_FILTERING_SET, Some(7), &face);
    assert_eq!(missing_set, of_class_2);
}

// A default ignorable is optional unless the reading keeps its format bit; once substituted it is required.
#[test]
fn an_ignorable_is_optional_unless_its_format_bit_is_kept() {
    let cache = carets();
    let face = Face::new(&cache);
    let reading = |keeps| Reading::new(Filter::default(), 1, 0, keeps);
    let zwj = ignorable(3, text::ZWJ);
    assert_eq!(reading(0).sees(&zwj, &face), Seen::Optional);
    assert_eq!(reading(text::ZWJ).sees(&zwj, &face), Seen::Required);
    let mut substituted = zwj;
    substituted.class |= class::SUBSTITUTED;
    assert_eq!(reading(0).sees(&substituted, &face), Seen::Required);
}

// Forward, a match ends past the glyph that stops it, or past the last glyph read; back, it ends before the glyph that
// stops it, or at the start. Hidden and optional glyphs are passed on the way.
#[test]
fn a_read_ends_past_the_glyph_that_stops_it() {
    let cache = carets();
    let face = Face::new(&cache);
    let reading = Reading::new(Filter::of_classes(class::MARK), 1, 0, 0);
    let mut out_of_mask = glyph(4, class::BASE, 0);
    out_of_mask.mask = 2;
    let glyphs: Vec<Glyph> =
        [glyph(1, class::BASE, 0), glyph(2, class::MARK, 0), ignorable(3, 0), out_of_mask, glyph(5, class::BASE, 0)]
            .into();
    assert_eq!(reading.after(&glyphs, 0, &face, &|g: u32| g == 3), Ok(2));
    assert_eq!(reading.after(&glyphs, 0, &face, &|g: u32| g == 5), Err(4));
    assert_eq!(reading.after(&glyphs, 0, &face, &Present), Err(4));
    assert_eq!(reading.after(&glyphs[..3], 0, &face, &|g: u32| g == 9), Err(3));
    assert_eq!(reading.before(&glyphs, 5, &face, &|g: u32| g == 1), Err(3));
    assert_eq!(reading.before(&glyphs, 3, &face, &|g: u32| g == 1), Ok(0));
    assert_eq!(reading.before(&glyphs, 3, &face, &|g: u32| g == 9), Err(0));
    assert_eq!(reading.peek(&glyphs, 0, &face), Peek::Optional(3, 2));
}
