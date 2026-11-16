use super::*;
use crate::daecore::daetype::format::ivs::DeltaSource;
use crate::daecore::daetype::format::ivs::DeltaView;
use crate::daecore::daetype::TableBytes;

// With no map, an index is outer in its high word and inner in its low, as the spec's implicit mapping has it:
// 0x00050000 is row 0 of the sixth data subtable, drawn or measured.
#[test]
fn with_no_map_an_index_splits_into_outer_and_inner() {
    let store = super::super::testing::store(6, -50);
    let source = DeltaSource::read(&store, 0, None).expect("a store");
    let scalars = source.scalars(&[1.0]);
    assert_eq!(resolve_delta(Deltas::Measured(Some(&source), &scalars), 0x0005_0000, 0), -50.0);
    let drawn = DeltaView::read(&TableBytes::from_vec(store.clone()), 0, None).expect("a store");
    let at = drawn.scalars_at(&[0x10000]);
    assert_eq!(resolve_delta(Deltas::Drawn(Some((&drawn, &at))), 0x0005_0000, 0), -50.0, "drawn");
    let with_empty_map = [store.clone(), alloc::vec![0, 0, 0, 0]].concat();
    let source = DeltaSource::read(&with_empty_map, 0, Some(store.len())).expect("a store");
    let delta = resolve_delta(Deltas::Measured(Some(&source), &scalars), 0x0005_0000, 0);
    assert_eq!(delta, -50.0, "an empty map is no map");
}

// A field's index past 0xFFFFFFFF names no item rather than wrapping to the first: base 0xFFFFFFFE's third field
// reads nothing, drawn or measured.
#[test]
fn an_index_past_32_bits_names_no_item() {
    let store = super::super::testing::store(1, -50);
    let drawn = DeltaView::read(&TableBytes::from_vec(store.clone()), 0, None).expect("a store");
    let at = drawn.scalars_at(&[0x10000]);
    assert_eq!(resolve_delta(Deltas::Drawn(Some((&drawn, &at))), 0xFFFF_FFFE, 2), 0.0);
    assert_eq!(resolve_delta(Deltas::Drawn(Some((&drawn, &at))), u32::MAX, 0), 0.0, "no variation");
    let source = DeltaSource::read(&store, 0, None).expect("a store");
    let scalars = source.scalars(&[1.0]);
    assert_eq!(resolve_delta(Deltas::Measured(Some(&source), &scalars), 0xFFFF_FFFE, 2), 0.0);
}

// Alpha 0x7FFF moved by 1 is 0x8000, 2.0 in F2DOT14, summed whole whichever way it's read, not wrapped to 16 bits.
#[test]
fn a_varied_alpha_is_summed_whole() {
    let store = super::super::testing::store(1, 1);
    let drawn = DeltaView::read(&TableBytes::from_vec(store.clone()), 0, None).expect("a store");
    let at = drawn.scalars_at(&[0x10000]);
    let source = DeltaSource::read(&store, 0, None).expect("a store");
    let scalars = source.scalars(&[1.0]);
    let alpha = |deltas: Deltas| {
        let ctx = Colrv1Ctx {
            colr: &[], cpal: None, palette: None, deltas, base_glyph_list_off: 0, layer_list_off: None,
            drawn: matches!(deltas, Deltas::Drawn(_)),
        };
        resolve_var_field(0x7FFF, VarField::Alpha, 0, 0, &ctx)
    };
    assert_eq!(alpha(Deltas::Drawn(Some((&drawn, &at)))), 2.0);
    assert_eq!(alpha(Deltas::Measured(Some(&source), &scalars)), 2.0);
}
