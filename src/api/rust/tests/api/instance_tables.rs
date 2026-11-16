use std::collections::BTreeMap;

use daegun::{Font, GlyphId, Location, Orientation};
use daegun::raw::build_font as build_ttf;
use crate::located;
use daegun::Tag;
use daegun::shape::ShapeOptions;

fn bytes_of(rel: &str) -> Vec<u8> {
    std::fs::read(format!("{}/{rel}", crate::FONTS)).expect("fixture")
}

fn tables_of(bytes: &[u8]) -> BTreeMap<Tag, Vec<u8>> {
    let f = Font::from_bytes(bytes).expect("parses");
    f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect()
}

fn instance(bytes: &[u8], axes: &[(&str, f64)]) -> BTreeMap<Tag, Vec<u8>> {
    let f = Font::from_bytes(bytes).expect("opens");
    tables_of(&f.instance(&located(&f, axes)).expect("instanced"))
}

fn u16_at(t: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([t[at], t[at + 1]])
}

fn i16_at(t: &[u8], at: usize) -> i16 {
    u16_at(t, at) as i16
}

type Key = (u16, u16, u16, u16);

// Every name record as (platform, encoding, language, name ID) and its text.
fn names(name: &[u8]) -> BTreeMap<Key, String> {
    let storage = usize::from(u16_at(name, 4));
    (0..usize::from(u16_at(name, 2)))
        .map(|i| {
            let rec = 6 + 12 * i;
            let key = (u16_at(name, rec), u16_at(name, rec + 2), u16_at(name, rec + 4), u16_at(name, rec + 6));
            let (len, off) = (usize::from(u16_at(name, rec + 8)), usize::from(u16_at(name, rec + 10)));
            let raw = &name[storage + off..storage + off + len];
            let text = if key.0 == 1 {
                raw.iter().map(|&b| char::from(b)).collect()
            } else {
                char::decode_utf16(raw.chunks(2).map(|c| u16_at(c, 0))).map(|c| c.unwrap_or('?')).collect()
            };
            (key, text)
        })
        .collect()
}

fn name_table(records: &BTreeMap<Key, String>) -> Vec<u8> {
    let mut head = vec![0, 0];
    head.extend((records.len() as u16).to_be_bytes());
    head.extend(((6 + 12 * records.len()) as u16).to_be_bytes());
    let mut storage = Vec::new();
    for (&(p, e, l, id), text) in records {
        let bytes: Vec<u8> =
            if p == 1 { text.bytes().collect() } else { text.encode_utf16().flat_map(u16::to_be_bytes).collect() };
        for v in [p, e, l, id, bytes.len() as u16, storage.len() as u16] {
            head.extend(v.to_be_bytes());
        }
        storage.extend(bytes);
    }
    head.extend(storage);
    head
}

fn english(names: &BTreeMap<Key, String>, id: u16) -> Option<&str> {
    names.get(&(3, 1, 0x409, id)).map(String::as_str)
}

// A source with long loca whose instance fits short offsets gets short ones, as a subset does, with head
// saying so; the glyphs read the same either way.
#[test]
fn an_instance_writes_loca_short_wherever_it_fits() {
    let mut tables = tables_of(&bytes_of("structure/TestGVAREight.ttf"));
    assert_eq!(i16_at(&tables[&Tag::new(b"head")], 50), 0, "the fixture's loca was short");
    let long: Vec<u8> =
        tables[&Tag::new(b"loca")].chunks(2).flat_map(|c| (2 * u32::from(u16_at(c, 0))).to_be_bytes()).collect();
    tables.insert(Tag::new(b"loca"), long);
    tables.get_mut(&Tag::new(b"head")).expect("head")[50..52].copy_from_slice(&1i16.to_be_bytes());
    let source = build_ttf(&tables).unwrap();
    let axis = Font::from_bytes(&source).expect("opens").axes()[0].clone();
    let out = instance(&source, &[(&axis.tag.to_string(), axis.max)]);
    assert_eq!(i16_at(&out[&Tag::new(b"head")], 50), 0, "the instance kept long loca");
    assert_eq!(out[&Tag::new(b"loca")].len(), tables[&Tag::new(b"loca")].len() / 2);
}

#[test]
fn a_broken_optional_variation_table_leaves_the_rest_varied() {
    let intact = Font::from_bytes(&bytes_of("inter/InterVariable.ttf")).unwrap();
    let h = intact.glyph_id('H').unwrap();
    let width = |f: &Font, wght| crate::advances(f, &[h], &located(f, &[("wght", wght)]))[0];
    let heavy = width(&intact, 900.0);
    assert_ne!(heavy, width(&intact, 400.0));

    let mut mvar = tables_of(&bytes_of("inter/InterVariable.ttf"));
    mvar.get_mut(&Tag::new(b"MVAR")).unwrap()[6..8].copy_from_slice(&4u16.to_be_bytes());
    let mut hvar = tables_of(&bytes_of("inter/InterVariable.ttf"));
    hvar.get_mut(&Tag::new(b"HVAR")).unwrap()[4..8].copy_from_slice(&0xFFFF_FF00u32.to_be_bytes());
    for (what, map) in [("MVAR", mvar), ("HVAR", hvar)] {
        let font = Font::from_vec(build_ttf(&map).unwrap(), 0).unwrap();
        let advance = width(&font, 900.0);
        assert!((advance - heavy).abs() <= 1.0, "with a broken {what}, H at wght 900 is {advance}, not {heavy}");
    }
}

#[test]
fn names_in_other_languages_are_not_relabeled_english() {
    let jp = instance(&bytes_of("source-han-sans/SourceHanSansJP-VF.otf"), &[("wght", 700.0)]);
    let jp = names(&jp[&Tag::new(b"name")]);
    assert!(english(&jp, 1).is_some());
    for id in [1, 2, 4, 6] {
        let japanese = jp.get(&(3, 1, 0x411, id));
        assert!(japanese.is_none_or(|t| Some(t.as_str()) != english(&jp, id)), "Japanese name {id} reads {japanese:?}");
    }

    let mut inter = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let mut records = names(&inter[&Tag::new(b"name")]);
    for id in [1, 2, 4, 6] {
        let text = english(&records, id).unwrap().to_string();
        records.insert((0, 3, 0, id), text);
    }
    inter.insert(Tag::new(b"name"), name_table(&records));
    let bold = names(&instance(&build_ttf(&inter).unwrap(), &[("wght", 700.0)])[&Tag::new(b"name")]);
    assert_eq!(bold.get(&(0, 3, 0, 2)).map(String::as_str), Some("Bold"), "the Unicode platform's names were dropped");
    assert_eq!(bold.get(&(0, 3, 0, 6)), bold.get(&(3, 1, 0x409, 6)));

    let garamond = instance(&bytes_of("eb-garamond/EBGaramond.ttf"), &[("wght", 600.0)]);
    assert!(names(&garamond[&Tag::new(b"name")]).keys().all(|k| k.0 != 1), "Mac names added to a font with none");
}

#[test]
fn the_weight_class_is_the_weight_drawn() {
    let inter = bytes_of("inter/InterVariable.ttf");
    for (asked, class) in [(2000.0, 900), (50.0, 100), (650.0, 650)] {
        let os2 = &instance(&inter, &[("wght", asked)])[&Tag::new(b"OS/2")];
        assert_eq!(u16_at(os2, 4), class, "asked for {asked}");
    }
}

#[test]
fn a_black_instance_is_regular_in_its_own_family() {
    let black = instance(&bytes_of("inter/InterVariable.ttf"), &[("wght", 900.0)]);
    let n = names(&black[&Tag::new(b"name")]);
    assert_eq!(
        (english(&n, 1), english(&n, 2), english(&n, 17)),
        (Some("Inter Variable Black"), Some("Regular"), Some("Black"))
    );
    assert_eq!(u16_at(&black[&Tag::new(b"OS/2")], 62) & 0x61, 0x40, "fsSelection is not REGULAR alone");
    assert_eq!(u16_at(&black[&Tag::new(b"head")], 44) & 1, 0, "macStyle says bold");
}

#[test]
fn an_off_instance_location_gets_its_own_postscript_name() {
    let inter = bytes_of("inter/InterVariable.ttf");
    let at = |wght: f64| instance(&inter, &[("wght", wght)]);
    let (n650, n750) = (names(&at(650.0)[&Tag::new(b"name")]), names(&at(750.0)[&Tag::new(b"name")]));
    assert_eq!(english(&n750, 6), Some("InterVariable_750wght"));
    assert_eq!(english(&n650, 6), Some("InterVariable_650wght"));
    assert!(english(&n750, 3).is_some_and(|id| id.contains("InterVariable_750wght")), "{:?}", english(&n750, 3));
    assert!(n750.keys().all(|k| k.3 != 25), "the variable font's PostScript prefix is kept");
    assert_eq!(english(&n750, 2), Some("Regular"));
    assert_eq!(u16_at(&at(750.0)[&Tag::new(b"OS/2")], 62) & 0x21, 0, "the bits say bold where name 2 says Regular");
}

// Inter's GDEF with a LigCaretList whose one caret is format 3 with VariationIndex (0, 0).
#[test]
fn a_variable_caret_is_baked_into_the_instance() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let gdef = map.get_mut(&Tag::new(b"GDEF")).unwrap();
    let list = gdef.len();
    gdef[8..10].copy_from_slice(&(list as u16).to_be_bytes());
    for v in [6u16, 1, 12, 1, 1, 100, 1, 4, 3, 300, 6, 0, 0, 0x8000] {
        gdef.extend(v.to_be_bytes());
    }
    let varied = Font::from_vec(build_ttf(&map).unwrap(), 0).unwrap();
    let at_900 = varied.ligature_carets(GlyphId(100), &located(&varied, &[("wght", 900.0)]), Orientation::Horizontal);
    assert_ne!(
        at_900,
        varied.ligature_carets(GlyphId(100), &Location::default(), Orientation::Horizontal),
        "the caret does not vary, so the test proves nothing"
    );
    let baked = Font::from_bytes(&varied.instance(&located(&varied, &[("wght", 900.0)])).expect("instanced")).unwrap();
    let carets = baked.ligature_carets(GlyphId(100), &Location::default(), Orientation::Horizontal);
    assert_eq!(carets.len(), 1);
    let unit = 1000.0 / f64::from(baked.units_per_em());
    let (baked_caret, varied_caret) = (carets[0].expect("a caret"), at_900[0].expect("a caret"));
    assert!((baked_caret - varied_caret).abs() <= unit / 2.0, "{carets:?} against {at_900:?}");
}

// head's box, hhea's extremes and OS/2's average width, each from the instance's glyphs measured there: Inter's and
// Source Serif 4's at wght 700, daegun's recorded values.
#[test]
fn an_instances_extents_follow_its_outlines() {
    let inter = instance(&bytes_of("inter/InterVariable.ttf"), &[("wght", 700.0)]);
    let serif = instance(&bytes_of("source-serif/SourceSerif4Variable-Roman.otf"), &[("wght", 700.0)]);
    let extents = |t: &BTreeMap<Tag, Vec<u8>>| {
        let (head, hhea) = (&t[&Tag::new(b"head")], &t[&Tag::new(b"hhea")]);
        [i16_at(head, 36), i16_at(head, 38), i16_at(head, 40), i16_at(head, 42), u16_at(hhea, 10) as i16,
            i16_at(hhea, 12), i16_at(hhea, 14), i16_at(hhea, 16), i16_at(&t[&Tag::new(b"OS/2")], 2)]
    };
    assert_eq!(extents(&inter), [-1613, -685, 5290, 2279, 5492, -1613, -2567, 5290, 1355]);
    assert_eq!(extents(&serif), [-290, -304, 2317, 1026, 2357, -240, -322, 2317, 577]);
}

// Source Han Sans JP's BASE carries a store; at wght 900 its first horizontal DFLT baselines take their deltas, -67
// and 827 becoming -94 and 854, daegun's recorded values.
#[test]
fn base_coordinates_are_baked() {
    let base = &instance(&bytes_of("source-han-sans/SourceHanSansJP-VF.otf"), &[("wght", 900.0)])[&Tag::new(b"BASE")];
    assert_eq!(u32::from_be_bytes(base[8..12].try_into().unwrap()), 0, "the store is kept");
    let axis = usize::from(u16_at(base, 4));
    let list = axis + usize::from(u16_at(base, axis + 2));
    let script = list + usize::from(u16_at(base, list + 6));
    let values = script + usize::from(u16_at(base, script));
    let coords: Vec<(u16, i16)> = (0..2)
        .map(|j| values + usize::from(u16_at(base, values + 4 + 2 * j)))
        .map(|at| (u16_at(base, at), i16_at(base, at + 2)))
        .collect();
    assert_eq!(coords, [(1, -94), (1, 854)]);
}

// At opsz 12, Caption (8 to 12, nominal 8) and SmallText (12 to 18, nominal 16) touch, and the spec
// takes the higher range.
#[test]
fn touching_ranges_keep_the_one_the_spec_picks() {
    let stat = &instance(&bytes_of("source-serif/SourceSerif4Variable-Roman.otf"), &[("wght", 400.0), ("opsz", 12.0)])
        [&Tag::new(b"STAT")];
    let offsets = u32::from_be_bytes(stat[14..18].try_into().unwrap()) as usize;
    let opsz: Vec<f64> = (0..usize::from(u16_at(stat, 12)))
        .map(|i| offsets + usize::from(u16_at(stat, offsets + 2 * i)))
        .filter(|&at| u16_at(stat, at) == 2 && u16_at(stat, at + 2) == 0)
        .map(|at| f64::from(i32::from_be_bytes(stat[at + 8..at + 12].try_into().unwrap())) / 65536.0)
        .collect();
    assert_eq!(opsz, [16.0], "nominal values kept on opsz");
}

// A vmtx whose vhea counts no long records gives no glyph a vertical metric, so each advances down by the line's
// height at every location, 2,478 units, and an instance keeps the table as it is.
#[test]
fn a_vmtx_without_long_metrics_instances_as_it_is() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let glyphs = usize::from(u16_at(&map[&Tag::new(b"maxp")], 4));
    let mut vhea = vec![0, 1, 0, 0];
    vhea.resize(36, 0);
    map.insert(Tag::new(b"vhea"), vhea);
    map.insert(Tag::new(b"vmtx"), vec![0; 2 * glyphs]);
    let source = build_ttf(&map).unwrap();
    let out = instance(&source, &[("wght", 900.0)]);
    assert_eq!((usize::from(u16_at(&out[&Tag::new(b"vhea")], 34)), out[&Tag::new(b"vmtx")].len()), (0, 2 * glyphs));
    let font = Font::from_bytes(&source).unwrap();
    let down = |axes| font.advance(GlyphId(10), &located(&font, axes), Orientation::Vertical);
    assert_eq!([&[][..], &[("wght", 900.0)]].map(down), [2478.0; 2]);
}

#[test]
fn the_default_request_names_the_default_location() {
    let inter = bytes_of("inter/InterVariable.ttf");
    assert_eq!(instance(&inter, &[]), instance(&inter, &[("wght", 400.0)]));
}

#[test]
fn a_font_without_os2_still_instances() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    map.remove(&Tag::new(b"OS/2"));
    let font = Font::from_vec(build_ttf(&map).unwrap(), 0).unwrap();
    let h = font.glyph_id('H').unwrap();
    let width = |wght| crate::advances(&font, &[h], &located(&font, &[("wght", wght)]));
    assert_ne!(width(900.0), width(400.0));
    assert!(
        !tables_of(&font.instance(&located(&font, &[("wght", 900.0)])).expect("instanced"))
            .contains_key(&Tag::new(b"OS/2"))
    );
}

// With no long records hmtx gives no glyph an advance, so each advances half an em at every location, which HVAR
// doesn't move, and an instance keeps the table as it is.
#[test]
fn a_cff2_font_without_long_metrics_advances_half_an_em_everywhere() {
    let source = bytes_of("source-serif/SourceSerif4Variable-Roman.otf");
    let mut map = tables_of(&source);
    map.get_mut(&Tag::new(b"hhea")).unwrap()[34..36].copy_from_slice(&0u16.to_be_bytes());
    let cut = build_ttf(&map).unwrap();
    let out = instance(&cut, &[("wght", 700.0)]);
    assert_eq!((u16_at(&out[&Tag::new(b"hhea")], 34), &out[&Tag::new(b"hmtx")]), (0, &map[&Tag::new(b"hmtx")]));
    let font = Font::from_bytes(&cut).unwrap();
    let widths = [&[][..], &[("wght", 700.0)]]
        .map(|axes| crate::advances(&font, &[GlyphId(10), GlyphId(100)], &located(&font, axes)));
    assert_eq!(widths, [[500.0; 2]; 2]);
}

// Shaping advances a glyph past 65,535 units where HVAR takes it there, rather than cutting it to 16 bits: Inter's H,
// stored at 65,530, widened at wght 900.
#[test]
fn an_advance_past_65535_units_is_not_cut() {
    let bytes = bytes_of("inter/InterVariable.ttf");
    let mut tables = tables_of(&bytes);
    let h = Font::from_bytes(&bytes).expect("opens").glyph_id('H').expect("H");
    let at = 4 * h.0 as usize;
    let widened =
        u16_at(&instance(&bytes, &[("wght", 900.0)])[&Tag::new(b"hmtx")], at) - u16_at(&tables[&Tag::new(b"hmtx")], at);
    tables.get_mut(&Tag::new(b"hmtx")).expect("hmtx")[at..at + 2].copy_from_slice(&65_530u16.to_be_bytes());
    let wide = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("still opens");
    let units = 65_530 + u64::from(widened);
    assert!(units > 65_535, "H widens by {widened} at wght 900");
    assert_eq!(crate::advances(&wide, &[h], &located(&wide, &[("wght", 900.0)])), [units as f64]);
}

// A glyph's bytes in glyf, through loca.
fn glyph_bytes(t: &BTreeMap<Tag, Vec<u8>>, gid: u32) -> &[u8] {
    let (loca, g) = (&t[&Tag::new(b"loca")], gid as usize);
    let (s, e) = if i16_at(&t[&Tag::new(b"head")], 50) == 0 {
        (2 * usize::from(u16_at(loca, 2 * g)), 2 * usize::from(u16_at(loca, 2 * g + 2)))
    } else {
        let at = |k: usize| u32::from_be_bytes([loca[k], loca[k + 1], loca[k + 2], loca[k + 3]]) as usize;
        (at(4 * g), at(4 * g + 4))
    };
    &t[&Tag::new(b"glyf")][s..e]
}

// Any negative contour count is a composite: one written -2 varies as it would at -1.
#[test]
fn a_composite_of_any_negative_count_is_varied() {
    let bytes = bytes_of("inter/InterVariable.ttf");
    let gid = Font::from_bytes(&bytes).expect("opens").glyph_id('Á').expect("Á");
    let mut tables = tables_of(&bytes);
    let at = glyph_bytes(&tables, gid.0).as_ptr() as usize - tables[&Tag::new(b"glyf")].as_ptr() as usize;
    assert_eq!(i16_at(&tables[&Tag::new(b"glyf")], at), -1, "Á is a composite");
    let want = instance(&bytes, &[("wght", 700.0)]);
    tables.get_mut(&Tag::new(b"glyf")).expect("glyf")[at + 1] = 0xFE;
    let got = instance(&build_ttf(&tables).unwrap(), &[("wght", 700.0)]);
    let (want, got) = (glyph_bytes(&want, gid.0), glyph_bytes(&got, gid.0));
    assert_ne!(want[2..], glyph_bytes(&tables_of(&bytes), gid.0)[2..], "the composite's offsets do not vary");
    assert_eq!(want[2..], got[2..]);
}

// An unbuildable location fails `instance`, `instance_tables` and subsetting, but drawing and shaping read the font's
// own tables: Inter without hmtx draws and shapes H at wght 900; Source Serif 4, its CFF2 cut short, only shapes.
#[test]
fn a_location_the_instancer_cannot_build_reports_its_error() {
    let cases = [
        ("inter/InterVariable.ttf", "missing hmtx", false),
        ("source-serif/SourceSerif4Variable-Roman.otf", "CFF2: CharStrings doesn't read", true),
    ];
    for (rel, want, varies) in cases {
        let mut tables = tables_of(&bytes_of(rel));
        match tables.get_mut(&Tag::new(b"CFF2")) {
            Some(cff2) => cff2.truncate(20_000),
            None => drop(tables.remove(&Tag::new(b"hmtx"))),
        }
        let broken = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("still opens");
        let at = [("wght", 900.0)];
        let heavy = located(&broken, &at);
        let error = |r: Result<(), daegun::Error>| r.map_err(|e| e.to_string());
        assert_eq!(error(broken.instance(&heavy).map(drop)), Err(want.to_string()), "{rel}: instance");
        assert_eq!(error(broken.instance_tables(&heavy).map(drop)), Err(want.to_string()), "{rel}: instance_tables");
        let glyphs = [GlyphId(1), GlyphId(2), GlyphId(3)];
        assert_eq!(error(broken.subset(glyphs, &heavy).map(drop)), Err(want.to_string()), "{rel}: subset");

        let g = broken.glyph_id('H').expect("H");
        let draw = |f: &Font, at: &[(&str, f64)]| {
            let mut path = daegun::outline::Path::default();
            f.outline(g, &located(f, at), &mut path).ok().map(|()| path)
        };
        let whole = Font::from_vec(bytes_of(rel), 0).expect("opens");
        let drawn = if varies { [None, None] } else { [draw(&whole, &at), draw(&whole, &[])] };
        assert_eq!([draw(&broken, &at), draw(&broken, &[])], drawn, "{rel}: drawn at wght 900 and the default");
        assert!(varies || drawn[0] != drawn[1], "{rel}: H varies");
        let (shaped, plain) = (
            broken.shape("Hamburg", &located(&broken, &at), &ShapeOptions::default()),
            broken.shape("Hamburg", &Location::default(), &ShapeOptions::default()),
        );
        let expected = match varies {
            true => whole.shape("Hamburg", &located(&whole, &at), &ShapeOptions::default()).advances.clone(),
            false => vec![1024.0, 1024.0, 1024.0, 1024.0, 1024.0, 960.0, 1024.0],
        };
        assert_eq!((&shaped.glyphs, &shaped.advances), (&plain.glyphs, &expected), "{rel}: shaped at wght 900");
        assert_ne!(shaped.advances, plain.advances, "{rel}: varied at wght 900");
    }
}

// `tables` with glyph `gid`'s record replaced by `record`, glyf padded to 4 and loca written long.
fn with_glyph(tables: &mut BTreeMap<Tag, Vec<u8>>, gid: u32, record: &[u8]) {
    let glyphs = u32::from(u16_at(&tables[&Tag::new(b"maxp")], 4));
    let records: Vec<Vec<u8>> =
        (0..glyphs).map(|g| if g == gid { record.to_vec() } else { glyph_bytes(tables, g).to_vec() }).collect();
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for r in records {
        loca.extend((glyf.len() as u32).to_be_bytes());
        glyf.extend(r);
        glyf.resize(glyf.len().next_multiple_of(4), 0);
    }
    loca.extend((glyf.len() as u32).to_be_bytes());
    tables.get_mut(&Tag::new(b"head")).expect("head")[50..52].copy_from_slice(&1u16.to_be_bytes());
    tables.insert(Tag::new(b"glyf"), glyf);
    tables.insert(Tag::new(b"loca"), loca);
}

// A glyph of no contours keeps its header and instructions (which can move phantom points) in an instance, here
// Inter's space given 3 bytes; off the default, none or overlong instructions keep nothing.
#[test]
fn an_empty_glyph_keeps_its_instructions() {
    let bytes = bytes_of("inter/InterVariable.ttf");
    let gid = Font::from_bytes(&bytes).expect("opens").glyph_id(' ').expect("a space");
    let hinted = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 0xB0, 0x01, 0x21];
    let (past, none) = ([&hinted[..11], &[9], &hinted[12..]].concat(), [&hinted[..11], &[0]].concat());
    for (record, off_default) in [(&hinted[..], &hinted[..]), (&past[..], &[][..]), (&none[..], &[][..])] {
        let mut tables = tables_of(&bytes);
        assert!(glyph_bytes(&tables, gid.0).is_empty(), "the space is empty");
        with_glyph(&mut tables, gid.0, record);
        for (wght, kept) in [(400.0, record), (700.0, off_default)] {
            let got = instance(&build_ttf(&tables).unwrap(), &[("wght", wght)]);
            assert_eq!(glyph_bytes(&got, gid.0).get(..kept.len()), Some(kept), "{record:?} at {wght}");
            assert!(glyph_bytes(&got, gid.0)[kept.len()..].iter().all(|&b| b == 0), "{record:?} at {wght}");
        }
    }
}

// MATH's values of VariationIndex devices take their deltas from GDEF's store and drop the devices in an instance:
// Inter given a MATH whose MathLeading of 100 names GDEF's row (0, 0), -119 at its heaviest, daegun's recorded value.
#[test]
fn math_values_are_baked_into_the_instance() {
    let mut tables = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let u16s = |v: &[u16]| v.iter().flat_map(|x| x.to_be_bytes()).collect::<Vec<u8>>();
    let constants = [u16s(&[0, 0, 0, 0, 100, 214]), vec![0; 50 * 4 + 2], u16s(&[0, 0, 0x8000])].concat();
    tables.insert(Tag::new(b"MATH"), [u16s(&[1, 0, 10, 0, 0]), constants].concat());
    let font = build_ttf(&tables).unwrap();
    let locations: [(&[(&str, f64)], i16); 3] =
        [(&[("wght", 900.0)], -19), (&[("wght", 650.0), ("opsz", 20.0)], 46), (&[("wght", 100.0)], 100)];
    for (axes, leading) in locations {
        let math = &instance(&font, axes)[&Tag::new(b"MATH")];
        assert_eq!((i16_at(math, 18), u16_at(math, 20)), (leading, 0), "{axes:?}");
    }
}

// An instanced subset's head box and lsb flag, hhea's widest advance and extremes, OS/2's average width and maxp's
// maxima. Each is taken over the glyphs the subset keeps.
fn subset_fields(bytes: &[u8], text: &str, axes: &[(&str, f64)]) -> Vec<i32> {
    let f = Font::from_bytes(bytes).expect("opens");
    fields_of(&f.subset_text(text, &located(&f, axes)).expect("a subset").font)
}

fn fields_of(ttf: &[u8]) -> Vec<i32> {
    let t = tables_of(ttf);
    let at = |tag: &[u8; 4], at: usize| i32::from(i16_at(&t[&Tag::new(tag)], at));
    let mut fields = [36, 38, 40, 42].map(|o| at(b"head", o)).to_vec();
    fields.extend([i32::from(u16_at(&t[&Tag::new(b"head")], 16) & 2), i32::from(u16_at(&t[&Tag::new(b"hhea")], 10))]);
    fields.extend([12, 14, 16].map(|o| at(b"hhea", o)));
    fields.push(at(b"OS/2", 2));
    if u16_at(&t[&Tag::new(b"maxp")], 0) == 1 {
        fields.extend([6, 8, 10, 12, 28, 30].map(|o| i32::from(u16_at(&t[&Tag::new(b"maxp")], o))));
    }
    fields
}

// With .notdef's outline kept: Inter's at default and wght 700, A's bearing off its xMin, and with only empty glyphs;
// Source Serif's CFF2 boxes measured there, extremes taking the instance's bearings.
#[test]
fn an_instanced_subset_measures_only_the_glyphs_it_keeps() {
    let inter = bytes_of("inter/InterVariable.ttf");
    let at_400 = [24, -654, 2822, 2144, 2, 2846, 24, 24, 2822, 2148, 72, 10, 66, 4, 2, 1];
    assert_eq!(subset_fields(&inter, "A", &[("wght", 400.0)]), at_400);
    assert_eq!(subset_fields(&inter, "A", &[]), at_400);
    let kept = [0, 2, 1560, 1899, 1900, 1903, 1964];
    let kept = kept.map(GlyphId);
    let raw = Font::from_bytes(&inter).expect("opens").subset(kept, &Location::default()).expect("a subset");
    assert_eq!(fields_of(&raw.font), at_400, "a variable font's own subset is its default instance's");
    let at_700 = [24, -654, 2822, 2144, 2, 2846, 24, 24, 2822, 2167, 72, 10, 66, 4, 2, 1];
    assert_eq!(subset_fields(&inter, "A", &[("wght", 700.0)]), at_700);

    let mut moved = tables_of(&inter);
    let a = 4 * Font::from_bytes(&inter).expect("opens").glyph_id('A').expect("an A").0 as usize + 2;
    let lsb = i16_at(&moved[&Tag::new(b"hmtx")], a) + 5;
    moved.get_mut(&Tag::new(b"hmtx")).expect("hmtx")[a..a + 2].copy_from_slice(&lsb.to_be_bytes());
    assert_eq!(subset_fields(&build_ttf(&moved).unwrap(), "A", &[("wght", 400.0)])[4], 0, "the flag is cleared");
    let mut cleared = tables_of(&inter);
    cleared.get_mut(&Tag::new(b"head")).expect("head")[17] &= !2;
    assert_eq!(subset_fields(&build_ttf(&cleared).unwrap(), "A", &[("wght", 400.0)])[4], 2, "the flag is set");

    let mut empty = tables_of(&inter);
    with_glyph(&mut empty, 0, &[]);
    let none = [0, 0, 0, 0, 2, 1344, -1513, -2200, 5290, 823, 0, 0, 0, 0, 0, 0];
    assert_eq!(subset_fields(&build_ttf(&empty).unwrap(), " ", &[("wght", 400.0)]), none);

    let serif = bytes_of("source-serif/SourceSerif4Variable-Roman.otf");
    assert_eq!(subset_fields(&serif, " AVy", &[]), [-3, -250, 665, 845, 2, 674, -3, 0, 665, 520]);
    let at_900 = subset_fields(&serif, " AVy", &[("wght", 900.0)]);
    assert_eq!([&at_900[..4], &at_900[5..6], &at_900[9..]].concat(), [-11, -232, 665, 833, 680, 527]);
}

// A whole instance's head flag and maxp maxima are recounted from its glyf too: Inter with them cleared comes back
// with the font's own at wght 700.
#[test]
fn an_instance_recounts_maxp_and_heads_lsb_flag() {
    let mut tables = tables_of(&bytes_of("inter/InterVariable.ttf"));
    tables.get_mut(&Tag::new(b"head")).expect("head")[17] &= !2;
    for at in [6, 8, 10, 12, 28, 30] {
        tables.get_mut(&Tag::new(b"maxp")).expect("maxp")[at..at + 2].fill(0);
    }
    let got = instance(&build_ttf(&tables).unwrap(), &[("wght", 700.0)]);
    let maxima = [6, 8, 10, 12, 28, 30].map(|at| u16_at(&got[&Tag::new(b"maxp")], at));
    assert_eq!((u16_at(&got[&Tag::new(b"head")], 16) & 2, maxima), (2, [132, 12, 112, 7, 7, 1]));
}

// Glyphs ending on the last advance keep bearings alone, as the last long record's advance carries on to them: Source
// Han Sans at 900 states 17,617 of 17,944 long records, Inter with its last three alike 2,934, a subset of emoji one.
#[test]
fn trailing_glyphs_of_the_last_advance_keep_bearings_alone() {
    let longs = |t: &BTreeMap<Tag, Vec<u8>>| (u16_at(&t[&Tag::new(b"hhea")], 34), u16_at(&t[&Tag::new(b"vhea")], 34));
    let shs = instance(&bytes_of("source-han-sans/SourceHanSansJP-VF.otf"), &[("wght", 900.0)]);
    assert_eq!(longs(&shs), (17_617, 17_617));
    assert_eq!(
        (shs[&Tag::new(b"hmtx")].len(), shs[&Tag::new(b"vmtx")].len()),
        (4 * 17_617 + 2 * 327, 4 * 17_617 + 2 * 327)
    );
    let mut inter = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let hmtx = inter.get_mut(&Tag::new(b"hmtx")).expect("hmtx");
    let last = hmtx.len() / 4 - 1;
    let advance = [hmtx[4 * (last - 3)], hmtx[4 * (last - 3) + 1]];
    (last - 2..=last).for_each(|g| hmtx[4 * g..4 * g + 2].copy_from_slice(&advance));
    assert_eq!(u16_at(&instance(&build_ttf(&inter).unwrap(), &[])[&Tag::new(b"hhea")], 34), 2_934);
    let emoji = Font::from_bytes(&bytes_of("noto-color-emoji/NotoColorEmoji.ttf")).expect("opens");
    let subset =
        tables_of(&emoji.subset_text("8\u{FE0F}\u{20E3}\u{1F642}A", &Location::default()).expect("a subset").font);
    assert_eq!(
        (longs(&subset), subset[&Tag::new(b"hmtx")].len(), subset[&Tag::new(b"vmtx")].len()),
        ((1, 1), 4 + 2 * 5, 4 + 2 * 5)
    );
}

// The font with `extra` tables more, tagged after every other, each naming the whole file.
fn naming_the_file(bytes: &[u8], extra: usize) -> Vec<u8> {
    let n = usize::from(u16_at(bytes, 4));
    let mut out = [&bytes[..4], &((n + extra) as u16).to_be_bytes()[..], &bytes[6..12]].concat();
    for e in bytes[12..12 + 16 * n].chunks(16) {
        let at = u32::from_be_bytes([e[8], e[9], e[10], e[11]]) + 16 * extra as u32;
        out.extend([&e[..8], &at.to_be_bytes()[..], &e[12..]].concat());
    }
    let len = ((bytes.len() + 16 * extra) as u32).to_be_bytes();
    (0..extra).for_each(|i| out.extend([format!("zzz{i}").as_bytes(), &[0; 8], &len[..]].concat()));
    [out, bytes[12 + 16 * n..].to_vec()].concat()
}

// A variable font and a static one whose tables take 3x their file, two more each naming the whole of it: 1.x read
// and wrote fonts like them, so they still instance, whole and by table.
#[test]
fn fonts_whose_tables_overlap_3x_still_instance() {
    for rel in ["noto-khmer/NotoSansKhmer.ttf", "scheherazade-new/ScheherazadeNew-Regular.ttf"] {
        let f = Font::from_vec(naming_the_file(&bytes_of(rel), 2), 0).expect("opens");
        let at = located(&f, &[("wght", 700.0)]);
        assert!(f.instance(&at).is_ok(), "{rel}");
        assert!(f.instance_tables(&at).is_ok(), "{rel}");
    }
}
