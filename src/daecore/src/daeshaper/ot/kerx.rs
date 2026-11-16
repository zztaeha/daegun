use alloc::vec::Vec;
use super::aat::{run_machine, start_glyphs, SubtableSets, NO_ACTION};
use super::apply::{Filter, Present, Reading};
use super::kern;
use crate::daecore::daeshaper::run::{class, AttachKind, Fact, Mask, Run};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daetype::decoder::{midpoint_search, read_i16_be, read_u16_be, read_u32_be, window};
use crate::daecore::daetype::format::index::GlyphBits;
use crate::daecore::daetype::format::aat::{
    tuple_kern, Entry, GlyphRanges, Lookup, Machine, ObsoleteStateTable, StateTable,
};
use crate::daecore::daetype::format::ankr::Ankr;
use crate::daecore::daetype::format::round::ot_round;
use crate::daecore::daetype::format::sanitize::AatTable;
use crate::daecore::daekit::base::sync::Shared;

const KERN_STACK: usize = 8;

mod entry_flags {
    pub(super) const PUSH: u16 = 0x8000;
    pub(super) const MARK: u16 = 0x8000;
    pub(super) const RESET: u16 = 0x2000;
    pub(super) const KERN_OFFSET: u16 = 0x3FFF;
}

// Coverage as `kerx`'s top byte holds it, whose first three bits Apple's `kern` shares; OpenType `kern`'s minimum and
// override flags go in bits `kerx` leaves unused.
mod coverage {
    pub(super) const VERTICAL: u8 = 0x80;
    pub(super) const CROSS_STREAM: u8 = 0x40;
    pub(super) const VARIATION: u8 = 0x20;
    pub(super) const BACKWARDS: u8 = 0x10;
    pub(super) const OVERRIDE: u8 = 0x02;
    pub(super) const MINIMUM: u8 = 0x01;
}

// The tables walked as one: `kerx`, and `kern` in its OpenType and Apple forms.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Kerx,
    KernOt,
    KernAat,
}

#[derive(Clone, Copy)]
pub(super) struct Subtable<'a> {
    // What bounds the subtable's reads: its own bytes, or for OpenType kern's last one the table's, as a 16-bit length
    // can't hold a format 0 subtable of more than 10,920 pairs.
    pub(super) data: &'a [u8],
    at: usize,
    kind: Kind,
    pub(super) format: u8,
    coverage: u8,
    tuple_count: u32,
}

fn subtable_count(table: &[u8], kind: Kind) -> u32 {
    match kind {
        Kind::KernOt => read_u16_be(table, 2).map_or(0, u32::from),
        _ => read_u32_be(table, 4).unwrap_or(0),
    }
}

// The subtables in order; the walk ends at one whose length doesn't fit the table.
pub(super) fn subtables(table: &[u8], kind: Kind) -> impl Iterator<Item = Subtable<'_>> {
    let (count, mut at) = (subtable_count(table, kind), if kind == Kind::KernOt { 4 } else { 8 });
    // kerx ignores a subtable's tuple count before version 4.
    let tuples = kind == Kind::Kerx && read_u16_be(table, 0).is_some_and(|v| v >= 4);
    (0..count).map_while(move |i| {
        let (mut sub, next) = Subtable::parse(table, at, kind, i + 1 == count)?;
        if !tuples {
            sub.tuple_count = 0;
        }
        at = next;
        Some(sub)
    })
}

impl<'a> Subtable<'a> {
    fn parse(table: &'a [u8], at: usize, kind: Kind, last: bool) -> Option<(Self, usize)> {
        let word = |h: &[u8], i: usize| u32::from_be_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
        let (length, format, coverage, tuple_count) = match kind {
            Kind::Kerx => {
                let h = window::<12>(table, at)?;
                (word(h, 0), h[7], h[4], word(h, 8))
            }
            Kind::KernOt => {
                let h = window::<6>(table, at)?;
                let vertical = if h[5] & 0x01 == 0 { coverage::VERTICAL } else { 0 };
                let minimum = if h[5] & 0x02 != 0 { coverage::MINIMUM } else { 0 };
                let cross_stream = if h[5] & 0x04 != 0 { coverage::CROSS_STREAM } else { 0 };
                let overrides = if h[5] & 0x08 != 0 { coverage::OVERRIDE } else { 0 };
                let flags = vertical | minimum | cross_stream | overrides;
                (u32::from(u16::from_be_bytes([h[2], h[3]])), h[4], flags, 0)
            }
            Kind::KernAat => {
                let h = window::<8>(table, at)?;
                let flags = coverage::VERTICAL | coverage::CROSS_STREAM | coverage::VARIATION;
                (word(h, 0), h[5], h[4] & flags, 0)
            }
        };
        let length = length as usize;
        let end = at.checked_add(length).filter(|&end| length >= Self::header_len(kind) && end <= table.len())?;
        let (data, at) = if last && kind == Kind::KernOt { (table, at) } else { (&table[at..end], 0) };
        Some((Subtable { data, at, kind, format, coverage, tuple_count }, end))
    }

    fn header_len(kind: Kind) -> usize {
        match kind {
            Kind::Kerx => 12,
            Kind::KernOt => 6,
            Kind::KernAat => 8,
        }
    }

    pub(super) fn body(&self) -> usize {
        self.at + Self::header_len(self.kind)
    }

    fn horizontal(&self) -> bool {
        self.coverage & coverage::VERTICAL == 0
    }

    pub(super) fn cross_stream(&self) -> bool {
        self.coverage & coverage::CROSS_STREAM != 0
    }

    fn backwards(&self) -> bool {
        self.coverage & coverage::BACKWARDS != 0
    }

    // OpenType `kern`'s override: its value for a pair replaces what the subtables before it added (kern, coverage).
    fn overrides(&self) -> bool {
        self.kind == Kind::KernOt && self.coverage & coverage::OVERRIDE != 0
    }

    // Apple `kern`'s variation subtable holds one tuple's deltas, and OpenType `kern`'s minimum one sets a floor on
    // what kerning and tracking apply, neither of which is kerning to add; kerx's variation values kern.
    fn skipped(&self) -> bool {
        let flag = match self.kind {
            Kind::Kerx => 0,
            Kind::KernAat => coverage::VARIATION,
            Kind::KernOt => coverage::MINIMUM,
        };
        self.coverage & flag != 0
    }

    // What is collected of the subtable once a face: a pair format's left and right glyphs, or a machine's initial
    // glyphs, and a kerx machine's class index.
    fn sets(&self, face: &Face, num_glyphs: u32) -> SubtableSets {
        let lookup = |at: Option<u32>| at.and_then(|at| self.lookup(at as usize, num_glyphs)).map(|l| l.glyph_set());
        let starts = |first: Option<GlyphRanges>| SubtableSets { first: first.unwrap_or_default(), ..<_>::default() };
        let field = |i: usize| read_u32_be(self.data, self.body() + i);
        match (self.kind, self.format) {
            (_, 0) => {
                let (count, pairs) = self.pairs();
                let (mut lefts, mut rights) = (GlyphBits::new(), GlyphBits::new());
                for key in (0..count).filter_map(|i| read_u32_be(self.data, pairs + i * 6)) {
                    lefts.insert(key >> 16);
                    rights.insert(key & 0xFFFF);
                }
                let (first, second) = (GlyphRanges::of_bits(&lefts), GlyphRanges::of_bits(&rights));
                SubtableSets { first, second, classes: None }
            }
            (Kind::Kerx, 1 | 4) => {
                let table = self.extended(num_glyphs);
                table.map(|t| SubtableSets::machine(face, &t, self.actionable())).unwrap_or_default()
            }
            (_, 1) => starts(self.obsolete().map(|t| start_glyphs(&t, self.actionable()))),
            (_, 2) => {
                let Some((left, right, _)) = self.class_offsets() else { return SubtableSets::default() };
                let (first, second) = (self.class_glyphs(left, num_glyphs), self.class_glyphs(right, num_glyphs));
                SubtableSets { first, second, classes: None }
            }
            (Kind::KernOt | Kind::KernAat, 3) => kern::format3_sets(self, num_glyphs),
            (Kind::Kerx, 6) => SubtableSets {
                first: lookup(field(8)).unwrap_or_default(),
                second: lookup(field(12)).unwrap_or_default(),
                classes: None,
            },
            _ => SubtableSets::default(),
        }
    }

    // Format 0's pairs: how many fit, and where they start.
    fn pairs(&self) -> (usize, usize) {
        let body = self.body();
        let (count, at) = match self.kind {
            Kind::Kerx => (read_u32_be(self.data, body).map_or(0, |n| n as usize), body + 16),
            _ => (read_u16_be(self.data, body).map_or(0, usize::from), body + 8),
        };
        (count.min(self.data.len().saturating_sub(at) / 6), at)
    }

    // The subtable's value for a pair, None where it holds none. Formats 0, 2, 3 and 6 kern only from a glyph of the
    // first set to one of the second, so an id past the font, which format 3 would read as class 0, takes no kern.
    fn kerning(&self, left: u32, right: u32, num_glyphs: u32, sets: &SubtableSets) -> Option<i32> {
        let paired = || sets.first.contains(left) && sets.second.contains(right);
        match self.format {
            0 if paired() => self.format0(left, right),
            2 if paired() => Some(self.format2(left, right, num_glyphs)),
            3 if paired() => Some(kern::format3(self, left, right)),
            6 if paired() => Some(self.format6(left, right, num_glyphs)),
            _ => None,
        }
    }

    // A pair the font lists twice kerns by the copy the search lands on. With tuples, a pair of the sets' glyphs that
    // the search doesn't find reads its tuple from the subtable's start; without, it has no value.
    fn format0(&self, left: u32, right: u32) -> Option<i32> {
        let (count, pairs) = self.pairs();
        let want = u64::from(left) << 16 | u64::from(right);
        let found = midpoint_search(count, |i| read_u32_be(self.data, pairs + i * 6).map(|pair| want.cmp(&pair.into())))
            .and_then(|i| read_i16_be(self.data, pairs + i * 6 + 4));
        if found.is_none() && self.tuple_count == 0 {
            return None;
        }
        Some(tuple_kern(self.data, self.at, found.map_or(0, i32::from), self.tuple_count))
    }

    // Format 2's left and right class tables and its array, as offsets from the subtable.
    fn class_offsets(&self) -> Option<(usize, usize, usize)> {
        let (d, body) = (self.data, self.body());
        if self.kind == Kind::Kerx {
            let word = |i: usize| read_u32_be(d, body + i).map(|o| o as usize);
            Some((word(4)?, word(8)?, word(12)?))
        } else {
            let word = |i: usize| read_u16_be(d, body + i).map(usize::from);
            Some((word(2)?, word(4)?, word(6)?))
        }
    }

    // A format 2 class table's glyphs: those a `kerx` lookup lists, or those a `kern` class array holds in any class
    // but 1, out of bounds.
    fn class_glyphs(&self, at: usize, num_glyphs: u32) -> GlyphRanges {
        if self.kind == Kind::Kerx {
            return self.lookup(at, num_glyphs).map(|l| l.glyph_set()).unwrap_or_default();
        }
        let at = self.at + at;
        let (Some(first), Some(count)) = (read_u16_be(self.data, at), read_u16_be(self.data, at + 2)) else {
            return GlyphRanges::default();
        };
        let held = (0..count).filter(|&i| read_u16_be(self.data, at + 4 + 2 * usize::from(i)).is_some_and(|k| k != 1));
        GlyphRanges::new(held.map(|i| u32::from(first) + u32::from(i)).map(|g| (g, g)).collect())
    }

    // A format 2 class: a `kerx` lookup's value by search, none for format 10, or a `kern` class array's, and 0
    // where there is none.
    fn class(&self, at: usize, glyph: u32, num_glyphs: u32) -> u16 {
        if self.kind == Kind::Kerx {
            return self.lookup(at, num_glyphs).and_then(|l| l.typed_value(glyph)).unwrap_or(0);
        }
        let at = self.at + at;
        let i = read_u16_be(self.data, at).and_then(|first| glyph.checked_sub(first.into()));
        let i = i.filter(|&i| read_u16_be(self.data, at + 2).is_some_and(|count| i < count.into()));
        i.and_then(|i| read_u16_be(self.data, at + 4 + i as usize * 2)).unwrap_or(0)
    }

    // Both classes are byte offsets, pre-multiplied by a row's and a value's bytes, and sum to the value's place:
    // `kerx`'s from its array, `kern`'s from the subtable, which name nothing short of the array. An odd sum reads the
    // value it falls in.
    fn format2(&self, left: u32, right: u32, num_glyphs: u32) -> i32 {
        let Some((left_at, right_at, array)) = self.class_offsets() else { return 0 };
        let (l, r) = (self.class(left_at, left, num_glyphs), self.class(right_at, right, num_glyphs));
        let sum = usize::from(l) + usize::from(r);
        let offset = if self.kind == Kind::Kerx { Some(sum) } else { sum.checked_sub(array) };
        let value = offset.and_then(|o| read_i16_be(self.data, self.at.checked_add(array)?.checked_add(o / 2 * 2)?));
        value.map_or(0, |v| tuple_kern(self.data, self.at, v.into(), self.tuple_count))
    }

    fn lookup(&self, at: usize, num_glyphs: u32) -> Option<Lookup<'a>> {
        Lookup::parse(self.data.get(self.at.checked_add(at)?..)?, num_glyphs).ok()
    }

    // Rows and columns by search, 0 where it finds none. Long values are 32 bits, and an index that overflows reads 0.
    fn format6(&self, left: u32, right: u32, num_glyphs: u32) -> i32 {
        let field = |i: usize| read_u32_be(self.data, self.body() + i).map(|v| v as usize);
        let (Some(flags), Some(rows), Some(columns), Some(array), Some(vector)) =
            (field(0), field(8), field(12), field(16), field(20))
        else {
            return 0;
        };
        let (Some(rows), Some(columns)) = (self.lookup(rows, num_glyphs), self.lookup(columns, num_glyphs)) else {
            return 0;
        };
        let Some(array) = self.at.checked_add(array) else { return 0 };
        let value = if flags & 1 != 0 {
            let (l, r) = (rows.value_u32(left).unwrap_or(0), columns.value_u32(right).unwrap_or(0));
            let at = l.checked_add(r).and_then(|i| i.checked_mul(4)).and_then(|i| array.checked_add(i as usize));
            at.and_then(|at| read_u32_be(self.data, at)).map(|v| v as i32)
        } else {
            let i = usize::from(rows.value(left).unwrap_or(0)) + usize::from(columns.value(right).unwrap_or(0));
            array.checked_add(2 * i).and_then(|at| read_i16_be(self.data, at)).map(i32::from)
        };
        let base = self.at.checked_add(vector);
        value.zip(base).map_or(0, |(v, base)| tuple_kern(self.data, base, v, self.tuple_count))
    }

    // A `kerx` machine's state table, after the header.
    fn extended(&self, num_glyphs: u32) -> Option<StateTable<'a>> {
        StateTable::parse(self.data.get(self.body()..)?, 1, num_glyphs).ok()
    }

    // A `kern` machine's, in the 16-bit form.
    fn obsolete(&self) -> Option<ObsoleteStateTable<'a>> {
        ObsoleteStateTable::parse(self.data, self.body())
    }

    fn actionable(&self) -> fn(&Entry) -> bool {
        if self.kind == Kind::Kerx { acts } else { kern_acts }
    }
}

// Both `kerx` machines act on an entry naming an action: kerning values, or the anchor points to attach by.
pub(crate) fn acts(e: &Entry) -> bool {
    e.word1 != NO_ACTION
}

// A `kern` entry names its kerning values by a byte offset in its flags.
fn kern_acts(e: &Entry) -> bool {
    e.flags & entry_flags::KERN_OFFSET != 0
}

fn table<'a>(face: &Face<'a>) -> Option<&'a [u8]> {
    face.aat(AatTable::Kerx)
}

// Each subtable's sets by place, for those a run in `horizontal` text takes, as the font cache keeps them: the plan
// holds them, so a shape reads them without asking the face.
pub(super) fn plan_sets(face: &Face, table: &[u8], kind: Kind, horizontal: bool) -> Vec<Option<Shared<SubtableSets>>> {
    let which = if kind == Kind::Kerx { AatTable::Kerx } else { AatTable::Kern };
    // A slot for each subtable that fits, not each the header counts, which may run to 2^32.
    let (count, num_glyphs) = (subtables(table, kind).count(), face.glyph_count());
    let runs = |sub: &Subtable| !sub.skipped() && horizontal == sub.horizontal();
    let sets = |(index, sub): (usize, Subtable)| {
        runs(&sub).then(|| face.subtable_sets(which, count, index, || sub.sets(face, num_glyphs)))
    };
    subtables(table, kind).enumerate().map(sets).collect()
}

// kerx's subtables' sets for a plan whose kerx runs.
pub(crate) fn kerx_sets(face: &Face, horizontal: bool) -> Vec<Option<Shared<SubtableSets>>> {
    table(face).map_or_else(Vec::new, |table| plan_sets(face, table, Kind::Kerx, horizontal))
}

// Whether the font has a kerx the sanitizer passes, whichever of its subtables run.
pub(crate) fn has_data(face: &Face) -> bool {
    table(face).is_some()
}

pub(crate) fn apply(face: &Face, buffer: &mut Run, kern_mask: Mask, planned: &[Option<Shared<SubtableSets>>]) {
    if let Some(table) = table(face) {
        walk(face, buffer, table, Kind::Kerx, kern_mask, planned);
    }
}

// Runs kerx and both forms of `kern`: each subtable in the buffer's direction whose first glyphs the buffer holds, the
// first cross-stream one chaining every glyph. Its sets are the plan's, or the face's past the plan's direction.
pub(super) fn walk(
    face: &Face,
    buffer: &mut Run,
    table: &[u8],
    kind: Kind,
    kern_mask: Mask,
    planned: &[Option<Shared<SubtableSets>>],
) {
    let requested = kern_mask != 0;
    buffer.unsafe_to_concat_all();
    let num_glyphs = face.glyph_count();
    // Every anchor of an ankr the sanitizer rejects, or of one without a lookup, reads as (0, 0).
    let ankr = face.aat(AatTable::Ankr).filter(|d| kind == Kind::Kerx && read_u32_be(d, 4) != Some(0));
    let ankr = ankr.and_then(|d| Ankr::parse(d, num_glyphs).ok());
    let mut seen_cross_stream = false;
    let mut reversed = false;
    // Where an override subtable follows others, each pair's sum so far, by its first glyph.
    let mut sums = if subtables(table, kind).any(|s| s.overrides()) { alloc::vec![0; buffer.len] } else { Vec::new() };

    let which = if kind == Kind::Kerx { AatTable::Kerx } else { AatTable::Kern };
    let count = subtables(table, kind).count();
    for (index, sub) in subtables(table, kind).enumerate() {
        let horizontal = buffer.direction.is_horizontal();
        if sub.skipped() || horizontal != sub.horizontal() {
            continue;
        }
        let fetched;
        let sets = match planned.get(index) {
            Some(Some(sets)) => &**sets,
            _ => {
                fetched = face.subtable_sets(which, count, index, || sub.sets(face, num_glyphs));
                &*fetched
            }
        };
        if !buffer.glyphs[..buffer.len].iter().any(|info| sets.first.contains(info.id)) {
            continue;
        }
        let cross_stream = sub.cross_stream();
        if !seen_cross_stream && cross_stream {
            seen_cross_stream = true;
            chain_for_cross_stream(buffer);
        }
        let reverse = sub.backwards() != buffer.direction.is_backward();
        if reverse != reversed {
            buffer.reverse();
            reversed = reverse;
        }

        let pairs = requested && !sub.backwards();
        match (kind, sub.format) {
            (_, 0 | 2) | (Kind::KernOt | Kind::KernAat, 3) | (Kind::Kerx, 6) if pairs => {
                kern_pairs(face, buffer, kern_mask, &sub, num_glyphs, sets, &mut sums);
            }
            (Kind::Kerx, 1) if requested || cross_stream => {
                if let Some(table) = sub.extended(num_glyphs) {
                    machine_kern(&sub, &table.with_classes(sets.classes.as_ref()), buffer, kern_mask);
                }
            }
            (_, 1) if requested || cross_stream => {
                if let Some(table) = sub.obsolete() {
                    machine_kern(&sub, &table, buffer, kern_mask);
                }
            }
            (Kind::Kerx, 4) => {
                if let Some(table) = sub.extended(num_glyphs) {
                    let table = table.with_classes(sets.classes.as_ref());
                    machine_attach(face, &sub, &table, buffer, ankr.as_ref(), reversed);
                }
            }
            _ => {}
        }
    }

    if reversed {
        buffer.reverse();
    }
    if seen_cross_stream {
        flag_carried_shifts(buffer);
    }
}

// A cross-stream shift carries down its chain to the glyphs after it, so a break it crosses isn't safe: shaped
// apart, the side after starts unshifted.
fn flag_carried_shifts(buffer: &mut Run) {
    let (back, horizontal, n) = (buffer.direction.is_backward(), buffer.direction.is_horizontal(), buffer.len);
    let mut carried = 0i32;
    // Parents first: each glyph hangs from the one before it in the run's order.
    for k in 0..n {
        let i = if back { n - 1 - k } else { k };
        let pos = buffer.places[i];
        let parent = i.wrapping_add_signed(isize::from(pos.attach.to));
        let linked = pos.attach.kind == AttachKind::Cursive && k > 0 && parent == if back { i + 1 } else { i - 1 };
        if linked && carried != 0 {
            buffer.unsafe_to_break(i.min(parent)..i.max(parent) + 1);
        }
        let own = if horizontal { pos.offset_y } else { pos.offset_x };
        carried = if linked { carried.saturating_add(own) } else { own };
    }
}

// Format 1, shared by `kern` and `kerx`: values go to the pushed glyphs, last first. The list (ending at 0xFFFF, or at
// kern's odd value) is read whole before any applies, so one running past the subtable kerns nothing.
fn machine_kern(sub: &Subtable, table: &impl Machine, buffer: &mut Run, kern_mask: Mask) {
    let machine = sub.body();
    let kerx = sub.kind == Kind::Kerx;
    let offset = if kerx {
        read_u32_be(sub.data, machine + 16).map(|o| o as usize)
    } else {
        read_u16_be(sub.data, machine + 8).map(usize::from)
    };
    let Some(offset) = offset else { return };
    let Some(array) = machine.checked_add(offset) else { return };
    let actionable = sub.actionable();
    let horizontal = buffer.direction.is_horizontal();
    let cross_stream = sub.cross_stream();
    let step = 2 * u64::from(sub.tuple_count.max(1));
    let mut stack = [0usize; KERN_STACK];
    let mut depth = 0;

    run_machine(table, buffer, false, actionable, None, |buffer, entry| {
        if kerx && entry.flags & entry_flags::RESET != 0 {
            depth = 0;
        }
        if entry.flags & entry_flags::PUSH != 0 {
            if depth < KERN_STACK {
                stack[depth] = buffer.cursor;
                depth += 1;
            } else {
                // A push past a full stack empties it, where a piece holding fewer of its pushes never fills it.
                buffer.unsafe_to_break(stack[0]..(buffer.cursor + 1).min(buffer.len));
                depth = 0;
            }
        }
        if !actionable(entry) || depth == 0 {
            return;
        }

        // `kerx` names a value by its index in the array; `kern` by its byte offset from the state table, so one
        // short of the array names nothing.
        let index = if kerx {
            Some(usize::from(entry.word1))
        } else {
            usize::from(entry.flags & entry_flags::KERN_OFFSET).checked_sub(offset).map(|o| o / 2)
        };
        let Some(start) = index.and_then(|i| array.checked_add(i * 2)) else {
            depth = 0;
            return;
        };
        let (mut values, mut n) = ([0i32; KERN_STACK], 0);
        while n < depth {
            // In 64 bits, as a font's tupleCount can put a value past a 32-bit usize, and so past the subtable.
            let at = usize::try_from(n as u64 * step).ok().and_then(|offset| start.checked_add(offset));
            let Some(v) = at.and_then(|at| read_i16_be(sub.data, at)).map(i32::from) else {
                depth = 0;
                return;
            };
            if kerx && v == -1 {
                break;
            }
            values[n] = if kerx { v } else { v & !1 };
            n += 1;
            if !kerx && v & 1 != 0 {
                break;
            }
        }
        let mut earliest = buffer.cursor;
        for &v in &values[..n] {
            depth -= 1;
            let target = stack[depth];
            if target < buffer.len {
                kern_value(buffer, target, v, kern_mask, horizontal, cross_stream);
                earliest = earliest.min(target);
            }
        }
        // A value for a glyph pushed before a break is lost to the piece before it, which this action isn't in.
        buffer.unsafe_to_break(earliest..(buffer.cursor + 1).min(buffer.len));
    });
}

fn kern_value(buffer: &mut Run, at: usize, v: i32, kern_mask: Mask, horizontal: bool, cross_stream: bool) {
    let glyph_mask = buffer.glyphs[at].mask;
    let pos = &mut buffer.places[at];

    if cross_stream {
        if v == -0x8000 {
            pos.attach.kind = AttachKind::None;
            pos.attach.to = 0;
            *pos.across_mut(horizontal) = 0;
        } else if pos.attach.kind != AttachKind::None {
            let across = pos.across_mut(horizontal);
            *across = across.saturating_add(v);
            buffer.facts.note(Fact::Attachments);
        }
    } else if glyph_mask & kern_mask != 0 {
        let (advance, offset) = pos.along_mut(horizontal);
        *advance = advance.saturating_add(v);
        *offset = offset.saturating_add(v);
    }
}

// Format 4: a glyph attaches to the last one marked by its action's points, anchors (one missing at (0, 0)) or
// coordinates; Mark marks whatever the action. An unreadable action, missing point or mark past 16 bits does nothing.
fn machine_attach(
    face: &Face, sub: &Subtable, table: &StateTable, buffer: &mut Run, ankr: Option<&Ankr>, reversed: bool,
) {
    let Some(body) = sub.data.get(sub.body()..) else { return };
    let Some(flags) = read_u32_be(body, 16) else { return };

    let action_kind = (flags & 0xC000_0000) >> 30;
    let actions_at = (flags & 0x00FF_FFFF) as usize;

    let mut marked: Option<usize> = None;

    run_machine(table, buffer, false, acts, None, |buffer, entry| {
        let idx = buffer.cursor;
        let mark = marked.filter(|_| entry.word1 != NO_ACTION && idx < buffer.len);
        let chain = mark.and_then(|mark| i16::try_from(mark as isize - idx as isize).ok());
        let chain = chain.and_then(|c| if reversed { c.checked_neg() } else { Some(c) });
        if let (Some(mark), Some(chain)) = (mark, chain) {
            let action = usize::from(entry.word1);
            let offsets = match action_kind {
                0 => window::<4>(body, actions_at + action * 4).and_then(|points| {
                    let point = |glyph: u32, i: usize| {
                        face.outline_point(glyph, u16::from_be_bytes([points[i], points[i + 1]]))
                    };
                    let (m, c) = (point(buffer.glyphs[mark].id, 0)?, point(buffer.glyphs[idx].id, 2)?);
                    Some((ot_round(m.0 - c.0), ot_round(m.1 - c.1)))
                }),
                1 => window::<4>(body, actions_at + action * 4).map(|points| {
                    let anchor = |glyph: u32, i: usize| {
                        let index = u16::from_be_bytes([points[i], points[i + 1]]);
                        let p = ankr.and_then(|a| a.anchor_point(glyph, index)).unwrap_or_default();
                        (i32::from(p.x), i32::from(p.y))
                    };
                    let (m, c) = (anchor(buffer.glyphs[mark].id, 0), anchor(buffer.glyphs[idx].id, 2));
                    (m.0 - c.0, m.1 - c.1)
                }),
                2 => window::<8>(body, actions_at + action * 8).map(|points| {
                    let coordinate = |i: usize| i32::from(i16::from_be_bytes([points[i], points[i + 1]]));
                    (coordinate(0) - coordinate(4), coordinate(2) - coordinate(6))
                }),
                _ => None,
            };
            if let Some((dx, dy)) = offsets {
                // A piece starting between the two has no mark, and leaves the glyph unattached.
                buffer.unsafe_to_break(mark.min(idx)..mark.max(idx) + 1);
                buffer.places[idx].offset_x = dx;
                buffer.places[idx].offset_y = dy;
                buffer.places[idx].attach.kind = AttachKind::Mark;
                buffer.places[idx].attach.to = chain;
                buffer.facts.note(Fact::Attachments);
            }
        }

        if entry.flags & entry_flags::MARK != 0 {
            marked = Some(idx);
        }
    });
}

// Each glyph and the next one that isn't a mark, kerned by the subtable.
fn kern_pairs(
    face: &Face,
    buffer: &mut Run,
    kern_mask: Mask,
    sub: &Subtable,
    num_glyphs: u32,
    sets: &SubtableSets,
    sums: &mut [i32],
) {
    buffer.unsafe_to_concat_all();
    let horizontal = buffer.direction.is_horizontal();
    let cross_stream = sub.cross_stream();

    // The next glyph as kerning reads it: past marks and every format character, within the kern feature's mask.
    let reading = Reading::new(Filter::of_classes(class::MARK), kern_mask, 0, 0);

    let mut idx = 0;
    while idx < buffer.len {
        if buffer.glyphs[idx].mask & kern_mask == 0 {
            idx += 1;
            continue;
        }

        let j = match reading.after(&buffer.glyphs[..buffer.len], idx, face, &Present) {
            Ok(j) => j,
            Err(end) => {
                idx = end;
                continue;
            }
        };

        let value = sub.kerning(buffer.glyphs[idx].id, buffer.glyphs[j].id, num_glyphs, sets);
        let kern = match (value, sums.get_mut(idx)) {
            (Some(v), Some(sum)) if !cross_stream => {
                let added = if sub.overrides() { v.saturating_sub(*sum) } else { v };
                *sum = sum.saturating_add(added);
                added
            }
            (v, _) => v.unwrap_or(0),
        };
        if kern != 0 {
            apply_pair(buffer, idx, j, kern, horizontal, cross_stream);
            buffer.unsafe_to_break(idx..j + 1);
        }

        idx = j;
    }
}

fn apply_pair(buffer: &mut Run, i: usize, j: usize, kern: i32, horizontal: bool, cross_stream: bool) {
    if cross_stream {
        *buffer.places[j].across_mut(horizontal) = kern;
        buffer.facts.note(Fact::Attachments);
        return;
    }

    // The whole value goes on the first glyph's advance, as GPOS pair positioning's value1 does.
    let (advance, _) = buffer.places[i].along_mut(horizontal);
    *advance = advance.saturating_add(kern);
}

fn chain_for_cross_stream(buffer: &mut Run) {
    let back = buffer.direction.is_backward();
    for pos in &mut buffer.places[..buffer.len] {
        pos.attach.kind = AttachKind::Cursive;
        pos.attach.to = if back { 1 } else { -1 };
    }
}
