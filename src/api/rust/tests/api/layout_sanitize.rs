use std::collections::BTreeMap;

use daegun::{Font, GlyphId, Location, Orientation};

use super::aat_subset::{carets_with, shapes_alike, u16s, F, F_I, I};
use daegun::Tag;
use crate::located;
use daegun::shape::ShapeOptions;

// A GSUB or GPOS whose `tag`, the default script's one feature, runs these (type, subtable) lookups in order.
fn layout(tag: &[u8; 4], lookups: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let scripts = [u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0])].concat();
    let indices: Vec<u16> = (0..lookups.len() as u16).collect();
    let features = [u16s(&[1]), tag.to_vec(), u16s(&[8, 0, lookups.len() as u16]), u16s(&indices)].concat();
    let mut list = u16s(&[lookups.len() as u16]);
    let mut bodies = Vec::new();
    for (kind, subtable) in lookups {
        list.extend(u16s(&[(2 + 2 * lookups.len() + bodies.len()) as u16]));
        bodies.extend([u16s(&[*kind, 0, 1, 8]), subtable.clone()].concat());
    }
    let (at_scripts, at_features) = (10, 10 + scripts.len() as u16);
    let at_lookups = at_features + features.len() as u16;
    [u16s(&[1, 0, at_scripts, at_features, at_lookups]), scripts, features, list, bodies].concat()
}

// i a mark and f_i one caret, as a format 3 caret of coordinate 300 whose device lies at `device`.
fn gdef(device: u16) -> Vec<u8> {
    let carets = u16s(&[6, 1, 12, 1, 1, F_I, 1, 4, 3, 300, device]);
    [u16s(&[1, 0, 12, 0, 22, 0]), u16s(&[2, 1, I, I, 3]), carets].concat()
}

fn f_to_i() -> (u16, Vec<u8>) {
    (1, u16s(&[2, 8, 1, I, 1, 1, F]))
}

// Last in its table, a SingleSubst whose substitutes run past it, and a SinglePos whose values do.
fn substitutes_past_the_table() -> (u16, Vec<u8>) {
    (1, u16s(&[2, 6, 9, 1, 1, F]))
}

fn values_past_the_table() -> (u16, Vec<u8>) {
    (1, u16s(&[2, 8, 4, 9, 1, 1, F]))
}

// A SingleSubst whose substitutes' count leaves the table has none, and the rest of the GSUB is read: f still turns
// into i by the lookup before it, and the font lists its script and feature.
#[test]
fn a_gsub_s_subtable_past_the_table_leaves_the_rest_read() {
    let read = carets_with(&[("GSUB", layout(b"ccmp", &[f_to_i()]))]);
    assert_eq!(read.shape("f", &Location::default(), &ShapeOptions::default()).glyphs, [I].map(|g| GlyphId(g.into())));
    assert_eq!(
        read.glyph_closure([GlyphId(F.into())], &Location::default()).expect("a closure"),
        [0, F, I].map(|g| GlyphId(g.into()))
    );
    assert_eq!(read.script_tags(), [Tag::new(b"DFLT")]);
    assert_eq!(read.feature_tags(None, None), [Tag::new(b"ccmp")]);
    let past = carets_with(&[("GSUB", layout(b"ccmp", &[f_to_i(), substitutes_past_the_table()]))]);
    assert_eq!(past.shape("f", &Location::default(), &ShapeOptions::default()).glyphs, [I].map(|g| GlyphId(g.into())));
    assert_eq!(
        past.glyph_closure([GlyphId(F.into())], &Location::default()).expect("a closure"),
        [0, F, I].map(|g| GlyphId(g.into()))
    );
    assert_eq!((past.script_tags(), past.feature_tags(None, None)), (vec![Tag::new(b"DFLT")], vec![Tag::new(b"ccmp")]));
    let (sub, _) = shapes_alike(&past, "fi");
    assert!(sub.table(Tag::new(b"GSUB")).is_some());
}

// fi kerned by -40, beside a SinglePos whose values' count leaves the table: those values are absent, and the pair
// still kerns.
#[test]
fn a_gpos_s_subtable_past_the_table_leaves_the_rest_read() {
    let pair = (2, u16s(&[1, 12, 4, 0, 1, 18, 1, 1, F, 1, I, (-40i16) as u16]));
    let read = carets_with(&[("GPOS", layout(b"kern", std::slice::from_ref(&pair)))]);
    assert_eq!(read.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [360.0, 400.0]);
    let past = carets_with(&[("GPOS", layout(b"kern", &[pair, values_past_the_table()]))]);
    assert_eq!(past.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [360.0, 400.0]);
    let (sub, _) = shapes_alike(&past, "fi");
    assert!(sub.table(Tag::new(b"GPOS")).is_some());
}

// i a mark, zeroed late, and f_i a caret at 300 whose device lies past the table: the device is absent, the caret and
// the rest of the GDEF read.
#[test]
fn a_gdef_s_device_past_the_table_leaves_the_rest_read() {
    let read = carets_with(&[("GDEF", gdef(0))]);
    assert_eq!(read.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [400.0, 0.0]);
    assert_eq!(read.glyph_class(GlyphId(I.into())), Some(daegun::GlyphClass::Mark));
    assert_eq!(read.ligature_carets(GlyphId(F_I.into()), &Location::default(), Orientation::Horizontal), [Some(300.0)]);
    let past = carets_with(&[("GDEF", gdef(0x100))]);
    assert_eq!(past.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [400.0, 0.0]);
    assert_eq!(past.glyph_class(GlyphId(I.into())), Some(daegun::GlyphClass::Mark));
    let caret = past.ligature_carets(GlyphId(F_I.into()), &Location::default(), Orientation::Horizontal);
    assert_eq!(caret, [Some(300.0)]);
    let (sub, _) = shapes_alike(&past, "fi");
    assert!(sub.table(Tag::new(b"GDEF")).is_some());
}

// Inter with each of GSUB, GPOS and GDEF swapped for one with a part past its end: the font reads each at every
// location, the part absent, so its instance holds all three.
#[test]
fn an_instance_keeps_a_layout_table_with_a_part_past_its_end() {
    let inter = std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("fixture");
    let source = Font::from_vec(inter, 0).expect("Inter parses");
    let mut tables: BTreeMap<Tag, Vec<u8>> =
        source.table_tags().map(|t| (t, source.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"GSUB"), layout(b"ccmp", &[substitutes_past_the_table()]));
    tables.insert(Tag::new(b"GPOS"), layout(b"kern", &[values_past_the_table()]));
    tables.insert(Tag::new(b"GDEF"), gdef(0x100));
    let font = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    let instance =
        Font::from_bytes(&font.instance(&located(&font, &[("wght", 700.0)])).expect("an instance")).expect("it parses");
    for tag in [b"GSUB", b"GPOS", b"GDEF"].map(Tag::new) {
        assert!(instance.table(tag).is_some(), "the instance has no {tag}");
    }
}

// i, a mark, attached to f by GPOS's mark feature: i's anchor at the origin and f's `base_anchor`, after which lies
// `tail`, where a format 3 anchor's devices sit 10 bytes on.
fn mark_on_f(base_anchor: &[u16], tail: &[u8]) -> Font {
    let head = [1, 12, 18, 1, 24, 36, 1, 1, I, 1, 1, F, 1, 0, 6, 1, 0, 0, 1, 4];
    let subtable = [u16s(&head), u16s(base_anchor), tail.to_vec()].concat();
    carets_with(&[("GDEF", super::aat_shaping::gdef(&[(I, I, 3)])), ("GPOS", layout(b"mark", &[(4, subtable)]))])
}

// A device of format 3 for sizes 1 to 2,001: 2,008 bytes.
fn wide_device() -> Vec<u8> {
    [u16s(&[1, 2_001, 3]), vec![0; 2_002]].concat()
}

fn offsets(f: &Font, text: &str) -> Vec<(f64, f64)> {
    crate::xy(&f.shape(text, &Location::default(), &ShapeOptions::default()).offsets)
}

// f's anchor is read where i attaches: one whose device lies past the table places by its coordinates, the device
// absent, and one of an unknown format is skipped, so i stays unattached.
#[test]
fn an_anchor_is_checked_where_a_mark_attaches() {
    assert_eq!(offsets(&mark_on_f(&[1, 300, 500], &[]), "fi"), [(0.0, 0.0), (-100.0, 500.0)]);
    assert_eq!(offsets(&mark_on_f(&[3, 300, 500, 0x100, 0], &[]), "fi"), [(0.0, 0.0), (-100.0, 500.0)]);
    assert_eq!(offsets(&mark_on_f(&[4, 300, 500], &[]), "fi"), [(0.0, 0.0), (0.0, 0.0)]);
}

// Each mark reads f's anchor again, both its devices the one wide device; a check of an anchor's own fields spends
// nothing, so all 40 marks attach however many read it.
#[test]
fn every_mark_attaches_however_often_its_anchor_is_read() {
    let f = mark_on_f(&[3, 300, 500, 10, 10], &wide_device());
    let text: String = core::iter::once('f').chain(core::iter::repeat_n('i', 40)).collect();
    assert!(offsets(&f, &text)[1..].iter().all(|&(_, y)| y == 500.0));
}

// f's exit joins i's entry, whose device lies past the table: the device is absent and the two join by the
// entry's coordinates.
#[test]
fn a_cursive_anchor_is_checked_where_it_joins() {
    let cursive = |entry: &[u16]| {
        let head = [1, 14, 2, 0, 22, 28, 0, 1, 2, F, I, 1, 400, 0];
        let gpos = layout(b"curs", &[(3, [u16s(&head), u16s(entry)].concat())]);
        carets_with(&[("GPOS", gpos)])
    };
    assert_eq!(offsets(&cursive(&[1, 0, 100]), "fi"), [(0.0, 0.0), (0.0, -100.0)]);
    assert_eq!(offsets(&cursive(&[3, 0, 100, 0x100, 0]), "fi"), [(0.0, 0.0), (0.0, -100.0)]);
}

// A table of another major version fails the sanitizer and isn't read: without the GPOS the fallback places the mark
// i, and each subset leaves the table out.
#[test]
fn a_layout_table_of_another_major_is_not_read() {
    let major_2 = |mut table: Vec<u8>| {
        table[..2].copy_from_slice(&2u16.to_be_bytes());
        table
    };
    let gsub = carets_with(&[("GSUB", major_2(layout(b"ccmp", &[f_to_i()])))]);
    assert_eq!(gsub.shape("f", &Location::default(), &ShapeOptions::default()).glyphs, [F].map(|g| GlyphId(g.into())));
    let classes = carets_with(&[("GDEF", major_2(gdef(0)))]);
    assert_eq!(classes.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [400.0, 400.0]);
    assert_eq!(classes.glyph_class(GlyphId(I.into())), None);
    let pair = (2, u16s(&[1, 12, 4, 0, 1, 18, 1, 1, F, 1, I, (-40i16) as u16]));
    let gpos = carets_with(&[("GDEF", gdef(0)), ("GPOS", major_2(layout(b"kern", &[pair])))]);
    assert_eq!(gpos.shape("fi", &Location::default(), &ShapeOptions::default()).advances, [400.0, 0.0]);
    let shaped = gpos.shape("fi", &Location::default(), &ShapeOptions::default());
    assert_eq!(crate::xy(&shaped.offsets), [(0.0, 0.0), (-400.0, 0.0)]);
    for (f, tag) in [(&gsub, b"GSUB"), (&classes, b"GDEF"), (&gpos, b"GPOS")].map(|(f, t)| (f, Tag::new(t))) {
        let (sub, _) = shapes_alike(f, "fi");
        assert!(sub.table(tag).is_none(), "the subset's {tag}");
    }
}

// A GPOS with a null LookupList, as Noto Sans Palmyrene's, still turns the fallback off, and its subset writes one
// empty that places as the font does.
#[test]
fn a_null_lookup_list_subsets_as_none() {
    let gpos = [u16s(&[1, 0, 10, 28, 0, 1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 0, 0])].concat();
    let font = carets_with(&[("GPOS", gpos)]);
    let (sub, _) = shapes_alike(&font, "f\u{301}");
    assert_eq!(offsets(&font, "f\u{301}"), [(0.0, 0.0), (0.0, 0.0)]);
    assert_eq!(offsets(&sub, "f\u{301}"), offsets(&font, "f\u{301}"));
}
