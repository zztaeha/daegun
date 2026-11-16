use std::time::{Duration, Instant};

use daegun::{Font, Location, Orientation};
use daegun::outline::{OutlineOptions, OutlinePen, Path};
use daegun::raster::{RasterOptions, Rasterizer};

const WARMUP: usize = 50;
const ROUNDS: usize = 200;
const BATCH: usize = 500;
// Copies of the bytes an open reads, each for ROUNDS / PLACES rounds in turn.
const PLACES: usize = 20;
// The clocks and memory take about 100 ms to come up after the core idles, longer than 50 batches of a short call.
const SETTLE: Duration = Duration::from_millis(300);

fn font_bytes() -> Vec<u8> {
    let path = format!("{}/inter/InterVariable.ttf", crate::FONTS);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"))
}

// Called through a vtable hidden from the optimizer, as latency.c's pen is functions C calls through pointers: a
// row times the call, not the pen.
#[derive(Default)]
pub(crate) struct Count(pub(crate) usize);

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

fn opaque(pen: &mut Count) -> &mut dyn OutlinePen {
    std::hint::black_box(pen)
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

fn warm<T>(mut f: impl FnMut() -> T) {
    let start = Instant::now();
    let mut done = 0;
    while done < WARMUP || start.elapsed() < SETTLE {
        std::hint::black_box(f());
        done += 1;
    }
}

fn time<T>(mut f: impl FnMut() -> T) -> Vec<Duration> {
    warm(|| {
        for _ in 0..BATCH {
            std::hint::black_box(f());
        }
    });
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

// Each call told its round, the warm-up's being round 0.
fn time_once<T>(mut f: impl FnMut(usize) -> T) -> Vec<Duration> {
    warm(|| f(0));
    let mut out = Vec::with_capacity(ROUNDS);
    let mut held: Option<T> = None;
    for round in 0..ROUNDS {
        let t = Instant::now();
        let made = std::hint::black_box(f(round));
        out.push(t.elapsed() * BATCH as u32);
        held = Some(made);
    }
    drop(held);
    out
}

// latency.c's twin, row for row and timed the same way, so the gate can hold C's rows to these.
#[test]
#[ignore = "a measurement, not an assertion: run it explicitly"]
fn api_latency() {
    let bytes = font_bytes();
    let font = Font::from_bytes(&bytes).expect("the fixture parses");
    let gid = font.glyph_id('g').expect("a glyph for 'g'");

    println!("\ndaegun – Rust API, {ROUNDS} rounds after {WARMUP} warmup and {} ms\n", SETTLE.as_millis());

    // Where an open's bytes lie in memory moves its best by as much as 16%, and one copy fixes that for a whole
    // run: the best over 20 copies holds within 1% from run to run.
    let copies: Vec<Vec<u8>> = (0..PLACES).map(|_| bytes.clone()).collect();
    let per_copy = ROUNDS / PLACES;
    report("font_open", time_once(|round| Font::from_bytes(&copies[round / per_copy]).expect("parses")));
    drop(copies);
    // Each group's buffers copied just before it opens, as C's row does, so long-evicted bytes don't time cache misses.
    // Eight opens a sample, as one alone reads in whole ticks of the 41.67 ns clock; a Vec keeps them unmoved.
    {
        const GROUP: usize = 8;
        warm(|| Font::from_vec(bytes.clone(), 0).expect("parses"));
        let mut samples = Vec::with_capacity(ROUNDS);
        let mut held = Vec::new();
        for _ in 0..ROUNDS {
            let bufs: [Vec<u8>; GROUP] = core::array::from_fn(|_| bytes.clone());
            let mut made = Vec::with_capacity(GROUP);
            let t = Instant::now();
            for buf in bufs {
                made.push(std::hint::black_box(Font::from_vec(buf, 0).expect("parses")));
            }
            samples.push(t.elapsed() * BATCH as u32 / GROUP as u32);
            held = made;
        }
        drop(held);
        report("font_open_owned", samples);
    }

    report("units_per_em", time(|| font.units_per_em()));
    report("glyph_id", time(|| font.glyph_id('g')));
    report("advance", time(|| font.advance(gid, &Location::default(), Orientation::Horizontal)));
    let bold = crate::located(&font, &[("wght", 700.0)]);
    report("advance at wght 700", time(|| font.advance(gid, &bold, Orientation::Horizontal)));

    report(
        "outline",
        time(|| {
            let mut c = Count::default();
            let _ = font.outline(gid, &Location::default(), opaque(&mut c));
            c.0
        }),
    );

    report("ascender", time(|| font.ascender(&Location::default())));
    report("descender", time(|| font.descender(&Location::default())));
    report("cap_height", time(|| font.cap_height(&Location::default())));
    report("glyph_count", time(|| font.glyph_count()));
    report("line_metrics", time(|| font.line_metrics(&Location::default(), Orientation::Horizontal)));

    report("quads", time(|| font.quads(gid, &Location::default())));
    let opts = OutlineOptions::default();
    report(
        "prepared_outline",
        time(|| {
            let mut c = Count::default();
            let _ = font.prepared_outline(gid, &Location::default(), 16.0, &opts, opaque(&mut c));
            c.0
        }),
    );
    let mut path = Path::default();
    font.outline(gid, &Location::default(), &mut path).expect("the glyph draws");
    let max_area = daegun::outline::max_area_for(16.0, f32::from(font.units_per_em()));
    report("flatten", time(|| path.flatten(max_area)));
    let mut r = Rasterizer::new();
    let options = RasterOptions::DEFAULT;
    report("raster_glyph", time(|| r.glyph(&font, gid, &Location::default(), 16.0, &options).map(|i| i.pixels.len())));
    println!();
}
