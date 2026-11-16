use alloc::vec::Vec;
use crate::daecore::daeshaper::run::{Fact, Run, Glyph};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::generated::syllable_tables::DEAD;
use crate::daecore::daeshaper::plan::Plan;

// The most glyphs a syllable is reordered as one: a longer syllable is reordered in parts of this many, as fonts are
// built to expect, and the pass stays linear in the run.
const MAX_SYLLABLE_LENGTH: usize = 64;

// Dead ends as `position << 8 | state`, sorted, the front dropped as the scan's start passes it. The first `N` live
// inline, so only a pathological run allocates.
struct DeadEnds<const N: usize> {
    inline: [u64; N],
    head: usize,
    len: usize,
    spill: Vec<u64>,
}

fn key(position: usize, state: u8) -> u64 {
    (position as u64) << 8 | u64::from(state)
}

impl<const N: usize> DeadEnds<N> {
    fn new() -> Self {
        DeadEnds { inline: [0; N], head: 0, len: 0, spill: Vec::new() }
    }

    fn keys(&self) -> &[u64] {
        if self.spill.is_empty() { &self.inline[self.head..self.len] } else { &self.spill }
    }

    fn contains(&self, key: u64) -> bool {
        self.keys().binary_search(&key).is_ok()
    }

    fn insert(&mut self, key: u64) {
        let Err(k) = self.keys().binary_search(&key) else { return };
        if !self.spill.is_empty() {
            self.spill.insert(k, key);
            return;
        }
        if self.len == N {
            if self.head == 0 {
                self.spill.reserve(N * 2);
                self.spill.extend_from_slice(&self.inline);
                self.spill.insert(k, key);
                self.len = 0;
                return;
            }
            self.inline.copy_within(self.head..self.len, 0);
            self.len -= self.head;
            self.head = 0;
        }
        let at = self.head + k;
        if at < self.len {
            self.inline.copy_within(at..self.len, at + 1);
        }
        self.inline[at] = key;
        self.len += 1;
    }

    fn drop_before(&mut self, key: u64) {
        let n = self.keys().partition_point(|&k| k < key);
        if !self.spill.is_empty() {
            self.spill.drain(..n);
        } else if self.head + n == self.len {
            (self.head, self.len) = (0, 0);
        } else {
            self.head += n;
        }
    }
}

impl Glyph {
    pub(crate) fn syllable_kind(&self) -> u8 {
        self.syllable & 0x0F
    }
}

// Marks each syllable with its kind and a serial cycling 1 to 15, so neighbors never match, flags the run when one
// is broken, and keeps each from breaking.
pub(crate) fn setup<const N: usize, T: Copy + Into<u8>>(
    buffer: &mut Run,
    transitions: &[[u8; N]],
    category_of: impl Fn(&[Glyph], usize) -> Option<u8>,
    accept: impl Fn(u8) -> Option<T>,
    broken: u8,
) {
    let len = buffer.len;
    if segment(&mut buffer.glyphs[..len], transitions, category_of, accept) & 1 << broken != 0 {
        buffer.facts.note(Fact::BrokenSyllable);
    }
    for_each(buffer, |buffer, start, end| buffer.unsafe_to_break(start..end));
}

// The longest syllable the machine accepts from each start, a position without a category joining the one before it.
// Each (position, state) past a scan's last accept is a dead end later scans stop at, so the work stays linear.
fn segment<const N: usize, T: Copy + Into<u8>>(
    info: &mut [Glyph],
    transitions: &[[u8; N]],
    category_of: impl Fn(&[Glyph], usize) -> Option<u8>,
    accept: impl Fn(u8) -> Option<T>,
) -> u16 {
    let len = info.len();
    let from = |info: &[Glyph], mut i: usize| {
        while i < len {
            if let Some(c) = category_of(info, i) {
                return (i, Some(c));
            }
            i += 1;
        }
        (len, None)
    };
    let step = |state: u8, c: u8| {
        let row = transitions.get(usize::from(state));
        row.and_then(|row| row.get(usize::from(c))).copied().unwrap_or(DEAD)
    };
    let mut dead = DeadEnds::<32>::new();
    let (mut kinds, mut serial) = (0u16, 0u8);
    let (mut at, mut first) = from(info, 0);

    while first.is_some() {
        dead.drop_before(key(at, 0));
        let (mut state, mut i, mut category, mut found) = (0u8, at, first, None);
        let mut accepted = (i, state, category);
        while let Some(c) = category {
            if dead.contains(key(i, state)) {
                break;
            }
            let next = step(state, c);
            if next == DEAD {
                break;
            }
            state = next;
            (i, category) = from(info, i + 1);
            if let Some(kind) = accept(state) {
                found = Some(kind);
                accepted = (i, state, category);
            }
        }

        // The dead ends: the path from the last accept to where the scan stopped, walked again rather than kept.
        let (mut j, mut s, mut c) = accepted;
        dead.insert(key(j, s));
        while j < i {
            let Some(cat) = c else { break };
            s = step(s, cat);
            (j, c) = from(info, j + 1);
            dead.insert(key(j, s));
        }

        let (end, _, next) = accepted;
        match found {
            Some(kind) => {
                let kind = kind.into();
                serial = serial % 15 + 1;
                for glyph in &mut info[at..end] {
                    glyph.syllable = serial << 4 | kind;
                }
                kinds |= 1 << kind;
                (at, first) = (end, next);
            }
            None => (at, first) = from(info, at + 1),
        }
    }
    kinds
}

// Where the syllable at `start` ends, or `MAX_SYLLABLE_LENGTH` glyphs on.
pub(crate) fn next(buffer: &Run, start: usize) -> usize {
    let limit = start + (buffer.len - start).min(MAX_SYLLABLE_LENGTH);
    let syllable = buffer.glyphs[start].syllable;
    let mut end = start + 1;
    while end < limit && buffer.glyphs[end].syllable == syllable {
        end += 1;
    }
    end
}

pub(crate) fn for_each(buffer: &mut Run, mut f: impl FnMut(&mut Run, usize, usize)) {
    let mut start = 0;
    while start < buffer.len {
        let end = next(buffer, start);
        f(buffer, start, end);
        start = end;
    }
}

pub(crate) fn clear(buffer: &mut Run, _: &Plan, _: &Face) -> bool {
    let len = buffer.len;
    for info in &mut buffer.glyphs[..len] {
        info.syllable = 0;
    }
    false
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/script/syllable_storage.rs"]
mod storage;

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/script/syllable_scanning.rs"]
mod scanning;
