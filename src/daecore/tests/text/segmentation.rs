use std::sync::Arc;

use daegun::layout::LayoutOptions;
use daegun::shape::{ClusterLevel, Ignorables, ShapeOptions, ShapedRun, Shaper};
use daegun::unicode::{
    BaseDirection, BidiParagraph, GeneralCategory, Script, ScriptDirection, ScriptRun, VerticalOrientation, VisualRun,
    WordSegmenter, general_category, grapheme_boundaries, line_break_opportunities,
    script_runs, vertical_form, vertical_orientation, word_boundaries,
};
use daegun::{Font, GlyphClass, GlyphId, Location, Tag};

fn paragraphs(p: &BidiParagraph) -> Vec<(usize, usize, u8)> {
    p.paragraphs.iter().map(|r| (r.start, r.end, r.level)).collect()
}

// Each paragraph finds its own level, a CR LF ending one, and is reordered as a line of its own (UAX #9 P1-P3, L1,
// L2), as byte offsets: a level for each byte, each Hebrew letter two, the separator at its paragraph's level.
#[test]
fn each_paragraph_takes_its_own_level() {
    let p = BidiParagraph::new("\u{5D0}\u{5D1}\u{5D2}\nabc", BaseDirection::Auto);
    let levels = [1, 1, 1, 1, 1, 1, 1, 0, 0, 0];
    assert_eq!((&p.levels[..], &p.visual_order[..]), (&levels[..], &[6, 4, 2, 0, 7, 8, 9][..]));
    assert_eq!(paragraphs(&p), [(0, 7, 1), (7, 10, 0)]);
    let crlf = BidiParagraph::new("\u{5D0}\r\nb", BaseDirection::Auto);
    assert_eq!((paragraphs(&crlf), crlf.levels), (vec![(0, 4, 1), (4, 5, 0)], vec![1, 1, 1, 1, 0]));
    let stated = BidiParagraph::new("ab\u{2029}cd", BaseDirection::RightToLeft);
    assert_eq!((paragraphs(&stated), stated.visual_order), (vec![(0, 5, 1), (5, 7, 1)], vec![2, 0, 1, 5, 6]));
}

// A line's trailing space goes back to its own paragraph's level, left to right after a right-to-left paragraph.
#[test]
fn a_line_takes_the_level_of_its_paragraph() {
    let text = "\u{5D0}\u{5D1}\u{5D2}\nab ";
    let para = BidiParagraph::new(text, BaseDirection::Auto);
    let runs: Vec<_> =
        para.visual_runs(text, 7..10).into_iter().map(|r| (core::ops::Range::from(r.bytes), r.level)).collect();
    assert_eq!(runs, [(7..10, 0)]);
}

// Two right-to-left paragraphs shape as two runs, each reversed alone, never one run across the break.
#[test]
fn bidi_runs_stay_within_their_paragraph() {
    let f = super::layout::font("test-fixtures/carets.ttf");
    let runs: Vec<_> = f
        .shape_bidi(
            "\u{5D0}\u{5D1}\n\u{5D2}\u{5D3}",
            &Location::default(),
            BaseDirection::Auto,
            &ShapeOptions::default(),
        )
        .into_iter()
        .map(|r| (core::ops::Range::from(r.bytes), r.level))
        .collect();
    assert_eq!(runs, [(0..5, 1), (5..9, 1)]);
}

// A line separator breaks the line but not the paragraph, so the Hebrew after it keeps the paragraph's left-to-right
// direction and starts at the left edge.
#[test]
fn a_line_separator_keeps_the_paragraphs_direction() {
    let f = super::layout::font("test-fixtures/carets.ttf");
    let options = LayoutOptions::default().with_max_inline_size(5000.0);
    let layout = f.layout("abc\u{2028}\u{5D0}\u{5D1}\u{5D2}", &Location::default(), &options);
    let starts: Vec<f64> = layout.lines.iter().map(|l| l.runs[0].offset.x).collect();
    assert_eq!(starts, [0.0, 0.0]);
}

#[test]
fn a_line_is_ordered_per_line_and_not_by_slicing_the_paragraph() {
    let text = "\u{633}\u{644}\u{627}\u{645} abc  def \u{633}\u{644}\u{627}\u{645}";
    let para = BidiParagraph::new(text, BaseDirection::Auto);
    assert_eq!(para.base_level, 1, "the fixture text is meant to be a right-to-left paragraph");

    // A run's characters' starts in the order they're drawn: an odd level's last to first.
    let drawn = |r: daegun::unicode::VisualRun| -> Vec<usize> {
        let starts = text[core::ops::Range::from(r.bytes)].char_indices().map(|(at, _)| r.bytes.start + at);
        if r.level % 2 == 1 { starts.rev().collect() } else { starts.collect() }
    };
    // The four Arabic letters, two bytes each, " abc ".
    let (start, end) = (0, 13);
    let per_line: Vec<usize> = para.visual_runs(text, start..end).into_iter().flat_map(drawn).collect();
    let sliced: Vec<usize> = para.visual_order.iter().copied()
        .filter(|&c| c >= start && c < end).collect();

    assert_ne!(per_line, sliced, "slicing agreed, so the fixture no longer exercises L1");
    assert_eq!(
        per_line.first(), Some(&12),
        "L1 did not move the line's trailing space to the reading end: {per_line:?}",
    );
    assert_eq!(sliced.last(), Some(&0), "sanity: the sliced order still ends at the paragraph start");

    let mut seen = per_line.clone();
    seen.sort_unstable();
    let starts: Vec<usize> = text[start..end].char_indices().map(|(at, _)| start + at).collect();
    assert_eq!(seen, starts, "a line's chars are not partitioned exactly");

    let all = text.len();
    let whole: Vec<usize> = para.visual_runs(text, 0..all).into_iter().flat_map(drawn).collect();
    assert_eq!(whole, para.visual_order, "one line should be the paragraph order");

    assert!(para.visual_runs(text, 5..5).is_empty(), "an empty range returned runs");
    assert!(para.visual_runs(text, 0..9_999).is_empty(), "a range past the text returned runs");
}

#[test]
fn script_runs_split_where_the_script_changes() {
    let text = "سلام abc 日本 हिन्दी";
    let runs = script_runs(text);
    let names: Vec<&str> = runs.iter().map(|r| r.script.name()).collect();
    assert_eq!(names, ["Arabic", "Latin", "Han", "Devanagari"], "wrong split: {names:?}");

    assert_eq!(runs[0].bytes.start, 0, "the first run does not start at 0");
    assert_eq!(runs.last().unwrap().bytes.end, text.len(), "the last run does not reach the end");
    assert!(runs.windows(2).all(|w| w[0].bytes.end == w[1].bytes.start), "runs do not meet");

    assert_eq!(script_runs("ab, cd").len(), 1, "a comma between Latin words split the run");
    assert!(script_runs("").is_empty(), "empty text yielded runs");

    assert!(script_runs(", ")[0].script.is_context_dependent(), "Common is not context-dependent");
    assert!(!script_runs("abc")[0].script.is_context_dependent(), "Latin is context-dependent");
}

fn runs(text: &str) -> Vec<(usize, usize, &'static str)> {
    script_runs(text).iter().map(|r| (r.bytes.start, r.bytes.end, r.script.name())).collect()
}

// A bracket closes in the script it opened in, an unmatched closer dropping the openers above its match. Runs as
// byte offsets.
#[test]
fn brackets_close_in_the_script_they_opened_in() {
    assert_eq!(runs("gamma (\u{3B3}) is"), [(0, 7, "Latin"), (7, 9, "Greek"), (9, 13, "Latin")]);
    assert_eq!(runs("abc (\u{5D0}\u{5D1}\u{5D2}) def"), [(0, 5, "Latin"), (5, 11, "Hebrew"), (11, 16, "Latin")]);
    assert_eq!(
        runs("\u{65E5}\u{672C}\u{300C}\u{410}\u{411}(\u{3B3})\u{300D}\u{65E5}"),
        [(0, 9, "Han"), (9, 14, "Cyrillic"), (14, 16, "Greek"), (16, 17, "Cyrillic"), (17, 23, "Han")],
    );
    assert_eq!(runs("(\u{5D0}[b)c"), [(0, 4, "Hebrew"), (4, 5, "Latin"), (5, 6, "Hebrew"), (6, 7, "Latin")]);
}

// UAX #24 §5.3: a Common character continues only runs its Script_Extensions name, narrowing a run with no script
// yet to the largest left. §5.2: a mark never leaves its base, a Hebrew point on a Latin letter included.
#[test]
fn script_extensions_limit_the_runs_a_character_continues() {
    assert_eq!(runs("abc\u{30FC}"), [(0, 3, "Latin"), (3, 6, "Hiragana")]);
    assert_eq!(runs("\u{65E5}\u{672C}\u{30FC}"), [(0, 6, "Han"), (6, 9, "Hiragana")]);
    assert_eq!(runs("a\u{964}"), [(0, 1, "Latin"), (1, 4, "Devanagari")]);
    assert_eq!(runs("abc \u{60C}"), [(0, 4, "Latin"), (4, 6, "Arabic")]);
    assert_eq!(runs("a\u{3002}"), [(0, 1, "Latin"), (1, 4, "Han")]);
    assert_eq!(runs("\u{640}"), [(0, 2, "Arabic")]);
    assert_eq!(runs("\u{640}\u{60C}\u{710}"), [(0, 6, "Syriac")]);
    assert_eq!(runs("a\u{5B0}b"), [(0, 4, "Latin")]);
    assert_eq!(runs("\u{25CC}\u{5B0}"), [(0, 5, "Hebrew")]);
    assert_eq!(runs("abc\u{1CD0}"), [(0, 6, "Latin")]);
}

// Regional indicators pair from the start of each run of them, so other text between two runs starts the count
// again: one flag's halves, then a half alone. Each indicator is four bytes.
#[test]
fn regional_indicators_pair_again_after_other_text() {
    let text = "\u{1F1E6}a\u{1F1E7}\u{1F1E8}\u{1F1E9}";
    assert_eq!(grapheme_boundaries(text), [0, 4, 5, 13, 17]);
    assert_eq!(word_boundaries(text), [0, 4, 5, 13, 17]);
}

fn breaks(text: &str) -> Vec<usize> {
    line_break_opportunities(text, None).iter().map(|b| b.at).collect()
}

// Text without spaces breaks between the words its script's list finds: UDHR Article 1 in Thai, Lao, Khmer and Burmese,
// daegun's recorded byte offsets, none before the mark after a Thai repeat mark, which LB9 forbids.
#[cfg(feature = "dictionaries")]
#[test]
fn sa_text_breaks_between_the_words_of_its_list() {
    assert_eq!(breaks("มนุษย์ทั้งหลายเกิดมามีอิ"), [18, 30, 42, 54, 60, 72]);
    assert_eq!(breaks("ມະນຸດເກີດມາມີສິດເສລີພາບ "), [15, 27, 33, 39, 48, 70]);
    assert_eq!(breaks("មនុស្សទាំងអស់ កើតមកមានសេ"), [18, 40, 55, 64, 70]);
    assert_eq!(breaks("လူတိုင်းသည် တူညီ လွတ်လပ်"), [6, 24, 34, 47, 68]);
    assert_eq!(breaks("ศัลๆํแสมีด"), [15, 21, 30]);
}

// Marks LB9 folds into the preceding character belong to no word, so where the rules break before a run's first letter
// its words start there: after Khmer's camnuc pii kuuh and marks, "មនុស្សទាំងអស់" divides as alone, "ទឞ" too short to.
#[cfg(feature = "dictionaries")]
#[test]
fn a_run_after_a_rule_break_is_divided_from_its_first_letter() {
    assert_eq!(breaks("\u{1782}\u{17D6}\u{17C7}\u{17C4}មនុស្សទាំងអស់"), [12, 30, 51]);
    assert_eq!(breaks("\u{1782}\u{17D6}\u{17C7}\u{17C4}\u{1791}\u{179E}"), [12, 18]);
}

// Without the word lists, text written without spaces breaks only where other text would.
#[cfg(not(feature = "dictionaries"))]
#[test]
fn sa_text_without_word_lists_breaks_only_where_other_text_would() {
    assert_eq!(breaks("มนุษย์ทั้งหลายเกิดมามีอิ"), [72]);
    assert_eq!(breaks("มนุษย์ ทั้งหลาย"), [19, 43]);
}

// A caller's segmenter gets each SA run, its offsets counting only between combining sequences: the second character in
// "\u{E01}\u{E34}" goes, as do a repeat, the run's end, offsets past it and 0 after "a".
#[test]
fn a_word_segmenter_divides_each_sa_run() {
    let runs = std::sync::Mutex::new(Vec::new());
    let segment = |run: &str, out: &mut Vec<usize>| {
        runs.lock().unwrap().push(run.to_string());
        let char_at = |k| run.char_indices().nth(k).map_or(run.len(), |(at, _)| at);
        out.extend([0, char_at(1), char_at(2), run.len() + 9, char_at(2), usize::MAX]);
    };
    let at = |text| -> Vec<usize> {
        line_break_opportunities(text, Some(WordSegmenter(&segment))).iter().map(|b| b.at).collect()
    };
    assert_eq!(at("\u{E01}\u{E34}\u{E02}\u{E04} ab \u{E87}\u{E8A}"), [6, 13, 16, 19, 22]);
    assert_eq!(at("\u{E01}\u{E02}\u{E87}\u{E8A}"), [3, 6, 12]);
    assert_eq!(at("a\u{E01}\u{E02}"), [4, 7]);
    assert_eq!(
        runs.into_inner().unwrap(),
        ["\u{E01}\u{E34}\u{E02}\u{E04}", "\u{E87}\u{E8A}", "\u{E01}\u{E02}\u{E87}\u{E8A}", "\u{E01}\u{E02}"]
    );
}

// A segmenter gets a run whose first mark is folded into the character before it from its first letter on where the
// rules break before that letter, as "中" does and "a" doesn't.
#[test]
fn a_segmenter_gets_a_run_after_a_rule_break_from_its_first_letter() {
    let runs = std::sync::Mutex::new(Vec::new());
    let segment = |run: &str, out: &mut Vec<usize>| {
        runs.lock().unwrap().push(run.to_string());
        out.extend(run.char_indices().nth(1).map(|(at, _)| at));
    };
    let at = |text| -> Vec<usize> {
        line_break_opportunities(text, Some(WordSegmenter(&segment))).iter().map(|b| b.at).collect()
    };
    assert_eq!(at("中\u{E31}\u{E01}\u{E02}"), [6, 9, 12]);
    assert_eq!(at("a\u{E31}\u{E01}\u{E02}"), [4, 10]);
    assert_eq!(runs.into_inner().unwrap(), ["\u{E01}\u{E02}", "\u{E31}\u{E01}\u{E02}"]);
}

#[test]
fn vertical_orientation_is_answerable() {
    assert_eq!(vertical_orientation('日'), VerticalOrientation::Upright, "CJK stands upright in vertical text");
    assert_eq!(vertical_orientation('A'), VerticalOrientation::Rotated, "Latin lies sideways in vertical text");
    assert_eq!(vertical_orientation('\u{3001}'), VerticalOrientation::TransformedUpright, "the comma takes its form");
    assert_eq!(vertical_orientation('\u{30FC}'), VerticalOrientation::TransformedRotated, "the long vowel mark turns");

    assert_eq!(vertical_form('\u{3001}'), Some('\u{FE11}'), "the ideographic comma has a vertical form");
    assert_eq!(vertical_form('A'), None, "a letter has no vertical form");
    assert_eq!(vertical_form(','), None, "an ASCII comma is rotated, not substituted");

    assert_eq!(general_category(' '), GeneralCategory::SpaceSeparator);
    assert_eq!(general_category('A'), GeneralCategory::UppercaseLetter);
    assert_ne!(general_category('\u{0301}'), GeneralCategory::UppercaseLetter, "a mark is not a letter");
}

#[test]
fn a_script_can_be_asked_which_way_it_runs_and_what_a_font_calls_it() {
    let script_of = |text: &str| script_runs(text)[0].script;

    assert_eq!(script_of("سلام").direction(), ScriptDirection::RightToLeft, "Arabic runs right to left");
    assert_eq!(script_of("עברית").direction(), ScriptDirection::RightToLeft, "Hebrew runs right to left");
    assert_eq!(script_of("abc").direction(), ScriptDirection::LeftToRight, "Latin runs left to right");
    assert_eq!(script_of("日本").direction(), ScriptDirection::LeftToRight, "Han runs left to right");
    assert_eq!(script_of("ᚠᚢᚦ").direction(), ScriptDirection::LeftToRight, "Runic's letters are of Bidi class L");

    let tags = |text: &str| -> Vec<String> { script_of(text).opentype_tags().map(|t| t.to_string()).collect() };
    assert_eq!(tags("हिन्दी"), ["dev3", "dev2", "deva"], "Devanagari's tags are wrong or out of order");
    assert_eq!(tags("မြန်မာ"), ["mym2", "mymr"], "Myanmar gained a mym3");
    assert_eq!(tags("abc"), ["latn"]);
    assert_eq!(tags(", "), ["DFLT"], "Common should yield DFLT");
}

#[allow(unused_imports, reason = "each name resolving is what is being tested")]
#[test]
fn every_name_the_daeshaper_rounds_added_is_reachable() {
    let f = Font::from_bytes(&std::fs::read(
        format!("{}/eb-garamond/EBGaramond.ttf", crate::FONTS)).unwrap()).unwrap();
    let _: Option<GlyphClass> = f.glyph_class(GlyphId(1));
    let _: Option<std::num::NonZeroU32> = f.mark_attachment_class(GlyphId(1));
    let _: Vec<Tag> = f.script_tags();
    let _: Vec<Tag> = f.language_system_tags(Tag::new(b"latn"));
    let _: Vec<Tag> = f.feature_tags(None, None);
    let p: BidiParagraph = BidiParagraph::new("abc", BaseDirection::Auto);
    let _: Vec<VisualRun> = p.visual_runs("abc", 0..3);
    let r: Vec<ScriptRun> = script_runs("abc");
    let s: Script = r[0].script;
    let _: (&str, ScriptDirection, Vec<Tag>, bool) =
        (s.name(), s.direction(), s.opentype_tags().collect(), s.is_context_dependent());
    let _: GeneralCategory = general_category('a');
    let _: VerticalOrientation = vertical_orientation('日');
    let _: Option<char> = vertical_form('\u{3001}');
    let _ = (grapheme_boundaries("a"), word_boundaries("a"), line_break_opportunities("a", None));
    let options = ShapeOptions::default()
        .with_report_unsafe_to_concat(true).with_report_tatweel_positions(true)
        .with_ignorables(Ignorables::Remove).with_suppress_dotted_circle(true)
        .with_invisible_glyph(GlyphId(1)).with_cluster_level(ClusterLevel::Characters);
    let run: Arc<ShapedRun> = f.shape("hi", &Location::default(), &options);
    let _: (&Vec<bool>, &Vec<bool>, bool, bool, Shaper, &str) = (
        &run.unsafe_to_concat, &run.safe_to_insert_tatweel,
        run.complete, run.has_broken_syllable, run.shaper, run.shaper.name(),
    );
    let _: (bool, bool) = (ClusterLevel::Characters.is_monotone(), ClusterLevel::Graphemes.is_graphemes());
}
