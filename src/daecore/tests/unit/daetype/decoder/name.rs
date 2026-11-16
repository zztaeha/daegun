use super::*;
use crate::daecore::daetype::TableBytes;

#[test]
fn mac_bytes_and_characters_round_trip() {
    for b in 0..=0xFFu8 {
        assert_eq!(mac_roman_byte(mac_roman_char(b) as u32), Some(b), "MacRoman 0x{b:02X}");
        assert_eq!(mac_turkish_byte(mac_turkish_char(b) as u32), Some(b), "MacOS Turkish 0x{b:02X}");
    }
    assert_eq!(mac_roman_byte(0x0100), None);
    assert_eq!(mac_roman_byte('Ğ' as u32), None);
}

// Cut to the records its readers take, a name table reads as the whole one: each ID alone and all together, in
// fixtures naming each ID on two platforms.
#[test]
fn a_table_cut_to_the_records_read_reads_alike() {
    let names = |map: &Tables| {
        (parse_all_name_strings::<u16>(map), (0..=320).map(|id| read_name_string(map, id)).collect::<Vec<_>>())
    };
    for rel in ["stix-two-math/STIX2Math.otf", "colr-v1-test-glyphs/test_glyphs_variable.ttf",
        "structure/TestShapeLana.ttf", "structure/TestSFNTTwo.ttf"]
    {
        let bytes = std::fs::read(format!("{}/assets/test-fonts/{rel}", env!("CARGO_MANIFEST_DIR"))).expect("fixture");
        let mut map = super::super::extract_ttf_tables(&bytes).expect("parses");
        let whole = names(&map);
        let lean = read_records_only(&map["name"]).expect("a table with more records than IDs is cut");
        assert!(lean.len() < map["name"].len(), "{rel}");
        map.insert("name", TableBytes::from_vec(lean));
        assert_eq!(names(&map), whole, "{rel}");
    }
}

// Records sharing one long string would run out of a cut table's budget sooner, so such a table stays whole; one
// holding a single record an ID stays whole too, as cutting it saves nothing.
#[test]
fn a_table_cut_down_would_read_differently_stays_whole() {
    let key = |platform, encoding, language, name_id| NameKey { platform, encoding, language, name_id };
    let shared = encode_name(key(3, 1, 0x409, 0), &"x".repeat(400)).expect("encodes");
    let own: Vec<Vec<u8>> = (0..10u16).map(|id| alloc::format!("{id:>200}").into_bytes()).collect();
    let mac: Vec<(NameKey, &[u8])> = (0..10u16).map(|id| (key(1, 0, 0, id), &own[usize::from(id)][..])).collect();
    let alone = write_name_table(mac.clone(), &[]).expect("writes");
    assert_eq!(read_records_only(&alone), None, "nothing to cut");
    let mut records: Vec<(NameKey, &[u8])> = (0..10u16).map(|id| (key(3, 1, 0x409, id), &shared[..])).collect();
    records.extend(mac);
    let both = write_name_table(records, &[]).expect("writes");
    assert_eq!(read_records_only(&both), None, "a budget a cut table would run out of");
    let map = Tables::from([("name", TableBytes::from_vec(both))]);
    assert_eq!(parse_all_name_strings::<u16>(&map).len(), 10, "the whole table reads every ID");
}

// Names read on asking read as every name read at once, as do those of the IDs up to one alone: each fixture ID, and
// a table whose records share one long string, so the decoding budget runs out partway, at the same ID.
#[test]
fn a_name_read_on_asking_reads_as_among_them_all() {
    let alike = |map: &Tables, ids: core::ops::RangeInclusive<u16>| {
        let (all, records) = (parse_all_name_strings::<u16>(map), NameRecords::new(map));
        ids.for_each(|id| {
            assert_eq!(records.get(id), all.get(&id).cloned(), "name {id}");
            assert_eq!(NameRecords::upto(map, id).get(id), all.get(&id).cloned(), "name {id} alone");
        });
        all.len()
    };
    for rel in ["stix-two-math/STIX2Math.otf", "colr-v1-test-glyphs/test_glyphs_variable.ttf",
        "structure/TestShapeLana.ttf", "inter/InterVariable.ttf"]
    {
        let bytes = std::fs::read(format!("{}/assets/test-fonts/{rel}", env!("CARGO_MANIFEST_DIR"))).expect("fixture");
        alike(&super::super::extract_ttf_tables(&bytes).expect("parses"), 0..=400);
    }
    let key = |name_id| NameKey { platform: 3, encoding: 1, language: 0x409, name_id };
    let shared = encode_name(key(0), &"x".repeat(400)).expect("encodes");
    let records = (0..20u16).map(|id| (key(id), &shared[..]));
    let map = Tables::from([("name", TableBytes::from_vec(write_name_table(records.collect(), &[]).expect("writes")))]);
    let read = alike(&map, 0..=20);
    assert!(read > 0 && read < 20, "the budget runs out after {read} of 20");
}

// A name reads once per language, by BCP 47 tag: Windows' and Mac's IDs from their tables, a format 1 table's own tag,
// `und` for an ID no table names and a Unicode record, Windows' record over Mac's for one language, sorted by tag.
#[test]
fn a_name_reads_in_each_language_it_is_recorded_in() {
    let key = |platform, encoding, language| NameKey { platform, encoding, language, name_id: 1 };
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    let (en, ja, es, other, plain) = (utf16("Sample"), utf16("見本"), utf16("Muestra"), utf16("Other"), utf16("Plain"));
    let records = alloc::vec![
        (key(3, 1, 0x0409), &en[..]), (key(1, 0, 0), &b"Sample Mac"[..]), (key(1, 0, 1), &b"Exemple"[..]),
        (key(3, 1, 0x0411), &ja[..]), (key(3, 1, 0x8000), &es[..]), (key(3, 1, 0x007B), &other[..]),
        (key(0, 3, 0), &plain[..]), (key(1, 0, 38), &b"Cesky"[..]),
    ];
    let tag = utf16("es-419");
    let table = write_name_table(records, &[&tag[..]]).expect("writes");
    let map = Tables::from([("name", TableBytes::from_vec(table))]);
    let names = read_localized_names(&map, 1);
    let got: Vec<(&str, &str)> = names.iter().map(|n| (n.language.as_str(), n.name.as_str())).collect();
    let want =
        [("cs", "Cesky"), ("en", "Sample"), ("es-419", "Muestra"), ("fr", "Exemple"), ("ja", "見本"), ("und", "Other")];
    assert_eq!(got, want);
    assert!(read_localized_names(&map, 2).is_empty(), "an ID with no record read a name");
}

// fsSelection's oblique bit wins over its italic bit when both are set; head's macStyle marks italic alone.
#[test]
fn a_style_is_oblique_before_italic() {
    let style = |selection: u16, mac_style: u16| {
        let (mut os2, mut head) = (alloc::vec![0u8; 64], alloc::vec![0u8; 54]);
        os2[62..].copy_from_slice(&selection.to_be_bytes());
        head[44..46].copy_from_slice(&mac_style.to_be_bytes());
        let table = |(tag, bytes): (&str, Vec<u8>)| (String::from(tag), TableBytes::from(bytes));
        let tables: Tables = [("OS/2", os2), ("head", head)].into_iter().map(table).collect();
        read_font_style(&tables)
    };
    assert_eq!(style(0, 0), FontStyle::Normal);
    assert_eq!((style(0x0001, 0), style(0, 0x0002)), (FontStyle::Italic, FontStyle::Italic));
    assert_eq!((style(0x0200, 0), style(0x0201, 0x0002)), (FontStyle::Oblique, FontStyle::Oblique));
}

// 2,000 records all naming one string of 65,534 bytes: what's reserved and decoded stays within four times the
// table, not 2,000 times the string.
#[test]
fn names_sharing_one_long_string_reserve_within_the_budget() {
    let n = 2_000;
    let mut t = [&[0u8, 0][..], &(n as u16).to_be_bytes(), &((6 + 12 * n) as u16).to_be_bytes()].concat();
    for id in 0..n as u16 {
        [3u16, 1, 0x409, id, 65_534, 0].iter().for_each(|v| t.extend(v.to_be_bytes()));
    }
    t.extend(alloc::vec![0u8; 65_534]);
    let len = t.len();
    let mut names = NameText::default();
    each_name_string(&Tables::from([("name", TableBytes::from_vec(t))]), &mut names);
    assert!(names.text.capacity() <= 4 * len && names.text.len() <= 4 * len);
}

// 1,000 languages' records of one name all naming one string of 32,767 CJK characters: decoding stops within four
// times the table, as for every name.
#[test]
fn localized_names_sharing_one_long_string_decode_within_the_budget() {
    let n = 1_000;
    let storage: Vec<u8> = (0..32_767u16).flat_map(|i| (0x4E00 + i).to_be_bytes()).collect();
    let mut t = [&[0u8, 1][..], &(n as u16).to_be_bytes(), &((6 + 12 * n + 2 + 4 * n) as u16).to_be_bytes()].concat();
    for i in 0..n as u16 {
        [3u16, 1, 0x8000 + i, 1, 65_534, 0].iter().for_each(|v| t.extend(v.to_be_bytes()));
    }
    t.extend((n as u16).to_be_bytes());
    (0..n as u16).for_each(|i| [2u16, 2 * i].iter().for_each(|v| t.extend(v.to_be_bytes())));
    t.extend(storage);
    let len = t.len();
    let names = read_localized_names(&Tables::from([("name", TableBytes::from_vec(t))]), 1);
    let bytes: usize = names.iter().map(|n| n.name.len()).sum();
    assert!(!names.is_empty() && bytes <= 6 * len, "{bytes} bytes from a table of {len}");
}

// A format 1 table whose `n` records of one name each take a language tag of 2,000 characters, one character apart in
// one run: the tags decoded, two bytes a character as stored, stay within four times the table.
#[test]
fn language_tags_over_one_long_string_decode_within_the_budget() {
    let tagged = |n: u16| {
        let mut t = [&[0u8, 1][..], &n.to_be_bytes(), &(6 + 12 * n + 2 + 4 * n).to_be_bytes()].concat();
        for i in 0..n {
            [3u16, 1, 0x8000 + i, 1, 2, 0].iter().for_each(|v| t.extend(v.to_be_bytes()));
        }
        t.extend(n.to_be_bytes());
        (0..n).for_each(|i| [4_000u16, 2 * i].iter().for_each(|v| t.extend(v.to_be_bytes())));
        t.extend((0..2_000 + n).flat_map(|i| u16::from(b'a' + (i % 26) as u8).to_be_bytes()));
        let len = t.len();
        let names = read_localized_names(&Tables::from([("name", TableBytes::from_vec(t))]), 1);
        (names.iter().map(|n| n.language.len()).sum::<usize>(), names.len(), len)
    };
    let (_, three, _) = tagged(3);
    assert_eq!(three, 3);
    let (bytes, _, len) = tagged(100);
    assert!(bytes <= 2 * len, "{bytes} bytes of tags from a table of {len}");
}
