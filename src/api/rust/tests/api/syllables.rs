use daegun::{Font, Location};

use super::FONTS;
use daegun::shape::ShapeOptions;

fn font(rel: &str) -> Font {
    Font::from_vec(std::fs::read(format!("{FONTS}/{rel}")).expect("the fixture font"), 0).expect("it parses")
}

type Runs<T> = &'static [(T, usize)];

// Each value with how many times it repeats in a row.
fn runs<T: Copy + PartialEq>(values: &[T]) -> Vec<(T, usize)> {
    let mut out: Vec<(T, usize)> = Vec::new();
    for &v in values {
        match out.last_mut() {
            Some((last, n)) if *last == v => *n += 1,
            _ => out.push((v, 1)),
        }
    }
    out
}

// A syllable longer than 64 glyphs is reordered 64 at a time: a pre-base vowel past the first 64 moves within its
// own part, and the parts' clusters merge apart.
#[test]
fn a_long_syllable_is_reordered_64_glyphs_at_a_time() {
    let deva = font("noto-devanagari/NotoSansDevanagari.ttf");
    let myanmar = font("noto-myanmar/NotoSansMyanmar.ttf");
    let cases: [(&Font, String, Runs<u32>, Runs<u32>); 3] = [
        (&deva, format!("{}क", "क्".repeat(40)), &[(232, 31), (56, 1), (103, 1), (232, 8), (56, 1)], &[]),
        (
            &deva,
            format!("र्{}कि", "क्".repeat(33)),
            &[(232, 30), (56, 1), (103, 1), (556, 1), (232, 2), (56, 1), (506, 1)],
            &[(0, 37)],
        ),
        (
            &myanmar,
            format!("က{}ေ", "္က".repeat(40)),
            &[(4, 1), (212, 2), (211, 29), (380, 1), (372, 1), (4, 1), (212, 2), (211, 6)],
            &[(0, 33), (192, 10)],
        ),
    ];
    for (i, (f, text, glyphs, clusters)) in cases.into_iter().enumerate() {
        let run = f.shape(&text, &Location::default(), &ShapeOptions::default());
        assert_eq!(runs(&run.glyphs.iter().map(|g| g.0).collect::<Vec<_>>()), glyphs, "case {i}");
        if !clusters.is_empty() {
            assert_eq!(runs(&run.clusters), clusters, "case {i}");
        }
    }
}

// 300 syllables of KA and its i matra, each kept apart: were the 256th to take the 255th's serial, the two would
// read as one syllable and its matra would move into the one before.
#[test]
fn neighboring_syllables_never_share_a_serial() {
    let run = font("noto-devanagari/NotoSansDevanagari.ttf").shape(
        &"कि".repeat(300),
        &Location::default(),
        &ShapeOptions::default(),
    );
    let clusters: Vec<u32> = (0..300).flat_map(|k| [6 * k, 6 * k]).collect();
    let glyphs: Vec<u32> = run.glyphs.iter().map(|g| g.0).collect();
    assert_eq!((glyphs.as_slice(), run.clusters.as_slice()), (&[545, 56].repeat(300)[..], &clusters[..]));
}

// Superscript two and three are syllable modifiers that follow their base, which the Khmer machine has no column
// for, so each stands apart rather than going unscanned into one syllable, and each is safe to break.
#[test]
fn a_category_the_khmer_grammar_never_names_is_a_cluster_of_its_own() {
    let khmer = font("noto-khmer/NotoSansKhmer.ttf");
    let shaped = |text: &str| {
        let run = khmer.shape(text, &Location::default(), &ShapeOptions::default());
        (run.glyphs.iter().map(|g| g.0).collect::<Vec<_>>(), run.clusters.to_vec(), run.unsafe_to_break.to_vec())
    };
    assert_eq!(shaped("ក²³"), (vec![49, 0, 0], vec![0, 3, 5], vec![false; 3]));
    assert_eq!(shaped("²³ក"), (vec![0, 0, 49], vec![0, 2, 4], vec![false; 3]));
}

// Microsoft's Khmer syllable stacks two subscripts under its base, and real text writes one more after its vowel, so
// a fourth coeng opens a broken cluster on a dotted circle.
#[test]
fn a_khmer_syllable_takes_three_subscripts_at_most() {
    let khmer = font("noto-khmer/NotoSansKhmer.ttf");
    let run = khmer.shape("ក្ក្ក្ក្ក", &Location::default(), &ShapeOptions::default());
    assert_eq!(run.glyphs.iter().map(|g| g.0).collect::<Vec<_>>(), [49, 183, 183, 183, 384, 183]);
}
