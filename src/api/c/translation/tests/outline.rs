use super::*;
use crate::ffi::testing::{CFont, asset, last_error};
use crate::{Location, Tag};

#[test]
fn every_hint_mode_cap_and_join_has_a_code() {
    for v in [HintMode::None, HintMode::Font, HintMode::Auto, HintMode::AutoForce] {
        let code = match v {
            HintMode::None => HINT_NONE,
            HintMode::Font => HINT_FONT,
            HintMode::Auto => HINT_AUTO,
            HintMode::AutoForce => HINT_AUTO_FORCE,
        };
        assert_eq!(hint_mode(code), Ok(v));
    }
    assert!([-1, 2, 5].iter().all(|&code| hint_mode(code).is_err()), "a code outside DAEGUN_HINT_* named a mode");
    let style = |cap, join| StrokeStyleC { width: 1.0, cap, join, miter_limit: 2.0 }.rust();
    for v in [Cap::Butt, Cap::Round, Cap::Square] {
        let code = match v {
            Cap::Butt => CAP_BUTT,
            Cap::Round => CAP_ROUND,
            Cap::Square => CAP_SQUARE,
        };
        assert_eq!(style(code, JOIN_ROUND).map(|s| s.cap), Ok(v));
    }
    for v in [Join::Miter { limit: 2.0 }, Join::Round, Join::Bevel] {
        let code = match v {
            Join::Miter { limit: _ } => JOIN_MITER,
            Join::Round => JOIN_ROUND,
            Join::Bevel => JOIN_BEVEL,
        };
        assert_eq!(style(CAP_BUTT, code).map(|s| s.join), Ok(v));
    }
    assert!(hint_mode(5).is_err() && style(3, JOIN_ROUND).is_err() && style(CAP_BUTT, 3).is_err());
}

// daegun_outline_options_default's struct is Rust's default, field for field, the pattern naming every field.
#[test]
fn the_default_outline_options_are_rusts() {
    let OutlineOptions { transform, hinting, stroke, embolden, oblique } =
        outline_options(&OutlineOptionsC::defaults()).unwrap();
    let d = OutlineOptions::default();
    assert_eq!((transform, hinting, stroke, embolden, oblique), (d.transform, d.hinting, d.stroke, d.embolden,
                                                                 d.oblique));
}

// Every STIX glyph's hint masks read through C as Rust gives them, each mask's point and bits, and none past the
// last.
#[test]
fn cff_hint_masks_reach_c_as_rust_gives_them() {
    let c = CFont::of(asset("stix-two-math/STIX2Math.otf"));
    let font = Font::from_bytes(&c.bytes).unwrap();
    let mut masked = 0;
    // Miri reads every 61st glyph, which reaches each call as every glyph does, in a minute rather than twelve.
    let stride = if cfg!(miri) { 61 } else { 1 };
    for gid in (0..font.glyph_count()).step_by(stride) {
        let Some(want) = font.cff_hints(GlyphId(gid), &crate::Location::default()) else { continue };
        let (mut hints, mut count) = (core::ptr::null_mut(), 0);
        unsafe {
            assert_eq!(daegun_font_cff_hints(c.handle, gid, core::ptr::null(), &mut hints), Status::Ok);
            assert_eq!(daegun_cff_hints_mask_count(hints, &mut count), Status::Ok);
            assert_eq!(count, want.masks.len(), "glyph {gid}");
            let mut mask = CffMaskC { point: 0, bits: core::ptr::null(), bits_len: 0 };
            for (i, crate::outline::CffMask { point, bits }) in want.masks.iter().enumerate() {
                assert_eq!(daegun_cff_hints_mask_at(hints, i, &mut mask), Status::Ok);
                let got = core::slice::from_raw_parts(mask.bits, mask.bits_len);
                assert_eq!((mask.point, got), (*point, &bits[..]), "glyph {gid} mask {i}");
            }
            assert_eq!(daegun_cff_hints_mask_at(hints, count, &mut mask), Status::InvalidArgument);
            daegun_cff_hints_free(hints);
        }
        masked += usize::from(count > 0);
    }
    assert!(masked * stride > 1_000, "{masked} glyphs with masks");
}

// EB Garamond with glyph a's first two end points swapped, so it no longer decodes: MALFORMED with the decoder's
// reason, not an absence.
#[test]
fn a_glyph_that_does_not_decode_is_malformed_not_absent() {
    let bytes = asset("eb-garamond/EBGaramond.ttf");
    let font = Font::from_bytes(&bytes).unwrap();
    let a = font.glyph_id('a').unwrap().0;
    let mut tables: alloc::collections::BTreeMap<Tag, Vec<u8>> =
        font.table_tags().map(|t| (t, font.table(t).unwrap().to_vec())).collect();
    let format = crate::raw::bytes::read_i16_be(&tables[&Tag::new(b"head")], 50).unwrap();
    let loca = crate::raw::parse_loca(&tables[&Tag::new(b"loca")], format, font.glyph_count() as usize);
    let at = loca[a as usize];
    let glyf = tables.get_mut(&Tag::new(b"glyf")).unwrap();
    assert!(glyf[at + 1] >= 2, "a has two contours to swap");
    glyf.swap(at + 10, at + 12);
    glyf.swap(at + 11, at + 13);
    let broken = CFont::of(crate::raw::build_font(&tables).unwrap());
    let user = core::ptr::null_mut();
    let pen = Pen { move_to: None, line_to: None, quad_to: None, curve_to: None, close: None, user };
    assert_eq!(unsafe { daegun_font_outline(broken.handle, a, core::ptr::null(), &pen) }, Status::Malformed);
    assert!(last_error().contains("end points"), "the reason is {:?}", last_error());

    // A glyph past the count answers as Rust's call fails.
    let rust = unsafe { &*broken.handle }.outline(GlyphId(65_535), &Location::default(), &mut PenBridge(pen));
    let c = unsafe { daegun_font_outline(broken.handle, 65_535, core::ptr::null(), &pen) };
    assert_eq!(c, rust.map_or_else(failed, |()| Status::Ok));
}

// A freed path is the thread's next, empty however much was drawn into it.
#[test]
fn a_freed_path_is_the_next_one() {
    let first = daegun_path_new();
    unsafe {
        assert_eq!(daegun_path_move_to(first, 1.0, 2.0), Status::Ok);
        assert_eq!(daegun_path_line_to(first, 3.0, 4.0), Status::Ok);
        daegun_path_free(first);
    }
    let second = daegun_path_new();
    assert!(second == first, "the path freed last was not reused");
    let (mut empty, mut verbs) = (false, 1);
    unsafe {
        assert_eq!(daegun_path_is_empty(second, &mut empty), Status::Ok);
        let _ = daegun_path_verbs(second, &mut verbs);
        daegun_path_free(second);
    }
    assert!(empty && verbs == 0, "a reused path kept what was drawn into it");
}

// A freed hinted outline is the thread's next, its points made again for the glyph it then holds.
#[test]
fn a_freed_hinted_outline_is_the_next_one() {
    let font = CFont::of(asset("inter/InterVariable.ttf"));
    let rust = unsafe { &*font.handle };
    let hinted = |c| unsafe {
        let mut h = core::ptr::null_mut();
        let g = rust.glyph_id(c).unwrap().0;
        let st = daegun_font_hinted_outline(font.handle, g, core::ptr::null(), 16.0, HINT_AUTO_FORCE, &mut h);
        assert_eq!(st, Status::Ok);
        h
    };
    let points = |h, c| unsafe {
        let (mut n, mut k) = (0, 0);
        let kinds = crate::ffi::testing::lent(daegun_hinted_outline_kinds(h, &mut k), k).to_vec();
        let points = crate::ffi::testing::lent(daegun_hinted_outline_points(h, &mut n), n).to_vec();
        let outline = rust.hinted_outline(rust.glyph_id(c).unwrap(), &Location::default(), 16.0, HintMode::AutoForce);
        let (want, want_kinds): (Vec<Point>, Vec<u8>) =
            outline.unwrap().unwrap().points().map(|(p, kind)| (p, kind as u8)).unzip();
        assert_eq!((points, kinds), (want, want_kinds), "{c}'s points");
    };
    let first = hinted('g');
    points(first, 'g');
    unsafe { daegun_hinted_outline_free(first) };
    let second = hinted('o');
    assert!(second == first, "the outline freed last was not reused");
    points(second, 'o');
    unsafe { daegun_hinted_outline_free(second) };
}

fn bounds_bits(b: Option<Bounds>) -> Option<[u64; 4]> {
    b.map(|b| [b.x_min, b.y_min, b.x_max, b.y_max].map(f64::to_bits))
}

fn quad_bits(quads: &[Quad]) -> Vec<[u32; 6]> {
    let bits = |q: &Quad| [q.start.x, q.start.y, q.control.x, q.control.y, q.end.x, q.end.y].map(f32::to_bits);
    quads.iter().map(bits).collect()
}

unsafe fn take_quads(quads: *mut Quads) -> Option<Vec<[u32; 6]>> {
    if quads.is_null() {
        return None;
    }
    let mut n = 0;
    let q = quad_bits(unsafe { crate::ffi::testing::lent(daegun_quads_data(quads, &mut n), n) });
    unsafe { daegun_quads_free(quads) };
    Some(q)
}

unsafe fn take_contours(contours: *mut Contours) -> Option<Vec<Vec<(u32, u32)>>> {
    use crate::ffi::testing::{bits32, lent};
    if contours.is_null() {
        return None;
    }
    let mut count = 0;
    let _ = unsafe { daegun_contours_count(contours, &mut count) };
    let all = (0..count)
        .map(|i| {
            let mut n = 0;
            let points = unsafe { lent(daegun_contours_points(contours, i, &mut n), n) };
            points.iter().map(|p| (bits32(p.x), bits32(p.y))).collect()
        })
        .collect();
    unsafe { daegun_contours_free(contours) };
    Some(all)
}

fn contour_bits(contours: &[Vec<Point>]) -> Vec<Vec<(u32, u32)>> {
    use crate::ffi::testing::bits32;
    contours.iter().map(|c| c.iter().map(|p| (bits32(p.x), bits32(p.y))).collect()).collect()
}

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    use crate::ffi::testing::{
        Diffs, bits32, c_path, c_pen, drawn, each_font, found, glyph_list, lent, rust_path, status,
    };
    use core::ptr::null_mut;
    const TRANSFORM: [f64; 6] = [0.5, 0.1, -0.2, 1.5, 10.0, -20.0];
    let c_style = StrokeStyleC { width: 20.0, cap: CAP_ROUND, join: JOIN_MITER, miter_limit: 4.0 };
    let style = StrokeStyle { width: 20.0, join: Join::Miter { limit: 4.0 }, cap: Cap::Round };
    let modes = [
        (HINT_NONE, HintMode::None),
        (HINT_FONT, HintMode::Font),
        (HINT_AUTO, HintMode::Auto),
        (HINT_AUTO_FORCE, HintMode::AutoForce),
    ];
    // Each set of prepared-outline options as C fills it and as Rust builds it.
    let base = OutlineOptionsC::defaults();
    let c_opts = [
        base,
        OutlineOptionsC { hinting: HINT_AUTO, ..base },
        OutlineOptionsC {
            hinting: HINT_FONT,
            has_transform: 1,
            transform: [1.0, 0.0, 0.25, 1.0, 3.0, -2.0],
            ..base
        },
        OutlineOptionsC {
            has_stroke: 1,
            stroke: StrokeStyleC { width: 30.0, cap: CAP_SQUARE, join: JOIN_BEVEL, miter_limit: 0.0 },
            ..base
        },
        OutlineOptionsC {
            hinting: HINT_FONT,
            has_embolden: 1,
            embolden: 20.0,
            has_oblique: 1,
            oblique: 0.2,
            ..base
        },
        OutlineOptionsC { has_embolden: 1, embolden: f32::NAN, ..base },
    ];
    let o = OutlineOptions::default();
    let rust_opts = [
        o,
        o.with_hinting(HintMode::Auto),
        o.with_hinting(HintMode::Font).with_transform([1.0, 0.0, 0.25, 1.0, 3.0, -2.0]),
        o.with_stroke(StrokeStyle { width: 30.0, join: Join::Bevel, cap: Cap::Square }),
        o.with_hinting(HintMode::Font).with_embolden(20.0).with_oblique(0.2),
        o.with_embolden(f32::NAN),
    ];
    let mut d = Diffs::default();
    each_font(&mut d, |f, d| unsafe {
        let gids = glyph_list(&f.rust);
        let count = f.rust.glyph_count();
        // The costlier checks walk only the first glyphs and the last.
        let few: Vec<u32> = gids.iter().copied().filter(|&g| g < 24 || g.saturating_add(4) >= count).collect();
        let max_area = daegun_max_area_for(16.0, f32::from(f.rust.units_per_em()));
        d.eq(
            &f.name,
            "max_area_for",
            bits32(max_area),
            bits32(crate::outline::max_area_for(16.0, f32::from(f.rust.units_per_em()))),
        );
        for &g in &gids {
            let mut flagged = false;
            let st = daegun_font_has_overlap_flag(f.c, g, &mut flagged);
            d.eq(&f.name, "has_overlap_flag", (g, st, flagged), (g, Status::Ok, f.rust.has_overlap_flag(GlyphId(g))));
        }
        for place in f.places().into_iter().filter(|p| p.whole) {
            let at = &alloc::format!("{} @{}", f.name, place.label);
            for &g in &gids {
                let (path, pen) = c_pen();
                let st = daegun_font_outline(f.c, g, place.c, &pen);
                let mut rust = Path::default();
                let r = f.rust.outline(GlyphId(g), &place.rust, &mut rust);
                d.eq(at, "outline", (g, st, c_path(path)), (g, status(&r), rust_path(&rust)));

                let (mut empty, mut cost, mut b) = (false, 0, Bounds::default());
                let _ = (daegun_path_is_empty(path, &mut empty), daegun_path_cost(path, &mut cost));
                let bst = daegun_path_bounds(path, &mut b);
                let c_bounds = (bst == Status::Ok).then_some(b);
                let got = (g, empty, cost, bst, bounds_bits(c_bounds));
                let want = (g, rust.is_empty(), rust.cost(), found(&rust.bounds()), bounds_bits(rust.bounds()));
                d.eq(at, "path is_empty, cost and bounds", got, want);

                let replayed = drawn(|pen| daegun_path_replay(path, TRANSFORM.as_ptr(), pen));
                let mut r = Path::default();
                rust.replay(Some(&TRANSFORM), &mut r);
                d.eq(at, "path_replay", (g, replayed), (g, (Status::Ok, rust_path(&r))));

                let mut quads = null_mut();
                let st = daegun_path_quads(path, 1.0, &mut quads);
                let _ = daegun_quads_normalize_winding(quads);
                let mut pen = QuadraticPen::new(1.0);
                rust.replay(None, &mut pen);
                let r = pen.finish().map(|mut q| {
                    crate::outline::normalize_winding(&mut q);
                    quad_bits(&q)
                });
                d.eq(at, "path_quads", (g, st, take_quads(quads)), (g, status(&r), r.ok()));

                let mut quads = null_mut();
                let st = daegun_font_quads(f.c, g, place.c, &mut quads);
                let r = f.rust.quads(GlyphId(g), &place.rust).map(|q| quad_bits(&q));
                d.eq(at, "font_quads", (g, st, take_quads(quads)), (g, status(&r), r.ok()));

                if few.contains(&g) {
                    for simplified in [false, true] {
                        let c = drawn(|pen| match simplified {
                            false => daegun_path_stroke(path, &c_style, 0.25, pen),
                            true => daegun_path_stroke_simplified(path, &c_style, 0.25, pen),
                        });
                        let mut r = Path::default();
                        let st = match simplified {
                            false => rust.stroke(&style, 0.25, &mut r),
                            true => rust.stroke_simplified(&style, 0.25, &mut r),
                        };
                        d.eq(at, "path_stroke", (g, simplified, c), (g, simplified, (status(&st), rust_path(&r))));
                    }
                    let mut contours = null_mut();
                    let st = daegun_path_flatten(path, max_area, &mut contours);
                    let r = rust.flatten(max_area);
                    let mut resolved = null_mut();
                    let rst = daegun_contours_resolve_overlaps(contours, &mut resolved);
                    let c = (g, st, take_contours(contours), rst, take_contours(resolved));
                    let rr = r.as_ref().ok().map(|r| crate::outline::resolve_overlaps(r));
                    let rust_resolved = (
                        match &rr {
                            Some(o) => found(o),
                            None => Status::Null,
                        },
                        rr.flatten().map(|r| contour_bits(&r)),
                    );
                    let want =
                        (g, status(&r), r.as_ref().ok().map(|r| contour_bits(r)), rust_resolved.0, rust_resolved.1);
                    d.eq(at, "path_flatten, resolve_overlaps", c, want);

                    for (code, mode) in modes {
                        for px in [9.0, 16.0, 33.5] {
                            let mut h = null_mut();
                            let st = daegun_font_hinted_outline(f.c, g, place.c, px, code, &mut h);
                            let got = (!h.is_null()).then(|| {
                                let (mut n, mut k, mut m) = (0, 0, 0);
                                let points = lent(daegun_hinted_outline_points(h, &mut n), n);
                                let kinds = lent(daegun_hinted_outline_kinds(h, &mut k), k).to_vec();
                                let contours = lent(daegun_hinted_outline_contours(h, &mut m), m).to_vec();
                                let points: Vec<(u32, u32)> =
                                    points.iter().map(|p| (bits32(p.x), bits32(p.y))).collect();
                                let drawing = drawn(|pen| daegun_hinted_outline_draw(h, pen)).1;
                                (points, kinds, contours, drawing)
                            });
                            daegun_hinted_outline_free(h);
                            let r = f.rust.hinted_outline(GlyphId(g), &place.rust, px, mode);
                            let hinted_status = match &r {
                                Ok(h) => found(h),
                                Err(_) => status(&r),
                            };
                            let want = r.ok().flatten().map(|h| {
                                let (points, kinds): (Vec<(u32, u32)>, Vec<u8>) =
                                    h.points().map(|(p, k)| ((bits32(p.x), bits32(p.y)), k as u8)).unzip();
                                let mut drawing = Path::default();
                                h.draw(&mut drawing);
                                (points, kinds, h.contours().to_vec(), rust_path(&drawing))
                            });
                            d.eq(
                                at,
                                "hinted_outline",
                                (g, code, bits32(px), st, got),
                                (g, code, bits32(px), hinted_status, want),
                            );
                        }
                    }
                    for (i, (c, r)) in c_opts.iter().zip(&rust_opts).enumerate() {
                        for px in [12.0, 40.0] {
                            let mut glyph = core::mem::MaybeUninit::<PreparedGlyph>::zeroed();
                            let (st, drawing) = drawn(|pen| {
                                daegun_font_prepared_outline(f.c, g, place.c, px, c, pen, glyph.as_mut_ptr())
                            });
                            let glyph = (st == Status::Ok).then(|| glyph.assume_init());
                            let mut r_drawing = Path::default();
                            let rg = f.rust.prepared_outline(GlyphId(g), &place.rust, px, r, &mut r_drawing);
                            let sig = |p: PreparedGlyph| (bits32(p.advance_width), bits32(p.advance_height), p.hinted);
                            let got = (g, i, st, glyph.map(sig), drawing);
                            d.eq(
                                at,
                                "prepared_outline",
                                got,
                                (g, i, status(&rg), rg.ok().map(sig), rust_path(&r_drawing)),
                            );
                        }
                    }
                }
                daegun_path_free(path);
            }
        }
        for place in f.places().iter().filter(|p| p.whole) {
            for &g in &gids {
                let mut h = null_mut();
                let st = daegun_font_cff_hints(f.c, g, place.c, &mut h);
                let got = (!h.is_null()).then(|| {
                    let mut n = 0;
                    let stems: Vec<(u32, u32, bool)> = lent(daegun_cff_hints_stems(h, &mut n), n)
                        .iter()
                        .map(|s| (bits32(s.min), bits32(s.max), s.vertical))
                        .collect();
                    let mut count = 0;
                    let _ = daegun_cff_hints_mask_count(h, &mut count);
                    let masks: Vec<(usize, Vec<u8>)> = (0..count)
                        .map(|i| {
                            let mut m = core::mem::MaybeUninit::<CffMaskC>::zeroed();
                            let _ = daegun_cff_hints_mask_at(h, i, m.as_mut_ptr());
                            let m = m.assume_init();
                            (m.point, lent(m.bits, m.bits_len).to_vec())
                        })
                        .collect();
                    (stems, masks)
                });
                daegun_cff_hints_free(h);
                let r = f.rust.cff_hints(GlyphId(g), &place.rust);
                let want = r.as_ref().map(|h| {
                    let stems = h.stems.iter().map(|s| (bits32(s.min), bits32(s.max), s.vertical)).collect();
                    (stems, h.masks.iter().map(|m| (m.point, m.bits.clone())).collect())
                });
                d.eq(&alloc::format!("{} at {}", f.name, place.label), "cff_hints", (g, st, got), (g, found(&r), want));
            }
        }
    });
    d.done("outline");
}
