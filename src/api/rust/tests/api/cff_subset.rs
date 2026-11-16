use std::collections::BTreeMap;

use daegun::{Font, GlyphId, Location};
use daegun::outline::OutlinePen;

use super::FONTS;
use daegun::Tag;
use crate::located;
use daegun::shape::ShapeOptions;

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font"), 0).expect("it parses")
}

// A CFF or CFF2 subset of glyphs keeps their ids, and reads each glyph outside the closure as dropped.
#[test]
fn a_subset_in_place_drops_what_its_closure_leaves() {
    for face in ["stix-two-math/STIX2Math.otf", "source-serif/SourceSerif4Variable-Roman.otf"] {
        let f = font(face);
        let gids: Vec<GlyphId> = "fifl AVTo".chars().filter_map(|c| f.glyph_id(c)).collect();
        let closure = f.glyph_closure(&gids, &Location::default()).expect("the closure converges");
        let subset = f.subset(&gids, &Location::default()).expect("the subset builds");
        let kept: Vec<GlyphId> =
            (0..f.glyph_count()).map(GlyphId).filter(|&g| subset.renumbered(g).is_some()).collect();
        assert_eq!(kept, closure, "{face}: the subset and its closure disagree about what survives");
        assert!(closure.iter().all(|&g| subset.renumbered(g) == Some(g)), "{face}: a kept glyph moved");
        let built = Font::from_bytes(&subset.font).expect("the subset parses");
        assert_eq!(built.glyph_count(), f.glyph_count(), "{face}: the subset renumbered its glyphs");
    }
}

#[derive(Default)]
struct Record(Vec<String>);

impl OutlinePen for Record {
    fn move_to(&mut self, x: f32, y: f32) { self.0.push(format!("M{x},{y}")); }
    fn line_to(&mut self, x: f32, y: f32) { self.0.push(format!("L{x},{y}")); }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("Q{x},{y}")); }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("C{x},{y}")); }
    fn close(&mut self) { self.0.push("Z".into()); }
}

fn outline(f: &Font, gid: GlyphId) -> Vec<String> {
    let mut pen = Record::default();
    let _ = f.outline(gid, &Location::default(), &mut pen);
    pen.0
}

// Each glyph the subset keeps has the source's name and draws the source's outline.
fn alike(f: &Font, out: &daegun::tables::Subset, kept: &[GlyphId], what: &str) {
    let sub = Font::from_vec(out.font.clone(), 0).expect("the subset parses");
    for &g in kept {
        let n = out.renumbered(g).unwrap_or_else(|| panic!("{what}: glyph {g} was dropped"));
        assert_eq!(sub.glyph_name(n), f.glyph_name(g), "{what}: glyph {g} was renamed");
        assert_eq!(outline(&sub, n), outline(f, g), "{what}: glyph {g} draws otherwise");
    }
}

// The fixtures' cmaps map each glyph a letter: the compacting subset is reached through text.
#[test]
fn each_cff_layout_subsets_both_ways_with_its_names_and_outlines() {
    for (rel, text) in [
        ("test-fixtures/cff-subrs.otf", "ÁÀÂabcdefghijkl"),
        ("test-fixtures/cff-cid.otf", "ABCDEFGHIJK"),
        ("test-fixtures/cff-expert.otf", " !\"$"),
    ] {
        let f = font(rel);
        let gids = f.shape(text, &Location::default(), &ShapeOptions::default()).glyphs.to_vec();
        alike(&f, &f.subset(&gids, &Location::default()).expect("a subset"), &gids, rel);
        alike(&f, &f.subset_text(text, &Location::default()).expect("a compacted subset"), &gids, rel);
    }
}

// Aacute seacs after a hint, Agrave from a subroutine and Acircumflex bare: each keeps A and its accent.
#[test]
fn every_seac_form_closes_over_its_components() {
    let f = font("test-fixtures/cff-subrs.otf");
    for (accented, accent) in [(6, 3), (7, 4), (8, 5)] {
        assert_eq!(
            f.glyph_closure([GlyphId(accented)], &Location::default()).expect("a closure"),
            [0, 2, accent, accented].map(GlyphId)
        );
        assert!(f.glyph_bounds(GlyphId(accented), &Location::default()).is_some(), "glyph {accented} draws nothing");
    }
}

// A seac names its base and accent by StandardEncoding SIDs, which a CID-keyed font's charset doesn't hold, so
// cff-cid's glyph 11, a seac in FD 1's subroutines, fails: no outline, no box, no components in its closure.
#[test]
fn a_seac_in_a_cid_keyed_font_fails_its_glyph() {
    let f = font("test-fixtures/cff-cid.otf");
    let def = Location::default();
    assert_eq!(f.glyph_closure([GlyphId(11)], &def).expect("a closure"), [0, 11].map(GlyphId));
    assert!(f.outline(GlyphId(11), &def, &mut Record::default()).is_err());
    assert_eq!(f.glyph_bounds(GlyphId(11), &def), None);
    alike(&f, &f.subset([GlyphId(11)], &def).expect("a subset"), &[GlyphId(11)], "cff-cid");
}

#[test]
fn a_math_font_subset_to_a_space_stays_a_math_font() {
    let f = font("stix-two-math/STIX2Math.otf");
    let out = f.subset_text(" ", &Location::default()).expect("a subset");
    let sub = Font::from_vec(out.font, 0).expect("the subset parses");
    let constants = |t: &[u8]| { let at = usize::from(u16::from_be_bytes([t[4], t[5]])); t[at..at + 214].to_vec() };
    assert_eq!(sub.table(Tag::new(b"MATH")).map(constants), f.table(Tag::new(b"MATH")).map(constants));
}

// An FDSelect naming an FD past its FDArray leaves that range's glyphs in FD 0 and refuses nothing else: the font
// subsets, its glyphs drawing as before, and a CFF2 instances.
#[test]
fn a_font_whose_fd_select_names_an_fd_past_its_fd_array_subsets_and_instances() {
    for (rel, tag, last_fd) in
        [("test-fixtures/cff-cid.otf", "CFF ", 177), ("source-serif/SourceSerif4Variable-Roman.otf", "CFF2", 877)]
    {
        let source = font(rel);
        let table = |t: Tag| (t, source.table(t).expect("listed").to_vec());
        let mut tables: BTreeMap<Tag, Vec<u8>> = source.table_tags().map(table).collect();
        tables.get_mut(&tag.parse::<Tag>().expect("a tag")).expect("the table")[last_fd] = 200;
        let f = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
        let def = Location::default();
        let a = f.glyph_id('A').expect("an A");
        let out = f.subset_text("A", &def).expect("a subset");
        let sub = Font::from_vec(out.font.clone(), 0).expect("the subset parses");
        assert_eq!(outline(&sub, out.renumbered(a).expect("A kept")), outline(&source, a), "{rel}");
        assert_eq!(outline(&f, a), outline(&source, a), "{rel}");
        if tag == "CFF2" {
            assert!(f.instance(&located(&f, &[("wght", 600.0)])).is_ok());
        }
    }
}
