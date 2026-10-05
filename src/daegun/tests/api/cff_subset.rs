use daegun::{Font, OutlinePen};

use super::FONTS;

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font")).expect("it parses")
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

fn outline(f: &Font, gid: u16) -> Vec<String> {
    let mut pen = Record::default();
    f.outline_glyph(gid, &mut pen);
    pen.0
}

// Each glyph the subset keeps has the source's name and draws the source's outline.
fn alike(f: &Font, out: &daegun::SubsetResult, kept: &[u16], what: &str) {
    let sub = Font::from_vec(out.ttf.clone()).expect("the subset parses");
    for &g in kept {
        let n = out.new_gid(g).unwrap_or_else(|| panic!("{what}: glyph {g} was dropped"));
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
        let gids = f.shape(text, &[], false).expect("shapes").glyphs.to_vec();
        alike(&f, &f.subset(&gids, &[]).expect("a subset"), &gids, rel);
        alike(&f, &f.subset_text(text, &[]).expect("a compacted subset"), &gids, rel);
    }
}

// Aacute seacs after a hint, Agrave from a subroutine and Acircumflex bare: each keeps A and its accent.
#[test]
fn every_seac_form_closes_over_its_components() {
    let f = font("test-fixtures/cff-subrs.otf");
    for (accented, accent) in [(6, 3), (7, 4), (8, 5)] {
        assert_eq!(f.glyph_closure(&[accented], &[]).expect("a closure"), [0, 2, accent, accented]);
        assert!(f.glyph_bounds(accented, &[]).is_some(), "glyph {accented} draws nothing");
    }
}

// A seac finds its components by name through StandardEncoding. A CID-keyed font names no glyph, so
// its CIDs 34 and 125 are not A and acute.
#[test]
fn a_seac_in_a_cid_keyed_font_resolves_no_component() {
    let f = font("test-fixtures/cff-cid.otf");
    assert_eq!(f.glyph_closure(&[11], &[]).expect("a closure"), [0, 11]);
    assert_eq!(f.glyph_bounds(11, &[]), None);
}

#[test]
fn a_math_font_subset_to_a_space_stays_a_math_font() {
    let f = font("stix-two-math/STIX2Math.otf");
    let out = f.subset_text(" ", &[]).expect("a subset");
    let sub = Font::from_vec(out.ttf).expect("the subset parses");
    let constants = |t: &[u8]| { let at = usize::from(u16::from_be_bytes([t[4], t[5]])); t[at..at + 214].to_vec() };
    assert_eq!(sub.table("MATH").map(constants), f.table("MATH").map(constants));
}
