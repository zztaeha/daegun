// What the syllabic models (Indic, Khmer, Myanmar, USE) share: a GSUB plan as a table, the dotted circles of broken
// syllables and look-alike vowels, and a syllable's glyphs numbered from its start with the moves reordering makes.
use core::ops::{Index, IndexMut, Range};

use super::syllable;
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::generated::vowel_constraints::{
    opens_invalid_vowel_sequence, INVALID_VOWEL_PAIRS, INVALID_VOWEL_TRIPLES,
};
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::plan::{FeatureUse, Features, Pause, Planner};
use crate::daecore::daeshaper::run::{class, text, Fact, Glyph, Mask, Run};
use crate::daecore::daeshaper::unicode::{self, GeneralCategory};

pub(crate) type Feature = (&'static [u8; 4], FeatureUse);

// One line of a model's GSUB plan. A stage holds the lookups of the features added since the stage before it ended.
pub(crate) enum Step {
    // Features whose lookups share the stage.
    Together(&'static [Feature]),
    // Features each given a stage of its own.
    OneByOne(&'static [Feature]),
    // The stage ends and the pause, if any, runs.
    End(Option<Pause>),
}

pub(crate) fn declare(planner: &mut Planner, steps: &[Step]) {
    for step in steps {
        match *step {
            Step::Together(features) => features.iter().for_each(|&(tag, how)| planner.add(Tag::new(tag), how)),
            Step::OneByOne(features) => {
                for &(tag, how) in features {
                    planner.add(Tag::new(tag), how);
                    planner.end_stage(None);
                }
            }
            Step::End(pause) => planner.end_stage(pause),
        }
    }
}

// Each feature's value 1 as a mask; 0 for a feature the plan dropped.
pub(crate) fn bits<const N: usize>(features: &Features, tags: [&[u8; 4]; N]) -> [Mask; N] {
    tags.map(|tag| features.on_bit(Tag::new(tag)))
}

impl Glyph {
    // A glyph a ligature formed has none of these classes: the characters it came from no longer say what it is.
    pub(crate) fn unligated_in(&self, classes: &[u8]) -> bool {
        classes.contains(&self.script_class) && !self.glyph_is(class::LIGATED)
    }
}

// The glyphs of one syllable, numbered from its first.
pub(crate) struct Syllable<'r> {
    pub(crate) run: &'r mut Run,
    pub(crate) start: usize,
    end: usize,
}

impl<'r> Syllable<'r> {
    pub(crate) fn new(run: &'r mut Run, span: Range<usize>) -> Self {
        Syllable { run, start: span.start, end: span.end }
    }

    pub(crate) fn len(&self) -> usize {
        self.end - self.start
    }

    pub(crate) fn glyphs(&mut self) -> &mut [Glyph] {
        &mut self.run.glyphs[self.start..self.end]
    }

    pub(crate) fn class(&self, i: usize) -> u8 {
        self[i].script_class
    }

    // The script's position of glyph `i`, kept in its own byte.
    pub(crate) fn pos(&self, i: usize) -> u8 {
        self[i].script_slot
    }

    pub(crate) fn set_pos(&mut self, i: usize, position: u8) {
        self[i].script_slot = position;
    }

    pub(crate) fn is(&self, i: usize, classes: &[u8]) -> bool {
        self[i].unligated_in(classes)
    }

    pub(crate) fn join(&mut self, span: Range<usize>) {
        self.run.join_clusters(self.start + span.start..self.start + span.end);
    }

    // Glyph `from` moves to `to`, those between closing up behind it; no cluster changes.
    pub(crate) fn place(&mut self, from: usize, to: usize) {
        let glyphs = self.glyphs();
        match from < to {
            true => glyphs[from..=to].rotate_left(1),
            false => glyphs[to..=from].rotate_right(1),
        }
    }

    // As `place`, the clusters it crosses joined first.
    pub(crate) fn carry(&mut self, from: usize, to: usize) {
        self.join(from.min(to)..from.max(to) + 1);
        self.place(from, to);
    }

    // Reverses the order of pre-base matra groups in `span`: each matra keeps the signs that follow it, and the group
    // typed first ends nearest the base.
    pub(crate) fn flip_pre_base(&mut self, span: Range<usize>, matras: &[u8]) {
        if span.len() < 2 {
            return;
        }
        let at = self.start;
        self.run.reverse_span(at + span.start..at + span.end);
        let mut group = span.start;
        for i in span {
            if matras.contains(&self.class(i)) {
                self.run.reverse_span(at + group..at + i + 1);
                group = i + 1;
            }
        }
    }
}

impl Index<usize> for Syllable<'_> {
    type Output = Glyph;

    fn index(&self, i: usize) -> &Glyph {
        &self.run.glyphs[self.start + i]
    }
}

impl IndexMut<usize> for Syllable<'_> {
    fn index_mut(&mut self, i: usize) -> &mut Glyph {
        &mut self.run.glyphs[self.start + i]
    }
}

// Runs `f` on each syllable whose kind `takes` accepts.
pub(crate) fn each_syllable(run: &mut Run, takes: impl Fn(u8) -> bool, mut f: impl FnMut(&mut Syllable)) {
    syllable::for_each(run, |run, start, end| {
        if takes(run.glyphs[start].syllable_kind()) {
            f(&mut Syllable::new(run, start..end));
        }
    });
}

// What a broken syllable's dotted circle is to its model.
pub(crate) struct Circle {
    // The broken syllable's kind.
    pub(crate) broken: u8,
    pub(crate) class: u8,
    pub(crate) position: Option<u8>,
    // A class that stays before the circle, as a repha keeps its place at the syllable's start.
    pub(crate) after: Option<u8>,
}

// Gives each broken syllable a dotted circle for the base it lacks (each script's spec, "invalid combining marks"),
// as a glyph: a pause runs once glyphs are mapped. Returns whether it rewrote the run.
pub(crate) fn circle_broken_syllables(face: &Face, run: &mut Run, circle: &Circle) -> bool {
    let wanted = run.settings.dotted_circles && run.facts.has(Fact::BrokenSyllable);
    let Some(glyph) = wanted.then(|| face.glyph_of(unicode::DOTTED_CIRCLE as u32)).flatten() else { return false };
    // A control character, so the text after it starts a word.
    let mut model = Glyph::of_char(glyph, 0);
    model.set_category(GeneralCategory::Control);
    model.script_class = circle.class;
    if let Some(position) = circle.position {
        model.script_slot = position;
    }

    run.start_rewrite();
    // A syllable starts where the glyph before it is of another, not where the last broken syllable did: a serial
    // (1 to 15) repeats 15 syllables on, and a broken syllable repeating one would go uncircled.
    let mut last = 0;
    while run.cursor < run.len && !run.failed {
        let here = *run.current();
        let starts = here.syllable != last;
        last = here.syllable;
        if !starts || here.syllable_kind() != circle.broken {
            run.keep();
            continue;
        }
        let stays = |g: &Glyph| g.syllable == last && Some(g.script_class) == circle.after;
        while run.cursor < run.len && !run.failed && stays(run.current()) {
            run.keep();
        }
        run.insert_glyph(Glyph { cluster: here.cluster, mask: here.mask, syllable: last, ..model });
    }
    run.finish_rewrite();
    true
}

// Unicode's chapter 12 forbids writing an independent vowel as a sequence of others; Microsoft's list of such
// sequences gets a dotted circle before its last character, which then opens no sequence of its own.
pub(crate) fn circle_vowel_lookalikes(run: &mut Run) {
    let (Some(script), true) = (run.script, run.settings.dotted_circles) else { return };
    run.start_rewrite();
    while run.cursor + 1 < run.len && !run.failed {
        let length = lookalike_at(run, script);
        if length > 0 {
            run.keep_n(length - 1);
            run.insert(unicode::DOTTED_CIRCLE as u32);
            let written = run.written;
            if written > 0 {
                run.out_mut()[written - 1].text &= !text::CONTINUES;
            }
        }
        run.keep();
    }
    run.finish_rewrite();
}

// The length of the look-alike sequence at the cursor, 0 for none.
fn lookalike_at(run: &Run, script: unicode::Script) -> usize {
    let codes = &run.glyphs[run.cursor..run.len];
    let first = codes[0].id;
    let ours = char::from_u32(first).is_some_and(|c| unicode::script(c) == script);
    if !ours || !opens_invalid_vowel_sequence(first) {
        return 0;
    }
    if INVALID_VOWEL_PAIRS.binary_search(&(first, codes[1].id)).is_ok() {
        return 2;
    }
    let triple = codes.get(2).map(|third| (first, codes[1].id, third.id));
    match triple.is_some_and(|t| INVALID_VOWEL_TRIPLES.binary_search(&t).is_ok()) {
        true => 3,
        false => 0,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/script/syllabic.rs"]
mod tests;
