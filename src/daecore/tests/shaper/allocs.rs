// Allocations through the crate's counting allocator, which release builds have under `--cfg daegun_counting`.
use std::hint::black_box;
use std::time::Instant;
use alloc::vec;
use alloc::vec::Vec;

use crate::allocations::{set_counting, tally};
use crate::daecore::daekit::cache::FontCache;
use crate::daecore::daeshaper::{face::Face, pipeline, plan::Plan, run::Run};
use crate::daecore::daetype::decoder::{extract_ttc_tables, extract_ttf_tables};
use crate::shape::ShapeOptions;
use crate::{Cache, Font, Location, Tag};

use super::bench::args;

#[derive(Clone, Copy)]
struct Take {
    ns: u64,
    count: usize,
    bytes: usize,
    live: isize,
}

fn take<T>(f: impl FnOnce() -> T) -> (T, Take) {
    let (c, b, l) = tally();
    let t = Instant::now();
    let out = f();
    let ns = t.elapsed().as_nanos() as u64;
    let (c2, b2, l2) = tally();
    (out, Take { ns, count: c2 - c, bytes: b2.wrapping_sub(b), live: l2.wrapping_sub(l) as isize })
}

fn row(what: &str, takes: &mut [Take]) {
    takes.sort_by_key(|t| t.ns);
    let t = takes[takes.len() / 2];
    println!("  {what:<28} {:>11.1} us  {:>7} allocs  {:>11} B  {:>+11} B held", t.ns as f64 / 1000.0, t.count, t.bytes,
             t.live);
}

fn plan_for(face: &Face<'_>, text: &str) -> Plan {
    let mut probe = Run::default();
    probe.push_str(text);
    let direction = pipeline::settle_script(&mut probe);
    let tags = probe.script.map(crate::daecore::daeshaper::ot::tag::script_tags);
    let tags = tags.as_ref().map_or(&[][..], |t| t.as_slice());
    crate::tests::plain_plan(face, probe.script, direction, tags)
}

// Allocations, bytes and time of a face's first uses and of a warm shape. Single shots are repeated on fresh fonts
// and the median taken; counts are exact. Arguments: font text [face-index] [axis=value...].
#[test]
#[ignore]
fn abench() {
    let a = args();
    let bytes = std::fs::read(&a[0]).expect("font");
    let text = a[1].as_str();
    let index: usize = a.get(2).map_or(0, |s| s.parse().expect("index"));
    let coords: Vec<(Tag, f64)> = a.iter().skip(3).map(|s| {
        let (t, v) = s.split_once('=').expect("axis=value");
        (t.parse().expect("tag"), v.parse().expect("value"))
    }).collect();
    let ttc = bytes.starts_with(b"ttcf");
    let tables = || if ttc { extract_ttc_tables(&bytes, index) } else { extract_ttf_tables(&bytes) }.expect("parse");
    const N: usize = 15;
    set_counting(true);

    // The core path: a cache, a face, a plan and shapes, as the latency bench runs them.
    let (mut load, mut face_t, mut plan_t, mut first, mut warm) = (vec![], vec![], vec![], vec![], vec![]);
    for _ in 0..N {
        let (fc, t) = take(|| FontCache::new(tables()).expect("load"));
        load.push(t);
        let (face, t) = take(|| Face::new(&fc));
        face_t.push(t);
        let (plan, t) = take(|| plan_for(&face, text));
        plan_t.push(t);
        let shape_once = || {
            let mut buffer = Run::default();
            buffer.push_str(text);
            let target = pipeline::settle_script(&mut buffer);
            take(|| { pipeline::shape(&face, &plan, &mut buffer, target, &[]); }).1
        };
        first.push(shape_once());
        for _ in 0..100 { shape_once(); }
        warm.push(shape_once());
    }
    println!("core, default location");
    row("cache (FontCache::new)", &mut load);
    row("face (Face::new)", &mut face_t);
    row("plan", &mut plan_t);
    row("first shape", &mut first);
    row("warm shape", &mut warm);

    // The public path, its run cache off, at the default and, if given, at the location.
    let open = || {
        let font = if ttc { Font::from_collection(&bytes, index) } else { Font::from_bytes(&bytes) }.expect("load");
        font.set_cache_budget(Cache::Shape, 0);
        font
    };
    let opts = ShapeOptions::default();
    for label in ["default", "location"] {
        if label == "location" && coords.is_empty() { break; }
        let (mut first, mut warm) = (vec![], vec![]);
        for _ in 0..N {
            let font = open();
            let at = if label == "location" { font.location(&coords) } else { Location::default() };
            first.push(take(|| { black_box(font.shape(text, &at, &opts)); }).1);
            for _ in 0..100 { black_box(font.shape(text, &at, &opts)); }
            warm.push(take(|| { black_box(font.shape(text, &at, &opts)); }).1);
        }
        println!("public shape, run cache off, {label}");
        row("first shape", &mut first);
        row("warm shape", &mut warm);
    }
    set_counting(false);
}
