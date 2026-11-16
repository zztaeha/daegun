use std::time::Instant;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::daecore::daekit::cache::FontCache;
use crate::daecore::daeshaper::{face::Face, pipeline, plan::Plan, run::Run};
use crate::layout::LayoutOptions;
use crate::shape::{Direction, ShapeOptions};
use crate::{Cache, Font, Location, Tag};

use super::bench::{args, cache};

fn plan_for<'f>(face: &Face<'f>, text: &str) -> Plan {
    let mut probe = Run::default();
    probe.push_str(text);
    let direction = pipeline::settle_script(&mut probe);
    let tags = probe.script.map(crate::daecore::daeshaper::ot::tag::script_tags);
    let tags = tags.as_ref().map_or(&[][..], |t| t.as_slice());
    crate::tests::plain_plan(face, probe.script, direction, tags)
}

// `iters` core shapes on one plan, a fresh buffer each; the glyphs shaped.
fn core_shapes(fc: &FontCache, plan: &Plan, text: &str, iters: usize) -> usize {
    let face = Face::new(fc);
    let mut n = 0;
    for _ in 0..iters {
        let mut buffer = Run::default();
        buffer.push_str(text);
        let target = pipeline::settle_script(&mut buffer);
        pipeline::shape(&face, plan, &mut buffer, target, &[]);
        n += crate::tests::placed(&buffer).len();
    }
    n
}

fn scale(name: &str, rel: &str, text: &str, iters: usize) {
    let fc = cache(&format!("{}/{rel}", super::FONTS), 0);
    let plan = plan_for(&Face::new(&fc), text);
    core_shapes(&fc, &plan, text, iters);
    let mut line = format!("{name:<11}");
    for threads in [1usize, 2, 4, 8] {
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t = Instant::now();
            std::thread::scope(|s| {
                let hs: Vec<_> = (0..threads).map(|_| s.spawn(|| core_shapes(&fc, &plan, text, iters))).collect();
                hs.into_iter().for_each(|h| { std::hint::black_box(h.join().unwrap()); });
            });
            best = best.min(t.elapsed().as_nanos() as f64 / iters as f64);
        }
        line += &format!("  {threads}t {best:>8.0}");
    }
    eprintln!("{line}");
}

// N threads shaping one plan, ns a shape per thread (wall time / shapes per thread), best of 5 rounds.
#[test]
#[ignore]
fn thread_scaling() {
    let k = std::env::var("DAEGUN_BENCH_SCALE").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(1);
    scale("inter", "inter/InterVariable.ttf",
        "Typography is the craft of endowing human language with a durable visual form.", 20_000 * k);
    scale("arabic", "scheherazade-new/ScheherazadeNew-Regular.ttf", "السلام عليكم ورحمة الله وبركاته", 15_000 * k);
    scale("devanagari", "noto-devanagari/NotoSansDevanagari.ttf", "हिन्दी भाषा में स्वागत है", 8_000 * k);
    scale("eb", "eb-garamond/EBGaramond.ttf", "The office staff shuffled a fistful of waffles.", 20_000 * k);
    scale("cjk", "source-han-sans/SourceHanSansJP-VF.otf",
        "日本語の組版では、文字の間隔と行の組み方が読みやすさを決める。", 120_000 * k);
}

// The scheduler then keeps the thread on performance cores while any is free; affinity can't be set on this chip.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "the QoS class is only a C call away")]
fn interactive() {
    unsafe extern "C" {
        fn pthread_set_qos_class_self_np(class: u32, priority: i32) -> i32;
    }
    unsafe { pthread_set_qos_class_self_np(0x21, 0) };
}

#[cfg(not(target_os = "macos"))]
fn interactive() {}

// The thread's shapes, and its P-core cycles, or None if it ran on an efficiency core.
fn counted(run: impl FnOnce() -> usize) -> (usize, Option<u64>) {
    interactive();
    let before = super::core_counts().unwrap_or_default();
    let n = run();
    let after = super::core_counts().unwrap_or_default();
    (n, (after[1] == before[1]).then(|| after[0][1] - before[0][1]))
}

fn scaling(label: &str, iters: usize, run: impl Fn(usize) -> usize + Sync) {
    run(0);
    let mut line = format!("{label:<22}");
    for threads in [1usize, 2, 4, 5, 6, 8] {
        let (mut best, mut cycles) = (f64::MAX, None::<f64>);
        for _ in 0..5 {
            let t = Instant::now();
            let counts: Vec<_> = std::thread::scope(|s| {
                let run = &run;
                let hs: Vec<_> = (0..threads).map(|i| s.spawn(move || counted(|| run(i)))).collect();
                hs.into_iter().map(|h| h.join().unwrap()).collect()
            });
            best = best.min(t.elapsed().as_nanos() as f64 / iters as f64);
            std::hint::black_box(&counts);
            if let Some(c) = counts.iter().try_fold(0, |m, &(_, c)| Some(m.max(c?))) {
                let c = c as f64 / iters as f64;
                cycles = Some(cycles.map_or(c, |b| b.min(c)));
            }
        }
        let cycles = cycles.map_or("-".to_string(), |c| format!("{c:.0}"));
        line += &format!("  {threads}t {best:>8.0} c {cycles:>8}");
    }
    println!("{line}");
}

// The run cache, warm: every call a hit, and every layout's runs.
fn cached(font: &Font, text: &str, iters: usize) {
    let (at, opts) = (Location::default(), ShapeOptions::default());
    let own: Vec<String> = (0..8).map(|i| format!("{text} {i}")).collect();
    own.iter().for_each(|t| { let _ = font.shape(t, &at, &opts); });
    scaling("cached, one text", iters, |_| (0..iters).map(|_| font.shape(text, &at, &opts).glyphs.len()).sum());
    scaling("cached, own text", iters, |i| (0..iters).map(|_| font.shape(&own[i], &at, &opts).glyphs.len()).sum());
    // A twentieth of the shapes' count, a layout taking some hundred hits' time; each thread its own options, as a
    // segmenter isn't shared between threads.
    let layouts = iters / 20;
    scaling("cached layout", layouts, |_| {
        let opts = LayoutOptions::default();
        (0..layouts).map(|_| font.layout(text, &at, &opts).lines.len()).sum()
    });
    // Each thread's text with its own digit after every character, so no piece or its context is another thread's.
    let apart: Vec<String> = (0..8u8).map(|i| text.chars().flat_map(|c| [c, char::from(b'0' + i)]).collect()).collect();
    apart.iter().for_each(|t| { let _ = font.layout(t, &at, &LayoutOptions::default()); });
    scaling("cached layout, own text", layouts, |i| {
        let opts = LayoutOptions::default();
        (0..layouts).map(|_| font.layout(&apart[i], &at, &opts).lines.len()).sum()
    });
}

// N threads shaping, best of 5 rounds: ns a shape per thread and cycles on the slowest thread's performance cores, down
// the core path, the public path, a Font each, the run cache and layouts. Args: font text iters [axis=value...].
#[test]
#[ignore]
fn tbench() {
    let a = args();
    let (path, text, iters) = (&a[0], a[1].as_str(), a[2].parse::<usize>().expect("iters"));
    let coords: Vec<(Tag, f64)> = a[3..].iter().map(|s| {
        let (t, v) = s.split_once('=').expect("axis=value");
        (t.parse().expect("tag"), v.parse().expect("value"))
    }).collect();

    let fc = cache(path, 0);
    let plan = plan_for(&Face::new(&fc), text);
    scaling("core, one plan", iters, |_| core_shapes(&fc, &plan, text, iters));
    let bytes = std::fs::read(path).expect("font");
    let open = || {
        let font = Font::from_bytes(&bytes).expect("load");
        font.set_cache_budget(Cache::Shape, 0);
        font
    };
    let opts = ShapeOptions::default();
    let shape_at = |font: &Font, at: &Location| font.shape(text, at, &opts).glyphs.len();
    let font = open();
    let (default, at) = (Location::default(), font.location(&coords));
    let cached_font = Font::from_bytes(&bytes).expect("load");
    if coords.is_empty() {
        scaling("public, default", iters, |_| (0..iters).map(|_| shape_at(&font, &default)).sum());
        return cached(&cached_font, text, iters);
    }
    let key = fc.axis_key(&coords);
    scaling("core, location", iters, |_| {
        (0..iters).map(|_| crate::text::shape::shape_run(&fc, &key, text, false, None, &opts).glyphs.len()).sum()
    });
    scaling("public, default", iters, |_| (0..iters).map(|_| shape_at(&font, &default)).sum());
    scaling("public, location", iters, |_| (0..iters).map(|_| shape_at(&font, &at)).sum());
    // Vertical, whose origins a font without vmtx measures glyph by glyph at the location.
    let down_opts = ShapeOptions::default().with_direction(Direction::TopToBottom);
    let down = |font: &Font| font.shape(text, &at, &down_opts).glyphs.len();
    scaling("public, location, down", iters, |_| (0..iters).map(|_| down(&font)).sum());
    let fonts: Vec<Font> = (0..8).map(|_| open()).collect();
    let ats: Vec<Location> = fonts.iter().map(|f| f.location(&coords)).collect();
    fonts.iter().zip(&ats).for_each(|(f, at)| { shape_at(f, at); });
    scaling("own Font, location", iters, |i| (0..iters).map(|_| shape_at(&fonts[i], &ats[i])).sum());
    // Each thread 25 units off the stated values, so no two share a location.
    let apart: Vec<Location> = (0..8)
        .map(|i| font.location(&coords.iter().map(|&(t, v)| (t, v - 25.0 * i as f64)).collect::<Vec<_>>()))
        .collect();
    apart.iter().for_each(|at| { shape_at(&font, at); });
    scaling("own location each", iters, |i| (0..iters).map(|_| shape_at(&font, &apart[i])).sum());
    cached(&cached_font, text, iters);
}
