#![allow(unsafe_code, reason = "a counting allocator wraps the system's")]
#![allow(clippy::disallowed_methods, reason = "a test reads its own thread's counts while the thread runs")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use alloc::vec::Vec;

use crate::color::SceneOptions;
use crate::outline::{HintMode, OutlineOptions, OutlinePen};
use crate::shape::{Feature, ShapeOptions};
use crate::unicode::BaseDirection;
use crate::{Cache, CacheStats, Font, GlyphId, Location, Orientation, Tag};

// Stores nothing, so what it counts is the extraction and not a pen filling its own buffers.
#[derive(Default)]
struct Sink(usize);

impl OutlinePen for Sink {
    fn move_to(&mut self, _: f32, _: f32) { self.0 += 1 }
    fn line_to(&mut self, _: f32, _: f32) { self.0 += 1 }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) { self.0 += 1 }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) { self.0 += 1 }
    fn close(&mut self) { self.0 += 1 }
}

const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts");

// A location from tags written as text, as the tests name them.
fn located(font: &crate::Font, axes: &[(&str, f64)]) -> crate::Location {
    let coords: Vec<(crate::Tag, f64)> = axes.iter().map(|&(tag, v)| (tag.parse().expect("a tag"), v)).collect();
    font.location(&coords)
}

// Each thread counts its own: the crate's other tests run beside these, and a finished test's thread can still be
// winding down, allocating.
thread_local! {
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    // Bytes held, and the most held since a test last reset it to what was held then.
    static LIVE: Cell<usize> = const { Cell::new(0) };
    static PEAK: Cell<usize> = const { Cell::new(0) };
    // Bytes ever allocated, which shows a copy made and freed again and again where the two above don't.
    static TOTAL: Cell<usize> = const { Cell::new(0) };
    static ODD_FREES: Cell<usize> = const { Cell::new(0) };
}

// Frees of one size nothing else here allocates, counted always, so a test can see its own buffer go.
const ODD_SIZE: usize = 12_345;

// Refused by the allocator below, as a real one refuses what it cannot find, so a failing allocation
// can be tested without really running out, on 32-bit targets too.
const FAIL_SIZE: usize = 1 << 30;


// The allocator's own counting, which a thread whose locals are gone skips.
fn counted() {
    let _ = COUNTING.try_with(|on| on.get().then(|| ALLOCS.try_with(|n| n.set(n.get() + 1))));
}

fn grew(by: usize) {
    let _ = TOTAL.try_with(|t| t.set(t.get().wrapping_add(by)));
    let _ = LIVE.try_with(|l| {
        l.set(l.get().wrapping_add(by));
        PEAK.try_with(|p| p.set(p.get().max(l.get())))
    });
}

fn shrank(by: usize) {
    let _ = LIVE.try_with(|l| l.set(l.get().wrapping_sub(by)));
}

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        counted();
        if l.size() == FAIL_SIZE {
            return std::ptr::null_mut();
        }
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            grew(l.size());
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if l.size() == ODD_SIZE {
            let _ = ODD_FREES.try_with(|n| n.set(n.get() + 1));
        }
        shrank(l.size());
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        counted();
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            shrank(l.size());
            grew(n);
        }
        q
    }
}

#[global_allocator]
static A: Counting = Counting;

const ROUNDS: usize = 200;

fn font() -> Font {
    let path = format!("{FONTS}/inter/InterVariable.ttf");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_vec(bytes, 0).expect("parses")
}

// This thread's allocations while counting, its bytes ever allocated and its bytes held, for the benches.
pub(crate) fn tally() -> (usize, usize, usize) {
    (ALLOCS.get(), TOTAL.get(), LIVE.get())
}

pub(crate) fn set_counting(on: bool) {
    COUNTING.set(on);
}

fn allocs_per_call(f: impl FnMut()) -> f64 {
    allocs_over(ROUNDS, f)
}

fn allocs_over(rounds: usize, mut f: impl FnMut()) -> f64 {
    for _ in 0..8 {
        f();
    }
    ALLOCS.set(0);
    COUNTING.set(true);
    for _ in 0..rounds {
        f();
    }
    COUNTING.set(false);
    ALLOCS.get() as f64 / rounds as f64
}

// A run shaped into again allocates nothing once it has the room, with options the cache doesn't hold and at a
// location: the shaper's buffer and plan are the font's, kept between calls.
#[test]
fn a_run_shaped_into_with_room_allocates_nothing() {
    let font = font();
    let bold = located(&font, &[("wght", 700.0)]);
    let liga = [Feature::new(Tag::new(b"liga"), 0)];
    let reports = ShapeOptions::default().with_report_unsafe_to_concat(true).with_features(&liga);
    let mut run = crate::shape::ShapedRun::default();
    for (at, opts) in [(&Location::default(), ShapeOptions::default()), (&bold, reports)] {
        let n = allocs_per_call(|| font.shape_into("your all-in-one text engine", at, &opts, &mut run));
        assert_eq!(n, 0.0, "shaping into a run with room allocated {n} times with {opts:?}");
    }
}

// A shape cache hit allocates nothing, at a location too: its key was built with the location, and the run's own is
// only borrowed.
#[test]
fn hot_paths_stay_within_their_allocation_budget() {
    let font = font();
    let text = "your all-in-one text engine";

    let bold = located(&font, &[("wght", 700.0)]);
    let with_axes = allocs_per_call(|| {
        std::hint::black_box(font.shape(text, &bold, &ShapeOptions::default()));
    });
    let without = allocs_per_call(|| {
        std::hint::black_box(font.shape(text, &Location::default(), &ShapeOptions::default()));
    });

    let gid = font.glyph_id('a').expect("font has a");
    let outline = allocs_per_call(|| {
        let mut pen = Sink::default();
        std::hint::black_box(font.outline(gid, &Location::default(), &mut pen)).expect("a draws");
    });

    assert_eq!(with_axes, 0.0, "a shape cache hit at a location allocated {with_axes} times");
    assert_eq!(without, 0.0, "an axis-free shape cache hit allocated");
    assert!(outline <= 2.0, "outline extraction allocated {outline} times, budget is 2");

    let opts = OutlineOptions::default();
    let prepared = allocs_per_call(|| {
        let mut pen = Sink::default();
        let _ = std::hint::black_box(font.prepared_outline(gid, &Location::default(), 40.0, &opts, &mut pen));
    });
    let quads = allocs_per_call(|| {
        let _ = std::hint::black_box(font.quads(gid, &Location::default()));
    });
    let mut kept_quads = Vec::new();
    let quads_into = allocs_per_call(|| {
        font.quads_into(gid, &Location::default(), &mut kept_quads).expect("a has curves");
        std::hint::black_box(&kept_quads);
    });
    let mut path = crate::outline::Path::default();
    font.outline(gid, &Location::default(), &mut path).expect("a has an outline");
    let max_area = crate::outline::max_area_for(40.0, f32::from(font.units_per_em()));
    let flat = allocs_per_call(|| {
        let _ = std::hint::black_box(path.flatten(max_area));
    });
    // Lists kept between calls are reused, each emptied, so flattening the same glyph again allocates nothing.
    let mut kept = vec![vec![crate::outline::Point { x: 9.0, y: 9.0 }; 3]; 7];
    let flat_into = allocs_per_call(|| {
        path.flatten_into(max_area, &mut kept).expect("flattens");
        std::hint::black_box(&kept);
    });
    assert_eq!(kept, path.flatten(max_area).expect("flattens"), "kept lists flatten as fresh ones");
    // Bytecode hinting reuses its context's buffers, so a hinted prepared outline allocates no more than an unhinted
    // one, though Irianis's programs loop through IP, SHP and ALIGNRP; a hinted glyph alone allocates its 4 vectors.
    let irianis = std::fs::read(format!("{FONTS}/Irianisadfstd/IrianisadfstdRegular.ttc")).expect("fixture");
    let irianis = Font::from_collection(&irianis, 0).expect("face 0");
    let hinting = OutlineOptions::default().with_hinting(HintMode::Font);
    for c in ['a', 'g', 'm'] {
        let gid = irianis.glyph_id(c).expect("the face has the letter");
        let prepare = |opts: &OutlineOptions| {
            allocs_per_call(|| {
                let _ = std::hint::black_box(irianis.prepared_outline(
                    gid,
                    &Location::default(),
                    16.0,
                    opts,
                    &mut Sink::default(),
                ));
            })
        };
        let through_path = OutlineOptions::default().with_transform([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        let (hinted, unhinted) = (prepare(&hinting), prepare(&through_path));
        let alone = allocs_per_call(|| {
            let _ = std::hint::black_box(irianis.hinted_outline(gid, &Location::default(), 16.0, HintMode::Font));
        });
        assert!(hinted <= unhinted, "'{c}' prepared hinted allocated {hinted} times, unhinted {unhinted}");
        assert!(alone <= 4.0, "'{c}' hinted alone allocated {alone} times, budget is 4");
    }
    assert_eq!(prepared, 0.0, "a plain prepared outline allocated {prepared} times");
    assert!(quads <= 2.0, "quads allocated {quads} times, budget is 2");
    assert_eq!(quads_into, 0.0, "quads into a kept list allocated {quads_into} times");
    assert!(flat <= 10.0, "flattening allocated {flat} times, budget is 10");
    assert_eq!(flat_into, 0.0, "flattening into kept lists allocated {flat_into} times");
}

#[cfg(feature = "capi")]
unsafe extern "C" {
    fn daegun_font_buffer_new(len: usize) -> *mut u8;
    fn daegun_font_open_owned(data: *mut u8, len: usize, index: usize, out: *mut *mut core::ffi::c_void) -> i32;
    fn daegun_general_category(codepoint: u32, out: *mut i32) -> i32;
}

// The header promises NULL when the allocation fails, where an allocation that cannot fail would abort.
#[cfg(feature = "capi")]
#[test]
fn a_buffer_the_allocator_refuses_is_null() {
    assert!(unsafe { daegun_font_buffer_new(FAIL_SIZE) }.is_null(), "a buffer the allocator refused was handed out");
}

// A failure writes its reason over the last in the thread's kept buffer, so one that fits allocates nothing.
#[cfg(feature = "capi")]
#[test]
fn a_refused_call_allocates_nothing_once_its_reason_fits() {
    let mut category = 0;
    let refused = allocs_per_call(|| {
        std::hint::black_box(unsafe { daegun_general_category(0xD800, &mut category) });
    });
    assert_eq!(refused, 0.0, "a refused codepoint allocated {refused} times");
}

// The header says daegun_font_open_owned takes the buffer whatever it returns, so a caller who passes
// a NULL out and lets go of the buffer must not leak it.
#[cfg(feature = "capi")]
#[test]
fn an_owned_buffer_is_freed_even_when_out_is_null() {
    let before = ODD_FREES.get();
    unsafe {
        let buf = daegun_font_buffer_new(ODD_SIZE);
        assert!(!buf.is_null(), "no buffer");
        daegun_font_open_owned(buf, ODD_SIZE, 0, core::ptr::null_mut());
    }
    assert_eq!(ODD_FREES.get() - before, 1, "open_owned with a NULL out kept the buffer");
}

// A curve flattens to as many as 256 points, so 4,500 of them take 1.15 million flat, past MAX_FLATTEN_POINTS, and a
// stroke built on them whole three times that. The stroke streams instead, holding a point a segment past its 2^16.
#[test]
fn a_stroke_streams_past_the_point_cap_holding_a_point_a_segment() {
    let mut path = crate::outline::Path::default();
    path.move_to(0.0, 0.0);
    for i in 0..4_500 {
        let (a, b) = if i % 2 == 0 { (0.0, 100.0) } else { (100.0, 0.0) };
        path.curve_to(a * 0.7 + b * 0.3, 100.0, a * 0.3 + b * 0.7, -100.0, b, 0.0);
    }
    let style =
        crate::outline::StrokeStyle { width: 1.0, join: crate::outline::Join::Bevel, cap: crate::outline::Cap::Butt };
    let mut pen = Sink::default();
    let before = LIVE.get();
    PEAK.set(before);
    path.stroke(&style, 0.001, &mut pen).expect("a stroke of any size draws");
    let held = PEAK.get() - before;
    assert!(pen.0 > crate::outline::MAX_FLATTEN_POINTS, "the stroke drew {} points", pen.0);
    assert!(held < 1 << 20, "stroking held {} KB at its peak", held >> 10);
}

// A pair-kerning class matrix of 2,001 by 2,001 classes, its grid cut off after two cells. The
// subsetter reads only cells that fit in the table, so it must not reserve room for four million.
fn gpos_with_classes(n: u16) -> Vec<u8> {
    let (cov, cd) = (20u16, 24 + 2 * n);
    let mut sub: Vec<u16> = vec![2, cov, 4, 0, cd, cd, n + 1, n + 1, 0, 0, 1, n];
    sub.extend(1..=n);
    sub.extend([2, n]);
    sub.extend((1..=n).flat_map(|g| [g, g, g]));
    let words = [vec![1u16, 0, 0, 0, 10, 1, 4, 2, 0, 1, 8], sub].concat();
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

#[test]
fn a_huge_class_matrix_is_not_reserved_up_front() {
    let f = font();
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"GPOS"), gpos_with_classes(2000));
    let hostile = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    let gids: Vec<GlyphId> = (0..=2000).map(GlyphId).collect();
    let before = LIVE.get();
    PEAK.set(before);
    let _ = hostile.subset(&gids, &Location::default());
    let held = PEAK.get() - before;
    assert!(held < 64 << 20, "subsetting held {} MB at its peak", held >> 20);
}

// A PairPos 2 of `classes` by `classes` classes, glyph g in class g on both sides, kerning 2 bytes a
// cell, its coverage and class definitions after the grid.
fn gpos_with_grid(classes: u16) -> Vec<u8> {
    let after = 16 + 2 * classes * classes;
    let class_def = [vec![1, 1, classes - 1], (1..classes).collect()].concat();
    let mut sub: Vec<u16> = vec![2, after, 4, 0, after + 10, after + 10, classes, classes];
    sub.extend((0..classes * classes).map(|c| c % 7));
    sub.extend([2, 1, 1, classes - 1, 0]);
    sub.extend(class_def);
    let words = [vec![1u16, 0, 0, 0, 10, 1, 4, 2, 0, 1, 8], sub].concat();
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

// What subsetting with the grid of `classes` holds at its peak, every glyph kept.
fn peak_with_grid(f: &Font, classes: u16) -> usize {
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"GPOS"), gpos_with_grid(classes));
    let patched = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    let gids: Vec<GlyphId> = (0..patched.glyph_count()).map(GlyphId).collect();
    let before = LIVE.get();
    PEAK.set(before);
    let _ = patched.subset(&gids, &Location::default());
    PEAK.get() - before
}

// A grid of 150 by 150 classes is 45 KB, and each cell two 24-byte records. As a Struct, its Vec and
// two 80-byte values, a cell would be about 136 times the bytes it is read from.
#[test]
fn a_class_grid_holds_a_small_multiple_of_its_bytes() {
    let f = font();
    let grid = peak_with_grid(&f, 150).saturating_sub(peak_with_grid(&f, 2));
    let bytes = 2 * 150 * 150;
    assert!(grid < 32 * bytes, "the grid held {} times its bytes", grid / bytes);
}

// A SinglePos whose 6,000 records name one 16 KB Device table through all four device offsets.
// Copied at each naming, a subset keeping every glyph would hold 185 MB at its peak.
fn gpos_naming_one_device(n: u16) -> Vec<u8> {
    let device_at = 18 + 8 * n;
    let mut sub: Vec<u16> = vec![2, 8 + 8 * n, 0x00F0, n];
    sub.extend((0..4 * n).map(|_| device_at));
    sub.extend([2, 1, 0, n - 1, 0, 0, u16::MAX, 1]);
    sub.extend([0; 8192]);
    let words = [vec![1u16, 0, 0, 0, 10, 1, 4, 1, 0, 1, 8], sub].concat();
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

#[test]
fn a_device_named_many_times_is_not_copied_each_time() {
    let f = font();
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"GPOS"), gpos_naming_one_device(6000));
    let hostile = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    let gids: Vec<GlyphId> = (0..hostile.glyph_count()).map(GlyphId).collect();
    let before = LIVE.get();
    PEAK.set(before);
    let _ = hostile.subset(&gids, &Location::default());
    let held = PEAK.get() - before;
    assert!(held < 64 << 20, "subsetting held {} MB at its peak", held >> 20);
}

// Two combs of 14 teeth across each other resolve into hundreds of small contours. What comes back
// holds those points, not room for every edge the union had left when each contour began.
#[test]
fn resolved_contours_hold_only_their_points() {
    let comb = |across: bool| -> Vec<(f32, f32)> {
        let mut c = vec![(0.0, -2.0)];
        for k in 0..14 {
            let x = k as f32 * 4.0;
            c.extend([(x, 0.0), (x, 60.0), (x + 2.0, 60.0), (x + 2.0, 0.0)]);
        }
        c.push((56.0, -2.0));
        // Swapped and reversed, so both wind the same way and their crossings are covered twice, and moved
        // off the first comb's grid so no edge lies along another.
        if across { c.into_iter().rev().map(|(x, y)| (y + 1.0, x + 1.0)).collect() } else { c }
    };
    let combs = [comb(false), comb(true)].map(|c| c.into_iter().map(crate::outline::Point::from).collect::<Vec<_>>());
    let before = LIVE.get();
    let resolved = crate::outline::resolve_overlaps(&combs).expect("the combs overlap");
    let held = LIVE.get() - before;
    let points: usize = resolved.iter().map(Vec::len).sum();
    assert!(held < 64 * points + (1 << 16), "{} contours of {points} points hold {} KB", resolved.len(), held >> 10);
}

// At max_area 0 every curve becomes 4,097 points, so 30,000 of them would read 123 million before the
// final count refused them. The loop stops at twice the limit instead.
#[test]
fn flattening_stops_reading_past_the_point_cap() {
    let mut path = crate::outline::Path::default();
    path.move_to(0.0, 0.0);
    for i in 0..30_000 {
        let x = (i % 100) as f32 * 10.0;
        path.curve_to(x + 3.0, 50.0, x + 6.0, -50.0, x + 10.0, 0.0);
    }
    let before = LIVE.get();
    PEAK.set(before);
    let kind = path.flatten(0.0).map_err(|e| e.kind()).err();
    assert_eq!(kind, Some(crate::ErrorKind::LimitExceeded), "123 million points were flattened");
    let held = PEAK.get() - before;
    assert!(held < 64 << 20, "flattening held {} MB at its peak", held >> 20);
}

// 2,000 strikes naming one index record of a 5,000-glyph bitmap subtable: about 116 KB of CBLC that would
// rebuild 40 MB of index. The subset is refused, and without building it all first.
#[test]
fn a_bitmap_subset_stops_at_its_budget() {
    use crate::daecore::daetype::subsetter::{subset_bitmap_strikes, GlyphSet};
    const N: usize = 2_000;
    const GLYPHS: u16 = 5_000;
    let mut cblc = [0x0003_0000u32, N as u32].map(u32::to_be_bytes).concat();
    for _ in 0..N {
        cblc.extend([(8 + 48 * N) as u32, 0, 1].map(u32::to_be_bytes).concat());
        cblc.extend([0; 28]);
        cblc.extend([0, 0, 0xFF, 0xFF, 16, 16, 32, 1]);
    }
    cblc.extend([1u16, GLYPHS].map(u16::to_be_bytes).concat());
    cblc.extend(8u32.to_be_bytes());
    cblc.extend([1u16, 17].map(u16::to_be_bytes).concat());
    cblc.extend(4u32.to_be_bytes());
    cblc.extend((0..=u32::from(GLYPHS)).flat_map(u32::to_be_bytes));
    let mut cbdt = 0x0003_0000u32.to_be_bytes().to_vec();
    cbdt.resize(4 + usize::from(GLYPHS), 7);
    let mut active = GlyphSet::new();
    (1..=u32::from(GLYPHS)).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u32> = (0..=u32::from(GLYPHS)).collect();

    let before = LIVE.get();
    PEAK.set(before);
    assert!(subset_bitmap_strikes(&cblc, &cbdt, &active, &gid_map).is_none(), "the index was built");
    let held = PEAK.get() - before;
    assert!(held < 4 << 20, "subsetting the strike held {} MB at its peak", held >> 20);
}

// Offsets alternating over one 20 KB image, so every other glyph of 4,000 names the same bytes:
// 36 KB of table that, copied at each naming, would hold 77 MB (sbix) and 39 MB (CBLC) at the peak.
const OVERLAPPING: usize = 4_000;
const IMAGE: usize = 20_000;

fn peak_of(f: impl FnOnce() -> bool) -> usize {
    let before = LIVE.get();
    PEAK.set(before);
    assert!(f(), "the call measured did not return what the test expects");
    PEAK.get() - before
}

#[test]
fn an_sbix_strike_of_overlapping_records_stops_at_its_budget() {
    use crate::daecore::daetype::subsetter::subset_sbix;
    let head = 4 + 4 * (OVERLAPPING + 1);
    let mut sbix = [1u16, 1].map(u16::to_be_bytes).concat();
    sbix.extend([1u32, 12].map(u32::to_be_bytes).concat());
    sbix.extend([16u16, 72].map(u16::to_be_bytes).concat());
    (0..=OVERLAPPING).for_each(|g| sbix.extend(((head + IMAGE * (g % 2)) as u32).to_be_bytes()));
    sbix.extend(b"\0\0\0\0png ");
    sbix.resize(12 + head + IMAGE, 7);
    let active: Vec<u32> = (0..OVERLAPPING as u32).collect();
    let held = peak_of(|| subset_sbix(&sbix, OVERLAPPING, &active, &active).is_none());
    assert!(held < 4 << 20, "subsetting the strike held {} MB at its peak", held >> 20);
}

// Every other glyph names one large image, each a run of its own: one copy is kept, and the copies made to compare
// stop at the budget.
#[test]
fn a_bitmap_subtable_of_overlapping_images_stops_at_its_budget() {
    use crate::daecore::daetype::subsetter::{subset_bitmap_strikes, GlyphSet};
    let mut cblc = [0x0003_0000u32, 1, 56, 0, 1, 0].map(u32::to_be_bytes).concat();
    cblc.resize(56, 0);
    cblc.extend([1u16, OVERLAPPING as u16].map(u16::to_be_bytes).concat());
    cblc.extend(8u32.to_be_bytes());
    cblc.extend([1u16, 17].map(u16::to_be_bytes).concat());
    cblc.extend(4u32.to_be_bytes());
    cblc.extend((0..=OVERLAPPING).flat_map(|g| ((IMAGE * (g % 2)) as u32).to_be_bytes()));
    let mut cbdt = 0x0003_0000u32.to_be_bytes().to_vec();
    cbdt.resize(4 + IMAGE, 7);
    let mut active = GlyphSet::new();
    (1..=OVERLAPPING as u32).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u32> = (0..=OVERLAPPING as u32).collect();
    let held = peak_of(|| subset_bitmap_strikes(&cblc, &cbdt, &active, &gid_map).is_none());
    assert!(held < 4 << 20, "subsetting the strike held {} MB at its peak", held >> 20);
}

// 200 FDs naming one Private DICT, its Subrs INDEX 65,535 subroutines long: about 290 KB of CFF.
// Read once for each FD, the subroutines' spans would hold 100 MB.
#[test]
fn fds_sharing_a_subrs_index_hold_it_once() {
    use crate::daecore::daetype::outline::CffOutlines;
    use crate::daecore::daetype::subsetter::cff::build::encode_cff_index;
    const FDS: usize = 200;
    let index = |objects: &[Vec<u8>]| encode_cff_index(objects).expect("an INDEX");
    let int = |v: usize| [&[29u8][..], &(v as i32).to_be_bytes()].concat();
    let name = index(&[b"T".to_vec()]);
    let charstrings = index(&[vec![14], vec![14]]);
    let fdselect = [3u8, 0, 1, 0, 0, 0, 0, 2];
    let fdselect_at = 4 + name.len() + index(&[vec![0; 37]]).len() + 4;
    let charstrings_at = fdselect_at + fdselect.len();
    let fdarray_at = charstrings_at + charstrings.len();
    let private_at = fdarray_at + index(&vec![vec![0; 11]; FDS]).len();
    let top = [int(0), int(0), int(0), vec![12, 30], int(charstrings_at), vec![17], int(fdarray_at), vec![12, 36],
               int(fdselect_at), vec![12, 37]].concat();
    let fd = [int(6), int(private_at), vec![18]].concat();
    let cff = [vec![1u8, 0, 4, 2], name, index(&[top]), vec![0, 0, 0, 0], fdselect.to_vec(), charstrings,
               index(&vec![fd; FDS]), int(6), vec![19], index(&vec![vec![11]; 65_535])].concat();

    let held = peak_of(|| CffOutlines::parse(&cff).is_ok());
    assert!(held < 8 << 20, "reading the outlines held {} MB at its peak", held >> 20);
}

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn all_glyphs(n: u32) -> (crate::daecore::daetype::subsetter::GlyphSet, Vec<u32>) {
    let mut active = crate::daecore::daetype::subsetter::GlyphSet::new();
    (0..n).for_each(|g| { active.insert(g); });
    (active, (0..n).collect())
}

// A version 2 morx or kerx holding these (type or format, body) subtables.
fn aat_table(morx: bool, subtables: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let body: Vec<u8> = subtables
        .iter()
        .flat_map(|(kind, b)| {
            [[(12 + b.len()) as u32, *kind, u32::from(morx)].map(u32::to_be_bytes).concat(), b.clone()].concat()
        })
        .collect();
    let n = subtables.len() as u32;
    if morx {
        [u16s(&[2, 0]), [1u32, 1, (16 + body.len()) as u32, 0, n].map(u32::to_be_bytes).concat(), body].concat()
    } else {
        [u16s(&[2, 0]), n.to_be_bytes().to_vec(), body].concat()
    }
}

// Glyphs 1 to 65,533 in one 18-byte segment, each lookup and class table of these 100 subtables.
// Rebuilt at four bytes a glyph, 3 KB of morx and 8 KB of kerx would each hold 26 MB.
#[test]
fn a_segment_lookup_is_not_rebuilt_glyph_by_glyph() {
    use crate::daecore::daetype::subsetter::{subset_kerx, subset_morx};
    let segment = u16s(&[2, 6, 2, 12, 1, 0, 65_533, 1, 4, 0xFFFF, 0xFFFF, 0]);
    let at = 20 + segment.len();
    let state = [
        [5u32, 20, at as u32, (at + 20) as u32, (at + 24) as u32].map(u32::to_be_bytes).concat(),
        segment.clone(),
        vec![0; 20],
        u16s(&[0, 0, 0xFFFF, 0]),
    ]
    .concat();
    let morx = aat_table(true, &(0..100).map(|_| (4, segment.clone())).collect::<Vec<_>>());
    let kerx = aat_table(false, &(0..100).map(|_| (1, state.clone())).collect::<Vec<_>>());
    let count = u32::from(u16::MAX);
    let (active, gid_map) = all_glyphs(count);
    let held = peak_of(|| subset_morx(&morx, &active, &gid_map, count).is_some_and(|m| m.len() < 2 * morx.len()));
    assert!(held < 4 << 20, "subsetting morx held {} MB at its peak", held >> 20);
    let held = peak_of(|| subset_kerx(&kerx, &active, &gid_map, count).is_some_and(|k| k.len() < 2 * kerx.len()));
    assert!(held < 4 << 20, "subsetting kerx held {} MB at its peak", held >> 20);
}

// 2,000 classed glyphs and 2,000 ligature actions at one offset: an action each taking its own range
// of components would make 8 MB from 16 KB.
#[test]
fn ligature_actions_sharing_an_offset_share_their_components() {
    use crate::daecore::daetype::subsetter::subset_morx;
    const N: u16 = 2_000;
    let classes = u16s(&[2, 6, 2, 12, 1, 0, N, 1, 4, 0xFFFF, 0xFFFF, 0]);
    let (class_at, state_at) = (28, 28 + classes.len());
    let entry_at = state_at + 20;
    let action_at = entry_at + 6;
    let component_at = action_at + 4 * usize::from(N);
    let ligature_at = component_at + 2 * usize::from(N);
    let body = [
        [
            5u32,
            class_at as u32,
            state_at as u32,
            entry_at as u32,
            action_at as u32,
            component_at as u32,
            ligature_at as u32,
        ]
        .map(u32::to_be_bytes)
        .concat(),
        classes,
        vec![0; 20],
        u16s(&[0, 0, 0]),
        (0..N).flat_map(|_| 0xBFFF_FFFFu32.to_be_bytes()).collect(),
        (0..N).flat_map(u16::to_be_bytes).collect(),
        (1..=N).flat_map(u16::to_be_bytes).collect(),
    ].concat();
    let morx = aat_table(true, &[(2, body)]);
    let (active, gid_map) = all_glyphs(u32::from(N) + 1);
    let held = peak_of(|| subset_morx(&morx, &active, &gid_map, u32::from(N) + 1).is_some());
    assert!(held < 4 << 20, "subsetting the ligatures held {} MB at its peak", held >> 20);
}

// 100,000 rows no entry names and an action 64 words deep: the walk holds slots for the states its entries name, where
// one for each row would take 13,000,000 of them, 312 MB from 1.2 MB of table.
#[test]
fn a_ligature_walk_holds_the_states_its_entries_name() {
    use crate::daecore::daetype::subsetter::subset_morx;
    let rows = [u16s(&[0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 2]), vec![0; 12 * 100_000]].concat();
    let mut actions = [0u32; 64];
    actions[63] = 0x8000_0001;
    let parts = [
        u16s(&[8, 1, 3, 4, 1, 5]),
        rows,
        u16s(&[0, 0, 0, 2, 0x8000, 0, 0, 0xA000, 0]),
        actions.map(u32::to_be_bytes).concat(),
        u16s(&[0, 0, 0, 1]),
        u16s(&[0, 4]),
    ];
    let mut at = 28u32;
    let mut body = 6u32.to_be_bytes().to_vec();
    for part in &parts {
        body.extend(at.to_be_bytes());
        at += part.len() as u32;
    }
    body.extend(parts.concat());
    let morx = aat_table(true, &[(2, body)]);
    let (active, gid_map) = all_glyphs(5);
    let held = peak_of(|| subset_morx(&morx, &active, &gid_map, 5).is_some());
    assert!(held < 8 << 20, "subsetting the ligatures held {} MB at its peak", held >> 20);
}

const SET_COMPONENT: u16 = 0x8000;
const PERFORM_ACTION: u16 = 0x2000;
const LIG_LAST: u32 = 0x8000_0000;

// A morx of one ligature subtable of `classes` classes: its header, then class table, states, entries, actions,
// components and ligatures.
fn ligature_of(classes: u32, parts: [Vec<u8>; 6]) -> Vec<u8> {
    let mut at = 28u32;
    let mut body = classes.to_be_bytes().to_vec();
    for part in &parts {
        body.extend(at.to_be_bytes());
        at += part.len() as u32;
    }
    body.extend(parts.concat());
    aat_table(true, &[(2, body)])
}

// Glyphs 1 to 256, each its own class whose entry runs its own one-word action, and one action 64 words deep: the
// walk's memo of what each action does from each stack, 256 stacks of 64 words a node, would hold 40 MB from 4 KB.
#[test]
fn a_ligature_walks_memo_spends_its_budget() {
    use crate::daecore::daetype::subsetter::{morx_closure, subset_morx};
    const C: u16 = 256;
    let classes = [u16s(&[8, 1, C]), (4..C + 4).flat_map(u16::to_be_bytes).collect()].concat();
    let mut entries = u16s(&[0, SET_COMPONENT, 0, 1, SET_COMPONENT, 0, 0, 0, 0, 1, 0, 0]);
    (0..=C).for_each(|a| entries.extend(u16s(&[0, PERFORM_ACTION, a])));
    let row = |s: u16| [u16s(&[s, 2 + s, 2 + s, 2 + s]), (4..C + 4).flat_map(u16::to_be_bytes).collect()].concat();
    let mut actions = vec![LIG_LAST; usize::from(C)];
    actions.extend([0; 63].into_iter().chain([LIG_LAST]));
    let actions = actions.into_iter().flat_map(u32::to_be_bytes).collect();
    let rows = [row(0), row(1)].concat();
    let morx = ligature_of(u32::from(C) + 4, [classes, rows, entries, actions, u16s(&[0]), u16s(&[0])]);
    let (active, gid_map) = all_glyphs(u32::from(C) + 1);
    let held = peak_of(|| morx_closure(&morx, &active, u32::from(C) + 1).len() <= 1);
    assert!(held < 16 << 20, "the closure held {} MB at its peak", held >> 20);
    let held = peak_of(|| subset_morx(&morx, &active, &gid_map, u32::from(C) + 1).is_some());
    assert!(held < 16 << 20, "subsetting the ligatures held {} MB at its peak", held >> 20);
}

// 512 states each pushing class 1, every kept glyph the class table doesn't list, and popping it with its own action
// word, all at one offset: its 4,097 glyphs are held once, where each word holding them would be 2 million.
#[test]
fn action_words_sharing_an_offset_hold_what_they_pop_once() {
    use crate::daecore::daetype::subsetter::{morx_closure, subset_morx};
    const S: u16 = 512;
    let (mut entries, mut rows) = (Vec::new(), Vec::new());
    for k in 0..S {
        entries.extend(u16s(&[k, SET_COMPONENT, 0, k, PERFORM_ACTION, k, (k + 1).min(S - 1), 0, 0]));
        rows.extend(u16s(&[3 * k + 1, 3 * k, 3 * k + 2, 3 * k + 2, 3 * k + 2]));
    }
    let actions = (0..S).flat_map(|_| LIG_LAST.to_be_bytes()).collect();
    let morx = ligature_of(5, [u16s(&[8, 0, 1, 4]), rows, entries, actions, u16s(&[0]), u16s(&[0])]);
    let (active, gid_map) = all_glyphs(4_097);
    let held = peak_of(|| morx_closure(&morx, &active, 4_097).is_empty());
    assert!(held < 4 << 20, "the closure held {} MB at its peak", held >> 20);
    let held = peak_of(|| subset_morx(&morx, &active, &gid_map, 4_097).is_some());
    assert!(held < 4 << 20, "subsetting the ligatures held {} MB at its peak", held >> 20);
}

// 500 glyphs naming one 50 KB GlyphInfo through their Zapf offsets: 52 KB, or 25 MB copied per glyph.
#[test]
fn a_glyph_info_many_glyphs_share_is_written_once() {
    use crate::daecore::daetype::subsetter::subset_zapf;
    const N: u16 = 500;
    let records_at = 8 + 4 * usize::from(N);
    let mut info = vec![0xFF; 8];
    info.extend([0, 0]);
    info.extend(200u16.to_be_bytes());
    (0..200).for_each(|_| { info.extend([0, 250]); info.extend([b'a'; 250]); });
    let zapf = [
        u16s(&[1, 0]), ((records_at + info.len()) as u32).to_be_bytes().to_vec(),
        (0..N).flat_map(|_| (records_at as u32).to_be_bytes()).collect(), info,
    ].concat();
    let (active, gid_map) = all_glyphs(u32::from(N));
    let held = peak_of(|| subset_zapf(&zapf, usize::from(N), &gid_map, &active, &gid_map).is_some());
    assert!(held < 4 << 20, "subsetting Zapf held {} MB at its peak", held >> 20);
}

// One script whose default and 50 language records name one language system of 50 priorities, all naming
// one priority whose eight slots name one list of 500 lookups: 1,442 bytes, 20.5 MB copied per naming.
#[test]
fn a_jstf_part_named_many_times_is_built_once() {
    use crate::daecore::daetype::subsetter::subset_jstf;
    let mut script = u16s(&[0, 306, 50]);
    (0..50).for_each(|_| script.extend([&b"dflt"[..], &306u16.to_be_bytes()].concat()));
    let jstf = [
        u16s(&[1, 0, 1]), b"latn".to_vec(), u16s(&[12]), script,
        u16s(&[50]), u16s(&[102; 50]),
        u16s(&[20, 20, 20, 20, 0, 20, 20, 20, 20, 0]),
        u16s(&[500]), u16s(&(0..500).collect::<Vec<_>>()),
    ].concat();
    let (active, _) = all_glyphs(1);
    let held = peak_of(|| subset_jstf(&jstf, &active, &[0]).is_some());
    assert!(held < 4 << 20, "subsetting JSTF held {} MB at its peak", held >> 20);
}

// A Context lookup whose 400 class sets are one set of 400 rules, all one rule of 16 lookup records: 3 KB of GSUB
// whose numbering, walking each naming, would gather 2.56 million lookup indices.
#[test]
fn a_rule_many_rule_sets_share_is_numbered_once() {
    use crate::daecore::daetype::subsetter::otl::gsub::subset_gsub;
    const SETS: u16 = 400;
    let rule = [u16s(&[1, 16]), u16s(&[0; 32])].concat();
    let set = [u16s(&[SETS]), u16s(&[2 + 2 * SETS; SETS as usize]), rule].concat();
    let context = [
        u16s(&[2, 8 + 2 * SETS, 14 + 2 * SETS, SETS]), u16s(&[18 + 2 * SETS; SETS as usize]),
        u16s(&[1, 1, 0, 2, 0]), set,
    ].concat();
    let gsub = [
        u16s(&[1, 0, 10, 30, 44]),
        u16s(&[1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0]),
        u16s(&[1]), b"liga".to_vec(), u16s(&[8, 0, 1, 0]),
        u16s(&[1, 4, 5, 0, 1, 8]), context,
    ].concat();
    let (active, gid_map) = all_glyphs(1);
    let held = peak_of(|| subset_gsub(&gsub, &active, &gid_map, &[], false).is_some());
    assert!(held < 4 << 20, "subsetting GSUB held {} MB at its peak", held >> 20);
}

// A CID-keyed CFF whose 2,000 FDs all name one Private DICT with 200 KB of Subrs: 400 MB if each FD
// copied them in. The subset reads them once.
#[test]
fn a_shared_private_dict_is_read_once() {
    use crate::daecore::daetype::subsetter::subset_cff;
    fn index(items: &[Vec<u8>]) -> Vec<u8> {
        let mut out = (items.len() as u16).to_be_bytes().to_vec();
        out.push(4);
        let mut at = 1u32;
        out.extend(at.to_be_bytes());
        for item in items {
            at += item.len() as u32;
            out.extend(at.to_be_bytes());
        }
        items.iter().for_each(|item| out.extend(item));
        out
    }
    let int = |v: usize| [&[29u8][..], &(v as i32).to_be_bytes()].concat();
    const FDS: usize = 2_000;
    let name = index(&[b"T".to_vec()]);
    let charstrings = index(&[vec![14], vec![14]]);
    let fdselect = [3u8, 0, 1, 0, 0, 0, 0, 2];
    let top_len = index(&[vec![0; 37]]).len();
    let fdselect_at = 4 + name.len() + top_len + 6;
    let charstrings_at = fdselect_at + fdselect.len();
    let fdarray_at = charstrings_at + charstrings.len();
    let private_at = fdarray_at + index(&vec![vec![0; 11]; FDS]).len();
    let top = [int(0), int(0), int(0), vec![12, 30], int(charstrings_at), vec![17], int(fdarray_at), vec![12, 36],
               int(fdselect_at), vec![12, 37]].concat();
    let fd = [int(6), int(private_at), vec![18]].concat();
    let cff = [vec![1u8, 0, 4, 4], name, index(&[top]), vec![0, 0, 0], vec![0, 0, 0], fdselect.to_vec(), charstrings,
               index(&vec![fd; FDS]), [int(6), vec![19]].concat(), index(&[vec![11u8; 200_000]])].concat();

    let before = LIVE.get();
    PEAK.set(before);
    let out = subset_cff(&cff, &[0, 1]);
    let held = PEAK.get() - before;
    assert!(out.is_ok(), "the subset failed: {:?}", out.err());
    assert!(held < 16 << 20, "subsetting held {} MB at its peak", held >> 20);
}

// One glyph whose MathKern has 4,001 records all naming one 32 KB device: 48 KB of MATH that, copying
// the device at each naming, would hold 131 MB. The subset keeps the MathKern and the device once.
#[test]
fn a_device_every_math_kern_record_names_is_copied_once() {
    use crate::daecore::daetype::subsetter::subset_math;
    const HEIGHTS: u16 = 2_000;
    let device = 2 + 4 * (2 * HEIGHTS + 1);
    let math = [
        u16s(&[1, 0, 0, 10, 0, 0, 0, 0, 8, 12, 1, 18, 0, 0, 0, 1, 1, 1, HEIGHTS]),
        (0..2 * HEIGHTS + 1).flat_map(|_| u16s(&[0, device])).collect(),
        u16s(&[0, 0x7FFF, 3]), vec![0; 32_768],
    ].concat();
    let (active, gid_map) = all_glyphs(2);
    let held = peak_of(|| subset_math(&math, &active, &gid_map).is_some_and(|m| m.len() > 32_768));
    assert!(held < 4 << 20, "subsetting MATH held {} MB at its peak", held >> 20);
}

// 1,000 glyphs naming constructions 4 bytes apart in one run of (0, 65,535) records, so each claims
// 65,535 variants: 270 KB of MATH whose closure would hold 131 MB, and subset 250 MB, unbudgeted.
#[test]
fn overlapping_constructions_stop_at_the_budget() {
    use crate::daecore::daetype::subsetter::{math_closure, subset_math};
    const N: u16 = 1_000;
    let coverage = 10 + 2 * N;
    let region = coverage + 4 + 2 * N;
    let math = [
        u16s(&[1, 0, 0, 0, 10, 0, coverage, 0, N, 0]),
        (0..N).flat_map(|i| (region + 4 * i).to_be_bytes()).collect(),
        u16s(&[1, N]), (1..=N).flat_map(u16::to_be_bytes).collect(),
        (0..u32::from(N) + 65_535).flat_map(|_| u16s(&[0, 0xFFFF])).collect(),
    ].concat();
    let (active, gid_map) = all_glyphs(u32::from(N) + 1);
    let held = peak_of(|| math_closure(&math, &active) == [0]);
    assert!(held < 4 << 20, "the MATH closure held {} MB at its peak", held >> 20);
    let held = peak_of(|| subset_math(&math, &active, &gid_map).is_none());
    assert!(held < 4 << 20, "subsetting MATH held {} MB at its peak", held >> 20);
}

// A 200 KB glyf that every even glyph's loca range names whole: 400 glyphs would copy it 200 times,
// a 40 MB subset. Copying stops at the subset budget.
#[test]
fn overlapping_loca_ranges_stop_at_the_subset_budget() {
    const GLYF: u32 = 200_000;
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/eb-garamond/EBGaramond.ttf")).expect("fixture"), 0)
        .expect("parses");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"glyf"), vec![0; GLYF as usize]);
    tables.insert(Tag::new(b"loca"), (0..=f.glyph_count()).flat_map(|i| (GLYF * (i % 2)).to_be_bytes()).collect());
    tables.get_mut(&Tag::new(b"head")).expect("head")[50..52].copy_from_slice(&[0, 1]);
    let g = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("parses");
    let gids: Vec<GlyphId> = (0..400).map(GlyphId).collect();
    let held = peak_of(|| g.subset(&gids, &Location::default()).is_err());
    assert!(held < 4 << 20, "subsetting glyf held {} MB at its peak", held >> 20);
}

// A ReverseChainSingleSubst naming one coverage of every 16-bit glyph as each of 100 backtrack places: 1 KB of GSUB
// that, holding each place's glyphs, would hold 25 MB. Past the table's work the subtable goes.
#[test]
fn a_coverage_every_backtrack_place_names_is_held_within_the_work_budget() {
    use crate::daecore::daetype::subsetter::otl::gsub::subset_gsub;
    const N: u16 = 100;
    let coverage = 12 + 2 * N;
    let subtable = [u16s(&[1, coverage, N]), u16s(&vec![coverage; N.into()]), u16s(&[0, 1, 1, 2, 1, 0, 0xFFFF, 0])];
    let gsub = [u16s(&[1, 0, 0, 0, 10, 1, 4, 8, 0, 1, 8]), subtable.concat()].concat();
    let (active, gid_map) = all_glyphs(3);
    let held = peak_of(|| subset_gsub(&gsub, &active, &gid_map, &[], true).is_some());
    assert!(held < 4 << 20, "subsetting GSUB held {} MB at its peak", held >> 20);
}

// A GlyphInfo naming an array of 100 groups, each six bytes on from the last and reading on through a run of 65,535
// glyph 0s, which the subset keeps: 141 KB of Zapf that would copy 13 MB before the budget is looked at.
#[test]
fn an_array_s_groups_stop_at_the_budget_as_each_is_placed() {
    use crate::daecore::daetype::subsetter::subset_zapf;
    const K: usize = 100;
    let groups_at = 4 + 4 * K;
    let info = [0u32.to_be_bytes(), 0xFFFF_FFFFu32.to_be_bytes(), [0; 4]].concat();
    let records_at = 8 + 4 * 2;
    let zapf = [
        u16s(&[1, 0]), ((records_at + info.len()) as u32).to_be_bytes().to_vec(),
        [0xFFFF_FFFF, records_at as u32].map(u32::to_be_bytes).concat(), info,
        u16s(&[0x4000 | K as u16, 0]), (0..K).flat_map(|i| ((groups_at + 6 * i) as u32).to_be_bytes()).collect(),
        u16s(&[1, 0, 0xFFFF]).repeat(K), vec![0; 2 * 65_535],
    ].concat();
    let (active, gid_map) = all_glyphs(2);
    let held = peak_of(|| subset_zapf(&zapf, 2, &gid_map, &active, &gid_map).is_none());
    assert!(held < 4 << 20, "subsetting Zapf held {} MB at its peak", held >> 20);
}

// 200 xref entries all naming one 64 KB string: 77 KB of table that, copying the string for each, would hold 12 MB.
#[test]
fn xref_entries_naming_one_string_hold_it_once() {
    use crate::daecore::daetype::subsetter::subset_xref;
    const N: usize = 200;
    let entry = [&b"abcd"[..], &[0; 8], &u16s(&[0, 0xFFFF])].concat();
    let xref = [vec![0; 8], [N as u32, (16 + 16 * N) as u32].map(u32::to_be_bytes).concat(), entry.repeat(N),
                vec![b'x'; 65_535]].concat();
    let held = peak_of(|| subset_xref(&xref, |_| true).is_none());
    assert!(held < 4 << 20, "subsetting xref held {} MB at its peak", held >> 20);
}

// One lookup of 10 SingleSubsts sharing one range over 65,534 glyphs, padded to 256 KB, which the closure's 3,246
// passes need: 655,340 substitutions, which the closure walks as sets of glyphs rather than holding them.
#[test]
fn substitutions_close_without_holding_them() {
    const N: u16 = 10;
    let subtables_at = 6 + 2 * N;
    let coverage = subtables_at + 6 * N;
    let lookup = [
        u16s(&[1, 0, N]),
        (0..N).flat_map(|i| (subtables_at + 6 * i).to_be_bytes()).collect(),
        (0..N).flat_map(|i| u16s(&[1, coverage - (subtables_at + 6 * i), 1])).collect(),
        u16s(&[2, 1, 1, 65_534, 0]),
    ]
    .concat();
    let mut gsub = [u16s(&[1, 0, 10, 30, 44, 1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0]),
                u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, 0]), u16s(&[1, 4]), lookup].concat();
    gsub.resize(256 << 10, 0);
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/eb-garamond/EBGaramond.ttf")).expect("fixture"), 0)
        .expect("parses");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"GSUB"), gsub);
    let g = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("parses");
    let held = peak_of(|| g.glyph_closure([GlyphId(1)], &Location::default()).is_ok());
    assert!(held < 4 << 20, "the closure held {} MB at its peak", held >> 20);
}

// One MultipleSubst naming one sequence of 65,535 glyphs 50 times, within the sanitizer's budget: 131 KB of GSUB
// naming 3.3 million substitutions, which the closure reads into a set of glyphs rather than holding them.
#[test]
fn shared_sequences_close_without_holding_them() {
    const N: u16 = 50;
    let coverage = 6 + 2 * N;
    let sequence = coverage + 10;
    let lookup = [u16s(&[2, 0, 1, 8, 1, coverage, N]), u16s(&vec![sequence; N.into()]), u16s(&[2, 1, 1, N, 0]),
                  u16s(&[u16::MAX]), u16s(&vec![2; u16::MAX.into()])].concat();
    let gsub = [u16s(&[1, 0, 10, 30, 44, 1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0]),
                u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, 0]), u16s(&[1, 4]), lookup].concat();
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/eb-garamond/EBGaramond.ttf")).expect("fixture"), 0)
        .expect("parses");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"GSUB"), gsub);
    let g = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("parses");
    let held = peak_of(|| g.glyph_closure([GlyphId(1)], &Location::default()).is_ok());
    assert!(held < 4 << 20, "the closure held {} MB at its peak", held >> 20);
}

// Each face of a collection holds its own tables, not the whole file, which for both faces of a
// two-font collection would be twice the file.
#[test]
fn faces_of_a_collection_hold_only_their_tables() {
    let bytes = std::fs::read(format!("{FONTS}/test-fixtures/EBGaramond-InterVariable.ttc")).expect("fixture");
    let before = LIVE.get();
    let faces: Vec<Font> =
        (0..Font::face_count(&bytes)).map(|i| Font::from_collection(&bytes, i).expect("a face")).collect();
    let held = LIVE.get() - before;
    assert_eq!(faces.len(), 2);
    assert!(held * 10 < bytes.len() * 12, "{} faces hold {held} bytes of a {}-byte file", faces.len(), bytes.len());
}

// A CFF name reads the INDEX headers and its one string, not every INDEX, which is 177 KB for STIX: the first keeps
// the face's SIDs, 2 bytes a glyph, and the next holds under 1 KB.
#[test]
fn a_cff_glyph_name_reads_only_its_string() {
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/stix-two-math/STIX2Math.otf")).expect("fixture"), 0)
        .expect("parses");
    let first = peak_of(|| f.glyph_name(GlyphId(5_000)).is_some());
    assert!(first < 2 * f.glyph_count() as usize + (1 << 10), "naming the first glyph held {first} bytes");
    let next = peak_of(|| f.glyph_name(GlyphId(5_001)).is_some());
    assert!(next < 1 << 10, "naming the next glyph held {next} bytes");
}

// Source Serif's GDEF with a ligature of two format 1 carets: they need no variation store, which
// would be 249 KB parsed on every call at a varied location.
#[test]
fn carets_that_do_not_vary_read_no_variation_store() {
    let bytes = std::fs::read(format!("{FONTS}/source-serif/SourceSerif4Variable-Roman.otf")).expect("fixture");
    let f = Font::from_vec(bytes, 0).expect("parses");
    let gdef = f.table(Tag::new(b"GDEF")).expect("GDEF").to_vec();
    let store = u32::from_be_bytes([gdef[14], gdef[15], gdef[16], gdef[17]]) as usize;
    let list = u16s(&[6, 1, 12, 1, 1, 100, 2, 6, 10, 1, 300, 1, 600]);
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(
        Tag::new(b"GDEF"),
        [
            u16s(&[1, 3, 0, 0, 18, 0, 0]),
            ((18 + list.len()) as u32).to_be_bytes().to_vec(),
            list,
            gdef[store..].to_vec(),
        ]
        .concat(),
    );
    let g = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("parses");
    let bold = located(&g, &[("wght", 700.0)]);
    let held = peak_of(|| g.ligature_carets(GlyphId(100), &bold, Orientation::Horizontal).len() == 2);
    assert!(held < 16 << 10, "two carets held {} KB", held >> 10);
}

// Only a format 2 caret reads the outline at the location. Inter's `a` has no carets, so asking for
// them at wght 700 instances none of Inter, at this location or any other.
#[test]
fn carets_without_a_point_instance_nothing() {
    let f =
        Font::from_vec(std::fs::read(format!("{FONTS}/inter/InterVariable.ttf")).expect("fixture"), 0).expect("parses");
    let (a, bold) = (f.glyph_id('a').expect("a"), located(&f, &[("wght", 700.0)]));
    let held = peak_of(|| f.ligature_carets(a, &bold, Orientation::Horizontal).is_empty());
    assert!(held < 4 << 10, "no carets held {} KB", held >> 10);
}

// A hinting context is kept for each of the last few sizes, so a glyph hinted at two sizes in turn
// allocates what one size does twice, not a new context's programs, storage and zone at each switch.
#[test]
fn alternating_sizes_keep_their_hinting_contexts() {
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/test-fixtures/hinted.ttf")).expect("fixture"), 0)
        .expect("parses");
    let opts = OutlineOptions::default().with_hinting(HintMode::Font);
    let draw = |px: f32| {
        let _ = f.prepared_outline(GlyphId(1), &Location::default(), px, &opts, &mut Sink::default());
    };
    let one = allocs_per_call(|| { draw(16.0); draw(16.0) });
    let two = allocs_per_call(|| { draw(12.0); draw(16.0) });
    assert!(two <= one, "alternating sizes made {two} allocations where one size made {one}");
}

// Varying the outlines reuses its buffers from glyph to glyph, so an instance off the default allocates
// little more than one at it: about 200 more over Inter's 2,937 glyphs, not 33,400.
#[test]
fn an_instance_allocates_nothing_glyph_by_glyph() {
    let f = font();
    // Each call a new location, so none is the cached instance; a step of 1e-9 never leaves the default.
    let instance = |wght: &mut f64, step: f64| {
        *wght += step;
        std::hint::black_box(f.instance(&located(&f, &[("wght", *wght)])).expect("instanced"));
    };
    let (mut at, mut off) = (400.0, 400.0);
    let default = allocs_over(4, || instance(&mut at, 1e-9));
    let varied = allocs_over(4, || instance(&mut off, 0.37));
    let glyphs = f64::from(f.glyph_count());
    assert!(varied - default < glyphs / 8.0, "an instance off the default made {varied} allocations, {default} at it");
}

// The outline cache's budget bounds the heap it holds, not only the bytes it counts: a path is kept
// without its growth slack and charged for its place in the cache too.
#[test]
fn the_outline_cache_holds_what_it_counts() {
    for name in ["inter/InterVariable.ttf", "eb-garamond/EBGaramond.ttf", "stix-two-math/STIX2Math.otf"] {
        let bytes = std::fs::read(format!("{FONTS}/{name}")).expect("fixture");
        let f = Font::from_vec(bytes, 0).expect("parses");
        f.set_cache_budget(Cache::Outline, 262_144);
        f.prewarm((0..600).map(GlyphId), &Location::default());
        f.clear_cache(Cache::Outline);
        let before = LIVE.get();
        f.prewarm((0..600).map(GlyphId), &Location::default());
        let held = LIVE.get() - before;
        let CacheStats { entries: _, bytes: counted } = f.cache_stats(Cache::Outline);
        assert!(held * 100 <= counted * 105, "{name}: the cache counts {counted} bytes and holds {held}");
    }
}

// Two variable fonts at ten weights hold within 2% of the bytes counted, and a budget of 0 frees all but the newest;
// warming first leaves out what is built once, the spare path, delta room and the last locations' autohinters.
#[test]
fn the_instance_budget_counts_what_its_locations_hold() {
    let text = "The quick brown fox jumps over the lazy dog, Hamburgefonstiv 0123456789. ".repeat(20);
    for rel in ["inter/InterVariable.ttf", "noto-devanagari/NotoSansDevanagari.ttf"] {
        let f = Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("fixture"), 0).expect("parses");
        f.set_cache_budget(Cache::Shape, 0);
        for w in [100.0, 150.0, 200.0] {
            let at = located(&f, &[("wght", w)]);
            let _ = std::hint::black_box(f.hinted_outline(GlyphId(40), &at, 20.0, HintMode::Auto));
        }
        f.clear_cache(Cache::Instance);
        let heavy = located(&f, &[("wght", 900.0)]);
        let _ = f.shape(&text, &Location::default(), &ShapeOptions::default());
        let _ = f.outline(GlyphId(40), &Location::default(), &mut Sink::default());
        let _ = f.shape(&text, &heavy, &ShapeOptions::default());
        for gid in 0..200 {
            let _ = f.outline(GlyphId(gid), &heavy, &mut Sink::default());
        }
        let _ = std::hint::black_box(f.hinted_outline(GlyphId(40), &heavy, 20.0, HintMode::Auto));
        let apart = f.cache_stats(Cache::Instance).bytes;
        let weights: Vec<Location> = (0..10).map(|w| located(&f, &[("wght", 325.0 + f64::from(w) * 50.0)])).collect();
        let before = LIVE.get();
        for at in &weights {
            std::hint::black_box(f.shape(&text, at, &ShapeOptions::default()));
            for gid in 0..200 {
                let _ = f.outline(GlyphId(gid), at, &mut Sink::default());
            }
            let _ = std::hint::black_box(f.hinted_outline(GlyphId(40), at, 20.0, HintMode::Auto));
        }
        let held = LIVE.get() - before;
        let CacheStats { entries: locations, bytes: counted } = f.cache_stats(Cache::Instance);
        let counted = counted - apart;
        assert_eq!(locations, 11, "{rel}");
        assert!(held.abs_diff(counted) * 50 <= held, "{rel}: {held} B held, {counted} B counted");

        f.set_cache_budget(Cache::Instance, 0);
        let newest = f.cache_stats(Cache::Instance).bytes;
        let left = (LIVE.get() + apart).saturating_sub(before);
        // The thread keeps resolved locations, each on cache lines of its own, and autohinters for its last few, about
        // 2.4 KB the budget leaves out on purpose: under 2% of what's left plus 2 KB.
        assert!(left.abs_diff(newest) <= left / 50 + 2048, "{rel}: {left} B left for a newest location of {newest} B");
    }
}

// The shape budget counts what its runs hold on the heap: 2,000 short runs and 2,000 long ones, with and
// without axes, hold within 2% of the bytes counted.
#[test]
fn the_shape_budget_counts_what_its_runs_hold() {
    let f = font();
    f.set_cache_budget(Cache::Shape, usize::MAX);
    for (words, at) in [(1, Location::default()), (40, Location::default()), (1, located(&f, &[("wght", 650.0)]))] {
        let _ = f.shape("warm", &at, &ShapeOptions::default());
        f.clear_cache(Cache::Shape);
        let before = LIVE.get();
        for i in 0..2000 {
            std::hint::black_box(f.shape(&format!("{}{i}", "word ".repeat(words)), &at, &ShapeOptions::default()));
        }
        let held = LIVE.get() - before;
        let CacheStats { entries, bytes: counted } = f.cache_stats(Cache::Shape);
        assert_eq!(entries, 2000);
        assert!(held.abs_diff(counted) * 50 <= held, "{words} words {at:?}: {held} B held, {counted} B counted");
    }
}

// A long run's buffer goes with it, not kept as the spare: 50,000 characters leave under 64 KB held, at the
// default and at a weight, while a run of 1,000 keeps its buffer for the next.
#[test]
fn a_long_run_leaves_no_buffer_behind() {
    let f = font();
    f.set_cache_budget(Cache::Shape, 0);
    let long = "word ".repeat(10_000);
    for at in [Location::default(), located(&f, &[("wght", 650.0)])] {
        let _ = f.shape("word word", &at, &ShapeOptions::default());
        let before = LIVE.get();
        drop(f.shape(&long, &at, &ShapeOptions::default()));
        let left = LIVE.get().saturating_sub(before);
        assert!(left < 64 * 1024, "{at:?}: {left} B held after the run");
        let before = LIVE.get();
        drop(f.shape(&long[..1_000], &at, &ShapeOptions::default()));
        let kept = LIVE.get().saturating_sub(before);
        assert!(kept >= 32 * 1000, "{at:?}: a run of 1,000 kept {kept} B");
    }
}

// Alternating directions make a bidi run of each character, and every run is told what surrounds it. Four
// times the characters allocate under six times the bytes, where copying the whole text into each run took 16.
#[test]
fn bidi_runs_cost_their_own_length() {
    let f = font();
    for name in ["shape_bidi", "caret_positions"] {
        let spent = |chars: usize| {
            let text = "a\u{5d0}".repeat(chars / 2);
            let before = TOTAL.get();
            match name {
                "shape_bidi" => {
                    drop(f.shape_bidi(&text, &Location::default(), BaseDirection::Auto, &ShapeOptions::default()))
                }
                _ => drop(f.caret_positions(&text, &Location::default(), Orientation::Horizontal)),
            }
            TOTAL.get() - before
        };
        spent(100);
        let (short, long) = (spent(1000), spent(4000));
        assert!(long < 6 * short, "{name}: {short} B for 1,000 characters, {long} B for 4,000");
    }
}

// A warm shape through morx allocates no more than one without: its subtables test glyphs against the run buffer's set,
// kept between runs. TestMORXTen's morx merges this text's clusters, four glyphs or more.
#[test]
fn a_morx_shape_allocates_no_more_than_one_without_morx() {
    let text = "A third sentence that goes";
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/aat/TestMORXTen.ttf")).expect("fixture"), 0).expect("parses");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.remove(&Tag::new(b"morx"));
    let plain = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("parses");
    let shape = |font: &Font| font.shape(text, &Location::default(), &ShapeOptions::default());
    assert_ne!(shape(&f).clusters, shape(&plain).clusters, "morx no longer acts here");
    // With no shape budget, each call shapes, on the instance's spare buffer.
    f.set_cache_budget(Cache::Shape, 0);
    plain.set_cache_budget(Cache::Shape, 0);
    let with_morx = allocs_per_call(|| {
        std::hint::black_box(shape(&f));
    });
    let without = allocs_per_call(|| {
        std::hint::black_box(shape(&plain));
    });
    assert!(with_morx <= without, "a warm morx shape allocated {with_morx} times, {without} without morx");
}

// The benches' texts and the scripts the syllable machines split, each shape on a buffer made beforehand: Arabic's
// output leaves the glyph array, the marks attach through GPOS's chains, and none of it allocates.
#[test]
fn a_warm_shape_allocates_nothing() {
    use crate::daecore::daekit::cache::FontCache;
    use crate::daecore::daeshaper::{face::Face, ot::tag::script_tags, pipeline, run::Run};
    let tai_tham = "ᨠᩣ".repeat(40);
    let cases = [
        ("inter/InterVariable.ttf", "Typography is the craft of endowing human language with a durable visual form."),
        ("scheherazade-new/ScheherazadeNew-Regular.ttf", "السلام عليكم ورحمة الله وبركاته"),
        ("noto-devanagari/NotoSansDevanagari.ttf", "हिन्दी भाषा में स्वागत है"),
        ("eb-garamond/EBGaramond.ttf", "The office staff shuffled a fistful of waffles."),
        ("source-han-sans/SourceHanSansJP-VF.otf", "日本語の組版では、文字の間隔と行の組み方が読みやすさを決める。"),
        ("noto-khmer/NotoSansKhmer.ttf", "ភាសាខ្មែរជាភាសាផ្លូវការរបស់ប្រទេសកម្ពុជា"),
        ("noto-myanmar/NotoSansMyanmar.ttf", "မြန်မာဘာသာစကားသည် မြန်မာနိုင်ငံ၏ ရုံးသုံးဘာသာစကား ဖြစ်သည်။"),
        ("structure/TestShapeLana.ttf", tai_tham.as_str()),
    ];
    for (rel, text) in cases {
        let path = format!("{FONTS}/{rel}");
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
        let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("parses");
        let cache = FontCache::new(tables).expect("the fixture loads");
        let face = Face::new(&cache);
        let buffer = || {
            let mut b = Run::default();
            b.push_str(text);
            let direction = pipeline::settle_script(&mut b);
            (b, direction)
        };
        let (probe, direction) = buffer();
        let tags = probe.script.map(script_tags);
        let tags = tags.as_ref().map_or(&[][..], |t| t.as_slice());
        let plan = crate::tests::plain_plan(&face, probe.script, direction, tags);
        let mut buffers: Vec<_> = (0..ROUNDS + 8).map(|_| buffer()).collect();
        let shapes = allocs_over(ROUNDS, || {
            let (mut b, direction) = buffers.pop().expect("one buffer a shape");
            pipeline::shape(&face, &plan, &mut b, direction, &[]);
        });
        assert_eq!(shapes, 0.0, "{rel}: a warm shape allocated {shapes} times");
    }
}

// A warm shape allocates only the run it returns (five vectors and an Arc), features and language too, as the plan and
// buffer are reused; plain options hit the run cache and allocate nothing, at a location too.
#[test]
fn a_warm_public_shape_allocates_only_its_run() {
    let cases = [
        ("inter/InterVariable.ttf", "Typography is the craft of endowing human language with a durable visual form."),
        ("scheherazade-new/ScheherazadeNew-Regular.ttf", "السلام عليكم ورحمة الله وبركاته"),
        ("noto-devanagari/NotoSansDevanagari.ttf", "हिन्दी भाषा में स्वागत है"),
        ("source-han-sans/SourceHanSansJP-VF.otf", "日本語の組版では、文字の間隔と行の組み方が読みやすさを決める。"),
    ];
    let kern = Feature { tag: Tag::new(b"kern"), value: 0, bytes: (2..9).into() };
    let features = [Feature::new(Tag::new(b"liga"), 0), kern];
    let given = ShapeOptions::default().with_features(&features).with_language("en-US");
    for (named, opts, allocs) in
        [("plain options", ShapeOptions::default(), 0.0), ("features and a language", given, 6.0)]
    {
        for (rel, text) in cases {
            let path = format!("{FONTS}/{rel}");
            let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
            let font = Font::from_vec(bytes, 0).expect("parses");
            let per = allocs_per_call(|| {
                std::hint::black_box(font.shape(text, &Location::default(), &opts));
            });
            assert_eq!(per, allocs, "{rel}, {named}: a warm public shape allocated {per} times");
            let bold = located(&font, &[("wght", 700.0)]);
            let at_bold = allocs_per_call(|| {
                std::hint::black_box(font.shape(text, &bold, &opts));
            });
            assert_eq!(at_bold, allocs, "{rel}, {named}: a warm shape at wght 700 allocated {at_bold} times");
        }
    }
}

// carets.ttf with a COLR v1 glyph f of `layers` layers, each one shared paint filling f with a solid.
fn layered(layers: u8) -> Font {
    let source = Font::from_vec(std::fs::read(format!("{FONTS}/test-fixtures/carets.ttf")).expect("the fixture"), 0)
        .expect("it parses");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = source.table_tags()
        .map(|t| (t, source.table(t).expect("listed").to_vec()))
        .collect();
    let n = u32::from(layers);
    let mut colr = u16s(&[1, 0, 0, 0, 0, 0, 0, 0, 34, 0, 50, 0, 0, 0, 0, 0, 0]);
    colr.extend([u16s(&[0, 1, 1, 0, 10]), vec![1, layers, 0, 0, 0, 0], n.to_be_bytes().to_vec()].concat());
    colr.extend((0..n).flat_map(|_| (4 + 4 * n).to_be_bytes()));
    colr.extend([10, 0, 0, 6, 0, 1, 2, 0, 0, 0x40, 0]);
    let cpal = [u16s(&[0, 1, 1, 1, 0, 14, 0]), vec![0, 0, 0xFF, 0xFF]].concat();
    tables.extend([(Tag::new(b"COLR"), colr), (Tag::new(b"CPAL"), cpal)]);
    Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses")
}

// A color glyph's paints are read where they lie and a scene keeps each list in one buffer, so sixty layers allocate no
// more than one. Each scene is kept, so each draws into lists of its own.
#[test]
fn a_color_glyph_allocates_nothing_a_paint() {
    let (one, sixty) = (layered(1), layered(60));
    let draw = |f: &Font| f.color_scene(GlyphId(1), &Location::default(), &SceneOptions::default()).expect("draws");
    assert_eq!(draw(&sixty).map(|s| s.ops().len()), Some(60));
    let scene = |f: &Font| {
        let mut kept = Vec::with_capacity(ROUNDS + 8);
        allocs_per_call(|| kept.push(draw(f)))
    };
    let origin = |f: &Font| f.vertical_origin(GlyphId(1), &Location::default());
    let extents = |f: &Font| allocs_per_call(|| { std::hint::black_box(origin(f)); });
    let (scenes, origins) = ((scene(&one), scene(&sixty)), (extents(&one), extents(&sixty)));
    assert_eq!(origin(&sixty), origin(&one));
    assert_eq!(scenes.1, scenes.0, "a scene of 60 layers allocated {} times, of 1 {}", scenes.1, scenes.0);
    assert_eq!(origins.1, origins.0, "extents over 60 layers allocated {} times, over 1 {}", origins.1, origins.0);
}

// Base glyph 2 of a font with outlines from glyph 2 on, its `layers` layers each a glyph of its own filled with
// one linear gradient.
fn gradient_layers(layers: u8) -> Font {
    let source = std::fs::read(format!("{FONTS}/structure/TestCMAPMacTurkish.ttf")).expect("the fixture");
    let source = Font::from_vec(source, 0).expect("it parses");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = source.table_tags()
        .map(|t| (t, source.table(t).expect("listed").to_vec()))
        .collect();
    let n = u16::from(layers);
    let mut colr = u16s(&[1, 0, 0, 0, 0, 0, 0, 0, 34, 0, 50, 0, 0, 0, 0, 0, 0]);
    colr.extend([u16s(&[0, 1, 2, 0, 10]), vec![1, layers, 0, 0, 0, 0], u32::from(n).to_be_bytes().to_vec()].concat());
    colr.extend((0..n).flat_map(|i| u32::from(4 + 4 * n + 6 * i).to_be_bytes()));
    for i in 0..n {
        let child = u32::from(6 * (n - i)).to_be_bytes();
        colr.extend([&[10], &child[1..], &(2 + i).to_be_bytes()[..]].concat());
    }
    colr.extend([vec![4, 0, 0, 16], u16s(&[0, 0, 100, 0, 0, 100])].concat());
    colr.extend([vec![0], u16s(&[2, 0, 0, 0x4000, 0x4000, 0, 0x4000])].concat());
    let cpal = [u16s(&[0, 1, 1, 1, 0, 14, 0]), vec![0, 0, 0xFF, 0xFF]].concat();
    tables.extend([(Tag::new(b"COLR"), colr), (Tag::new(b"CPAL"), cpal)]);
    Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses")
}

// A scene's paths, gradients and clips each grow one buffer by doubling, so sixty layers of their own glyphs and
// gradients take fewer allocations than the layers they add. Each scene is kept, so each draws into its own.
#[test]
fn a_scene_allocates_per_buffer_not_per_layer() {
    use crate::color::{Op, Paint};
    let (one, sixty) = (gradient_layers(1), gradient_layers(60));
    let draw = |f: &Font| f.color_scene(GlyphId(2), &Location::default(), &SceneOptions::default()).expect("draws");
    let scene = draw(&sixty).expect("a color glyph");
    let gradients = scene.ops().iter().filter(|o| matches!(o, Op::Fill { paint: Paint::Gradient(_), .. })).count();
    assert_eq!((scene.ops().len(), gradients), (60, 60), "{:?}", scene.ops());
    let allocs = |f: &Font| {
        let mut kept = Vec::with_capacity(ROUNDS + 8);
        allocs_per_call(|| kept.push(draw(f)))
    };
    let (a, b) = (allocs(&one), allocs(&sixty));
    assert!(b - a < 59.0, "sixty layers allocated {b} times, one {a}");
}

// A run copied with `clone_from` into a kept one fills the room the kept one has: a cached run of a short text into
// one that held a long text allocates nothing, and is the cached run.
#[test]
fn a_run_cloned_into_a_kept_one_allocates_nothing() {
    let f = font();
    let options = crate::shape::ShapeOptions::default();
    let mut kept = (*f.shape("Typography is the art of arranging type", &Location::default(), &options)).clone();
    let short = f.shape("Type", &Location::default(), &options);
    let allocs = allocs_per_call(|| kept.clone_from(&short));
    assert_eq!((allocs, &kept), (0.0, &*short));
}

// A scene dropped leaves its lists, emptied, in the thread's spare, which only `std` gives: scenes of sixty layers and
// of one, drawn in turn, allocate nothing once warm, and what the bigger left doesn't show in the smaller.
#[cfg(feature = "std")]
#[test]
fn scenes_drawn_in_turn_allocate_nothing_once_warm() {
    let (one, sixty) = (gradient_layers(1), gradient_layers(60));
    let draw = |f: &Font| f.color_scene(GlyphId(2), &Location::default(), &SceneOptions::default()).expect("draws");
    let small = draw(&one).expect("a color glyph");
    drop(draw(&sixty));
    assert!(draw(&one).is_some_and(|again| again == small), "a scene after a bigger one drew otherwise");
    drop(small);
    let allocs = allocs_per_call(|| {
        drop(std::hint::black_box(draw(&sixty)));
        drop(std::hint::black_box(draw(&one)));
    });
    assert_eq!(allocs, 0.0, "two scenes in turn allocated {allocs} times");
}

// Font data is read where it lies. Hint contexts hold views of hmtx, vmtx, fpgm and prep: with Irianis's hmtx padded
// to 1 MB, three more contexts hold no copy of it.
#[test]
fn hint_contexts_read_the_fonts_tables_in_place() {
    let irianis = std::fs::read(format!("{FONTS}/Irianisadfstd/IrianisadfstdRegular.ttc")).expect("fixture");
    let source = Font::from_collection(&irianis, 0).expect("face 0");
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> = source.table_tags()
        .map(|t| (t, source.table(t).expect("listed").to_vec()))
        .collect();
    tables.get_mut(&Tag::new(b"hmtx")).expect("hmtx").resize(1 << 20, 0);
    let f = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the padded font parses");
    let _ = std::hint::black_box(f.hinted_outline(GlyphId(40), &Location::default(), 9.0, HintMode::Font));
    let before = LIVE.get();
    for px in [12.0, 16.0, 24.0] {
        let _ = std::hint::black_box(f.hinted_outline(GlyphId(40), &Location::default(), px, HintMode::Font));
    }
    let held = LIVE.get() - before;
    assert!(held < 1 << 18, "three more contexts hold {held} B beside a 1 MB hmtx");
}

// A location keeps only the tables it changes: EB Garamond's at wght 800 shares the font's 33,083-byte post.
#[test]
fn a_location_shares_the_tables_it_leaves_as_they_were() {
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/eb-garamond/EBGaramond.ttf")).expect("fixture"), 0)
        .expect("parses");
    let _ = f.shape("Hamburgefonstiv", &located(&f, &[("wght", 800.0)]), &ShapeOptions::default());
    let CacheStats { entries: locations, bytes: held } = f.cache_stats(Cache::Instance);
    assert_eq!(locations, 1);
    assert!(held < 33_083, "the location holds {held} B");
}

// A font's first glyph reads loca, CharStrings and FDSelect where they lie: with a glyf font's drawn origins, 4
// bytes a glyph, it holds less than the 8 bytes a glyph a list of those offsets would take alone.
#[test]
fn a_first_glyph_reads_its_offsets_in_place() {
    for rel in ["eb-garamond/EBGaramond.ttf", "inter/InterVariable.ttf", "source-han-sans/SourceHanSansJP-VF.otf",
        "stix-two-math/STIX2Math.otf"]
    {
        let f = Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("fixture"), 0).expect("parses");
        let before = LIVE.get();
        let _ = f.outline(GlyphId(1), &Location::default(), &mut Sink::default());
        let held = LIVE.get() - before;
        let glyphs = f.glyph_count() as usize;
        assert!(held < glyphs * 6, "{rel}: the first glyph drawn holds {held} B for {glyphs} glyphs");
    }
}

// Advances are read where hmtx and vmtx lie: every glyph's both ways, at the default, keeps no byte.
#[test]
fn advances_keep_nothing() {
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/inter/InterVariable.ttf")).expect("fixture"), 0)
        .expect("parses");
    let gids: Vec<GlyphId> = (0..f.glyph_count()).map(GlyphId).collect();
    let before = LIVE.get();
    let mut widths = Vec::new();
    f.advances(&gids, &Location::default(), Orientation::Horizontal, &mut widths);
    drop(std::hint::black_box(widths));
    for &gid in &gids {
        std::hint::black_box(f.advance(gid, &Location::default(), Orientation::Vertical));
    }
    assert_eq!(LIVE.get(), before, "reading advances kept bytes");
}

// A face's indexes are built from the font's own coverages, classes, pairs and cmap, with no list of every glyph or
// code made first and freed: before, 270 to 704 KB a first shape; now 23 to 72 KB, the most of it the cmap's runs.
#[test]
fn a_first_shape_builds_its_indexes_without_glyph_lists() {
    let cases = [
        ("inter/InterVariable.ttf", "Typography is the craft of endowing human language with a durable visual form."),
        ("scheherazade-new/ScheherazadeNew-Regular.ttf", "السلام عليكم ورحمة الله وبركاته"),
        ("noto-devanagari/NotoSansDevanagari.ttf", "हिन्दी भाषा में स्वागत है"),
        ("eb-garamond/EBGaramond.ttf", "The office staff shuffled a fistful of waffles."),
        ("source-han-sans/SourceHanSansJP-VF.otf", "日本語の組版では、文字の間隔と行の組み方が読みやすさを決める。"),
    ];
    for (rel, text) in cases {
        let path = format!("{FONTS}/{rel}");
        let font = Font::from_vec(std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})")), 0)
            .expect("parses");
        let (total, live) = (TOTAL.get(), LIVE.get());
        let run = font.shape(text, &Location::default(), &ShapeOptions::default());
        drop(run);
        let transient = (TOTAL.get() - total) - (LIVE.get() - live);
        assert!(transient < 96 * 1024, "{rel}: a first shape allocated {transient} B it didn't keep");
    }
}

// Graphemes and words allocate their list alone, reading each character's classes as they walk the text.
#[test]
fn segmentation_allocates_only_its_list() {
    let text = "Typography's 1,000 forms: 👨‍👩‍👧 🇰🇷🇸🇪 हिन्दी क्षत्रिय, בְּרֵאשִׁית ภาษาไทย ﾃｷｽﾄ";
    let graphemes = allocs_per_call(|| {
        std::hint::black_box(crate::unicode::grapheme_boundaries(text));
    });
    let words = allocs_per_call(|| {
        std::hint::black_box(crate::unicode::word_boundaries(text));
    });
    assert_eq!(graphemes, 1.0, "graphemes allocated {graphemes} times: their list alone");
    assert_eq!(words, 1.0, "words allocated {words} times: their list alone");
}

// Line breaking reads each character's class once, into a record a significant character, and hands an SA run to
// a caller's segmenter as a slice of the text: its breaks' list and the records, then what the segmenter finds.
#[test]
fn line_breaking_allocates_only_its_lists() {
    let text = "Typography's 1,000 forms: 👨‍👩‍👧 🇰🇷🇸🇪 हिन्दी क्षत्रिय, בְּרֵאשִׁית ﾃｷｽﾄ";
    let lines = allocs_per_call(|| {
        std::hint::black_box(crate::unicode::line_break_opportunities(text, None));
    });
    assert_eq!(lines, 2.0, "lines allocated {lines} times: their list and a record a significant character");
    let cut = |run: &str, out: &mut Vec<usize>| out.push(run.chars().count() / 2);
    let segmented = allocs_per_call(|| {
        std::hint::black_box(crate::unicode::line_break_opportunities(
            "ภาษาไทย ภาษาไทย",
            Some(crate::unicode::WordSegmenter(&cut)),
        ));
    });
    assert_eq!(segmented, 3.0, "Thai through a segmenter allocated {segmented} times");
}

// A dictionary probe allocates nothing, so a Thai paragraph eight times as long allocates only what its four
// growing lists take to double three times more.
#[cfg(feature = "dictionaries")]
#[test]
fn dictionary_words_allocate_nothing_a_probe() {
    let run = "ภาษาไทยเป็นภาษาที่มีวรรณยุกต์และใช้อักษรไทยในการเขียนหนังสือ";
    let (short, long) = (run.repeat(4), run.repeat(32));
    let short_allocs = allocs_per_call(|| {
        std::hint::black_box(crate::unicode::line_break_opportunities(&short, None));
    });
    let long_allocs = allocs_per_call(|| {
        std::hint::black_box(crate::unicode::line_break_opportunities(&long, None));
    });
    assert!(long_allocs - short_allocs <= 12.0, "{short_allocs} allocations, then {long_allocs} eight times as long");
}

// The latency rows' calls into daecore, once warm, allocate nothing: their working memory stays where they keep it,
// much of it the thread's, which only `std` gives them.
#[cfg(feature = "std")]
#[test]
fn the_latency_rows_allocate_nothing_once_warm() {
    use crate::daecore::daetype::{decoder, hinting, instancer};

    let bytes = std::fs::read(format!("{FONTS}/test-fixtures/hinted.ttf")).expect("fixture");
    let map = decoder::extract_ttf_tables(&bytes).expect("parses");
    let head = map.get("head").expect("head");
    let (upm, fmt) = (decoder::read_u16_be(head, 18).expect("upm"), decoder::read_i16_be(head, 50).expect("fmt"));
    let n = decoder::read_u16_be(map.get("maxp").expect("maxp"), 4).expect("count");
    let loca = instancer::parse_loca(&map, fmt, n.into()).expect("loca");
    let glyf = map.get("glyf").expect("glyf").clone();
    let mut ctx = hinting::HintContext::new(&map, 16.into(), upm).expect("hints");
    let mut hinted = 0;
    let sweep = allocs_per_call(|| {
        for gid in 0..u32::from(n) {
            if let Some(out) = ctx.hint_glyph(&glyf, &loca, gid) {
                hinting::draw_hinted(out, &mut Sink::default());
                hinted += 1;
            }
        }
    });
    assert!(hinted > 0, "the fixture hinted nothing");
    assert_eq!(sweep, 0.0, "a warm context's hint_glyph sweep allocated {sweep} times");
    // A context made for each glyph takes the buffers the thread's last one grew.
    let per_glyph = allocs_per_call(|| {
        for gid in 0..u32::from(n) {
            let mut ctx = hinting::HintContext::new(&map, 16.into(), upm).expect("hints");
            if let Some(out) = ctx.hint_glyph(&glyf, &loca, gid) {
                hinting::draw_hinted(out, &mut Sink::default());
            }
        }
    });
    assert_eq!(per_glyph, 0.0, "a context a glyph allocated {per_glyph} times a sweep");

    let font = font();
    let opts = OutlineOptions::default();
    for c in ['g', 'Q', '&'] {
        let gid = font.glyph_id(c).expect("the font has the character");
        let prepared = allocs_per_call(|| {
            let _ = std::hint::black_box(font.prepared_outline(
                gid,
                &Location::default(),
                16.0,
                &opts,
                &mut Sink::default(),
            ));
        });
        assert_eq!(prepared, 0.0, "'{c}' prepared at 16 px allocated {prepared} times");
    }

    // A glyf sweep and Inter's H collected and autohinted, as `outline_glyf_sweep`, `autohint_inter_H` and
    // `autohint_sweep_inter` call them: the decoder's coordinates and the collected points are the thread's own.
    use crate::daecore::daetype::{hinting::auto, outline};
    for rel in ["eb-garamond/EBGaramond.ttf", "inter/InterVariable.ttf"] {
        let bytes = std::fs::read(format!("{FONTS}/{rel}")).expect("fixture");
        let map = decoder::extract_ttf_tables(&bytes).expect("parses");
        let fmt = decoder::read_i16_be(map.get("head").expect("head"), 50).expect("fmt");
        let n = decoder::read_u16_be(map.get("maxp").expect("maxp"), 4).expect("count");
        let loca = instancer::parse_loca(&map, fmt, n.into()).expect("loca");
        let sweep = allocs_over(4, || {
            for gid in 0..u32::from(n) {
                let _ = outline::outline_glyf_glyph_with_loca(&map, &loca, gid, &mut Sink::default());
            }
        });
        assert_eq!(sweep, 0.0, "{rel}: a glyf sweep allocated {sweep} times");

        let upm = decoder::read_u16_be(map.get("head").expect("head"), 18).expect("upm");
        let collect = |gid: u32| {
            let mut pen = auto::CollectPen::new();
            outline::outline_glyf_glyph_with_loca(&map, &loca, gid, &mut pen).ok()?;
            Some(pen.finish()).filter(|p| !p.is_empty())
        };
        let cmap = map.get("cmap").expect("cmap");
        let lookup = crate::daecore::daetype::subsetter::CmapLookup::new(cmap);
        let mut gid_of = |c: char| lookup.glyph_id(cmap, c as u32);
        let mut hinter = auto::AutoHinter::new(upm, &mut gid_of, &mut |g| collect(g)).expect("zones");
        let h = gid_of('H').expect("H");
        let hinted = allocs_per_call(|| {
            let pts = collect(h).expect("H has an outline");
            std::hint::black_box(hinter.hint(&pts, 16.into()).y.len());
        });
        assert_eq!(hinted, 0.0, "{rel}: H collected and autohinted allocated {hinted} times");
        let swept = allocs_over(2, || {
            for gid in 0..u32::from(n) {
                if let Some(pts) = collect(gid) {
                    std::hint::black_box(hinter.hint(&pts, 16.into()).y.len());
                }
            }
        });
        assert_eq!(swept, 0.0, "{rel}: an autohint sweep allocated {swept} times");
    }

    // STIX's CFF read, as `autohint_sweep_stix` calls it: the operand stack is the thread's own.
    let stix = std::fs::read(format!("{FONTS}/stix-two-math/STIX2Math.otf")).expect("fixture");
    let map = decoder::extract_ttf_tables(&stix).expect("parses");
    let cff = map.get("CFF ").expect("CFF").clone();
    let outlines = outline::CffOutlines::parse(&cff).expect("navigation");
    let n = decoder::read_u16_be(map.get("maxp").expect("maxp"), 4).expect("count");
    let read = allocs_over(2, || {
        for gid in 0..u32::from(n) {
            let _ = outline::outline_cff_glyph_with(&outlines, &cff, gid, &mut Sink::default());
        }
    });
    assert_eq!(read, 0.0, "a STIX CFF sweep allocated {read} times");

    // An owned open allocates what the font keeps, its buffer's count, table list and state; the fourth is the copy
    // of the buffer made for each open. The token its locations know it by waits for the first location.
    let inter = std::fs::read(format!("{FONTS}/inter/InterVariable.ttf")).expect("fixture");
    let opened = allocs_per_call(|| drop(std::hint::black_box(Font::from_vec(inter.clone(), 0).expect("parses"))));
    assert_eq!(opened, 4.0, "an owned open allocated {opened} times, the buffer's copy among them");
}

// A warm rasterizer allocates nothing: a glyph plain, filtered or sRGB alpha, a path, both distance fields, and with
// `std`'s thread lists a glyph darkened or hinted and a color glyph; nor do raster cache hits.
#[test]
fn a_warm_rasterizer_allocates_nothing() {
    use crate::raster::{Field, Gamma, RasterOptions, Rasterizer, StripeOrder, SubpixelLayout};
    let font = font();
    let glyphs: Vec<GlyphId> = "Rag&".chars().map(|c| font.glyph_id(c).expect("the font has it")).collect();
    let bold = located(&font, &[("wght", 700.0)]);
    let o = RasterOptions::DEFAULT;
    let srgb = Gamma::Srgb { text: crate::color::Rgba::opaque(20, 20, 20), contrast: 0.5 };
    let rgb = SubpixelLayout::horizontal(StripeOrder::Rgb);
    let options = [
        o,
        o.with_layout(&rgb),
        o.with_gamma(srgb),
        o.with_darken(true),
        o.with_outline(OutlineOptions::DEFAULT.with_hinting(HintMode::AutoForce)),
    ];
    let mut path = crate::outline::Path::default();
    font.outline(glyphs[1], &Location::default(), &mut path).expect("a draws");
    let fields = [Field::Sdf { spread: 4.0 }, Field::Msdf { spread: 4.0 }];
    let mut r = Rasterizer::new();
    let draw = |r: &mut Rasterizer, which: usize| {
        for at in [&Location::default(), &bold] {
            for &g in &glyphs {
                let _ = std::hint::black_box(match which {
                    0..5 => r.glyph(&font, g, at, 18.0, &options[which]),
                    5 | 6 => r.distance_field(&font, g, at, 32.0, fields[which - 5]),
                    _ => r.path(&path, crate::outline::FillRule::NonZero, None),
                }
                .expect("draws"));
            }
        }
    };
    for budget in [0, 16 << 20] {
        font.set_cache_budget(Cache::Raster, budget);
        let names = ["plain", "subpixel", "sRGB", "darkened", "hinted", "SDF", "MSDF", "path"];
        for (which, name) in names.iter().enumerate() {
            if cfg!(not(feature = "std")) && matches!(*name, "darkened" | "hinted") {
                continue;
            }
            let n = allocs_over(4, || draw(&mut r, which));
            assert_eq!(n, 0.0, "{name}, at a raster budget of {budget}, allocated {n} times a round");
        }
    }

    #[cfg(feature = "std")]
    {
        let color = layered(3);
        let scene = SceneOptions::default();
        let mut paint = || {
            let image = r.color_glyph(&color, GlyphId(1), &Location::default(), 24.0, &o, &scene).expect("draws");
            std::hint::black_box(image.is_some_and(|i| i.width > 0))
        };
        assert!(paint(), "the color glyph drew nothing");
        let cold = allocs_per_call(|| { paint(); });
        assert_eq!(cold, 0.0, "a warm color glyph allocated {cold} times");
        color.set_cache_budget(Cache::Raster, 1 << 20);
        let hit = allocs_per_call(|| { paint(); });
        assert_eq!(hit, 0.0, "a color glyph's hit allocated {hit} times");
    }
}

// The raster budget counts its images' heap: 600 glyphs at three sizes, in glyph order and out of it, hold within 5%,
// the maps' nodes filling by key order; each run's rasterizer is fresh, as one keeps its last lent image.
#[test]
fn the_raster_budget_counts_what_its_images_hold() {
    use crate::raster::{RasterOptions, Rasterizer};
    let f = font();
    let n = f.glyph_count().min(600);
    for (px, step) in [(12.0, 1), (24.0, 1), (64.0, 1), (12.0, 7919), (24.0, 7919), (64.0, 7919)] {
        let mut r = Rasterizer::new();
        let draw = |r: &mut Rasterizer| {
            for g in 0..n {
                let g = GlyphId(g * step % n);
                let _ = std::hint::black_box(r.glyph(&f, g, &Location::default(), px, &RasterOptions::DEFAULT));
            }
        };
        f.set_cache_budget(Cache::Raster, 0);
        draw(&mut r);
        f.set_cache_budget(Cache::Raster, usize::MAX);
        f.clear_cache(Cache::Raster);
        let before = LIVE.get();
        draw(&mut r);
        let held = LIVE.get() - before;
        let CacheStats { entries, bytes: counted } = f.cache_stats(Cache::Raster);
        assert_eq!(entries, n as usize);
        assert!(held.abs_diff(counted) * 20 <= held, "{px} px, step {step}: {held} B held, {counted} B counted");
    }
}

// A pool holds the fonts in it to its budget together: eight fonts shaping text none repeats grow the heap by what the
// pool keeps, under its 256 KB and the runs this thread's front holds past it, where alone they'd keep 2 MB.
#[test]
fn a_pool_holds_its_fonts_to_its_budget() {
    let fonts: Vec<Font> = (0..8).map(|_| font()).collect();
    let shape = |f: &Font, from: usize| {
        for i in from..from + 400 {
            let text = format!("text none repeats {i}");
            std::hint::black_box(f.shape(&text, &Location::default(), &ShapeOptions::DEFAULT));
        }
    };
    // Each font's tables, indexes and plans built first, outside what's measured.
    for f in &fonts {
        shape(f, 0);
        f.clear_cache(Cache::Shape);
    }
    let pool = crate::CachePool::new(256 << 10);
    fonts.iter().for_each(|f| f.set_cache_pool(Cache::Shape, Some(&pool)));
    let before = LIVE.get();
    fonts.iter().enumerate().for_each(|(k, f)| shape(f, 1000 * (k + 1)));
    let held = LIVE.get().wrapping_sub(before);
    let counted = pool.stats().bytes;
    assert!(held <= (256 << 10) + (64 << 10) && counted <= 256 << 10, "{held} B held, {counted} B counted");
    fonts.iter().for_each(|f| f.set_cache_pool(Cache::Shape, None));
    let alone = LIVE.get().wrapping_sub(before);
    fonts.iter().enumerate().for_each(|(k, f)| shape(f, 20_000 * (k + 1)));
    let grown = LIVE.get().wrapping_sub(before);
    assert!(grown > alone + (1 << 20), "alone the fonts kept only {} B more", grown - alone);
}

// What a rasterizer learned of a glyph stays past 4,096 glyphs and past 64 locations, under a budget in bytes that
// holds them: the first drawn again, at the first location too, learns nothing again and so allocates nothing.
#[test]
fn a_rasterizer_keeps_what_it_learned_past_any_count() {
    use crate::raster::{RasterOptions, Rasterizer};
    let han = format!("{FONTS}/source-han-sans/SourceHanSansJP-VF.otf");
    let han = Font::from_vec(std::fs::read(&han).expect("the fixture"), 0).expect("parses");
    let inter = font();
    let mut r = Rasterizer::new();
    r.set_cache_budget(16 << 20);
    let mut draw = |f: &Font, g: u32, at: &Location| {
        let _ = std::hint::black_box(r.glyph(f, GlyphId(g), at, 16.0, &RasterOptions::DEFAULT));
        ALLOCS.get()
    };
    for g in 1000..6000 {
        draw(&han, g, &Location::default());
    }
    let weights: Vec<Location> = (0..65).map(|w| located(&inter, &[("wght", 100.0 + 12.0 * f64::from(w))])).collect();
    weights.iter().for_each(|at| { draw(&inter, 40, at); });
    ALLOCS.set(0);
    COUNTING.set(true);
    let han_again = draw(&han, 1000, &Location::default());
    let weight_again = draw(&inter, 40, &weights[0]) - han_again;
    COUNTING.set(false);
    assert_eq!((han_again, weight_again), (0, 0), "the first of 5,000 glyphs, or of 65 locations, learned again");
}

// A rasterizer's budget bounds what it learned to the byte it counts: 3,000 glyphs of Source Han Sans at 256 KB grow
// the heap by no more, once its buffers have grown to the largest of them.
#[test]
fn a_rasterizer_holds_what_it_learned_to_its_budget() {
    use crate::raster::{RasterOptions, Rasterizer};
    let han = format!("{FONTS}/source-han-sans/SourceHanSansJP-VF.otf");
    let han = Font::from_vec(std::fs::read(&han).expect("the fixture"), 0).expect("parses");
    let mut r = Rasterizer::new();
    let draw = |r: &mut Rasterizer| {
        for g in 1000..4000 {
            let at = Location::default();
            let _ = std::hint::black_box(r.glyph(&han, GlyphId(g), &at, 16.0, &RasterOptions::DEFAULT));
        }
    };
    r.set_cache_budget(0);
    draw(&mut r);
    r.set_cache_budget(256 << 10);
    let before = LIVE.get();
    draw(&mut r);
    let held = LIVE.get().wrapping_sub(before);
    assert!(held <= (256 << 10) + (16 << 10), "{held} B held against a budget of 256 KB");
    r.set_cache_budget(0);
    assert!(LIVE.get().saturating_sub(before) < 16 << 10, "a budget of 0 left what was learned");
}

// Past its budget a rasterizer lets go of the glyphs least recently drawn: one drawn again after each of a thousand
// others, at a budget that holds a few dozen, learns nothing again.
#[test]
fn a_rasterizer_keeps_the_glyph_it_draws_again() {
    use crate::raster::{RasterOptions, Rasterizer};
    let han = format!("{FONTS}/source-han-sans/SourceHanSansJP-VF.otf");
    let han = Font::from_vec(std::fs::read(&han).expect("the fixture"), 0).expect("parses");
    let mut r = Rasterizer::new();
    r.set_cache_budget(64 << 10);
    let draw = |r: &mut Rasterizer, g| {
        let _ = std::hint::black_box(r.glyph(&han, GlyphId(g), &Location::default(), 16.0, &RasterOptions::DEFAULT));
    };
    draw(&mut r, 1000);
    for g in 2000..3000 {
        draw(&mut r, g);
        // What's past the budget let go first, uncounted, as the queue's own nodes may grow as it goes round.
        r.set_cache_budget(64 << 10);
        ALLOCS.set(0);
        COUNTING.set(true);
        draw(&mut r, 1000);
        COUNTING.set(false);
        assert_eq!(ALLOCS.get(), 0, "glyph 1000 was learned again after glyph {g}");
    }
}

// A thread that read a font at a location keeps it after the font is dropped, until it lets go: on a thread of its
// own, so nothing another test left on the harness's thread is counted.
#[cfg(feature = "std")]
#[test]
fn a_released_thread_gives_back_the_font_it_kept() {
    std::thread::spawn(|| {
        let before = LIVE.get();
        let f = font();
        std::hint::black_box(f.shape("Wave", &located(&f, &[("wght", 700.0)]), &ShapeOptions::DEFAULT));
        drop(f);
        let kept = LIVE.get().wrapping_sub(before);
        crate::release_thread();
        let left = LIVE.get().wrapping_sub(before);
        assert!(kept > 800 << 10 && left < 16 << 10, "{kept} B kept, {left} B left once released");
    })
    .join()
    .expect("the reading thread");
}

// A PairPos of 1,000 covered glyphs whose PairSet offsets all name one set of 1,000 records, its ValueFormat's Device
// field empty in each: narrowing it copies the shared set once, not once a naming, a million records.
#[test]
fn a_pair_set_shared_by_every_glyph_is_narrowed_once() {
    use crate::daecore::daetype::subsetter::{otl::gpos::subset_gpos, GlyphSet};
    const K: u16 = 1_000;
    let (coverage, set) = (10 + 2 * K, 20 + 2 * K);
    let gpos = [
        u16s(&[1, 0, 0, 0, 10, 1, 4, 2, 0, 1, 8, 1, coverage, 0x0010, 0, K]),
        (0..K).flat_map(|_| set.to_be_bytes()).collect(),
        u16s(&[2, 1, 1, K, 0, K]),
        (1..=K).flat_map(|g| u16s(&[g, 0])).collect(),
    ]
    .concat();
    let active: GlyphSet = (0..=u32::from(K)).collect();
    let gid_map: Vec<u32> = (0..=u32::from(K)).collect();
    let held = peak_of(|| subset_gpos(&gpos, &active, &gid_map, &[], false, false).is_some());
    assert!(held < 4 << 20, "subsetting GPOS held {} MB at its peak", held >> 20);
}

// A morx of one ligature subtable: `states` states whose four classes' entries `cells` gives, `entries` of (next
// state, flags, action), `actions` action words with LIG_LAST on the last where `last`, one component and `ligatures`
// ligatures.
fn ligature_morx(
    states: usize, cells: impl Fn(usize) -> [u16; 4], entries: &[[u16; 3]], actions: usize, last: bool,
    ligatures: usize,
) -> Vec<u8> {
    let class = u16s(&[8, 1, 1, 4]);
    let state: Vec<u8> = (0..states).flat_map(|s| u16s(&cells(s))).collect();
    let entry: Vec<u8> = entries.iter().flat_map(|e| u16s(e)).collect();
    let (class_at, state_at) = (28, 28 + class.len());
    let entry_at = state_at + state.len();
    let action_at = entry_at + entry.len();
    let (component_at, ligature_at) = (action_at + 4 * actions, action_at + 4 * actions + 2);
    let word = |i: usize| if last && i + 1 == actions { 0x8000_0000u32 } else { 0 };
    let words: Vec<u8> = (0..actions).flat_map(|i| word(i).to_be_bytes()).collect();
    let offsets = [4, class_at, state_at, entry_at, action_at, component_at, ligature_at];
    let body = [
        offsets.iter().flat_map(|&v| (v as u32).to_be_bytes()).collect(), class, state, entry, words, u16s(&[0]),
        (1..=ligatures as u16).flat_map(u16::to_be_bytes).collect(),
    ]
    .concat();
    let sub = 12 + body.len() as u32;
    let header = [&u16s(&[2, 0])[..], &1u32.to_be_bytes()].concat();
    let chain: Vec<u8> = [1, 16 + sub, 0, 1, sub, 2, 1].iter().flat_map(|v: &u32| v.to_be_bytes()).collect();
    [header, chain, body].concat()
}

// 4,000 action words with no last one, so the walk takes every glyph any word pops: one set of 4,000 ligatures held
// once, not once a word.
#[test]
fn a_morx_walk_given_up_holds_one_set_for_every_word() {
    use crate::daecore::daetype::subsetter::{morx_closure, GlyphSet};
    let morx = ligature_morx(2, |_| [0; 4], &[[0, 0x2000, 0]], 4_000, false, 4_000);
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let held = peak_of(|| morx_closure(&morx, &active, 70_000).len() == 4_000);
    assert!(held < 4 << 20, "closing over morx held {} MB at its peak", held >> 20);
}

// 500 states each pushing on class 0 and passing on the rest to the next, so state s is reached at every depth to 64,
// one action of 64 words and 32,000 ligatures: every node's stack is as wide as the ligatures, and they're held
// within the walk's budget rather than 160 MB of them.
#[test]
fn a_morx_walk_holds_its_stacks_within_its_budget() {
    use crate::daecore::daetype::subsetter::{morx_closure, GlyphSet};
    const STATES: u16 = 500;
    let mut entries: Vec<[u16; 3]> = (0..STATES).flat_map(|s| [[s + 1, 0x8000, 0], [s + 1, 0, 0]]).collect();
    entries.push([0, 0x2000, 0]);
    let cells = |s: usize| [2 * s as u16, 2 * s as u16 + 1, 2 * s as u16 + 1, 2 * s as u16 + 1];
    let morx = ligature_morx(usize::from(STATES), cells, &entries, 64, true, 32_000);
    let active: GlyphSet = [0u32, 1].into_iter().collect();
    let held = peak_of(|| {
        morx_closure(&morx, &active, 70_000);
        true
    });
    assert!(held < 32 << 20, "closing over morx held {} MB at its peak", held >> 20);
}

// A lookup of 32,000 subtable offsets naming one 4-byte ChainContext 3 that claims 65,535 backtrack coverages, read
// from the 1 MB of zeros after it. A plan keeps a slot for each table a count claims, drawn from the font's allowance
// as its indexes are, so a first shape keeps within it; before, 160 MB.
#[test]
fn a_plan_s_rule_slots_are_drawn_from_the_allowance() {
    let subtables = 32_000u16;
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let head = [words(&[1, 0, 10, 30, 44, 1]), b"DFLT".to_vec(), words(&[8, 4, 0, 0, 0xFFFF, 1, 0, 1])].concat();
    let lists = [head, b"ccmp".to_vec(), words(&[8, 0, 1, 0, 1, 4])].concat();
    let offsets = words(&vec![6 + 2 * subtables; usize::from(subtables)]);
    let gsub = [lists, words(&[6, 0, subtables]), offsets, words(&[3, 65_535]), vec![0; 1 << 20]].concat();
    let f = font();
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"GSUB"), gsub);
    let hostile = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    let before = LIVE.get();
    drop(hostile.shape("a", &Location::default(), &ShapeOptions::default()));
    let kept = LIVE.get() - before;
    assert!(kept < 32 << 20, "a first shape kept {} MB", kept >> 20);
}

// 100 lookups naming one lookup of 32,000 subtable offsets: an index holds an entry for each subtable, 384 B, drawn
// from the allowance with the rest, so a first shape keeps within it; before, 1.2 GB from 64 KB of GSUB.
#[test]
fn a_lookup_s_entries_are_drawn_from_the_allowance() {
    let (lookups, subtables) = (100, 32_000);
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let feature = [words(&[0, lookups]), words(&(0..lookups).collect::<Vec<_>>())].concat();
    let lists = [words(&[1, 0, 10, 30, 38 + feature.len() as u16, 1]), b"DFLT".to_vec()].concat();
    let lists = [lists, words(&[8, 4, 0, 0, 0xFFFF, 1, 0, 1]), b"ccmp".to_vec(), words(&[8]), feature].concat();
    let list = [words(&[lookups]), words(&vec![2 + 2 * lookups; usize::from(lookups)])].concat();
    let offsets = words(&vec![6 + 2 * subtables; usize::from(subtables)]);
    let gsub = [lists, list, words(&[1, 0, subtables]), offsets, words(&[1, 6, 0, 1, 0])].concat();
    let f = font();
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    tables.insert(Tag::new(b"GSUB"), gsub);
    let hostile = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    let before = LIVE.get();
    drop(hostile.shape("a", &Location::default(), &ShapeOptions::default()));
    let kept = LIVE.get() - before;
    assert!(kept < 32 << 20, "a first shape kept {} MB", kept >> 20);
}

// Inter's names, its family 5,000 characters long, beside 4,000 more English full names: a named instance's full
// name, 10 KB, is encoded once for every record it replaces to share, where a copy each held 40 MB.
#[test]
fn a_named_instance_s_full_name_is_encoded_once_for_every_record_it_replaces() {
    use crate::daecore::daetype::decoder::{lang_tags, name_records, write_name_table, NameKey};
    let f = font();
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        f.table_tags().map(|t| (t, f.table(t).expect("listed").to_vec())).collect();
    let name = f.table(Tag::new(b"name")).expect("a name table");
    let family: Vec<u8> = "F".repeat(5_000).encode_utf16().flat_map(u16::to_be_bytes).collect();
    let key = |name_id| NameKey { platform: 3, encoding: 1, language: 0x409, name_id };
    let mut records: Vec<(NameKey, &[u8])> = name_records(name).filter(|(k, _)| !matches!(k.name_id, 1 | 16)).collect();
    records.push((key(1), &family));
    records.extend((0..4_000).map(|_| (key(4), &[][..])));
    tables.insert(Tag::new(b"name"), write_name_table(records, &lang_tags(name)).expect("a name table"));
    let hostile = Font::from_vec(crate::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    let bold = located(&hostile, &[("wght", 700.0)]);
    let mut full = alloc::string::String::new();
    let held = peak_of(|| {
        hostile.visit_names(&bold, |id, name| if id == crate::NameId(4) { full = name.into() });
        true
    });
    assert_eq!(full, format!("{} Bold", "F".repeat(5_000)), "the instance's full name");
    assert!(held < 4 << 20, "naming the instance held {} MB at its peak", held >> 20);
}
