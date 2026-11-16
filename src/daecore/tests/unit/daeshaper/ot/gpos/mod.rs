use alloc::vec::Vec;

use crate::daecore::daeshaper::face::Deltas;

// One axis, one region peaking at its maximum, and rows of -125, 125 and 0: halfway, deltas of -62.5, 62.5 and 0.
pub(super) fn halfway() -> Deltas {
    let be = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let store = be(&[1, 0, 12, 1, 0, 22, 1, 1, 0, 0x4000, 0x4000, 3, 1, 1, 0, -125i16 as u16, 125, 0]);
    let store = crate::daecore::daetype::format::ivs::ItemVariationStore::parse(&store, 0).expect("a store");
    Deltas::at(crate::daecore::daekit::base::sync::Shared::new(store), &[0.5]).expect("off the default")
}

#[test]
fn a_device_delta_rounds_half_up() {
    let deltas = halfway();
    assert_eq!([0, 1, 2].map(|inner| deltas.of(0, inner)), [-62, 63, 0]);
}

// A record's advance device names a variation index: at a location it adds the delta, and the field moves the
// glyph even where that delta is 0; at the default only the value counts.
#[test]
fn a_device_at_a_location_moves_its_field_whatever_its_delta() {
    let deltas = halfway();
    let format = super::X_ADVANCE | 0x0040;
    let varied = |inner: u8| {
        let data = [0, 0, 0, 4, 0, 0, 0, inner, 0x80, 0];
        super::with_devices(&data, 0, format, &deltas)
    };
    assert_eq!(varied(0), ([0, 0, -62, 0], 4));
    assert_eq!(varied(2), ([0, 0, 0, 0], 4));
    assert_eq!(super::read_values(&[0, 0, 0, 4], 0, format), [0; 4]);
}

// A format 1 subtable whose first set is unsorted with a repeat and whose second stops two records short: the
// index names each second glyph, and every glyph it leaves out the search misses in both sets.
#[test]
fn the_second_glyph_index_leaves_out_only_what_no_pair_set_search_finds() {
    let be = |v: &[u16]| v.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let data = be(&[1, 0, 4, 0, 2, 14, 28, 3, 7, 10, 3, 20, 7, 30, 4, 9, 40, 5, 50]);
    let bits = super::super::pair_seconds(&data, &mut { usize::MAX }).expect("within the work");
    let seconds = crate::daecore::daetype::format::index::SparseIndex::build(
        &|e| { bits.runs().for_each(|(a, b)| e.put_run(a, b, 1, 0)); Some(()) }, 0, usize::MAX,
    ).expect("an index");
    let named: Vec<u16> = (0..12u16).filter(|&g| seconds.lookup(u32::from(g)) == 1).collect();
    assert_eq!(named, [3, 5, 7, 9]);
    for glyph in (0..12u16).filter(|g| !named.contains(g)) {
        assert_eq!(seconds.lookup(u32::from(glyph)), 0);
        for set in [&data[14..], &data[28..]] {
            assert_eq!(super::pair_record::<2>(set, glyph.into(), (4, 0)), None, "glyph {glyph}");
        }
    }
}
