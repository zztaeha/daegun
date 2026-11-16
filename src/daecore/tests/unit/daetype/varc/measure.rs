use super::*;
use alloc::vec::Vec;
use crate::daecore::daekit::cache::FontCache;

fn cache(bytes: &[u8]) -> FontCache {
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(bytes).expect("parses");
    FontCache::new(map).expect("loads")
}

fn fixture(name: &str) -> FontCache {
    let path = alloc::format!("{}/assets/test-fonts/varc/{name}", env!("CARGO_MANIFEST_DIR"));
    cache(&std::fs::read(path).expect("a fixture"))
}

// Each glyph's extents at `at`, as x bearing, y bearing, width and height.
fn measured(cache: &FontCache, at: &[(&str, f64)], gids: &[u32]) -> Vec<Option<[i32; 4]>> {
    let boxed = |e: crate::daecore::daekit::cache::GlyphExtents| [e.x_bearing, e.y_bearing, e.width, e.height];
    cache.described_at(&cache.axis_key(at), |at| gids.iter().map(|&g| at.glyph_extents(g).map(boxed)).collect())
}

// Counted run by run, each zero run 64 indices to its byte without one held.
#[test]
fn a_run_cut_short_ends_the_axis_indices() {
    assert_eq!(index_count(&[0x02, 1, 2, 3]), 3);
    assert_eq!(index_count(&[0x81]), 2);
    // A run of two bytes with one left ends the entry, the values before it kept.
    assert_eq!(index_count(&[0x01, 0x05]), 0);
    assert_eq!(index_count(&[0x00, 0x04, 0x01, 0x05]), 1);
    assert_eq!(index_count(&[0x01, 0x00, 0x07]), 2);
    assert_eq!(index_count(&[0x40, 0xFF, 0xFE]), 1);
    assert_eq!(index_count(&[0xBF; 1_000]), 64_000);
}

#[test]
fn axis_values_fail_a_run_past_their_count_or_their_data() {
    let mut p = 0;
    assert_eq!(strict_values(&[0x01, 5, 0xFB, 9], &mut p, 2), Some(()));
    assert_eq!(p, 3);
    assert_eq!(strict_values(&[0x02, 5, 6, 7], &mut 0, 2), None);
    assert_eq!(strict_values(&[0x41, 0, 1], &mut 0, 2), None);
    assert_eq!(strict_values(&[0x80], &mut 0, 2), None);
    assert_eq!(strict_values(&[], &mut 0, 0), Some(()));
}

// An entry counting more values than any record holds is refused at the record's end, before anything is set aside
// for them: here usize::MAX, which no allocation holds.
#[test]
fn axis_values_past_the_record_are_refused_before_they_are_held() {
    assert_eq!(strict_values(&[0xBF; 4], &mut 0, usize::MAX), None);
}

#[test]
fn a_five_byte_number_takes_its_last_four() {
    let mut p = 0;
    assert_eq!(uint32var(&[0xFF, 0x12, 0x34, 0x56, 0x78], &mut p), Some(0x1234_5678));
    assert_eq!(p, 5);
    assert_eq!(uint32var(&[0xBF, 0xFF], &mut 0), Some(0x3FFF));
    assert_eq!(uint32var(&[0xC0, 0x01], &mut 0), None);
}

// The fixtures' glyphs boxed as drawn through their components: 각 grows 120 right and 40 down by wght 900, an empty
// glyph is the empty box, and the hostile font's 10 is the square ending its chain of 20, turned 32.5° (the rest
// daegun's recorded output).
#[test]
fn the_fixtures_measure_through_their_components() {
    let hangul = fixture("varc-hangul.ttf");
    let at_default = measured(&hangul, &[], &[0, 1, 2]);
    assert_eq!(at_default, [Some([0, 0, 0, 0]), Some([50, 800, 750, -800]), Some([50, 800, 750, -800])]);
    assert_eq!(measured(&hangul, &[("wght", 900.0)], &[2]), [Some([50, 800, 870, -760])]);
    let cff2 = measured(&fixture("varc-short-cff2.otf"), &[], &[1, 2, 3]);
    assert_eq!(cff2, [Some([100, 100, 100, -100]), Some([0, 0, 0, 0]), Some([600, 200, 200, -200])]);
    let hostile = measured(&fixture("varc-hostile.ttf"), &[], &[5, 8, 10]);
    assert_eq!(hostile, [Some([91, 74, 329, -263]), Some([0, 143, 192, -143]), Some([193, 206, 138, -138])]);
}
