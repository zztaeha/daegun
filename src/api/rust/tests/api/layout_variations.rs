use std::collections::BTreeMap;

use daegun::shape::ShapeOptions;
use daegun::{Font, GlyphId, Location, Tag};

use super::aat_subset::{carets_with, u16s, u32s, F, I};
use crate::located;

fn inter() -> Vec<u8> {
    std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("fixture")
}

fn be16(t: &mut Vec<u8>, v: &[u16]) {
    v.iter().for_each(|x| t.extend_from_slice(&x.to_be_bytes()));
}

fn be32(t: &mut Vec<u8>, v: u32) {
    t.extend_from_slice(&v.to_be_bytes());
}

// A GSUB or GPOS 1.1 of one DFLT script and a lookup-less feature `tag`, `lookup` as lookup 0, whose FeatureVariations,
// `gap` bytes past the rest, swaps it in where `axis` is `min` or more, or everywhere when `axis` is None.
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
    let f = Font::from_bytes(bytes).expect("parses");
    let mut map: BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    map.remove(&Tag::new(b"GSUB"));
    map.remove(&Tag::new(b"GPOS"));
    for (tag, data) in tables {
        map.insert(tag.parse().expect("a tag"), data.clone());
    }
    daegun::raw::build_font(&map).unwrap()
}

fn wght_axis(font: &Font) -> u16 {
    font.axes().iter().position(|a| a.tag == Tag::new(b"wght")).expect("Inter has wght") as u16
}

#[test]
fn an_instance_applies_the_substitutions_its_location_selects() {
    let base = Font::from_bytes(&inter()).expect("fixture");
    let (a, b) = (base.glyph_id('A').unwrap(), base.glyph_id('B').unwrap());
    let wght = wght_axis(&base);
    for gap in [0, 70_000] {
        let gsub = layout(b"liga", 1, &single_substitution(a.0 as u16, b.0 as u16), gap, Some((wght, 0x2000)));
        let font = Font::from_vec(with_tables(&inter(), &[("GSUB", gsub)]), 0).expect("rebuilt");
        let heavy = font.shape("A", &located(&font, &[("wght", 900.0)]), &ShapeOptions::default());
        assert_eq!(heavy.glyphs, [b], "at wght 900, {gap} bytes out");
        let light = font.shape("A", &located(&font, &[("wght", 100.0)]), &ShapeOptions::default());
        assert_eq!(light.glyphs, [a], "at wght 100, {gap} bytes out");
    }
}

#[test]
fn an_instance_applies_the_positioning_its_location_selects() {
    let base = Font::from_bytes(&inter()).expect("fixture");
    let (a, v) = (base.glyph_id('A').unwrap(), base.glyph_id('V').unwrap());
    let gpos = layout(b"kern", 2, &pair_adjustment(a.0 as u16, v.0 as u16, -500), 0, Some((wght_axis(&base), 0x2000)));
    let font = Font::from_vec(with_tables(&inter(), &[("GPOS", gpos)]), 0).expect("rebuilt");
    let advance =
        |wght: f64| font.shape("AV", &located(&font, &[("wght", wght)]), &ShapeOptions::default()).advances[0];
    let plain = |wght: f64| base.shape("A", &located(&base, &[("wght", wght)]), &ShapeOptions::default()).advances[0];
    assert!((advance(900.0) - (plain(900.0) - 500.0)).abs() < 1e-6, "{} at 900", advance(900.0));
    assert!((advance(100.0) - plain(100.0)).abs() < 1e-6, "{} at 100", advance(100.0));
}

// The shaper looks up each table's own variation record: a GPOS that varies, beside a GSUB that
// does not, still swaps its feature.
#[test]
fn the_shaper_finds_each_table_its_own_variation() {
    let base = Font::from_bytes(&inter()).expect("fixture");
    let (a, v) = (base.glyph_id('A').unwrap(), base.glyph_id('V').unwrap());
    let gpos = layout(b"kern", 2, &pair_adjustment(a.0 as u16, v.0 as u16, -500), 0, None);
    let font = Font::from_vec(with_tables(&inter(), &[("GPOS", gpos)]), 0).expect("rebuilt");
    let advance = |text| font.shape(text, &Location::default(), &ShapeOptions::default()).advances[0];
    assert_eq!(advance("AV"), advance("A") - 500.0, "GPOS's own record was not applied");
}

// `head`, then an offset of `width` bytes from its start to each item, laid out after the offsets, or a null one for
// None.
fn with_offsets(mut head: Vec<u8>, width: usize, items: &[Option<Vec<u8>>]) -> Vec<u8> {
    let start = head.len() + width * items.len();
    let mut tail = Vec::new();
    for item in items {
        let off = item.as_ref().map_or(0, |item| {
            tail.extend(item);
            start + tail.len() - item.len()
        });
        head.extend(&(off as u32).to_be_bytes()[4 - width..]);
    }
    [head, tail].concat()
}

fn axis(index: u16, min: i16, max: i16) -> Option<Vec<u8>> {
    Some(u16s(&[1, index, min as u16, max as u16]))
}

fn value(default: i16, index: u32) -> Option<Vec<u8>> {
    Some([u16s(&[2, default as u16]), u32s(&[index])].concat())
}

fn all(conditions: &[Option<Vec<u8>>]) -> Option<Vec<u8>> {
    Some(with_offsets([u16s(&[3]), vec![conditions.len() as u8]].concat(), 3, conditions))
}

fn any(conditions: &[Option<Vec<u8>>]) -> Option<Vec<u8>> {
    Some(with_offsets([u16s(&[4]), vec![conditions.len() as u8]].concat(), 3, conditions))
}

fn not(condition: Option<Vec<u8>>) -> Option<Vec<u8>> {
    Some(with_offsets(u16s(&[5]), 3, &[condition]))
}

// A GSUB 1.1 whose ccmp and liga run `features`' indices into its one lookup, f to i, and whose variation record,
// held where all `conditions` do (always for None), swaps in `substitutions`: lookups, or None for a null offset.
fn gsub(
    (f, i): (u16, u16),
    features: &[&[u16]],
    conditions: Option<&[Option<Vec<u8>>]>,
    substitutions: &[(u16, Option<&[u16]>)],
) -> Vec<u8> {
    let feature = |lookups: &[u16]| [u16s(&[0, lookups.len() as u16]), u16s(lookups)].concat();
    let indices: Vec<u16> = (0..features.len() as u16).collect();
    let scripts = [u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, indices.len() as u16]), u16s(&indices)];
    let mut list = u16s(&[features.len() as u16]);
    let mut bodies = Vec::new();
    for (tag, lookups) in [b"ccmp", b"liga"].iter().zip(features) {
        list.extend([tag.to_vec(), u16s(&[(2 + 6 * features.len() + bodies.len()) as u16])].concat());
        bodies.extend(feature(lookups));
    }
    let lookups = u16s(&[1, 4, 1, 0, 1, 8, 2, 8, 1, i, 1, 1, f]);
    let mut table = u16s(&[1, 0, substitutions.len() as u16]);
    let mut alternates = Vec::new();
    for &(index, alternate) in substitutions {
        let off = alternate.map_or(0, |_| 6 + 6 * substitutions.len() + alternates.len());
        alternates.extend(alternate.map(feature).unwrap_or_default());
        table.extend([u16s(&[index]), u32s(&[off as u32])].concat());
    }
    let set = conditions.map(|c| with_offsets(u16s(&[c.len() as u16]), 4, c)).unwrap_or_default();
    let set_at = if conditions.is_some() { 16 } else { 0 };
    let variations = [u16s(&[1, 0]), u32s(&[1, set_at, 16 + set.len() as u32]), set, table, alternates].concat();
    let (scripts, list) = (scripts.concat(), [list, bodies].concat());
    let at = [14, 14 + scripts.len(), 14 + scripts.len() + list.len()];
    let header = [u16s(&[1, 1, at[0] as u16, at[1] as u16, at[2] as u16]), u32s(&[(at[2] + lookups.len()) as u32])];
    [header.concat(), scripts, list, lookups, variations].concat()
}

// GDEF 1.3 with only a store, whose one delta, (0, 0), is +2 at the end of Inter's second axis, wght.
fn gdef_store() -> Vec<u8> {
    let regions = u16s(&[2, 1, 0, 0, 0, 0, 16384, 16384]);
    let store = [u16s(&[1]), u32s(&[12]), u16s(&[1]), u32s(&[28]), regions, u16s(&[1, 0, 1, 0]), vec![2]];
    [u16s(&[1, 3, 0, 0, 0, 0, 0]), u32s(&[18]), store.concat()].concat()
}

// Whether f shapes as i under the GSUB `gsub` builds for the font's f and i: on carets.ttf, which has no axes, and on
// Inter at wght 900, with the GDEF above, where the instance resolves the variations.
fn substitutes(gsub: impl Fn((u16, u16)) -> Vec<u8>) -> [bool; 2] {
    let still = carets_with(&[("GSUB", gsub((F, I)))]);
    let base = Font::from_bytes(&inter()).expect("fixture");
    let (f, i) = (base.glyph_id('f').expect("an f"), base.glyph_id('i').expect("an i"));
    let varied =
        Font::from_vec(with_tables(&inter(), &[("GSUB", gsub((f.0 as u16, i.0 as u16))), ("GDEF", gdef_store())]), 0);
    let varied = varied.expect("rebuilt");
    let (still, varied) = (
        still.shape("f", &Location::default(), &ShapeOptions::default()),
        varied.shape("f", &located(&varied, &[("wght", 900.0)]), &ShapeOptions::default()),
    );
    [still.glyphs == [GlyphId(I.into())], varied.glyphs == [i]]
}

// Each format of condition, nested or not: an axis past the font's reads 0, a value varies by GDEF's store only at a
// location, and a null condition holds. Glyphs at no location and at wght 900.
#[test]
fn each_condition_format_holds_as_it_says() {
    let holds = |condition: Option<Vec<u8>>| {
        substitutes(|glyphs| gsub(glyphs, &[&[]], Some(std::slice::from_ref(&condition)), &[(0, Some(&[0]))]))
    };
    let heavy = || axis(1, 8192, 16384);
    assert_eq!(holds(heavy()), [false, true]);
    assert_eq!(holds(axis(5, -16384, 16384)), [true, true]);
    assert_eq!(holds(value(-1, 0)), [false, true]);
    assert_eq!(holds(value(-2, 0)), [false, false]);
    assert_eq!(holds(value(1, u32::MAX)), [true, true]);
    assert_eq!(holds(None), [true, true]);
    assert_eq!(holds(all(&[value(1, u32::MAX), heavy()])), [false, true]);
    assert_eq!(holds(all(&[value(1, u32::MAX), None])), [true, true]);
    assert_eq!(holds(any(&[None, heavy()])), [true, true]);
    assert_eq!(holds(not(heavy())), [true, false]);
    assert_eq!(holds(not(None)), [false, false]);
}

// The first record for a feature substitutes it, even after one for a later feature, and a null alternate leaves it
// no lookups. Glyphs at no location and at wght 900.
#[test]
fn a_variation_substitutes_a_feature_by_its_first_record() {
    let substituted = |features: &[&[u16]], substitutions: &[(u16, Option<&[u16]>)]| {
        substitutes(|glyphs| gsub(glyphs, features, None, substitutions))
    };
    assert_eq!(substituted(&[&[], &[]], &[(1, Some(&[])), (0, Some(&[0]))]), [true, true]);
    assert_eq!(substituted(&[&[]], &[(0, Some(&[0])), (0, Some(&[]))]), [true, true]);
    assert_eq!(substituted(&[&[0]], &[(0, None)]), [false, false]);
}

// A FeatureLookups record: its condition, None for a null one, and the lookup it turns on.
type Record = (Option<Vec<u8>>, u16);

// A GSUB 1.1 whose ccmp has no lookups of its own and lookup variations for it, `records`' FeatureLookups in order:
// lookup 0 turns f to i, 1 turns f to l.
fn lookup_variations((f, i, l): (u16, u16, u16), records: &[&[Record]]) -> Vec<u8> {
    let scripts = [u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0])].concat();
    let features = [u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 0])].concat();
    let lookup = |to: u16| u16s(&[1, 0, 1, 8, 2, 8, 1, to, 1, 1, f]);
    let lookups = [u16s(&[2, 6, 28]), lookup(i), lookup(l)].concat();
    let mut variations = [u16s(&[1, 1]), u32s(&[0, records.len() as u32])].concat();
    let mut tables = Vec::new();
    for pairs in records {
        let items: Vec<_> = pairs.iter().flat_map(|(c, lookup)| [c.clone(), Some(u16s(&[1, *lookup]))]).collect();
        let head = [u16s(&[1, 0, 0]), u32s(&[pairs.len() as u32])].concat();
        variations.extend([u16s(&[0]), u32s(&[(12 + 6 * records.len() + tables.len()) as u32])].concat());
        tables.extend(with_offsets(head, 4, &items));
    }
    let at = [14, 14 + scripts.len(), 14 + scripts.len() + features.len()];
    let header = [u16s(&[1, 1, at[0] as u16, at[1] as u16, at[2] as u16]), u32s(&[(at[2] + lookups.len()) as u32])];
    [header.concat(), scripts, features, lookups, variations, tables].concat()
}

// ccmp, of no lookups of its own, turns on f to i where wght is heavy and f to l where light, at each location and
// instance. Of two records the first holds, whether shaping or instancing.
#[test]
fn a_feature_turns_on_the_lookups_whose_conditions_hold() {
    let base = Font::from_bytes(&inter()).expect("fixture");
    let [f, i, l] = ['f', 'i', 'l'].map(|c| base.glyph_id(c).expect("a glyph"));
    let font = |records: &[&[Record]]| {
        let gsub = lookup_variations((f.0 as u16, i.0 as u16, l.0 as u16), records);
        Font::from_vec(with_tables(&inter(), &[("GSUB", gsub)]), 0).expect("rebuilt")
    };
    let (heavy, light) = ((axis(1, 8192, 16384), 0), (axis(1, -16384, -8192), 1));
    let shaped =
        |font: &Font, axes: &[(&str, f64)]| font.shape("f", &located(font, axes), &ShapeOptions::default()).glyphs[0];
    let varied = font(&[&[heavy.clone(), light.clone()]]);
    let at = [100.0, 900.0, 400.0, 250.0, 700.0].map(|w| shaped(&varied, &[("wght", w)]));
    assert_eq!(at, [l, i, f, l, i], "one font, location after location");
    let first = font(&[&[heavy, light], &[(None, 1)]]);
    assert_eq!([100.0, 400.0, 900.0].map(|w| shaped(&first, &[("wght", w)])), [l, f, i], "the first record");
    let instance = |w: f64| {
        Font::from_bytes(&first.instance(&located(&first, &[("wght", w)])).expect("instances")).expect("parses")
    };
    assert_eq!([100.0, 400.0, 900.0].map(|w| shaped(&instance(w), &[])), [l, f, i], "its instances");
}
