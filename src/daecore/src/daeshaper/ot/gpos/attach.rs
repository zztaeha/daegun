// Attachments resolve once positioning is done, as a glyph can still move after something attached to it: a mark
// takes its parent's offset along the run and the advances between them, a cursive glyph its offset across.
use alloc::vec::Vec;

use crate::daecore::daeshaper::ot::apply::NESTING_LIMIT;
use crate::daecore::daeshaper::run::{AttachKind, Fact, Placement, Run};

pub(crate) fn resolve_attachments(run: &mut Run) {
    if !run.facts.has(Fact::Attachments) {
        return;
    }
    let backward = run.direction.is_backward();
    let horizontal = run.direction.is_horizontal();
    let places = &mut run.places[..run.len];
    let mut sums = AdvanceSums::default();
    let mut links = [(0, 0); NESTING_LIMIT];
    // In the run's own order, as where a chain is cut, past the nesting limit or at a cycle, depends on where its
    // walk starts.
    for k in 0..places.len() {
        let i = if backward { places.len() - 1 - k } else { k };
        if places[i].attach.to != 0 {
            settle(places, i, (horizontal, backward), &mut links, &mut sums);
        }
    }
}

// Glyph `start` and the chain it hangs from, each link cut as it's read: the parents settle first, root down. A walk
// visits each glyph once, so a link back to one already on it would close a cycle and is dropped, ending the chain.
pub(super) fn settle(
    places: &mut [Placement],
    start: usize,
    (horizontal, backward): (bool, bool),
    links: &mut [(usize, usize)],
    sums: &mut AdvanceSums,
) {
    let mut n = 0;
    let mut child = start;
    loop {
        let to = core::mem::take(&mut places[child].attach.to);
        let parent = child.wrapping_add_signed(isize::from(to));
        let closes = links[..n].iter().any(|&(on_walk, _)| on_walk == parent);
        if parent >= places.len() || n == links.len() || closes {
            break;
        }
        links[n] = (child, parent);
        n += 1;
        if places[parent].attach.to == 0 {
            break;
        }
        child = parent;
    }
    for &(child, parent) in links[..n].iter().rev() {
        let (by_along, by_across) = {
            let p = &mut places[parent];
            (*p.along_mut(horizontal).1, *p.across_mut(horizontal))
        };
        let c = &mut places[child];
        if c.attach.kind == AttachKind::Cursive {
            let across = c.across_mut(horizontal);
            *across = across.saturating_add(by_across);
            continue;
        }
        let along = c.along_mut(horizontal).1;
        *along = along.saturating_add(by_along);
        // The pen lays the advances from the earlier glyph up to the later one: a mark after its base moves back
        // past them, one before it (as kerx may attach) on. A backward run lays them the other way.
        let (low, high) = if parent < child { (parent, child) } else { (child, parent) };
        let span = if backward { low + 1..high + 1 } else { low..high };
        let back = (parent < child) != backward;
        let (x, y) = (places[child].offset_x, places[child].offset_y);
        places[child].offset_x = sums.walk(places, x, Axis::X, back, span.clone());
        places[child].offset_y = sums.walk(places, y, Axis::Y, back, span);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Axis {
    X,
    Y,
}

// x + add, then clamped to lo..=hi: what a run of saturating additions to an i32 comes to, composed into one.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Shift {
    add: i64,
    lo: i32,
    hi: i32,
}

impl Shift {
    pub(super) const NONE: Shift = Shift { add: 0, lo: i32::MIN, hi: i32::MAX };

    pub(super) fn by(add: i64) -> Shift {
        Shift { add, ..Shift::NONE }
    }

    // This shift, then `next`.
    pub(super) fn then(self, next: Shift) -> Shift {
        let through = |v: i32| next.apply(v);
        Shift { add: self.add + next.add, lo: through(self.lo), hi: through(self.hi) }
    }

    pub(super) fn apply(self, x: i32) -> i32 {
        (i64::from(x) + self.add).clamp(i64::from(self.lo), i64::from(self.hi)) as i32
    }
}

// A run of shifts as a tree, each inner node its two children composed in order, so any span composes in log time.
pub(super) struct ShiftTree {
    nodes: Vec<Shift>,
}

impl ShiftTree {
    pub(super) fn new(shifts: impl ExactSizeIterator<Item = Shift>) -> ShiftTree {
        let leaves = shifts.len();
        let mut nodes = alloc::vec![Shift::NONE; leaves];
        nodes.extend(shifts);
        for i in (1..leaves).rev() {
            nodes[i] = nodes[2 * i].then(nodes[2 * i + 1]);
        }
        ShiftTree { nodes }
    }

    // The shifts of `span`, composed in order: the left edge's nodes taken left to right, the right edge's right to
    // left, each joined at the end.
    pub(super) fn span(&self, span: core::ops::Range<usize>) -> Shift {
        let leaves = self.nodes.len() / 2;
        let (mut l, mut r) = (span.start + leaves, span.end + leaves);
        let (mut from_left, mut from_right) = (Shift::NONE, Shift::NONE);
        while l < r {
            if l % 2 == 1 {
                from_left = from_left.then(self.nodes[l]);
                l += 1;
            }
            if r % 2 == 1 {
                r -= 1;
                from_right = self.nodes[r].then(from_right);
            }
            (l, r) = (l / 2, r / 2);
        }
        from_left.then(from_right)
    }
}

// The run's advances as shifts, a tree for each axis and sign, built the first time a long walk needs it.
#[derive(Default)]
pub(super) struct AdvanceSums {
    trees: [Option<ShiftTree>; 4],
}

impl AdvanceSums {
    // A mark's walk is a glyph or two, cheaper one at a time than through a tree; a crafted run walks past this, where
    // walking each glyph in turn would make resolving quadratic.
    pub(super) const SHORT: usize = 32;

    // `x` past the advances of `span` on `axis`, each added or, where `back`, subtracted, saturating at every one.
    pub(super) fn walk(&mut self, places: &[Placement], x: i32, axis: Axis, back: bool, span: core::ops::Range<usize>)
        -> i32 {
        let advance = |p: &Placement| if axis == Axis::Y { p.advance_y } else { p.advance_x };
        if span.len() <= Self::SHORT {
            return places[span].iter().fold(x, |x, p| match back {
                true => x.saturating_sub(advance(p)),
                false => x.saturating_add(advance(p)),
            });
        }
        let slot = usize::from(axis == Axis::Y) * 2 + usize::from(back);
        let tree = self.trees[slot].get_or_insert_with(|| {
            let sign = if back { -1 } else { 1 };
            ShiftTree::new(places.iter().map(|p| Shift::by(sign * i64::from(advance(p)))))
        });
        tree.span(span).apply(x)
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daeshaper/ot/gpos/attach.rs"]
mod tests;
