use super::*;
use alloc::vec::Vec;

fn words(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// Counts of 100 over tables holding 10 glyphs and 2 ranges: each answers from what it holds, as
// one cut short at 64 or fewer already did.
#[test]
fn a_coverage_cut_short_answers_from_what_it_holds() {
    let glyphs: Vec<u16> = [1, 100].into_iter().chain(10..20).collect();
    assert_eq!(coverage_index(&words(&glyphs), 15), Some(5));
    assert_eq!(coverage_index(&words(&[2, 100, 10, 19, 0, 30, 39, 10]), 35), Some(15));
}
