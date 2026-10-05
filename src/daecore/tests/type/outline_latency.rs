use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use daegun::daecore::daetype::outline::OutlinePen;
use daegun::daecore::daetype::TableBytes;

fn table_map_of(rel: &str) -> BTreeMap<String, TableBytes> {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    daegun::daecore::daetype::decoder::extract_ttf_tables(&bytes)
        .unwrap_or_else(|e| panic!("{path} did not parse: {e}"))
}

#[derive(Default)]
struct OutlineCountPen { ops: usize, acc: f32 }

impl OutlinePen for OutlineCountPen {
    fn move_to(&mut self, x: f32, y: f32) { self.ops += 1; self.acc += x + y; }
    fn line_to(&mut self, x: f32, y: f32) { self.ops += 1; self.acc += x + y; }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) { self.ops += 1; self.acc += cx + cy + x + y; }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.ops += 1;
        self.acc += c1x + c1y + c2x + c2y + x + y;
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

fn loca_of(map: &BTreeMap<String, TableBytes>) -> Vec<usize> {
    let head = map.get("head").expect("fixture carries head");
    let maxp = map.get("maxp").expect("fixture carries maxp");
    let fmt = daegun::daecore::daetype::decoder::read_i16_be(head, 50).expect("head carries indexToLocFormat");
    let n = daegun::daecore::daetype::decoder::read_u16_be(maxp, 4).expect("maxp carries numGlyphs") as usize;
    daegun::daecore::daetype::instancer::parse_loca(map, fmt, n).expect("loca parses")
}

fn bench_glyf(name: &str, rel: &str, gid: u16, iters: usize, detail: &str) {
    let map = table_map_of(rel);
    let loca = loca_of(&map);
    let mut pen = OutlineCountPen::default();

    let warm = daegun::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, &loca, gid, &mut pen);
    assert!(warm.is_ok(), "{name}: gid {gid} produced no outline");
    let expect = pen.ops;
    assert!(expect > 0, "{name}: gid {gid} decoded to zero pen operations");

    let mut samples = Vec::with_capacity(iters);
    let mut proof = 0usize;
    for _ in 0..iters {
        let mut pen = OutlineCountPen::default();
        let t = Instant::now();
        let r = daegun::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, &loca, gid, &mut pen);
        samples.push(t.elapsed());
        proof += pen.ops;
        core::hint::black_box((&r, &pen.acc));
    }
    assert_eq!(proof, iters * expect, "{name}: a run decoded a different number of operations");
    report(name, detail, &mut samples);
}

#[test]
#[ignore]
fn outline_glyf_scheherazade() {
    bench_glyf(
        "outline_glyf_scheherazade",
        "scheherazade-new/ScheherazadeNew-Regular.ttf",
        1583,
        20_000,
        "ScheherazadeNew-Regular.ttf gid 1583, 1683 points, glyf, counted only",
    );
}

#[test]
#[ignore]
fn outline_glyf_eb_garamond_composite() {
    bench_glyf(
        "outline_glyf_eb_garamond_composite",
        "eb-garamond/EBGaramond.ttf",
        2244,
        50_000,
        "EBGaramond.ttf gid 2244, a 7-component composite, glyf, counted only",
    );
}

#[test]
#[ignore]
fn outline_cff_stix() {
    const NAME: &str = "outline_cff_stix";
    const N: usize = 2_000;
    let map = table_map_of("stix-two-math/STIX2Math.otf");
    let cff = map.get("CFF ").expect("STIX2Math carries a CFF1 table");
    let outlines = daegun::daecore::daetype::outline::CffOutlines::parse(cff).expect("CFF navigation parses");

    let mut pen = OutlineCountPen::default();
    let warm = daegun::daecore::daetype::outline::outline_cff_glyph_with(&outlines, cff, 2257, &mut pen);
    assert!(warm.is_ok(), "{NAME}: gid 2257 produced no outline");
    let expect = pen.ops;
    assert!(expect > 0, "{NAME}: gid 2257 decoded to zero pen operations");

    let mut samples = Vec::with_capacity(N);
    let mut proof = 0usize;
    for _ in 0..N {
        let mut pen = OutlineCountPen::default();
        let t = Instant::now();
        let r = daegun::daecore::daetype::outline::outline_cff_glyph_with(&outlines, cff, 2257, &mut pen);
        samples.push(t.elapsed());
        proof += pen.ops;
        core::hint::black_box((&r, &pen.acc));
    }
    assert_eq!(proof, N * expect, "{NAME}: a run decoded a different number of operations");
    report(
        NAME,
        "STIX2Math.otf gid 2257, a 5819-byte Type 2 charstring, against pre-parsed navigation",
        &mut samples,
    );
}

fn bench_autohint(name: &str, rel: &str, ppem: u16, iters: usize, detail: &str) {
    use daegun::daecore::daetype::hinting::auto::{AutoHinter, CollectPen};

    let map = table_map_of(rel);
    let upm = daegun::daecore::daetype::decoder::read_u16_be(map.get("head").expect("head"), 18).expect("upm");
    let cmap = map.get("cmap").expect("cmap").clone();
    let is_cff = map.contains_key("CFF ");
    let loca = (!is_cff).then(|| loca_of(&map));
    let cff = map.get("CFF ").cloned();
    let outlines = cff.as_ref().map(|c| {
        daegun::daecore::daetype::outline::CffOutlines::parse(c).expect("CFF navigation parses")
    });

    let collect = |gid: u16| {
        let mut pen = CollectPen::new();
        match (&outlines, &cff, &loca) {
            (Some(o), Some(c), _) => {
                daegun::daecore::daetype::outline::outline_cff_glyph_with(o, c, gid, &mut pen).ok()?;
            }
            (_, _, Some(l)) => {
                daegun::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, l, gid, &mut pen).ok()?;
            }
            _ => return None,
        }
        let p = pen.finish();
        (!p.is_empty()).then_some(p)
    };
    let gid_of = |c: char| daegun::daecore::daetype::subsetter::cmap_glyph_id(&cmap, c as u32);

    let mut hinter = AutoHinter::new(upm, &mut |c| gid_of(c), &mut |g| collect(g))
        .unwrap_or_else(|| panic!("{name}: {rel} yields no blue zones"));
    let gid = gid_of('H').unwrap_or_else(|| panic!("{name}: no H"));
    let pts = collect(gid).unwrap_or_else(|| panic!("{name}: H has no outline"));

    let warm = hinter.hint(&pts, ppem.into());
    assert_eq!(warm.y.len(), pts.len(), "{name}: hinting changed the point count");

    let mut samples = Vec::with_capacity(iters);
    let mut proof = 0usize;
    for _ in 0..iters {
        let mut pen = CollectPen::new();
        let t = Instant::now();
        match (&outlines, &cff, &loca) {
            (Some(o), Some(c), _) => {
                daegun::daecore::daetype::outline::outline_cff_glyph_with(o, c, gid, &mut pen).expect("draws");
            }
            (_, _, Some(l)) => {
                daegun::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, l, gid, &mut pen).expect("draws");
            }
            _ => unreachable!(),
        }
        let p = pen.finish();
        let out = hinter.hint(&p, ppem.into());
        samples.push(t.elapsed());
        proof += out.y.len();
        core::hint::black_box(&out.y);
    }
    assert_eq!(proof, iters * pts.len(), "{name}: a run produced a different point count");
    report(name, detail, &mut samples);
}

#[test]
#[ignore]
fn autohint_inter_h() {
    bench_autohint(
        "autohint_inter_H",
        "inter/InterVariable.ttf",
        13,
        20_000,
        "InterVariable.ttf 'H' at 13 ppem, glyf, collect + grid fit",
    );
}

#[test]
#[ignore]
fn autohint_stix_h() {
    bench_autohint(
        "autohint_stix_H",
        "stix-two-math/STIX2Math.otf",
        13,
        20_000,
        "STIX2Math.otf 'H' at 13 ppem, CFF, collect + grid fit",
    );
}

#[test]
#[ignore]
fn outline_glyf_sweep() {
    let path = std::env::var("DAEGUN_SWEEP_FONT")
        .unwrap_or_else(|_| format!("{}/eb-garamond/EBGaramond.ttf", crate::FONTS));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let map = daegun::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("parses");
    let head = map.get("head").expect("head");
    let fmt = daegun::daecore::daetype::decoder::read_i16_be(head, 50).expect("locaFmt");
    let n = daegun::daecore::daetype::decoder::read_u16_be(map.get("maxp").expect("maxp"), 4).expect("n");
    let loca = daegun::daecore::daetype::instancer::parse_loca(&map, fmt, n as usize).expect("loca");

    let mut warm = OutlineCountPen::default();
    for gid in 0..n {
        let _ = daegun::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, &loca, gid, &mut warm);
    }
    let expect = warm.ops;
    assert!(expect > 0, "sweep decoded nothing");

    let iters = 60usize;
    let mut samples = Vec::with_capacity(iters);
    let mut proof = 0usize;
    for _ in 0..iters {
        let mut pen = OutlineCountPen::default();
        let t = Instant::now();
        for gid in 0..n {
            let _ = daegun::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, &loca, gid, &mut pen);
        }
        samples.push(t.elapsed());
        proof += pen.ops;
        core::hint::black_box(&pen.acc);
    }
    assert_eq!(proof, iters * expect, "a sweep decoded a different number of operations");
    report("outline_glyf_sweep", &format!("{n} glyphs, whole face per sample"), &mut samples);
}

#[test]
#[ignore]
fn outline_cff_sweep() {
    let map = table_map_of("stix-two-math/STIX2Math.otf");
    let cff = map.get("CFF ").expect("CFF ");
    let outlines = daegun::daecore::daetype::outline::CffOutlines::parse(cff).expect("navigation parses");
    let n = daegun::daecore::daetype::decoder::read_u16_be(map.get("maxp").expect("maxp"), 4).expect("n");

    let mut warm = OutlineCountPen::default();
    for gid in 0..n {
        let _ = daegun::daecore::daetype::outline::outline_cff_glyph_with(&outlines, cff, gid, &mut warm);
    }
    let expect = warm.ops;
    assert!(expect > 0, "the sweep decoded nothing");

    let iters = 30usize;
    let mut samples = Vec::with_capacity(iters);
    let mut proof = 0usize;
    for _ in 0..iters {
        let mut pen = OutlineCountPen::default();
        let t = Instant::now();
        for gid in 0..n {
            let _ = daegun::daecore::daetype::outline::outline_cff_glyph_with(&outlines, cff, gid, &mut pen);
        }
        samples.push(t.elapsed());
        proof += pen.ops;
        core::hint::black_box(&pen.acc);
    }
    assert_eq!(proof, iters * expect, "a sweep decoded a different number of operations");
    report("outline_cff_sweep", &format!("{n} glyphs, {expect} segments, whole face per sample"), &mut samples);
}

fn bench_autohint_sweep(name: &str, rel: &str, ppem: u16, iters: usize) {
    use daegun::daecore::daetype::hinting::auto::{AutoHinter, CollectPen};

    let map = table_map_of(rel);
    let upm = daegun::daecore::daetype::decoder::read_u16_be(map.get("head").expect("head"), 18).expect("upm");
    let cmap = map.get("cmap").expect("cmap").clone();
    let n = daegun::daecore::daetype::decoder::read_u16_be(map.get("maxp").expect("maxp"), 4).expect("n");
    let is_cff = map.contains_key("CFF ");
    let loca = (!is_cff).then(|| loca_of(&map));
    let cff = map.get("CFF ").cloned();
    let outlines = cff.as_ref().map(|c| {
        daegun::daecore::daetype::outline::CffOutlines::parse(c).expect("CFF navigation parses")
    });

    let collect = |gid: u16| {
        let mut pen = CollectPen::new();
        match (&outlines, &cff, &loca) {
            (Some(o), Some(c), _) => {
                daegun::daecore::daetype::outline::outline_cff_glyph_with(o, c, gid, &mut pen).ok()?;
            }
            (_, _, Some(l)) => {
                daegun::daecore::daetype::outline::outline_glyf_glyph_with_loca(&map, l, gid, &mut pen).ok()?;
            }
            _ => return None,
        }
        let p = pen.finish();
        (!p.is_empty()).then_some(p)
    };
    let gid_of = |c: char| daegun::daecore::daetype::subsetter::cmap_glyph_id(&cmap, c as u32);

    let mut hinter = AutoHinter::new(upm, &mut |c| gid_of(c), &mut |g| collect(g))
        .unwrap_or_else(|| panic!("{name}: {rel} yields no blue zones"));

    let mut warm = 0usize;
    for gid in 0..n {
        if let Some(pts) = collect(gid) {
            warm += hinter.hint(&pts, ppem.into()).y.len();
        }
    }
    assert!(warm > 0, "{name}: the sweep hinted nothing");

    let mut samples = Vec::with_capacity(iters);
    let mut proof = 0usize;
    for _ in 0..iters {
        let t = Instant::now();
        let mut fitted = 0usize;
        for gid in 0..n {
            if let Some(pts) = collect(gid) {
                fitted += hinter.hint(&pts, ppem.into()).y.len();
            }
        }
        samples.push(t.elapsed());
        proof += fitted;
    }
    assert_eq!(proof, iters * warm, "{name}: a sweep hinted a different number of points");
    report(name, &format!("{n} glyphs, {warm} points, whole face per sample"), &mut samples);
}

#[test]
#[ignore]
fn autohint_sweep_inter() {
    bench_autohint_sweep("autohint_sweep_inter", "inter/InterVariable.ttf", 13, 20);
}

#[test]
#[ignore]
fn autohint_sweep_stix() {
    bench_autohint_sweep("autohint_sweep_stix", "stix-two-math/STIX2Math.otf", 13, 10);
}
