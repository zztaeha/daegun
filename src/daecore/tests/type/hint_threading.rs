use alloc::vec::Vec;
use std::sync::Arc;
use std::time::Instant;

use crate::daecore::daekit::cache::FontCache;
use crate::daecore::daetype::hinting::{HintMode, HintedOutline};

const PPEM: u16 = 16;

fn fixture(rel: &str) -> (Arc<FontCache>, Vec<usize>, Vec<u8>, u16, u32) {
    let path = format!("{}/{}", super::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("parses");
    let upm = crate::daecore::daetype::decoder::read_u16_be(map.get("head").expect("head"), 18).expect("upm");
    let fmt = crate::daecore::daetype::decoder::read_i16_be(map.get("head").expect("head"), 50).expect("fmt");
    let n = crate::daecore::daetype::decoder::read_u16_be(map.get("maxp").expect("maxp"), 4)
        .map(u32::from)
        .expect("n");
    let loca = crate::daecore::daetype::instancer::parse_loca(&map, fmt, n as usize).expect("loca");
    let glyf = map.get("glyf").expect("glyf").to_owned_vec();
    (Arc::new(FontCache::new(map).expect("the fixture loads")), loca, glyf, upm, n)
}

#[test]
#[ignore]
fn hint_contention() {
    let (cache, loca, glyf, upm, n) = fixture("test-fixtures/hinted.ttf");
    let gids: Vec<u32> = (1..n).collect();
    let loca = Arc::new(loca);
    let glyf = Arc::new(glyf);

    for &g in &gids {
        cache.hint_glyph_cached(&glyf, loca.as_slice().into(), g, PPEM.into(), upm, HintMode::Font, |_| ());
    }

    const PASSES: usize = 4000;
    for threads in [1usize, 2, 4, 8] {
        let t = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..threads {
                let (cache, loca, glyf, gids) =
                    (Arc::clone(&cache), Arc::clone(&loca), Arc::clone(&glyf), gids.clone());
                s.spawn(move || {
                    let mut ops = 0usize;
                    for _ in 0..PASSES {
                        for &g in &gids {
                            let points = |o: &mut HintedOutline| o.x.len();
                            let loca = loca.as_slice().into();
                            ops += cache.hint_glyph_cached(&glyf, loca, g, PPEM.into(), upm, HintMode::Font, points)
                                .unwrap_or(0);
                        }
                    }
                    core::hint::black_box(ops);
                });
            }
        });
        eprintln!(
            "  hint_glyph_cached {threads} threads x {PASSES} passes x {} glyphs: {:?}",
            gids.len(),
            t.elapsed(),
        );
    }
}
