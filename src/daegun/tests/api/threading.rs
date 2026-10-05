use std::sync::Arc;
use std::time::Instant;

use daegun::{Font, HintMode};

fn font() -> Font {
    let path = format!("{}/eb-garamond/EBGaramond.ttf", crate::FONTS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).expect("parses")
}

fn outline_digest(font: &Font, gid_end: u16) -> u64 {
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
    impl daegun::daecore::daetype::outline::OutlinePen for Digest {
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
        let _ = font.outline_glyph(gid, &mut d);
    }
    d.0
}

#[test]
fn concurrent_outlining_agrees_with_one_thread() {
    let font = font();
    let n = font.num_glyphs().min(800);
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

// The only test of the autohinter under contention: its zones and contexts are shared per font.
#[test]
fn concurrent_hinting_agrees_with_one_thread() {
    let gids: Vec<u16> = (1..40).collect();
    let opts = daegun::OutlineOptions::default().with_hinting(HintMode::AutoForce);
    let draw = |f: &Font, g: u16| {
        let mut p = daegun::Path::default();
        f.prepared_outline(g, 24.0, &[], &opts, &mut p).map(|_| p)
    };
    let solo = self::font();
    let alone: Vec<Option<daegun::Path>> = gids.iter().map(|&g| draw(&solo, g)).collect();
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
    let n = font.num_glyphs();

    for threads in [1usize, 2, 4, 8] {
        let t = Instant::now();
        std::thread::scope(|s| {
            for _ in 0..threads {
                let font = Arc::clone(&font);
                s.spawn(move || {
                    let mut pen = daegun::daecore::daetype::outline::Path::default();
                    for gid in 0..n {
                        let _ = font.outline_glyph(gid, &mut pen);
                    }
                    core::hint::black_box(&pen);
                });
            }
        });
        eprintln!("  {threads} threads x {n} glyphs: {:?}", t.elapsed());
    }
}
