use crate::daecore::daekit::cache::FontCache;
use alloc::string::String;
use alloc::vec::Vec;

const LOCATIONS: [&[(&str, f64)]; 4] = [
    &[],
    &[("SWPS", 45.0), ("SWPE", -20.0), ("SWC1", 1.0)],
    &[("SWPS", 90.0), ("SWPE", 90.0), ("SWC1", 2.0)],
    &[("SWPS", 45.0), ("SWPE", 45.0), ("SWC1", 1.0)],
];

// One cache walked through the locations out of order draws each glyph as a cache built for that location alone.
#[test]
fn cached_paint_matches_uncached_across_locations() {
    let rel = "colr-v1-test-glyphs/test_glyphs_variable.ttf";
    let path = format!("{}/{}", super::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("parses");
    let n = crate::daecore::daetype::decoder::read_u16_be(map.get("maxp").expect("maxp"), 4)
        .map(u32::from)
        .expect("n");
    let cache = FontCache::new(map.clone()).expect("the fixture loads");
    let wants: Vec<Vec<String>> = LOCATIONS
        .iter()
        .map(|axes| {
            let alone = FontCache::new(map.clone()).expect("the fixture loads");
            let key = alone.axis_key(axes);
            (0..n).map(|gid| format!("{:?}", alone.colr_v1_paint(gid, &key, 0))).collect()
        })
        .collect();
    assert!(wants[1] != wants[2], "the locations draw alike, so nothing was really compared");

    let order = [0, 1, 2, 3, 1, 0, 3, 2, 3, 3, 0, 1];
    let mut resolved = 0usize;
    for &i in &order {
        let key = cache.axis_key(LOCATIONS[i]);
        for gid in 0..n {
            let got = cache.colr_v1_paint(gid, &key, 0);
            assert_eq!(
                wants[i][gid as usize],
                format!("{got:?}"),
                "gid {gid} at {:?} differs between the cached and uncached paths",
                LOCATIONS[i],
            );
            resolved += usize::from(got.is_some());
        }
    }
    assert!(resolved > 0, "no glyph resolved to a paint graph, so nothing was really compared");
}
