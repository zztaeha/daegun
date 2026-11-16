// Mark attachment (GPOS types 4 to 6): a mark's anchor laid on the anchor its class names on the base, ligature
// component or mark before it. The offset is the attachment's alone; the base's own movement is added at the end.
use super::{sole_format, variation_index};
use crate::daecore::daeshaper::face::Deltas;
use crate::daecore::daeshaper::ot::apply::{BaseMemo, Engine, Filter, Present, Side, Step, NESTING_LIMIT};
use crate::daecore::daeshaper::ot::{long, nth_table_width, table_at_width, Coverage};
use crate::daecore::daeshaper::run::{class, AttachKind, Fact, Glyph, Placement, Run};
use crate::daecore::daetype::decoder::{read_u16_be, read_uint};
use crate::daecore::daetype::format::round::ot_round_f32;
use crate::daecore::daetype::format::sanitize::anchor_fits;

// An anchor's point; format 2's contour point is left out, as shaping has no grid-fitted outline. Format 3's device
// deltas stay unrounded, as only the attachment is rounded; a null or unknown anchor sits at the origin.
pub(super) fn anchor(table: &[u8], offset: u32, deltas: Option<&Deltas>) -> (f32, f32) {
    let Some(at) = table.get(offset as usize..).filter(|_| offset != 0) else { return (0.0, 0.0) };
    let [format, x, y] = [0, 2, 4].map(|k| read_u16_be(at, k).unwrap_or(0));
    let point = (f32::from(x as i16), f32::from(y as i16));
    match format {
        1 | 2 => point,
        3 => deltas.map_or(point, |deltas| moved_anchor(at, point, deltas)),
        _ => (0.0, 0.0),
    }
}

// Out of line, as only a location reads devices.
#[cold]
#[inline(never)]
fn moved_anchor(anchor: &[u8], (x, y): (f32, f32), deltas: &Deltas) -> (f32, f32) {
    let delta = |k: usize| match read_u16_be(anchor, k) {
        Some(device @ 1..) => variation_index(anchor, device).map_or(0.0, |(outer, inner)| deltas.exact(outer, inner)),
        _ => 0.0,
    };
    (x + delta(6), y + delta(8))
}

// What a mark lookup attaches the current mark to.
#[derive(Clone, Copy)]
pub(super) enum Onto {
    Base,
    Ligature,
    Mark,
}

// MarkBasePos, MarkLigPos and MarkMarkPos (format 1, or 2 for 24 bits). Where the target or its anchors aren't found,
// the text from where the search stopped is unsafe to concatenate, as more text there could supply one.
pub(super) fn attach<const W: usize>(e: &mut Engine, data: &[u8], onto: Onto) -> bool {
    if read_u16_be(data, 0) != Some(sole_format(W)) {
        return false;
    }
    let l = long(W);
    let Some(index) = e.coverage(data, 2, l).index_of(e.run.current().id) else { return false };
    let Some(classes) = read_u16_be(data, 2 + 2 * l) else { return false };
    let (marks, array) = (table_at_width(data, 4 + 2 * l, l), table_at_width(data, 4 + 3 * l, l));
    let covered = e.rule_coverage(data, 2 + l, l, 0);
    let at = e.run.cursor;
    let target = match onto {
        Onto::Base => find_target(e, takes_marks, &covered).ok_or(0),
        Onto::Ligature => find_target(e, takes_ligature_marks, &covered).ok_or(0),
        Onto::Mark => e.prev_present(e.filter.marks_only()),
    };
    let found = target.and_then(|t| anchors_of::<W>(e.run, onto, &covered, array, t).map(|row| (t, row)).ok_or(t));
    let (target, (table, count, row)) = match found {
        Ok(found) => found,
        Err(from) => {
            e.unsafe_to_concat_across(from, at + 1);
            return false;
        }
    };
    let Some((class, own)) = mark_record::<W>(marks, index) else { return false };
    let Some(cell) = cell(table, (count, W), classes, (row, class)) else { return false };
    lay(e, (marks, own), (table, cell), target)
}

// The table holding `target`'s anchors, the width of its row count, and the row: a ligature's is the component the
// mark sits on, else its last; a mark takes another only where both sit on the same thing or either is ligated whole.
fn anchors_of<'a, const W: usize>(
    run: &Run,
    onto: Onto,
    covered: &Coverage,
    array: &'a [u8],
    target: usize,
) -> Option<(&'a [u8], usize, u32)> {
    let (mark, glyph) = (run.current(), &run.glyphs[target]);
    let row = covered.index_of(glyph.id);
    match onto {
        Onto::Base => Some((array, W, row?)),
        Onto::Ligature => {
            let components = nth_table_width(array, 0, W, W, row?);
            let count = read_u16_be(components, 0).filter(|&n| n != 0)?;
            let id = glyph.ligature_id();
            let on_it = id != 0 && id == mark.ligature_id() && mark.component() > 0;
            let component = if on_it { mark.component().min(count) } else { count };
            Some((components, 2, u32::from(component - 1)))
        }
        Onto::Mark => {
            let (one, other) = ((mark.ligature_id(), mark.component()), (glyph.ligature_id(), glyph.component()));
            let together = match one.0 == other.0 {
                true => one.0 == 0 || one.1 == other.1,
                false => (one.0 > 0 && one.1 == 0) || (other.0 > 0 && other.1 == 0),
            };
            (together && glyph.glyph_is(class::MARK)).then_some((array, 2, row?))
        }
    }
}

// Mark `index`'s class and anchor offset in the MarkArray; none past the array or for a null or unfit anchor.
fn mark_record<const W: usize>(marks: &[u8], index: u32) -> Option<(u16, u32)> {
    let at = W + (index as usize).checked_mul(2 + W)?;
    let offset = read_uint(marks, at + 2, W).filter(|&o| o != 0 && anchor_fits(marks, o))?;
    (index < read_uint(marks, 0, W)?).then_some((read_u16_be(marks, at)?, offset))
}

// The anchor offset for `class` in `row` of a BaseArray, LigatureAttach or Mark2Array; none for a missing, null or
// unfit anchor.
fn cell(table: &[u8], (count, width): (usize, usize), classes: u16, (row, class): (u32, u16)) -> Option<u32> {
    if row >= read_uint(table, 0, count).unwrap_or(0) || class >= classes {
        return None;
    }
    let k = u64::from(row) * u64::from(classes) + u64::from(class);
    let at = usize::try_from(k * width as u64 + count as u64).ok()?;
    read_uint(table, at, width).filter(|&offset| offset != 0 && anchor_fits(table, offset))
}

// The current mark's anchor at `mark` laid on glyph `target`'s at `onto`, each a table and an offset in it. A target
// further back than an i16 reaches is matched but leaves the mark where it is.
fn lay(e: &mut Engine, mark: (&[u8], u32), onto: (&[u8], u32), target: usize) -> bool {
    let deltas = e.deltas;
    let point = |(table, offset): (&[u8], u32)| anchor(table, offset, deltas);
    let ((mx, my), (tx, ty)) = (point(mark), point(onto));
    let at = e.run.cursor;
    e.run.unsafe_to_break(target..at + 1);
    e.run.cursor += 1;
    let Ok(to) = i16::try_from(target as isize - at as isize) else {
        e.run.places[at].attach.to = 0;
        return true;
    };
    let horizontal = e.run.direction.is_horizontal();
    let (mut dx, mut dy) = (ot_round_f32(tx - mx), ot_round_f32(ty - my));
    let across = if horizontal { &mut dy } else { &mut dx };
    *across = across.saturating_add(cross_stream(&e.run.places[..e.run.len], target, horizontal));
    let p = &mut e.run.places[at];
    (p.offset_x, p.offset_y) = (dx, dy);
    (p.attach.to, p.attach.kind) = (to, AttachKind::Mark);
    e.run.facts.note(Fact::Attachments);
    true
}

// Glyph `at`'s offset across the run, with that of each glyph up its cursive chain as deep as lookups nest, each once:
// a link back onto the walk closes a cycle and ends it. A mark takes this when it attaches, as it moves only along.
pub(super) fn cross_stream(places: &[Placement], at: usize, horizontal: bool) -> i32 {
    let across = |p: &Placement| if horizontal { p.offset_y } else { p.offset_x };
    let parent_of = |i: usize| {
        let link = places[i].attach;
        let parent = i.wrapping_add_signed(isize::from(link.to));
        (link.kind == AttachKind::Cursive && link.to != 0 && parent < places.len()).then_some(parent)
    };
    let mut total = across(&places[at]);
    let Some(mut i) = parent_of(at) else { return total };
    let mut walked = [at; NESTING_LIMIT + 1];
    for n in 1..=NESTING_LIMIT {
        if walked[..n].contains(&i) {
            break;
        }
        walked[n] = i;
        total = total.saturating_add(across(&places[i]));
        let Some(parent) = parent_of(i) else { break };
        i = parent;
    }
    total
}

// Whether a glyph can take marks as a base: not a piece a multiple substitution made past its first, as marks go on
// the first piece.
fn takes_marks(run: &Run, i: usize) -> bool {
    let g = &run.glyphs[i];
    let later_piece = |prev: &Glyph| {
        !prev.glyph_is(class::MARK)
            && prev.glyph_is(class::MULTIPLIED)
            && prev.ligature_id() == g.ligature_id()
            && prev.component().checked_add(1) == Some(g.component())
    };
    !(g.glyph_is(class::MULTIPLIED) && g.component() != 0 && i > 0 && later_piece(&run.glyphs[i - 1]))
}

// Whether a glyph can take marks as a ligature: not a piece of a multiple substitution.
fn takes_ligature_marks(run: &Run, i: usize) -> bool {
    let g = &run.glyphs[i];
    !g.glyph_is(class::MULTIPLIED) || g.component() == 0
}

// The nearest non-mark before the current mark that `takes` accepts or the subtable covers. It reads back only to
// where the last search started, so a base's run of marks costs one read each rather than a walk each.
fn find_target(e: &mut Engine, takes: fn(&Run, usize) -> bool, covered: &Coverage) -> Option<usize> {
    let cursor = e.run.cursor;
    if e.base.from > cursor {
        return search_again(e, takes, covered);
    }
    let reading = e.reading(Side::Input).with_filter(Filter::of_classes(class::MARK));
    let (run, face) = (&*e.run, e.face);
    let found = (e.base.from..cursor).rev().find(|&i| {
        let g = &run.glyphs[i];
        reading.step(g, face, &Present) == Step::Take && (takes(run, i) || covered.contains(g.id))
    });
    e.base = BaseMemo { found: found.or(e.base.found), from: cursor };
    e.base.found
}

// A nested record seeking back before the last search starts it over from the run's start. Searches after it read
// again what it steps back over, so the run pays for that and for the walk, read only as far back as it can pay.
#[cold]
#[inline(never)]
fn search_again(e: &mut Engine, takes: fn(&Run, usize) -> bool, covered: &Coverage) -> Option<usize> {
    let cursor = e.run.cursor;
    e.run.spend(e.base.from - cursor);
    e.base = BaseMemo { found: None, from: cursor.saturating_sub(e.run.ops_room()) };
    let found = find_target(e, takes, covered);
    e.run.spend(cursor - found.unwrap_or(0));
    found
}

#[cfg(test)]
#[path = "../../../../tests/unit/daeshaper/ot/gpos/marks.rs"]
mod tests;
