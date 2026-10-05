use std::collections::BTreeMap;

use daegun::Font;
use daegun::daecore::cache::FontCache;
use daegun::daecore::daeshaper::{buffer::Buffer, face::Face, plan::ShapePlan, shape};
use daegun::daecore::daetype::decoder::{build_ttf, extract_ttf_tables};

fn inter() -> Vec<u8> {
    std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("fixture")
}

fn be16(t: &mut Vec<u8>, v: &[u16]) {
    v.iter().for_each(|x| t.extend_from_slice(&x.to_be_bytes()));
}

fn be32(t: &mut Vec<u8>, v: u32) {
    t.extend_from_slice(&v.to_be_bytes());
}

// A GSUB or GPOS 1.1 with one DFLT script, one feature `tag` that has no lookups, and `lookup` as
// lookup 0. Its FeatureVariations, `gap` bytes past the rest, swaps in lookup 0 where the condition
// set holds: on `axis` from `min` up, or everywhere when `axis` is None.
fn layout(tag: &[u8; 4], lookup_type: u16, subtable: &[u8], gap: usize, condition: Option<(u16, i16)>) -> Vec<u8> {
    let mut t = Vec::new();
    be16(&mut t, &[1, 1, 14, 34, 46]);
    be32(&mut t, 0);
    be16(&mut t, &[1]);
    t.extend_from_slice(b"DFLT");
    be16(&mut t, &[8, 4, 0, 0, 0xFFFF, 1, 0]);
    assert_eq!(t.len(), 34);
    be16(&mut t, &[1]);
    t.extend_from_slice(tag);
    be16(&mut t, &[8, 0, 0]);
    assert_eq!(t.len(), 46);
    be16(&mut t, &[1, 4, lookup_type, 0, 1, 8]);
    t.extend_from_slice(subtable);
    t.resize(t.len() + gap, 0);

    let fv = t.len() as u32;
    t[10..14].copy_from_slice(&fv.to_be_bytes());
    be16(&mut t, &[1, 0]);
    be32(&mut t, 1);
    match condition {
        Some((axis, min)) => {
            be32(&mut t, 16);
            be32(&mut t, 30);
            be16(&mut t, &[1, 0, 6, 1, axis, min as u16, 0x4000]);
        }
        None => {
            be32(&mut t, 0);
            be32(&mut t, 16);
        }
    }
    be16(&mut t, &[1, 0, 1, 0]);
    be32(&mut t, 12);
    be16(&mut t, &[0, 1, 0]);
    t
}

fn single_substitution(from: u16, to: u16) -> Vec<u8> {
    let mut s = Vec::new();
    be16(&mut s, &[2, 8, 1, to, 1, 1, from]);
    s
}

fn pair_adjustment(first: u16, second: u16, x_advance: i16) -> Vec<u8> {
    let mut s = Vec::new();
    be16(&mut s, &[1, 18, 4, 0, 1, 12, 1, second, x_advance as u16, 1, 1, first]);
    s
}

fn with_tables(bytes: &[u8], tables: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut map: BTreeMap<String, Vec<u8>> =
        extract_ttf_tables(bytes).expect("parses").into_iter().map(|(t, d)| (t, d.to_vec())).collect();
    map.remove("GSUB");
    map.remove("GPOS");
    for (tag, data) in tables {
        map.insert(tag.to_string(), data.clone());
    }
    build_ttf(&map)
}

fn wght_axis(font: &Font) -> u16 {
    font.axes().iter().position(|a| a.tag == "wght").expect("Inter has wght") as u16
}

#[test]
fn an_instance_applies_the_substitutions_its_location_selects() {
    let base = Font::from_bytes(&inter()).expect("fixture");
    let (a, b) = (base.glyph_id('A' as u32).unwrap(), base.glyph_id('B' as u32).unwrap());
    let wght = wght_axis(&base);
    for gap in [0, 70_000] {
        let gsub = layout(b"liga", 1, &single_substitution(a, b), gap, Some((wght, 0x2000)));
        let font = Font::from_vec(with_tables(&inter(), &[("GSUB", gsub)])).expect("rebuilt");
        let heavy = font.shape("A", &[("wght", 900.0)], false).expect("shapes");
        assert_eq!(heavy.glyphs, [b], "at wght 900, {gap} bytes out");
        let light = font.shape("A", &[("wght", 100.0)], false).expect("shapes");
        assert_eq!(light.glyphs, [a], "at wght 100, {gap} bytes out");
    }
}

#[test]
fn an_instance_applies_the_positioning_its_location_selects() {
    let base = Font::from_bytes(&inter()).expect("fixture");
    let (a, v) = (base.glyph_id('A' as u32).unwrap(), base.glyph_id('V' as u32).unwrap());
    let gpos = layout(b"kern", 2, &pair_adjustment(a, v, -500), 0, Some((wght_axis(&base), 0x2000)));
    let font = Font::from_vec(with_tables(&inter(), &[("GPOS", gpos)])).expect("rebuilt");
    let advance = |wght: f64| font.shape("AV", &[("wght", wght)], false).expect("shapes").advances[0];
    let plain = |wght: f64| base.shape("A", &[("wght", wght)], false).expect("shapes").advances[0];
    let units = 1000.0 / f64::from(base.upm());
    assert!((advance(900.0) - (plain(900.0) - 500.0 * units)).abs() < 1e-6, "{} at 900", advance(900.0));
    assert!((advance(100.0) - plain(100.0)).abs() < 1e-6, "{} at 100", advance(100.0));
}

// The shaper looks up each table's own variation record: a GPOS that varies, beside a GSUB that
// does not, still swaps its feature.
#[test]
fn the_shaper_finds_each_table_its_own_variation() {
    let base = Font::from_bytes(&inter()).expect("fixture");
    let (a, v) = (base.glyph_id('A' as u32).unwrap(), base.glyph_id('V' as u32).unwrap());
    let gpos = layout(b"kern", 2, &pair_adjustment(a, v, -500), 0, None);
    let map = extract_ttf_tables(&with_tables(&inter(), &[("GPOS", gpos)])).expect("parses");
    let fc = FontCache::new(map);
    let face = Face::new(&fc, &[]);
    let advances = |text: &str| {
        let mut buffer = Buffer::new();
        buffer.push_str(text);
        let direction = shape::guess_segment_properties(&mut buffer);
        let plan = ShapePlan::with_script(buffer.script, &face, direction, &[], &[], &[], &[], &[]);
        shape::shape(&face, &plan, &mut buffer, direction);
        shape::shaped_glyphs(&buffer).iter().map(|g| g.x_advance).collect::<Vec<_>>()
    };
    assert_eq!(advances("AV")[0], advances("A")[0] - 500, "GPOS's own record was not applied");
}
