use daegun::{Font, GlyphId, Location, Tag};
use daegun::shape::{Direction, Feature, ShapeOptions};

use super::aat_subset::font;
use daegun::unicode::BaseDirection;

const SOURCE_SERIF: &str = "source-serif/SourceSerif4Variable-Roman.otf";
const INTER: &str = "inter/InterVariable.ttf";
const SCHEHERAZADE: &str = "scheherazade-new/ScheherazadeNew-Regular.ttf";

// Each glyph's id, cluster, advance and whether it's unsafe to break.
fn shaped(f: &Font, text: &str, opts: &ShapeOptions) -> Vec<(u32, u32, f64, bool)> {
    let run = f.shape(text, &Location::default(), opts);
    (0..run.glyphs.len()).map(|i| (run.glyphs[i].0, run.clusters[i], run.advances[i], run.unsafe_to_break[i])).collect()
}

// A right-to-left script's run asked left to right turns unless it reads as a number: digits or regional indicators
// and no letter. A break where a side alone reads otherwise than the whole is unsafe: in "ab12" before either digit,
// as "12" or "2" alone would read as a number, and in "12%" before the %, which alone wouldn't.
#[test]
fn a_break_where_a_side_alone_reads_as_a_number_otherwise_is_unsafe() {
    let f = font(INTER);
    let arabic = daegun::unicode::Script::ARABIC;
    let ltr_arabic = ShapeOptions::default().with_direction(Direction::LeftToRight).with_seed_script(arabic);
    let unsafe_at = |text: &str| shaped(&f, text, &ltr_arabic).iter().map(|g| g.3).collect::<Vec<_>>();
    assert_eq!(unsafe_at("ab12"), [false, true, true, true]);
    assert_eq!(unsafe_at("12%"), [false, false, true]);
}

// Asked left to right, an Arabic run turns and reads backward, meeting the text after it first: lam in "سل" before
// an alef takes the form it has in "سلا", whatever stands before the run, here a beh that would join it.
#[test]
fn a_turned_run_meets_the_text_after_it_first() {
    let f = font(SCHEHERAZADE);
    let lam = |text: &str, after: &str| {
        let ltr = ShapeOptions::default().with_direction(Direction::LeftToRight).with_context("\u{628}", after);
        shaped(&f, text, &ltr).iter().find(|g| g.1 == 2).map(|g| g.0)
    };
    assert_eq!(lam("\u{633}\u{644}", "\u{627}"), lam("\u{633}\u{644}\u{627}", ""));
}

// A feature over bytes start..end holds there alone, and a range counts bytes, é two of them.
#[test]
fn a_feature_holds_over_its_range_of_bytes() {
    let f = font(SOURCE_SERIF);
    let with = |text: &str, features: &[Feature]| {
        shaped(&f, text, &ShapeOptions::default().with_features(features))
    };
    let off = |start: usize, end: usize| Feature { tag: Tag::new(b"liga"), value: 0, bytes: (start..end).into() };
    let at = |(glyph, advance): (u32, f64), cluster: u32, unsafe_to_break| (glyph, cluster, advance, unsafe_to_break);
    let (fi, f_, i_, e) = ((417, 607.0), (33, 384.0), (36, 298.0), (264, 500.0));
    assert_eq!(with("fifi", &[off(0, 2)]), [at(f_, 0, false), at(i_, 1, true), at(fi, 2, false)]);
    assert_eq!(with("fifi", &[off(2, usize::MAX)]), [at(fi, 0, false), at(f_, 2, false), at(i_, 3, true)]);
    let on_again = [Feature::new(Tag::new(b"liga"), 0), Feature { value: 1, ..off(2, 4) }];
    assert_eq!(with("fifi", &on_again), [at(f_, 0, false), at(i_, 1, true), at(fi, 2, false)]);
    let first = [at(e, 0, false), at(f_, 2, true), at(i_, 3, true), at(fi, 4, false)];
    assert_eq!(with("\u{e9}fifi", &[off(2, 4)]), first);
    let second = [at(e, 0, false), at(fi, 2, true), at(f_, 4, false), at(i_, 5, true)];
    assert_eq!(with("\u{e9}fifi", &[off(4, 6)]), second);
}

// A stated direction beats the text's: "(1)" right to left runs backward, its parentheses mirrored, and "ab" bottom
// to top runs down the line backward. Inter at 2,048 units to the em.
#[test]
fn a_stated_direction_shapes_the_run() {
    let f = font(INTER);
    let rtl = ShapeOptions::default().with_direction(Direction::RightToLeft);
    let mirrored = [(1446, 2, 747.0, true), (1342, 1, 833.0, true), (1447, 0, 747.0, false)];
    assert_eq!(shaped(&f, "(1)", &rtl), mirrored);

    let btt = ShapeOptions::default().with_direction(Direction::BottomToTop);
    let run = f.shape("ab", &Location::default(), &btt);
    let glyph = |i: usize| (run.glyphs[i].0, run.clusters[i], run.offsets[i].into(), run.advances[i]);
    assert_eq!([glyph(0), glyph(1)], [(578, 1, (-627.0, -1972.0), 2478.0), (507, 0, (-575.0, -1792.0), 2478.0)]);
}

// A character the font lacks is drawn with the glyph asked for, and so is a selector it can't apply.
#[test]
fn not_found_glyphs_stand_for_what_the_font_lacks() {
    let f = font(INTER);
    let (a, b) = ((507, 0, 1150.0, false), |at| (578, at, 1254.0, false));
    let missing = ShapeOptions::default().with_not_found_glyph(GlyphId(5));
    assert_eq!(shaped(&f, "a\u{1E6C0}b", &missing), [a, (5, 1, 1413.0, false), b(5)]);
    assert_eq!(shaped(&f, "a\u{1E6C0}b", &ShapeOptions::default()), [a, (0, 1, 1344.0, false), b(5)]);
    let selector = ShapeOptions::default().with_not_found_variation_selector_glyph(GlyphId(5));
    assert_eq!(shaped(&f, "a\u{FE00}b", &selector), [a, (5, 0, 0.0, false), b(4)]);
    assert_eq!(shaped(&f, "a\u{FE00}b", &ShapeOptions::default()), [a, (1777, 0, 0.0, false), b(4)]);
}

// Bidi gives each run its direction, so a direction in the options passed to shape_bidi changes nothing.
#[test]
fn bidi_runs_keep_the_direction_bidi_gives_them() {
    let f = font(INTER);
    let ltr = ShapeOptions::default().with_direction(Direction::LeftToRight);
    let runs = |opts: &ShapeOptions| {
        let runs = f.shape_bidi("a (\u{5D0}\u{5D1})", &Location::default(), BaseDirection::RightToLeft, opts);
        runs.into_iter().map(|r| (r.level, r.bytes, r.run.glyphs.clone())).collect::<Vec<_>>()
    };
    assert_eq!(runs(&ltr), runs(&ShapeOptions::default()));
}

// Dae1 and DAE1 run before the script's features and Dae2 and DAE2 after, so only a later stage reaches
// beh's init form.
#[test]
fn the_stage_features_run_before_and_after_the_scripts() {
    use super::aat_shaping::cmap;
    use super::aat_subset::{carets_with, glyph_ids, F, I, L};
    use super::layout_build::{layout, lookup, single, to};
    for (stage, after_init) in [(b"Dae1", false), (b"DAE1", false), (b"Dae2", true), (b"DAE2", true)] {
        let lookups = vec![lookup(1, 0, vec![to(single(F, I))]), lookup(1, 0, vec![to(single(I, L))])];
        let gsub = layout(b"DFLT", &[(b"init", &[0]), (stage, &[1])], lookups);
        let f = carets_with(&[("GSUB", gsub), ("cmap", cmap(&[(0x66, 0x66, I), (0x628, 0x628, F)]))]);
        let arabic = f.shape("\u{628}\u{628}", &Location::default(), &ShapeOptions::default());
        let (init, name) = (if after_init { L } else { I }, String::from_utf8_lossy(stage));
        let want = glyph_ids(&[F, init]);
        assert_eq!((arabic.glyphs.as_slice(), arabic.clusters.as_slice()), (&want[..], &[2, 0][..]), "{name}");
        assert_eq!(
            f.shape("f", &Location::default(), &ShapeOptions::default()).glyphs.as_slice(),
            [GlyphId(L.into())],
            "{name}"
        );
    }
}

// One run reused across fonts, texts and options, long runs before short and reports asked for then not, answers as
// shape does each time, so nothing of an earlier run is left in it.
#[test]
fn a_run_shaped_into_answers_as_shape_does() {
    let reports = ShapeOptions::default().with_report_unsafe_to_concat(true).with_report_tatweel_positions(true);
    let liga = [Feature::new(Tag::new(b"liga"), 0)];
    let options = [
        reports,
        ShapeOptions::default(),
        ShapeOptions::default().with_features(&liga),
        ShapeOptions::default().with_direction(Direction::RightToLeft),
        ShapeOptions::default().with_direction(Direction::TopToBottom),
    ];
    let texts = ["office affluent ffi, the long line first", "السلام عليكم", "a", ""];
    let mut run = daegun::shape::ShapedRun::default();
    for path in [INTER, SOURCE_SERIF, "scheherazade-new/ScheherazadeNew-Regular.ttf"] {
        let f = font(path);
        for opts in &options {
            for text in texts {
                f.shape_into(text, &Location::default(), opts, &mut run);
                assert_eq!(run, *f.shape(text, &Location::default(), opts), "{path} {text:?} {opts:?}");
            }
        }
    }
}

// A feature written out reads back as itself, its bytes over the whole text, from 0, to the end, one alone (the first
// and the last), and a tag padded with a space; a direction too, by its first letter in either case.
#[test]
fn features_and_directions_read_back_as_written() {
    let kern = Feature::new(Tag::new(b"kern"), 1);
    let ranged = |start: usize, end: usize, value| Feature { bytes: (start..end).into(), value, ..kern };
    let features = [
        kern, ranged(0, usize::MAX, 0), ranged(0, usize::MAX, 7), ranged(3, 5, 2), ranged(0, 5, 1),
        ranged(3, usize::MAX, 0), ranged(0, 1, 1), ranged(4, 5, 0), ranged(usize::MAX - 1, usize::MAX, 3),
        ranged(usize::MAX, 0, 1),
        Feature::new(Tag::new(b"lao "), 1), Feature::new(Tag::new(b"a\"b "), 0),
    ];
    for feature in features {
        let text = feature.to_string();
        assert_eq!(text.parse::<Feature>(), Ok(feature), "{text}");
    }
    let written: Vec<String> = features[..4].iter().map(ToString::to_string).collect();
    assert_eq!(written, ["kern", "-kern", "kern=7", "kern[3:5]=2"]);
    for direction in [Direction::LeftToRight, Direction::RightToLeft, Direction::TopToBottom, Direction::BottomToTop] {
        assert_eq!(direction.to_string().parse(), Ok(direction));
        assert_eq!(direction.to_string().to_uppercase().parse(), Ok(direction));
    }
    assert_eq!("left".parse(), Ok(Direction::LeftToRight));
    for bad in ["", " ", "x", "up"] {
        assert!(bad.parse::<Direction>().is_err(), "{bad:?}");
    }
}
