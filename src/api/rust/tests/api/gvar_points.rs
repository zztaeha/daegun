use daegun::{Bounds, Font, GlyphId, Location, Tag};
use crate::located;

// test-fixtures/gvar-points.ttf: every glyph the box 100..500 with advance 600, and one gvar case
// each at wght 900 (see its builder, scripts/tools/fixtures/build_gvar_fixture.py).
fn fixture() -> Vec<u8> {
    std::fs::read(format!("{}/test-fixtures/gvar-points.ttf", crate::FONTS)).expect("fixture")
}

fn across(font: &Font, ch: char) -> (f64, f64) {
    let gid = font.glyph_id(ch).expect("mapped");
    let b = font.glyph_bounds(gid, &located(font, &[("wght", 900.0)])).expect("drawn");
    (b.x_min, b.x_max)
}

fn u16_at(t: &[u8], at: usize) -> usize {
    usize::from(u16::from_be_bytes([t[at], t[at + 1]]))
}

// Where glyph `gid`'s first tuple header sits in the font's gvar, which uses long offsets.
fn first_tuple(font: &[u8], gid: usize) -> usize {
    let n = u16_at(font, 4);
    let entry = (0..n).map(|i| 12 + 16 * i).find(|&e| &font[e..e + 4] == b"gvar").unwrap();
    let gvar = u32::from_be_bytes(font[entry + 8..entry + 12].try_into().unwrap()) as usize;
    let data = gvar + u32::from_be_bytes(font[gvar + 16..gvar + 20].try_into().unwrap()) as usize;
    data + u32::from_be_bytes(font[gvar + 20 + 4 * gid..gvar + 24 + 4 * gid].try_into().unwrap()) as usize + 4
}

#[test]
fn an_advance_moves_by_both_horizontal_phantom_points() {
    let font = Font::from_bytes(&fixture()).unwrap();
    let (a, e) = (font.glyph_id('A').unwrap(), font.glyph_id('E').unwrap());
    assert_eq!(crate::advances(&font, &[a, e], &located(&font, &[("wght", 900.0)])), [660.0, 630.0]);
}

#[test]
fn an_instanced_glyph_is_drawn_from_its_phantom_origin() {
    let font = Font::from_bytes(&fixture()).unwrap();
    assert_eq!(across(&font, 'A'), (110.0, 510.0), "A is drawn from phantom point 1, 40 units on");
    let instance = Font::from_bytes(&font.instance(&located(&font, &[("wght", 900.0)])).expect("instanced")).unwrap();
    let e = instance.glyph_id('E').unwrap();
    assert_eq!(
        instance.glyph_bounds(e, &Location::default()).map(|b| (b.x_min, b.x_max)),
        Some((120.0, 520.0)),
        "the composite's own phantom moves it"
    );
}

#[test]
fn point_numbers_are_read_as_the_spec_counts_them() {
    let font = Font::from_bytes(&fixture()).unwrap();
    assert_eq!(across(&font, 'B'), (130.0, 530.0), "a repeated point takes both its deltas");
    assert_eq!(across(&font, 'C'), (110.0, 530.0), "a point past the glyph drops only itself");
    assert_eq!(across(&font, 'D'), (108.0, 550.0), "nine point numbers on four points are all read");
}

// B names a shared tuple that doesn't exist, which fails its variation data, then absent: B draws as stored at
// wght 900 too, and A still varies.
#[test]
fn one_glyphs_bad_variation_leaves_the_rest_varied() {
    let mut bytes = fixture();
    let b = first_tuple(&bytes, 2);
    bytes[b + 2..b + 4].copy_from_slice(&0x0FFFu16.to_be_bytes());
    let font = Font::from_vec(bytes, 0).unwrap();
    let gid = font.glyph_id('B').expect("mapped");
    let stored = Some(Bounds { x_min: 100.0, y_min: 0.0, x_max: 500.0, y_max: 700.0 });
    assert_eq!(font.glyph_bounds(gid, &located(&font, &[("wght", 900.0)])), stored, "B at wght 900");
    assert_eq!(font.glyph_bounds(gid, &Location::default()), stored, "B at the default");
    assert_eq!(across(&font, 'A'), (110.0, 510.0));
}

#[test]
fn a_gvar_for_another_axis_count_varies_no_outline() {
    let mut bytes = fixture();
    let n = u16_at(&bytes, 4);
    let entry = (0..n).map(|i| 12 + 16 * i).find(|&e| &bytes[e..e + 4] == b"gvar").unwrap();
    let gvar = u32::from_be_bytes(bytes[entry + 8..entry + 12].try_into().unwrap()) as usize;
    bytes[gvar + 4..gvar + 6].copy_from_slice(&2u16.to_be_bytes());
    let font = Font::from_vec(bytes, 0).unwrap();
    assert_eq!(across(&font, 'A'), (100.0, 500.0));
    let a = font.glyph_id('A').unwrap();
    assert_eq!(crate::advances(&font, &[a], &located(&font, &[("wght", 900.0)])), [600.0]);
}

// Inter declares lsb = xMin (head flags bit 1); its instance keeps that for composites too.
#[test]
fn an_instances_composites_keep_lsb_at_xmin() {
    let inter = Font::from_bytes(&std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).unwrap()).unwrap();
    let instance = Font::from_bytes(&inter.instance(&located(&inter, &[("wght", 900.0)])).expect("instanced")).unwrap();
    let table = |tag| instance.table(Tag::new(tag)).expect("the instance has it");
    let (glyf, hmtx) = (table(b"glyf"), table(b"hmtx"));
    let long = u16_at(table(b"hhea"), 34);
    let loca = daegun::raw::parse_loca(table(b"loca"), 1, u16_at(table(b"maxp"), 4));
    let mut composites = 0;
    for gid in 0..loca.len() - 1 {
        let at = loca[gid];
        if loca[gid + 1] <= at || glyf[at..at + 2] != [0xFF, 0xFF] { continue; }
        composites += 1;
        let x_min = i16::from_be_bytes([glyf[at + 2], glyf[at + 3]]);
        let lsb_at = if gid < long { gid * 4 + 2 } else { long * 4 + (gid - long) * 2 };
        assert_eq!(i16::from_be_bytes([hmtx[lsb_at], hmtx[lsb_at + 1]]), x_min, "glyph {gid}");
    }
    assert!(composites > 1000);
}

// Source Han Sans JP's VVAR moves perthousand's vertical origin by 10 at full weight.
#[test]
fn the_vertical_origin_varies() {
    let shs =
        Font::from_bytes(&std::fs::read(format!("{}/source-han-sans/SourceHanSansJP-VF.otf", crate::FONTS)).unwrap())
            .unwrap();
    assert_eq!(shs.vertical_origin(GlyphId(480), &Location::default()), 863.0);
    assert_eq!(shs.vertical_origin(GlyphId(480), &located(&shs, &[("wght", 900.0)])), 873.0);
}

// A glyph's first draw at a location, its points decoded once for its origin and its outline, draws as its next, the
// origin kept and the points decoded again: each of Inter's first 300 glyphs, composites among them.
#[test]
fn a_first_draw_at_a_location_draws_as_the_next() {
    let font = Font::from_vec(std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("fixture"), 0);
    let font = font.expect("parses");
    let at = located(&font, &[("wght", 640.0)]);
    for gid in (0..300).map(GlyphId) {
        let (mut first, mut next) = (daegun::outline::Path::default(), daegun::outline::Path::default());
        let drawn = font.outline(gid, &at, &mut first).is_ok();
        assert_eq!(font.outline(gid, &at, &mut next).is_ok(), drawn, "gid {gid}");
        assert_eq!(first, next, "gid {gid}");
    }
}

// Points kept at a glyph's first draw at one location draw it at the next as a font that never kept them does: each
// of Inter's first 300 glyphs at wght 640, then at 320 against a fresh font there.
#[test]
fn points_kept_at_one_location_draw_another_as_decoded() {
    let read = || std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("fixture");
    let (kept, fresh) = (Font::from_vec(read(), 0).expect("parses"), Font::from_vec(read(), 0).expect("parses"));
    let (first, at, fresh_at) =
        (located(&kept, &[("wght", 640.0)]), located(&kept, &[("wght", 320.0)]), located(&fresh, &[("wght", 320.0)]));
    for gid in (0..300).map(GlyphId) {
        let _ = kept.outline(gid, &first, &mut daegun::outline::Path::default());
        let (mut copied, mut decoded) = (daegun::outline::Path::default(), daegun::outline::Path::default());
        let drawn = kept.outline(gid, &at, &mut copied).is_ok();
        assert_eq!(fresh.outline(gid, &fresh_at, &mut decoded).is_ok(), drawn, "gid {gid}");
        assert_eq!(copied, decoded, "gid {gid}");
    }
}
