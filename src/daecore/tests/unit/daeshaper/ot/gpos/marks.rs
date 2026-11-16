use crate::daecore::daeshaper::run::{AttachKind, Placement};

// A format 3 anchor's x device moves it at a location by its delta unrounded; its null y device leaves y.
#[test]
fn a_format_3_anchor_takes_its_devices_deltas_unrounded() {
    let deltas = super::super::tests::halfway();
    let owner = [0, 0, 0, 3, 0, 10, 0, 20, 0, 10, 0, 0, 0, 0, 0, 1, 0x80, 0];
    assert_eq!(super::anchor(&owner, 2, Some(&deltas)), (72.5, 20.0));
    assert_eq!(super::anchor(&owner, 2, None), (10.0, 20.0));
}

// A cursive ring, or a chain whose end loops back into it, adds each glyph's offset across once.
#[test]
fn a_cyclic_cursive_chain_lifts_a_mark_by_each_glyph_once() {
    let hanging = |links: [(i16, i32); 3]| {
        links.map(|(to, offset_y)| {
            let mut p = Placement { offset_y, ..Default::default() };
            (p.attach.to, p.attach.kind) = (to, AttachKind::Cursive);
            p
        })
    };
    assert_eq!(super::cross_stream(&hanging([(1, 10), (1, 20), (-2, 40)]), 0, true), 70);
    assert_eq!(super::cross_stream(&hanging([(1, 10), (1, 20), (-1, 40)]), 0, true), 70);
}
