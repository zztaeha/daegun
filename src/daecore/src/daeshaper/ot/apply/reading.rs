// The one matcher every lookup and context rule reads through: LookupFlag hides glyphs (OpenType chapter 2), a
// default ignorable may stand between the glyphs a rule names, and any other glyph must match or the read ends.
use super::SEQUENCE_LIMIT;
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::ot::lookup_flags;
use crate::daecore::daeshaper::run::{class, Glyph, Mask, Run};

// The glyphs a lookup's flags take out of its sight: whole GDEF classes, and marks outside its mark filtering set or
// of another mark attachment class.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Filter {
    // In the bits `class` lays where LookupFlag keeps its ignore bits, so one AND tells.
    classes: u8,
    mark_set: Option<u16>,
    attach_class: u8,
    // A cursive chain's last glyph, not its first, stays on the baseline (LookupFlag RIGHT_TO_LEFT).
    pub(crate) last_on_baseline: bool,
}

impl Filter {
    // A set index past GDEF's sets names no set, so it filters nothing.
    pub(crate) fn of_lookup(flags: u16, mark_set: Option<u16>, face: &Face) -> Filter {
        let filtering = flags & lookup_flags::USE_MARK_FILTERING_SET != 0;
        Filter {
            classes: (flags & lookup_flags::IGNORE_FLAGS) as u8,
            mark_set: mark_set.filter(|&set| filtering && set < face.mark_set_count()),
            attach_class: ((flags & lookup_flags::MARK_ATTACHMENT_TYPE_MASK) >> 8) as u8,
            last_on_baseline: flags & lookup_flags::RIGHT_TO_LEFT != 0,
        }
    }

    pub(crate) fn of_classes(classes: u8) -> Filter {
        Filter { classes, ..Filter::default() }
    }

    // The same marks out of sight, but no whole class.
    pub(crate) fn marks_only(self) -> Filter {
        Filter { classes: 0, ..self }
    }

    // A filtering set settles a mark before its attachment class is read.
    #[inline(always)]
    pub(crate) fn hides(&self, g: &Glyph, face: &Face) -> bool {
        if g.class & self.classes != 0 {
            return true;
        }
        if g.class & class::MARK == 0 {
            return false;
        }
        match self.mark_set {
            Some(set) => !face.in_mark_set(g.id, set),
            None => self.attach_class != 0 && self.attach_class != g.attach_class,
        }
    }
}

// What a glyph is to a match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Seen {
    // Out of the lookup's sight: passed whatever the rule names.
    Hidden,
    // A default ignorable the lookup may pass: taken where the rule names it, else passed.
    Optional,
    // Taken where the rule names it, else the match ends.
    Required,
}

// What a match does at a glyph.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Step {
    Take,
    Pass,
    End,
}

// What a match looks for in its next glyph.
pub(crate) trait Want {
    fn takes(&self, glyph: u32, optional: bool) -> bool;
}

// A glyph that has to be there: any required one, never one the lookup may pass.
pub(crate) struct Present;

impl Want for Present {
    #[inline(always)]
    fn takes(&self, _: u32, optional: bool) -> bool {
        !optional
    }
}

impl<F: Fn(u32) -> bool> Want for F {
    #[inline(always)]
    fn takes(&self, glyph: u32, _: bool) -> bool {
        self(glyph)
    }
}

// One way of reading the run: the lookup's filter, the feature's mask, the syllable a match stays in (0 for none), and
// the format characters a default ignorable stays required for.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Reading {
    pub(crate) filter: Filter,
    mask: Mask,
    syllable: u8,
    keeps: u8,
}

impl Reading {
    pub(crate) fn new(filter: Filter, mask: Mask, syllable: u8, keeps: u8) -> Reading {
        Reading { filter, mask, syllable, keeps }
    }

    pub(crate) fn with_filter(self, filter: Filter) -> Reading {
        Reading { filter, ..self }
    }

    #[inline(always)]
    pub(crate) fn sees(&self, g: &Glyph, face: &Face) -> Seen {
        if self.filter.hides(g, face) {
            Seen::Hidden
        } else if g.is_ignorable() && g.text & self.keeps == 0 {
            Seen::Optional
        } else {
            Seen::Required
        }
    }

    // The mask and syllable are read before the value, which may cost a search.
    #[inline(always)]
    pub(crate) fn step(&self, g: &Glyph, face: &Face, want: &impl Want) -> Step {
        let seen = self.sees(g, face);
        if seen == Seen::Hidden {
            return Step::Pass;
        }
        match (self.in_scope(g) && want.takes(g.id, seen == Seen::Optional), seen) {
            (true, _) => Step::Take,
            (false, Seen::Required) => Step::End,
            (false, _) => Step::Pass,
        }
    }

    // The first glyph past `from` taken, or else the end of what was read: past the glyph that ended the match, or
    // past the last one there was.
    #[inline(always)]
    pub(crate) fn after(&self, glyphs: &[Glyph], from: usize, face: &Face, want: &impl Want) -> Result<usize, usize> {
        let rest = glyphs.get(from + 1..).unwrap_or_default();
        for (i, g) in (from + 1..).zip(rest) {
            match self.step(g, face, want) {
                Step::Take => return Ok(i),
                Step::End => return Err(i + 1),
                Step::Pass => {}
            }
        }
        Err(glyphs.len().max(from + 1))
    }

    // The nearest glyph before `from` taken, or else where the read ends: from the glyph before the one that ended
    // the match, or from the start.
    pub(crate) fn before(&self, glyphs: &[Glyph], from: usize, face: &Face, want: &impl Want) -> Result<usize, usize> {
        for (i, g) in glyphs[..from].iter().enumerate().rev() {
            match self.step(g, face, want) {
                Step::Take => return Ok(i),
                Step::End => return Err(i.saturating_sub(1)),
                Step::Pass => {}
            }
        }
        Err(0)
    }

    #[inline(always)]
    fn in_scope(&self, g: &Glyph) -> bool {
        g.mask & self.mask != 0 && (self.syllable == 0 || g.syllable == self.syllable)
    }

    // What a value would be compared with next past `from`: the first glyph in scope, `after` taking any value but
    // telling each glyph's kind as it goes.
    pub(crate) fn peek(&self, glyphs: &[Glyph], from: usize, face: &Face) -> Peek {
        let rest = glyphs.get(from + 1..).unwrap_or_default();
        for (i, g) in (from + 1..).zip(rest) {
            let seen = self.sees(g, face);
            match (seen, seen != Seen::Hidden && self.in_scope(g)) {
                (Seen::Hidden, _) => {}
                (Seen::Required, true) => return Peek::Glyph(g.id, i + 1),
                (Seen::Required, false) => return Peek::Stop(i + 1),
                (_, true) => return Peek::Optional(g.id, i),
                (_, false) => {}
            }
        }
        Peek::Stop(glyphs.len().max(from + 1))
    }
}

// The glyph a value is compared with next: a required glyph and the end past it; none, the read stopping before
// `end`; or an optional glyph and its place, which a value may take or pass, so it tells nothing alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Peek {
    Glyph(u32, usize),
    Stop(usize),
    Optional(u32, usize),
}

// An input matched: where it ends in the unread glyphs, and how many ligature components it holds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Input {
    pub(crate) end: usize,
    pub(crate) components: u16,
}

// Matches `count` input glyphs from the cursor's into `places`; on failure, where what it marks ends. An input stays on
// one ligature component, so a mark isn't ligated away from the component it sits on, and a glyph on another fails it
// marking nothing. Beside it, how many glyphs the match read, back to a ligature too.
pub(crate) fn input(
    run: &Run,
    face: &Face,
    reading: &Reading,
    count: usize,
    wants: impl Fn(u32, usize) -> bool,
    places: &mut [u32; SEQUENCE_LIMIT],
) -> (Result<Input, usize>, usize) {
    let start = run.cursor;
    if count > SEQUENCE_LIMIT {
        return (Err(start), 0);
    }
    let glyphs = &run.glyphs[..run.len];
    let first = &glyphs[start];
    let (ligature, component) = (first.ligature_id(), first.component());
    let on_component = ligature != 0 && component != 0;
    let mut ligature_hidden = None;
    let mut back = 0;
    let mut components = first.components();
    let mut at = start;
    for k in 0..count.saturating_sub(1) {
        at = match reading.after(glyphs, at, face, &|g: u32| wants(g, k)) {
            Ok(at) => at,
            Err(end) => return (Err(end), end - start + back),
        };
        let g = &glyphs[at];
        let elsewhere = g.ligature_id() != 0 && g.component() != 0 && g.ligature_id() != ligature;
        let kept = match on_component {
            true if (g.ligature_id(), g.component()) == (ligature, component) => true,
            true => *ligature_hidden.get_or_insert_with(|| hides_ligature(run, face, reading, ligature, &mut back)),
            false => !elsewhere,
        };
        if !kept {
            return (Err(start), at + 1 - start + back);
        }
        places[k + 1] = at as u32;
        components = components.saturating_add(g.components_joined());
    }
    places[0] = start as u32;
    (Ok(Input { end: at + 1, components }), at + 1 - start + back)
}

// Whether the lookup has the ligature that the glyphs before the cursor of id `ligature` sit on out of its sight,
// `read` counting the glyphs passed back to it.
fn hides_ligature(run: &Run, face: &Face, reading: &Reading, ligature: u8, read: &mut usize) -> bool {
    let written = &run.out()[..run.behind()];
    let mut back = written.iter().rev().take_while(|g| g.ligature_id() == ligature).inspect(|_| *read += 1);
    back.find(|g| g.component() == 0).is_some_and(|g| reading.filter.hides(g, face))
}

// Matches `count` glyphs before the cursor, written ones while a rewrite runs, `wants(glyph, k)` naming the `k`th
// back. Where the backtrack starts, or else where the read ends.
pub(crate) fn backtrack(
    run: &Run,
    face: &Face,
    reading: &Reading,
    count: usize,
    wants: impl Fn(u32, usize) -> bool,
) -> Result<usize, usize> {
    let glyphs = &run.out()[..run.behind()];
    (0..count).try_fold(run.behind(), |at, k| reading.before(glyphs, at, face, &|g: u32| wants(g, k)))
}

// Matches `count` glyphs from `end`, past an input, `wants(glyph, k)` naming the `k`th. Where the lookahead ends,
// or else where the read ends.
pub(crate) fn lookahead(
    run: &Run,
    face: &Face,
    reading: &Reading,
    end: usize,
    count: usize,
    wants: impl Fn(u32, usize) -> bool,
) -> Result<usize, usize> {
    debug_assert!(end >= 1);
    let glyphs = &run.glyphs[..run.len];
    let last = (0..count).try_fold(end.max(1) - 1, |at, k| reading.after(glyphs, at, face, &|g: u32| wants(g, k)))?;
    Ok(last + 1)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daeshaper/ot/apply/reading.rs"]
mod tests;
