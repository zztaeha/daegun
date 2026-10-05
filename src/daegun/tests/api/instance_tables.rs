use std::collections::BTreeMap;

use daegun::Font;
use daegun::daecore::daetype::decoder::{build_ttf, extract_ttf_tables};

fn bytes_of(rel: &str) -> Vec<u8> {
    std::fs::read(format!("{}/{rel}", crate::FONTS)).expect("fixture")
}

fn tables_of(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    extract_ttf_tables(bytes).expect("parses").into_iter().map(|(t, d)| (t, d.to_vec())).collect()
}

fn instance(bytes: &[u8], axes: &[(&str, f64)]) -> BTreeMap<String, Vec<u8>> {
    tables_of(&Font::from_bytes(bytes).expect("opens").instance(axes))
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
        let bytes: Vec<u8> = if p == 1 { text.bytes().collect() } else { text.encode_utf16().flat_map(u16::to_be_bytes).collect() };
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

#[test]
fn a_broken_optional_variation_table_leaves_the_rest_varied() {
    let intact = Font::from_bytes(&bytes_of("inter/InterVariable.ttf")).unwrap();
    let h = intact.glyph_id('H' as u32).unwrap();
    let heavy = intact.advance_widths(&[h], &[("wght", 900.0)])[0];
    assert_ne!(heavy, intact.advance_widths(&[h], &[("wght", 400.0)])[0]);

    let mut mvar = tables_of(&bytes_of("inter/InterVariable.ttf"));
    mvar.get_mut("MVAR").unwrap()[6..8].copy_from_slice(&4u16.to_be_bytes());
    let mut hvar = tables_of(&bytes_of("inter/InterVariable.ttf"));
    hvar.get_mut("HVAR").unwrap()[4..8].copy_from_slice(&0xFFFF_FF00u32.to_be_bytes());
    for (what, map) in [("MVAR", mvar), ("HVAR", hvar)] {
        let font = Font::from_vec(build_ttf(&map)).unwrap();
        let advance = font.advance_widths(&[h], &[("wght", 900.0)])[0];
        assert!((advance - heavy).abs() <= 1.0, "with a broken {what}, H at wght 900 is {advance}, not {heavy}");
    }
}

#[test]
fn names_in_other_languages_are_not_relabeled_english() {
    let jp = instance(&bytes_of("source-han-sans/SourceHanSansJP-VF.otf"), &[("wght", 700.0)]);
    let jp = names(&jp["name"]);
    assert!(english(&jp, 1).is_some());
    for id in [1, 2, 4, 6] {
        let japanese = jp.get(&(3, 1, 0x411, id));
        assert!(japanese.is_none_or(|t| Some(t.as_str()) != english(&jp, id)), "Japanese name {id} reads {japanese:?}");
    }

    let mut inter = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let mut records = names(&inter["name"]);
    for id in [1, 2, 4, 6] {
        let text = english(&records, id).unwrap().to_string();
        records.insert((0, 3, 0, id), text);
    }
    inter.insert("name".into(), name_table(&records));
    let bold = names(&instance(&build_ttf(&inter), &[("wght", 700.0)])["name"]);
    assert_eq!(bold.get(&(0, 3, 0, 2)).map(String::as_str), Some("Bold"), "the Unicode platform's names were dropped");
    assert_eq!(bold.get(&(0, 3, 0, 6)), bold.get(&(3, 1, 0x409, 6)));

    let garamond = instance(&bytes_of("eb-garamond/EBGaramond.ttf"), &[("wght", 600.0)]);
    assert!(names(&garamond["name"]).keys().all(|k| k.0 != 1), "Mac names added to a font with none");
}

#[test]
fn the_weight_class_is_the_weight_drawn() {
    let inter = bytes_of("inter/InterVariable.ttf");
    for (asked, class) in [(2000.0, 900), (50.0, 100), (650.0, 650)] {
        let os2 = &instance(&inter, &[("wght", asked)])["OS/2"];
        assert_eq!(u16_at(os2, 4), class, "asked for {asked}");
    }
}

#[test]
fn a_black_instance_is_regular_in_its_own_family() {
    let black = instance(&bytes_of("inter/InterVariable.ttf"), &[("wght", 900.0)]);
    let n = names(&black["name"]);
    assert_eq!((english(&n, 1), english(&n, 2), english(&n, 17)), (Some("Inter Variable Black"), Some("Regular"), Some("Black")));
    assert_eq!(u16_at(&black["OS/2"], 62) & 0x61, 0x40, "fsSelection is not REGULAR alone");
    assert_eq!(u16_at(&black["head"], 44) & 1, 0, "macStyle says bold");
}

#[test]
fn an_off_instance_location_gets_its_own_postscript_name() {
    let inter = bytes_of("inter/InterVariable.ttf");
    let at = |wght: f64| instance(&inter, &[("wght", wght)]);
    let (n650, n750) = (names(&at(650.0)["name"]), names(&at(750.0)["name"]));
    assert_eq!(english(&n750, 6), Some("InterVariable_750wght"));
    assert_eq!(english(&n650, 6), Some("InterVariable_650wght"));
    assert!(english(&n750, 3).is_some_and(|id| id.contains("InterVariable_750wght")), "{:?}", english(&n750, 3));
    assert!(n750.keys().all(|k| k.3 != 25), "the variable font's PostScript prefix is kept");
    assert_eq!(english(&n750, 2), Some("Regular"));
    assert_eq!(u16_at(&at(750.0)["OS/2"], 62) & 0x21, 0, "the bits say bold where name 2 says Regular");
}

// Inter's GDEF with a LigCaretList whose one caret is format 3 with VariationIndex (0, 0).
#[test]
fn a_variable_caret_is_baked_into_the_instance() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let gdef = map.get_mut("GDEF").unwrap();
    let list = gdef.len();
    gdef[8..10].copy_from_slice(&(list as u16).to_be_bytes());
    for v in [6u16, 1, 12, 1, 1, 100, 1, 4, 3, 300, 6, 0, 0, 0x8000] {
        gdef.extend(v.to_be_bytes());
    }
    let varied = Font::from_vec(build_ttf(&map)).unwrap();
    let at_900 = varied.ligature_carets(100, &[("wght", 900.0)], false);
    assert_ne!(at_900, varied.ligature_carets(100, &[], false), "the caret does not vary, so the test proves nothing");
    let baked = Font::from_vec(varied.instance(&[("wght", 900.0)])).unwrap();
    let carets = baked.ligature_carets(100, &[], false);
    assert_eq!(carets.len(), 1);
    let unit = 1000.0 / f64::from(baked.upm());
    let (baked_caret, varied_caret) = (carets[0].expect("a caret"), at_900[0].expect("a caret"));
    assert!((baked_caret - varied_caret).abs() <= unit / 2.0, "{carets:?} against {at_900:?}");
}

// head's box, hhea's extremes and OS/2's average width, as fontTools 4.63 recalculates them for the
// same instances.
#[test]
fn an_instances_extents_follow_its_outlines() {
    let inter = instance(&bytes_of("inter/InterVariable.ttf"), &[("wght", 700.0)]);
    let serif = instance(&bytes_of("source-serif/SourceSerif4Variable-Roman.otf"), &[("wght", 700.0)]);
    let extents = |t: &BTreeMap<String, Vec<u8>>| {
        let (head, hhea) = (&t["head"], &t["hhea"]);
        [i16_at(head, 36), i16_at(head, 38), i16_at(head, 40), i16_at(head, 42), u16_at(hhea, 10) as i16,
            i16_at(hhea, 12), i16_at(hhea, 14), i16_at(hhea, 16), i16_at(&t["OS/2"], 2)]
    };
    assert_eq!(extents(&inter), [-1613, -685, 5290, 2279, 5492, -1613, -2567, 5290, 1355]);
    assert_eq!(extents(&serif), [-290, -305, 2317, 1027, 2357, -240, -323, 2317, 577]);
}

// Source Han Sans JP's BASE carries a store; at wght 900 fontTools reads its first horizontal DFLT
// baselines as -94 and 854, from -67 and 827.
#[test]
fn base_coordinates_are_baked() {
    let base = &instance(&bytes_of("source-han-sans/SourceHanSansJP-VF.otf"), &[("wght", 900.0)])["BASE"];
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
    let stat = &instance(&bytes_of("source-serif/SourceSerif4Variable-Roman.otf"), &[("wght", 400.0), ("opsz", 12.0)])["STAT"];
    let offsets = u32::from_be_bytes(stat[14..18].try_into().unwrap()) as usize;
    let opsz: Vec<f64> = (0..usize::from(u16_at(stat, 12)))
        .map(|i| offsets + usize::from(u16_at(stat, offsets + 2 * i)))
        .filter(|&at| u16_at(stat, at) == 2 && u16_at(stat, at + 2) == 0)
        .map(|at| f64::from(i32::from_be_bytes(stat[at + 8..at + 12].try_into().unwrap())) / 65536.0)
        .collect();
    assert_eq!(opsz, [16.0], "nominal values kept on opsz");
}

#[test]
fn a_vmtx_of_bearings_only_keeps_its_count() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let glyphs = usize::from(u16_at(&map["maxp"], 4));
    let mut vhea = vec![0, 1, 0, 0];
    vhea.resize(36, 0);
    map.insert("vhea".into(), vhea);
    map.insert("vmtx".into(), vec![0; 2 * glyphs]);
    let out = instance(&build_ttf(&map), &[("wght", 900.0)]);
    assert_eq!((u16_at(&out["vhea"], 34), out["vmtx"].len()), (0, 2 * glyphs));
}

#[test]
fn the_default_request_names_the_default_location() {
    let inter = bytes_of("inter/InterVariable.ttf");
    assert_eq!(instance(&inter, &[]), instance(&inter, &[("wght", 400.0)]));
}

#[test]
fn a_font_without_os2_still_instances() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    map.remove("OS/2");
    let font = Font::from_vec(build_ttf(&map)).unwrap();
    let h = font.glyph_id('H' as u32).unwrap();
    assert_ne!(font.advance_widths(&[h], &[("wght", 900.0)]), font.advance_widths(&[h], &[("wght", 400.0)]));
    assert!(!tables_of(&font.instance(&[("wght", 900.0)])).contains_key("OS/2"));
}

#[test]
fn a_cff2_font_without_long_metrics_keeps_its_count() {
    let mut map = tables_of(&bytes_of("source-serif/SourceSerif4Variable-Roman.otf"));
    let hmtx_len = map["hmtx"].len();
    map.get_mut("hhea").unwrap()[34..36].copy_from_slice(&0u16.to_be_bytes());
    let out = instance(&build_ttf(&map), &[("wght", 700.0)]);
    assert_eq!((u16_at(&out["hhea"], 34), out["hmtx"].len()), (0, hmtx_len));
}

// A glyph's bytes in glyf, through loca.
fn glyph_bytes(t: &BTreeMap<String, Vec<u8>>, gid: u16) -> &[u8] {
    let (loca, g) = (&t["loca"], usize::from(gid));
    let (s, e) = if i16_at(&t["head"], 50) == 0 {
        (2 * usize::from(u16_at(loca, 2 * g)), 2 * usize::from(u16_at(loca, 2 * g + 2)))
    } else {
        let at = |k: usize| u32::from_be_bytes([loca[k], loca[k + 1], loca[k + 2], loca[k + 3]]) as usize;
        (at(4 * g), at(4 * g + 4))
    };
    &t["glyf"][s..e]
}

// Any negative contour count is a composite: one written -2 varies as it would at -1.
#[test]
fn a_composite_of_any_negative_count_is_varied() {
    let bytes = bytes_of("inter/InterVariable.ttf");
    let gid = Font::from_bytes(&bytes).expect("opens").glyph_id('Á' as u32).expect("Á");
    let mut tables = tables_of(&bytes);
    let at = glyph_bytes(&tables, gid).as_ptr() as usize - tables["glyf"].as_ptr() as usize;
    assert_eq!(i16_at(&tables["glyf"], at), -1, "Á is a composite");
    let want = instance(&bytes, &[("wght", 700.0)]);
    tables.get_mut("glyf").expect("glyf")[at + 1] = 0xFE;
    let got = instance(&build_ttf(&tables), &[("wght", 700.0)]);
    let (want, got) = (glyph_bytes(&want, gid), glyph_bytes(&got, gid));
    assert_ne!(want[2..], glyph_bytes(&tables_of(&bytes), gid)[2..], "the composite's offsets do not vary");
    assert_eq!(want[2..], got[2..]);
}
