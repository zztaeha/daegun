// Hebrew, after Microsoft's Hebrew script development spec: marks positioned only by a font's Hebrew GPOS, a
// presentation form composed where the font can't place marks, and a meteg kept after the vowel it was typed after.
use core::ops::Range;

use super::{Model, Shaper};
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::run::{Glyph, Run};
use crate::daecore::daeshaper::unicode;

pub(crate) struct Hebrew;

pub(crate) const MODEL: Hebrew = Hebrew;

// Shin and sin with their dot, then with a dagesh too.
const WITH_DAGESH: [(char, char); 2] = [('\u{FB2A}', '\u{FB2C}'), ('\u{FB2B}', '\u{FB2D}')];
const DAGESH: char = '\u{05BC}';

// The mark classes the meteg rule reads, as `mark_class` gives Hebrew's points: patah and qamats, then sheva and
// hiriq, then meteg and any mark below.
const WIDE_VOWELS: [u8; 2] = [20, 21];
const NARROW_VOWELS: [u8; 2] = [22, 23];
const METEG_OR_BELOW: [u8; 2] = [25, 220];

impl Model for Hebrew {
    fn kind(&self) -> Shaper {
        Shaper::Hebrew
    }

    fn gpos_script(&self) -> Option<Tag> {
        Some(Tag::new(b"hebr"))
    }

    // A font that positions marks keeps them apart; one that can't is better served by the presentation forms
    // composition excludes.
    fn compose(&self, a: char, b: char, marks_positioned: bool) -> Option<char> {
        unicode::compose(a, b).or_else(|| match marks_positioned {
            true => None,
            false => WITH_DAGESH
                .iter()
                .find(|&&(dotted, _)| dotted == a && b == DAGESH)
                .map(|&(_, form)| form)
                .or_else(|| unicode::hebrew_presentation_form(a, b)),
        })
    }

    // A meteg or below mark typed after a wide then a narrow vowel goes with the wide one: the first such mark moves
    // back before the narrow vowel, which canonical order put first.
    fn reorder_marks(&self, run: &mut Run, marks: Range<usize>) {
        let of = |g: &Glyph, classes: &[u8]| classes.contains(&g.mark_class());
        let found = run.glyphs[marks.clone()]
            .windows(3)
            .position(|w| of(&w[0], &WIDE_VOWELS) && of(&w[1], &NARROW_VOWELS) && of(&w[2], &METEG_OR_BELOW));
        if let Some(at) = found.map(|k| marks.start + k + 1) {
            run.join_clusters(at..at + 2);
            run.glyphs[at..at + 2].reverse();
        }
    }
}
