use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use daegun::daecore::daetype::hinting::{HintContext, HintMode};
use daegun::daecore::daetype::outline::OutlinePen;
use daegun::daecore::daetype::TableBytes;

#[derive(Default)]
struct CountPen { ops: usize, acc: f32 }

impl OutlinePen for CountPen {
    fn move_to(&mut self, x: f32, y: f32) { self.ops += 1; self.acc += x + y; }
    fn line_to(&mut self, x: f32, y: f32) { self.ops += 1; self.acc += x + y; }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) { self.ops += 1; self.acc += cx + cy + x + y; }
    fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
        self.ops += 1;
        self.acc += a + b + c + d + x + y;
    }
    fn close(&mut self) { self.ops += 1; }
}

fn font_bytes() -> Vec<u8> {
    match std::env::var("DAEGUN_HINT_FONT") {
        Ok(p) => std::fs::read(&p).unwrap_or_else(|e| panic!("DAEGUN_HINT_FONT unreadable: {p} ({e})")),
        Err(_) => {
            let p = format!("{}/test-fixtures/hinted.ttf", crate::FONTS);
            std::fs::read(&p).unwrap_or_else(|e| panic!("fixture missing: {p} ({e})"))
        }
    }
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

fn hinted_gids(map: &BTreeMap<String, TableBytes>, loca: &[usize], n: u16) -> Vec<u16> {
    let glyf = map.get("glyf").expect("glyf");
    (0..n)
        .filter(|&g| {
            let (s, e) = (loca[g as usize], loca[g as usize + 1]);
            if e <= s { return false; }
            let Some(nc) = daegun::daecore::daetype::decoder::read_i16_be(glyf, s) else { return false };
            if nc <= 0 { return false; }
            let at = s + 10 + nc as usize * 2;
            daegun::daecore::daetype::decoder::read_u16_be(glyf, at).is_some_and(|len| len > 0)
        })
        .collect()
}

#[test]
#[ignore]
fn hint_glyph_bytecode() {
    const PPEM: u16 = 16;
    let bytes = font_bytes();
    let map = daegun::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("font parses");
    let head = map.get("head").expect("head");
    let maxp = map.get("maxp").expect("maxp");
    let upm = daegun::daecore::daetype::decoder::read_u16_be(head, 18).expect("upm");
    let fmt = daegun::daecore::daetype::decoder::read_i16_be(head, 50).expect("locaFmt");
    let n = daegun::daecore::daetype::decoder::read_u16_be(maxp, 4).expect("numGlyphs");
    let loca = daegun::daecore::daetype::instancer::parse_loca(&map, fmt, n as usize).expect("loca");
    let glyf = map.get("glyf").expect("glyf").clone();

    let gids = hinted_gids(&map, &loca, n);
    assert!(!gids.is_empty(), "font carries no per-glyph instructions to hint");

    let mut ctx = HintContext::new(&map, PPEM.into(), upm, HintMode::Subpixel).expect("font carries hinting");

    let mut expect = 0usize;
    for &gid in &gids {
        let mut pen = CountPen::default();
        if let Some(out) = ctx.hint_glyph(&glyf, &loca, gid) {
            daegun::daecore::daetype::hinting::draw_hinted(&out, &mut pen);
        }
        expect += pen.ops;
    }
    assert!(expect > 0, "hinting produced no pen operations");

    let iters = (200_000 / gids.len().max(1)).max(20);
    let mut samples = Vec::with_capacity(iters);
    let mut proof = 0usize;
    for _ in 0..iters {
        let mut pen = CountPen::default();
        let t = Instant::now();
        for &gid in &gids {
            if let Some(out) = ctx.hint_glyph(&glyf, &loca, gid) {
                daegun::daecore::daetype::hinting::draw_hinted(&out, &mut pen);
            }
        }
        samples.push(t.elapsed());
        proof += pen.ops;
        core::hint::black_box(&pen.acc);
    }
    assert_eq!(proof, iters * expect, "a run hinted a different number of operations");
    report(
        "hint_glyph_bytecode",
        &format!("{} hinted glyphs at {PPEM}ppem, whole sweep per sample", gids.len()),
        &mut samples,
    );
}

#[test]
#[ignore]
fn hint_glyph_context_per_glyph() {
    const PPEM: u16 = 16;
    let bytes = font_bytes();
    let map = daegun::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("parses");
    let upm = daegun::daecore::daetype::decoder::read_u16_be(map.get("head").unwrap(), 18).unwrap();
    let fmt = daegun::daecore::daetype::decoder::read_i16_be(map.get("head").unwrap(), 50).unwrap();
    let n = daegun::daecore::daetype::decoder::read_u16_be(map.get("maxp").unwrap(), 4).unwrap();
    let loca = daegun::daecore::daetype::instancer::parse_loca(&map, fmt, n as usize).expect("loca");
    let glyf = map.get("glyf").expect("glyf").clone();
    let gids = hinted_gids(&map, &loca, n);

    let iters = (20_000 / gids.len().max(1)).max(5);
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let mut pen = CountPen::default();
        let t = Instant::now();
        for &gid in &gids {
            if let Some(mut ctx) = HintContext::new(&map, PPEM.into(), upm, HintMode::Subpixel)
                && let Some(o) = ctx.hint_glyph(&glyf, &loca, gid)
            {
                daegun::daecore::daetype::hinting::draw_hinted(&o, &mut pen);
            }
        }
        samples.push(t.elapsed());
        core::hint::black_box(&pen.acc);
    }
    report("hint_glyph_context_per_glyph", &format!("{} glyphs, HintContext rebuilt each time", gids.len()), &mut samples);
}

#[test]
#[ignore]
fn hint_glyph_context_cached() {
    const PPEM: u16 = 16;
    let bytes = font_bytes();
    let map = daegun::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("parses");
    let upm = daegun::daecore::daetype::decoder::read_u16_be(map.get("head").unwrap(), 18).unwrap();
    let fmt = daegun::daecore::daetype::decoder::read_i16_be(map.get("head").unwrap(), 50).unwrap();
    let n = daegun::daecore::daetype::decoder::read_u16_be(map.get("maxp").unwrap(), 4).unwrap();
    let loca = daegun::daecore::daetype::instancer::parse_loca(&map, fmt, n as usize).expect("loca");
    let glyf = map.get("glyf").expect("glyf").clone();
    let gids = hinted_gids(&map, &loca, n);
    let cache = daegun::daecore::cache::FontCache::new(map);

    let iters = (200_000 / gids.len().max(1)).max(20);
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let mut pen = CountPen::default();
        let t = Instant::now();
        for &gid in &gids {
            if let Some(o) = cache.hint_glyph_cached(&glyf, &loca, gid, PPEM.into(), upm, HintMode::Subpixel) {
                daegun::daecore::daetype::hinting::draw_hinted(&o, &mut pen);
            }
        }
        samples.push(t.elapsed());
        core::hint::black_box(&pen.acc);
    }
    report("hint_glyph_context_cached", &format!("{} glyphs through FontCache", gids.len()), &mut samples);
}
