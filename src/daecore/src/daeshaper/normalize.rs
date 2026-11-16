// Normalization as the font can draw it (UAX #15, Unicode 3.11): a character it lacks splits into parts it has, marks
// sort by class and rejoin their base where the font has the composite. Every character leaves with `mapped` set.
use super::face::Face;
use super::run::{text, Fact, Glyph, Run};
use super::script::{Model, Normalization};
use super::unicode;

// A run of marks longer than this keeps its order: the sort is quadratic in it, and text stays far below it (UAX #15's
// stream-safe format allows 30).
const MOST_MARKS_SORTED: usize = 32;

// How many levels a decomposition is followed down. Unicode's go three deep, and the models' own no deeper.
const MOST_LEVELS: usize = 4;

const COMBINING_GRAPHEME_JOINER: u32 = 0x034F;

pub(crate) fn normalize(run: &mut Run, face: &Face, model: &dyn Model, marks_positioned: bool) {
    if run.len == 0 {
        return;
    }
    let how = model.normalization();
    let marked = split_run(run, face, model, how);
    // A failed run is left as it stands.
    if run.failed {
        return;
    }
    if marked {
        sort_marks(run, model);
    }
    if run.facts.has(Fact::Cgj) {
        show_idle_joiners(run);
    }
    if marked && how != Normalization::Unrecomposed {
        recompose(run, face, model, marks_positioned);
    }
}

// A non-breaking hyphen the font lacks is drawn as a hyphen.
pub(crate) fn hyphen_stand_in(c: char) -> Option<char> {
    (c == '\u{2011}').then_some('\u{2010}')
}

// Which level of its decomposition a character takes: the first the font draws, which is the character itself where
// the font maps it, or the last, so its marks come apart to be sorted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Shallowest,
    Deepest,
}

// The levels a character alone and one with marks after it take.
fn levels(how: Normalization) -> (Level, Level) {
    match how {
        Normalization::Precomposed => (Level::Shallowest, Level::Deepest),
        Normalization::Recomposed => (Level::Deepest, Level::Deepest),
        Normalization::Unrecomposed => (Level::Shallowest, Level::Shallowest),
    }
}

// The first pass, a character and the marks after it at a time; true where any had marks or came apart, so the
// recomposing pass, which runs over the whole run or not at all, gives a character alike whatever stands beside it.
fn split_run(run: &mut Run, face: &Face, model: &dyn Model, how: Normalization) -> bool {
    let (alone, with_marks) = levels(how);
    let mut marked = false;
    run.start_rewrite();
    let mapped = if alone == Level::Shallowest { map_in_place(run, face) } else { 0 };
    run.keep_n(mapped);
    while run.cursor < run.len && !run.failed {
        let start = run.cursor;
        let end = start + 1 + run.glyphs[start + 1..run.len].iter().take_while(|g| g.is_mark_char()).count();
        if end == start + 1 {
            marked |= take(run, face, model, alone);
            continue;
        }
        marked = true;
        if run.glyphs[start..end].iter().any(|g| unicode::is_variation_selector(g.id)) {
            take_selected(run, face, end);
        } else {
            while run.cursor < end && !run.failed {
                take(run, face, model, with_marks);
            }
        }
    }
    run.finish_rewrite();
    marked
}

// The leading characters `take` would keep as they stand, each the font's glyph and no mark after it, mapped where they
// are, so the rewrite reads only from the first that splits, stands in or carries marks. How many.
fn map_in_place(run: &mut Run, face: &Face) -> usize {
    let glyphs = &mut run.glyphs[..run.len];
    for i in 0..glyphs.len() {
        let marked = glyphs.get(i + 1).is_some_and(|g| g.is_mark_char());
        let own = char::from_u32(glyphs[i].id).and_then(|c| face.glyph_of(u32::from(c)));
        match own {
            Some(glyph) if !marked => glyphs[i].mapped = glyph,
            _ => return i,
        }
    }
    glyphs.len()
}

// One level of a character's canonical decomposition: each level splits the last one's first part in two. A level
// counts only where the font maps its first part and every second part above it.
#[derive(Clone, Copy)]
struct Parts {
    first: (char, u32),
    // The second parts met on the way down, with their glyphs, the shallowest first.
    seconds: [(char, u32); MOST_LEVELS],
    count: usize,
}

impl Parts {
    // The shallowest or deepest level below `c` the font draws.
    fn below(c: char, face: &Face, model: &dyn Model, wanted: Level) -> Option<Parts> {
        let mut at = Parts { first: (c, 0), seconds: [('\0', 0); MOST_LEVELS], count: 0 };
        let mut found = None;
        for _ in 0..MOST_LEVELS {
            let Some((first, second)) = model.decompose(at.first.0) else { break };
            if let Some(second) = second {
                // A second part the font lacks leaves this level undrawn, and every level below it.
                let Some(glyph) = face.glyph_of(u32::from(second)) else { break };
                at.seconds[at.count] = (second, glyph);
                at.count += 1;
            }
            at.first = (first, 0);
            if let Some(glyph) = face.glyph_of(u32::from(first)) {
                found = Some(Parts { first: (first, glyph), ..at });
                if wanted == Level::Shallowest {
                    break;
                }
            }
        }
        found
    }

    // In text order: the first part, then the second parts from the deepest up.
    fn in_order(&self) -> impl Iterator<Item = (char, u32)> + '_ {
        core::iter::once(self.first).chain(self.seconds[..self.count].iter().rev().copied())
    }
}

// The current character at the level wanted, itself counting as the shallowest; else a stand-in. True where it came
// apart.
fn take(run: &mut Run, face: &Face, model: &dyn Model, wanted: Level) -> bool {
    let missing = run.settings.missing;
    let Some(c) = char::from_u32(run.current().id) else {
        keep_as(run, missing);
        return false;
    };
    let own = face.glyph_of(u32::from(c));
    if let Some(glyph) = own.filter(|_| wanted == Level::Shallowest) {
        keep_as(run, glyph);
        return false;
    }
    match (Parts::below(c, face, model, wanted), own) {
        (Some(parts), _) => {
            write_parts(run, &parts);
            return parts.count > 0;
        }
        (None, Some(glyph)) => keep_as(run, glyph),
        (None, None) => stand_in(run, face, c),
    }
    false
}

fn keep_as(run: &mut Run, glyph: u32) {
    run.current_mut().mapped = glyph;
    run.keep();
}

// Each part goes out as a copy of the character with its own Unicode facts, and the character is passed over.
fn write_parts(run: &mut Run, parts: &Parts) {
    for (c, glyph) in parts.in_order() {
        let at = run.written;
        run.insert(u32::from(c));
        if run.written == at {
            return;
        }
        let part = &mut run.out_mut()[at];
        part.mapped = glyph;
        let facts = part.read_char();
        run.facts.add(facts);
    }
    run.pass_over();
}

// A character the font can't draw any way: a space with the font's space or the invisible glyph, sized later by its
// kind; a non-breaking hyphen as a hyphen; anything else as the missing glyph.
fn stand_in(run: &mut Run, face: &Face, c: char) {
    let space = face.glyph_of(0x20).or(run.settings.invisible);
    if let Some((kind, glyph)) = unicode::space_fallback(c).zip(space) {
        run.current_mut().set_space_fallback(kind);
        run.facts.note(Fact::SpaceStandIns);
        return keep_as(run, glyph);
    }
    let hyphen = hyphen_stand_in(c).and_then(|h| face.glyph_of(u32::from(h)));
    keep_as(run, hyphen.unwrap_or(run.settings.missing));
}

// A variation selector picks a glyph for the character as written, so nothing decomposes: the font's glyph for the
// pair takes both, and where it has none, both stay and the selector is marked for its stand-in.
fn take_selected(run: &mut Run, face: &Face, end: usize) {
    while run.cursor < end && !run.failed {
        let base = run.current().id;
        let next = (run.cursor + 1 < end).then(|| run.glyphs[run.cursor + 1].id);
        let Some(selector) = next.filter(|&id| unicode::is_variation_selector(id)) else {
            keep_mapped(run, face);
            continue;
        };
        match face.variant_of(base, selector) {
            Some(glyph) => {
                run.current_mut().mapped = glyph;
                run.replace_span(2, &[base]);
            }
            None => {
                keep_mapped(run, face);
                if run.failed {
                    return;
                }
                run.facts.note(Fact::SelectorFallback);
                let keeps_stand_in = run.settings.missing_selector.is_some();
                let lone = run.current_mut();
                lone.set_selector_fallback(true);
                if keeps_stand_in {
                    lone.text &= !text::IGNORABLE;
                }
                keep_mapped(run, face);
            }
        }
        while run.cursor < end && !run.failed && unicode::is_variation_selector(run.current().id) {
            keep_mapped(run, face);
        }
    }
}

fn keep_mapped(run: &mut Run, face: &Face) {
    let glyph = face.glyph_of(run.current().id).unwrap_or(run.settings.missing);
    keep_as(run, glyph);
}

// Canonical ordering (Unicode 3.11) by mark class: each run of marks sorted stably, then handed to the model, which
// may move them further.
fn sort_marks(run: &mut Run, model: &dyn Model) {
    let classed = |g: &Glyph| g.mark_class() != 0;
    let mut from = 0;
    while let Some(skipped) = run.glyphs[from..run.len].iter().position(classed) {
        let start = from + skipped;
        let end = start + run.glyphs[start..run.len].iter().take_while(|g| classed(g)).count();
        if end - start <= MOST_MARKS_SORTED {
            run.sort_span_by(start..end, Glyph::mark_class);
            model.reorder_marks(run, start..end);
        }
        from = end;
    }
}

// A CGJ keeps the marks either side of it from reordering across it. Where they were in order anyway it held nothing
// apart, so it stops hiding from GSUB, where it would only stand between them.
fn show_idle_joiners(run: &mut Run) {
    for i in 1..run.len.saturating_sub(1) {
        if run.glyphs[i].id != COMBINING_GRAPHEME_JOINER {
            continue;
        }
        let (before, after) = (run.glyphs[i - 1].mark_class(), run.glyphs[i + 1].mark_class());
        let held_apart = after != 0 && before > after;
        if !held_apart {
            run.glyphs[i].text &= !text::HIDDEN;
        }
    }
}

// Canonical composition (UAX #15): a mark joins the last starter before it where no mark between has a class as high
// and the model composes the two into a character the font maps.
fn recompose(run: &mut Run, face: &Face, model: &dyn Model, marks_positioned: bool) {
    run.start_rewrite();
    run.keep();
    let mut starter = 0;
    while run.cursor < run.len && !run.failed {
        let mark = *run.current();
        let blocked = run.written - 1 != starter && run.last_out().mark_class() >= mark.mark_class();
        let composite = (mark.is_mark_char() && !blocked)
            .then(|| composite_of(run.out()[starter].id, mark.id, face, model, marks_positioned))
            .flatten();
        match composite {
            Some((c, glyph)) => absorb(run, starter, c, glyph),
            None => {
                run.keep();
                if run.last_out().mark_class() == 0 {
                    starter = run.written - 1;
                }
            }
        }
    }
    run.finish_rewrite();
}

fn composite_of(a: u32, b: u32, face: &Face, model: &dyn Model, marks_positioned: bool) -> Option<(char, u32)> {
    let c = model.compose(char::from_u32(a)?, char::from_u32(b)?, marks_positioned)?;
    Some((c, face.glyph_of(u32::from(c))?))
}

// The current mark goes into the starter, which becomes the composite: written and joined in cluster with it, then
// taken back.
fn absorb(run: &mut Run, starter: usize, c: char, glyph: u32) {
    if !run.keep() {
        return;
    }
    run.join_out_clusters(starter..run.written);
    run.written -= 1;
    let composite = &mut run.out_mut()[starter];
    composite.id = u32::from(c);
    composite.mapped = glyph;
    let facts = composite.read_char();
    run.facts.add(facts);
}

#[cfg(test)]
#[path = "../../tests/unit/daeshaper/normalize.rs"]
mod tests;
