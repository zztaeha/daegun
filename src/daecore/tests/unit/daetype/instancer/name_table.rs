use super::*;

fn utf16(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_be_bytes).collect()
}

// A record keyed (platform, encoding, language, name ID), and its string.
type Record = ((u16, u16, u16, u16), Vec<u8>);

// A name table of `records` and, when there are any, format 1's language tags, each string stored
// in turn.
fn table(records: &[Record], tags: &[&str]) -> Vec<u8> {
    let header = 6 + 12 * records.len() + if tags.is_empty() { 0 } else { 2 + 4 * tags.len() };
    let words = |w: &[usize]| w.iter().flat_map(|&v| (v as u16).to_be_bytes()).collect::<Vec<u8>>();
    let mut out = words(&[usize::from(!tags.is_empty()), records.len(), header]);
    let mut strings = Vec::new();
    for &((platform, encoding, language, id), ref s) in records {
        out.extend(words(&[platform.into(), encoding.into(), language.into(), id.into(), s.len(), strings.len()]));
        strings.extend(s);
    }
    if !tags.is_empty() {
        out.extend(words(&[tags.len()]));
        for tag in tags.iter().map(|t| utf16(t)) {
            out.extend(words(&[tag.len(), strings.len()]));
            strings.extend(tag);
        }
    }
    out.extend(strings);
    out
}

fn read(name: &[u8]) -> Vec<Record> {
    name_records(name).map(|(k, s)| ((k.platform, k.encoding, k.language, k.name_id), s.to_vec())).collect()
}

// Windows' ShiftJIS can't hold the update, so its English record goes with the old name.
#[test]
fn an_update_drops_the_english_records_it_cannot_encode() {
    let name = table(&[((3, 1, 0x409, 1), utf16("Old")), ((3, 2, 0x409, 1), alloc::vec![0x82, 0xA0])], &[]);
    let out = rewrite_name_table(&name, &[(1, "New".into())], &[]).expect("a name table");
    assert_eq!(read(&out), [((3, 1, 0x409, 1), utf16("New"))]);
}

// Language 0x8009 names the tenth language tag, here Japanese, not an English LCID.
#[test]
fn a_record_in_a_language_tag_is_not_english() {
    let tags = ["en", "fr", "de", "es", "it", "nl", "pt", "ko", "zh", "ja"];
    let name = table(&[((3, 1, 0x409, 1), utf16("Old")), ((3, 1, 0x8009, 1), utf16("古い"))], &tags);
    let out = rewrite_name_table(&name, &[(1, "New".into())], &[]).expect("a name table");
    assert_eq!(read(&out), [((3, 1, 0x409, 1), utf16("New"))]);
    assert_eq!(lang_tags(&out).len(), 10);
}

// A count past the table's end gives the records that fit, and a tag string past it reads as
// empty, so the instance is renamed rather than left with the variable font's names.
#[test]
fn a_table_cut_short_is_renamed_where_it_reads() {
    let mut name = table(&[((3, 1, 0x409, 1), utf16("A")), ((3, 1, 0x409, 2), utf16("B"))], &[]);
    name[2..4].copy_from_slice(&3u16.to_be_bytes());
    let out = rewrite_name_table(&name, &[(1, "New".into())], &[]).expect("a name table");
    assert_eq!(read(&out), [((3, 1, 0x409, 1), utf16("New")), ((3, 1, 0x409, 2), utf16("B"))]);

    let mut name = table(&[((3, 1, 0x409, 1), utf16("A"))], &["en"]);
    name[22..24].copy_from_slice(&0xFFu16.to_be_bytes());
    let out = rewrite_name_table(&name, &[(1, "New".into())], &[]).expect("a name table");
    assert_eq!(read(&out), [((3, 1, 0x409, 1), utf16("New"))]);
    assert_eq!(lang_tags(&out), [&[] as &[u8]]);
}
