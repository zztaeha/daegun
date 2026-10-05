use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use daegun::{Font, OutlinePen};

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

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static COUNTING: AtomicBool = AtomicBool::new(false);

// Frees of one size nothing else here allocates, counted always, so a test can see its own buffer go.
const ODD_SIZE: usize = 12_345;
static ODD_FREES: AtomicUsize = AtomicUsize::new(0);

// Refused by the allocator below, as a real one refuses what it cannot find, so a failing allocation
// can be tested without asking for terabytes.
const FAIL_SIZE: usize = 1 << 40;

// Bytes held, and the most held since a test last reset it to what was held then.
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    PEAK.fetch_max(LIVE.fetch_add(by, Ordering::Relaxed) + by, Ordering::Relaxed);
}

// The allocation counter is process-wide, so a test that allocates must not run while another counts.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
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
            ODD_FREES.fetch_add(1, Ordering::Relaxed);
        }
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            LIVE.fetch_sub(l.size(), Ordering::Relaxed);
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
    Font::from_vec(bytes).expect("parses")
}

fn allocs_per_call(f: impl FnMut()) -> f64 {
    allocs_over(ROUNDS, f)
}

fn allocs_over(rounds: usize, mut f: impl FnMut()) -> f64 {
    for _ in 0..8 {
        f();
    }
    ALLOCS.store(0, Ordering::Relaxed);
    COUNTING.store(true, Ordering::Relaxed);
    for _ in 0..rounds {
        f();
    }
    COUNTING.store(false, Ordering::Relaxed);
    ALLOCS.load(Ordering::Relaxed) as f64 / rounds as f64
}

// One test, not three: the counter is process-wide, so cases measuring it cannot run concurrently.
// A hit allocates one string per axis tag, the vector holding them, and the text of the key.
#[test]
fn hot_paths_stay_within_their_allocation_budget() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let font = font();
    let text = "your all-in-one text engine";

    let axes: &[(&str, f64)] = &[("wght", 400.0)];
    let with_axes = allocs_per_call(|| {
        std::hint::black_box(font.shape(text, axes, false));
    });
    let without = allocs_per_call(|| {
        std::hint::black_box(font.shape(text, &[], false));
    });

    let gid = font.glyph_id('a' as u32).expect("font has a");
    let outline = allocs_per_call(|| {
        let mut pen = Sink::default();
        std::hint::black_box(font.outline_glyph(gid, &mut pen));
    });

    assert!(with_axes <= 3.0, "a shape cache hit allocated {with_axes} times, budget is 3");
    assert!(without <= 1.0, "an axis-free shape cache hit allocated {without} times, budget is 1");
    assert!(outline <= 2.0, "outline extraction allocated {outline} times, budget is 2");

    let opts = daegun::OutlineOptions::default();
    let prepared = allocs_per_call(|| {
        let mut pen = Sink::default();
        std::hint::black_box(font.prepared_outline(gid, 40.0, &[], &opts, &mut pen));
    });
    let quads = allocs_per_call(|| {
        let _ = std::hint::black_box(font.glyph_quads(gid, &[]));
    });
    let mut path = daegun::Path::default();
    font.outline_glyph(gid, &mut path).expect("a has an outline");
    let max_area = daegun::max_area_for(40.0, f32::from(font.upm()));
    let flat = allocs_per_call(|| {
        std::hint::black_box(daegun::flatten(&path, max_area));
    });
    assert!(prepared <= 10.0, "a plain prepared outline allocated {prepared} times, budget is 10");
    assert!(quads <= 2.0, "glyph_quads allocated {quads} times, budget is 2");
    assert!(flat <= 10.0, "flattening allocated {flat} times, budget is 10");
}

#[cfg(feature = "capi")]
unsafe extern "C" {
    fn daegun_font_buffer_new(len: usize) -> *mut u8;
    fn daegun_font_open_owned(data: *mut u8, len: usize, out: *mut *mut core::ffi::c_void) -> i32;
}

// The header promises NULL when the allocation fails, where an allocation that cannot fail would abort.
#[cfg(feature = "capi")]
#[test]
fn a_buffer_the_allocator_refuses_is_null() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    assert!(unsafe { daegun_font_buffer_new(FAIL_SIZE) }.is_null(), "a buffer the allocator refused was handed out");
}

// The header says daegun_font_open_owned takes the buffer whatever it returns, so a caller who passes
// a NULL out and lets go of the buffer must not leak it.
#[cfg(feature = "capi")]
#[test]
fn an_owned_buffer_is_freed_even_when_out_is_null() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let before = ODD_FREES.load(Ordering::Relaxed);
    unsafe {
        let buf = daegun_font_buffer_new(ODD_SIZE);
        assert!(!buf.is_null(), "no buffer");
        daegun_font_open_owned(buf, ODD_SIZE, core::ptr::null_mut());
    }
    assert_eq!(ODD_FREES.load(Ordering::Relaxed) - before, 1, "open_owned with a NULL out kept the buffer");
}

// A curve flattens to as many as 256 points, so 100,000 of them flat take 200 MB, and the stroke built
// on them over a gigabyte. The stroker stops at MAX_FLATTEN_POINTS instead, and draws nothing.
#[test]
fn a_stroke_stops_flattening_at_the_point_cap() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut path = daegun::Path::default();
    path.move_to(0.0, 0.0);
    for i in 0..100_000 {
        let (a, b) = if i % 2 == 0 { (0.0, 100.0) } else { (100.0, 0.0) };
        path.curve_to(a * 0.7 + b * 0.3, 100.0, a * 0.3 + b * 0.7, -100.0, b, 0.0);
    }
    let style = daegun::StrokeStyle { width: 1.0, join: daegun::Join::Bevel, cap: daegun::Cap::Butt };
    let mut pen = Sink::default();
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    daegun::stroke(&path, &style, 0.001, &mut pen);
    let held = PEAK.load(Ordering::Relaxed) - before;
    assert_eq!(pen.0, 0, "a stroke past the cap drew");
    assert!(held < 64 << 20, "stroking held {} MB at its peak", held >> 20);
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
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = font();
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> =
        f.table_tags().into_iter().map(|t| (t.to_string(), f.table(t).expect("listed").to_vec())).collect();
    tables.insert("GPOS".into(), gpos_with_classes(2000));
    let hostile = Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses");
    let gids: Vec<u16> = (0..=2000).collect();
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let _ = hostile.subset(&gids, &[]);
    let held = PEAK.load(Ordering::Relaxed) - before;
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
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> =
        f.table_tags().into_iter().map(|t| (t.to_string(), f.table(t).expect("listed").to_vec())).collect();
    tables.insert("GPOS".into(), gpos_with_grid(classes));
    let patched = Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses");
    let gids: Vec<u16> = (0..patched.num_glyphs()).collect();
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let _ = patched.subset(&gids, &[]);
    PEAK.load(Ordering::Relaxed) - before
}

// A grid of 150 by 150 classes is 45 KB, and each cell two 24-byte records. As a Struct, its Vec and
// two 80-byte values, a cell would be about 136 times the bytes it is read from.
#[test]
fn a_class_grid_holds_a_small_multiple_of_its_bytes() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = font();
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> =
        f.table_tags().into_iter().map(|t| (t.to_string(), f.table(t).expect("listed").to_vec())).collect();
    tables.insert("GPOS".into(), gpos_naming_one_device(6000));
    let hostile = Font::from_vec(daegun::build_font(&tables)).expect("the patched font parses");
    let gids: Vec<u16> = (0..hostile.num_glyphs()).collect();
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let _ = hostile.subset(&gids, &[]);
    let held = PEAK.load(Ordering::Relaxed) - before;
    assert!(held < 64 << 20, "subsetting held {} MB at its peak", held >> 20);
}

// Two combs of 14 teeth across each other resolve into hundreds of small contours. What comes back
// holds those points, not room for every edge the union had left when each contour began.
#[test]
fn resolved_contours_hold_only_their_points() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let combs = vec![comb(false), comb(true)];
    let before = LIVE.load(Ordering::Relaxed);
    let resolved = daegun::resolve_overlaps(&combs).expect("the combs overlap");
    let held = LIVE.load(Ordering::Relaxed) - before;
    let points: usize = resolved.iter().map(Vec::len).sum();
    assert!(held < 64 * points + (1 << 16), "{} contours of {points} points hold {} KB", resolved.len(), held >> 10);
}

// At max_area 0 every curve becomes 4,097 points, so 30,000 of them would read 123 million before the
// final count refused them. The loop stops at twice the limit instead.
#[test]
fn flattening_stops_reading_past_the_point_cap() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut path = daegun::Path::default();
    path.move_to(0.0, 0.0);
    for i in 0..30_000 {
        let x = (i % 100) as f32 * 10.0;
        path.curve_to(x + 3.0, 50.0, x + 6.0, -50.0, x + 10.0, 0.0);
    }
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    assert!(daegun::flatten(&path, 0.0).is_none(), "123 million points were flattened");
    let held = PEAK.load(Ordering::Relaxed) - before;
    assert!(held < 64 << 20, "flattening held {} MB at its peak", held >> 20);
}

// 2,000 index records of one 5,000-glyph bitmap subtable: about 40 KB of CBLC that would rebuild 40 MB of
// index. The subset is refused, and without building it all first.
#[test]
fn a_bitmap_subset_stops_at_its_budget() {
    use daegun::daecore::daetype::subsetter::{subset_bitmap_strikes, GlyphSet};
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    const N: usize = 2_000;
    const GLYPHS: u16 = 5_000;
    let mut cblc = [0x0003_0000u32, 1, 56, 0, N as u32, 0].map(u32::to_be_bytes).concat();
    cblc.resize(56, 0);
    for _ in 0..N {
        cblc.extend([1u16, GLYPHS].map(u16::to_be_bytes).concat());
        cblc.extend(((N * 8) as u32).to_be_bytes());
    }
    cblc.extend([1u16, 17].map(u16::to_be_bytes).concat());
    cblc.extend(4u32.to_be_bytes());
    cblc.extend((0..=u32::from(GLYPHS)).flat_map(u32::to_be_bytes));
    let mut cbdt = 0x0003_0000u32.to_be_bytes().to_vec();
    cbdt.resize(4 + usize::from(GLYPHS), 7);
    let mut active = GlyphSet::new();
    (1..=GLYPHS).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u16> = (0..=GLYPHS).collect();

    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    assert!(subset_bitmap_strikes(&cblc, &cbdt, &active, &gid_map).is_none(), "the index was built");
    let held = PEAK.load(Ordering::Relaxed) - before;
    assert!(held < 4 << 20, "subsetting the strike held {} MB at its peak", held >> 20);
}

// Offsets alternating over one 20 KB image, so every other glyph of 4,000 names the same bytes:
// 36 KB of table that, copied at each naming, would hold 77 MB (sbix) and 39 MB (CBLC) at the peak.
const OVERLAPPING: usize = 4_000;
const IMAGE: usize = 20_000;

fn peak_of(f: impl FnOnce() -> bool) -> usize {
    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    assert!(f(), "the call measured did not return what the test expects");
    PEAK.load(Ordering::Relaxed) - before
}

#[test]
fn an_sbix_strike_of_overlapping_records_stops_at_its_budget() {
    use daegun::daecore::daetype::subsetter::subset_sbix;
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let head = 4 + 4 * (OVERLAPPING + 1);
    let mut sbix = [1u16, 1].map(u16::to_be_bytes).concat();
    sbix.extend([1u32, 12].map(u32::to_be_bytes).concat());
    sbix.extend([16u16, 72].map(u16::to_be_bytes).concat());
    (0..=OVERLAPPING).for_each(|g| sbix.extend(((head + IMAGE * (g % 2)) as u32).to_be_bytes()));
    sbix.extend(b"\0\0\0\0png ");
    sbix.resize(12 + head + IMAGE, 7);
    let active: Vec<u16> = (0..OVERLAPPING as u16).collect();
    let held = peak_of(|| subset_sbix(&sbix, OVERLAPPING, &active, &active).is_none());
    assert!(held < 4 << 20, "subsetting the strike held {} MB at its peak", held >> 20);
}

#[test]
fn a_bitmap_subtable_of_overlapping_images_stops_at_its_budget() {
    use daegun::daecore::daetype::subsetter::{subset_bitmap_strikes, GlyphSet};
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    (1..=OVERLAPPING as u16).for_each(|g| { active.insert(g); });
    let gid_map: Vec<u16> = (0..=OVERLAPPING as u16).collect();
    let held = peak_of(|| subset_bitmap_strikes(&cblc, &cbdt, &active, &gid_map).is_none());
    assert!(held < 4 << 20, "subsetting the strike held {} MB at its peak", held >> 20);
}

fn u16s(words: &[u16]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn all_glyphs(n: u16) -> (daegun::daecore::daetype::subsetter::GlyphSet, Vec<u16>) {
    let mut active = daegun::daecore::daetype::subsetter::GlyphSet::new();
    (0..n).for_each(|g| { active.insert(g); });
    (active, (0..n).collect())
}

// A version 2 morx or kerx holding these (type or format, body) subtables.
fn aat_table(morx: bool, subtables: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let body: Vec<u8> = subtables.iter()
        .flat_map(|(kind, b)| [[(12 + b.len()) as u32, *kind, u32::from(morx)].map(u32::to_be_bytes).concat(), b.clone()].concat())
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
    use daegun::daecore::daetype::subsetter::{subset_kerx, subset_morx};
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let segment = u16s(&[2, 6, 2, 12, 1, 0, 65_533, 1, 4, 0xFFFF, 0xFFFF, 0]);
    let at = 20 + segment.len();
    let state = [[5u32, 20, at as u32, (at + 20) as u32, (at + 24) as u32].map(u32::to_be_bytes).concat(), segment.clone(), vec![0; 20], u16s(&[0, 0, 0xFFFF, 0])].concat();
    let morx = aat_table(true, &(0..100).map(|_| (4, segment.clone())).collect::<Vec<_>>());
    let kerx = aat_table(false, &(0..100).map(|_| (1, state.clone())).collect::<Vec<_>>());
    let (active, gid_map) = all_glyphs(u16::MAX);
    let held = peak_of(|| subset_morx(&morx, &active, &gid_map, u16::MAX).is_some_and(|m| m.len() < 2 * morx.len()));
    assert!(held < 4 << 20, "subsetting morx held {} MB at its peak", held >> 20);
    let held = peak_of(|| subset_kerx(&kerx, &active, &gid_map, u16::MAX).is_some_and(|k| k.len() < 2 * kerx.len()));
    assert!(held < 4 << 20, "subsetting kerx held {} MB at its peak", held >> 20);
}

// 2,000 classed glyphs and 2,000 ligature actions at one offset: an action each taking its own range
// of components would make 8 MB from 16 KB.
#[test]
fn ligature_actions_sharing_an_offset_share_their_components() {
    use daegun::daecore::daetype::subsetter::subset_morx;
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    const N: u16 = 2_000;
    let classes = u16s(&[2, 6, 2, 12, 1, 0, N, 1, 4, 0xFFFF, 0xFFFF, 0]);
    let (class_at, state_at) = (28, 28 + classes.len());
    let entry_at = state_at + 20;
    let action_at = entry_at + 6;
    let component_at = action_at + 4 * usize::from(N);
    let ligature_at = component_at + 2 * usize::from(N);
    let body = [
        [5u32, class_at as u32, state_at as u32, entry_at as u32, action_at as u32, component_at as u32, ligature_at as u32].map(u32::to_be_bytes).concat(),
        classes, vec![0; 20], u16s(&[0, 0, 0]),
        (0..N).flat_map(|_| 0xBFFF_FFFFu32.to_be_bytes()).collect(),
        (0..N).flat_map(u16::to_be_bytes).collect(),
        (1..=N).flat_map(u16::to_be_bytes).collect(),
    ].concat();
    let morx = aat_table(true, &[(2, body)]);
    let (active, gid_map) = all_glyphs(N + 1);
    let held = peak_of(|| subset_morx(&morx, &active, &gid_map, N + 1).is_some());
    assert!(held < 4 << 20, "subsetting the ligatures held {} MB at its peak", held >> 20);
}

// 500 glyphs naming one 50 KB GlyphInfo through their Zapf offsets: 52 KB, or 25 MB copied per glyph.
#[test]
fn a_glyph_info_many_glyphs_share_is_written_once() {
    use daegun::daecore::daetype::subsetter::subset_zapf;
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    let (active, gid_map) = all_glyphs(N);
    let held = peak_of(|| subset_zapf(&zapf, usize::from(N), &gid_map, &active, &gid_map).is_some());
    assert!(held < 4 << 20, "subsetting Zapf held {} MB at its peak", held >> 20);
}

// One script whose default and 50 language records name one language system of 50 priorities, all naming
// one priority whose eight slots name one list of 500 lookups: 1,442 bytes, 20.5 MB copied per naming.
#[test]
fn a_jstf_part_named_many_times_is_built_once() {
    use daegun::daecore::daetype::subsetter::subset_jstf;
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

// A CID-keyed CFF whose 2,000 FDs all name one Private DICT with 200 KB of Subrs: 400 MB if each FD
// copied them in. The subset reads them once.
#[test]
fn a_shared_private_dict_is_read_once() {
    use daegun::daecore::daetype::subsetter::subset_cff;
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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

    let before = LIVE.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let out = subset_cff(&cff, &[0, 1]);
    let held = PEAK.load(Ordering::Relaxed) - before;
    assert!(out.is_ok(), "the subset failed: {:?}", out.err());
    assert!(held < 16 << 20, "subsetting held {} MB at its peak", held >> 20);
}

// One glyph whose MathKern has 4,001 records all naming one 32 KB device: 48 KB of MATH that, copying
// the device at each naming, would hold 131 MB. The subset keeps the MathKern and the device once.
#[test]
fn a_device_every_math_kern_record_names_is_copied_once() {
    use daegun::daecore::daetype::subsetter::subset_math;
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
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
    use daegun::daecore::daetype::subsetter::{math_closure, subset_math};
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    const N: u16 = 1_000;
    let coverage = 10 + 2 * N;
    let region = coverage + 4 + 2 * N;
    let math = [
        u16s(&[1, 0, 0, 0, 10, 0, coverage, 0, N, 0]),
        (0..N).flat_map(|i| (region + 4 * i).to_be_bytes()).collect(),
        u16s(&[1, N]), (1..=N).flat_map(u16::to_be_bytes).collect(),
        (0..u32::from(N) + 65_535).flat_map(|_| u16s(&[0, 0xFFFF])).collect(),
    ].concat();
    let (active, gid_map) = all_glyphs(N + 1);
    let held = peak_of(|| math_closure(&math, &active) == [0]);
    assert!(held < 4 << 20, "the MATH closure held {} MB at its peak", held >> 20);
    let held = peak_of(|| subset_math(&math, &active, &gid_map).is_none());
    assert!(held < 4 << 20, "subsetting MATH held {} MB at its peak", held >> 20);
}

// A 200 KB glyf that every even glyph's loca range names whole: 400 glyphs would copy it 200 times,
// a 40 MB subset. Copying stops at the subset budget.
#[test]
fn overlapping_loca_ranges_stop_at_the_subset_budget() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    const GLYF: u32 = 200_000;
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/eb-garamond/EBGaramond.ttf")).expect("fixture")).expect("parses");
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> =
        f.table_tags().into_iter().map(|t| (t.to_string(), f.table(t).expect("listed").to_vec())).collect();
    tables.insert("glyf".into(), vec![0; GLYF as usize]);
    tables.insert("loca".into(), (0..=u32::from(f.num_glyphs())).flat_map(|i| (GLYF * (i % 2)).to_be_bytes()).collect());
    tables.get_mut("head").expect("head")[50..52].copy_from_slice(&[0, 1]);
    let g = Font::from_vec(daegun::build_font(&tables)).expect("parses");
    let gids: Vec<u16> = (0..400).collect();
    let held = peak_of(|| g.subset(&gids, &[]).is_err());
    assert!(held < 4 << 20, "subsetting glyf held {} MB at its peak", held >> 20);
}

// One lookup of 1,000 SingleSubsts sharing a coverage of one range over 65,534 glyphs: 8 KB of GSUB
// naming 65 million substitutions. The closure refuses it at the subset budget instead of holding them.
#[test]
fn substitutions_past_the_subset_budget_refuse_the_closure() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    const N: u16 = 1_000;
    let subtables_at = 6 + 2 * N;
    let coverage = subtables_at + 6 * N;
    let lookup = [u16s(&[1, 0, N]), (0..N).flat_map(|i| (subtables_at + 6 * i).to_be_bytes()).collect(),
                  (0..N).flat_map(|i| u16s(&[1, coverage - (subtables_at + 6 * i), 1])).collect(), u16s(&[2, 1, 1, 65_534, 0])].concat();
    let gsub = [u16s(&[1, 0, 10, 30, 44, 1]), b"DFLT".to_vec(), u16s(&[8, 4, 0, 0, 0xFFFF, 1, 0]),
                u16s(&[1]), b"ccmp".to_vec(), u16s(&[8, 0, 1, 0]), u16s(&[1, 4]), lookup].concat();
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/eb-garamond/EBGaramond.ttf")).expect("fixture")).expect("parses");
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> =
        f.table_tags().into_iter().map(|t| (t.to_string(), f.table(t).expect("listed").to_vec())).collect();
    tables.insert("GSUB".into(), gsub);
    let g = Font::from_vec(daegun::build_font(&tables)).expect("parses");
    let held = peak_of(|| g.glyph_closure(&[1], &[]).is_err());
    assert!(held < 4 << 20, "the closure held {} MB at its peak", held >> 20);
}

// Each face of a collection holds its own tables, not the whole file, which for both faces of a
// two-font collection would be twice the file.
#[test]
fn faces_of_a_collection_hold_only_their_tables() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let bytes = std::fs::read(format!("{FONTS}/test-fixtures/EBGaramond-InterVariable.ttc")).expect("fixture");
    let before = LIVE.load(Ordering::Relaxed);
    let faces: Vec<Font> = (0..Font::ttc_font_count(&bytes)).map(|i| Font::from_ttc(&bytes, i).expect("a face")).collect();
    let held = LIVE.load(Ordering::Relaxed) - before;
    assert_eq!(faces.len(), 2);
    assert!(held * 10 < bytes.len() * 12, "{} faces hold {held} bytes of a {}-byte file", faces.len(), bytes.len());
}

// One glyph's CFF name reads the INDEX headers and its one string, not every INDEX, which is 177 KB
// for each STIX glyph named.
#[test]
fn a_cff_glyph_name_reads_only_its_string() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/stix-two-math/STIX2Math.otf")).expect("fixture")).expect("parses");
    let held = peak_of(|| f.glyph_name(5_000).is_some());
    assert!(held < 1 << 10, "naming one glyph held {held} bytes");
}

// Source Serif's GDEF with a ligature of two format 1 carets: they need no variation store, which
// would be 249 KB parsed on every call at a varied location.
#[test]
fn carets_that_do_not_vary_read_no_variation_store() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/source-serif/SourceSerif4Variable-Roman.otf")).expect("fixture")).expect("parses");
    let gdef = f.table("GDEF").expect("GDEF").to_vec();
    let store = u32::from_be_bytes([gdef[14], gdef[15], gdef[16], gdef[17]]) as usize;
    let list = u16s(&[6, 1, 12, 1, 1, 100, 2, 6, 10, 1, 300, 1, 600]);
    let mut tables: std::collections::BTreeMap<String, Vec<u8>> =
        f.table_tags().into_iter().map(|t| (t.to_string(), f.table(t).expect("listed").to_vec())).collect();
    tables.insert("GDEF".into(), [u16s(&[1, 3, 0, 0, 18, 0, 0]), ((18 + list.len()) as u32).to_be_bytes().to_vec(), list, gdef[store..].to_vec()].concat());
    let g = Font::from_vec(daegun::build_font(&tables)).expect("parses");
    let held = peak_of(|| g.ligature_carets(100, &[("wght", 700.0)], false).len() == 2);
    assert!(held < 16 << 10, "two carets held {} KB", held >> 10);
}

// Only a format 2 caret reads the outline at the location. Inter's `a` has no carets, so asking for
// them at wght 700 instances none of Inter, at this location or any other.
#[test]
fn carets_without_a_point_instance_nothing() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/inter/InterVariable.ttf")).expect("fixture")).expect("parses");
    let a = f.glyph_id('a' as u32).expect("a");
    let held = peak_of(|| f.ligature_carets(a, &[("wght", 700.0)], false).is_empty());
    assert!(held < 4 << 10, "no carets held {} KB", held >> 10);
}

// A hinting context is kept for each of the last few sizes, so a glyph hinted at two sizes in turn
// allocates what one size does twice, not a new context's programs, storage and zone at each switch.
#[test]
fn alternating_sizes_keep_their_hinting_contexts() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = Font::from_vec(std::fs::read(format!("{FONTS}/test-fixtures/hinted.ttf")).expect("fixture")).expect("parses");
    let opts = daegun::OutlineOptions::default().with_hinting(daegun::HintMode::Classic);
    let draw = |px: f32| { f.prepared_outline(1, px, &[], &opts, &mut Sink::default()); };
    let one = allocs_per_call(|| { draw(16.0); draw(16.0) });
    let two = allocs_per_call(|| { draw(12.0); draw(16.0) });
    assert!(two <= one, "alternating sizes made {two} allocations where one size made {one}");
}

// Varying the outlines reuses its buffers from glyph to glyph, so an instance off the default allocates
// little more than one at it: about 200 more over Inter's 2,937 glyphs, not 33,400.
#[test]
fn an_instance_allocates_nothing_glyph_by_glyph() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let f = font();
    // Each call a new location, so none is the cached instance; a step of 1e-9 never leaves the default.
    let instance = |wght: &mut f64, step: f64| {
        *wght += step;
        std::hint::black_box(f.instance(&[("wght", *wght)]));
    };
    let (mut at, mut off) = (400.0, 400.0);
    let default = allocs_over(4, || instance(&mut at, 1e-9));
    let varied = allocs_over(4, || instance(&mut off, 0.37));
    let glyphs = f64::from(f.num_glyphs());
    assert!(varied - default < glyphs / 8.0, "an instance off the default made {varied} allocations, {default} at it");
}

// The outline cache's budget bounds the heap it holds, not only the bytes it counts: a path is kept
// without its growth slack and charged for its place in the cache too.
#[test]
fn the_outline_cache_holds_what_it_counts() {
    let _serial = SERIAL.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for name in ["inter/InterVariable.ttf", "eb-garamond/EBGaramond.ttf", "stix-two-math/STIX2Math.otf"] {
        let bytes = std::fs::read(format!("{FONTS}/{name}")).expect("fixture");
        let f = Font::from_vec(bytes).expect("parses");
        f.set_outline_cache_bytes(262_144);
        f.prewarm(0..600, &[]);
        f.clear_prewarm();
        let before = LIVE.load(Ordering::Relaxed);
        f.prewarm(0..600, &[]);
        let held = LIVE.load(Ordering::Relaxed) - before;
        let (_, counted) = f.outline_cache_stats();
        assert!(held * 100 <= counted * 105, "{name}: the cache counts {counted} bytes and holds {held}");
    }
}
