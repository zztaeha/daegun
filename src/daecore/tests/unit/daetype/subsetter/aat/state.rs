use super::*;
use crate::daecore::daetype::format::aat::StateTable;

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// A format 0 class lookup for four glyphs whose state array starts at its third value, so glyphs
// 2 and 3 take their classes from the first two cells. The shaper reads them, and so does a subset.
#[test]
fn a_class_lookup_reaching_into_the_state_array_is_read_as_the_shaper_reads_it() {
    let mut t = [5u32, 16, 22, 42].map(u32::to_be_bytes).concat();
    t.extend(words(&[0, 4, 4]));
    t.extend(words(&[4, 4, 0, 0, 0, 0, 0, 0, 0, 0]));
    t.extend(words(&[0; 10]));
    let mut active = GlyphSet::new();
    (0..4).for_each(|g| { active.insert(g); });
    let mut work = usize::MAX;
    let subset = subset_state_table(&t, &[], &active, &[0, 1, 2, 3], 4, &mut work).expect("a subset");
    let classes = |table: &[u8]| {
        let table = StateTable::parse(table, 0, 4).expect("a state table");
        (0..4).map(|g| table.class(g)).collect::<Vec<_>>()
    };
    assert_eq!(classes(&subset), classes(&t));
}
