use super::*;

#[test]
fn every_kern_corner_has_a_code() {
    use MathKernCorner::*;
    for v in [TopRight, TopLeft, BottomRight, BottomLeft] {
        let code = match v {
            TopRight => MATH_KERN_TOP_RIGHT,
            TopLeft => MATH_KERN_TOP_LEFT,
            BottomRight => MATH_KERN_BOTTOM_RIGHT,
            BottomLeft => MATH_KERN_BOTTOM_LEFT,
        };
        assert_eq!(corner_of(code), Ok(v));
    }
    assert!(corner_of(4).is_err());
}

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    use crate::Orientation;
    use crate::ffi::testing::{Diffs, bits, each_font, found, glyph_list, lent, status};
    use core::mem::MaybeUninit;
    let corners = [
        (MATH_KERN_TOP_RIGHT, MathKernCorner::TopRight),
        (MATH_KERN_TOP_LEFT, MathKernCorner::TopLeft),
        (MATH_KERN_BOTTOM_RIGHT, MathKernCorner::BottomRight),
        (MATH_KERN_BOTTOM_LEFT, MathKernCorner::BottomLeft),
    ];
    let mut d = Diffs::default();
    each_font(&mut d, |f, d| unsafe {
        // A font without MATH answers none at any location, so the default alone is asked.
        let has_math = f.rust.has_table(crate::Tag::new(b"MATH"));
        for place in f.places().iter().filter(|p| has_math || p.label == "default") {
            let at = alloc::format!("{} at {}", f.name, place.label);
            let mut constants = MaybeUninit::<MathConstants>::zeroed();
            let st = daegun_font_math_constants(f.c, place.c, constants.as_mut_ptr());
            let got = (st == Status::Ok).then(|| alloc::format!("{:?}", constants.assume_init()));
            let r = f.rust.math_constants(&place.rust);
            d.eq(&at, "math_constants", (st, got), (found(&r), r.map(|c| alloc::format!("{c:?}"))));
            let mut overlap = 0.0;
            let st = daegun_font_math_min_connector_overlap(f.c, &mut overlap);
            let r = f.rust.math_min_connector_overlap();
            d.eq(
                &at,
                "math_min_connector_overlap",
                (st, (st == Status::Ok).then_some(bits(overlap))),
                (found(&r), r.map(bits)),
            );
            for g in glyph_list(&f.rust) {
                let glyph = GlyphId(g);
                let (mut italics, mut accent, mut extended) = (0.0, 0.0, false);
                let ist = daegun_font_math_italics_correction(f.c, g, place.c, &mut italics);
                let ast = daegun_font_math_top_accent_attachment(f.c, g, place.c, &mut accent);
                let est = daegun_font_math_is_extended_shape(f.c, g, &mut extended);
                let got = (g, ist, (ist == Status::Ok).then_some(bits(italics)), ast, bits(accent), est, extended);
                let r = f.rust.math_italics_correction(glyph, &place.rust);
                let want = (
                    g,
                    found(&r),
                    r.map(bits),
                    Status::Ok,
                    bits(f.rust.math_top_accent_attachment(glyph, &place.rust)),
                    Status::Ok,
                    f.rust.math_is_extended_shape(glyph),
                );
                d.eq(&at, "math glyph info", got, want);
                for (code, corner) in corners {
                    for height in [-500.0, 0.0, 250.5, 1e6, f64::NAN] {
                        let mut kern = 0.0;
                        let st = daegun_font_math_kern(f.c, g, place.c, code, height, &mut kern);
                        let r = f.rust.math_kern(glyph, &place.rust, corner, height);
                        let want = (status(&r), r.ok().map(bits));
                        d.eq(
                            &at,
                            "math_kern",
                            (g, code, bits(height), (st, (st == Status::Ok).then_some(bits(kern)))),
                            (g, code, bits(height), want),
                        );
                    }
                }
                for (code, orientation) in
                    [(crate::ffi::HORIZONTAL, Orientation::Horizontal), (crate::ffi::VERTICAL, Orientation::Vertical)]
                {
                    let mut c = core::ptr::null_mut();
                    let st = daegun_font_math_glyph_variants(f.c, g, place.c, code, &mut c);
                    let got = (!c.is_null()).then(|| {
                        let mut n = 0;
                        let variants = lent(daegun_math_construction_variants(c, &mut n), n).to_vec();
                        let mut a = MaybeUninit::<GlyphAssemblyC>::zeroed();
                        let ast = daegun_math_construction_assembly(c, a.as_mut_ptr());
                        let assembly = (ast == Status::Ok).then(|| {
                            let a = a.assume_init();
                            crate::math::GlyphAssembly {
                                italics_correction: a.italics_correction,
                                parts: lent(a.parts, a.parts_len).to_vec(),
                            }
                        });
                        alloc::format!("{:?}", MathGlyphConstruction { assembly, variants })
                    });
                    daegun_math_construction_free(c);
                    let r = f.rust.math_glyph_variants(glyph, &place.rust, orientation);
                    let want = (found(&r), r.map(|c| alloc::format!("{c:?}")));
                    d.eq(&at, "math_glyph_variants", (g, code, (st, got)), (g, code, want));
                }
            }
        }
    });
    d.done("math");
}
