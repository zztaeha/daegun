// Hangul, after Microsoft's Hangul spec and Core spec 3.12: jamo composed to a syllable the font has, a syllable it
// lacks split to jamo it has, loose jamo given ljmo, vjmo and tjmo, a tone mark put before its syllable.
use core::ops::Range;

use super::{MarkAdvances, Model, ModelData, Normalization, Shaper};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::plan::{FeatureUse, Features, Plan, Planner};
use crate::daecore::daeshaper::run::{Mask, Run};
use crate::daecore::daeshaper::unicode::{hangul_jamo, hangul_syllable, Script, DOTTED_CIRCLE};

pub(crate) struct Hangul;

pub(crate) const MODEL: Hangul = Hangul;

// The jamo features, by the value a jamo keeps in its script byte; 0 is none.
const JAMO: [&[u8; 4]; 3] = [b"ljmo", b"vjmo", b"tjmo"];
const LEADING: u8 = 1;
const VOWEL: u8 = 2;
const TRAILING: u8 = 3;

struct Masks {
    // By jamo value, none at 0.
    jamo: [Mask; 4],
    calt: Mask,
}

impl Model for Hangul {
    fn kind(&self) -> Shaper {
        Shaper::Hangul
    }

    fn plan_features(&self, planner: &mut Planner<'_>, _: Option<Tag>) {
        JAMO.iter().for_each(|&tag| planner.add(Tag::new(tag), FeatureUse::PLAIN));
    }

    // Asked again as a plain feature, calt takes a bit of its own, which `setup_masks` takes off every jamo: the jamo
    // features alone shape them.
    fn plan_late_features(&self, planner: &mut Planner<'_>) {
        planner.add(Tag::new(b"calt"), FeatureUse::PLAIN);
    }

    fn plan_data(&self, features: &Features, _: &Face, _: Option<Script>, _: Option<Tag>) -> Option<ModelData> {
        let bit = |tag: &[u8; 4]| features.on_bit(Tag::new(tag));
        let [l, v, t] = JAMO.map(bit);
        Some(alloc::boxed::Box::new(Masks { jamo: [0, l, v, t], calt: bit(b"calt") }))
    }

    fn mark_advances(&self) -> MarkAdvances {
        MarkAdvances::Keep
    }

    // Composing here asks what the font maps, which the normalizer doesn't, so it keeps out entirely.
    fn normalization(&self) -> Normalization {
        Normalization::Unrecomposed
    }

    fn prepare(&self, run: &mut Run, _: &Plan, face: &Face) {
        compose_syllables(face, run);
    }

    fn setup_masks(&self, run: &mut Run, plan: &Plan, _: &Face) {
        let Some(masks) = plan.data::<Masks>() else { return };
        let len = run.len;
        for glyph in &mut run.glyphs[..len] {
            glyph.mask |= masks.jamo.get(usize::from(glyph.script_slot)).copied().unwrap_or(0);
            if jamo_of(glyph.id).is_some() {
                glyph.mask &= !masks.calt;
            }
        }
    }
}

// Which jamo `code` is, the old and extended ones included.
fn jamo_of(code: u32) -> Option<u8> {
    match code {
        0x1100..=0x115F | 0xA960..=0xA97C => Some(LEADING),
        0x1160..=0x11A7 | 0xD7B0..=0xD7C6 => Some(VOWEL),
        0x11A8..=0x11FF | 0xD7CB..=0xD7FB => Some(TRAILING),
        _ => None,
    }
}

fn is_tone(code: u32) -> bool {
    matches!(code, 0x302E | 0x302F)
}

fn has_no_width(face: &Face, code: u32) -> bool {
    face.glyph_of(code).is_some_and(|glyph| face.width(glyph) == 0)
}

fn compose_syllables(face: &Face, run: &mut Run) {
    run.start_rewrite();
    // The written glyphs of the syllable just formed, which a tone mark after it joins.
    let mut syllable: Option<Range<usize>> = None;
    while run.cursor < run.len && !run.failed {
        let code = run.current().id;
        let next = run.glyphs[run.cursor + 1..run.len].first().map(|g| g.id);
        let start = run.written;
        syllable = if is_tone(code) {
            place_tone(face, run, code, syllable);
            None
        } else if jamo_of(code) == Some(LEADING) && next.is_some_and(|v| jamo_of(v) == Some(VOWEL)) {
            Some(start..from_jamo(face, run, start))
        } else if let Some(end) = from_syllable(face, run, start, next) {
            Some(start..end)
        } else {
            run.keep();
            None
        };
    }
    run.finish_rewrite();
}

// A tone mark with width after a syllable goes before it, into its cluster: Hangul writes tone marks to the left.
// With no syllable before it, it takes a dotted circle, going before the circle if it has width and after if not.
fn place_tone(face: &Face, run: &mut Run, tone: u32, syllable: Option<Range<usize>>) {
    if let Some(span) = syllable.filter(|s| !s.is_empty() && s.end == run.written) {
        run.unsafe_to_break_across(span.start, run.cursor);
        if run.keep() && !has_no_width(face, tone) {
            run.join_out_clusters(span.start..span.end + 1);
            run.out_mut()[span.start..=span.end].rotate_right(1);
        }
        return;
    }
    let circle = DOTTED_CIRCLE as u32;
    if !run.settings.dotted_circles || !face.maps(circle) {
        run.keep();
    } else if has_no_width(face, tone) {
        run.replace_span(1, &[circle, tone]);
    } else {
        run.replace_span(1, &[tone, circle]);
    }
}

// Leading jamo, vowel jamo and maybe a trailing jamo at the cursor: their syllable if the font has it, else the jamo
// each with its feature, one grapheme. Gives the written end.
fn from_jamo(face: &Face, run: &mut Run, start: usize) -> usize {
    let at = run.cursor;
    let (l, v) = (run.glyphs[at].id, run.glyphs[at + 1].id);
    let t = run.glyphs[at + 2..run.len].first().map(|g| g.id).filter(|&t| jamo_of(t) == Some(TRAILING));
    let count = if t.is_some() { 3 } else { 2 };
    run.unsafe_to_break(at..at + count);
    if let Some(s) = hangul_syllable(l, v, t).filter(|&s| face.maps(s)) {
        run.replace_span(count, &[s]);
        return start + 1;
    }
    for value in [LEADING, VOWEL, TRAILING].into_iter().take(count) {
        run.current_mut().script_slot = value;
        run.keep();
    }
    if !run.failed {
        run.join_out_graphemes(start..start + count);
    }
    start + count
}

// A precomposed syllable at the cursor: an LV takes a trailing jamo into an LVT the font has, or else goes to jamo, as
// a syllable the font lacks does where it has them. None where it's no syllable or there's no way to draw it.
fn from_syllable(face: &Face, run: &mut Run, start: usize, next: Option<u32>) -> Option<usize> {
    let at = run.cursor;
    let code = run.current().id;
    let (l, v, t) = hangul_jamo(code)?;
    let trailing = next.filter(|&n| t.is_none() && jamo_of(n) == Some(TRAILING));
    if let Some(lvt) = trailing.and_then(|n| hangul_syllable(l, v, Some(n))) {
        if face.maps(lvt) {
            run.replace_span(2, &[lvt]);
            return Some(start + 1);
        }
        run.unsafe_to_break(at..at + 2);
    }

    let drawn = face.maps(code);
    if drawn && trailing.is_none() {
        run.keep();
        return Some(start + 1);
    }
    let jamo = [l, v, t.unwrap_or(0)];
    let parts = &jamo[..2 + usize::from(t.is_some())];
    if !parts.iter().all(|&j| face.maps(j)) {
        if trailing.is_some() {
            run.unsafe_to_break(at..at + 2);
        }
        return drawn.then(|| {
            run.keep();
            start + 1
        });
    }
    run.replace_span(1, parts);
    // A drawn LV split for its trailing jamo takes the jamo along.
    let taken = parts.len() + usize::from(drawn);
    if drawn {
        run.keep();
    }
    if !run.failed {
        let out = &mut run.out_mut()[start..start + taken];
        out.iter_mut().zip([LEADING, VOWEL, TRAILING]).for_each(|(glyph, value)| glyph.script_slot = value);
        run.join_out_graphemes(start..start + taken);
    }
    Some(start + taken)
}
