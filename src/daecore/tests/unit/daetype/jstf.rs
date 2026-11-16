use super::*;
use alloc::string::String;
use crate::daecore::daetype::TableBytes;

// A JSTF of the one script `tag`, with no language records and a default of `levels` priority
// levels that all name one level enabling nothing.
fn jstf(tag: &[u8; 4], levels: u16) -> Tables {
    let mut t: Vec<u8> = [1u16, 0, 1].iter().flat_map(|v| v.to_be_bytes()).collect();
    t.extend(tag);
    t.extend([12u16, 0, 6, 0, levels].iter().flat_map(|v| v.to_be_bytes()));
    (0..levels).for_each(|_| t.extend((2 + 2 * levels).to_be_bytes()));
    t.extend([0u8; 20]);
    [(String::from("JSTF"), TableBytes::from(t))].into_iter().collect()
}

// The spec's default applies in the absence of language data, so a language asked for by name that
// has none takes the script's default levels.
#[test]
fn a_language_without_data_takes_the_scripts_default() {
    let map = jstf(b"latn", 1);
    assert_eq!(jstf_priorities(&map, *b"latn", Some(*b"ENG ")).map(|l| l.len()), Some(1));
}

// A JSTF of another major version is skipped: it names no script.
#[test]
fn a_jstf_of_another_major_version_is_skipped() {
    let mut t = jstf(b"latn", 1).get("JSTF").expect("a JSTF").to_vec();
    t[1] = 2;
    let map = [(String::from("JSTF"), TableBytes::from(t))].into_iter().collect();
    assert_eq!(jstf_priorities(&map, *b"latn", None), None);
}

// The empty tag is four spaces, which a record of other bytes never matches.
#[test]
fn an_empty_tag_names_no_script() {
    assert_eq!(jstf_priorities(&jstf(&[0xFF; 4], 1), *b"    ", None), None);
}

// A JSTF 1.1 naming 'latn' in both lists: scriptList's script extends glyph 5 alone; scriptList2's extends 7 and
// 65,537 in 24 bits, has a default of no levels and an 'ENG ' of one, each at a 32-bit or 16-bit offset.
fn version_1_1() -> Vec<u8> {
    [
        &[0, 1, 0, 1, 0, 1, b'l', b'a', b't', b'n', 0, 22][..],
        &[0, 1, b'l', b'a', b't', b'n', 0, 0, 0, 32],
        &[0, 6, 0, 0, 0, 0, 0, 1, 0, 5],
        &[0, 0, 0, 16, 0, 0, 0, 24, 0, 1, b'E', b'N', b'G', b' ', 0, 26],
        &[0, 2, 0, 0, 7, 1, 0, 1, 0, 0, 0, 1, 0, 4],
        &[0; 20],
    ]
    .concat()
}

// Version 1.1 reads scriptList2 where it names a script; 1.0, or an empty scriptList2, reads scriptList.
#[test]
fn version_1_1_reads_its_scripts_from_script_list_2() {
    let read = |t: &[u8]| {
        let map = [(String::from("JSTF"), TableBytes::from(t.to_vec()))].into_iter().collect();
        let levels = |lang| jstf_priorities(&map, *b"latn", lang).map(|l| l.len());
        (jstf_extender_glyphs(&map, *b"latn"), levels(None), levels(Some(*b"ENG ")))
    };
    let mut t = version_1_1();
    assert_eq!(read(&t), (Some(alloc::vec![7, 0x1_0001]), Some(0), Some(1)));
    t[3] = 0;
    assert_eq!(read(&t), (Some(alloc::vec![5]), None, None));
    t[3] = 1;
    t[13] = 0;
    assert_eq!(read(&t), (Some(alloc::vec![5]), None, None));
}

fn extenders(t: &[u8]) -> Option<Vec<u32>> {
    let map = [(String::from("JSTF"), TableBytes::from(t.to_vec()))].into_iter().collect();
    jstf_extender_glyphs(&map, *b"latn")
}

// scriptList2's records are 8 bytes, a tag and an Offset32: the second of two is found.
#[test]
fn script_list_2_records_are_8_bytes() {
    let script = |g: u8| [&[0, 0, 0, 10, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0][..], &[g]].concat();
    let list = [0, 1, 0, 1, 0, 0, 0, 2, b'a', b'r', b'a', b'b', 0, 0, 0, 24, b'l', b'a', b't', b'n', 0, 0, 0, 39];
    assert_eq!(extenders(&[&list[..], &script(3), &script(9)].concat()), Some(alloc::vec![9]));
}

// A 1.1 table ending before its scriptList2 has none, an absent structure, so its 1.0 list is read as at 1.0:
// 'latn' at 12, its extenders at 14 naming glyph 5.
#[test]
fn a_script_list_2_past_the_table_leaves_the_1_0_list() {
    let mut t = [0, 1, 0, 1, 0, 2, b'l', b'a', b't', b'n', 0, 12, 0, 2, 0, 1, 0, 5];
    assert_eq!(extenders(&t), Some(alloc::vec![5]));
    t[3] = 0;
    assert_eq!(extenders(&t), Some(alloc::vec![5]));
}

// A record whose offset is NULL names no script, where the header would read as a script of 256 extenders.
#[test]
fn a_null_script_offset_names_no_script() {
    let mut t = alloc::vec![0, 1, 0, 0, 0, 1, b'l', b'a', b't', b'n', 0, 0];
    t.resize(600, 0);
    assert_eq!(extenders(&t), None);
}

// Every level a font names is read, past 64 too, 30,000 of them naming one table.
#[test]
fn every_level_is_read() {
    assert_eq!(jstf_priorities(&jstf(b"latn", 65), *b"latn", None).map(|l| l.len()), Some(65));
    assert_eq!(jstf_priorities(&jstf(b"latn", 30_000), *b"latn", None).map(|l| l.len()), Some(30_000));
}
