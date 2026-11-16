use std::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use crate::Font;
use crate::daecore::daetype::decoder::{build_ttf, extract_ttf_tables, parse_all_name_strings};
use crate::Location;

fn bytes_of(rel: &str) -> Vec<u8> {
    std::fs::read(format!("{}/{rel}", super::FONTS)).expect("fixture")
}

fn tables_of(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    extract_ttf_tables(bytes).expect("parses").into_iter().map(|(t, d)| (t.into(), d.to_vec())).collect()
}

// The directory entry of `tag` in a built font: where its offset and length fields sit.
fn entry(font: &[u8], tag: &str) -> usize {
    let n = usize::from(u16::from_be_bytes([font[4], font[5]]));
    (0..n).map(|i| 12 + 16 * i).find(|&e| &font[e..e + 4] == tag.as_bytes()).expect("tag in the directory")
}

fn put32(font: &mut [u8], at: usize, v: u32) {
    font[at..at + 4].copy_from_slice(&v.to_be_bytes());
}

#[test]
fn a_bad_directory_entry_is_skipped_not_fatal() {
    let mut map = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    map.insert("zzzz".into(), vec![0; 8]);
    let built = build_ttf(&map).unwrap();

    let mut past = built.clone();
    put32(&mut past, entry(&built, "zzzz") + 12, 1 << 30);
    let font = Font::from_bytes(&past).expect("an entry past the file is skipped");
    assert_eq!(font.family_name(&Location::default()).as_deref(), Some("EB Garamond"));

    let mut junk = built.clone();
    junk[entry(&built, "zzzz")] = 0xC3;
    assert!(Font::from_bytes(&junk).is_ok(), "a tag that is not ASCII is skipped");

    let mut clipped = built.clone();
    put32(&mut clipped, entry(&built, "hmtx") + 12, 1 << 30);
    let font = Font::from_bytes(&clipped).expect("hmtx past the file is clipped");
    let a = font.glyph_id('a').unwrap();
    let rebuilt = Font::from_bytes(&built).unwrap();
    assert_eq!(
        super::advances(&font, &[a], &Location::default()),
        super::advances(&rebuilt, &[a], &Location::default())
    );
}

#[test]
fn a_table_listed_twice_is_read_from_its_first_entry() {
    let mut map = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    map.insert("zzzz".into(), vec![0; 8]);
    let mut built = build_ttf(&map).unwrap();
    let (name, post, zzzz) = (entry(&built, "name"), entry(&built, "post"), entry(&built, "zzzz"));
    built.copy_within(post + 4..post + 16, zzzz + 4);
    built[zzzz..zzzz + 4].copy_from_slice(b"name");
    assert!(zzzz > name);
    let font = Font::from_bytes(&built).expect("opens");
    assert_eq!(font.family_name(&Location::default()).as_deref(), Some("EB Garamond"));
}

// Of a tag listed twice, the first entry counts though it's empty, and reads as missing, so a later copy is never
// read.
#[test]
fn an_empty_first_entry_of_a_repeated_tag_leaves_it_missing() {
    let mut map = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    map.insert("aaaa".into(), vec![0; 8]);
    let built = build_ttf(&map).unwrap();
    let with_empty = |tag: &str| {
        let (mut font, e) = (built.clone(), entry(&built, "aaaa"));
        font[e..e + 4].copy_from_slice(tag.as_bytes());
        put32(&mut font, e + 12, 0);
        Font::from_vec(font, 0).expect("opens")
    };
    assert_eq!(with_empty("name").family_name(&Location::default()), None, "a later name was read");
    let font = with_empty("loca");
    let a = font.glyph_id('a').unwrap();
    let mut path = crate::outline::Path::default();
    assert!(font.outline(a, &Location::default(), &mut path).is_err(), "a later loca was read");
}

// hhea and hmtx renamed HHEA and HMTX, the count widened, a bearing moved off its xMin: where hhea is missing, every
// path reads them as the lowercase pair, drawing, hinting and shaping alike.
#[test]
fn every_path_reads_hhea_and_hmtx_beyond_64k_where_hhea_is_missing() {
    let pair = |rel: &str, c: char| {
        let mut map = tables_of(&bytes_of(rel));
        let gid = Font::from_vec(build_ttf(&map).unwrap(), 0).unwrap().glyph_id(c).unwrap();
        let (g, long) = (gid.0 as usize, usize::from(u16::from_be_bytes([map["hhea"][34], map["hhea"][35]])));
        let at = if g < long { 4 * g + 2 } else { 4 * long + 2 * (g - long) };
        let lsb = i16::from_be_bytes([map["hmtx"][at], map["hmtx"][at + 1]]) + 50;
        map.get_mut("hmtx").unwrap()[at..at + 2].copy_from_slice(&lsb.to_be_bytes());
        let narrow = Font::from_vec(build_ttf(&map).unwrap(), 0).expect("opens");
        let hhea = map.remove("hhea").unwrap();
        let count = u32::from(u16::from_be_bytes([hhea[34], hhea[35]]));
        map.insert("HHEA".into(), [&hhea[..34], &count.to_be_bytes()[..]].concat());
        let hmtx = map.remove("hmtx").unwrap();
        map.insert("HMTX".into(), hmtx);
        (gid, narrow, Font::from_vec(build_ttf(&map).unwrap(), 0).expect("opens"))
    };
    let (a, narrow, wide) = pair("eb-garamond/EBGaramond.ttf", 'a');
    let outline = |f: &Font| {
        let mut path = crate::outline::Path::default();
        f.outline(a, &Location::default(), &mut path).ok().map(|()| path)
    };
    assert_eq!(outline(&wide), outline(&narrow), "drawn from another origin");
    let advance = super::advances(&narrow, &[a], &Location::default());
    assert!(
        advance != [500.0] && super::advances(&wide, &[a], &Location::default()) == advance,
        "shaping skipped HMTX"
    );
    let (a, narrow, wide) = pair("test-fixtures/hinted.ttf", 'A');
    let mode = crate::outline::HintMode::Font;
    let hinted = |f: &Font| f.hinted_outline(a, &Location::default(), 16.0, mode).expect("a finite px above zero");
    assert!(hinted(&narrow).is_some(), "nothing was hinted");
    assert_eq!(hinted(&wide), hinted(&narrow), "hinted from other phantom points");
}

// hhea counting long records to 'a''s and hmtx cut after its advance: drawing reads a long record whole or not at all,
// so 'a' has a bearing of 0 and draws from its xMin, its stored bearing left of where it was.
#[test]
fn a_cut_long_record_draws_its_glyph_from_its_xmin() {
    let mut map = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    let a = Font::from_vec(build_ttf(&map).unwrap(), 0).unwrap().glyph_id('a').unwrap();
    let at = 4 * a.0 as usize;
    let lsb = f32::from(i16::from_be_bytes([map["hmtx"][at + 2], map["hmtx"][at + 3]]));
    assert_ne!(lsb, 0.0, "'a' has no bearing to lose");
    let first = |map: &BTreeMap<String, Vec<u8>>| {
        let mut path = crate::outline::Path::default();
        let font = Font::from_vec(build_ttf(map).unwrap(), 0).unwrap();
        font.outline(a, &Location::default(), &mut path).expect("draws");
        path.parts().1[0]
    };
    let whole = first(&map);
    map.get_mut("hhea").unwrap()[34..36].copy_from_slice(&(a.0 as u16 + 1).to_be_bytes());
    map.get_mut("hmtx").unwrap().truncate(at + 2);
    assert_eq!(first(&map), (whole.x - lsb, whole.y).into());
}

#[test]
fn a_zero_length_table_is_a_missing_one() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    map.insert("gvar".into(), Vec::new());
    let font = Font::from_vec(build_ttf(&map).unwrap(), 0).expect("opens");
    let h = font.glyph_id('H').unwrap();
    let light = super::advances(&font, &[h], &super::located(&font, &[("wght", 100.0)]));
    let heavy = super::advances(&font, &[h], &super::located(&font, &[("wght", 900.0)]));
    assert_ne!(light, heavy, "an empty gvar failed the instance, so HVAR never applied");
}

#[test]
fn cff2_signs_and_ties_as_cff_does() {
    let serif = tables_of(&bytes_of("source-serif/SourceSerif4Variable-Roman.otf"));
    assert_eq!(&build_ttf(&serif).unwrap()[..4], b"OTTO", "a CFF2 font is an OTTO font");

    let mut both = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    both.insert("CFF2".into(), serif["CFF2"].clone());
    let mut built = build_ttf(&both).unwrap();
    built[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    let map = extract_ttf_tables(&built).expect("parses");
    assert!(map.contains_key("glyf") && !map.contains_key("CFF2"), "a TrueType signature keeps glyf");
}

#[test]
fn a_collection_counts_only_the_faces_it_has_room_for() {
    assert_eq!(Font::face_count(b"ttcf\0\x01\0\0\xff\xff\xff\xff\0\0\0\0"), 1);
}

// Every name record moved to a platform, encoding or language other than (3, 1, 0x409) and Mac.
#[test]
fn a_name_is_read_from_other_platforms_encodings_and_languages() {
    let base = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    for (platform, encoding, language) in [(3, 0, 0x409), (0, 3, 0), (3, 1, 0x809), (3, 10, 0x409), (3, 1, 0x40C)] {
        let mut map = base.clone();
        let name = map.get_mut("name").unwrap();
        let count = usize::from(u16::from_be_bytes([name[2], name[3]]));
        for i in 0..count {
            let rec = 6 + 12 * i;
            if name[rec + 1] == 3 {
                name[rec..rec + 6].copy_from_slice(&[0, platform, 0, encoding, (language >> 8) as u8, language as u8]);
            } else {
                name[rec + 1] = 7;
            }
        }
        let font = Font::from_vec(build_ttf(&map).unwrap(), 0).expect("opens");
        let from = format!("({platform}, {encoding}, {language:#x})");
        assert_eq!(font.family_name(&Location::default()).as_deref(), Some("EB Garamond"), "from {from}");
    }
}

#[test]
fn a_mac_name_in_another_script_is_not_read_as_roman() {
    let mut map = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    let mut name = vec![0, 0, 0, 1, 0, 18, 0, 1, 0, 1, 0, 11, 0, 1, 0, 4, 0, 0];
    name.extend([0x8C, 0xB9, 0x83, 0x6D]);
    map.insert("name".into(), name);
    assert_eq!(Font::from_vec(build_ttf(&map).unwrap(), 0).unwrap().family_name(&Location::default()), None);
}

// 2,048 records naming distinct IDs, each the whole 65,535-byte storage: 134 million bytes to decode
// from a table of 90 thousand.
#[test]
fn names_sharing_one_string_decode_within_the_tables_size() {
    const N: usize = 2048;
    let storage = 6 + 12 * N;
    let mut name = vec![0, 0];
    name.extend((N as u16).to_be_bytes());
    name.extend((storage as u16).to_be_bytes());
    for i in 0..N {
        name.extend([0, 1, 0, 0, 0, 0]);
        name.extend((i as u16 + 300).to_be_bytes());
        name.extend([0xFF, 0xFF, 0, 0]);
    }
    name.resize(storage + 0xFFFF, b'x');
    let len = name.len();
    let mut map = extract_ttf_tables(&bytes_of("eb-garamond/EBGaramond.ttf")).expect("parses");
    map.insert("name", crate::daecore::daetype::TableBytes::from_vec(name));
    let decoded: usize = parse_all_name_strings::<u16>(&map).values().map(String::len).sum();
    assert!(decoded <= 4 * len, "{decoded} bytes decoded from a {len}-byte table");
}

#[test]
fn an_axis_reports_its_flags_and_display_name() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let fvar = map.get_mut("fvar").unwrap();
    let axes_at = usize::from(u16::from_be_bytes([fvar[4], fvar[5]]));
    fvar[axes_at + 17] |= 1;
    let font = Font::from_vec(build_ttf(&map).unwrap(), 0).expect("opens");
    let axes = font.axes();
    assert!(axes[0].hidden && !axes[1].hidden);
    assert!(axes.iter().all(|a| a.name_id.0 > 255), "{:?}", axes.iter().map(|a| a.name_id).collect::<Vec<_>>());
    assert!(font.name(axes[1].name_id, &Location::default()).is_some());
}

// A later minor version may lengthen an instance record; the PostScript name is still where it was.
#[test]
fn a_longer_instance_record_keeps_its_postscript_name() {
    let base = Font::from_bytes(&bytes_of("inter/InterVariable.ttf")).unwrap();
    let names: Vec<_> = base.named_instances().into_iter().map(|i| i.postscript_name).collect();
    assert!(names.iter().all(Option::is_some));

    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let fvar = map.get_mut("fvar").unwrap();
    let field = |at: usize| usize::from(u16::from_be_bytes([fvar[at], fvar[at + 1]]));
    let (axes_at, axis_count, axis_size, count, size) = (field(4), field(8), field(10), field(12), field(14));
    let start = axes_at + axis_count * axis_size;
    let mut longer = fvar[..start].to_vec();
    for i in 0..count {
        longer.extend_from_slice(&fvar[start + i * size..start + (i + 1) * size]);
        longer.extend([0, 0]);
    }
    longer[14..16].copy_from_slice(&((size + 2) as u16).to_be_bytes());
    *fvar = longer;
    let font = Font::from_vec(build_ttf(&map).unwrap(), 0).expect("opens");
    let longer_names: Vec<_> = font.named_instances().into_iter().map(|i| i.postscript_name).collect();
    assert_eq!(longer_names, names);
}

// fsSelection's typo metrics and oblique bits are read at any OS/2 version: versions before 4 only reserved them.
#[test]
fn bits_os2_reserves_before_version_four_still_count() {
    let mut map = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    let os2 = map.get_mut("OS/2").unwrap();
    os2[0..2].copy_from_slice(&3u16.to_be_bytes());
    os2[63] |= 0x80;
    os2[62] |= 0x02;
    let info = Font::from_vec(build_ttf(&map).unwrap(), 0).unwrap().os2(&Location::default()).expect("an OS/2 table");
    assert!(info.uses_typo_metrics() && info.is_oblique());
}
