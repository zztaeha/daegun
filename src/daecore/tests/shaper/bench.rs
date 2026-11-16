// The research benches that time the core shaper, run from the library's test binary since the core is private:
// `tasks/tools/perf/libbench.zsh` builds it and runs one, its arguments in DAEGUN_BENCH_ARGS, one a line.
use std::hint::black_box;
use std::time::Instant;
use alloc::string::String;
use alloc::vec::Vec;

use crate::daecore::daekit::cache::FontCache;
use crate::daecore::daeshaper::{face::Face, pipeline, run::{Direction, Run}};
use crate::daecore::daetype::decoder::{extract_ttc_tables, extract_ttf_tables};

pub(super) fn args() -> Vec<String> {
    let all = std::env::var("DAEGUN_BENCH_ARGS").expect("DAEGUN_BENCH_ARGS, one argument a line");
    all.lines().map(String::from).collect()
}

pub(super) fn cache(path: &str, index: usize) -> FontCache {
    let bytes = std::fs::read(path).expect("font");
    let map = if bytes.starts_with(b"ttcf") { extract_ttc_tables(&bytes, index) } else { extract_ttf_tables(&bytes) };
    FontCache::new(map.expect("parse")).expect("load")
}

#[inline(never)]
fn stack_here() -> usize {
    let x = 0u8;
    black_box(&x) as *const u8 as usize
}

#[inline(never)]
fn shim<const K: usize>(f: &mut dyn FnMut()) {
    black_box(&[0u8; K]);
    f();
    black_box(());
}

#[inline(never)]
fn descend(depth: usize, shim16: usize, f: &mut dyn FnMut()) {
    if depth > 0 {
        descend(depth - 1, shim16, f)
    } else {
        match shim16 {
            0 => shim::<0>(f),
            1 => shim::<16>(f),
            2 => shim::<32>(f),
            _ => shim::<48>(f),
        }
    }
    black_box(());
}

#[inline(never)]
fn stack_at(depth: usize, shim16: usize) -> usize {
    let mut at = 0;
    descend(depth, shim16, &mut || at = stack_here());
    at
}

// Runs `f` with the stack PAD's length lower, as the run scripts' environment pad shifted a binary's main thread: a
// test runs on a thread of its own, which no environment reaches. Frames vary by build, so each depth is measured.
pub(super) fn padded(f: &mut dyn FnMut()) {
    let bytes = std::env::var_os("PAD").map_or(0, |p| p.len());
    let base = stack_at(0, 0);
    let (depth, shim16) = (0..=bytes / 16)
        .flat_map(|d| (0..4).map(move |k| (d, k)))
        .find(|&(d, k)| base - stack_at(d, k) == bytes)
        .unwrap_or_else(|| panic!("no frame depth lowers the stack by {bytes} bytes (a debug build?)"));
    descend(depth, shim16, f);
}

// This thread's instructions over both core types, 0 where the kernel refuses.
fn instructions() -> u64 {
    super::core_counts().map_or(0, |c| c[0][0] + c[1][0])
}

// shape() on a fresh buffer per iteration, filled and guessed outside the timed region, with one plan made beforehand,
// 1,000 warm-up shapes and the median of the rest, so cold caches and outliers don't count. Down the page if `down`.
fn shape_bench(down: bool) {
    let a = args();
    let (text, iters) = (&a[1], a[2].parse::<usize>().expect("iters"));
    let fc = cache(&a[0], a.get(4).map_or(0, |s| s.parse().expect("index")));
    // NOINDEX shapes without the cmap, coverage and class indexes, searching instead, to time the searches alone.
    if std::env::var_os("NOINDEX").is_some() {
        fc.set_cmap_index_allowance(0);
    }
    let face = Face::new(&fc);

    let mut probe = Run::default();
    probe.push_str(text);
    let guessed = pipeline::settle_script(&mut probe);
    let direction = if down { Direction::TopToBottom } else { guessed };
    let tags = probe.script.map(crate::daecore::daeshaper::ot::tag::script_tags);
    let tags = tags.as_ref().map_or(&[][..], |t| t.as_slice());
    let plan = crate::tests::plain_plan(&face, probe.script, direction, tags);

    let (mut samples, mut expect, mut start) = (Vec::with_capacity(iters), 0, 0);
    padded(&mut || {
        for i in -1000..iters as i64 {
            if i == 0 {
                start = instructions();
            }
            let mut buffer = Run::default();
            buffer.push_str(text);
            let guessed = pipeline::settle_script(&mut buffer);
            let target = if down { direction } else { guessed };
            let t = Instant::now();
            pipeline::shape(&face, &plan, &mut buffer, target, &[]);
            let ns = t.elapsed().as_nanos() as u64;
            let n = crate::tests::placed(&buffer).len();
            black_box(&buffer);
            if i < 0 {
                expect = n;
            } else {
                assert_eq!(n, expect, "length moved");
                samples.push(ns);
            }
        }
    });
    let insn = (instructions() - start) / iters as u64;
    samples.sort_unstable();
    println!(
        "{:<12} dg  n {iters}  glyphs {expect}  min {} ns  median {} ns  p95 {} ns  insn {insn}",
        a.get(3).map_or("", |s| s.as_str()),
        samples[0],
        samples[iters / 2],
        samples[(iters as f64 * 0.95) as usize],
    );
}

// Arguments: font text iters [name] [face-index].
#[test]
#[ignore]
fn sbench() {
    shape_bench(false);
}

// sbench down the page, top to bottom as the public path asks; the same arguments.
#[test]
#[ignore]
fn vbench() {
    shape_bench(true);
}
