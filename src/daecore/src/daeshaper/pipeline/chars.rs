// The stages that read the run's characters, before any glyph is mapped.
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::plan::Plan;
use crate::daecore::daeshaper::run::{text, ClusterLevel, Direction, Fact, Facts, Glyph, Mask, Run};
use crate::daecore::daeshaper::unicode::{self, GeneralCategory};

// Each glyph starts with `global` and its character's facts, and those continuing a grapheme are marked so.
pub(super) fn read_chars(run: &mut Run, global: Mask) {
    let mut facts = run.facts;
    let glyphs = &mut run.glyphs[..run.len];
    let mut i = 0;
    while i < glyphs.len() {
        take_char(&mut glyphs[i], global, &mut facts);
        if joins_what_precedes(glyphs, i) {
            glyphs[i].text |= text::CONTINUES;
        }
        let next = glyphs.get(i + 1).and_then(|g| char::from_u32(g.id));
        if glyphs[i].char_is(text::ZWJ) && next.is_some_and(unicode::is_extended_pictographic) {
            i += 1;
            take_char(&mut glyphs[i], global, &mut facts);
            glyphs[i].text |= text::CONTINUES;
        }
        if glyphs[i].char_is(text::CONTINUES) {
            facts.note(Fact::Continuations);
        }
        i += 1;
    }
    run.facts = facts;
}

fn take_char(glyph: &mut Glyph, global: Mask, facts: &mut Facts) {
    glyph.mask = global;
    facts.add(glyph.read_char());
}

// UAX #29 joins to what precedes them emoji modifiers, a ZWJ and the pictograph after it (GB11) and a regional
// indicator's pair (GB12, GB13), marks too (GB9, GB9a, set by `read_char`); halfwidth sound marks and tags go on too.
fn joins_what_precedes(glyphs: &[Glyph], i: usize) -> bool {
    let glyph = &glyphs[i];
    match glyph.category {
        GeneralCategory::UppercaseLetter
        | GeneralCategory::LowercaseLetter
        | GeneralCategory::TitlecaseLetter
        | GeneralCategory::OtherLetter
        | GeneralCategory::SpaceSeparator => false,
        GeneralCategory::ModifierSymbol => (0x1F3FB..=0x1F3FF).contains(&glyph.id),
        _ if unicode::is_regional_indicator(glyph.id) => i.checked_sub(1).is_some_and(|p| {
            unicode::is_regional_indicator(glyphs[p].id) && !glyphs[p].char_is(text::CONTINUES)
        }),
        _ => glyph.char_is(text::ZWJ) || matches!(glyph.id, 0xFF9E..=0xFF9F | 0xE0020..=0xE007F),
    }
}

// A mark opening the text has nothing to sit on, so a dotted circle goes before it where the font has one.
pub(super) fn circle_before_leading_mark(run: &mut Run, face: &Face) {
    let settings = run.settings;
    let opens = settings.dotted_circles && settings.starts_text && run.context.before().is_empty();
    if run.len == 0 || !opens || !run.glyphs[0].is_mark_char() || !face.maps(unicode::DOTTED_CIRCLE as u32) {
        return;
    }
    let first = run.glyphs[0];
    let mut circle = Glyph { mask: first.mask, ..Glyph::of_char(unicode::DOTTED_CIRCLE as u32, first.cluster) };
    run.facts.add(circle.read_char());
    if run.room_for(run.len + 1) {
        run.glyphs.copy_within(0..run.len, 1);
        run.glyphs[0] = circle;
        run.len += 1;
    }
}

pub(super) fn form_graphemes(run: &mut Run) {
    if !run.facts.has(Fact::Continuations) {
        return;
    }
    let mut start = 0;
    while start < run.len {
        let end = run.grapheme_end(start);
        run.join_graphemes(start..end);
        start = end;
    }
}

// A run against its script's direction, or bottom to top, turns a grapheme at a time so each keeps its order, the text
// either side trading places; a right-to-left script's run of only digits or regional indicators reads left to right.
pub(super) fn turn_to_native_direction(run: &mut Run) {
    let asked = run.direction;
    let native = run.script.map(unicode::horizontal_direction).map(|native| {
        if native != Direction::RightToLeft || asked != Direction::LeftToRight {
            return native;
        }
        mark_sides_read_otherwise(run);
        if reads_as_number(run) { Direction::LeftToRight } else { native }
    });
    let turn = match native {
        _ if asked.is_vertical() => asked != Direction::TopToBottom,
        Some(native) => asked != native,
        None => false,
    };
    if !turn {
        return;
    }
    let mut start = 0;
    while start < run.len {
        let end = run.grapheme_end(start);
        if run.settings.cluster_level == ClusterLevel::MonotoneCharacters {
            run.join_clusters(start..end);
        }
        run.reverse_span(start..end);
        start = end;
    }
    run.reverse();
    run.context.turn();
    run.direction = asked.reverse();
}

fn reads_as_number(run: &Run) -> bool {
    let glyphs = &run.glyphs[..run.len];
    !glyphs.iter().any(is_letter) && glyphs.iter().any(is_number)
}

// A side of a break that alone would read as a number where the run doesn't, or the other way, turns another way,
// so the break is unsafe.
fn mark_sides_read_otherwise(run: &mut Run) {
    let glyphs = &run.glyphs[..run.len];
    let span = |is: fn(&Glyph) -> bool| Some((glyphs.iter().position(is)?, glyphs.iter().rposition(is)?));
    let (letters, Some(numbers)) = (span(is_letter), span(is_number)) else { return };
    let reads = |letter: bool, number: bool| !letter && number;
    let whole = letters.is_none();
    for k in 1..run.len {
        let head = reads(letters.is_some_and(|l| l.0 < k), numbers.0 < k);
        let tail = reads(letters.is_some_and(|l| l.1 >= k), numbers.1 >= k);
        if head != whole || tail != whole {
            run.unsafe_to_break(k - 1..k + 1);
        }
    }
}

fn is_letter(g: &Glyph) -> bool {
    g.category.is_letter()
}

fn is_number(g: &Glyph) -> bool {
    g.category == GeneralCategory::DecimalNumber || unicode::is_regional_indicator(g.id)
}

// A backward run's characters take their mirrored forms (Bidi_Mirroring_Glyph) where the font has them, else the
// `rtlm` feature; a vertical run without `vert` takes the vertical presentation forms the font has.
pub(super) fn mirror_and_rotate(face: &Face, plan: &Plan, run: &mut Run, target: Direction) {
    let in_font = |id: u32, form: fn(char) -> Option<char>| {
        char::from_u32(id).and_then(form).map(u32::from).filter(|&c| face.maps(c))
    };
    let glyphs = &mut run.glyphs[..run.len];
    if target.is_backward() {
        for glyph in glyphs.iter_mut() {
            match in_font(glyph.id, unicode::mirrored) {
                Some(mirrored) => glyph.id = mirrored,
                None => glyph.mask |= plan.masks.rtlm,
            }
        }
    }
    if target.is_vertical() && !plan.has_vert {
        for glyph in glyphs.iter_mut() {
            glyph.id = in_font(glyph.id, unicode::vertical_form).unwrap_or(glyph.id);
        }
    }
}

// Digits either side of a fraction slash take `numr` before it and `dnom` after it, all of them `frac`, swapped in a
// backward run; a slash without digits on a side is unsafe to concatenate there.
pub(super) fn fraction_masks(plan: &Plan, run: &mut Run) {
    if !run.facts.has(Fact::NonAscii) || !plan.has_fractions {
        return;
    }
    let m = plan.masks;
    let (before, after) = match run.direction.is_backward() {
        true => (m.frac | m.dnom, m.numr | m.frac),
        false => (m.numr | m.frac, m.frac | m.dnom),
    };
    let is_digit = |g: &Glyph| g.category == GeneralCategory::DecimalNumber;
    let mut at = 0;
    while let Some(found) = run.glyphs[at..run.len].iter().position(|g| g.id == 0x2044) {
        let slash = at + found;
        let start = slash - run.glyphs[..slash].iter().rev().take_while(|g| is_digit(g)).count();
        let end = slash + 1 + run.glyphs[slash + 1..run.len].iter().take_while(|g| is_digit(g)).count();
        if start == slash || end == slash + 1 {
            run.unsafe_to_concat(slash..slash + 1);
            at = slash + 1;
            continue;
        }
        run.unsafe_to_break(start..end);
        run.glyphs[start..slash].iter_mut().for_each(|g| g.mask |= before);
        run.glyphs[slash].mask |= m.frac;
        run.glyphs[slash + 1..end].iter_mut().for_each(|g| g.mask |= after);
        at = end;
    }
}
