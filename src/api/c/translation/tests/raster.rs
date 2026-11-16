use alloc::vec::Vec;

use super::*;
use crate::ffi::outline::HINT_AUTO_FORCE;
use crate::ffi::testing::{CFont, asset};
use crate::outline::{HintMode, OutlineOptions, OutlinePen};

#[test]
fn every_stripe_order_has_a_code() {
    for v in [StripeOrder::Rgb, StripeOrder::Bgr] {
        let code = match v {
            StripeOrder::Rgb => STRIPES_RGB,
            StripeOrder::Bgr => STRIPES_BGR,
        };
        assert_eq!(order(code), Ok(v));
    }
    assert!(order(2).is_err());
}

// The image C lent, as Rust's reads.
fn lent(image: &ImageC) -> (i32, i32, u32, u32, usize, u8, Vec<u8>) {
    let pixels = unsafe { core::slice::from_raw_parts(image.pixels, image.pixels_len) };
    (image.left, image.top, image.width, image.height, image.stride, image.channels, pixels.to_vec())
}

fn rust(image: Image<'_>) -> (i32, i32, u32, u32, usize, u8, Vec<u8>) {
    (image.left, image.top, image.width, image.height, image.stride, image.channels, image.pixels.to_vec())
}

#[test]
fn every_gamma_has_a_code() {
    let mut opts = RasterOptionsC::defaults();
    for (code, known) in [(GAMMA_LINEAR, true), (GAMMA_SRGB, true), (2, false)] {
        opts.gamma = code;
        assert_eq!(unsafe { raster_options(&opts) }.is_ok(), known, "gamma {code}");
    }
}

// What C lends is the image Rust draws: plain, hinted, through a layout, as sRGB alpha, darkened and moved; each
// field; a path by either rule; each color glyph.
#[test]
fn an_image_lent_through_c_is_rust_s() {
    let c = CFont::of(asset("inter/InterVariable.ttf"));
    let (font, at, here) = (unsafe { &*c.handle }, core::ptr::null(), crate::Location::default());
    let g = font.glyph_id('g').expect("Inter has g");
    let (cr, mut r) = (daegun_rasterizer_new(), Rasterizer::new());
    // Miri reaches each call at a few pixels and one color glyph; the sizes and glyphs that cover more run without it.
    let (px, field_px) = if cfg!(miri) { (5.0, 6.0) } else { (14.0, 24.0) };
    let rgb = SubpixelLayout::horizontal(StripeOrder::Rgb);
    let text = Rgba::opaque(30, 30, 30);
    let srgb = Gamma::Srgb { text, contrast: 0.5 };
    let (pixels, base) = (core::ptr::null(), RasterOptionsC::defaults());
    let mut out = ImageC { left: 0, top: 0, width: 0, height: 0, stride: 0, pixels, pixels_len: 0, channels: 0 };
    let mut hinted = OutlineOptionsC::defaults();
    hinted.hinting = HINT_AUTO_FORCE;
    let force = OutlineOptions::DEFAULT.with_hinting(HintMode::AutoForce);
    let o = RasterOptions::DEFAULT;
    let sets = [
        (base, o),
        (RasterOptionsC { outline: &hinted, ..base }, o.with_outline(force)),
        (RasterOptionsC { layout: &rgb, ..base }, o.with_layout(&rgb)),
        (RasterOptionsC { gamma: GAMMA_SRGB, text, contrast: 0.5, ..base }, o.with_gamma(srgb)),
        // darken is read as a byte, so any but 0 darkens, not only a bool's 1.
        (RasterOptionsC { darken: 2, offset: [0.25, 0.5], ..base }, o.with_darken(true).with_offset(0.25, 0.5)),
    ];
    for (i, (opts, options)) in sets.iter().enumerate() {
        assert_eq!(unsafe { daegun_rasterizer_glyph(cr, c.handle, g.0, at, px, opts, &mut out) }, Status::Ok);
        assert_eq!(lent(&out), rust(r.glyph(font, g, &here, px, options).expect("draws")), "set {i}");
    }
    for (code, field) in [(FIELD_SDF, Field::Sdf { spread: 2.0 }), (FIELD_MSDF, Field::Msdf { spread: 2.0 })] {
        let st = unsafe { daegun_rasterizer_distance_field(cr, c.handle, g.0, at, field_px, code, 2.0, &mut out) };
        assert_eq!(st, Status::Ok);
        assert_eq!(lent(&out), rust(r.distance_field(font, g, &here, field_px, field).expect("draws")));
    }
    let unknown = unsafe { daegun_rasterizer_distance_field(cr, c.handle, g.0, at, field_px, 2, 2.0, &mut out) };
    assert_eq!(unknown, Status::InvalidArgument);
    // A box inside a box, both one way round, which the rules fill apart.
    let mut path = Path::new();
    for [x0, y0, x1, y1] in [[1.0, 1.0, 9.0, 7.0], [3.0, 3.0, 7.0, 5.0]] {
        OutlinePen::move_to(&mut path, x0, y0);
        for (x, y) in [(x1, y0), (x1, y1), (x0, y1)] {
            OutlinePen::line_to(&mut path, x, y);
        }
        OutlinePen::close(&mut path);
    }
    let m = [1.5, 0.0, 0.0, 1.5, 0.25, 0.0];
    let mut fills = Vec::new();
    for (code, rule) in [(FILL_NONZERO, FillRule::NonZero), (FILL_EVENODD, FillRule::EvenOdd)] {
        assert_eq!(unsafe { daegun_rasterizer_path(cr, &path, code, m.as_ptr(), &mut out) }, Status::Ok);
        assert_eq!(lent(&out), rust(r.path(&path, rule, Some(&m)).expect("draws")));
        fills.push(lent(&out));
    }
    assert_ne!(fills[0], fills[1], "the rules filled the inner box alike");
    assert_eq!(unsafe { daegun_rasterizer_path(cr, &path, 2, core::ptr::null(), &mut out) }, Status::InvalidArgument);
    let mut scene = SceneOptionsC::defaults();
    (scene.palette, scene.foreground) = (1, Rgba::opaque(200, 40, 0));
    let options = SceneOptions { palette: 1, foreground: scene.foreground };
    let none = unsafe { daegun_rasterizer_color_glyph(cr, c.handle, g.0, at, 20.0, &base, &scene, &mut out) };
    assert_eq!(none, Status::Absent, "a glyph without a scene");
    let colr = CFont::of(asset("colr-v1-test-glyphs/test_glyphs.ttf"));
    let (colr_font, mut painted) = (unsafe { &*colr.handle }, 0);
    for gid in 1..colr_font.glyph_count() {
        let st = unsafe { daegun_rasterizer_color_glyph(cr, colr.handle, gid, at, field_px, &base, &scene, &mut out) };
        let drawn = r.color_glyph(colr_font, GlyphId(gid), &here, field_px, &o, &options).expect("draws");
        let Some(image) = drawn else {
            assert_eq!(st, Status::Absent, "glyph {gid}");
            continue;
        };
        assert_eq!(st, Status::Ok, "glyph {gid}");
        assert_eq!(lent(&out), rust(image), "glyph {gid}");
        painted += 1;
        if cfg!(miri) {
            break;
        }
    }
    assert!(painted > 0, "no color glyph drew");
    unsafe { daegun_rasterizer_free(cr) };
}
