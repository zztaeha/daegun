use daegun::Font;

// test-fixtures/gvar-points.ttf: every glyph the box 100..500 with advance 600, and one gvar case
// each at wght 900 (see its builder, scripts/tools/oracles/diff/build_gvar_fixture.py).
fn fixture() -> Vec<u8> {
    std::fs::read(format!("{}/test-fixtures/gvar-points.ttf", crate::FONTS)).expect("fixture")
}

fn across(font: &Font, ch: char) -> (f64, f64) {
    let gid = font.glyph_id(ch as u32).expect("mapped");
    let (x0, _, x1, _) = font.glyph_bounds(gid, &[("wght", 900.0)]).expect("drawn");
    (x0, x1)
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
    let (a, e) = (font.glyph_id('A' as u32).unwrap(), font.glyph_id('E' as u32).unwrap());
    assert_eq!(font.advance_widths(&[a, e], &[("wght", 900.0)]), [660.0, 630.0]);
}

#[test]
fn an_instanced_glyph_is_drawn_from_its_phantom_origin() {
    let font = Font::from_bytes(&fixture()).unwrap();
    assert_eq!(across(&font, 'A'), (110.0, 510.0), "FreeType draws A from phantom point 1, 40 units on");
    let instance = Font::from_vec(font.instance(&[("wght", 900.0)])).unwrap();
    let e = instance.glyph_id('E' as u32).unwrap();
    assert_eq!(instance.glyph_bounds(e, &[]).map(|b| (b.0, b.2)), Some((120.0, 520.0)), "the composite's own phantom moves it");
}

#[test]
fn point_numbers_are_read_as_the_spec_counts_them() {
    let font = Font::from_bytes(&fixture()).unwrap();
    assert_eq!(across(&font, 'B'), (130.0, 530.0), "a repeated point takes both its deltas");
    assert_eq!(across(&font, 'C'), (110.0, 530.0), "a point past the glyph drops only itself");
    assert_eq!(across(&font, 'D'), (108.0, 550.0), "nine point numbers on four points are all read");
}

#[test]
fn one_glyphs_bad_variation_leaves_the_rest_varied() {
    let mut bytes = fixture();
    let b = first_tuple(&bytes, 2);
    bytes[b + 2..b + 4].copy_from_slice(&0x0FFFu16.to_be_bytes());
    let font = Font::from_vec(bytes).unwrap();
    assert_eq!(across(&font, 'B'), (100.0, 500.0), "B names a shared tuple that does not exist");
    assert_eq!(across(&font, 'A'), (110.0, 510.0));
}

#[test]
fn a_gvar_for_another_axis_count_varies_no_outline() {
    let mut bytes = fixture();
    let n = u16_at(&bytes, 4);
    let entry = (0..n).map(|i| 12 + 16 * i).find(|&e| &bytes[e..e + 4] == b"gvar").unwrap();
    let gvar = u32::from_be_bytes(bytes[entry + 8..entry + 12].try_into().unwrap()) as usize;
    bytes[gvar + 4..gvar + 6].copy_from_slice(&2u16.to_be_bytes());
    let font = Font::from_vec(bytes).unwrap();
    assert_eq!(across(&font, 'A'), (100.0, 500.0));
    let a = font.glyph_id('A' as u32).unwrap();
    assert_eq!(font.advance_widths(&[a], &[("wght", 900.0)]), [600.0]);
}

// Inter declares lsb = xMin (head flags bit 1); its instance keeps that for composites too.
#[test]
fn an_instances_composites_keep_lsb_at_xmin() {
    let inter = Font::from_bytes(&std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).unwrap()).unwrap();
    let map = daegun::daecore::daetype::decoder::extract_ttf_tables(&inter.instance(&[("wght", 900.0)])).unwrap();
    let (glyf, hmtx) = (&map["glyf"], &map["hmtx"]);
    let long = u16_at(&map["hhea"], 34);
    let loca = daegun::daecore::daetype::instancer::parse_loca(&map, 1, u16_at(&map["maxp"], 4)).unwrap();
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

// Source Han Sans JP's VVAR moves perthousand's vertical origin by 10 at full weight, as HarfBuzz
// reports it.
#[test]
fn the_vertical_origin_varies() {
    let shs = Font::from_bytes(&std::fs::read(format!("{}/source-han-sans/SourceHanSansJP-VF.otf", crate::FONTS)).unwrap()).unwrap();
    assert_eq!(shs.vertical_origin(480, &[]), Some(863));
    assert_eq!(shs.vertical_origin(480, &[("wght", 900.0)]), Some(873));
}
