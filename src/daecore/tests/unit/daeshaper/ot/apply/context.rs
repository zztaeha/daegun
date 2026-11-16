use alloc::vec::Vec;

// Each form's rules with their records, null coverages and a null rule set; a chain whose last record is cut off,
// and a format no context has.
#[test]
fn each_context_form_names_the_lookups_of_its_records() {
    let be = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let cases: [(&[u16], bool, &[u16]); 8] = [
        (&[1, 0, 2, 10, 0, 1, 4, 2, 2, 9, 0, 7, 1, 7], false, &[7, 7]),
        (&[2, 0, 0, 1, 10, 1, 4, 1, 1, 0, 3], false, &[3]),
        (&[3, 2, 2, 0, 0, 0, 4, 1, 5], false, &[4, 5]),
        (&[1, 0, 1, 8, 1, 4, 1, 9, 2, 9, 1, 9, 1, 0, 6], true, &[6]),
        (&[2, 0, 0, 0, 0, 1, 14, 1, 4, 0, 1, 0, 1, 0, 2], true, &[2]),
        (&[3, 1, 0, 1, 0, 2, 0, 0, 2, 0, 8, 0, 1], true, &[8, 1]),
        (&[3, 1, 0, 1, 0, 2, 0, 0, 3, 0, 8, 0, 1, 0], true, &[8, 1]),
        (&[7, 1, 1, 0, 9], false, &[]),
    ];
    for (i, (words, chain, want)) in cases.into_iter().enumerate() {
        let mut named = Vec::new();
        super::record_lookups(&be(words), chain, &mut { usize::MAX }, |index| named.push(index));
        assert_eq!(named, want, "case {i}");
    }
}

// The 24-bit forms, each a rule naming one lookup: Context 4 at 16-bit offsets, ChainContext 4 at 24-bit with a
// backtrack and lookahead, Context 5 and ChainContext 5 class rules at 24-bit, Context 6 records past 24-bit coverage.
#[test]
fn each_24_bit_context_form_names_the_lookups_of_its_records() {
    let u16s = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let u24s = |v: &[u32]| v.iter().flat_map(|v| v.to_be_bytes()[1..].to_vec()).collect::<Vec<u8>>();
    let cases: [(Vec<u8>, bool, u16); 5] = [
        ([u16s(&[4, 0, 0]), u24s(&[1, 12]), u16s(&[1, 4, 2, 1]), u24s(&[9]), u16s(&[1, 7])].concat(), false, 7),
        ([u16s(&[4, 0, 0]), u24s(&[1, 12]), u16s(&[1]), u24s(&[5]), u16s(&[1]), u24s(&[9]), u16s(&[2]), u24s(&[8]),
            u16s(&[1]), u24s(&[7]), u16s(&[1, 0, 6])].concat(), true, 6),
        ([u16s(&[5, 0, 0, 0, 0]), u24s(&[1, 16]), u16s(&[1]), u24s(&[5]), u16s(&[2, 1, 3, 1, 4])].concat(),
            false, 4),
        ([u16s(&[5, 0, 0, 0, 0, 0, 0, 0, 0, 1]), u24s(&[23]), u16s(&[1]), u24s(&[5]), u16s(&[0, 2, 3, 0, 1, 0, 5])]
            .concat(), true, 5),
        ([u16s(&[6, 2, 1]), u24s(&[0, 0]), u16s(&[1, 3])].concat(), false, 3),
    ];
    for (i, (bytes, chain, want)) in cases.into_iter().enumerate() {
        let mut named = Vec::new();
        super::record_lookups(&bytes, chain, &mut { usize::MAX }, |index| named.push(index));
        assert_eq!(named, [want], "case {i}");
    }
}

fn places(len: usize, end: usize, at: &[u32]) -> super::Sequence {
    let mut places = [0; super::SEQUENCE_LIMIT];
    places[..at.len()].copy_from_slice(at);
    super::Sequence { places, len, end }
}

// Multiplying input glyph 1 (place 3) into three adds two places after it and moves the rest on; ligating the two
// after it takes their places out and moves the rest back.
#[test]
fn an_input_moves_with_what_a_nested_lookup_adds_or_takes() {
    let mut grown = places(3, 9, &[1, 3, 8]);
    assert!(grown.moved(1, 2));
    assert_eq!((&grown.places[..grown.len], grown.end), (&[1, 3, 4, 5, 10][..], 11));
    let mut shrunk = places(5, 9, &[1, 3, 4, 5, 8]);
    assert!(shrunk.moved(1, -2));
    assert_eq!((&shrunk.places[..shrunk.len], shrunk.end), (&[1, 3, 6][..], 7));
    // A ligature reaching past the input's end leaves the end at its glyph.
    let mut past = places(3, 6, &[1, 3, 5]);
    assert!(past.moved(1, -5));
    assert_eq!((&past.places[..past.len], past.end), (&[1, 3][..], 3));
    let mut full = places(super::SEQUENCE_LIMIT, 70, &[0; 64]);
    assert!(!full.moved(0, 1));
}

// A chained rule lists its backtrack, input after the first glyph, lookahead and records, each counted before it.
#[test]
fn a_chained_rule_reads_its_lists_in_order() {
    let be = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let bytes = be(&[2, 7, 8, 3, 4, 5, 1, 9, 1, 2, 6]);
    let rule = super::Rule::<2>::read(&bytes, true);
    let list = |l: &super::ValueList<2>| (0..usize::from(l.len)).map_while(|i| l.get(i)).collect::<Vec<_>>();
    let lists = (list(&rule.back), rule.count, list(&rule.input), list(&rule.ahead));
    assert_eq!(lists, (vec![7, 8], 3, vec![4, 5], vec![9]));
    assert_eq!(rule.records.get(0), Some((2, 6)));
    assert_eq!(super::first_read::<2>(&bytes, true), Some(super::FirstRead::Input(4)));
    let one_input = be(&[0, 1, 1, 9, 0]);
    assert_eq!(super::first_read::<2>(&one_input, true), Some(super::FirstRead::Ahead(9)));
}

// A first read goes into the plan's word and comes back as it was, for every 16-bit value and for none.
#[test]
fn a_first_read_packs_into_a_word_and_back() {
    use super::FirstRead::{Ahead, Input};
    for first in [0, 1, 0x7FFF, 0xFFFF].into_iter().flat_map(|v| [Some(Input(v)), Some(Ahead(v))]).chain([None]) {
        assert_eq!(super::FirstRead::unpack(super::FirstRead::pack(first)), first);
    }
}

// A chain format 3's header reads whole or not at all: lists running past the subtable, or a count cut short, give
// none. Records past its end are left to be refused one by one as a match reads them.
#[test]
fn a_chain_format_3s_lists_read_whole_or_not_at_all() {
    let lists = |words: &[u16], len: usize| {
        let bytes = words.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
        super::chain_lists(&bytes[..len])
    };
    assert_eq!(lists(&[3, 1, 0, 1, 0, 2, 0, 0, 2, 0, 8, 0, 1], 26), Some([1, 1, 2, 2]));
    assert_eq!(lists(&[3, 0, 0, 0, 0], 10), Some([0, 0, 0, 0]));
    assert_eq!(lists(&[3, 0, 0, 0, 5], 10), Some([0, 0, 0, 5]));
    assert_eq!(lists(&[3, 0, 0, 0, 0], 9), None);
    assert_eq!(lists(&[3, 0, 0, 2, 0], 10), None);
}
