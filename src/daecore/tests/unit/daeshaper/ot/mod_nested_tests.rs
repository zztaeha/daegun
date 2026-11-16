use super::*;
use alloc::vec::Vec;

// A GSUB of one lookup, a Context 6 whose record names lookup 1.
fn context_6() -> Vec<u8> {
    let words = [1u16, 0, 0, 0, 10, 1, 4, 5, 0, 1, 8, 6, 1, 1, 0, 0, 1].map(u16::to_be_bytes).concat();
    [&words[..30], &[0], &words[30..]].concat()
}

// A 24-bit context names its records' lookups for the plan to index, as a 16-bit one does: Context 6 naming 1.
#[test]
fn a_24_bit_context_lists_the_lookups_its_records_name() {
    let gsub = context_6();
    let t = LayoutTable::parse(&gsub).expect("a GSUB");
    let kinds = crate::daecore::daeshaper::ot::gsub::KINDS;
    let lookup = t.lookup(0).expect("lookup 0");
    let index = lookup_index(&lookup, &kinds, false, |_, _| None, |_, _| None, |_| true, &mut { usize::MAX });
    assert_eq!(&*index.nested, [1]);
}

// The lookups a context's records name are kept as far as the face's allowance draws them, two bytes each, and past
// it none are: shaping then tells each apart where a record runs it.
#[test]
fn a_lookups_nested_list_is_drawn_from_the_allowance() {
    let gsub = context_6();
    let t = LayoutTable::parse(&gsub).expect("a GSUB");
    let (kinds, lookup) = (crate::daecore::daeshaper::ot::gsub::KINDS, t.lookup(0).expect("lookup 0"));
    let mut drawn = 0;
    let draw = |bytes| {
        drawn += bytes;
        true
    };
    let index = lookup_index(&lookup, &kinds, false, |_, _| None, |_, _| None, draw, &mut { usize::MAX });
    assert_eq!((&*index.nested, drawn), (&[1][..], 2));
    let refused = lookup_index(&lookup, &kinds, false, |_, _| None, |_, _| None, |_| false, &mut { usize::MAX });
    assert!(refused.nested.is_empty());
}
