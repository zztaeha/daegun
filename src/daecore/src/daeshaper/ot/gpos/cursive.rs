// Cursive attachment (GPOS type 3): the current glyph's entry anchor meets the exit anchor of the glyph before it.
// Along the run the advances join the two; across it one glyph hangs from the other, the chain resolved at the end.
use super::marks::anchor;
use super::sole_format;
use crate::daecore::daeshaper::ot::apply::{Engine, NESTING_LIMIT};
use crate::daecore::daeshaper::ot::long;
use crate::daecore::daeshaper::run::{AttachKind, Fact, Placement};
use crate::daecore::daetype::decoder::{read_u16_be, read_uint};
use crate::daecore::daetype::format::round::ot_round_f32;
use crate::daecore::daetype::format::sanitize::anchor_fits;

// Format 1, or 2 for the 24-bit form, whose EntryExitRecords hold 24-bit offsets counted in 24 bits.
pub(super) fn attach<const W: usize>(e: &mut Engine, data: &[u8]) -> bool {
    if read_u16_be(data, 0) != Some(sole_format(W)) {
        return false;
    }
    let l = long(W);
    let coverage = e.coverage(data, 2, l);
    let Some(count) = read_uint(data, 2 + l, W) else { return false };
    // A glyph's entry (0) or exit (1) anchor; 0, none, for one the records don't cover.
    let anchor_of = |glyph: u32, side: usize| {
        let record = coverage.index_of(glyph).filter(|&i| i < count);
        record.and_then(|i| read_uint(data, 2 + l + W + (i as usize * 2 + side) * W, W)).unwrap_or(0)
    };
    let usable = |offset: u32| offset != 0 && anchor_fits(data, offset);

    let at = e.run.cursor;
    let entry = anchor_of(e.run.current().id, 0);
    if !usable(entry) {
        return false;
    }
    let prev = match e.prev_present(e.filter) {
        Ok(prev) => prev,
        Err(from) => {
            e.unsafe_to_concat_across(from, at + 1);
            return false;
        }
    };
    let exit = anchor_of(e.run.glyphs[prev].id, 1);
    if !usable(exit) {
        e.unsafe_to_concat_across(prev, at + 1);
        return false;
    }
    let (exit, entry) = (anchor(data, exit, e.deltas), anchor(data, entry, e.deltas));
    e.run.unsafe_to_break(prev..at + 1);

    let direction = e.run.direction;
    let horizontal = direction.is_horizontal();
    let (head, tail) = e.run.places.split_at_mut(at);
    let (before, this) = (&mut head[prev], &mut tail[0]);
    let along = |point: (f32, f32)| ot_round_f32(if horizontal { point.0 } else { point.1 });
    let (before_advance, before_offset) = before.along_mut(horizontal);
    let (advance, offset) = this.along_mut(horizontal);
    // Forward, the glyph before ends at its exit and this one starts at its entry; backward, the other way about.
    if direction.is_backward() {
        let shift = along(exit).saturating_add(*before_offset);
        *before_advance = before_advance.saturating_sub(shift);
        *before_offset = before_offset.saturating_sub(shift);
        *advance = along(entry).saturating_add(*offset);
    } else {
        *before_advance = along(exit).saturating_add(*before_offset);
        let shift = along(entry).saturating_add(*offset);
        *advance = advance.saturating_sub(shift);
        *offset = offset.saturating_sub(shift);
    }

    // The chain's first glyph keeps the baseline and each later one hangs from the one before it, unless the lookup
    // keeps the last on it instead.
    let rise = ot_round_f32(if horizontal { entry.1 - exit.1 } else { entry.0 - exit.0 });
    let (child, parent, rise) = match e.filter.last_on_baseline {
        true => (prev, at, rise),
        false => (at, prev, rise.saturating_neg()),
    };
    let len = e.run.len;
    let places = &mut e.run.places[..len];
    reroot(places, child, parent, horizontal);
    e.run.cursor += 1;
    let Ok(to) = i16::try_from(parent as isize - child as isize) else {
        places[child].attach.to = 0;
        return true;
    };
    (places[child].attach.to, places[child].attach.kind) = (to, AttachKind::Cursive);
    *places[child].across_mut(horizontal) = rise;
    // A parent that hung from its new child no longer does.
    if i32::from(places[parent].attach.to) == -i32::from(to) {
        places[parent].attach.to = 0;
        *places[parent].across_mut(horizontal) = 0;
    }
    e.run.facts.note(Fact::Attachments);
    true
}

// A glyph about to hang from a new parent first becomes the root of its cursive chain: each link up from it turns
// around, its offset across negated, until the new parent, the chain's end, or a link back onto the walk (a cycle).
pub(super) fn reroot(places: &mut [Placement], glyph: usize, new_parent: usize, horizontal: bool) {
    let hangs = |p: &Placement| p.attach.to != 0 && p.attach.kind == AttachKind::Cursive;
    if !hangs(&places[glyph]) {
        return;
    }
    let mut links = [(0, 0, 0); NESTING_LIMIT + 1];
    let mut n = 0;
    let mut child = glyph;
    while n < links.len() && hangs(&places[child]) {
        let to = core::mem::take(&mut places[child].attach.to);
        let parent = child.wrapping_add_signed(isize::from(to));
        let closes = links[..n].iter().any(|&(on_walk, ..)| on_walk == parent);
        let Some(back) = to.checked_neg().filter(|_| parent < places.len() && parent != new_parent && !closes) else {
            break;
        };
        links[n] = (child, parent, back);
        n += 1;
        child = parent;
    }
    for &(child, parent, back) in links[..n].iter().rev() {
        let across = places[child].across_mut(horizontal).saturating_neg();
        *places[parent].across_mut(horizontal) = across;
        (places[parent].attach.to, places[parent].attach.kind) = (back, AttachKind::Cursive);
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daeshaper/ot/gpos/cursive.rs"]
mod tests;
