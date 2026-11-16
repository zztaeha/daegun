use super::*;
use crate::daecore::daetype::TableBytes;
use alloc::vec;
use alloc::vec::Vec;

// An MVAR whose gsp0, hcrs and vasc records move by 5, -7 and 9 at the axis's maximum.
fn mvar() -> Vec<u8> {
    let mut t = vec![0, 1, 0, 0, 0, 0, 0, 8, 0, 3, 0, 36];
    for (tag, inner) in [(b"gsp0", 0u8), (b"hcrs", 1), (b"vasc", 2)] {
        t.extend(tag);
        t.extend([0, 0, 0, inner]);
    }
    t.extend([0, 1, 0, 0, 0, 12, 0, 1, 0, 0, 0, 22]);
    t.extend([0, 1, 0, 1, 0, 0, 0x40, 0, 0x40, 0]);
    t.extend([0, 3, 0, 0, 0, 1, 0, 0, 5, 0xF9, 9]);
    t
}

#[test]
fn every_value_tag_reaches_its_table() {
    let map: Tables = [("MVAR", TableBytes::from_vec(mvar()))].into_iter().collect();
    let (mut hhea, mut vhea) = (vec![0u8; 36], vec![0u8; 36]);
    hhea[18..20].copy_from_slice(&100i16.to_be_bytes());
    vhea[4..6].copy_from_slice(&500i16.to_be_bytes());
    let mut gasp = vec![0, 1, 0, 2, 0, 8, 0, 2, 0xFF, 0xFF, 0, 15];
    let targets = MvarTargets { hhea: &mut hhea, vhea: &mut vhea, os2: &mut [], post: &mut [], gasp: &mut gasp };
    apply_mvar(&map, targets, &[1.0]).expect("applies");
    assert_eq!((read_i16_be(&hhea, 18), read_i16_be(&vhea, 4), read_u16_be(&gasp, 4)), (Some(93), Some(509), Some(13)));
}
