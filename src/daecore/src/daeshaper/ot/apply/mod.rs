// Applying a layout table's lookups to a run (OpenType chapter 2, GSUB and GPOS): each lookup over the whole run in
// turn; at each glyph its feature's mask and its flags let in, its subtables in order until one applies.
mod context;
mod reading;

pub(crate) use context::{
    chain_lists, chained, first_read, record_lookups, sequence, Coverages, FirstRead, Rule, ValueList,
};
pub(crate) use reading::{Filter, Input, Peek, Present, Reading, Step};
// The subsetter's closure walks nested lookups as deep as shaping does.
pub(crate) use self::NESTING_LIMIT as MAX_NESTING_LEVEL;

use core::ops::{ControlFlow, Range};

use super::digest::Digest;
use super::{
    gpos, gsub, medium_kind, resolve_extension, reverse_of, table_at_width, ClassDef, Coverage, LayoutTable, Lookup,
    LookupIndex, LookupKinds, SetIndex, SubtableIndex,
};
use crate::daecore::daekit::base::sync::Shared;
use crate::daecore::daeshaper::face::{Deltas, Face};
use crate::daecore::daeshaper::plan::{Plan, StageLookup, Table};
use crate::daecore::daeshaper::run::{class, flag, text, Glyph, Mask, Run};
use crate::daecore::daeshaper::unicode::GeneralCategory;
use crate::daecore::daetype::format::index::SparseIndex;

// Lookups nest at most 64 deep and an input holds at most 64 glyphs: a font past either is cut off there, so nesting
// ends and an input's places fit an array on the stack.
pub(crate) const NESTING_LIMIT: usize = 64;
pub(crate) const SEQUENCE_LIMIT: usize = 64;

// Which part of a rule a reading serves: its input, or the backtrack and lookahead about it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Side {
    Input,
    Context,
}

// The base the last mark-to-base or mark-to-ligature search found, and where that search started: the next search
// reads back only that far, so a base's marks cost a read each rather than a walk each.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct BaseMemo {
    pub(crate) found: Option<usize>,
    pub(crate) from: usize,
}

// What an edit made of the current glyph, with the class a font without GDEF classes gives it (0: its own stays).
#[derive(Clone, Copy)]
enum Made {
    Substitute,
    Ligature(u8),
    Piece(u8),
}

// The state lookups share as they run over one table.
pub(crate) struct Engine<'a> {
    pub(crate) run: &'a mut Run,
    pub(crate) face: &'a Face<'a>,
    // GPOS's device deltas at the face's location; none at the default.
    pub(crate) deltas: Option<&'a Deltas>,
    table: Option<LayoutTable<'a>>,
    positions: bool,
    // The run's unsafe-to-concatenate setting, kept here: a failed match asks it, and the run's settings sit on a line
    // nothing else a match reads shares, a cache miss each time (2,350 cycles of a 26,153-cycle Arabic shape).
    reports_concat: bool,
    // What the plan built of each of the table's lookups, by index.
    indexes: &'a [Option<Shared<LookupIndex>>],
    // Every glyph the run has held since the table began, so a lookup none of whose glyphs it holds is passed unread.
    seen: Digest,
    // The lookup the stage runs, as its feature has it.
    mask: Mask,
    skips_zwnj: bool,
    skips_zwj: bool,
    pub(crate) random: bool,
    per_syllable: bool,
    // The lookup now applying, nested or not: its flags, the glyphs it can start at, its subtables' indexes and what
    // the plan built for the subtable being tried, which every reader of it asks.
    pub(crate) filter: Filter,
    gate: Option<&'a Digest>,
    subtables: Option<&'a [SubtableIndex]>,
    subtable: Option<&'a SubtableIndex>,
    // What follows the current glyph as a rule's input and its lookahead read it, once a lookup at a glyph has asked:
    // its subtables all read it alike, and none that fails changes it.
    peeks: [Option<Peek>; 2],
    levels_left: usize,
    seed: u32,
    pub(crate) base: BaseMemo,
    // The last input matched, written by each match and read up to its count.
    matched: [u32; SEQUENCE_LIMIT],
}

impl<'a> Engine<'a> {
    fn new(face: &'a Face<'a>, run: &'a mut Run, table: Option<LayoutTable<'a>>, positions: bool) -> Self {
        Engine {
            reports_concat: run.settings.reports(flag::UNSAFE_TO_CONCAT),
            run,
            face,
            deltas: face.deltas(),
            table,
            positions,
            indexes: &[],
            seen: Digest::default(),
            mask: 1,
            skips_zwnj: true,
            skips_zwj: true,
            random: false,
            per_syllable: false,
            filter: Filter::default(),
            gate: None,
            subtables: None,
            subtable: None,
            peeks: [None; 2],
            levels_left: NESTING_LIMIT,
            seed: 1,
            base: BaseMemo::default(),
            matched: [0; SEQUENCE_LIMIT],
        }
    }

    // A new lookup from the stage: its mask, and no base remembered from another.
    fn begin(&mut self, mask: Mask) {
        self.mask = mask;
        self.base = BaseMemo::default();
    }

    pub(crate) fn mask(&self) -> Mask {
        self.mask
    }

    pub(crate) fn unsafe_to_concat(&mut self, span: Range<usize>) {
        if self.reports_concat {
            self.run.unsafe_to_concat(span);
        }
    }

    pub(crate) fn unsafe_to_concat_across(&mut self, start: usize, end: usize) {
        if self.reports_concat {
            self.run.unsafe_to_concat_across(start, end);
        }
    }

    // A lookup a context rule applies, not one the stage does.
    pub(crate) fn is_nested(&self) -> bool {
        self.levels_left != NESTING_LIMIT
    }

    // Lookup `index`, its flags taken; none where the table lacks it.
    fn enter(&mut self, index: u16) -> Option<Lookup<'a>> {
        let lookup = self.table.as_ref()?.lookup(index)?;
        self.filter = Filter::of_lookup(lookup.flags, lookup.mark_filtering_set(), self.face);
        Some(lookup)
    }

    // Substitution sees ZWNJ and hidden glyphs (CGJ, tags, Mongolian selectors), there to part what would join;
    // positioning, coming after, reads past them. ZWJ, and ZWNJ about an input, only where the feature reads them.
    fn keeps(&self, side: Side) -> u8 {
        let zwj = if self.skips_zwj { 0 } else { text::ZWJ };
        let zwnj = if self.skips_zwnj { 0 } else { text::ZWNJ };
        match (self.positions, side) {
            (false, Side::Input) => text::ZWNJ | text::HIDDEN | zwj,
            (false, Side::Context) => text::HIDDEN | zwnj,
            (true, Side::Input) => zwj,
            (true, Side::Context) => 0,
        }
    }

    // A match reads within the syllable of the glyph it starts at, where its feature asks; about its input it reads
    // whatever the masks.
    pub(crate) fn reading(&self, side: Side) -> Reading {
        let syllable = if self.per_syllable { self.run.current().syllable } else { 0 };
        let mask = if side == Side::Input { self.mask } else { Mask::MAX };
        Reading::new(self.filter, mask, syllable, self.keeps(side))
    }

    // Each read below costs the run the glyphs it reads, as an edit does: hidden glyphs lengthen every read, and a
    // rule reaches 65,535 glyphs back and ahead. A read the run can't pay for fails where it ended.
    fn paid<T>(&mut self, n: usize, read: Result<T, usize>, end: impl FnOnce(&T) -> usize) -> Result<T, usize> {
        let paid = self.run.spend(n);
        read.and_then(|found| if paid { Ok(found) } else { Err(end(&found)) })
    }

    // A read of the one glyph next to the cursor, the first glyph it reads paid for by the subtable reading it.
    fn paid_past<T>(&mut self, n: usize, read: Result<T, usize>, end: impl FnOnce(&T) -> usize) -> Result<T, usize> {
        if n <= 1 { read } else { self.paid(n - 1, read, end) }
    }

    pub(crate) fn match_input(&mut self, count: u16, wants: impl Fn(u32, usize) -> bool) -> Result<Input, usize> {
        // Once the run has failed nothing more is read, so a set's tries past it come back at once.
        if self.run.failed {
            return Err(self.run.cursor);
        }
        let reading = self.reading(Side::Input);
        let (read, n) = reading::input(self.run, self.face, &reading, usize::from(count), wants, &mut self.matched);
        self.paid(n, read, |input| input.end)
    }

    pub(crate) fn match_backtrack(&mut self, count: u16, wants: impl Fn(u32, usize) -> bool) -> Result<usize, usize> {
        let read = reading::backtrack(self.run, self.face, &self.reading(Side::Context), usize::from(count), wants);
        let (Ok(from) | Err(from)) = read;
        self.paid(self.run.behind().saturating_sub(from), read, |&from| from)
    }

    pub(crate) fn match_lookahead(&mut self, end: usize, count: u16, wants: impl Fn(u32, usize) -> bool)
        -> Result<usize, usize> {
        let reading = self.reading(Side::Context);
        let read = reading::lookahead(self.run, self.face, &reading, end, usize::from(count), wants);
        let (Ok(to) | Err(to)) = read;
        self.paid(to.saturating_sub(end), read, |&to| to)
    }

    // What a value is compared with next past glyph `from`, the glyphs read past the first paid for.
    pub(crate) fn peek(&mut self, from: usize, side: Side) -> Peek {
        let peek = self.peek_unpaid(from, side);
        let last = match peek {
            Peek::Glyph(_, end) | Peek::Stop(end) => end - 1,
            Peek::Optional(_, at) => at,
        };
        if last > from + 1 {
            self.run.spend(last - from - 1);
        }
        peek
    }

    fn peek_unpaid(&self, from: usize, side: Side) -> Peek {
        self.reading(side).peek(&self.run.glyphs[..self.run.len], from, self.face)
    }

    // `peek` past the current glyph, read once a lookup at a glyph.
    pub(crate) fn peek_once(&mut self, side: Side) -> Peek {
        let peek = match self.peeks[side as usize] {
            Some(peek) => peek,
            None => self.peek(self.run.cursor, side),
        };
        debug_assert_eq!(peek, self.peek_unpaid(self.run.cursor, side), "a read past the glyph went stale");
        self.peeks[side as usize] = Some(peek);
        peek
    }

    // The next glyph past the cursor that has to be there, as a pair reads it.
    pub(crate) fn next_present(&mut self) -> Result<usize, usize> {
        let at = self.run.cursor;
        let read = self.reading(Side::Input).after(&self.run.glyphs[..self.run.len], at, self.face, &Present);
        let (Ok(to) | Err(to)) = read;
        self.paid_past(to.saturating_sub(at), read, |&to| to + 1)
    }

    // The nearest glyph before the cursor that has to be there past what `filter` hides, as cursive and mark-to-mark
    // attachment read it.
    pub(crate) fn prev_present(&mut self, filter: Filter) -> Result<usize, usize> {
        let at = self.run.cursor;
        let reading = self.reading(Side::Input).with_filter(filter);
        let read = reading.before(&self.run.glyphs[..at], at, self.face, &Present);
        let (Ok(to) | Err(to)) = read;
        self.paid_past(at.saturating_sub(to), read, |&to| to)
    }

    // The places of the last input matched, copied: a lookup applied at one matches inputs of its own.
    pub(crate) fn matched(&self) -> [u32; SEQUENCE_LIMIT] {
        self.matched
    }

    // What the plan built for the subtable being tried.
    pub(crate) fn subtable_index(&self) -> Option<&'a SubtableIndex> {
        self.subtable
    }

    // The indexes of the tables the subtable reads past its first coverage, as `ot::rule_tables` orders them.
    pub(crate) fn rule_indexes(&self) -> &'a [Option<Shared<SparseIndex>>] {
        self.subtable_index().map_or(&[], |i| &i.rules)
    }

    // The coverage at an offset of `width` bytes at `at`; a subtable's first, at 2, through its index.
    #[inline(always)]
    pub(crate) fn coverage<'s>(&self, data: &'s [u8], at: usize, width: usize) -> Coverage<'s>
    where
        'a: 's,
    {
        let index = if at == 2 { self.subtable_index().and_then(|i| i.coverage.as_ref()) } else { None };
        Coverage::with_index(unless_indexed(index, data, at, width), index)
    }

    // A coverage the subtable reads past its first, through the `i`th of `rule_indexes`.
    pub(crate) fn rule_coverage<'s>(&self, data: &'s [u8], at: usize, width: usize, i: usize) -> Coverage<'s>
    where
        'a: 's,
    {
        let index = self.rule_indexes().get(i).and_then(Option::as_deref);
        Coverage::with_index(unless_indexed(index, data, at, width), index)
    }

    // A class definition the subtable reads past its first coverage, through the `i`th of `rule_indexes`.
    pub(crate) fn rule_classes<'s>(&self, data: &'s [u8], at: usize, width: usize, i: usize) -> ClassDef<'s>
    where
        'a: 's,
    {
        let index = self.rule_indexes().get(i).and_then(Option::as_deref);
        ClassDef::with_index(unless_indexed(index, data, at, width), index)
    }

    // A 16-bit PairPos format 2's first or second class definition, through the index built for it.
    pub(crate) fn pair_classes<'s>(&'s self, data: &'s [u8], at: usize, width: usize) -> ClassDef<'s> {
        let index = self.subtable_index().filter(|_| width == 2).and_then(|i| match at {
            8 => i.class1.as_ref(),
            10 => i.second.as_ref(),
            _ => None,
        });
        ClassDef::with_index(unless_indexed(index, data, at, width), index)
    }

    // A chain format 3's list lengths and record count, as the plan read them where it did.
    pub(crate) fn chain_lists(&self, data: &[u8]) -> Option<[u16; 4]> {
        match self.subtable_index().and_then(|i| i.sets.as_ref()) {
            Some(SetIndex::Lists(lists)) => Some(*lists),
            _ => context::chain_lists(data),
        }
    }

    // The subtable's digest of rule set `set`, every bit set where it has none, and its rules' first reads as far as
    // the plan kept them.
    pub(crate) fn rule_set(&self, set: u32) -> (u64, &'a [u32]) {
        match self.subtable_index().and_then(|i| i.sets.as_ref()) {
            Some(SetIndex::RuleSets(sets)) => (sets.digest(set), sets.firsts(set)),
            _ => (u64::MAX, &[]),
        }
    }

    // Lookup `index` over the whole run: front to back, or back to front in place for a reverse chain (GSUB type 8).
    fn run_lookup(&mut self, index: u16, reverse: bool) {
        if self.run.len == 0 || self.mask == 0 {
            return;
        }
        let Some(lookup) = self.enter(index) else { return };
        match (self.positions, reverse) {
            (false, false) => self.forward(&lookup, gsub::at_glyph),
            (false, true) => self.backward(&lookup, gsub::at_glyph),
            (true, _) => self.forward(&lookup, gpos::at_glyph),
        }
    }

    // Generic over the table's own glyph step, so each table's loop calls its subtables directly.
    fn forward(&mut self, lookup: &Lookup<'a>, at_glyph: impl Fn(&mut Engine<'a>, &Lookup<'a>) -> bool) {
        let rewrites = !self.positions;
        if rewrites {
            self.run.start_rewrite();
        }
        self.run.cursor = 0;
        while !self.run.failed {
            let idle = self.idle_glyphs();
            if idle > 0 {
                self.run.keep_n(idle);
            }
            if self.run.failed || self.run.cursor >= self.run.len {
                break;
            }
            let before = (self.run.cursor, self.run.written);
            if !at_glyph(self, lookup) || (self.run.cursor, self.run.written) == before {
                self.run.keep();
            }
        }
        if rewrites {
            self.run.finish_rewrite();
        }
    }

    fn backward(&mut self, lookup: &Lookup<'a>, at_glyph: impl Fn(&mut Engine<'a>, &Lookup<'a>) -> bool) {
        debug_assert!(!self.run.is_rewriting());
        for i in (0..self.run.len).rev() {
            self.run.cursor = i;
            if self.starts_at(self.run.current()) {
                at_glyph(self, lookup);
            }
        }
    }

    // Whether the lookup may start at `g`, by its digest, its mask and its flags.
    #[inline(always)]
    fn starts_at(&self, g: &Glyph) -> bool {
        self.gate.is_none_or(|d| d.may_have(g.id)) && g.mask & self.mask != 0 && !self.filter.hides(g, self.face)
    }

    // How many glyphs from the cursor the lookup can't start at. A loop of its own, as `Iterator::position` stayed
    // out of line: a call, and the engine's fields loaded again, each time a lookup moved on.
    #[inline(always)]
    fn idle_glyphs(&self) -> usize {
        let ahead = &self.run.glyphs[self.run.cursor..self.run.len];
        for (n, g) in ahead.iter().enumerate() {
            if self.starts_at(g) {
                return n;
            }
        }
        ahead.len()
    }

    // `lookup`'s subtables at the current glyph until one applies, each costing one of the run's budget so offsets
    // naming one subtable over and over can't stall it. `apply` takes the type, `MEDIUM` set for a 24-bit format.
    pub(crate) fn each_subtable(
        &mut self,
        lookup: &Lookup<'a>,
        kinds: &LookupKinds,
        mut apply: impl FnMut(&mut Engine<'a>, u16, &'a [u8]) -> bool,
    ) -> bool {
        // A lone subtable's digest is its lookup's, which the pass has read already.
        let glyph = (lookup.subtable_count > 1 || self.gate.is_none()).then(|| self.run.current().id);
        // Taken now, as a nested lookup puts back what it found when it ends.
        let indexes = self.subtables;
        self.peeks = [None; 2];
        let outcome = (0..lookup.subtable_count).try_for_each(|i| {
            let index = indexes.and_then(|all| all.get(usize::from(i)));
            if glyph.is_some_and(|g| index.is_some_and(|x| !x.digest.may_have(g))) {
                return ControlFlow::Continue(());
            }
            let pay = |e: &mut Engine| match e.run.spend(1) {
                true => ControlFlow::Continue(()),
                false => ControlFlow::Break(false),
            };
            // One that reads as none is paid for too, as an unindexed lookup may hold any number of them.
            let Some((kind, data)) = subtable_of(lookup, kinds, i, index) else { return pay(self) };
            pay(self)?;
            self.subtable = index;
            if apply(self, kind, data) { ControlFlow::Break(true) } else { ControlFlow::Continue(()) }
        });
        outcome == ControlFlow::Break(true)
    }

    // Lookup `index` at the current glyph, as a context rule's record asks: with its own flags and subtables, without
    // its mask, and passed by where its digest lacks the glyph. Past the nesting limit the run fails.
    pub(crate) fn nested(&mut self, index: u16) -> bool {
        let Some(left) = self.levels_left.checked_sub(1) else {
            self.run.failed = true;
            return false;
        };
        if !self.run.spend(1) {
            return false;
        }
        let own = self.indexes.get(usize::from(index)).and_then(Option::as_deref);
        let starts = match own {
            Some(own) => own.digest.may_have(self.run.current().id),
            None => self.applies_unindexed(index),
        };
        if !starts {
            return false;
        }
        let own = own.map(|l| &*l.subtables);
        let outer = (
            self.filter,
            self.gate.take(),
            core::mem::replace(&mut self.subtables, own),
            self.subtable,
            self.peeks,
            core::mem::replace(&mut self.levels_left, left),
        );
        let applied = self.enter(index).is_some_and(|lookup| match self.positions {
            true => gpos::at_glyph(self, &lookup),
            false => gsub::at_glyph(self, &lookup),
        });
        (self.filter, self.gate, self.subtables, self.subtable, self.peeks, self.levels_left) = outer;
        applied
    }

    // Whether a lookup the plan didn't index applies at all, told as indexing tells it (`reverse_of`), its subtables'
    // types read at a cost of one each: one of mixed types applies nothing, whatever the font's allowance left.
    #[inline(never)]
    fn applies_unindexed(&mut self, index: u16) -> bool {
        let kinds = if self.positions { &gpos::KINDS } else { &gsub::KINDS };
        let lookup = self.table.and_then(|table| table.lookup(index));
        lookup.is_some_and(|l| self.run.spend(l.subtable_count.into()) && reverse_of(&l, kinds).is_some())
    }

    // Park and Miller's minimal standard generator, for `rand`: one sequence per table run, so shaping repeats.
    pub(crate) fn next_random(&mut self) -> u32 {
        self.seed = (u64::from(self.seed) * 48_271 % 0x7FFF_FFFF) as u32;
        self.seed
    }

    // The current glyph's history and GDEF class for what it becomes: a font's own class where it has classes, else
    // the class the edit implies, else the one it had.
    fn note_edit(&mut self, glyph: u32, made: Made) {
        let face = self.face;
        let g = self.run.current_mut();
        let mut history = (g.class & class::HISTORY) | class::SUBSTITUTED;
        let implied = match made {
            Made::Substitute => 0,
            // A glyph ligated again is one ligature, no longer a piece of a glyph that multiplied: of ligate,
            // multiply, ligate, only the last counts.
            Made::Ligature(implied) => {
                history = (history | class::LIGATED) & !class::MULTIPLIED;
                implied
            }
            Made::Piece(implied) => {
                history |= class::MULTIPLIED;
                implied
            }
        };
        if face.gives_classes() {
            let (own, attach) = face.classes(glyph);
            (g.class, g.attach_class) = (own | history, attach);
        } else if implied != 0 {
            (g.class, g.attach_class) = (implied | history, 0);
        } else {
            g.class = (g.class & !class::HISTORY) | history;
        }
        self.seen.add(glyph);
    }

    // The current glyph written as `glyph`.
    pub(crate) fn substitute(&mut self, glyph: u32) {
        self.note_edit(glyph, Made::Substitute);
        self.run.replace(glyph);
    }

    // The current glyph turned into `glyph` where it stands, as a reverse chain substitutes.
    pub(crate) fn substitute_here(&mut self, glyph: u32) {
        self.note_edit(glyph, Made::Substitute);
        self.run.current_mut().id = glyph;
    }

    // One piece of what the current glyph multiplies into, written before it; `implied` the class it takes without
    // GDEF classes.
    pub(crate) fn put_piece(&mut self, glyph: u32, implied: u8) {
        self.note_edit(glyph, Made::Piece(implied));
        self.run.insert(glyph);
    }

    // The input just matched becomes `glyph`. Glyphs the match passed over stay after it, each moved onto the component
    // it followed so mark positioning puts it there; so do marks after the input still on its last part.
    pub(crate) fn ligate(&mut self, count: usize, end: usize, components: u16, glyph: u32) {
        let places = self.matched;
        let parts = &places[1..count];
        let first = self.run.cursor;
        self.run.join_clusters(first..end);
        let glyphs = &self.run.glyphs;
        let marks_follow = parts.iter().all(|&p| glyphs[p as usize].glyph_is(class::MARK));
        let mark_led = glyphs[first].glyph_is(class::MARK);
        // Marks on a base stay a base and marks on a mark stay a mark; anything else is a ligature.
        let whole = !(marks_follow && (mark_led || glyphs[first].glyph_is(class::BASE)));
        let id = if whole { self.run.next_ligature_id() } else { 0 };

        let head = self.run.current_mut();
        let mut tally = Tally { before: 0, id: head.ligature_id(), count: u32::from(head.components()) };
        if whole {
            head.make_ligature(id, components);
            if head.category == GeneralCategory::NonspacingMark {
                head.set_category(GeneralCategory::OtherLetter);
            }
        }
        self.note_edit(glyph, Made::Ligature(if whole { class::LIGATURE } else { 0 }));
        self.run.replace(glyph);
        for &part in parts {
            while self.run.cursor < part as usize && !self.run.failed {
                if whole {
                    let g = self.run.current_mut();
                    g.attach_to_ligature(id, tally.place(g.component()));
                }
                self.run.keep();
            }
            tally.pass(self.run.current());
            self.run.pass_over();
        }
        if (mark_led && marks_follow) || tally.id == 0 {
            return;
        }
        let rest = self.run.cursor..self.run.len;
        let on_last = |g: &&mut Glyph| g.ligature_id() == tally.id && g.component() != 0;
        for g in self.run.glyphs[rest].iter_mut().take_while(on_last) {
            g.attach_to_ligature(id, tally.place(g.component()));
        }
    }
}

// The new ligature's components as its parts are read: how many the parts before the last bring, and the last
// part's own ligature id and component count.
struct Tally {
    before: u32,
    id: u8,
    count: u32,
}

impl Tally {
    // Component `c` of the last part as one of the new ligature's; 0, the part as a whole, takes its last.
    fn place(&self, c: u16) -> u16 {
        let c = if c == 0 { self.count } else { u32::from(c).min(self.count) };
        u16::try_from(self.before + c).unwrap_or(u16::MAX)
    }

    fn pass(&mut self, part: &Glyph) {
        self.before += self.count;
        (self.id, self.count) = (part.ligature_id(), u32::from(part.components_joined()));
    }
}

// The table at an offset of `width` bytes at `at`, read only where the plan has no index to answer for it: an index
// answers every question shaping asks of a coverage or class definition.
#[inline(always)]
fn unless_indexed<'s>(index: Option<&SparseIndex>, data: &'s [u8], at: usize, width: usize) -> &'s [u8] {
    if index.is_some() { &[] } else { table_at_width(data, at, width) }
}

// Subtable `i` of `lookup` with its type, through what the plan resolved where it did. Inline, as a call returned
// the pair through memory on every subtable tried; only a subtable the plan didn't resolve is read apart.
#[inline(always)]
fn subtable_of<'a>(
    lookup: &Lookup<'a>,
    kinds: &LookupKinds,
    i: u16,
    index: Option<&SubtableIndex>,
) -> Option<(u16, &'a [u8])> {
    match index.and_then(|x| x.resolved) {
        Some(resolved) => resolved.map(|(kind, start)| (kind, lookup.tail(start))),
        None => unresolved(lookup, kinds, i),
    }
}

// An Extension followed to the subtable it names.
#[inline(never)]
fn unresolved<'a>(lookup: &Lookup<'a>, kinds: &LookupKinds, i: u16) -> Option<(u16, &'a [u8])> {
    resolve_extension(lookup.subtable(i), lookup.kind, kinds.extension)
        .map(|(kind, data)| (medium_kind(kind, data, kinds), data))
}

// One table's lookups over a run, each passed by unread where its glyphs' digest and the run's share none.
pub(crate) struct TableRun<'a> {
    engine: Engine<'a>,
}

impl<'a> TableRun<'a> {
    // `digest` holds at least the run's glyphs.
    pub(crate) fn new(table: Table, face: &'a Face<'a>, plan: &'a Plan, run: &'a mut Run, digest: Digest) -> Self {
        let mut engine = Engine::new(face, run, face.layout(table), table == Table::Gpos);
        engine.indexes = &plan.schedule(table).indexes;
        engine.seen = digest;
        TableRun { engine }
    }

    // A script's own lookups, not the font's: each run whole by `apply_lookup`.
    pub(crate) fn synthetic(face: &'a Face<'a>, gsub: &'a LayoutTable<'a>, run: &'a mut Run) -> Self {
        TableRun { engine: Engine::new(face, run, Some(*gsub), false) }
    }

    pub(crate) fn apply(&mut self, lookup: &StageLookup) {
        let e = &mut self.engine;
        let indexes = e.indexes;
        let Some(index) = indexes.get(usize::from(lookup.index)).and_then(Option::as_deref) else { return };
        if !index.digest.may_intersect(&e.seen) {
            return;
        }
        (e.gate, e.subtables) = (Some(&index.digest), Some(index.subtables.as_slice()));
        (e.skips_zwnj, e.skips_zwj, e.random) = (lookup.skips_zwnj, lookup.skips_zwj, lookup.random);
        e.per_syllable = !e.positions && lookup.per_syllable;
        e.begin(lookup.mask);
        e.run_lookup(lookup.index, !e.positions && index.reverse);
        debug_assert!(
            e.run.glyphs[..e.run.len].iter().all(|g| e.seen.may_have(g.id)),
            "a lookup wrote a glyph the run's digest missed",
        );
    }

    pub(crate) fn apply_lookup(&mut self, index: u16, mask: Mask, digest: Option<&'a Digest>) {
        let e = &mut self.engine;
        e.begin(mask);
        e.gate = digest;
        e.run_lookup(index, false);
    }

    pub(crate) fn run(&mut self) -> &mut Run {
        self.engine.run
    }

    pub(crate) fn glyphs_changed(&mut self) {
        let run = &*self.engine.run;
        self.engine.seen = Digest::of(&run.glyphs[..run.len]);
    }

    pub(crate) fn digest(self) -> Digest {
        self.engine.seen
    }
}
