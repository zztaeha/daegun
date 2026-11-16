use super::*;

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    use crate::Orientation;
    use crate::ffi::testing::{Diffs, SAMPLE, bits, each_font, filled, found, lent, status};
    use core::ptr::null_mut;
    let orientations =
        [(crate::ffi::HORIZONTAL, Orientation::Horizontal), (crate::ffi::VERTICAL, Orientation::Vertical)];
    let mut d = Diffs::default();
    each_font(&mut d, |f, d| unsafe {
        let mut free = false;
        let st = daegun_font_has_glyph_free_base(f.c, &mut free);
        d.eq(&f.name, "has_glyph_free_base", (st, free), (Status::Ok, f.rust.has_glyph_free_base()));
        let scripts = f.rust.script_tags();
        let got = filled(|o, l, c| daegun_font_script_tags(f.c, o, l, c));
        d.eq(&f.name, "script_tags", got, (Status::Ok, scripts.iter().map(|t| t.0).collect()));
        let mut probes: Vec<u32> = scripts.iter().map(|t| t.0).collect();
        probes.extend([*b"DFLT", *b"latn", *b"arab", *b"hani", *b"cyrl", *b"zzzz"].map(u32::from_be_bytes));
        let (st, list) = filled(|o, l, c| daegun_font_feature_tags(f.c, 0, 0, o, l, c));
        let want: Vec<u32> = f.rust.feature_tags(None, None).iter().map(|t| t.0).collect();
        d.eq(&f.name, "feature_tags", (0, 0, st, list), (0, 0, Status::Ok, want));
        for &script in &probes {
            let (st, list) = filled(|o, l, c| daegun_font_language_system_tags(f.c, script, o, l, c));
            let languages = f.rust.language_system_tags(Tag(script));
            let want: Vec<u32> = languages.iter().map(|t| t.0).collect();
            d.eq(&f.name, "language_tags", (script, st, list), (script, Status::Ok, want.clone()));
            for language in core::iter::once(0).chain(want).chain([u32::from_be_bytes(*b"ZZZ ")]) {
                let (st, list) = filled(|o, l, c| daegun_font_feature_tags(f.c, script, language, o, l, c));
                let r = f.rust.feature_tags(Some(Tag(script)), crate::ffi::opt_tag(language));
                let want: Vec<u32> = r.iter().map(|t| t.0).collect();
                d.eq(
                    &f.name,
                    "feature_tags",
                    (script, language, st, list),
                    (script, language, Status::Ok, want),
                );
            }
        }
        let count = f.rust.glyph_count();
        let gids = [0, 1, 2, 5, 37, count.saturating_sub(1), count.saturating_add(5)];
        for place in f.places().into_iter().filter(|p| p.whole) {
            let at = &alloc::format!("{} @{}", f.name, place.label);
            for &script in &probes {
                for (code, orientation) in orientations {
                    let mut s = null_mut();
                    let st = daegun_font_base_script(f.c, script, place.c, code, &mut s);
                    let got = (!s.is_null()).then(|| {
                        let (mut tag, mut n) = (0, 0);
                        let dst = daegun_base_script_default_baseline(s, &mut tag);
                        let baselines: Vec<(u32, u64)> = lent(daegun_base_script_baselines(s, &mut n), n)
                            .iter()
                            .map(|b| (b.tag, bits(b.position)))
                            .collect();
                        ((dst == Status::Ok).then_some(tag), baselines)
                    });
                    daegun_base_script_free(s);
                    let r = f.rust.base_script(Tag(script), &place.rust, orientation);
                    let want = r.as_ref().map(|b| {
                        (b.default_baseline.map(|t| t.0), b.baselines.iter().map(|(t, &p)| (t.0, bits(p))).collect())
                    });
                    d.eq(at, "base_script", (script, code, st, got), (script, code, found(&r), want));
                    let languages = f.rust.language_system_tags(Tag(script));
                    for language in core::iter::once(0).chain(languages.iter().map(|t| t.0)) {
                        for feature in [0, u32::from_be_bytes(*b"kern"), u32::from_be_bytes(*b"vert")] {
                            let mut e = core::mem::MaybeUninit::<ExtentsC>::zeroed();
                            let st =
                                daegun_font_base_extents(f.c, script, place.c, code, language, feature, e.as_mut_ptr());
                            let got = (st == Status::Ok).then(|| {
                                let e = e.assume_init();
                                (e.has_min.then_some(bits(e.min)), e.has_max.then_some(bits(e.max)))
                            });
                            let r = f.rust.base_extents(
                                Tag(script),
                                &place.rust,
                                orientation,
                                crate::ffi::opt_tag(language),
                                crate::ffi::opt_tag(feature),
                            );
                            let want = r.as_ref().map(|e| (e.min.map(bits), e.max.map(bits)));
                            let what = (script, language, feature, code);
                            d.eq(at, "base_extents", (what, st, got), (what, found(&r), want));
                        }
                    }
                }
            }
            let (st, closure) = filled(|o, l, c: &mut usize| {
                daegun_font_glyph_closure(f.c, gids.as_ptr(), gids.len(), place.c, o, l, c)
            });
            let got = (st == Status::Ok).then_some(closure);
            let r = f.rust.glyph_closure(glyphs_of(&gids), &place.rust);
            d.eq(at, "glyph_closure", (st, got), (status(&r), r.ok().map(|g| g.iter().map(|g| g.0).collect())));

            let mut subsets = [null_mut(), null_mut()];
            let sts = [
                daegun_font_subset(f.c, gids.as_ptr(), gids.len(), place.c, &mut subsets[0]),
                daegun_font_subset_text(f.c, SAMPLE.as_ptr().cast(), SAMPLE.len(), place.c, &mut subsets[1]),
            ];
            let rust = [f.rust.subset(glyphs_of(&gids), &place.rust), f.rust.subset_text(SAMPLE, &place.rust)];
            for (i, ((st, s), r)) in sts.into_iter().zip(subsets).zip(&rust).enumerate() {
                let got = (!s.is_null()).then(|| {
                    let mut n = 0;
                    let bytes = lent(daegun_subset_bytes(s, &mut n), n).to_vec();
                    let map: Vec<Option<u32>> = (0..count.saturating_add(2))
                        .map(|old| {
                            let mut new = 0;
                            (daegun_subset_renumbered(s, old, &mut new) == Status::Ok).then_some(new)
                        })
                        .collect();
                    (bytes, map)
                });
                daegun_subset_free(s);
                let want = r.as_ref().ok().map(|s| {
                    let map = (0..count.saturating_add(2)).map(|old| s.renumbered(GlyphId(old)).map(|g| g.0)).collect();
                    (s.font.clone(), map)
                });
                d.eq(at, "subset", (i, st, got), (i, status(r), want));
            }
        }
    });
    d.done("tables");
}
