use std::time::{Duration, Instant};

use daegun::{Font, OutlineOptions, OutlinePen, Path};

const WARMUP: usize = 50;
const ROUNDS: usize = 200;
const BATCH: usize = 500;

fn font_bytes() -> Vec<u8> {
    let path = format!("{}/inter/InterVariable.ttf", crate::FONTS);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"))
}

#[derive(Default)]
struct Count(usize);

impl OutlinePen for Count {
    fn move_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn line_to(&mut self, _: f32, _: f32) {
        self.0 += 1;
    }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
        self.0 += 1;
    }
    fn close(&mut self) {
        self.0 += 1;
    }
}

fn report(name: &str, mut samples: Vec<Duration>) {
    samples.sort();
    let per = |d: Duration| d.as_nanos() as f64 / BATCH as f64;
    println!(
        "  {name:<26} {:>9.1} ns   median {:>9.1} ns",
        per(samples[0]),
        per(samples[samples.len() / 2])
    );
}

fn time<T>(mut f: impl FnMut() -> T) -> Vec<Duration> {
    for _ in 0..WARMUP {
        for _ in 0..BATCH {
            std::hint::black_box(f());
        }
    }
    let mut out = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        let t = Instant::now();
        for _ in 0..BATCH {
            std::hint::black_box(f());
        }
        out.push(t.elapsed());
    }
    out
}

fn time_once<T>(mut f: impl FnMut() -> T) -> Vec<Duration> {
    for _ in 0..WARMUP {
        std::hint::black_box(f());
    }
    let mut out = Vec::with_capacity(ROUNDS);
    let mut held: Option<T> = None;
    for _ in 0..ROUNDS {
        let t = Instant::now();
        let made = std::hint::black_box(f());
        out.push(t.elapsed() * BATCH as u32);
        held = Some(made);
    }
    drop(held);
    out
}

#[test]
#[ignore = "a measurement, not an assertion: run it explicitly"]
fn api_latency() {
    let bytes = font_bytes();
    let font = Font::from_bytes(&bytes).expect("the fixture parses");
    let gid = font.glyph_id(u32::from('g')).expect("a glyph for 'g'");

    println!("\ndaegun – Rust API, {} rounds after {} warmup\n", ROUNDS, WARMUP);

    report("font_open", time_once(|| Font::from_bytes(&bytes).expect("parses")));
    {
        let mut pool: Vec<Vec<u8>> = (0..WARMUP + ROUNDS).map(|_| bytes.clone()).collect();
        for _ in 0..WARMUP {
            std::hint::black_box(Font::from_vec(pool.pop().expect("pooled")).expect("parses"));
        }
        let mut samples = Vec::with_capacity(ROUNDS);
        let mut held = None;
        for _ in 0..ROUNDS {
            let buf = pool.pop().expect("pooled");
            let t = Instant::now();
            let made = std::hint::black_box(Font::from_vec(buf).expect("parses"));
            samples.push(t.elapsed() * BATCH as u32);
            held = Some(made);
        }
        drop(held);
        report("font_open_owned", samples);
    }

    report("upm", time(|| font.upm()));
    report("glyph_id", time(|| font.glyph_id(u32::from('g'))));
    let one = [gid];
    report("advance_widths x1", time(|| font.advance_widths(&one, &[])));

    report(
        "outline_glyph",
        time(|| {
            let mut c = Count::default();
            font.outline_glyph(gid, &mut c);
            c.0
        }),
    );

    report("ascender", time(|| font.ascender()));
    report("descender", time(|| font.descender()));
    report("cap_height", time(|| font.cap_height()));
    report("num_glyphs", time(|| font.num_glyphs()));
    report("line_metrics", time(|| font.line_metrics(false)));

    report("glyph_quads", time(|| font.glyph_quads(gid, &[])));
    let opts = OutlineOptions::default();
    report(
        "prepared_outline",
        time(|| {
            let mut c = Count::default();
            font.prepared_outline(gid, 16.0, &[], &opts, &mut c);
            c.0
        }),
    );
    let mut path = Path::default();
    font.outline_glyph(gid, &mut path);
    let max_area = daegun::max_area_for(16.0, f32::from(font.upm()));
    report("flatten", time(|| daegun::flatten(&path, max_area)));
    println!();
}
