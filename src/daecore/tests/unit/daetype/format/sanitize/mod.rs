use super::*;
use alloc::vec;

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn u32s(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn reads(which: AatTable, table: &[u8]) -> bool {
    shapes_with(which, |tag| (tag == which.tag()).then_some(table))
}

// morx chains of 16 bytes that each count every subtable there is, so each reads the rest of the table, each later
// chain's header as a subtable: 64 of them yield 2,016 where their 1,032 bytes hold 86. Past twice that, morx isn't
// read; a chain reading one subtable past its end still is.
#[test]
fn a_morx_whose_chains_read_each_other_is_not_read() {
    let chains = |c: u32| [u16s(&[2, 0]), u32s(&[c]), u32s(&[16, 16, 0, u32::MAX]).repeat(c as usize)].concat();
    assert!(!reads(AatTable::Morx, &chains(64)), "chains reading each other are read");
    let overrun = [u16s(&[2, 0]), u32s(&[2, 1, 28, 0, 2, 12, 4, 1, 28, 28, 0, 1, 12, 4, 1])].concat();
    assert!(reads(AatTable::Morx, &overrun), "a chain reading one past its end isn't read");
}

// A header's version and length decide; what follows it is read, or found absent, by the table's own reader.
#[test]
fn aat_tables_are_read_at_the_versions_the_manual_defines() {
    let morx = |version: u16| [u16s(&[version, 0]), u32s(&[1])].concat();
    assert_eq!([1, 2, 3, 4].map(|v| reads(AatTable::Morx, &morx(v))), [false, true, true, false]);
    assert!(!reads(AatTable::Morx, &morx(2)[..7]), "a header cut short");
    assert!(reads(AatTable::Morx, &[morx(2), vec![0xFF; 3]].concat()), "a chain past the table");
    assert_eq!([1, 2, 3, 4, 5].map(|v| reads(AatTable::Kerx, &morx(v))), [false, true, true, true, false]);

    let ot = |count: u16| u16s(&[0, count]);
    let apple = |count: u32| [u32s(&[0x0001_0000]), u32s(&[count])].concat();
    assert!(reads(AatTable::Kern, &ot(1)) && reads(AatTable::Kern, &apple(1)));
    assert!(!reads(AatTable::Kern, &ot(0)) && !reads(AatTable::Kern, &apple(0)), "no subtables");
    assert!(!reads(AatTable::Kern, &u16s(&[2, 1])), "another version");
    assert!(!reads(AatTable::Kern, &u16s(&[1, 1, 0, 1])), "Apple's version 1.1");

    let ankr = |version: u16| [u16s(&[version, 0]), u32s(&[0, 12])].concat();
    assert!(reads(AatTable::Ankr, &ankr(0)) && !reads(AatTable::Ankr, &ankr(1)));
    assert!(reads(AatTable::Ankr, &[u16s(&[0, 0]), u32s(&[0x100, 0x100])].concat()), "parts past the table");
}

#[test]
fn layout_tables_are_read_at_major_1_with_their_header() {
    let gsub = |major: u16, minor: u16| u16s(&[major, minor, 0, 0, 0]);
    assert!(otl_passes(OtlTable::Gsub, &gsub(1, 0)) && otl_passes(OtlTable::Gpos, &gsub(1, 7)));
    assert!(!otl_passes(OtlTable::Gsub, &gsub(2, 0)), "another major");
    assert!(!otl_passes(OtlTable::Gsub, &gsub(1, 0)[..9]), "a header cut short");
    assert!(otl_passes(OtlTable::Gsub, &u16s(&[1, 1, 0xFF, 0xFF, 0xFF])), "lists past the table, and 1.1's offset");
    assert!(otl_passes(OtlTable::Gdef, &u16s(&[1, 3, 0, 0, 0, 0])), "1.3's fields past the table");
    assert!(!otl_passes(OtlTable::Gdef, &u16s(&[1, 0, 0, 0, 0])), "a header cut short");
}

#[test]
fn color_tables_are_read_at_versions_0_and_1() {
    let colr = |version: u16, len: usize| {
        let mut t = vec![0; len];
        t[..2].copy_from_slice(&version.to_be_bytes());
        t
    };
    assert_eq!([(0, 14), (1, 34), (2, 34)].map(|(v, len)| colr_passes(&colr(v, len))), [true, true, false]);
    assert!(!colr_passes(&colr(0, 13)) && !colr_passes(&colr(1, 33)), "a header cut short");
    let mut far = colr(1, 34);
    far[14..34].fill(0xFF);
    assert!(colr_passes(&far), "lists past the table");
    assert_eq!([0, 1, 2].map(|v| cpal_passes(&colr(v, 12))), [true, true, false]);
    assert!(!cpal_passes(&colr(0, 11)));
    let varc = |major: u16| [u16s(&[major, 0]), vec![0; 20]].concat();
    assert!(varc_passes(&varc(1)) && !varc_passes(&varc(2)) && !varc_passes(&varc(1)[..23]));
}

#[test]
fn feature_names_and_language_tags_are_read_at_version_1() {
    let feat = |major: u16| [u16s(&[major, 0]), vec![0; 8]].concat();
    assert!(feat_passes(&feat(1)) && !feat_passes(&feat(2)) && !feat_passes(&feat(1)[..11]));
    let ltag = |version: u32| [u32s(&[version]), vec![0; 8]].concat();
    assert!(ltag_passes(&ltag(1)) && !ltag_passes(&ltag(2)) && !ltag_passes(&ltag(1)[..11]));
}

// An anchor is read where its format's fields lie in the data its offset is taken from; one of an unknown format is
// skipped, and its devices are left to their own reader.
#[test]
fn an_anchor_is_read_where_its_own_fields_lie_in_the_table() {
    let anchor = |words: &[u16]| [u16s(&[0, 0]), u16s(words)].concat();
    assert!(anchor_fits(&anchor(&[1, 10, 20]), 4));
    assert!(!anchor_fits(&anchor(&[1, 10]), 4), "cut short");
    assert!(anchor_fits(&anchor(&[2, 10, 20, 3]), 4) && !anchor_fits(&anchor(&[2, 10, 20]), 4));
    assert!(anchor_fits(&anchor(&[3, 10, 20, 0xFF, 0xFF]), 4), "devices past the table");
    assert!(!anchor_fits(&anchor(&[4, 10, 20, 0, 0]), 4), "an unknown format");
    assert!(!anchor_fits(&anchor(&[1, 10, 20]), 100), "an offset past the table");
}

// An extended state table of 4 classes, its header then rows of `rows`, then entries of 2 words each.
fn machine(rows: &[[u16; 4]], entries: &[[u16; 2]]) -> Vec<u8> {
    let entries_at = 16 + 8 * rows.len() as u32;
    let rows: Vec<u8> = rows.iter().flat_map(|r| u16s(r)).collect();
    let entries: Vec<u8> = entries.iter().flat_map(|e| u16s(e)).collect();
    [u32s(&[4, 0, 16, entries_at]), rows, entries].concat()
}

// From state 0 the sweep follows each row's entries to the states they lead to, until nothing new is reached.
#[test]
fn a_state_table_s_reach_is_what_state_0_leads_to() {
    let chain = machine(&[[0, 0, 0, 1], [0, 0, 2, 0], [0; 4], [3; 4]], &[[0, 0], [1, 0], [2, 0], [3, 0]]);
    assert_eq!(state_reach(&chain, 4), Some((3, 3)), "row 3 and entry 3 left unreached");
    let ring = machine(&[[1; 4], [0; 4]], &[[0, 0], [1, 0]]);
    assert_eq!(state_reach(&ring, 4), Some((2, 2)));
    assert_eq!(state_reach(&machine(&[[0; 4]], &[[0, 0]])[..15], 4), None, "a header cut short");
    assert_eq!(state_reach(&[u32s(&[3, 0, 16, 24]), vec![0; 12]].concat(), 4), None, "fewer than 4 classes");
}

// A row or entry past the table is absent, and so are the states only it leads to.
#[test]
fn a_state_table_s_reach_stops_at_the_table_s_end() {
    let past = machine(&[[0, 0, 0, 1]], &[[0, 0], [5, 0]]);
    assert_eq!(state_reach(&past, 4), Some((2, 2)), "state 5 past the 2 rows the table's 16 bytes from 16 hold");
    let cut = machine(&[[0, 0, 0, 1]], &[[0, 0], [0, 0]]);
    assert_eq!(state_reach(&cut[..cut.len() - 1], 4), Some((1, 1)), "entry 1 cut short");
    assert_eq!(state_reach(&u32s(&[4, 0, 16, 16]), 4), Some((0, 0)), "no row at all");
}
