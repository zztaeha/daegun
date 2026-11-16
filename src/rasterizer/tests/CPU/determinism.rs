// One hash over every kind of image the rasterizer draws, pinned: every CPU, with std or without, draws these same
// bytes. The gate runs it on x86_64 Linux and Windows besides the host, and the 32-bit run on i686.
use std::vec::Vec;

use super::coverage::{FONTS, font, sample};
use crate::color::{Rgba, SceneOptions};
use crate::daecore::daetype::outline::{FillRule, OutlinePen, Path};
use crate::outline::{HintMode, OutlineOptions, StrokeStyle};
use crate::raster::{Field, Gamma, Image, RasterOptions, Rasterizer, StripeOrder, SubpixelLayout};
use crate::{Location, Tag};

const PINNED: u64 = 0xd534_e6d4_9ec6_1cc1;

struct Fnv(u64);

impl Fnv {
    fn bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn image(&mut self, image: &Image<'_>) {
        for v in [image.left, image.top] {
            self.bytes(&v.to_le_bytes());
        }
        for v in [image.width, image.height, u32::from(image.channels)] {
            self.bytes(&v.to_le_bytes());
        }
        for y in 0..image.height {
            self.bytes(image.row(y));
        }
    }
}

// Closed polygons and curves from a fixed seed, crossing themselves and each other.
fn paths() -> Vec<Path> {
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed % 4096) as f32 / 64.0
    };
    (0..24)
        .map(|i| {
            let mut path = Path::new();
            for _ in 0..1 + i % 3 {
                path.move_to(next(), next());
                for k in 0..3 + i % 5 {
                    match k % 3 {
                        0 => path.line_to(next(), next()),
                        1 => path.quad_to(next(), next(), next(), next()),
                        _ => path.curve_to(next(), next(), next(), next(), next(), next()),
                    }
                }
                path.close();
            }
            path
        })
        .collect()
}

#[test]
fn every_build_draws_the_same_bytes() {
    let mut h = Fnv(0xcbf2_9ce4_8422_2325);
    let mut r = Rasterizer::new();
    let (rgb, bgr) = (SubpixelLayout::horizontal(StripeOrder::Rgb), SubpixelLayout::vertical(StripeOrder::Bgr));
    let srgb = Gamma::Srgb { text: Rgba::opaque(40, 40, 40), contrast: 0.5 };
    let outline = OutlineOptions::DEFAULT;
    let o = RasterOptions::DEFAULT;
    let sets = [
        (9.0, o),
        (14.0, o.with_offset(0.25, 0.625)),
        (31.0, o.with_layout(&rgb)),
        (12.0, o.with_layout(&bgr).with_offset(0.5, 0.0)),
        (16.0, o.with_gamma(srgb)),
        (11.0, o.with_darken(true)),
        (13.0, o.with_outline(outline.with_hinting(HintMode::Font))),
        (15.0, o.with_outline(outline.with_hinting(HintMode::AutoForce))),
        (20.0, o.with_outline(outline.with_stroke(StrokeStyle::new(30.0)))),
        (18.0, o.with_outline(outline.with_embolden(25.0).with_oblique(0.2))),
        (64.0, o.with_outline(outline.with_transform([0.9, 0.2, -0.3, 1.1, 2.5, -1.25]))),
    ];
    let mut drawn = 0;
    for name in FONTS {
        let font = font(name).expect("a pinned font is present");
        let heavy = font.location(&[(Tag::new(b"wght"), 900.0)]);
        for g in sample(&font, 16) {
            for (i, (px, options)) in sets.iter().enumerate() {
                let at = if i % 2 == 0 { &Location::default() } else { &heavy };
                h.image(&r.glyph(&font, g, at, *px, options).expect("draws"));
            }
            for field in [Field::Sdf { spread: 3.0 }, Field::Msdf { spread: 3.0 }] {
                h.image(&r.distance_field(&font, g, &Location::default(), 24.0, field).expect("draws"));
            }
            drawn += sets.len() + 2;
        }
    }
    let scene = SceneOptions { palette: 1, foreground: Rgba::opaque(200, 40, 0) };
    for name in ["colr-v1-test-glyphs/test_glyphs.ttf", "bungee-tint/BungeeTint-Regular.ttf"] {
        let font = font(name).expect("a pinned font is present");
        for g in sample(&font, 64) {
            let options = o.with_offset(0.375, 0.125);
            if let Some(image) = r.color_glyph(&font, g, &Location::default(), 28.0, &options, &scene).expect("draws") {
                h.image(&image);
                drawn += 1;
            }
        }
    }
    for (i, path) in paths().iter().enumerate() {
        let rule = if i % 2 == 0 { FillRule::NonZero } else { FillRule::EvenOdd };
        let m = [1.25, 0.125 * i as f64, -0.25, 0.75, 0.3, 0.7];
        h.image(&r.path(path, rule, if i % 3 == 0 { None } else { Some(&m) }).expect("draws"));
        drawn += 1;
    }
    assert_eq!(drawn, 1_986, "images drawn");
    assert_eq!(h.0, PINNED, "the bytes drawn moved: {:#018x} over {drawn} images", h.0);
}
