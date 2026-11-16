// Timings, run with `--ignored`: the bench's sentence drawn in Inter, its outlines warm, at a size each.
use std::time::Instant;

use crate::raster::{RasterOptions, Rasterizer};
use crate::{Cache, Font, Location};

const MESSAGE: &str = "Sphinx of black quartz, judge my vow.";

fn sentence(px: f32) {
    sentence_in("inter/InterVariable.ttf", px);
}

fn sentence_in(name: &str, px: f32) {
    let path = std::format!("{}/assets/test-fonts/{name}", env!("CARGO_MANIFEST_DIR"));
    let Ok(bytes) = std::fs::read(path) else { return };
    let font = Font::from_bytes(&bytes).unwrap();
    let at = Location::default();
    let gids: std::vec::Vec<_> = MESSAGE.chars().filter_map(|c| font.glyph_id(c)).collect();
    font.set_cache_budget(Cache::Outline, 1 << 22);
    font.prewarm(&gids, &at);
    let mut r = Rasterizer::new();
    let mut run = || {
        let mut n = 0;
        for &g in &gids {
            n += r.glyph(&font, g, &at, px, &RasterOptions::DEFAULT).map_or(0, |i| i.pixels.len());
        }
        std::hint::black_box(n)
    };
    run();
    let rounds = 2_000;
    let t = Instant::now();
    for _ in 0..rounds {
        run();
    }
    std::println!("raster sentence {name} {px} px: {:?}", t.elapsed() / rounds);
}

// The outline alone, prepared onto a pen that keeps nothing: what drawing costs before any coverage.
#[test]
#[ignore = "a timing"]
fn raster_prepare_only_10() {
    struct Nothing;
    impl crate::outline::OutlinePen for Nothing {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {}
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    let Ok(bytes) = std::fs::read(path) else { return };
    let font = Font::from_bytes(&bytes).unwrap();
    let at = Location::default();
    let gids: std::vec::Vec<_> = MESSAGE.chars().filter_map(|c| font.glyph_id(c)).collect();
    font.set_cache_budget(Cache::Outline, 1 << 22);
    font.prewarm(&gids, &at);
    let options = crate::outline::OutlineOptions::DEFAULT;
    let rounds = 20_000;
    let t = Instant::now();
    for _ in 0..rounds {
        for &g in &gids {
            let _ = std::hint::black_box(font.prepared_outline(g, &at, 10.0, &options, &mut Nothing));
        }
    }
    std::println!("prepare only 10 px: {:?}", t.elapsed() / rounds);
}

#[test]
#[ignore = "a timing"]
fn raster_sentence_10() {
    sentence(10.0);
}

#[test]
#[ignore = "a timing"]
fn raster_sentence_40() {
    sentence(40.0);
}

#[test]
#[ignore = "a timing"]
fn raster_sentence_160() {
    sentence(160.0);
}

// EB Garamond's contours don't overlap, so every glyph after its first draw is summed straight.
#[test]
#[ignore = "a timing"]
fn raster_sentence_lean_10() {
    sentence_in("eb-garamond/EBGaramond.ttf", 10.0);
}

#[test]
#[ignore = "a timing"]
fn raster_sentence_lean_40() {
    sentence_in("eb-garamond/EBGaramond.ttf", 40.0);
}

#[test]
#[ignore = "a timing"]
fn raster_sentence_lean_160() {
    sentence_in("eb-garamond/EBGaramond.ttf", 160.0);
}

// Coverage alone, as the bench times it: the sentence's outlines prepared once, each filled as a path.
fn paths_in(name: &str, px: f32) {
    let path = std::format!("{}/assets/test-fonts/{name}", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(path).unwrap();
    let font = Font::from_bytes(&bytes).unwrap();
    let at = Location::default();
    let paths: std::vec::Vec<_> = MESSAGE
        .chars()
        .filter_map(|c| font.glyph_id(c))
        .filter_map(|g| {
            let mut p = crate::outline::Path::new();
            font.prepared_outline(g, &at, px, &crate::outline::OutlineOptions::DEFAULT, &mut p).ok()?;
            (!p.is_empty()).then_some(p)
        })
        .collect();
    let mut r = Rasterizer::new();
    let mut run = || {
        let rule = crate::outline::FillRule::NonZero;
        let n: usize = paths.iter().map(|p| r.path(p, rule, None).map_or(0, |i| i.pixels.len())).sum();
        std::hint::black_box(n)
    };
    run();
    let mut best = std::time::Duration::MAX;
    for _ in 0..15 {
        let t = Instant::now();
        for _ in 0..400 {
            run();
        }
        best = best.min(t.elapsed() / 400);
    }
    std::println!("paths {name} {px} px: {best:?}");
}

#[test]
#[ignore = "a timing"]
fn raster_paths() {
    for name in ["inter/InterVariable.ttf", "stix-two-math/STIX2Math.otf"] {
        for px in [10.0, 20.0, 40.0, 400.0] {
            paths_in(name, px);
        }
    }
}
