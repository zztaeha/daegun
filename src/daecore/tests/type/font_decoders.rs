use std::collections::BTreeMap;

use daegun::Font;
use daegun::daecore::daetype::decoder::{build_ttf, extract_ttf_tables, parse_all_name_strings};

fn bytes_of(rel: &str) -> Vec<u8> {
    std::fs::read(format!("{}/{rel}", crate::FONTS)).expect("fixture")
}

fn tables_of(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    extract_ttf_tables(bytes).expect("parses").into_iter().map(|(t, d)| (t, d.to_vec())).collect()
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
    let built = build_ttf(&map);

    let mut past = built.clone();
    put32(&mut past, entry(&built, "zzzz") + 12, 1 << 30);
    let font = Font::from_bytes(&past).expect("an entry past the file is skipped");
    assert_eq!(font.family_name().as_deref(), Some("EB Garamond"));

    let mut junk = built.clone();
    junk[entry(&built, "zzzz")] = 0xC3;
    assert!(Font::from_bytes(&junk).is_ok(), "a tag that is not ASCII is skipped");

    let mut clipped = built.clone();
    put32(&mut clipped, entry(&built, "hmtx") + 12, 1 << 30);
    let font = Font::from_bytes(&clipped).expect("hmtx past the file is clipped");
    let a = font.glyph_id('a' as u32).unwrap();
    assert_eq!(font.advance_widths(&[a], &[]), Font::from_bytes(&built).unwrap().advance_widths(&[a], &[]));
}

#[test]
fn a_table_listed_twice_is_read_from_its_first_entry() {
    let mut map = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    map.insert("zzzz".into(), vec![0; 8]);
    let mut built = build_ttf(&map);
    let (name, post, zzzz) = (entry(&built, "name"), entry(&built, "post"), entry(&built, "zzzz"));
    built.copy_within(post + 4..post + 16, zzzz + 4);
    built[zzzz..zzzz + 4].copy_from_slice(b"name");
    assert!(zzzz > name);
    let font = Font::from_bytes(&built).expect("opens");
    assert_eq!(font.family_name().as_deref(), Some("EB Garamond"));
}

#[test]
fn a_zero_length_table_is_a_missing_one() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    map.insert("gvar".into(), Vec::new());
    let font = Font::from_vec(build_ttf(&map)).expect("opens");
    let h = font.glyph_id('H' as u32).unwrap();
    let light = font.advance_widths(&[h], &[("wght", 100.0)]);
    let heavy = font.advance_widths(&[h], &[("wght", 900.0)]);
    assert_ne!(light, heavy, "an empty gvar failed the instance, so HVAR never applied");
}

#[test]
fn cff2_signs_and_ties_as_cff_does() {
    let serif = tables_of(&bytes_of("source-serif/SourceSerif4Variable-Roman.otf"));
    assert_eq!(&build_ttf(&serif)[..4], b"OTTO", "a CFF2 font is an OTTO font");

    let mut both = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    both.insert("CFF2".into(), serif["CFF2"].clone());
    let mut built = build_ttf(&both);
    built[..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    let map = extract_ttf_tables(&built).expect("parses");
    assert!(map.contains_key("glyf") && !map.contains_key("CFF2"), "a TrueType signature keeps glyf");
}

#[test]
fn a_collection_counts_only_the_faces_it_has_room_for() {
    assert_eq!(Font::ttc_font_count(b"ttcf\0\x01\0\0\xff\xff\xff\xff\0\0\0\0"), 1);
}

// Every name record moved to a platform, encoding or language other than (3, 1, 0x409) and Mac.
#[test]
fn a_name_is_read_from_any_record_freetype_would_read() {
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
        let font = Font::from_vec(build_ttf(&map)).expect("opens");
        assert_eq!(font.family_name().as_deref(), Some("EB Garamond"), "from ({platform}, {encoding}, {language:#x})");
    }
}

#[test]
fn a_mac_name_in_another_script_is_not_read_as_roman() {
    let mut map = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    let mut name = vec![0, 0, 0, 1, 0, 18, 0, 1, 0, 1, 0, 11, 0, 1, 0, 4, 0, 0];
    name.extend([0x8C, 0xB9, 0x83, 0x6D]);
    map.insert("name".into(), name);
    assert_eq!(Font::from_vec(build_ttf(&map)).unwrap().family_name(), None);
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
    map.insert("name".into(), daegun::daecore::daetype::TableBytes::from_vec(name));
    let decoded: usize = parse_all_name_strings(&map).values().map(String::len).sum();
    assert!(decoded <= 4 * len, "{decoded} bytes decoded from a {len}-byte table");
}

#[test]
fn an_axis_reports_its_flags_and_display_name() {
    let mut map = tables_of(&bytes_of("inter/InterVariable.ttf"));
    let fvar = map.get_mut("fvar").unwrap();
    let axes_at = usize::from(u16::from_be_bytes([fvar[4], fvar[5]]));
    fvar[axes_at + 17] |= 1;
    let font = Font::from_vec(build_ttf(&map)).expect("opens");
    let axes = font.axes();
    assert!(axes[0].is_hidden() && !axes[1].is_hidden());
    assert!(axes.iter().all(|a| a.name_id > 255), "{:?}", axes.iter().map(|a| a.name_id).collect::<Vec<_>>());
    assert!(font.name_string(axes[1].name_id).is_some());
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
    let font = Font::from_vec(build_ttf(&map)).expect("opens");
    let longer_names: Vec<_> = font.named_instances().into_iter().map(|i| i.postscript_name).collect();
    assert_eq!(longer_names, names);
}

#[test]
fn bits_os2_reserves_before_version_four_are_ignored() {
    let mut map = tables_of(&bytes_of("eb-garamond/EBGaramond.ttf"));
    let os2 = map.get_mut("OS/2").unwrap();
    os2[0..2].copy_from_slice(&3u16.to_be_bytes());
    os2[63] |= 0x80;
    os2[62] |= 0x02;
    let info = Font::from_vec(build_ttf(&map)).unwrap().os2_info().expect("an OS/2 table");
    assert!(!info.uses_typo_metrics() && !info.is_oblique());
}
