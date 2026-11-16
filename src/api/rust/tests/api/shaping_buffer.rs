use daegun::{Font, GlyphId, Location};

use super::aat_subset::{carets_with, glyph_ids, u16s, F, I, L};
use super::cmap_reading::{cmap, format4};
use super::layout_lookups::layout_lookup;
use super::FONTS;
use daegun::shape::ShapeOptions;

// carets.ttf whose ccmp takes f to `sequence`, by a multiple substitution.
fn f_to(sequence: &[u16]) -> Font {
    let subtable = [u16s(&[1, 8, 1, 14, 1, 1, F, sequence.len() as u16]), u16s(sequence)].concat();
    carets_with(&[("GSUB", layout_lookup(b"ccmp", 2, &[subtable]))])
}

// f grows to f l before 39 i's the lookup passes over at once, and the run keeps all 41 glyphs, none lost to growth.
#[test]
fn an_expansion_early_in_a_run_keeps_every_glyph_after_it() {
    let run = f_to(&[F, L]).shape(&format!("f{}", "i".repeat(39)), &Location::default(), &ShapeOptions::default());
    let glyphs = glyph_ids(&[[F, L].as_slice(), &[I; 39]].concat());
    let clusters: Vec<u32> = [0].into_iter().chain(0..40).collect();
    assert_eq!((run.glyphs.as_slice(), run.clusters.as_slice(), run.complete), (&glyphs[..], &clusters[..], true));
}

// é and an acute in a font with e, the acute and i but no é: é decomposes, and the 39 i's after it go out at once.
#[test]
fn a_decomposition_early_in_a_run_keeps_every_glyph_after_it() {
    let f = carets_with(&[("cmap", cmap(&[(3, 1, &format4(&[(0x65, F), (0x69, I), (0x301, L)]))]))]);
    let run = f.shape(&format!("\u{e9}\u{301}{}", "i".repeat(39)), &Location::default(), &ShapeOptions::default());
    let glyphs = glyph_ids(&[[F, L, L].as_slice(), &[I; 39]].concat());
    let clusters: Vec<u32> = [0, 0, 0].into_iter().chain(4..43).collect();
    assert_eq!((run.glyphs.as_slice(), run.clusters.as_slice(), run.complete), (&glyphs[..], &clusters[..], true));
}

// A run of two glyphs may hold the least a run holds, 65,536, so each f of two grows to 32,768 and fills it; at 32,769
// the second lookup would pass it, so it fails and the run keeps what it held.
#[test]
fn a_run_never_passes_its_limit() {
    let shaped = |n: usize| {
        let run = f_to(&vec![F; n]).shape("ff", &Location::default(), &ShapeOptions::default());
        (run.glyphs.len(), run.glyphs.iter().all(|&g| g == GlyphId(F.into())), run.complete)
    };
    assert_eq!([32_768, 32_769].map(shaped), [(65_536, true, true), (2, true, false)]);
}

// 500 f's of 200 glyphs each make 100,000, within 256 a glyph, so the run holds them whole.
#[test]
fn a_run_grows_as_far_as_its_budget_lets_it() {
    let run = f_to(&[F; 200]).shape(&"f".repeat(500), &Location::default(), &ShapeOptions::default());
    assert_eq!((run.glyphs.len(), run.complete), (100_000, true));
}

// The budget is counted in 64 bits, so 524,288 and 524,290 characters both shape whole, morx doubling each.
#[test]
fn a_long_runs_budget_never_turns_negative() {
    let bytes = std::fs::read(format!("{FONTS}/aat/TestMORXThirtythree.ttf")).expect("the fixture font");
    let f = Font::from_vec(bytes, 0).expect("it parses");
    let shaped = |k: usize| {
        let run = f.shape(&"ha".repeat(k), &Location::default(), &ShapeOptions::default());
        (run.glyphs.len(), run.complete)
    };
    assert_eq!([262_144, 262_145].map(shaped), [(1_048_576, true), (1_048_580, true)]);
}

// 524,290 Thai SARA AMs each split in two, nikhahit and sara aa, within the room the run has.
#[test]
fn a_long_run_of_sara_am_splits_whole() {
    let bytes = std::fs::read(format!("{FONTS}/test-fixtures/carets.ttf")).expect("the fixture font");
    let run = Font::from_vec(bytes, 0).expect("it parses").shape(
        &"\u{e33}".repeat(524_290),
        &Location::default(),
        &ShapeOptions::default(),
    );
    assert_eq!((run.glyphs.len(), run.complete), (1_048_580, true));
}
