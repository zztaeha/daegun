// The flags a shaped run reports. Breaking the text, or joining two runs, inside a span an edit read could change its
// glyphs; a tatweel fits where Arabic joining joined.
use core::ops::Range;

use super::{flag, ClusterLevel, Fact, Glyph, Mask, Run};

// Marking costs a span's length, so past this, which only hundreds of marks on one base or an edit reaching far back
// make, a span unsafe to break or join marks the whole run once it settles and the run stays linear.
const MOST_MARKED: usize = 255;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    // Every glyph of the span.
    Every,
    // Every glyph of the span but those of its least cluster, which the edit's result stays with.
    PastLeast,
}

// One side of a span: written glyphs, or the run's own.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Written,
    Unread,
}

impl Run {
    pub(crate) fn unsafe_to_break(&mut self, span: Range<usize>) {
        self.flag_span(span.start, span.end, false, flag::UNSAFE_TO_BREAK | flag::UNSAFE_TO_CONCAT, Reach::PastLeast);
    }

    // A span from written glyph `start` across the cursor to unread glyph `end`.
    pub(crate) fn unsafe_to_break_across(&mut self, start: usize, end: usize) {
        self.flag_span(start, end, true, flag::UNSAFE_TO_BREAK | flag::UNSAFE_TO_CONCAT, Reach::PastLeast);
    }

    // Most shapes don't ask for it, so the check stays inline at every caller and the marking out of line.
    pub(crate) fn unsafe_to_concat(&mut self, span: Range<usize>) {
        if self.settings.reports(flag::UNSAFE_TO_CONCAT) {
            self.concat_span(span.start, span.end, false);
        }
    }

    pub(crate) fn unsafe_to_concat_across(&mut self, start: usize, end: usize) {
        if self.settings.reports(flag::UNSAFE_TO_CONCAT) {
            self.concat_span(start, end, true);
        }
    }

    // The whole run unsafe to break past its least cluster, as a random alternate makes it.
    pub(crate) fn unsafe_to_break_all(&mut self) {
        self.unsafe_to_break(0..self.len);
    }

    pub(crate) fn unsafe_to_concat_all(&mut self) {
        if self.settings.reports(flag::UNSAFE_TO_CONCAT) {
            self.facts.note(Fact::Flagged);
            let len = self.len;
            self.glyphs[..len].iter_mut().for_each(|g| g.mask |= flag::UNSAFE_TO_CONCAT);
        }
    }

    pub(crate) fn safe_to_insert_tatweel(&mut self, span: Range<usize>) {
        if self.settings.reports(flag::SAFE_TO_INSERT_TATWEEL) {
            self.flag_span(span.start, span.end, false, flag::SAFE_TO_INSERT_TATWEEL, Reach::PastLeast);
        } else {
            self.unsafe_to_break(span);
        }
    }

    #[inline(never)]
    fn concat_span(&mut self, start: usize, end: usize, across: bool) {
        self.flag_span(start, end, across, flag::UNSAFE_TO_CONCAT, Reach::Every);
    }

    fn flag_span(&mut self, start: usize, end: usize, across: bool, bits: Mask, reach: Reach) {
        // Across a rewrite each side counts its own glyphs, as an edit may have grown or shrunk what's written; a span
        // ending before its start marks nothing.
        let rewriting = across && self.is_rewriting();
        let len = match rewriting {
            true => self.written.checked_sub(start).zip(end.checked_sub(self.cursor)).map(|(w, u)| w + u),
            false => end.checked_sub(start),
        };
        let Some(len) = len else { return };
        if len > MOST_MARKED {
            // Past the cap the run is marked whole as it settles; a tatweel left out of a span is only a lost place.
            if bits & flag::UNSAFE_TO_BREAK != 0 {
                self.facts.note(Fact::Unbreakable);
            } else if bits & flag::UNSAFE_TO_CONCAT != 0 {
                self.facts.note(Fact::Unjoinable);
            }
            self.facts.note(Fact::Flagged);
            return;
        }
        let end = end.min(self.len);
        if !across && reach == Reach::PastLeast && end.saturating_sub(start) < 2 {
            return;
        }
        self.facts.note(Fact::Flagged);
        let parts = match rewriting {
            true => [(Side::Written, start..self.written), (Side::Unread, self.cursor..end)],
            false => [(Side::Unread, start..end), (Side::Unread, 0..0)],
        };
        let least = match reach {
            Reach::Every => None,
            Reach::PastLeast => Some(parts.iter().fold(u32::MAX, |least, (side, part)| {
                self.least_of(*side, part.clone()).map_or(least, |c| c.min(least))
            })),
        };
        for (side, part) in parts {
            self.mark(side, part, bits, least);
        }
    }

    fn side(&mut self, side: Side) -> &mut [Glyph] {
        match side {
            Side::Written => self.out_mut(),
            Side::Unread => &mut self.glyphs,
        }
    }

    // A part's least cluster: at one of its ends unless characters may stand in any order.
    fn least_of(&self, side: Side, part: Range<usize>) -> Option<u32> {
        if part.is_empty() {
            return None;
        }
        let glyphs = match side {
            Side::Written => &self.out()[part],
            Side::Unread => &self.glyphs[part],
        };
        match self.settings.cluster_level {
            ClusterLevel::Characters => glyphs.iter().map(|g| g.cluster).min(),
            _ => Some(glyphs[0].cluster.min(glyphs[glyphs.len() - 1].cluster)),
        }
    }

    // Marks a part but its glyphs of cluster `least`. At an ordered level those stand together at one end, so only the
    // glyphs past the nearest of them are taken; at the character level they may stand anywhere.
    fn mark(&mut self, side: Side, part: Range<usize>, bits: Mask, least: Option<u32>) {
        if part.is_empty() {
            return;
        }
        let Some(least) = least else {
            self.side(side)[part].iter_mut().for_each(|g| g.mask |= bits);
            return;
        };
        self.spend(part.len());
        let anywhere = self.settings.cluster_level == ClusterLevel::Characters;
        let glyphs = &mut self.side(side)[part];
        let n = glyphs.len();
        let others = |g: &&mut Glyph| g.cluster != least;
        let taken = if anywhere || (glyphs[0].cluster != least && glyphs[n - 1].cluster != least) {
            0..n
        } else if glyphs[0].cluster == least {
            n - glyphs.iter_mut().rev().take_while(others).count()..n
        } else {
            0..glyphs.iter_mut().take_while(others).count()
        };
        glyphs[taken].iter_mut().filter(others).for_each(|g| g.mask |= bits);
    }

    // Each cluster's glyphs carry the union of its flags. Where tatweel is reported, a cluster unsafe to break takes
    // none, and one that takes it is unsafe both ways. Feature bits go, as nothing reads them past here.
    pub(crate) fn settle_flags(&mut self) {
        if !self.facts.has(Fact::Flagged) {
            return;
        }
        let len = self.len;
        if self.facts.has(Fact::Unbreakable) {
            let least = self.least_of(Side::Unread, 0..len);
            self.mark(Side::Unread, 0..len, flag::UNSAFE_TO_BREAK | flag::UNSAFE_TO_CONCAT, least);
        }
        if self.facts.has(Fact::Unjoinable) {
            self.mark(Side::Unread, 0..len, flag::UNSAFE_TO_CONCAT, None);
        }
        let reported = self.settings.reported;
        let tatweel = self.settings.reports(flag::SAFE_TO_INSERT_TATWEEL);
        for cluster in self.glyphs[..len].chunk_by_mut(|a, b| a.cluster == b.cluster) {
            let union = cluster.iter().fold(0, |bits, g| bits | g.mask);
            let bits = (if tatweel { with_tatweel(union) } else { union }) & reported;
            cluster.iter_mut().for_each(|g| g.mask = bits);
        }
    }
}

fn with_tatweel(bits: Mask) -> Mask {
    let bits = if bits & flag::UNSAFE_TO_BREAK != 0 { bits & !flag::SAFE_TO_INSERT_TATWEEL } else { bits };
    match bits & flag::SAFE_TO_INSERT_TATWEEL {
        0 => bits,
        _ => bits | flag::UNSAFE_TO_BREAK | flag::UNSAFE_TO_CONCAT,
    }
}
