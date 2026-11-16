use std::sync::Arc;

use daegun::layout::{Align, BreakStrategy, LayoutLine, LayoutOptions, TextLayout, TextOrientation, WritingMode};
use daegun::shape::{Direction, ShapeOptions, ShapedRun};
use daegun::unicode::{BaseDirection, WordSegmenter};
use daegun::{Font, GlyphId, Location, Orientation, Tag};

pub(super) fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    Font::from_vec(std::fs::read(&path).expect("the fixture font"), 0).expect("it parses")
}

// Text shaped at the default instance, in the direction it reads in or, `vertical`, down the line.
fn shaped(f: &Font, text: &str, vertical: bool) -> Arc<ShapedRun> {
    let options = ShapeOptions::default();
    f.shape(
        text,
        &Location::default(),
        &if vertical { options.with_direction(Direction::TopToBottom) } else { options },
    )
}

// A length in px at a 1000px font size, in the font's own units.
fn at_1000px(f: &Font, px: f64) -> f64 {
    px * f64::from(f.units_per_em()) / 1000.0
}

const INTER: &str = "inter/InterVariable.ttf";
const ARABIC: &str = "scheherazade-new/ScheherazadeNew-Regular.ttf";
const SOURCE_HAN: &str = "source-han-sans/SourceHanSansJP-VF.otf";

// Where each line starts along the inline axis, and its own size.
fn starts(layout: &TextLayout) -> Vec<(f64, f64)> {
    layout
        .lines
        .iter()
        .map(|l| (l.runs.iter().map(|r| r.offset.x).fold(f64::INFINITY, f64::min), l.inline_size))
        .collect()
}

// With the default, infinite measure the lines align against the widest of them. Every line ends its
// paragraph there, so Justify starts them as Start does.
#[test]
fn an_infinite_measure_aligns_against_the_widest_line() {
    for (rel, text, rtl) in
        [(INTER, "ab\nabcdefghij", false), (ARABIC, "\u{628}\u{628}\n\u{628}\u{628}\u{628}\u{628}\u{628}\u{628}", true)]
    {
        let f = font(rel);
        let at = |align| f.layout(text, &Location::default(), &LayoutOptions::default().with_align(align));
        let start = at(Align::Start);
        let (widest, short) = (start.inline_size, start.lines[0].inline_size);
        assert!(short < widest && start.lines[1].inline_size == widest, "{rel}: {:?}", starts(&start));
        let (near, far) = if rtl { (widest - short, 0.0) } else { (0.0, widest - short) };
        assert_eq!(starts(&start), [(near, short), (0.0, widest)], "{rel} start");
        assert_eq!(starts(&at(Align::Justify)), [(near, short), (0.0, widest)], "{rel} justify");
        assert_eq!(starts(&at(Align::End)), [(far, short), (0.0, widest)], "{rel} end");
        assert_eq!(starts(&at(Align::Center)), [((widest - short) / 2.0, short), (0.0, widest)], "{rel} center");
    }
}

// Vertical baselines are physical: vertical-lr's first line sits a descent in from the left, the rest rightward, rl's
// an ascent in from the right, the rest leftward. Source Han Sans's vhea is made 700 over 300, so the edges differ.
#[test]
fn vertical_lines_advance_rightward_in_lr_and_leftward_in_rl() {
    let source = font(SOURCE_HAN);
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        source.table_tags().map(|t| (t, source.table(t).expect("listed").to_vec())).collect();
    let vhea = tables.get_mut(&Tag::new(b"vhea")).expect("vhea");
    vhea[4..8].copy_from_slice(&[700i16.to_be_bytes(), (-300i16).to_be_bytes()].concat());
    let f = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("loads");
    let m = f.line_metrics(&Location::default(), Orientation::Vertical);
    assert_eq!((m.ascent, m.descent), (700.0, -300.0));
    let text = "\u{65E5}\u{672C}\n\u{8A9E}\n\u{6587}\u{5B57}";
    let at = |mode| f.layout(text, &Location::default(), &LayoutOptions::default().with_writing_mode(mode));
    let (lr, rl) = (at(WritingMode::VerticalLr), at(WritingMode::VerticalRl));
    let block = 2.0 * m.line_height() + m.ascent - m.descent;
    assert_eq!((lr.block_size, rl.block_size), (block, block));
    let baselines = |l: &TextLayout| l.lines.iter().map(|l| l.baseline).collect::<Vec<_>>();
    assert_eq!(baselines(&lr), (0..3).map(|i| -m.descent + f64::from(i) * m.line_height()).collect::<Vec<_>>());
    assert_eq!(baselines(&rl), (0..3).map(|i| block - m.ascent - f64::from(i) * m.line_height()).collect::<Vec<_>>());
    let horizontal = f.layout(text, &Location::default(), &LayoutOptions::default());
    let h = f.line_metrics(&Location::default(), Orientation::Horizontal);
    assert_eq!(baselines(&horizontal), (0..3).map(|i| h.ascent + f64::from(i) * h.line_height()).collect::<Vec<_>>());
}

// Each glyph's position along its line, by the character it stands for.
fn glyph_starts(layout: &TextLayout, line: usize) -> Vec<(usize, f64, f64)> {
    let mut out = Vec::new();
    for r in &layout.lines[line].runs {
        let mut x = r.offset.x;
        for (i, &advance) in r.run.advances.iter().enumerate() {
            out.push((r.bytes.start + r.run.clusters[i] as usize, x, advance));
            x += advance;
        }
    }
    out.sort_by_key(|g| g.0);
    out
}

// A tab shapes as the space and reaches the next stop, tab_size spaces from the start edge, past one nearer than half a
// '0'. Both breakers measure it exactly, it hangs at a line's end, and justifying widens only spaces after it.
#[test]
fn a_tab_reaches_its_stop_from_the_start_edge() {
    let f = font(INTER);
    let w = |t: &str| shaped(&f, t, false).advances.iter().sum::<f64>();
    let (space, zero) = (w(" "), w("0"));
    let with = |text: &str, opts: LayoutOptions| f.layout(text, &Location::default(), &opts);
    let d = LayoutOptions::default();

    let plain = with("a\tb", d);
    assert_eq!(plain.lines[0].runs[0].run.glyphs[1], f.glyph_id(' ').expect("Inter maps the space"));
    assert_eq!(glyph_starts(&plain, 0)[2].1, 8.0 * space, "b sits on the first stop");
    let close = (w("a") + zero / 4.0) / space;
    assert_eq!(
        glyph_starts(&with("a\tb", d.with_tab_size(close)), 0)[2].1,
        2.0 * close * space,
        "a stop a quarter '0' away is skipped"
    );
    assert_eq!(
        glyph_starts(&with("a\tb", d.with_tab_size(0.0)), 0)[1].2,
        0.0,
        "a tab size of 0 draws tabs at no width"
    );

    let arabic = font(ARABIC);
    let line = arabic.layout("\u{628}\t\u{628}", &Location::default(), &d);
    let aw = |t: &str| shaped(&arabic, t, false).advances.iter().sum::<f64>();
    assert_eq!(line.inline_size, 8.0 * aw(" ") + aw("\u{628}"), "the stop is measured from the right edge");

    let interval = 30.0 * space;
    for strategy in [BreakStrategy::Greedy, BreakStrategy::Optimal] {
        let opts = d.with_tab_size(30.0).with_max_inline_size(interval + w("b") + 1.0).with_strategy(strategy);
        let broken = with("aaaa\tb bbb", opts);
        let chars: Vec<_> = broken.lines.iter().map(|l| core::ops::Range::from(l.bytes)).collect();
        assert_eq!(chars, [0..7, 7..10], "{strategy:?} measured the tab as wide as it draws");
    }

    let hanging = with("abc\tdef", d.with_max_inline_size(w("abc") + 1.0));
    assert_eq!(
        (core::ops::Range::from(hanging.lines[0].bytes), hanging.lines[0].inline_size),
        (0..4, w("abc")),
        "a tab at a line's end hangs"
    );

    let measure = 8.0 * space + w("z w") + 500.0;
    let justified = with("x y\tz w qqqqqqqqqq", d.with_max_inline_size(measure).with_align(Align::Justify));
    assert_eq!(justified.lines[0].inline_size, measure);
    let g = glyph_starts(&justified, 0);
    assert_eq!(g[1].2, space, "the space before the tab kept its width");
    assert_eq!(g[4].1, 8.0 * space, "z sits on the stop");
    assert!(g[5].2 > space, "the space after the tab took the slack");
}

// A line broken at a soft hyphen shows the font's hyphen (U+2010 in Inter), charged only to a line ending there,
// exactly as drawn: Inter kerns "tv" across it, a line broken there loses that kern, and unbroken it shows nothing.
#[test]
fn a_soft_hyphen_break_shows_a_hyphen() {
    let f = font(INTER);
    let w = |t: &str| shaped(&f, t, false).advances.iter().sum::<f64>();
    let text = "ab Selbst\u{AD}verst\u{E4}ndlichkeit";
    let hyphen = f.glyph_id('\u{2010}').expect("Inter maps U+2010");
    let at = |size| f.layout(text, &Location::default(), &LayoutOptions::default().with_max_inline_size(size));
    let shows = |l: &LayoutLine| l.runs.iter().any(|r| r.run.glyphs.contains(&hyphen));

    let broken = at(w("ab Selbst\u{2010}"));
    let first = &broken.lines[0];
    assert_eq!(first.bytes, (0..11).into(), "through the soft hyphen, two bytes");
    assert!(shows(first), "the line that breaks at the soft hyphen shows no hyphen");
    assert_eq!(first.inline_size, w("ab Selbst\u{2010}"));
    assert!(!shows(&broken.lines[1]));

    let short = at(w("ab Selbst\u{2010}") - 1.0);
    assert_eq!(short.lines[0].bytes, (0..3).into(), "the hyphen's width should push the break back to the space");

    let whole = at(f64::INFINITY);
    assert!(!shows(&whole.lines[0]));
    assert_eq!(whole.lines[0].inline_size, w(text));
}

// In a finite measure Start, End and Center offset each line by none, all and half of its slack, and Justify
// fills every line but its paragraph's last. A right-to-left paragraph, set or detected, swaps Start and End.
#[test]
fn a_finite_measure_aligns_each_line_within_it() {
    let beh = "\u{628}\u{628}";
    for (rel, words, base_direction, rtl) in [
        (INTER, ["ab", "cd", "ef"], BaseDirection::Auto, false),
        (INTER, ["ab", "cd", "ef"], BaseDirection::RightToLeft, true),
        (ARABIC, [beh; 3], BaseDirection::Auto, true),
    ] {
        let f = font(rel);
        let w = |t: &str| shaped(&f, t, false).advances.iter().sum::<f64>();
        let two = words[..2].join(" ");
        let measure = w(&two) + 1.0;
        let text = words.join(" ");
        let options = LayoutOptions::default().with_max_inline_size(measure).with_base_direction(base_direction);
        let at = |align| f.layout(&text, &Location::default(), &options.with_align(align));
        let start = at(Align::Start);
        let split = two.len() + 1;
        let ranges: Vec<_> = start.lines.iter().map(|l| core::ops::Range::from(l.bytes)).collect();
        assert_eq!(ranges, [0..split, split..text.len()], "{rel}");
        let natural: Vec<f64> = start.lines.iter().map(|l| l.inline_size).collect();
        let slack = |i: usize| measure - natural[i];
        let edge = |i: usize, end: bool| if end != rtl { slack(i) } else { 0.0 };
        let lines = |offsets: [f64; 2]| vec![(offsets[0], natural[0]), (offsets[1], natural[1])];

        assert_eq!(starts(&start), lines([edge(0, false), edge(1, false)]), "{rel} {base_direction:?} start");
        assert_eq!(start.inline_size, natural[0].max(natural[1]));
        assert_eq!(starts(&at(Align::End)), lines([edge(0, true), edge(1, true)]), "{rel} {base_direction:?} end");
        let centered = lines([slack(0) / 2.0, slack(1) / 2.0]);
        assert_eq!(starts(&at(Align::Center)), centered, "{rel} {base_direction:?} center");

        let justified = at(Align::Justify);
        let first = starts(&justified)[0];
        assert!(first.0 == 0.0 && (first.1 - measure).abs() < 1e-9, "{rel} {base_direction:?} justify: {first:?}");
        let last = (edge(1, false), natural[1]);
        assert_eq!(starts(&justified)[1], last, "{rel}: a paragraph's last line isn't justified");
        assert_eq!(justified.inline_size, justified.lines[0].inline_size);
    }
}

// In vertical text Mixed sets Han upright and Latin sideways, and Upright and Sideways set every run one way.
// Each run advances down the line by its own orientation's advance.
#[test]
fn text_orientation_sets_each_runs_advance_in_vertical_text() {
    let f = font(SOURCE_HAN);
    let advance = |t: &str, upright| shaped(&f, t, upright).advances.iter().sum::<f64>();
    assert_ne!(advance("a", true), advance("a", false), "the test needs Latin to advance differently upright");
    for (orientation, han, latin) in [
        (TextOrientation::Mixed, true, false),
        (TextOrientation::Upright, true, true),
        (TextOrientation::Sideways, false, false),
    ] {
        let opts =
            LayoutOptions::default().with_writing_mode(WritingMode::VerticalRl).with_text_orientation(orientation);
        let line = &f.layout("\u{65E5}a", &Location::default(), &opts).lines[0];
        let got: Vec<_> =
            line.runs.iter().map(|r| (core::ops::Range::from(r.bytes), r.upright, r.offset.into())).collect();
        let han_advance = advance("\u{65E5}", han);
        assert_eq!(got, [(0..3, han, (0.0, 0.0)), (3..4, latin, (0.0, han_advance))], "{orientation:?}");
        assert_eq!(line.inline_size, han_advance + advance("a", latin), "{orientation:?}");
    }
}

// CSS Writing Modes 4: text-orientation upright makes the direction ltr and every character strong LTR, so bidi
// reorders nothing and each run advances down the line in the text's order.
#[test]
fn upright_text_is_laid_out_left_to_right() {
    let f = font(ARABIC);
    let first = shaped(&f, "\u{628}\u{628} ", true).advances.iter().sum::<f64>();
    for base_direction in [BaseDirection::Auto, BaseDirection::RightToLeft] {
        let opts = LayoutOptions::default()
            .with_writing_mode(WritingMode::VerticalRl)
            .with_text_orientation(TextOrientation::Upright)
            .with_base_direction(base_direction);
        let line = &f.layout("\u{628}\u{628} ab", &Location::default(), &opts).lines[0];
        let got: Vec<_> =
            line.runs.iter().map(|r| (core::ops::Range::from(r.bytes), r.level, r.offset.into())).collect();
        assert_eq!(got, [(0..5, 0, (0.0, 0.0)), (5..7, 0, (0.0, first))], "{base_direction:?}");
    }
}

// Baselines are line_height apart, the font's own by default, and the first sits an ascent down. The block
// runs from the first line's ascent to the last line's descent.
#[test]
fn line_height_spaces_the_baselines_and_sets_the_block() {
    let f = font(INTER);
    let m = f.line_metrics(&Location::default(), Orientation::Horizontal);
    for options in [LayoutOptions::default(), LayoutOptions::default().with_line_height(1500.0)] {
        let (layout, line_height) = (f.layout("a\nb\nc", &Location::default(), &options), options.line_height);
        let step = line_height.unwrap_or(m.line_height());
        let baselines: Vec<f64> = layout.lines.iter().map(|l| l.baseline).collect();
        assert_eq!(baselines, [m.ascent, m.ascent + step, m.ascent + 2.0 * step], "{line_height:?}");
        assert_eq!(layout.block_size, 2.0 * step + m.ascent - m.descent, "{line_height:?}");
        assert!(layout.lines.iter().all(|l| (l.ascent, l.descent) == (m.ascent, m.descent)));
    }
}

// max_lines keeps that many lines, hard-broken or wrapped, and `truncated` says where the first line left out
// starts. The block ends at the last line kept.
#[test]
fn max_lines_truncates_and_says_where() {
    let f = font(INTER);
    let at = |text: &str, size, max_lines: Option<usize>| {
        let options = LayoutOptions::default().with_max_inline_size(size);
        f.layout(text, &Location::default(), &max_lines.map_or(options, |n| options.with_max_lines(n)))
    };
    let hard = at("a\nb\nc", f64::INFINITY, Some(2));
    assert_eq!((hard.lines.len(), hard.truncated), (2, Some(4)));
    assert_eq!(hard.block_size, at("a\nb", f64::INFINITY, None).block_size);

    let narrow = ["ab", "cd", "ef"].map(|w| shaped(&f, w, false).width()).into_iter().fold(0.0, f64::max) + 1.0;
    let wrapped = at("ab cd ef", narrow, Some(1));
    assert_eq!((wrapped.lines.len(), wrapped.truncated), (1, Some(3)));
    assert_eq!(at("ab cd ef", narrow, Some(3)).truncated, None, "three lines fit in three");
    let none = at("ab cd ef", narrow, Some(0));
    assert_eq!((none.lines.len(), none.truncated, none.block_size), (0, Some(0), 0.0));
}

fn line_starts(f: &Font, text: &str, max_inline_size: f64, strategy: BreakStrategy) -> Vec<usize> {
    let opts = LayoutOptions::default().with_max_inline_size(max_inline_size).with_strategy(strategy);
    f.layout(text, &Location::default(), &opts).lines.iter().map(|l| l.bytes.start).collect()
}

// A word no line can hold is cut at the last grapheme that fits, as CSS's `overflow-wrap: anywhere` cuts it, and
// what follows goes on after the rest. The starts are daegun's recorded output for Inter.
#[test]
fn a_word_no_line_can_hold_is_cut_where_the_line_fills() {
    let f = font(INTER);
    let starts = |text: &str, px| line_starts(&f, text, at_1000px(&f, px), BreakStrategy::Greedy);
    let word = "Pneumonoultramicroscopicsilicovolcanoconiosis";
    assert_eq!(starts(word, 3000.0), [0, 4, 8, 14, 20, 26, 32, 37, 42]);
    assert_eq!(starts(&format!("an {word} word"), 3000.0), [0, 3, 7, 11, 17, 23, 29, 35, 40, 45, 49]);
    assert_eq!(starts(&format!("an {word} word"), 5000.0), [0, 3, 10, 20, 31, 40, 49]);
    assert_eq!(starts("TATATATATATATAYAYAYAYAWAWAWAWAWA", 2500.0), [0, 4, 8, 12, 16, 20, 23, 26, 29]);
    assert_eq!(starts("wwwwwwww", 700.0), [0, 1, 2, 3, 4, 5, 6, 7], "a grapheme wider than the line takes one");
    assert_eq!(starts("e\u{301}e\u{301}e\u{301}", 100.0), [0, 3, 6], "a cut split a grapheme");
}

// CSS shapes a cut word "as if the word were not broken": "AVAV" takes 2,486.3, its last V kerned toward the next
// line's A, where alone it takes 2,554.7. Lam-alef's ligature is never split.
#[test]
fn a_cut_word_keeps_the_shaping_it_had_whole() {
    let avav: &[usize] = &[0, 4, 8, 12, 16, 20, 24, 28, 31];
    let salam: &[usize] = &[0, 6, 12, 18, 24, 30];
    for (rel, text, width, greedy, optimal) in [
        (INTER, "AVAVAVAVAVAVAVAVAVAVAVAVAVAVAVAV", 2500.0, avav, &[0, 4, 8, 12, 16, 20, 23, 27, 31][..]),
        (ARABIC, concat!("\u{633}\u{644}\u{627}\u{645}\u{644}\u{627}\u{633}\u{644}\u{627}\u{645}\u{644}\u{627}",
            "\u{633}\u{644}\u{627}\u{645}"), 1500.0, salam, salam),
    ] {
        let f = font(rel);
        let whole = shaped(&f, text, false);
        for (strategy, want) in [(BreakStrategy::Greedy, greedy), (BreakStrategy::Optimal, optimal)] {
            let options = LayoutOptions::default().with_max_inline_size(at_1000px(&f, width)).with_strategy(strategy);
            let layout = f.layout(text, &Location::default(), &options);
            let starts: Vec<usize> = layout.lines.iter().map(|l| l.bytes.start).collect();
            assert_eq!(starts, want, "{rel} {strategy:?}");
            for line in &layout.lines {
                let (from, to) = (line.bytes.start, line.bytes.end);
                let part = |i: &usize| (from..to).contains(&(whole.clusters[*i] as usize));
                let glyphs: Vec<GlyphId> = (0..whole.glyphs.len()).filter(part).map(|i| whole.glyphs[i]).collect();
                let width: f64 = (0..whole.glyphs.len()).filter(part).map(|i| whole.advances[i]).sum();
                assert_eq!(line.runs.iter().flat_map(|r| r.run.glyphs.clone()).collect::<Vec<_>>(), glyphs, "{rel}");
                let drawn = line.inline_size;
                assert!((drawn - width).abs() < 1e-9, "{rel} {:?}: {drawn} drawn, {width} whole", line.bytes);
            }
        }
    }
}

// A word is cut only where no break opportunity lies in the line (CSS Text 3, `overflow-wrap`), so a URL breaks
// after a slash, where UAX #14 lets a line break before a letter, and cuts only what no line holds.
#[test]
fn a_line_breaks_at_an_opportunity_before_it_cuts_a_word() {
    let f = font(INTER);
    let url = "https://example.com/a/very/long/path/that/never/ends";
    let measure = at_1000px(&f, 4200.0);
    assert_eq!(line_starts(&f, url, measure, BreakStrategy::Greedy), [0, 8, 15, 22, 27, 32, 37, 42, 48]);
    assert_eq!(line_starts(&f, url, measure, BreakStrategy::Optimal), [0, 8, 14, 20, 27, 32, 37, 42, 48]);
}

// A caller's segmenter gives SA text its breaks in layout too, and a line ends at one before cutting a word: Thai
// letters cut after the first, on a measure holding three, start their lines at the first and second, not the fourth.
#[test]
fn layout_breaks_sa_text_where_its_segmenter_says() {
    let f = font(INTER);
    let text = "\u{E01}\u{E02}\u{E04}\u{E07}";
    let width = shaped(&f, "\u{E01}\u{E02}\u{E04}", false).width() + 1.0;
    let cut_after_first = |run: &str, out: &mut Vec<usize>| out.extend(run.char_indices().nth(1).map(|(at, _)| at));
    let segmenter = Some(WordSegmenter(&cut_after_first));
    let starts = |segmenter: Option<WordSegmenter>| -> Vec<usize> {
        let opts = LayoutOptions::default().with_max_inline_size(width);
        let opts = segmenter.map_or(opts, |s| opts.with_segmenter(s));
        f.layout(text, &Location::default(), &opts).lines.iter().map(|l| l.bytes.start).collect()
    };
    assert_eq!(starts(segmenter), [0, 3]);
    assert_eq!(starts(None), [0, 9]);
}

// A line is shaped and measured with what stays on it, hanging spaces too: Source Serif's "Y" kerns against its space
// (3,759), and "https://" loses EB Garamond's "/e" kern where it's cut, so 3,000 still holds the first line.
#[test]
fn a_line_is_measured_as_its_ends_are_drawn() {
    let serif = font("source-serif/SourceSerif4Variable-Roman.otf");
    let opts = LayoutOptions::default().with_max_inline_size(3765.0);
    let layout = serif.layout("SAT TOY WAY VAT TOW SAY TAT YAW", &Location::default(), &opts);
    assert_eq!(layout.lines.iter().map(|l| l.bytes.start).collect::<Vec<_>>(), [0, 8, 12, 16, 20, 28]);
    assert_eq!(layout.lines[0].inline_size, 3759.0);

    let garamond = font("eb-garamond/EBGaramond.ttf");
    let url = "https://example.com/a/very/long/path/that/never/ends";
    for strategy in [BreakStrategy::Greedy, BreakStrategy::Optimal] {
        let opts = LayoutOptions::default().with_max_inline_size(3000.0).with_strategy(strategy);
        let layout = garamond.layout(url, &Location::default(), &opts);
        assert!(layout.lines.iter().all(|l| l.inline_size <= 3000.0), "{strategy:?}: a line passes the measure");
    }
    assert_eq!(line_starts(&garamond, url, 3000.0, BreakStrategy::Greedy)[..2], [0, 7]);
}

// A line overflows only where nothing in it could be broken, under either strategy: in Inter on 600 each "w" is
// too wide for a line and gets its own, where one line from "ii" on would overflow just once.
#[test]
fn a_line_overflows_only_with_one_grapheme() {
    let f = font(INTER);
    for strategy in [BreakStrategy::Greedy, BreakStrategy::Optimal] {
        assert_eq!(line_starts(&f, "wwii wwww", at_1000px(&f, 600.0), strategy), [0, 1, 2, 5, 6, 7, 8], "{strategy:?}");
    }
}

// A justified paragraph's last line keeps its natural spaces, so the breaker gives it nothing to shrink. On a
// measure half a space short, which its three spaces could make up by shrinking, it takes two lines, not one.
#[test]
fn a_justified_paragraph_never_shrinks_its_last_line() {
    let f = font(INTER);
    let text = "aaa aaa aaa aaa";
    let natural = f.layout(text, &Location::default(), &LayoutOptions::default()).lines[0].inline_size;
    let measure = natural - shaped(&f, " ", false).width() / 2.0;
    let opts = LayoutOptions::default()
        .with_max_inline_size(measure)
        .with_align(Align::Justify)
        .with_strategy(BreakStrategy::Optimal);
    let layout = f.layout(text, &Location::default(), &opts);
    assert_eq!(layout.lines.iter().map(|l| l.bytes.start).collect::<Vec<_>>(), [0, 12]);
    assert!(layout.lines.iter().all(|l| l.inline_size <= measure));
}

// A glyph cluster is one unit: TestMORXTen's morx makes "A third sentence that goes" a single cluster, so no line
// breaks inside it, and the line draws the glyphs it was shaped with whole.
#[test]
fn no_line_breaks_inside_a_glyph_cluster() {
    let f = font("aat/TestMORXTen.ttf");
    let text = "A third sentence that goes";
    assert_eq!(line_starts(&f, text, 3000.0, BreakStrategy::Greedy), [0]);
    let layout = f.layout(text, &Location::default(), &LayoutOptions::default().with_max_inline_size(3000.0));
    let glyphs: Vec<GlyphId> = layout.lines[0].runs.iter().flat_map(|r| r.run.glyphs.clone()).collect();
    assert_eq!(glyphs, shaped(&f, text, false).glyphs);
}

// UAX #14 breaks between a space and the combining mark after it, inside their grapheme, as LB10 makes the mark AL
// and LB18 breaks after spaces: "xx \u{308}yy zz" in Inter on 1,500 starts its second line with the mark, shaped alone.
#[test]
fn a_line_may_start_with_a_mark_after_a_space() {
    let f = font(INTER);
    assert_eq!(line_starts(&f, "xx \u{308}yy zz", at_1000px(&f, 1500.0), BreakStrategy::Greedy), [0, 3, 8]);
}

// A word cut before the soft hyphen it breaks at keeps its last letter kerned against the hyphen, Source Serif's "a‐"
// 499 rather than 509; no cut falls just before the soft hyphen.
#[test]
fn a_cut_word_ending_at_a_soft_hyphen_kerns_against_it() {
    let f = font("source-serif/SourceSerif4Variable-Roman.otf");
    let width = shaped(&f, "aaa\u{2010}", false).width();
    let layout =
        f.layout("aaaaaaaa\u{AD}bb", &Location::default(), &LayoutOptions::default().with_max_inline_size(width));
    let line = layout.lines.iter().find(|l| l.bytes.end == 10).expect("a line breaks at the soft hyphen");
    let drawn: Vec<f64> = line.runs.iter().flat_map(|r| r.run.advances.clone()).collect();
    assert_eq!(drawn[drawn.len() - 2..], shaped(&f, "a\u{2010}", false).advances[..]);
    assert_eq!(drawn[drawn.len() - 2], 499.0);
}

// Each run's clusters count from its own bytes' start, as it was shaped from there, in wrapped, mixed-direction and
// hyphenated text alike: every cluster starts a character inside the run's bytes.
#[test]
fn a_runs_clusters_count_from_its_bytes() {
    for (rel, text) in [
        (INTER, "Wrapped text a long way past one line, in\u{AD}deed hyphen\u{AD}ated, and on into a third line."),
        (ARABIC, "سلام (a) سلام عليكم ورحمة الله وبركاته abc (د) abc سلام"),
    ] {
        let f = font(rel);
        let opts = LayoutOptions::default().with_max_inline_size(f64::from(f.units_per_em()) * 6.0);
        let layout = f.layout(text, &Location::default(), &opts);
        assert!(layout.lines.len() > 2, "{rel}: the text didn't wrap, so the case is vacuous");
        for run in layout.lines.iter().flat_map(|l| &l.runs) {
            let piece = &text[run.bytes];
            for &cluster in &run.run.clusters {
                let at = cluster as usize;
                assert!(at < piece.len() && piece.is_char_boundary(at), "{rel}: cluster {at} in {piece:?}");
            }
        }
    }
}

// A layout's features reach each run as shaping alone uses them: Inter's `tnum` widens its 1 as in a run, and a repeat
// is served alike. A feature over some bytes holds there alone, through the wrap: only the 1 in range is tabular.
#[test]
fn a_layouts_features_reach_its_runs() {
    use daegun::shape::Feature;
    let f = font(INTER);
    let tag = Tag::new(b"tnum");
    let width = |text: &str, features: &[Feature]| {
        f.shape(text, &Location::default(), &ShapeOptions::default().with_features(features)).width()
    };
    let laid = |text: &str, features: &[Feature]| {
        let opts = LayoutOptions::default().with_features(features);
        f.layout(text, &Location::default(), &opts).lines[0].inline_size
    };
    let all = [Feature::new(tag, 1)];
    assert_ne!(width("111", &all), width("111", &[]), "tnum moves nothing, so the case is vacuous");
    assert_eq!(laid("111", &[]), width("111", &[]));
    assert_eq!(laid("111", &all), width("111", &all));
    assert_eq!(laid("111", &all), width("111", &all), "a repeat, served from the cache");
    assert_eq!(laid("111", &[]), width("111", &[]), "the plain run was served for the featured one, or back");

    // Bytes 4 to 5 of the whole are the first of the second line's 1s, which that line takes alone.
    let second = [Feature { bytes: (4..5).into(), ..Feature::new(tag, 1) }];
    let opts = LayoutOptions::default().with_features(&second).with_max_inline_size(width("111", &[]) * 1.5);
    let layout = f.layout("111 111", &Location::default(), &opts);
    assert_eq!(layout.lines.len(), 2, "the text didn't wrap, so the case is vacuous");
    assert_eq!(layout.lines[0].inline_size, width("111", &[]));
    let local = [Feature { bytes: (0..1).into(), ..Feature::new(tag, 1) }];
    assert_eq!(layout.lines[1].inline_size, width("111", &local));
    assert_ne!(layout.lines[1].inline_size, width("111", &[]));
}

// A measure below 0 lays out as 0 does, and NaN as no measure, greedy or optimal, over paragraphs of breaks and a
// leading space, which a measure below 0 could otherwise cut backward through.
#[test]
fn a_measure_below_0_lays_out_as_0_and_nan_as_none() {
    let f = font(INTER);
    for strategy in [BreakStrategy::Greedy, BreakStrategy::Optimal] {
        let at = |text: &str, measure: f64| {
            let opts = LayoutOptions::default().with_max_inline_size(measure).with_strategy(strategy);
            f.layout(text, &Location::default(), &opts)
        };
        for text in ["\n", " a", "a\n\nb", "one two three"] {
            assert_eq!(at(text, -1.0), at(text, 0.0), "{strategy:?} {text:?}");
            assert_eq!(at(text, f64::NAN), at(text, f64::INFINITY), "{strategy:?} {text:?}");
        }
    }
}

// Tab stops count from each line's start, so a line of tabbed text measures as its text alone does, wherever the
// paragraph wraps it.
#[test]
fn each_tabbed_line_measures_as_its_text_alone() {
    let f = font(INTER);
    let text = "\tab cd\tef gh ij\tk lm\tno pq rs tu\tvw xy z\tab cd ef\tgh";
    let alone = |line: &str| f.layout(line, &Location::default(), &LayoutOptions::default()).lines[0].inline_size;
    for strategy in [BreakStrategy::Greedy, BreakStrategy::Optimal] {
        for measure in [600.0, 1500.0, 3000.0, 5000.0] {
            let opts = LayoutOptions::default().with_max_inline_size(at_1000px(&f, measure)).with_strategy(strategy);
            let laid = f.layout(text, &Location::default(), &opts);
            assert!(laid.lines.len() > 1 || measure > 3000.0, "{strategy:?} at {measure} px");
            for line in &laid.lines {
                assert_eq!(line.inline_size, alone(&text[line.bytes]), "{strategy:?} at {measure} px");
            }
        }
    }
}

// A tab before a comma gives no break, so "WW\t,WWWWWW" is one segment cut at its graphemes; a piece holding the tab
// is measured to its stop, and no line of more than one grapheme passes a measure of three W.
#[test]
fn a_tab_in_a_cut_segment_is_measured_to_its_stop() {
    let f = font(INTER);
    let text = "WW\t,WWWWWW";
    let measure = 3.0 * shaped(&f, "W", false).advances[0];
    for strategy in [BreakStrategy::Greedy, BreakStrategy::Optimal] {
        let opts = LayoutOptions::default().with_max_inline_size(measure).with_strategy(strategy);
        for line in &f.layout(text, &Location::default(), &opts).lines {
            let graphemes = text[line.bytes].chars().count();
            let (bytes, size) = (line.bytes, line.inline_size);
            assert!(size <= measure || graphemes == 1, "{strategy:?}: {bytes:?} is {size} wide");
        }
    }
}

// A tab size so small its stops overflow draws tabs at no width, as 0 does and one so large its interval overflows;
// every alignment stays finite.
#[test]
fn a_tab_size_whose_stops_overflow_draws_tabs_at_no_width() {
    let f = font(INTER);
    for align in [Align::Start, Align::Center, Align::End] {
        let at = |size: f64| {
            let opts = LayoutOptions::default().with_tab_size(size).with_align(align);
            f.layout("a\tb", &Location::default(), &opts)
        };
        assert_eq!([at(1e-310), at(1e306)], [at(0.0), at(0.0)], "{align:?}");
    }
}
