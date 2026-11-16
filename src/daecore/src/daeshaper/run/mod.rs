// The run being shaped: its glyphs and their placements, what the caller asked of it, what shaping has found of it,
// and the limits that keep a hostile font's work linear in the text.
mod clusters;
mod flags;
mod glyph;
mod rewrite;
mod settings;

use alloc::vec::Vec;

use super::unicode::Script;
pub(crate) use glyph::{class, flag, text, AttachKind, Glyph, Ligature, Mask, Placement};
pub(crate) use settings::{Context, Fact, Facts, Settings, CONTEXT_CHARS};
pub use settings::{ClusterLevel, Direction};

// A run of n glyphs grows to max(256n, 65,536) and spends max(4,096n, 65,536) edits, so a font that loops ends in
// linear work.
const LEN_FACTOR: usize = 256;
const OPS_FACTOR: i64 = 4096;
const LEAST: usize = 65_536;
// As many 20-byte records as a 32-bit byte count holds, so a run stops growing at the same length on every platform.
const MOST_GLYPHS: usize = u32::MAX as usize / 20;

#[derive(Default)]
pub(crate) struct Run {
    // Live up to `len`; past it, room for the run to grow into.
    pub(crate) glyphs: Vec<Glyph>,
    pub(crate) places: Vec<Placement>,
    // Where a rewrite writes once its output would overtake what it still has to read.
    apart: Vec<Glyph>,
    pub(crate) len: usize,
    pub(crate) cursor: usize,
    pub(crate) written: usize,
    output: Output,
    positioned: bool,
    // Set once an edit ran out of room or budget; the glyphs are then left as they stand, or where a rewrite wrote over
    // those it read, as written and still unread.
    pub(crate) failed: bool,
    budget: Budget,
    ligature_serial: u8,
    pub(crate) facts: Facts,
    // What the caller gives: how to shape, the text either side, and the run's script and direction.
    pub(crate) settings: Settings,
    pub(crate) context: Context,
    pub(crate) script: Option<Script>,
    pub(crate) direction: Direction,
    // morx's glyph set, kept between runs so a run doesn't allocate it.
    pub(crate) morx_seen: super::ot::morx::Seen,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Output {
    // No rewrite: edits apply where the glyphs are.
    #[default]
    Off,
    // A rewrite writing over the glyphs it has read, never past the cursor.
    Overlaid,
    // A rewrite writing into `apart`, since a write would have passed the cursor.
    Apart,
}

#[derive(Clone, Copy, Debug)]
struct Budget {
    most_glyphs: usize,
    edits: i64,
}

// Outside a shape nothing is counted.
impl Default for Budget {
    fn default() -> Budget {
        Budget { most_glyphs: usize::MAX, edits: i64::MAX }
    }
}

impl Budget {
    fn for_glyphs(count: usize) -> Budget {
        let edits = i64::try_from(count).unwrap_or(i64::MAX).saturating_mul(OPS_FACTOR);
        Budget { most_glyphs: count.saturating_mul(LEN_FACTOR).max(LEAST), edits: edits.max(LEAST as i64) }
    }
}

impl Run {
    // Empty and as new, its arrays' room kept and their contents dropped, so the next run doesn't start from this one.
    pub(crate) fn reset(&mut self) {
        let mut used = core::mem::take(self);
        [&mut used.glyphs, &mut used.apart].into_iter().for_each(Vec::clear);
        used.places.clear();
        (self.glyphs, self.places, self.apart, self.morx_seen) = (used.glyphs, used.places, used.apart, used.morx_seen);
    }

    pub(crate) fn glyph_capacity(&self) -> usize {
        [self.glyphs.capacity(), self.places.capacity(), self.apart.capacity()].into_iter().max().unwrap_or(0)
    }

    // Each character with its byte offset as its cluster, the unit the API's positions take; room past the text too,
    // so a few inserted or decomposed glyphs don't grow every array mid-shape.
    pub(crate) fn push_str(&mut self, text: &str) {
        let wanted = self.len + text.chars().count() + 32;
        let _ = self.room_for(wanted);
        text.char_indices().for_each(|(offset, c)| self.push(c.into(), offset as u32));
    }

    pub(crate) fn push(&mut self, codepoint: u32, cluster: u32) {
        if self.room_for(self.len + 1) {
            self.glyphs[self.len] = Glyph::of_char(codepoint, cluster);
            self.len += 1;
        }
    }

    pub(crate) fn set_context(&mut self, before: &str, after: &str) {
        self.context = Context::new(before, after);
    }

    // The limits for the glyphs the run starts with, ligature ids from 1 and nothing found yet.
    pub(crate) fn begin(&mut self) {
        (self.budget, self.ligature_serial, self.facts) = (Budget::for_glyphs(self.len), 0, Facts::default());
    }

    pub(crate) fn end(&mut self) {
        (self.budget, self.ligature_serial) = (Budget::default(), 0);
    }

    // Room for `size` glyphs, the glyph array grown by half again plus 32 until it holds them and the placements and
    // output given the same room, filled only once used. False, failing the run, past its limits.
    #[must_use]
    pub(crate) fn room_for(&mut self, size: usize) -> bool {
        let fits = size <= self.budget.most_glyphs;
        if fits && size < self.glyphs.len().max(1) {
            return true;
        }
        match (fits && !self.failed).then(|| self.grown_past(size)).flatten() {
            Some(n) => {
                self.glyphs.resize(n, Glyph::of_char(0, 0));
                if !self.apart.is_empty() {
                    self.apart.resize(n, Glyph::of_char(0, 0));
                }
                true
            }
            None => {
                self.failed = true;
                false
            }
        }
    }

    // The first length along the growth chain past `size`, reserved in all three arrays.
    fn grown_past(&mut self, size: usize) -> Option<usize> {
        let chain = core::iter::successors(Some(self.glyphs.len()), |&n| n.checked_add(n / 2 + 32));
        let n = chain.take_while(|&n| n <= MOST_GLYPHS).find(|&n| n > size)?;
        let reserved = self.glyphs.try_reserve_exact(n - self.glyphs.len()).is_ok()
            && self.places.try_reserve_exact(n - self.places.len()).is_ok()
            && self.apart.try_reserve_exact(n - self.apart.len()).is_ok();
        reserved.then_some(n)
    }

    // Glyphs and placements for `extra` more past the end, for an edit that spreads the run back from its end and
    // sets `len` itself.
    #[must_use]
    pub(crate) fn tail_room(&mut self, extra: usize) -> bool {
        let size = self.len + extra;
        if !self.room_for(size) {
            return false;
        }
        self.places.resize(size, Placement::default());
        true
    }

    // Spends `n` edits, failing the run once none are left.
    pub(crate) fn spend(&mut self, n: usize) -> bool {
        let left = self.take_ops(u32::try_from(n).unwrap_or(u32::MAX));
        // A branch, not `failed |= ...`: loading and storing the flag at every spend cost shaping about 1%.
        if left < 0 {
            self.failed = true;
        }
        left >= 0
    }

    // Takes `n` edits and tells how many are left, for a caller that decides what running out means.
    pub(crate) fn take_ops(&mut self, n: u32) -> i64 {
        self.budget.edits = self.budget.edits.saturating_sub(i64::from(n));
        self.budget.edits
    }

    pub(crate) fn ops_left(&self) -> i64 {
        self.budget.edits
    }

    // `ops_left` as a count: none once spent, and the largest where the edits left pass it, as on 32-bit targets.
    pub(crate) fn ops_room(&self) -> usize {
        usize::try_from(self.budget.edits.max(0)).unwrap_or(usize::MAX)
    }

    // Ids cycle through a byte, skipping 0, which is no ligature.
    pub(crate) fn next_ligature_id(&mut self) -> u8 {
        self.ligature_serial = self.ligature_serial.checked_add(1).unwrap_or(1);
        self.ligature_serial
    }

    // Placements for every glyph, all zero, and no rewrite open. Within the room `room_for` keeps, so it allocates
    // nothing.
    pub(crate) fn start_positions(&mut self) {
        (self.output, self.written, self.positioned) = (Output::Off, 0, true);
        self.places.clear();
        self.places.resize(self.len, Placement::default());
    }

    // The glyphs of `clusters` take `value` within `mask`, spending the run's length.
    pub(crate) fn mask_clusters(&mut self, clusters: core::ops::Range<u32>, mask: Mask, value: Mask) {
        if mask != 0 {
            self.spend(self.len);
            let len = self.len;
            let inside = self.glyphs[..len].iter_mut().filter(|g| clusters.contains(&g.cluster));
            inside.for_each(|g| g.mask ^= (g.mask ^ value) & mask);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/run/mod.rs"]
mod tests;
