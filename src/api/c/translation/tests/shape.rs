use super::*;
use crate::Orientation;
use crate::ffi::font::{Coord, daegun_font_location, daegun_location_free};
use crate::ffi::testing::{CFont, asset};
use crate::unicode::Script;

// Each reads as Rust's parser does, a range that holds nothing staying empty though C's end of 0 runs on.
#[test]
fn a_feature_and_a_tag_read_from_their_text_as_rust_does() {
    const READ: [&str; 10] = [
        "kern", "-liga", "+kern=0", "aalt=2", "kern[3:5]=0", "kern[3]", "kern[:7]", "kern[2:]", "kern[:0]", "kern[5:0]",
    ];
    for text in READ {
        let mut out = FeatureC { tag: 0, value: 0, start: 0, end: 0 };
        let st = unsafe { daegun_feature_from_string(text.as_ptr().cast(), text.len(), &raw mut out) };
        let rust: Feature = text.parse().expect("the text parses");
        let got = feature_of(&out);
        assert_eq!((st, got.tag, got.value), (Status::Ok, rust.tag, rust.value), "{text}");
        match rust.bytes.start < rust.bytes.end {
            true => assert_eq!(got.bytes, rust.bytes, "{text}"),
            false => assert!(got.bytes.start >= got.bytes.end, "{text} covers {:?}", got.bytes),
        }
    }
    let mut out = FeatureC { tag: 0, value: 0, start: 0, end: 0 };
    for text in ["", "=1", "kern[", "toolong"] {
        let st = unsafe { daegun_feature_from_string(text.as_ptr().cast(), text.len(), &raw mut out) };
        assert_eq!(st, Status::InvalidArgument, "{text}");
    }
    let (null, some) = (core::ptr::null(), b"kern".as_ptr().cast());
    assert_eq!(unsafe { daegun_feature_from_string(null, 4, &raw mut out) }, Status::Null);
    assert_eq!(unsafe { daegun_feature_from_string(some, 4, core::ptr::null_mut()) }, Status::Null);
    let mut tag = 0;
    let st = unsafe { crate::ffi::base::daegun_tag_from_string(c"wght".as_ptr(), 4, &raw mut tag) };
    assert_eq!((st, tag), (Status::Ok, u32::from_be_bytes(*b"wght")));
    let st = unsafe { crate::ffi::base::daegun_tag_from_string(c"lao".as_ptr(), 3, &raw mut tag) };
    assert_eq!((st, tag), (Status::Ok, u32::from_be_bytes(*b"lao ")));
    let st = unsafe { crate::ffi::base::daegun_tag_from_string(c"toolong".as_ptr(), 7, &raw mut tag) };
    assert_eq!(st, Status::InvalidArgument);
    assert_eq!(unsafe { crate::ffi::base::daegun_tag_from_string(null, 4, &raw mut tag) }, Status::Null);
}

#[test]
fn every_direction_cluster_level_and_ignorable_has_a_code() {
    for v in [None, Some(Direction::LeftToRight), Some(Direction::RightToLeft), Some(Direction::TopToBottom),
              Some(Direction::BottomToTop)] {
        let code = match v {
            None => DIRECTION_DEFAULT,
            Some(Direction::LeftToRight) => DIRECTION_LTR,
            Some(Direction::RightToLeft) => DIRECTION_RTL,
            Some(Direction::TopToBottom) => DIRECTION_TTB,
            Some(Direction::BottomToTop) => DIRECTION_BTT,
        };
        assert_eq!(direction(code), Ok(v));
    }
    for v in [ClusterLevel::MonotoneGraphemes, ClusterLevel::MonotoneCharacters, ClusterLevel::Characters,
              ClusterLevel::Graphemes] {
        let code = match v {
            ClusterLevel::MonotoneGraphemes => CLUSTER_MONOTONE_GRAPHEMES,
            ClusterLevel::MonotoneCharacters => CLUSTER_MONOTONE_CHARACTERS,
            ClusterLevel::Characters => CLUSTER_CHARACTERS,
            ClusterLevel::Graphemes => CLUSTER_GRAPHEMES,
        };
        assert_eq!(cluster_level(code), Ok(v));
    }
    for v in [Ignorables::Hide, Ignorables::Remove, Ignorables::Preserve] {
        let code = match v {
            Ignorables::Hide => IGNORABLES_HIDE,
            Ignorables::Remove => IGNORABLES_REMOVE,
            Ignorables::Preserve => IGNORABLES_PRESERVE,
        };
        assert_eq!(ignorables(code), Ok(v));
    }
    assert!(direction(5).is_err() && cluster_level(4).is_err() && ignorables(3).is_err());
}

// daegun_shape_options_default's struct is Rust's default, field for field. The pattern names every field, so one
// that Rust gains does not build until C has it.
#[test]
fn the_default_shape_options_are_rusts() {
    let c = ShapeOptionsC::defaults();
    let given = unsafe { slice_of(c.features, c.features_len) }.unwrap();
    let features: Vec<Feature> = given.iter().map(feature_of).collect();
    let ShapeOptions {
        direction,
        script,
        language,
        features,
        cluster_level,
        before,
        after,
        beginning_of_text,
        point_size,
        report_unsafe_to_concat,
        report_tatweel_positions,
        ignorables,
        suppress_dotted_circle,
        invisible_glyph,
        seed_script,
        not_found_glyph,
        not_found_variation_selector_glyph,
    } = shape_options(&c, &features).unwrap();
    let d = ShapeOptions::default();
    assert_eq!((direction, script, language, features.len()), (d.direction, d.script, d.language, d.features.len()));
    assert_eq!((cluster_level, before, after, beginning_of_text), (d.cluster_level, d.before, d.after,
                                                                    d.beginning_of_text));
    assert_eq!((point_size, report_unsafe_to_concat, report_tatweel_positions),
               (d.point_size, d.report_unsafe_to_concat, d.report_tatweel_positions));
    assert_eq!((ignorables, suppress_dotted_circle, invisible_glyph, seed_script),
               (d.ignorables, d.suppress_dotted_circle, d.invisible_glyph, d.seed_script));
    assert_eq!((not_found_glyph, not_found_variation_selector_glyph),
               (d.not_found_glyph, d.not_found_variation_selector_glyph));
}

// Features on the stack and past it read alike: an end of 0 or UINT32_MAX runs to the text's end.
#[test]
fn features_convert_alike_on_the_stack_and_past_it() {
    for count in [0, 3, STACK_FEATURES, STACK_FEATURES + 1, 40] {
        let given: Vec<FeatureC> = (0..count as u32)
            .map(|i| {
                let end = [0, u32::MAX, i + 9][i as usize % 3];
                FeatureC { tag: u32::from_be_bytes(*b"liga") + i, value: i, start: i, end }
            })
            .collect();
        let mut c = ShapeOptionsC::defaults();
        (c.features, c.features_len) = (given.as_ptr(), given.len());
        let got = unsafe { with_options(&c, |o| o.features.to_vec()) }.unwrap();
        let want: Vec<Feature> = given
            .iter()
            .map(|f| {
                let end = if f.end == 0 || f.end == u32::MAX { usize::MAX } else { f.end as usize };
                Feature { bytes: (f.start as usize..end).into(), ..Feature::new(Tag(f.tag), f.value) }
            })
            .collect();
        assert_eq!(got, want, "{count} features");
    }
}

// A flag byte other than 1, as a memset or another language's binding may leave, reads as set.
#[test]
fn any_nonzero_flag_byte_is_set() {
    let mut c = ShapeOptionsC::defaults();
    (c.has_point_size, c.point_size, c.suppress_dotted_circle) = (2, 12.0, 0xFF);
    let built = shape_options(&c, &[]).unwrap();
    assert_eq!((built.point_size, built.suppress_dotted_circle), (Some(12.0), true));
}

fn shaped(font: *const Font, wght: f64) -> (Vec<u32>, Vec<f64>) {
    let coord = Coord { tag: u32::from_be_bytes(*b"wght"), value: wght };
    let (mut at, mut run) = (core::ptr::null_mut(), core::ptr::null_mut());
    unsafe {
        assert_eq!(daegun_font_location(font, &coord, 1, &mut at), Status::Ok);
        let text = c"office affine";
        assert_eq!(daegun_font_shape(font, text.as_ptr(), 13, at, core::ptr::null(), &mut run), Status::Ok);
        let (mut g, mut a) = (0, 0);
        let glyphs = core::slice::from_raw_parts(daegun_shaped_run_glyphs(run, &mut g), g);
        let advances = core::slice::from_raw_parts(daegun_shaped_run_advances(run, &mut a), a).to_vec();
        let glyphs = glyphs.iter().map(|g| g.0).collect();
        daegun_shaped_run_free(run);
        daegun_location_free(at);
        (glyphs, advances)
    }
}

// Rule 5: threads sharing one cold font fill its caches at once, each making its own locations, and shape as one font
// alone does.
#[test]
fn a_font_shared_by_threads_shapes_as_one_alone() {
    let weights = [100.0, 400.0, 900.0];
    let alone = CFont::of(asset("inter/InterVariable.ttf"));
    let want: Vec<_> = weights.iter().map(|&w| shaped(alone.handle, w)).collect();
    let shared = CFont::of(asset("inter/InterVariable.ttf"));
    let font: &Font = unsafe { &*shared.handle };
    std::thread::scope(|s| {
        // Two threads under Miri, which runs them in turn and checks each access.
        let threads: Vec<_> = (0..if cfg!(miri) { 2 } else { 4 })
            .map(|_| s.spawn(|| weights.iter().map(|&w| shaped(font, w)).collect::<Vec<_>>()))
            .collect();
        for t in threads {
            assert_eq!(t.join().unwrap(), want);
        }
    });
}

// "(" alone in an Arabic font, seeded Arabic as a layout seeds punctuation between Arabic words: C shapes it as Rust's
// seeded run is, the Arabic shaper and the mirrored parenthesis, and lends the shaper's static name.
#[test]
fn a_seed_script_reaches_the_shaper() {
    let c = CFont::of(asset("scheherazade-new/ScheherazadeNew-Regular.ttf"));
    let font = Font::from_bytes(&c.bytes).unwrap();
    let arabic = Script::from_iso15924("Arab").unwrap();
    let at = crate::Location::default();
    let want = font.shape("(", &at, &ShapeOptions::default().with_seed_script(arabic));
    let plain = font.shape("(", &at, &ShapeOptions::default());
    assert_ne!(want.glyphs, plain.glyphs, "seeding changes nothing here to test");

    let mut opts = ShapeOptionsC::defaults();
    opts.seed_script = u32::from_be_bytes(*b"Arab");
    let (mut run, mut again) = (core::ptr::null_mut(), core::ptr::null_mut());
    let (mut n, mut code, mut shaper, mut second) = (0, -1, Str::EMPTY, Str::EMPTY);
    unsafe {
        let text = c"(".as_ptr();
        assert_eq!(daegun_font_shape(c.handle, text, 1, core::ptr::null(), &opts, &mut run), Status::Ok);
        let glyphs = core::slice::from_raw_parts(daegun_shaped_run_glyphs(run, &mut n), n);
        assert_eq!(glyphs, &want.glyphs[..]);
        assert_eq!(daegun_shaped_run_shaper(run, &mut code), Status::Ok);
        assert_eq!(code, crate::shape::Shaper::Arabic as i32);
        assert_eq!(daegun_shaper_name(code, &mut shaper), Status::Ok);
        let name = core::slice::from_raw_parts(shaper.data.cast::<u8>(), shaper.len);
        assert_eq!(name, want.shaper.name().as_bytes());
        assert_eq!(*shaper.data.add(shaper.len), 0, "the shaper's name is not NUL-terminated");
        assert_eq!(daegun_font_shape(c.handle, text, 1, core::ptr::null(), &opts, &mut again), Status::Ok);
        assert_eq!(daegun_shaped_run_shaper(again, &mut code), Status::Ok);
        assert_eq!(daegun_shaper_name(code, &mut second), Status::Ok);
        assert_eq!(second.data, shaper.data, "the shaper's name was copied for a run");
        daegun_shaped_run_free(again);
        daegun_shaped_run_free(run);
    }
}

// Each code is its shaper's place in the list C reads names from, and a code past it is refused.
#[test]
fn every_shaper_has_its_code_and_name() {
    let shapers = &crate::daecore::daeshaper::script::ALL;
    for (i, s) in shapers.iter().enumerate() {
        assert_eq!(s.kind() as usize, i, "{:?} is not at its code", s.kind());
        let mut name = Str::EMPTY;
        assert_eq!(unsafe { daegun_shaper_name(i as i32, &mut name) }, Status::Ok);
        assert_eq!(unsafe { crate::ffi::testing::str_of(name) }, s.kind().name());
    }
    let mut name = Str::EMPTY;
    for bad in [-1, shapers.len() as i32, i32::MAX] {
        assert_eq!(unsafe { daegun_shaper_name(bad, &mut name) }, Status::InvalidArgument, "shaper {bad}");
    }
    assert_eq!(unsafe { daegun_shaper_name(0, core::ptr::null_mut()) }, Status::Null);
}

// A run of the caller's alone is shaped in place; one the shape cache shares is copied first, the cache's untouched;
// a failure leaves the run as it was.
#[test]
fn shaping_into_a_run_reuses_it() {
    use crate::ffi::testing::{c_run, rust_run};
    use core::ptr::{eq, null, null_mut};
    let c = CFont::of(asset("inter/InterVariable.ttf"));
    let font = Font::from_bytes(&c.bytes).unwrap();
    let rust = |t: &str| Some(rust_run(&font.shape(t, &crate::Location::default(), &ShapeOptions::default())));
    let (first, second) = ("Typography", "Hand-set type");
    unsafe {
        let into = |t: &str, run| daegun_font_shape_into(c.handle, t.as_ptr().cast(), t.len(), null(), null(), run);
        let shape = |t: &str, run| daegun_font_shape(c.handle, t.as_ptr().cast(), t.len(), null(), null(), run);
        let mut run = null_mut();
        assert_eq!(into(first, &mut run), Status::Ok);
        assert_eq!(c_run(run), rust(first));
        let kept = run;
        assert_eq!(into(second, &mut run), Status::Ok);
        assert!(eq(run, kept) && c_run(run) == rust(second), "a run of its own was not shaped in place");
        let bad = [0xffu8];
        let st = daegun_font_shape_into(c.handle, bad.as_ptr().cast(), 1, null(), null(), &mut run);
        assert!(st == Status::InvalidArgument && eq(run, kept), "a refused text took the run");
        assert_eq!(c_run(run), rust(second), "a refused text changed the run");
        daegun_shaped_run_free(run);

        let mut cached = null_mut();
        assert_eq!(shape(first, &mut cached), Status::Ok);
        let shared = cached;
        assert_eq!(into(second, &mut cached), Status::Ok);
        assert!(!eq(cached, shared) && c_run(cached) == rust(second), "a run the cache shares was shaped in place");
        let mut again = null_mut();
        assert_eq!(shape(first, &mut again), Status::Ok);
        assert!(eq(again, shared) && c_run(again) == rust(first), "the cache's run changed");
        daegun_shaped_run_free(again);
        daegun_shaped_run_free(cached);
        assert_eq!(into(first, null_mut()), Status::Null);
    }
}

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    use crate::ffi::testing::{Diffs, TEXTS, bits, c_run, lent, rust_run};
    use crate::unicode::BaseDirection;
    let features = [
        FeatureC { tag: u32::from_be_bytes(*b"liga"), value: 0, start: 0, end: 0 },
        FeatureC { tag: u32::from_be_bytes(*b"kern"), value: 0, start: 2, end: 6 },
        FeatureC { tag: u32::from_be_bytes(*b"smcp"), value: 1, start: 0, end: u32::MAX },
    ];
    let rust_features = [
        Feature::new(Tag::new(b"liga"), 0),
        Feature { bytes: (2..6).into(), ..Feature::new(Tag::new(b"kern"), 0) },
        Feature::new(Tag::new(b"smcp"), 1),
    ];
    // Each set of options as C fills it and as Rust builds it.
    let mut c_opts = alloc::vec::Vec::new();
    let mut rust_opts = alloc::vec::Vec::new();
    let base = ShapeOptionsC::defaults();
    c_opts.push(ShapeOptionsC { direction: DIRECTION_TTB, ..base });
    rust_opts.push(ShapeOptions { direction: Some(Direction::TopToBottom), ..ShapeOptions::default() });
    c_opts.push(ShapeOptionsC { language: c"tr".as_ptr(), ..base });
    rust_opts.push(ShapeOptions { language: Some("tr"), ..ShapeOptions::default() });
    c_opts.push(ShapeOptionsC {
        script: u32::from_be_bytes(*b"latn"),
        features: features.as_ptr(),
        features_len: features.len(),
        ..base
    });
    rust_opts.push(ShapeOptions {
        script: Some(Tag::new(b"latn")),
        features: &rust_features,
        ..ShapeOptions::default()
    });
    c_opts.push(ShapeOptionsC {
        direction: DIRECTION_RTL,
        cluster_level: CLUSTER_CHARACTERS,
        before: c"ab".as_ptr(),
        before_len: 2,
        after: "\u{627}".as_ptr().cast(),
        after_len: 2,
        report_unsafe_to_concat: 1,
        report_tatweel_positions: 1,
        has_point_size: 1,
        point_size: 48.0,
        ignorables: IGNORABLES_PRESERVE,
        has_invisible_glyph: 1,
        invisible_glyph: 3,
        not_found_glyph: 1,
        ..base
    });
    rust_opts.push(ShapeOptions {
        direction: Some(Direction::RightToLeft),
        cluster_level: ClusterLevel::Characters,
        before: "ab",
        after: "\u{627}",
        report_unsafe_to_concat: true,
        report_tatweel_positions: true,
        point_size: Some(48.0),
        ignorables: Ignorables::Preserve,
        invisible_glyph: Some(GlyphId(3)),
        not_found_glyph: GlyphId(1),
        ..ShapeOptions::default()
    });
    c_opts.push(ShapeOptionsC {
        direction: DIRECTION_BTT,
        suppress_dotted_circle: 1,
        beginning_of_text: 1,
        seed_script: u32::from_be_bytes(*b"Arab"),
        has_not_found_variation_selector_glyph: 1,
        not_found_variation_selector_glyph: 2,
        ignorables: IGNORABLES_REMOVE,
        cluster_level: CLUSTER_GRAPHEMES,
        ..base
    });
    rust_opts.push(ShapeOptions {
        direction: Some(Direction::BottomToTop),
        suppress_dotted_circle: true,
        beginning_of_text: true,
        seed_script: Some(crate::unicode::Script::from_iso15924("Arab").unwrap()),
        not_found_variation_selector_glyph: Some(GlyphId(2)),
        ignorables: Ignorables::Remove,
        cluster_level: ClusterLevel::Graphemes,
        ..ShapeOptions::default()
    });
    let plain = ShapeOptions::default();
    let mut d = Diffs::default();
    crate::ffi::testing::each_font(&mut d, |f, d| unsafe {
        for place in f.places().into_iter().filter(|p| p.whole) {
            let at = &alloc::format!("{} @{}", f.name, place.label);
            for (t, text) in TEXTS.iter().enumerate() {
                let mut run = core::ptr::null_mut();
                let st = daegun_font_shape(f.c, text.as_ptr().cast(), text.len(), place.c, core::ptr::null(), &mut run);
                let want = rust_run(&f.rust.shape(text, &place.rust, &ShapeOptions::default()));
                d.eq(at, "shape", (t, st, c_run(run)), (t, Status::Ok, Some(want.clone())));
                let (bytes, len) = (text.as_ptr().cast(), text.len());
                let st = daegun_font_shape_into(f.c, bytes, len, place.c, core::ptr::null(), &mut run);
                d.eq(at, "shape_into", (t, st, c_run(run)), (t, Status::Ok, Some(want)));
                daegun_shaped_run_free(run);
                for (o, (c, r)) in c_opts.iter().zip(&rust_opts).enumerate() {
                    let mut run = core::ptr::null_mut();
                    let st = daegun_font_shape(f.c, text.as_ptr().cast(), text.len(), place.c, c, &mut run);
                    let want = rust_run(&f.rust.shape(text, &place.rust, r));
                    d.eq(at, "shape with options", (t, o, st, c_run(run)), (t, o, Status::Ok, Some(want)));
                    daegun_shaped_run_free(run);
                }
                let bases = [
                    (crate::ffi::BASE_AUTO, BaseDirection::Auto),
                    (crate::ffi::BASE_LTR, BaseDirection::LeftToRight),
                    (crate::ffi::BASE_RTL, BaseDirection::RightToLeft),
                ];
                // Right to left also with a set of options, the others with none.
                for (b, (code, base)) in bases.into_iter().enumerate() {
                    let (c, r) = match b == 2 {
                        true => (&raw const c_opts[3], &rust_opts[3]),
                        false => (core::ptr::null(), &plain),
                    };
                    let mut runs = core::ptr::null_mut();
                    let st = daegun_font_shape_bidi(f.c, text.as_ptr().cast(), text.len(), place.c, code, c, &mut runs);
                    let mut n = 0;
                    let _ = daegun_bidi_runs_count(runs, &mut n);
                    let mut got = alloc::vec::Vec::new();
                    for i in 0..n {
                        let (mut run, mut level, mut start, mut end) = (core::ptr::null(), 0, 0, 0);
                        let _ = daegun_bidi_runs_at(runs, i, &mut run, &mut level, &mut start, &mut end);
                        got.push((level, start, end, c_run(run)));
                    }
                    daegun_bidi_runs_free(runs);
                    let want: alloc::vec::Vec<_> = (f.rust.shape_bidi(text, &place.rust, base, r).iter())
                        .map(|b| (b.level, b.bytes.start, b.bytes.end, Some(rust_run(&b.run))))
                        .collect();
                    d.eq(at, "shape_bidi", (t, b, st, got), (t, b, Status::Ok, want));
                }
                for (code, o) in
                    [(crate::ffi::HORIZONTAL, Orientation::Horizontal), (crate::ffi::VERTICAL, Orientation::Vertical)]
                {
                    let mut out = alloc::vec![0.0; text.len() + 1];
                    let mut n = 0;
                    let st = daegun_font_caret_positions(
                        f.c,
                        text.as_ptr().cast(),
                        text.len(),
                        place.c,
                        code,
                        out.as_mut_ptr(),
                        out.len(),
                        &mut n,
                    );
                    let c: alloc::vec::Vec<_> = lent(out.as_ptr(), n).iter().map(|&v| bits(v)).collect();
                    let want: alloc::vec::Vec<_> =
                        f.rust.caret_positions(text, &place.rust, o).iter().map(|&v| bits(v)).collect();
                    d.eq(at, "caret_positions", (t, st, c), (t, Status::Ok, want));
                }
            }
        }
    });
    d.done("shape");
}
