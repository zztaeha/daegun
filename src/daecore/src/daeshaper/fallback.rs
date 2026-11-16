// Where a font doesn't: spaces without a glyph take their own widths, and marks GPOS didn't place go by combining
// class. That placement is daegun's own rule, not a specification's: each mark sits where its class says, stacked.
use super::face::Face;
use super::plan::Plan;
use super::run::{class, text, Direction, Glyph, Run};
use super::unicode::{self, GeneralCategory, SpaceFallback};
use crate::daecore::daekit::cache::GlyphExtents;

// Canonical combining classes that name a position (UAX #44, Canonical_Combining_Class).
pub(crate) mod ccc {
    pub(crate) const ATTACHED_BELOW_LEFT: u8 = 200;
    pub(crate) const ATTACHED_BELOW: u8 = 202;
    pub(crate) const ATTACHED_ABOVE: u8 = 214;
    pub(crate) const ATTACHED_ABOVE_RIGHT: u8 = 216;
    pub(crate) const BELOW_LEFT: u8 = 218;
    pub(crate) const BELOW: u8 = 220;
    pub(crate) const BELOW_RIGHT: u8 = 222;
    pub(crate) const ABOVE_LEFT: u8 = 228;
    pub(crate) const ABOVE: u8 = 230;
    pub(crate) const ABOVE_RIGHT: u8 = 232;
    pub(crate) const DOUBLE_BELOW: u8 = 233;
    pub(crate) const DOUBLE_ABOVE: u8 = 234;
}

// Each nonspacing mark's class made one that names where it sits, for the classes Unicode gives by script rather
// than by position.
pub(crate) fn recategorize_marks(run: &mut Run) {
    for g in run.glyphs[..run.len].iter_mut().filter(|g| g.category == GeneralCategory::NonspacingMark) {
        let positioned = positional_class(g.id, g.mark_class());
        g.set_mark_class(positioned);
    }
}

fn positional_class(c: u32, class: u8) -> u8 {
    if class >= ccc::ATTACHED_BELOW_LEFT {
        return class;
    }
    by_position(thai_lao(c, class))
}

// Thai and Lao marks Unicode puts in class 0 (and Thai's phinthu, a virama), by where they sit on the letter.
fn thai_lao(c: u32, class: u8) -> u8 {
    match (c, class) {
        (0x0E31 | 0x0E34..=0x0E37 | 0x0E47 | 0x0E4C..=0x0E4E, 0) => ccc::ABOVE_RIGHT,
        (0x0EB1 | 0x0EB4..=0x0EB7 | 0x0EBB | 0x0ECC | 0x0ECD, 0) => ccc::ABOVE,
        (0x0EBC, 0) => ccc::BELOW,
        (0x0E3A, 1..) => ccc::BELOW_RIGHT,
        _ => class,
    }
}

// Where each script-numbered class sits, the classes as `unicode::mark_class` gives them: Hebrew's, Arabic's, Thai's
// and Tibetan's renumbered so marks sort in the order fonts expect. Any other class keeps its number.
fn by_position(class: u8) -> u8 {
    use ccc::{ABOVE, ABOVE_LEFT, ABOVE_RIGHT, ATTACHED_ABOVE, BELOW, BELOW_RIGHT};
    match class {
        // Hebrew: shin dot; sin dot and holam; rafe; the points under the letter; varika. Dagesh, inside it, stays.
        10 => ABOVE_RIGHT,
        11 | 14 => ABOVE_LEFT,
        13 => ATTACHED_ABOVE,
        15..=25 => BELOW,
        26 => ABOVE,
        // Arabic: kasratan and kasra below, the other harakat above, as is Syriac's superscript alaph.
        30 | 33 => BELOW,
        27..=36 => ABOVE,
        // Thai sara u and uu, then the tone marks.
        3 => BELOW_RIGHT,
        107 => ABOVE_RIGHT,
        // Lao u and uu, then the tone marks.
        118 => BELOW,
        122 => ABOVE,
        // Tibetan aa and u below, i above.
        129 | 131 => BELOW,
        132 => ABOVE,
        other => other,
    }
}

// A space the font has no glyph for stands in as the plain space or the invisible glyph, so each kind takes the width
// its character has: a fraction of the em, a figure's or punctuation's width, or half the space it stands in as.
pub(crate) fn adjust_spaces(face: &Face, run: &mut Run) {
    let horizontal = run.direction.is_horizontal();
    let upm = i32::from(face.upm());
    let along = |width: i32| if horizontal { width } else { -width };
    let invisible = run.settings.invisible;
    for (g, p) in run.glyphs[..run.len].iter().zip(&mut run.places[..run.len]) {
        if !g.is_space_char() || g.glyph_is(class::LIGATED) {
            continue;
        }
        let advance = p.along_mut(horizontal).0;
        // The invisible glyph has no width of its own to give.
        if invisible == Some(g.id) {
            *advance = along(upm / 4);
        }
        let width = match g.space_fallback() {
            None | Some(SpaceFallback::Space) => continue,
            Some(SpaceFallback::EmDiv(n)) => {
                let n = i32::from(n.max(1));
                (upm + n / 2) / n
            }
            Some(SpaceFallback::Em4Of18) => upm * 4 / 18,
            Some(SpaceFallback::Narrow) => {
                *advance /= 2;
                continue;
            }
            Some(kind @ (SpaceFallback::Figure | SpaceFallback::Punctuation)) => {
                let Some(source) = kind.width_sources().iter().find_map(|&c| face.glyph_of(c as u32)) else { continue };
                if horizontal { face.width(source) } else { face.height(source) }
            }
        };
        *advance = along(width);
    }
}

// Each cluster of a base and the marks after it: the marks laid about the base's ink, where the font has it, else
// just given no advance. `shift_offsets` moves a mark whose advance goes back by that advance.
pub(crate) fn position_marks(plan: &Plan, face: &Face, run: &mut Run, shift_offsets: bool) {
    let mut start = 0;
    while start < run.len {
        let end = (start + 1..run.len).find(|&i| !stays_in_cluster(&run.glyphs[i])).unwrap_or(run.len);
        let base = (start..end).find(|&i| !run.glyphs[i].is_mark_char());
        if let Some(base) = base.filter(|_| end - start > 1) {
            place_about(plan, face, run, base..end, shift_offsets);
        }
        start = end;
    }
}

// Marks stay with the base before them, as do the glyphs a run hides or doesn't draw (morx leaves its deletions so).
fn stays_in_cluster(g: &Glyph) -> bool {
    g.is_mark_char() || g.char_is(text::HIDDEN) || g.is_ignorable()
}

// The room marks stack into about a base, worked out in 64 bits: across, its advance's span (or a ligature
// component's share of it); up and down, its ink's top and bottom, which each mark placed moves out.
#[derive(Clone, Copy, Debug)]
struct Room {
    left: i64,
    width: i64,
    top: i64,
    bottom: i64,
}

// What's worked out in 64 bits, clamped back to a position.
fn clamped(v: i64) -> i32 {
    v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

fn place_about(plan: &Plan, face: &Face, run: &mut Run, cluster: core::ops::Range<usize>, shift_offsets: bool) {
    let (base, end) = (cluster.start, cluster.end);
    run.unsafe_to_break(cluster.clone());
    let base_glyph = run.glyphs[base];
    let Some(ink) = face.extents(base_glyph.id) else {
        let marks = run.glyphs[base + 1..end].iter().zip(&mut run.places[base + 1..end]);
        for (_, p) in marks.filter(|(g, _)| g.category == GeneralCategory::NonspacingMark) {
            p.zero_advance(shift_offsets);
        }
        return;
    };
    let top = i64::from(ink.y_bearing.saturating_add(run.places[base].offset_y));
    let whole = Room { left: 0, width: i64::from(face.width(base_glyph.id)), top, bottom: top + i64::from(ink.height) };

    let direction = run.direction;
    let backward = direction.is_backward();
    // Where the pen stands, from the base's origin, as the glyphs between the base and a mark advance it.
    let mut pen = match backward {
        true => (0i32, 0i32),
        false => (run.places[base].advance_x.saturating_neg(), run.places[base].advance_y.saturating_neg()),
    };
    let gap = i32::from(face.upm()) / 16;
    // A ligature's components share its advance in the order its script writes them.
    let components = i64::from(base_glyph.components());
    let writes_left_to_right = match plan.direction.is_horizontal() {
        true => plan.direction == Direction::LeftToRight,
        false => run.script.map_or(Direction::LeftToRight, unicode::horizontal_direction) == Direction::LeftToRight,
    };
    let mut component = None;
    let mut part = whole;
    let mut stack: Option<(u8, Room)> = None;
    for i in base + 1..end {
        let mark_class = run.glyphs[i].mark_class();
        if mark_class == 0 {
            let p = &run.places[i];
            pen = match backward {
                true => (pen.0.saturating_add(p.advance_x), pen.1.saturating_add(p.advance_y)),
                false => (pen.0.saturating_sub(p.advance_x), pen.1.saturating_sub(p.advance_y)),
            };
            continue;
        }
        if components > 1 {
            let mark = &run.glyphs[i];
            let own = base_glyph.ligature_id() != 0 && mark.ligature_id() == base_glyph.ligature_id();
            let on = mark.component().checked_sub(1).map(i64::from).filter(|&c| own && c < components);
            let on = on.unwrap_or(components - 1);
            if component != Some(on) {
                component = Some(on);
                stack = None;
                let nth = if writes_left_to_right { on } else { components - 1 - on };
                let left = whole.left + nth * whole.width / components;
                part = Room { left, width: whole.width / components, ..whole };
            }
        }
        // Marks of one class stack on each other; a mark of another class starts from the base again.
        let room = match stack {
            Some((stacked, room)) if stacked == mark_class => room,
            _ => part,
        };
        let mut room = room;
        if let Some(mark) = face.extents(run.glyphs[i].id) {
            let p = &mut run.places[i];
            (p.offset_x, p.offset_y) = lay(mark, mark_class, &mut room, gap, direction);
        }
        stack = Some((mark_class, room));
        let p = &mut run.places[i];
        (p.advance_x, p.advance_y) = (0, 0);
        (p.offset_x, p.offset_y) = (p.offset_x.saturating_add(pen.0), p.offset_y.saturating_add(pen.1));
    }
}

// The offset laying `mark` against `room`'s edge by its class, `gap` clear unless the class attaches; it grows the
// room. A mark already past the edge stays, and one above that would have to come down comes halfway.
fn lay(mark: GlyphExtents, class: u8, room: &mut Room, gap: i32, direction: Direction) -> (i32, i32) {
    use ccc::*;
    let (x_bearing, y_bearing) = (i64::from(mark.x_bearing), i64::from(mark.y_bearing));
    let (width, height) = (i64::from(mark.width), i64::from(mark.height));
    let x = match class {
        DOUBLE_BELOW | DOUBLE_ABOVE if direction.is_horizontal() => {
            let edge = if direction.is_backward() { 0 } else { room.width };
            room.left + edge - i64::from(mark.width / 2) - x_bearing
        }
        ATTACHED_BELOW_LEFT | BELOW_LEFT | ABOVE_LEFT => room.left - x_bearing,
        ATTACHED_ABOVE_RIGHT | BELOW_RIGHT | ABOVE_RIGHT => room.left + room.width - width - x_bearing,
        _ => room.left + (room.width - width) / 2 - x_bearing,
    };
    let attached = matches!(class, ATTACHED_BELOW_LEFT | ATTACHED_BELOW | ATTACHED_ABOVE | ATTACHED_ABOVE_RIGHT);
    // Which way a mark counts as already out reads the face's gap, attached or not.
    let apart = gap > 0;
    let gap = i64::from(if attached { 0 } else { gap });
    let y = match class {
        DOUBLE_BELOW | BELOW_LEFT | BELOW | BELOW_RIGHT | ATTACHED_BELOW_LEFT | ATTACHED_BELOW => {
            room.bottom -= gap;
            let mut y = room.bottom - y_bearing;
            if (y > 0) == apart {
                room.bottom = y_bearing;
                y = 0;
            }
            room.bottom += height;
            y
        }
        DOUBLE_ABOVE | ABOVE_LEFT | ABOVE | ABOVE_RIGHT | ATTACHED_ABOVE | ATTACHED_ABOVE_RIGHT => {
            room.top += gap;
            let mut y = room.top - y_bearing - height;
            if (y > 0) != apart {
                let halfway = -(clamped(y) / 2);
                room.top += i64::from(halfway);
                y += i64::from(halfway);
            }
            room.top -= height;
            y
        }
        _ => 0,
    };
    (clamped(x), clamped(y))
}

#[cfg(test)]
#[path = "../../tests/unit/daeshaper/fallback.rs"]
mod tests;
