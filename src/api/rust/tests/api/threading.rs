use std::sync::Arc;
use std::time::Instant;

use daegun::outline::HintMode;
use daegun::shape::{Direction, ShapeOptions};
use daegun::{Cache, Font, GlyphId, Location};

use crate::located;

fn font() -> Font {
    let path = format!("{}/eb-garamond/EBGaramond.ttf", crate::FONTS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).expect("parses")
}

fn outline_digest(font: &Font, gid_end: u32) -> u64 {
    outline_digest_at(font, &Location::default(), gid_end)
}

fn outline_digest_at(font: &Font, at: &Location, gid_end: u32) -> u64 {
    #[derive(Default)]
    struct Digest(u64);
    impl Digest {
        fn f32(&mut self, v: f32) {
            for b in v.to_bits().to_le_bytes() {
                self.0 ^= u64::from(b);
                self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    impl daegun::outline::OutlinePen for Digest {
        fn move_to(&mut self, x: f32, y: f32) { self.f32(1.0); self.f32(x); self.f32(y); }
        fn line_to(&mut self, x: f32, y: f32) { self.f32(2.0); self.f32(x); self.f32(y); }
        fn quad_to(&mut self, a: f32, b: f32, x: f32, y: f32) {
            self.f32(3.0); self.f32(a); self.f32(b); self.f32(x); self.f32(y);
        }
        fn curve_to(&mut self, a: f32, b: f32, c: f32, d: f32, x: f32, y: f32) {
            self.f32(4.0); self.f32(a); self.f32(b); self.f32(c); self.f32(d); self.f32(x); self.f32(y);
        }
        fn close(&mut self) { self.f32(5.0); }
    }

    let mut d = Digest(0xcbf2_9ce4_8422_2325);
    for gid in 0..gid_end {
        let _ = font.outline(GlyphId(gid), at, &mut d);
    }
    d.0
}

#[test]
fn concurrent_outlining_agrees_with_one_thread() {
    let font = font();
    let n = font.glyph_count().min(800);
    let alone = outline_digest(&font, n);

    let shared = Arc::new(self::font());
    let digests: Vec<u64> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let f = Arc::clone(&shared);
                s.spawn(move || outline_digest(&f, n))
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("thread panicked")).collect()
    });

    for (i, d) in digests.iter().enumerate() {
        assert_eq!(*d, alone, "thread {i} outlined a different shape than a single thread did");
    }
}

// Threads drawing one location's glyphs race to keep each one's deltas, which they read without a lock, and each draws
// what a thread alone there draws.
#[test]
fn concurrent_outlining_at_a_location_agrees_with_one_thread() {
    let read = || std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("the font reads");
    let open = || Font::from_bytes(&read()).expect("Inter opens");
    let font = open();
    let n = font.glyph_count().min(800);
    let alone = outline_digest_at(&font, &located(&font, &[("wght", 700.0)]), n);
    assert_ne!(alone, outline_digest(&font, n), "a location that moves nothing proves nothing");
    let shared = open();
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| assert_eq!(outline_digest_at(&shared, &located(&shared, &[("wght", 700.0)]), n), alone));
        }
    });
}

// A thread resolves each font's locations as a fresh thread does: past how many it keeps, and on two fonts naming one
// axis, whose keys read alike and whose coordinates don't.
#[test]
fn a_thread_resolves_each_fonts_locations_as_a_fresh_thread_does() {
    let read = |rel: &str| std::fs::read(format!("{}/{rel}", crate::FONTS)).expect("the font reads");
    let open = |rel: &str| Font::from_bytes(&read(rel)).expect("the font opens");
    let fonts = [open("inter/InterVariable.ttf"), open("source-han-sans/SourceHanSansJP-VF.otf")];
    let weights: Vec<f64> = (0..20).map(|i| 300.0 + 25.0 * f64::from(i)).collect();
    let resolve = |font: &Font, w: f64| {
        let at = located(font, &[("wght", w)]);
        (at.coords().to_vec(), at.normalized().to_vec())
    };
    let fresh = |font: &Font, w: f64| {
        std::thread::scope(|s| s.spawn(|| resolve(font, w)).join().expect("the thread resolves"))
    };
    let alone: Vec<Vec<_>> = fonts.iter().map(|font| weights.iter().map(|&w| fresh(font, w)).collect()).collect();
    assert_ne!(alone[0][8].1, alone[1][8].1, "fonts whose coordinates agree prove nothing");
    assert_eq!(alone[0][8].0, alone[1][8].0, "keys that differ can't be taken for each other");
    for _ in 0..2 {
        for (w, &weight) in weights.iter().enumerate() {
            for (f, font) in fonts.iter().enumerate() {
                assert_eq!(resolve(font, weight), alone[f][w], "font {f} at wght {weight}");
            }
        }
    }
}

// Threads at four weights of one font, under a budget that keeps only the newest location, drop each other's
// locations as they go: each still shapes as one thread alone does, and the font still keeps one location.
#[test]
fn located_shaping_under_a_tight_budget_agrees_with_one_thread() {
    let bytes = std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("fixture");
    let weights = [300.0, 500.0, 700.0, 900.0];
    let shape = |f: &Font, w: f64| {
        format!("{:?}", f.shape("Office affinity, AVA Wave.", &located(f, &[("wght", w)]), &Default::default()))
    };
    let solo = Font::from_bytes(&bytes).expect("parses");
    let alone: Vec<String> = weights.iter().map(|&w| shape(&solo, w)).collect();
    let shared = Font::from_bytes(&bytes).expect("parses");
    shared.set_cache_budget(Cache::Instance, 0);
    std::thread::scope(|s| {
        for t in 0..8 {
            let (shared, alone, shape) = (&shared, &alone, &shape);
            s.spawn(move || {
                for round in 0..50 {
                    let k = (t + round) % weights.len();
                    assert_eq!(shape(shared, weights[k]), alone[k], "thread {t} shaped wght {} apart", weights[k]);
                }
            });
        }
    });
    assert_eq!(shared.cache_stats(Cache::Instance).entries, 1, "a budget of 0 kept more than the newest location");
}

// Eight threads shape, draw, rasterize and instance through four fonts sharing one pool for every cache, while a ninth
// sets its budget and moves the fonts' caches out of it and back: each answer is a lone font's, and the pool counts
// what its fonts hold.
#[test]
fn fonts_sharing_a_pool_answer_alike_across_threads() {
    use daegun::raster::{RasterOptions, Rasterizer};
    use daegun::CachePool;
    use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
    const CACHES: [Cache; 4] = [Cache::Shape, Cache::Instance, Cache::Outline, Cache::Raster];
    let bytes = std::fs::read(format!("{}/inter/InterVariable.ttf", crate::FONTS)).expect("fixture");
    let open = || Font::from_bytes(&bytes).expect("parses");
    let ask = |f: &Font, w: f64, r: &mut Rasterizer| {
        let at = located(f, &[("wght", w)]);
        let run = format!("{:?}", f.shape("Office affinity, AVA Wave.", &at, &ShapeOptions::default()));
        let image = r.glyph(f, GlyphId(40), &at, 20.0, &RasterOptions::DEFAULT).expect("draws").to_bitmap().pixels;
        let instance = f.instance(&at).expect("instances").len();
        f.prewarm((30..60).map(GlyphId), &at);
        (run, outline_digest_at(f, &at, 80), image, instance)
    };
    let solo = open();
    let alone = [300.0, 700.0].map(|w| ask(&solo, w, &mut Rasterizer::new()));
    let pool = CachePool::new(64 << 10);
    let fonts: Vec<Font> = (0..4).map(|_| open()).collect();
    for f in &fonts {
        CACHES.iter().for_each(|&c| f.set_cache_pool(c, Some(&pool)));
        f.set_cache_budget(Cache::Raster, 1 << 20);
    }
    let done = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for t in 0..8 {
            let (fonts, alone, done, ask) = (&fonts, &alone, &done, &ask);
            s.spawn(move || {
                let mut r = Rasterizer::new();
                for round in 0..8 {
                    let (f, k) = (&fonts[(t + round) % fonts.len()], round % 2);
                    assert!(ask(f, [300.0, 700.0][k], &mut r) == alone[k], "thread {t} answered round {round} apart");
                }
                done.fetch_add(1, Relaxed);
            });
        }
        let (fonts, pool, done) = (&fonts, &pool, &done);
        s.spawn(move || {
            for n in 0.. {
                if done.load(Relaxed) == 8 {
                    break;
                }
                pool.set_budget([0, 16 << 10, 1 << 20][n % 3]);
                std::hint::black_box(pool.stats());
                let f = &fonts[n % fonts.len()];
                f.set_cache_pool(CACHES[n / fonts.len() % 4], (n % 3 != 0).then_some(pool));
            }
        });
    });
    for f in &fonts {
        CACHES.iter().for_each(|&c| f.set_cache_pool(c, Some(&pool)));
    }
    pool.set_budget(64 << 10);
    let entries: usize = fonts.iter().flat_map(|f| CACHES.map(|c| f.cache_stats(c).entries)).sum();
    let past = |f: &Font| f.cache_stats(Cache::Instance).bytes;
    let held = pool.stats();
    assert_eq!(held.entries, entries, "the pool counted otherwise than its fonts hold");
    assert!(held.bytes <= (64 << 10) + fonts.iter().map(past).sum::<usize>(), "{held:?} past the budget");
}

// Each thread starts on the same cold font, so every table built on first use is raced for.
#[test]
fn concurrent_shaping_agrees_with_one_thread() {
    type Case = (&'static str, &'static str, &'static [(&'static str, f64)], bool);
    let cases: &[Case] = &[
        ("inter/InterVariable.ttf", "Office affinity, AVA Wave.", &[], false),
        ("inter/InterVariable.ttf", "Office affinity, AVA Wave.", &[("wght", 700.0)], false),
        ("inter/InterVariable.ttf", "Vertical Inter", &[("wght", 300.0)], true),
        ("source-han-sans/SourceHanSansJP-VF.otf", "日本語の縦書き、「テスト」。", &[], true),
        ("source-han-sans/SourceHanSansJP-VF.otf", "日本語の縦書き、「テスト」。", &[("wght", 700.0)], true),
        ("scheherazade-new/ScheherazadeNew-Regular.ttf", "بِسْمِ ٱللَّهِ ٱلرَّحْمَٰنِ", &[], false),
        ("eb-garamond/EBGaramond.ttf", "Fiery ffl ligatures, Th.", &[], false),
    ];
    let open = || -> Vec<Font> {
        let read = |name: &str| std::fs::read(format!("{}/{name}", crate::FONTS)).expect("fixture");
        cases.iter().map(|(name, ..)| Font::from_bytes(&read(name)).expect("parses")).collect()
    };
    let shape_all = |fonts: &[Font], first: usize| -> Vec<String> {
        let mut out = vec![String::new(); cases.len()];
        for k in (0..cases.len()).map(|k| (k + first) % cases.len()) {
            let (_, text, axes, vertical) = cases[k];
            let o = ShapeOptions::default();
            let o = if vertical { o.with_direction(Direction::TopToBottom) } else { o };
            out[k] = format!("{:?}", fonts[k].shape(text, &located(&fonts[k], axes), &o));
        }
        out
    };
    let solo = open();
    let alone = shape_all(&solo, 0);
    assert!(alone.iter().all(|r| !r.contains("glyphs: []") && r.contains("complete: true")), "a case shaped nothing");

    let shared = Arc::new(open());
    let runs: Vec<Vec<String>> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..8)
            .map(|t| {
                let fonts = Arc::clone(&shared);
                s.spawn(move || shape_all(&fonts, t))
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("thread panicked")).collect()
    });

    for (t, run) in runs.iter().enumerate() {
        for (k, (got, want)) in run.iter().zip(&alone).enumerate() {
            assert_eq!(got, want, "thread {t} shaped case {k} differently than a single thread did");
        }
    }
    // Racing threads build each index once between them, so each is drawn from the allowance once.
    for (k, (raced, alone)) in shared.iter().zip(&solo).enumerate() {
        assert_eq!(raced.index_allowance(), alone.index_allowance(), "case {k}'s allowance");
    }
}

// The only test of the autohinter under contention: its zones and contexts are shared per font.
#[test]
fn concurrent_hinting_agrees_with_one_thread() {
    let gids: Vec<u32> = (1..40).collect();
    let opts = daegun::outline::OutlineOptions::default().with_hinting(HintMode::AutoForce);
    let draw = |f: &Font, g: u32| {
        let mut p = daegun::outline::Path::default();
        f.prepared_outline(GlyphId(g), &Location::default(), 24.0, &opts, &mut p).ok().map(|_| p)
    };
    let solo = self::font();
    let alone: Vec<Option<daegun::outline::Path>> = gids.iter().map(|&g| draw(&solo, g)).collect();
    assert!(alone.iter().flatten().count() > 30, "too few glyphs drew to mean anything");

    let shared = Arc::new(self::font());
    std::thread::scope(|s| {
        for _ in 0..8 {
            let f = Arc::clone(&shared);
            let (gids, want) = (gids.clone(), alone.clone());
            s.spawn(move || {
                for (i, &g) in gids.iter().enumerate() {
                    assert_eq!(draw(&f, g), want[i], "gid {g} hinted differently under contention");
                }
            });
        }
    });
}

#[test]
#[ignore]
fn threaded_outline_throughput() {
    let font = Arc::new(font());
    let n = font.glyph_count();

    for threads in [1usize, 2, 4, 8] {
        let t = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..threads {
                let font = Arc::clone(&font);
                s.spawn(move || {
                    let mut pen = daegun::outline::Path::default();
                    for gid in 0..n {
                        let _ = font.outline(GlyphId(gid), &Location::default(), &mut pen);
                    }
                    core::hint::black_box(&pen);
                });
            }
        });
        eprintln!("  {threads} threads x {n} glyphs: {:?}", t.elapsed());
    }
}

// Hinted by threads at once, each switching font, location, size and mode at every glyph, each glyph comes out as a
// fresh thread hinting nothing else gives it: what a thread keeps to hint with must never answer for another of them.
#[test]
fn threads_hint_as_a_thread_on_one_case_does() {
    use daegun::outline::{HintMode, HintedOutline};
    use daegun::{Font, GlyphId, Location};
    type Case<'a> = (&'a Font, &'a Location, f32, HintMode);
    let read = |rel: &str| std::fs::read(format!("{}/{rel}", crate::FONTS)).expect("the font reads");
    let inter = Font::from_bytes(&read("inter/InterVariable.ttf")).expect("Inter opens");
    let collection = |rel| Font::from_collection(&read(rel), 0).expect("Irianis opens");
    let irianis = collection("Irianisadfstd/IrianisadfstdRegular.ttc");
    let irianis_bold = collection("Irianisadfstd/IrianisadfstdBold.ttc");
    let (origin, bold) = (Location::default(), located(&inter, &[("wght", 700.0)]));
    let fonts: [(&Font, &Location, HintMode); 5] = [
        (&inter, &origin, HintMode::AutoForce),
        (&inter, &bold, HintMode::AutoForce),
        (&irianis, &origin, HintMode::AutoForce),
        (&irianis, &origin, HintMode::Font),
        (&irianis_bold, &origin, HintMode::Font),
    ];
    // Sizes outermost, so a font and the next lie side by side at one size and mode, which alone tells them apart.
    let cases: Vec<Case<'_>> =
        [12.0, 16.0].into_iter().flat_map(|px| fonts.map(|(f, at, mode)| (f, at, px, mode))).collect();
    let glyphs = || (36..76).map(GlyphId);
    let hint = |case: &Case<'_>, gid| case.0.hinted_outline(gid, case.1, case.2, case.3).ok().flatten();
    let alone: Vec<Vec<Option<HintedOutline>>> = std::thread::scope(|s| {
        let runs: Vec<_> =
            cases.iter().map(|case| s.spawn(move || glyphs().map(|g| hint(case, g)).collect())).collect();
        runs.into_iter().map(|run| run.join().expect("the thread runs")).collect()
    });
    let (hinted, asked) = (alone.iter().flatten().flatten().count(), alone.iter().flatten().count());
    assert!(hinted * 4 >= asked * 3, "only {hinted} of {asked} glyphs hinted: too few to prove anything");
    for (at, case) in cases.iter().enumerate() {
        let other = &alone[(at + 1) % alone.len()];
        assert!(alone[at] != *other, "{:?} and the next case hint alike, so switching proves nothing", case.3);
    }
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                for (g, gid) in glyphs().enumerate() {
                    for (case, want) in cases.iter().zip(&alone) {
                        assert!(hint(case, gid) == want[g], "glyph {} hinted differently at {:?}", gid.0, case.3);
                    }
                }
            });
        }
    });
}

// Threads on one font against a font each, which no lock is behind: a row far over it queues on something. One font
// took 10× to 20× on these rows with a lock for each glyph or location, and no other test times threads.
#[test]
#[ignore]
fn one_font_scales() {
    use daegun::outline::{HintMode, OutlinePen};
    use daegun::shape::ShapeOptions;
    use daegun::{Font, GlyphId, Location, Tag};
    use std::hint::black_box;

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
    type Call<'a> = &'a (dyn Fn(&Font, &Location, usize, usize) + Sync);
    type Row<'a> = (&'a str, &'a [u8], Option<f64>, usize, Call<'a>);

    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    if threads < 4 {
        println!("{threads} threads are too few to tell");
        return;
    }
    // Ns a call: each thread's own location kept, as the documented way is, and warmed before the clock starts.
    let time = |fonts: &[Font], weight: Option<f64>, iters: usize, call: Call<'_>| -> f64 {
        let gate = std::sync::Barrier::new(threads);
        let (gate, each) = (&gate, 0..threads);
        let taken: Vec<f64> = std::thread::scope(|s| {
            let runs: Vec<_> = each
                .map(|t| {
                    s.spawn(move || {
                        let font = &fonts[t % fonts.len()];
                        let at = weight.map_or_else(Location::default, |w| font.location(&[(Tag::new(b"wght"), w)]));
                        (0..200).for_each(|i| call(font, &at, t, i));
                        gate.wait();
                        let start = std::time::Instant::now();
                        (0..iters).for_each(|i| call(font, &at, t, i + 200));
                        start.elapsed().as_nanos() as f64 / iters as f64
                    })
                })
                .collect();
            runs.into_iter().map(|run| run.join().expect("the thread runs")).collect()
        });
        taken.iter().sum::<f64>() / threads as f64
    };
    let read = |rel: &str| std::fs::read(format!("{}/{rel}", crate::FONTS)).expect("the font reads");
    let (inter, han, irianis) = (
        read("inter/InterVariable.ttf"),
        read("source-han-sans/SourceHanSansJP-VF.otf"),
        read("Irianisadfstd/IrianisadfstdRegular.ttc"),
    );
    let glyph = |i: usize| GlyphId(36 + (i % 400) as u32);
    let opts = ShapeOptions::default();
    let shape: Call<'_> = &|font, at, t, i| {
        black_box(font.shape(&format!("日本語の組版{t}{i}"), at, &opts));
    };
    // More texts than a thread's front holds, so each hit goes to the font's lists.
    let texts: Vec<String> = (0..500).map(|i| format!("日本語の組版{i}")).collect();
    let hits: Call<'_> = &|font, at, t, i| {
        black_box(font.shape(&texts[(i + t * 60) % texts.len()], at, &opts));
    };
    let outline: Call<'_> = &|font, at, _, i| {
        let mut pen = Count(0);
        black_box(font.outline(glyph(i), at, &mut pen).is_ok());
        black_box(pen.0);
    };
    let bytecode: Call<'_> = &|font, at, _, i| {
        black_box(font.hinted_outline(glyph(i), at, 16.0, HintMode::Font).ok());
    };
    let autohint: Call<'_> = &|font, at, _, i| {
        black_box(font.hinted_outline(glyph(i), at, 16.0, HintMode::AutoForce).ok());
    };
    let moving = |font: &Font, t: usize, i: usize| {
        let step = 0.5 + (i + t * 1000) as f64 * 0.618_033_988_749_894_9;
        font.location(&[(Tag::new(b"wght"), 100.0 + 800.0 * step.fract())])
    };
    let location: Call<'_> = &|font, _, t, i| {
        black_box(moving(font, t, i));
    };
    let advance: Call<'_> = &|font, _, t, i| {
        black_box(font.advance(glyph(i), &moving(font, t, i), daegun::Orientation::Horizontal));
    };
    let rows: [Row<'_>; 9] = [
        ("shape misses", &han, None, 20_000, shape),
        ("shape, cache off", &han, None, 20_000, shape),
        ("shape hits", &han, None, 100_000, hits),
        ("outline", &inter, None, 40_000, outline),
        ("outline at a location", &inter, Some(560.0), 40_000, outline),
        ("a new location", &inter, None, 40_000, location),
        ("an advance there", &inter, None, 8_000, advance),
        ("bytecode", &irianis, None, 2_000, bytecode),
        ("autohinter", &inter, None, 4_000, autohint),
    ];
    let mut said = Vec::new();
    for (name, bytes, weight, iters, call) in rows {
        let open = |n: usize| -> Vec<Font> {
            let one = || {
                let font = Font::from_bytes(bytes).or_else(|_| Font::from_collection(bytes, 0));
                let font = font.expect("the font opens");
                if name == "shape, cache off" {
                    font.set_cache_budget(daegun::Cache::Shape, 0);
                }
                font
            };
            (0..n).map(|_| one()).collect()
        };
        let best = |n: usize| (0..3).map(|_| time(&open(n), weight, iters, call)).fold(f64::MAX, f64::min);
        let (shared, own) = (best(1), best(threads));
        let at = format!("at {threads} threads");
        // A hit moves its list's lock between cores, where a font each keeps it on one: 3.3× here, 13× with one lock.
        // With the cache off nothing is shared, so the two meet: 1.0× here, 3.7× spinning on a list for nothing.
        let limit = match name {
            "shape hits" => 6.0,
            "shape, cache off" => 1.5,
            _ => 4.0,
        };
        assert!(shared <= limit * own, "{name}: {shared:.0} ns on one font, {own:.0} ns on a font each {at}");
        said.push(format!("{name} {:.1}× ({shared:.0} against {own:.0} ns)", shared / own));
    }
    println!("one font at {threads} threads against a font each: {}", said.join(", "));
}
