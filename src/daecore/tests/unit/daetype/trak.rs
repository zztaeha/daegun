use super::*;
use alloc::string::String;
use crate::daecore::daetype::TableBytes;
use alloc::vec::Vec;

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// One track, at 0, over sizes 10 and 20 worth 10 and -10, across only: header, TrackData, its entry, sizes
// from 28 and values from 36, 40 bytes in all.
fn trak(version: u16) -> Vec<u8> {
    let mut t = words(&[version, 0, 0, 12, 0, 0, 1, 2, 0, 28, 0, 0, 256, 36, 10, 0, 20, 0]);
    t.extend(words(&[10, (-10i16).cast_unsigned()]));
    t
}

fn at(trak: Vec<u8>, ptem: f64, horizontal: bool) -> f64 {
    let map: Tables = [(String::from("trak"), TableBytes::from(trak))].into_iter().collect();
    tracking(&map, ptem, horizontal)
}

// A trak is read at version 1.0, and sizes or a track's values that leave the table are absent: with its one track's
// values cut, or its sizes moved past the end, this direction has nothing to add.
#[test]
fn a_trak_reads_only_at_version_1_and_in_bounds() {
    assert_eq!([5.0, 12.5, 15.0, 25.0].map(|p| at(trak(1), p, true)), [10.0, 5.0, 0.0, -10.0]);
    assert_eq!(at(trak(1), 5.0, false), 0.0, "no vertical data");
    assert_eq!(at(trak(2), 5.0, true), 0.0, "version 2");
    let mut cut = trak(1);
    cut.truncate(38);
    assert_eq!(at(cut, 5.0, true), 0.0, "values past the end");
    let mut far = trak(1);
    far[19] = 40;
    assert_eq!(at(far, 5.0, true), 0.0, "sizes past the end");
}
