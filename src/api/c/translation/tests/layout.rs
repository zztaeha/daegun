use super::*;
use crate::Orientation;
use crate::ffi::testing::last_error;

#[test]
fn every_layout_and_justification_code_reads_back() {
    for v in [WritingMode::Horizontal, WritingMode::VerticalRl, WritingMode::VerticalLr] {
        let code = match v {
            WritingMode::Horizontal => WRITING_HORIZONTAL,
            WritingMode::VerticalRl => WRITING_VERTICAL_RL,
            WritingMode::VerticalLr => WRITING_VERTICAL_LR,
        };
        assert_eq!(writing_mode(code), Ok(v));
    }
    for v in [Align::Start, Align::End, Align::Center, Align::Justify] {
        let code = match v {
            Align::Start => ALIGN_START,
            Align::End => ALIGN_END,
            Align::Center => ALIGN_CENTER,
            Align::Justify => ALIGN_JUSTIFY,
        };
        assert_eq!(align(code), Ok(v));
    }
    for v in [TextOrientation::Mixed, TextOrientation::Upright, TextOrientation::Sideways] {
        let code = match v {
            TextOrientation::Mixed => TEXT_ORIENTATION_MIXED,
            TextOrientation::Upright => TEXT_ORIENTATION_UPRIGHT,
            TextOrientation::Sideways => TEXT_ORIENTATION_SIDEWAYS,
        };
        assert_eq!(text_orientation(code), Ok(v));
    }
    for v in [BreakStrategy::Greedy, BreakStrategy::Optimal] {
        let code = match v {
            BreakStrategy::Greedy => BREAK_GREEDY,
            BreakStrategy::Optimal => BREAK_OPTIMAL,
        };
        assert_eq!(strategy(code), Ok(v));
    }
    for v in [Adjustment::Shrink, Adjustment::Extend] {
        let code = match v {
            Adjustment::Shrink => ADJUST_SHRINK,
            Adjustment::Extend => ADJUST_EXTEND,
        };
        assert_eq!(adjustment_of(code), Ok(v));
    }
    assert!(writing_mode(3).is_err() && align(4).is_err() && text_orientation(3).is_err());
    assert!(strategy(2).is_err() && adjustment_of(2).is_err());
}

// daegun_layout_options_default's struct is Rust's default, field for field, the pattern naming every field.
#[test]
fn the_default_layout_options_are_rusts() {
    let c = LayoutOptionsC::defaults();
    let LayoutOptions {
        max_inline_size,
        align,
        writing_mode,
        text_orientation,
        base_direction,
        language,
        line_height,
        strategy,
        max_lines,
        tab_size,
        segmenter,
        features,
    } = layout_options(&c, None, &[]).unwrap();
    let d = LayoutOptions::default();
    assert_eq!(
        (max_inline_size, align, writing_mode, text_orientation, base_direction, language),
        (d.max_inline_size, d.align, d.writing_mode, d.text_orientation, d.base_direction, d.language)
    );
    assert_eq!(
        (line_height, strategy, max_lines, tab_size, segmenter.is_none(), features),
        (d.line_height, d.strategy, d.max_lines, d.tab_size, d.segmenter.is_none(), d.features)
    );
}

// Each list a level names comes back under its own selector, one it does not name as absent rather than empty, and a
// selector past the eight, or a level past the last, as invalid with its reason.
#[test]
fn each_jstf_lookup_list_reaches_c_under_its_own_selector() {
    let lists = [
        Some(alloc::vec![1u16]),
        None,
        Some(alloc::vec![3, 4]),
        Some(Vec::new()),
        Some(alloc::vec![5]),
        None,
        Some(alloc::vec![7, 8, 9]),
        Some(alloc::vec![10]),
    ];
    let level = JstfModLists {
        shrinkage_enable_gsub: lists[0].clone(),
        shrinkage_disable_gsub: lists[1].clone(),
        shrinkage_enable_gpos: lists[2].clone(),
        shrinkage_disable_gpos: lists[3].clone(),
        extension_enable_gsub: lists[4].clone(),
        extension_disable_gsub: lists[5].clone(),
        extension_enable_gpos: lists[6].clone(),
        extension_disable_gpos: lists[7].clone(),
    };
    let priorities = JstfPriorities(alloc::vec![level]);
    let selectors = [
        JSTF_SHRINKAGE_ENABLE_GSUB,
        JSTF_SHRINKAGE_DISABLE_GSUB,
        JSTF_SHRINKAGE_ENABLE_GPOS,
        JSTF_SHRINKAGE_DISABLE_GPOS,
        JSTF_EXTENSION_ENABLE_GSUB,
        JSTF_EXTENSION_DISABLE_GSUB,
        JSTF_EXTENSION_ENABLE_GPOS,
        JSTF_EXTENSION_DISABLE_GPOS,
    ];
    let (mut data, mut len) = (core::ptr::null(), 0);
    for (which, want) in selectors.into_iter().zip(&lists) {
        let status = unsafe { daegun_jstf_priorities_lookups(&priorities, 0, which, &mut data, &mut len) };
        match want {
            None => assert_eq!(status, Status::Absent, "selector {which}"),
            Some(want) => {
                assert_eq!(status, Status::Ok, "selector {which}");
                assert_eq!(unsafe { core::slice::from_raw_parts(data, len) }, &want[..], "selector {which}");
            }
        }
    }
    for (level, which) in [(0, 8), (1, 0)] {
        crate::ffi::set_error("stale");
        let status = unsafe { daegun_jstf_priorities_lookups(&priorities, level, which, &mut data, &mut len) };
        assert_eq!(status, Status::InvalidArgument, "level {level}, selector {which}");
        assert_ne!(last_error(), "stale", "level {level}, selector {which} gave no reason");
    }
}

// C against Rust over the corpus, as every area's ignored test does: run them with `--ignored`.
#[test]
#[ignore]
fn c_answers_as_rust_over_the_corpus() {
    use crate::ffi::testing::{Diffs, TEXTS, bits, c_run, each_font, filled, lent, rust_run};
    use crate::unicode::BaseDirection;
    use core::ptr::{null, null_mut};
    const PARAGRAPH: &str = "The quick brown fox jumps over the lazy dog.\tTabs\tand soft\u{ad}hyphen\u{ad}ated \
        words, \u{627}\u{644}\u{633}\u{644}\u{627}\u{645} mixed in, then a longer stretch of plain text so that lines \
        wrap more than once.\nA hard break, then \u{6f22}\u{5b57}\u{304b}\u{306a} and \
        \u{e2a}\u{e27}\u{e31}\u{e2a}\u{e14}\u{e35} to end.";
    // Each set of options as C fills it and as Rust builds it.
    let base = LayoutOptionsC::defaults();
    let mut c_feature = core::mem::MaybeUninit::<crate::ffi::shape::FeatureC>::zeroed();
    let text = "liga[4:40]=0";
    let parse = crate::ffi::shape::daegun_feature_from_string;
    let st = unsafe { parse(text.as_ptr().cast(), text.len(), c_feature.as_mut_ptr()) };
    assert_eq!(st, Status::Ok);
    let c_features = [unsafe { c_feature.assume_init() }];
    let rust_features = [text.parse::<crate::shape::Feature>().expect("a feature")];
    let c_opts = [
        base,
        LayoutOptionsC { max_inline_size: 9000.0, ..base },
        LayoutOptionsC {
            max_inline_size: 9000.0,
            align: ALIGN_JUSTIFY,
            strategy: BREAK_OPTIMAL,
            has_line_height: 1,
            line_height: 1500.0,
            tab_size: 4.0,
            ..base
        },
        LayoutOptionsC {
            align: ALIGN_CENTER,
            writing_mode: WRITING_VERTICAL_RL,
            text_orientation: TEXT_ORIENTATION_MIXED,
            max_inline_size: 6000.0,
            has_max_lines: 1,
            max_lines: 3,
            base_direction: crate::ffi::BASE_RTL,
            language: c"ja".as_ptr(),
            ..base
        },
        LayoutOptionsC {
            writing_mode: WRITING_VERTICAL_LR,
            text_orientation: TEXT_ORIENTATION_UPRIGHT,
            align: ALIGN_END,
            max_inline_size: 300.0,
            ..base
        },
        LayoutOptionsC {
            max_inline_size: 9000.0,
            features: c_features.as_ptr(),
            features_len: c_features.len(),
            ..base
        },
    ];
    let r = LayoutOptions::default();
    let rust_opts = [
        r,
        LayoutOptions { max_inline_size: 9000.0, ..r },
        LayoutOptions {
            max_inline_size: 9000.0,
            align: Align::Justify,
            strategy: BreakStrategy::Optimal,
            line_height: Some(1500.0),
            tab_size: 4.0,
            ..r
        },
        LayoutOptions {
            align: Align::Center,
            writing_mode: WritingMode::VerticalRl,
            text_orientation: TextOrientation::Mixed,
            max_inline_size: 6000.0,
            max_lines: Some(3),
            base_direction: BaseDirection::RightToLeft,
            language: Some("ja"),
            ..r
        },
        LayoutOptions {
            writing_mode: WritingMode::VerticalLr,
            text_orientation: TextOrientation::Upright,
            align: Align::End,
            max_inline_size: 300.0,
            ..r
        },
        LayoutOptions { max_inline_size: 9000.0, features: &rust_features, ..r },
    ];
    assert_eq!(c_opts.len(), rust_opts.len(), "a set of options one side lacks would be skipped");
    type Line = (usize, usize, usize, u64, u64, u64, u64, bool);
    type Run = (u64, u64, usize, usize, u8, bool, Option<crate::ffi::testing::RunSig>);
    let mut d = Diffs::default();
    each_font(&mut d, |f, d| unsafe {
        for place in f.places().into_iter().filter(|p| p.whole) {
            let at = &alloc::format!("{} @{}", f.name, place.label);
            for (o, (c, r)) in c_opts.iter().zip(&rust_opts).enumerate() {
                let mut layout = null_mut();
                let st = daegun_font_layout(f.c, PARAGRAPH.as_ptr().cast(), PARAGRAPH.len(), place.c, c, &mut layout);
                let mut info = core::mem::MaybeUninit::<LayoutInfoC>::zeroed();
                let _ = daegun_layout_info_of(layout, info.as_mut_ptr());
                let info = info.assume_init();
                let mut lines: alloc::vec::Vec<(Line, alloc::vec::Vec<Run>)> = alloc::vec::Vec::new();
                for i in 0..info.line_count {
                    let mut l = core::mem::MaybeUninit::<LayoutLineC>::zeroed();
                    let _ = daegun_layout_line_at(layout, i, l.as_mut_ptr());
                    let l = l.assume_init();
                    let mut runs = alloc::vec::Vec::new();
                    for k in 0..l.run_count {
                        let mut run = core::mem::MaybeUninit::<LayoutRunC>::zeroed();
                        let _ = daegun_layout_run_at(layout, i, k, run.as_mut_ptr());
                        let run = run.assume_init();
                        runs.push((
                            bits(run.offset.x),
                            bits(run.offset.y),
                            run.start,
                            run.end,
                            run.level,
                            run.upright,
                            c_run(run.run),
                        ));
                    }
                    let line = (
                        l.run_count,
                        l.start,
                        l.end,
                        bits(l.baseline),
                        bits(l.inline_size),
                        bits(l.ascent),
                        bits(l.descent),
                        l.hard_break,
                    );
                    lines.push((line, runs));
                }
                daegun_layout_free(layout);
                let rust = f.rust.layout(PARAGRAPH, &place.rust, r);
                let want: alloc::vec::Vec<(Line, alloc::vec::Vec<Run>)> = (rust.lines.iter())
                    .map(|l| {
                        let line = (
                            l.runs.len(),
                            l.bytes.start,
                            l.bytes.end,
                            bits(l.baseline),
                            bits(l.inline_size),
                            bits(l.ascent),
                            bits(l.descent),
                            l.hard_break,
                        );
                        let runs = (l.runs.iter())
                            .map(|r| {
                                (
                                    bits(r.offset.x),
                                    bits(r.offset.y),
                                    r.bytes.start,
                                    r.bytes.end,
                                    r.level,
                                    r.upright,
                                    Some(rust_run(&r.run)),
                                )
                            })
                            .collect();
                        (line, runs)
                    })
                    .collect();
                let c_info =
                    (st, bits(info.inline_size), bits(info.block_size), info.has_truncated.then_some(info.truncated));
                let r_info = (Status::Ok, bits(rust.inline_size), bits(rust.block_size), rust.truncated);
                d.eq(at, "layout", (o, c_info, lines), (o, r_info, want));
            }

            for (t, text) in TEXTS.iter().enumerate() {
                for (code, orientation) in
                    [(crate::ffi::HORIZONTAL, Orientation::Horizontal), (crate::ffi::VERTICAL, Orientation::Vertical)]
                {
                    for (script, target, tolerance) in
                        [(*b"arab", 9000.0, 50.0), (*b"hani", 3000.0, 0.0), (*b"latn", 20000.0, 10.0)]
                    {
                        let c = JustifyOptionsC {
                            script: u32::from_be_bytes(script),
                            target_width: target,
                            tolerance,
                            ..JustifyOptionsC::defaults()
                        };
                        let (mut run, mut info) = (null_mut(), core::mem::MaybeUninit::<JustificationC>::zeroed());
                        let st = daegun_font_justify(
                            f.c,
                            text.as_ptr().cast(),
                            text.len(),
                            place.c,
                            code,
                            &c,
                            &mut run,
                            info.as_mut_ptr(),
                        );
                        let info = info.assume_init();
                        let got = (
                            st,
                            bits(info.width),
                            info.has_level.then_some(info.level),
                            info.adjustment,
                            info.best_effort,
                            c_run(run),
                        );
                        crate::ffi::shape::daegun_shaped_run_free(run);
                        let options = JustifyOptions { tolerance, ..JustifyOptions::new(Tag(c.script), target) };
                        let j = f.rust.justify(text, &place.rust, orientation, &options);
                        let adjustment = match j.adjustment {
                            Adjustment::Shrink => ADJUST_SHRINK,
                            Adjustment::Extend => ADJUST_EXTEND,
                        };
                        let run = Some(rust_run(&j.run));
                        let want = (Status::Ok, bits(j.width), j.level, adjustment, j.best_effort, run);
                        d.eq(at, "justify", (t, code, got), (t, code, want));
                    }
                }
            }
            for script in [*b"arab", *b"latn", *b"DFLT", [0; 4]] {
                let tag = u32::from_be_bytes(script);
                let (st, glyphs) = filled(|o, l, c| daegun_font_justification_glyphs(f.c, tag, o, l, c));
                let rust = f
                    .rust
                    .justification_glyphs(Tag(tag))
                    .map(|g| g.iter().map(|g| g.0).collect::<alloc::vec::Vec<_>>());
                let got = (st, (st == Status::Ok).then_some(glyphs));
                d.eq(at, "justification_glyphs", got, (crate::ffi::testing::found(&rust), rust));
                let mut p = null_mut();
                let st = daegun_font_justification_priorities(f.c, tag, 0, &mut p);
                let rust = f.rust.justification_priorities(Tag(tag), None);
                d.eq(at, "justification_priorities", st, crate::ffi::testing::found(&rust));
                let Some(levels) = rust else { continue };
                let mut n = 0;
                let _ = daegun_jstf_priorities_count(p, &mut n);
                d.eq(at, "priority levels", n, levels.len());
                for (i, level) in levels.iter().enumerate() {
                    let lists = [
                        &level.shrinkage_enable_gsub,
                        &level.shrinkage_disable_gsub,
                        &level.shrinkage_enable_gpos,
                        &level.shrinkage_disable_gpos,
                        &level.extension_enable_gsub,
                        &level.extension_disable_gsub,
                        &level.extension_enable_gpos,
                        &level.extension_disable_gpos,
                    ];
                    for (which, list) in lists.into_iter().enumerate() {
                        let (mut lookups, mut k) = (null(), 0);
                        let st = daegun_jstf_priorities_lookups(p, i, which as i32, &mut lookups, &mut k);
                        let got = (st, (st == Status::Ok).then(|| lent(lookups, k).to_vec()));
                        d.eq(
                            at,
                            "jstf lookups",
                            (i, which, got),
                            (i, which, (crate::ffi::testing::found(list), list.clone())),
                        );
                    }
                    for (code, adjustment) in [(ADJUST_SHRINK, Adjustment::Shrink), (ADJUST_EXTEND, Adjustment::Extend)]
                    {
                        let text = TEXTS[1];
                        let mut run = null_mut();
                        let st = daegun_font_shape_justified(
                            f.c,
                            text.as_ptr().cast(),
                            text.len(),
                            place.c,
                            crate::ffi::HORIZONTAL,
                            p,
                            i,
                            code,
                            &mut run,
                        );
                        let want = rust_run(&f.rust.shape_justified(
                            text,
                            &place.rust,
                            Orientation::Horizontal,
                            level,
                            adjustment,
                        ));
                        d.eq(at, "shape_justified", (i, code, st, c_run(run)), (i, code, Status::Ok, Some(want)));
                        crate::ffi::shape::daegun_shaped_run_free(run);
                    }
                }
                daegun_jstf_priorities_free(p);
            }
        }
    });
    d.done("layout");
}
