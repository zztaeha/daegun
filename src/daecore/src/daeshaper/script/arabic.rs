// Arabic and Syriac, after Microsoft's specs for them: joining forms from Unicode's joining types (ArabicShaping.txt,
// Core spec 9.2 and 9.3), one feature at a time; modifier marks first (UAX #53); `stch` stretched once placed.
use alloc::boxed::Box;
use core::ops::Range;

use super::{Model, ModelData, Shaper};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::plan::{FeatureUse as U, Features, Plan, Planner};
use crate::daecore::daeshaper::run::{class, Direction, Fact, Glyph, Mask, Run};
use crate::daecore::daeshaper::unicode::{self, GeneralCategory, JoiningType as T, Script};

mod fallback;
pub(crate) use fallback::{Fallback, SubsetNeeds};

pub(crate) struct Arabic;

pub(crate) const MODEL: Arabic = Arabic;

// A glyph's form, kept in its script byte: a joining form indexes `FORMS` and its mask; then what `stch` made of it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
enum Form {
    Isol,
    Fina,
    Fin2,
    Fin3,
    Medi,
    Med2,
    Init,
    // Doesn't join: transparent, or joining nothing.
    Unjoined,
    // A glyph `stch` made that is drawn once, and one repeated to fill the room.
    Stretched,
    Repeated,
}

// The joining forms' features, in `Form`'s order. The Syriac alaph's fin2, fin3 and med2 end in a digit.
const FORMS: [&[u8; 4]; 7] = [b"isol", b"fina", b"fin2", b"fin3", b"medi", b"med2", b"init"];

fn syriac_only(tag: &[u8; 4]) -> bool {
    tag[3].is_ascii_digit()
}

impl Form {
    fn of(glyph: &Glyph) -> u8 {
        glyph.script_slot
    }

    fn set(self, glyph: &mut Glyph) {
        glyph.script_slot = self as u8;
    }
}

// The joining forms' masks, by `Form`; nothing for `Unjoined`. A script joining as Arabic does, shaped by another
// model, gives its letters their forms through it too.
pub(crate) struct Joining {
    masks: [Mask; 8],
}

impl Joining {
    pub(crate) fn new(features: &Features) -> Joining {
        let mut masks = [0; 8];
        masks.iter_mut().zip(FORMS).for_each(|(mask, tag)| *mask = features.on_bit(Tag::new(tag)));
        Joining { masks }
    }

    // Joins the run's letters, then gives each glyph its form's mask.
    pub(crate) fn apply(&self, run: &mut Run) {
        join_letters(run);
        if run.script == Some(Script::MONGOLIAN) {
            // A free variation selector reads as the letter before it.
            for i in 1..run.len {
                if unicode::is_free_variation_selector(run.glyphs[i].id) {
                    run.glyphs[i].script_slot = run.glyphs[i - 1].script_slot;
                }
            }
        }
        let len = run.len;
        for glyph in &mut run.glyphs[..len] {
            glyph.mask |= self.masks.get(usize::from(Form::of(glyph))).copied().unwrap_or(0);
        }
    }
}

struct Data {
    joining: Joining,
    stretches: bool,
    fallback: Option<Fallback>,
}

impl Model for Arabic {
    fn kind(&self) -> Shaper {
        Shaper::Arabic
    }

    // The Arabic spec's order: stch and its pause, the preprocessing forms, each joining form in a stage of its own,
    // then the ligatures. No feature skips ZWJ, which ends a ligature in Arabic as ZWNJ does.
    fn plan_features(&self, planner: &mut Planner<'_>, script_tag: Option<Tag>) {
        let arabic = script_tag == Some(Tag::new(b"arab"));
        let tag = |name: &[u8; 4]| Tag::new(name);
        planner.on(tag(b"stch"), U::PLAIN);
        planner.end_stage(Some(mark_stretches));
        planner.on(tag(b"ccmp"), U::SEES_ZWJ);
        planner.on(tag(b"locl"), U::SEES_ZWJ);
        planner.end_stage(None);
        for name in FORMS {
            // Arabic text in a font with none of a form's lookups draws it from its presentation forms.
            let how = if arabic && !syriac_only(name) { U::FALLBACK } else { U::PLAIN };
            planner.add(tag(name), U::SEES_ZWJ.and(how));
            planner.end_stage(None);
        }
        planner.on(tag(b"rlig"), U::SEES_ZWJ.and(U::FALLBACK));
        if arabic {
            planner.end_stage(Some(shape_from_presentation_forms));
        }
        // A font with rclt applies calt and rclt in one stage.
        planner.on(tag(b"calt"), U::SEES_ZWJ);
        if !planner.lists(tag(b"rclt")) {
            planner.end_stage(None);
        }
        for name in [b"liga", b"clig", b"mset"] {
            planner.on(tag(name), U::SEES_ZWJ);
        }
    }

    fn plan_data(&self, features: &Features, face: &Face, _: Option<Script>, script_tag: Option<Tag>)
        -> Option<ModelData> {
        let lacks_forms = FORMS.iter().all(|&tag| syriac_only(tag) || features.lacks_table(Tag::new(tag)));
        let fallback = (script_tag == Some(Tag::new(b"arab")) && lacks_forms).then(|| Fallback::new(features, face));
        Some(Box::new(Data {
            joining: Joining::new(features),
            stretches: features.on_bit(Tag::new(b"stch")) != 0,
            fallback: fallback.flatten(),
        }))
    }

    fn reorder_marks(&self, run: &mut Run, marks: Range<usize>) {
        modifier_marks_first(run, marks);
    }

    fn setup_masks(&self, run: &mut Run, plan: &Plan, _: &Face) {
        if let Some(data) = plan.data::<Data>() {
            data.joining.apply(run);
        }
    }

    fn finish(&self, run: &mut Run, _: &Plan, face: &Face) {
        if run.facts.has(Fact::ForScript) {
            stretch(face, run);
        }
    }
}

// The fallback the plan built, where the font's Arabic has none of the forms.
pub(crate) fn plan_fallback(plan: &Plan) -> Option<&Fallback> {
    plan.data::<Data>()?.fallback.as_ref()
}

fn shape_from_presentation_forms(run: &mut Run, plan: &Plan, face: &Face) -> bool {
    let Some(fallback) = plan_fallback(plan) else { return false };
    fallback.shape(face, run);
    true
}

// What the last letter that joins at all offers the next one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Prior {
    // Nothing to join: the text's start or a letter of type U.
    Nothing,
    // A right-joining letter, or an alaph with nothing before it.
    RightJoining,
    // Syriac's dalath or rish, after which an alaph takes fin3.
    DalathRish,
    // A dual- or left-joining letter joined to nothing behind it.
    Open,
    // A dual-joining letter joined behind.
    Joined,
    // An alaph joined to the letter behind it.
    AlaphJoined,
    // An alaph in fin2 or fin3.
    AlaphFinal,
}

impl Prior {
    // Whether the letter could join the next.
    fn reaches_on(self) -> bool {
        matches!(self, Prior::Open | Prior::Joined | Prior::AlaphJoined | Prior::AlaphFinal)
    }
}

fn joins_back(t: T) -> bool {
    matches!(t, T::RightJoining | T::DualJoining | T::Alaph | T::DalathRish)
}

// A letter of type `t` after `prior`: the previous letter's new form, if any, this one's form, and what it offers next.
// Syriac's alaph: fin2 after a letter not joining it, fin3 after dalath or rish, med2 joined behind, a letter after.
fn join(prior: Prior, t: T) -> (Option<Form>, Form, Prior) {
    use Prior as P;
    match t {
        T::LeftJoining => return (None, Form::Isol, P::Open),
        _ if !joins_back(t) => return (None, Form::Unjoined, P::Nothing),
        _ => {}
    }
    let before = match prior {
        P::Open => Some(Form::Init),
        P::Joined => Some(Form::Medi),
        P::AlaphJoined => Some(Form::Med2),
        P::AlaphFinal => Some(Form::Isol),
        P::Nothing | P::RightJoining | P::DalathRish => None,
    };
    let linked = matches!(prior, P::Open | P::Joined);
    let this = match (prior, t) {
        _ if linked => Form::Fina,
        (P::AlaphJoined | P::AlaphFinal | P::RightJoining, T::Alaph) => Form::Fin2,
        (P::DalathRish, T::Alaph) => Form::Fin3,
        _ => Form::Isol,
    };
    let next = match t {
        T::RightJoining => P::RightJoining,
        T::DalathRish => P::DalathRish,
        T::DualJoining if linked => P::Joined,
        T::DualJoining => P::Open,
        _ if linked => P::AlaphJoined,
        _ if prior == P::Nothing => P::RightJoining,
        _ => P::AlaphFinal,
    };
    (before, this, next)
}

fn type_of(code: u32) -> T {
    char::from_u32(code).map_or(T::NonJoining, unicode::joining_type)
}

// The nearest character of `context` that isn't transparent.
fn nearest(context: &[char]) -> Option<T> {
    context.iter().map(|&c| unicode::joining_type(c)).find(|&t| t != T::Transparent)
}

// Each letter's form, from the letters either side of it and the text around the run. A pair that joins may take a
// tatweel between; a pair where one letter reaches for the other without joining is unsafe to concatenate.
fn join_letters(run: &mut Run) {
    let mut prior = nearest(run.context.before()).map_or(Prior::Nothing, |t| join(Prior::Nothing, t).2);
    let mut last: Option<usize> = None;
    for i in 0..run.len {
        let t = type_of(run.glyphs[i].id);
        if t == T::Transparent {
            Form::Unjoined.set(&mut run.glyphs[i]);
            continue;
        }
        let (before, this, next) = join(prior, t);
        match (last, before) {
            (Some(p), Some(form)) => {
                form.set(&mut run.glyphs[p]);
                run.safe_to_insert_tatweel(p..i + 1);
            }
            (Some(p), None) if joins_back(t) || prior.reaches_on() => run.unsafe_to_concat(p..i + 1),
            (None, _) if joins_back(t) => run.unsafe_to_concat(0..i + 1),
            _ => {}
        }
        this.set(&mut run.glyphs[i]);
        (last, prior) = (Some(i), next);
    }
    let (Some(p), Some(t)) = (last, nearest(run.context.after())) else { return };
    match join(prior, t).0 {
        Some(form) => {
            form.set(&mut run.glyphs[p]);
            run.safe_to_insert_tatweel(p..run.len);
        }
        None if prior.reaches_on() => run.unsafe_to_concat(p..run.len),
        None => {}
    }
}

// After `stch`, a glyph it split from one: the odd pieces repeat to fill the room, the even ones are drawn once.
fn mark_stretches(run: &mut Run, plan: &Plan, _: &Face) -> bool {
    if !plan.data::<Data>().is_some_and(|data| data.stretches) {
        return false;
    }
    let mut found = false;
    let len = run.len;
    for glyph in run.glyphs[..len].iter_mut().filter(|g| g.glyph_is(class::MULTIPLIED)) {
        let form = if glyph.component() % 2 == 1 { Form::Repeated } else { Form::Stretched };
        form.set(glyph);
        found = true;
    }
    if found {
        run.facts.note(Fact::ForScript);
    }
    false
}

// The most glyphs one stretch is drawn with: its tiles are bounded, so 256 span any line it can be set on, and a
// hostile font can't fill the run's room from one word.
const MOST_STRETCH_GLYPHS: i64 = 256;

fn stretching(glyph: &Glyph) -> bool {
    matches!(Form::of(glyph), f if f == Form::Stretched as u8 || f == Form::Repeated as u8)
}

// What fills the room before a stretch: caseless letters, marks, numbers and symbols, unassigned and private-use
// characters, and default ignorables.
fn fills_word(glyph: &Glyph) -> bool {
    use GeneralCategory::*;
    let gc = glyph.category;
    let letter_or_unknown = matches!(gc, ModifierLetter | OtherLetter | Unassigned | PrivateUse);
    let number = matches!(gc, DecimalNumber | LetterNumber | OtherNumber);
    let symbol = matches!(gc, MathSymbol | CurrencySymbol | ModifierSymbol | OtherSymbol);
    glyph.is_ignorable() || letter_or_unknown || gc.is_mark() || number || symbol
}

// One stretch: the glyphs `stch` made and the word before it whose width it spans.
struct Stretch {
    word: usize,
    pieces: Range<usize>,
    // Copies of each repeated piece beyond itself, and how far each copy overlaps the one before.
    copies: i64,
    overlap: i32,
    repeats: i64,
    // Room left over, split either side.
    spare: i32,
}

impl Stretch {
    // The stretch ending at `end`, reading the run from its end (pieces and word lie before `end`).
    fn ending_at(run: &Run, end: usize, width: &impl Fn(u32) -> i32) -> Stretch {
        let start = end - run.glyphs[..end].iter().rev().take_while(|g| stretching(g)).count();
        let (mut fixed, mut repeated, mut repeats) = (0i32, 0i32, 0i64);
        for glyph in &run.glyphs[start..end] {
            match Form::of(glyph) == Form::Repeated as u8 {
                true => (repeated, repeats) = (repeated.saturating_add(width(glyph.id)), repeats + 1),
                false => fixed = fixed.saturating_add(width(glyph.id)),
            }
        }
        let fills = |g: &Glyph| !stretching(g) && fills_word(g);
        let word = start - run.glyphs[..start].iter().rev().take_while(|g| fills(g)).count();
        let room = run.places[word..start].iter().rev().fold(0i32, |w, p| w.saturating_add(p.advance_x));

        // In 64 bits, so no difference of widths wraps.
        let (left, tile) = (i64::from(room) - i64::from(fixed), i64::from(repeated));
        let mut spare = room.saturating_sub(fixed);
        let mut copies = if tile > 0 && left > tile { left / tile - 1 } else { 0 };
        let mut overlap = 0;
        if repeats > 0 && left > tile * (copies + 1) {
            copies += 1;
            let excess = tile * (copies + 1) - left;
            if excess > 0 {
                overlap = i32::try_from(excess / (copies * repeats)).unwrap_or(i32::MAX);
                spare = 0;
            }
        }
        let pieces = (end - start) as i64;
        let most = match repeats > 0 && pieces < MOST_STRETCH_GLYPHS {
            true => (MOST_STRETCH_GLYPHS - pieces) / repeats,
            false => 0,
        };
        Stretch { word, pieces: start..end, copies: copies.min(most), overlap, repeats, spare }
    }

    fn added(&self) -> usize {
        (self.copies * self.repeats) as usize
    }
}

// The `stch` feature's stretch: each repeated piece copied until the pieces span the word before them, the copies
// overlapping evenly, the pieces drawn with no advance over the word. The run is read right to left.
fn stretch(face: &Face, run: &mut Run) {
    let rtl = run.direction == Direction::RightToLeft;
    let flip = |run: &mut Run| if !rtl { run.reverse() };
    flip(run);
    let widths = face.advances(false);
    let width = |glyph: u32| widths.of(glyph) as i32;

    let mut added = 0;
    let mut end = run.len;
    while end > 0 {
        if stretching(&run.glyphs[end - 1]) {
            let s = Stretch::ending_at(run, end, &width);
            added += s.added();
            end = s.pieces.start;
        } else {
            end -= 1;
        }
    }

    if run.tail_room(added) {
        // Back to front, so a glyph only moves forward, onto room no unread glyph holds.
        let mut to = run.len + added;
        let mut end = run.len;
        while end > 0 {
            if !stretching(&run.glyphs[end - 1]) {
                end -= 1;
                to -= 1;
                (run.glyphs[to], run.places[to]) = (run.glyphs[end], run.places[end]);
                continue;
            }
            let s = Stretch::ending_at(run, end, &width);
            run.unsafe_to_break(s.word..end);
            let mut x = s.spare / 2;
            for k in s.pieces.clone().rev() {
                let w = width(run.glyphs[k].id);
                let times = if Form::of(&run.glyphs[k]) == Form::Repeated as u8 { 1 + s.copies } else { 1 };
                run.places[k].advance_x = 0;
                for copy in 0..times {
                    let overlap = if copy > 0 { s.overlap } else { 0 };
                    if rtl {
                        x = x.saturating_sub(w).saturating_add(overlap);
                    }
                    run.places[k].offset_x = x;
                    to -= 1;
                    (run.glyphs[to], run.places[to]) = (run.glyphs[k], run.places[k]);
                    if !rtl {
                        x = x.saturating_add(w).saturating_sub(overlap);
                    }
                }
            }
            end = s.pieces.start;
        }
        debug_assert_eq!(to, 0);
        run.len += added;
    }
    flip(run);
}

// The marks UAX #53 calls modifier combining marks, which go before other marks of their class.
const MODIFIER_MARKS: [u32; 14] = [
    0x0654, 0x0655, 0x0658, 0x06DC, 0x06E3, 0x06E7, 0x06E8, 0x08CA, 0x08CB, 0x08CD, 0x08CE, 0x08CF, 0x08D3, 0x08F3,
];

// The classes below and above that modifier marks take once moved, sorting them first should the marks sort again.
const BELOW: (u8, u8) = (220, 22);
const ABOVE: (u8, u8) = (230, 26);

// UAX #53's transient reordering: in a run of marks, sorted canonically, the modifier marks leading the class 220
// marks, then those leading the class 230 marks, go to the run's front, in that order.
fn modifier_marks_first(run: &mut Run, marks: Range<usize>) {
    let mut front = marks.start;
    let mut i = marks.start;
    for (class, moved) in [BELOW, ABOVE] {
        i += run.glyphs[i..marks.end].iter().take_while(|g| g.mark_class() < class).count();
        if i == marks.end {
            return;
        }
        let leading = |g: &Glyph| g.mark_class() == class && MODIFIER_MARKS.contains(&g.id);
        let count = run.glyphs[i..marks.end].iter().take_while(|g| leading(g)).count();
        if count == 0 {
            continue;
        }
        run.join_clusters(front..i + count);
        run.glyphs[front..i + count].rotate_right(count);
        run.glyphs[front..front + count].iter_mut().for_each(|g| g.set_mark_class(moved));
        front += count;
        i += count;
    }
}
