use super::*;
use crate::daecore::daetype::TableBytes;
use alloc::vec;

// Two tuples over two CVT entries: the first has no peak of its own, which cvar cannot resolve,
// and the second moves entry 1 by 12 at the axis's maximum.
#[test]
fn a_tuple_without_its_own_peak_is_skipped_alone() {
    let mut cvar = vec![0, 1, 0, 0, 0, 2, 0, 18];
    cvar.extend([0, 4, 0x20, 0]);
    cvar.extend([0, 5, 0xA0, 0, 0x40, 0]);
    cvar.extend([0, 1, 0x80, 0x80]);
    cvar.extend([1, 0, 1, 0, 12]);
    let map: Tables = [("cvar", TableBytes::from_vec(cvar))].into_iter().collect();
    let mut cvt = vec![0, 100, 0, 200];
    apply_cvar(&map, &mut cvt, &[1.0]).expect("applies");
    assert_eq!((read_i16_be(&cvt, 0), read_i16_be(&cvt, 2)), (Some(100), Some(212)));
}
