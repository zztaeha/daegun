use std::time::{Duration, Instant};

use daegun::Font;
use daegun::daecore::daetype::outline::OutlinePen;

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

fn sweep(name: &str, rel: &str, iters: usize) {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let font = Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path} did not parse: {e}"));
    let n = font.num_glyphs();

    let mut warm = CountPen::default();
    for gid in 0..n {
        let _ = font.outline_glyph(gid, &mut warm);
    }
    let expect = warm.ops;
    assert!(expect > 0, "{name}: sweep decoded nothing");

    let mut samples = Vec::with_capacity(iters);
    let mut proof = 0usize;
    for _ in 0..iters {
        let mut pen = CountPen::default();
        let t = Instant::now();
        for gid in 0..n {
            let _ = font.outline_glyph(gid, &mut pen);
        }
        samples.push(t.elapsed());
        proof += pen.ops;
        core::hint::black_box(&pen.acc);
    }
    assert_eq!(proof, iters * expect, "{name}: a sweep decoded a different number of operations");
    report(name, &format!("{n} glyphs through Font::outline_glyph, whole face per sample"), &mut samples);
}

#[test]
#[ignore]
fn sweep_public_glyf() {
    sweep("sweep_public_glyf", "eb-garamond/EBGaramond.ttf", 60);
}

#[test]
#[ignore]
fn sweep_public_cff() {
    sweep("sweep_public_cff", "stix-two-math/STIX2Math.otf", 10);
}

#[test]
#[ignore]
fn sweep_cff_hinted() {
    let path = format!("{}/stix-two-math/STIX2Math.otf", crate::FONTS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let font = Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path} did not parse: {e}"));
    let gid = font.glyph_id('A' as u32).expect("A");

    for hinting in [daegun::HintMode::None, daegun::HintMode::Auto] {
        let opts = daegun::OutlineOptions::default().with_hinting(hinting);
        let mut samples = Vec::with_capacity(300);
        for i in 0..300 {
            let px = 16.0 + i as f32 * 0.01;
            let mut path = daegun::Path::default();
            let t = Instant::now();
            let r = font.prepared_outline(gid, px, &[], &opts, &mut path);
            samples.push(t.elapsed());
            assert!(r.is_some() && !path.is_empty(), "prepared_outline drew nothing, so the bench measured nothing");
            core::hint::black_box(&path);
        }
        report(&format!("sweep_cff_hinted {hinting:?}"), "STIX2Math 'A', prepared_outline, hinted at a new size each time", &mut samples);
    }
}
