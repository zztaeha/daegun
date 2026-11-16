use std::time::{Duration, Instant};
use crate::daecore::daetype::Tables;
use crate::daecore::Tag;
use alloc::vec::Vec;

fn table_map_of(rel: &str) -> Tables {
    let path = format!("{}/{}", super::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    crate::daecore::daetype::decoder::extract_ttf_tables(&bytes)
        .unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

fn report(name: &str, detail: &str, samples: &mut [Duration]) {
    samples.sort();
    let n = samples.len();
    let sum: Duration = samples.iter().sum();
    eprintln!("{name}");
    eprintln!("  {detail}");
    eprintln!(
        "  n {n}  min {:?}  mean {:?}  median {:?}  p95 {:?}  max {:?}",
        samples[0], sum / n as u32, samples[n / 2], samples[(n as f64 * 0.95) as usize], samples[n - 1],
    );
}

fn base_glyphs(colr: &[u8]) -> Vec<u32> {
    let off = crate::daecore::daetype::decoder::read_u32_be(colr, 14).unwrap_or(0) as usize;
    if off == 0 { return Vec::new(); }
    let n = crate::daecore::daetype::decoder::read_u32_be(colr, off).unwrap_or(0) as usize;
    (0..n)
        .filter_map(|i| crate::daecore::daetype::decoder::read_u16_be(colr, off + 4 + i * 6).map(u32::from))
        .collect()
}

// Every base glyph's paint graph as `Font::paint_graph` walks it, at `coords`, timed over the whole sweep.
fn bench(name: &str, rel: &str, coords: &[(Tag, f64)]) {
    let map = table_map_of(rel);
    let gids = base_glyphs(map.get("COLR").expect("COLR present"));
    assert!(!gids.is_empty(), "{name}: font declares no v1 base glyphs");
    let cache = crate::daecore::daekit::cache::FontCache::new(map).expect("the fixture loads");
    let location = cache.axis_key(coords);

    let expect = gids.iter().filter(|&&g| cache.colr_v1_paint(g, &location, 0).is_some()).count();
    assert!(expect > 0, "{name}: no glyph resolved to a paint graph");

    let iters = (400_000 / gids.len().max(1)).max(20);
    let mut samples = Vec::with_capacity(iters);
    let mut proof = 0usize;
    for _ in 0..iters {
        let t = Instant::now();
        let mut hit = 0usize;
        for &g in &gids {
            let p = cache.colr_v1_paint(g, &location, 0);
            if p.is_some() { hit += 1; }
            core::hint::black_box(&p);
        }
        samples.push(t.elapsed());
        proof += hit;
    }
    assert_eq!(proof, iters * expect, "{name}: a run resolved a different number of graphs");
    report(name, &format!("{} base glyphs, whole sweep per sample", gids.len()), &mut samples);
}

#[test]
#[ignore]
fn colr_v1_paint_graph_static() {
    bench("colr_v1_paint_graph_static", "colr-v1-test-glyphs/test_glyphs.ttf", &[]);
}

// Halfway from each axis's default to its maximum: normalized 0.5 on each.
#[test]
#[ignore]
fn colr_v1_paint_graph_variable() {
    let rel = "colr-v1-test-glyphs/test_glyphs_variable.ttf";
    let axes = crate::daecore::daetype::decoder::parse_fvar_axes(&table_map_of(rel)).expect("fvar");
    let half: Vec<(Tag, f64)> = axes.iter().take(3).map(|a| (a.tag, a.default + (a.max - a.default) / 2.0)).collect();
    bench("colr_v1_paint_graph_variable", rel, &half);
}

#[test]
#[ignore]
fn cache_colr_variable() {
    let at = [(Tag::new(b"SWPS"), 45.0), (Tag::new(b"SWPE"), 45.0), (Tag::new(b"SWC1"), 1.0)];
    bench("cache_colr_variable", "colr-v1-test-glyphs/test_glyphs_variable.ttf", &at);
}
