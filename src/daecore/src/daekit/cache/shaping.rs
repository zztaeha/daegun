use super::*;
use crate::daecore::daetype::subsetter::{self, CharMap, CharTables, UvsLookup};
use alloc::sync::Arc;

// What an index is built from: a function giving its entries, called twice (`SparseIndex::build`).
pub(crate) type IndexEntries<'a> = dyn Fn(&mut crate::daecore::daetype::format::index::Entries) -> Option<()> + 'a;

pub(crate) struct Drawn {
    font: Shared<FontState>,
    bytes: usize,
}

impl Drop for Drawn {
    fn drop(&mut self) {
        self.font.index_allowance().add(self.bytes);
    }
}

// Built under the guard, which `build` reads only through what it's handed, so a racing caller waits rather than
// drawing a second copy's bytes. Past the slots, built and not kept; `held` pays for the slots and what they keep.
fn lookup_slot<S: Default, T: Clone>(
    (cells, held): (&[Mutable<S>], &crate::daecore::daekit::base::sync::Counter),
    slots_of: fn(&mut S) -> &mut Vec<Option<T>>,
    (table_index, lookup_count, index): (usize, usize, usize),
    build: impl FnOnce(&mut S) -> T,
    heap: impl FnOnce(&T) -> usize,
) -> T {
    let Some(cell) = cells.get(table_index) else { return build(&mut S::default()) };
    let mut cell = write(cell);
    let slots = slots_of(&mut cell);
    if slots.len() != lookup_count {
        slots.clear();
        slots.resize_with(lookup_count, || None);
        held.add(crate::daecore::daekit::base::sync::heap_of(slots));
    }
    match slots.get(index) {
        Some(Some(hit)) => return hit.clone(),
        Some(None) => {}
        None => return build(&mut cell),
    }
    let built = build(&mut cell);
    held.add(heap(&built));
    slots_of(&mut cell)[index] = Some(built.clone());
    built
}

impl FontState {
    // What a described location holds, once `described_tables` has been asked and answered with them.
    #[inline]
    pub(super) fn described_held(&self) -> &Tables {
        match self.described_tables.get() {
            Some(Ok(kept)) => kept,
            _ => {
                debug_assert!(false, "a described location is built once its tables are");
                &self.tables
            }
        }
    }

    // Drawn from the font's allowance, so None once the allowance can't cover it.
    fn build_index(
        &self,
        each: &IndexEntries,
        absent: u16,
    ) -> Option<crate::daecore::daetype::format::index::SparseIndex> {
        let left = self.index_allowance().get();
        let index = crate::daecore::daetype::format::index::SparseIndex::build(each, absent, left)?;
        self.index_allowance().draw(index.bytes()).then_some(index)
    }

    // The allowance, at first the floor and INDEX_BYTES_PER_TABLE_BYTE a byte the font's tables span.
    pub(super) fn index_allowance(&self) -> &crate::daecore::daekit::base::sync::Counter {
        self.index_allowance.get_or_init(|| {
            let spanned = INDEX_BYTES_PER_TABLE_BYTE.saturating_mul(distinct_bytes(&self.tables));
            crate::daecore::daekit::base::sync::Counter::new(spanned.saturating_add(INDEX_FLOOR_BYTES))
        })
    }
}

impl FontCache {
    pub(crate) fn lookup_index_cached(
        &self,
        table_index: usize,
        lookup_count: usize,
        index: u16,
        build: impl FnOnce(&mut OtlIndexes) -> crate::daecore::daeshaper::ot::LookupIndex,
    ) -> Shared<crate::daecore::daeshaper::ot::LookupIndex> {
        use crate::daecore::daeshaper::ot::LookupIndex;
        use crate::daecore::daekit::base::sync::shared_of;
        let heap = |built: &Shared<LookupIndex>| shared_of::<LookupIndex>() + built.heap_bytes();
        let cells = (&self.lookup_indexes[..], &self.held.built);
        let build = |(_, indexes): &mut (LookupIndexSlots, OtlIndexes)| Shared::new(build(indexes));
        lookup_slot(cells, |(slots, _)| slots, (table_index, lookup_count, index.into()), build, heap)
    }

    pub(crate) fn subtable_sets_cached(
        &self,
        table: crate::daecore::daetype::format::sanitize::AatTable,
        subtable_count: usize,
        index: usize,
        build: impl FnOnce() -> crate::daecore::daeshaper::ot::aat::SubtableSets,
    ) -> Shared<crate::daecore::daeshaper::ot::aat::SubtableSets> {
        use crate::daecore::daeshaper::ot::aat::SubtableSets;
        use crate::daecore::daekit::base::sync::shared_of;
        let heap = |sets: &Shared<SubtableSets>| shared_of::<SubtableSets>() + sets.heap_bytes();
        let cells = (&self.aat_sets[..], &self.held.built);
        lookup_slot(cells, |slots| slots, (table as usize, subtable_count, index), |_| Shared::new(build()), heap)
    }

    // `bytes` drawn from the font's allowance for an index this cache keeps, and given back when it's dropped:
    // false once the allowance can't cover them.
    pub(crate) fn reserve_index_bytes(&self, bytes: usize) -> bool {
        if !self.font.index_allowance().draw(bytes) {
            return false;
        }
        self.index_spent.add(bytes);
        true
    }

    // `bytes` drawn from the font's allowance for data a plan keeps, given back when the `Drawn` is dropped, as a
    // plan can go long before its location: None once the allowance can't cover them.
    pub(crate) fn draw_index_bytes(&self, bytes: usize) -> Option<Drawn> {
        self.font.index_allowance().draw(bytes).then(|| Drawn { font: Shared::clone(&self.font), bytes })
    }

    // Drawn from the font's allowance, and given back when this cache is dropped.
    pub(crate) fn build_index(
        &self,
        each: &IndexEntries,
        absent: u16,
    ) -> Option<crate::daecore::daetype::format::index::SparseIndex> {
        let index = self.font.build_index(each, absent)?;
        self.index_spent.add(index.bytes());
        Some(index)
    }

    // What shaping reads of this cache's location against `font`, the font's own cache, built once.
    pub(crate) fn variation(&self, font: &FontCache) -> &crate::daecore::daeshaper::face::Variation {
        use crate::daecore::{daeshaper::face::Variation, daekit::base::sync::shared_of};
        let build = || Shared::new(Variation::new(font, self.location.as_deref().map_or(&[], |l| &l.coords[..])));
        let charge = |v: &Shared<Variation>| self.held.built.add(shared_of::<Variation>() + v.heap_bytes());
        once_slot(&self.variation, build, charge).as_ref()
    }

    // GDEF's glyph classes, mark attachment classes and each mark glyph set's coverage, None for one that can't be
    // indexed.
    pub(crate) fn gdef_class_indexes(&self, gdef: Option<&crate::daecore::daeshaper::ot::Gdef>) -> &GdefClassIndexes {
        use crate::daecore::{daeshaper::ot::{ClassDef, COVERAGE_ABSENT}, daetype::format::index::SparseIndex};
        use crate::daecore::daekit::base::sync::shared_of;
        let build = || {
            let index = |c: Option<&ClassDef>| c.and_then(|c| self.build_index(&|e| c.each_entry(e), 0));
            let index = |c| index(c).map(Shared::new);
            let set = |i| gdef?.mark_set(i).and_then(|c| self.build_index(&|e| c.each_entry(e), COVERAGE_ABSENT));
            let sets: Vec<_> = (0..gdef.map_or(0, |g| g.mark_glyph_set_count())).map(set).collect();
            let classes = index(gdef.and_then(|g| g.glyph_classes.as_ref()));
            let marks = index(gdef.and_then(|g| g.mark_attach_classes.as_ref()));
            (classes, marks, sets.iter().any(Option::is_some).then(|| Shared::new(sets)))
        };
        let charge = |(classes, marks, sets): &GdefClassIndexes| {
            let bytes = |i: &Option<Shared<SparseIndex>>| {
                i.as_ref().map_or(0, |i| shared_of::<SparseIndex>() + i.bytes())
            };
            let sets = sets.as_ref().map_or(0, |s| {
                shared_of::<Vec<Option<SparseIndex>>>()
                    + crate::daecore::daekit::base::sync::heap_of(s)
                    + s.iter().flatten().map(SparseIndex::bytes).sum::<usize>()
            });
            self.held.built.add(bytes(classes) + bytes(marks) + sets);
        };
        once_slot(&self.gdef_class_indexes, build, charge)
    }

    // GSUB's or GPOS's index of the coverage or class definition at `key`, built once for every subtable naming it,
    // in `map`, which a lookup's index is handed with its table's guard held.
    pub(crate) fn otl_index_in(
        &self,
        map: &mut OtlIndexes,
        key: (usize, bool),
        build: impl FnOnce() -> Option<crate::daecore::daetype::format::index::SparseIndex>,
    ) -> Option<Shared<crate::daecore::daetype::format::index::SparseIndex>> {
        use crate::daecore::{daetype::format::index::SparseIndex, daekit::base::sync::shared_of};
        if let Some(hit) = map.get(&key) {
            return hit.clone();
        }
        let built = build().map(Shared::new);
        let entry = core::mem::size_of::<((usize, bool), Option<Shared<SparseIndex>>)>();
        self.held.built.add(entry + built.as_ref().map_or(0, |i| shared_of::<SparseIndex>() + i.bytes()));
        map.insert(key, built.clone());
        built
    }

    pub fn shaped_run(
        &self, axes: &AxisKey, text: &str, vertical: bool,
    ) -> Arc<crate::daecore::daekit::text::shape::ShapedRun> {
        let ctx = crate::daecore::daekit::cache::RunContext::default();
        self.shaped_run_in_context(axes, text, vertical, &ctx)
    }

    pub fn shaped_run_in_context(
        &self, axes: &AxisKey, text: &str, vertical: bool,
        ctx: &crate::daecore::daekit::cache::RunContext,
    ) -> Arc<crate::daecore::daekit::text::shape::ShapedRun> {
        match self.run_in_context(axes, text.as_bytes(), Some(text), vertical, ctx) {
            Some(run) => run,
            None => unreachable!("a miss given its text as a `str` shapes it"),
        }
    }

    /// [`Self::shaped_run_in_context`] over bytes that may not be UTF-8, read as UTF-8 only where the cache misses,
    /// as a held run's text was when it was shaped: `None` where they aren't.
    #[cfg(feature = "capi")]
    pub fn shaped_bytes_in_context(
        &self, axes: &AxisKey, text: &[u8], vertical: bool, ctx: &crate::daecore::daekit::cache::RunContext,
    ) -> Option<Arc<crate::daecore::daekit::text::shape::ShapedRun>> {
        self.run_in_context(axes, text, None, vertical, ctx)
    }

    // The run of `bytes`, which a miss shapes as `text` or, given none, reads as UTF-8 first: `None` where they
    // aren't. One body for both, as a second copy of a hit's path left less of each in line.
    fn run_in_context(
        &self, axes: &AxisKey, bytes: &[u8], text: Option<&str>, vertical: bool,
        ctx: &crate::daecore::daekit::cache::RunContext,
    ) -> Option<Arc<crate::daecore::daekit::text::shape::ShapedRun>> {
        let n = crate::daecore::daeshaper::run::CONTEXT_CHARS;
        let pre = ctx.before.char_indices().rev().nth(n - 1).map_or(ctx.before, |(at, _)| &ctx.before[at..]);
        let post = ctx.after.char_indices().nth(n).map_or(ctx.after, |(at, _)| &ctx.after[..at]);

        // Looked up borrowed, so a hit copies nothing it only compares; a miss builds the owned key it keeps.
        let seed = ctx.seed_script.map(|s| s.0);
        let head: RunHead<'_> = (axes, bytes, vertical, ctx.rtl, pre, post, ctx.language, seed);
        let parts: RunParts<'_> = (head, ctx.features);
        if let Some(run) = self.front_run(&parts) {
            return Some(run);
        }
        // A busy cache isn't waited for, to read or to keep: threads queueing took 17× as long a shape at 8 threads,
        // and a later miss keeps the run. This one is shaped either way.
        if let Some((list, at)) = self.run_list(&parts.0)
            && let Some(cache) = try_read_shared(list)
        {
            self.stamp_front(&cache, at);
            if let Some((key, hit)) = cache.get_held(&parts as &dyn RunKey) {
                let hit = Arc::clone(hit);
                self.remember_run(key, &parts, &hit, at);
                return Some(hit);
            }
        }
        let text = match text {
            Some(text) => text,
            None => core::str::from_utf8(bytes).ok()?,
        };
        Some(self.shape_and_keep(&parts, text, ctx))
    }

    // A miss, shaped and kept if it fits: out of line, so a hit's path saves none of the registers shaping uses.
    #[inline(never)]
    fn shape_and_keep(
        &self, parts: &RunParts<'_>, text: &str, ctx: &crate::daecore::daekit::cache::RunContext,
    ) -> Arc<crate::daecore::daekit::text::shape::ShapedRun> {
        let &((axes, _, vertical, rtl, pre, post, language, _), features) = parts;
        let opts = crate::daecore::daekit::text::shape::ShapeOptions {
            before: pre,
            after: post,
            language,
            seed_script: ctx.seed_script,
            features,
            ..Default::default()
        };
        let run = crate::daecore::daekit::text::shape::shape_run(self, axes, text, vertical, rtl, &opts);
        if let Some(picked) = self.run_list(&parts.0) {
            self.keep_run(parts, text, &run, picked);
        }
        run
    }

    // Kept only if the list picked before its lock is still the run's: a budget changing the count of lists in between
    // would leave the run where no read looks, counted past the ceiling.
    fn keep_run(&self, parts: &RunParts<'_>, text: &str, run: &Arc<Run>, (list, at): (&Shareable<RunCache>, usize)) {
        let &((axes, _, vertical, rtl, pre, post, language, seed), features) = parts;
        let (language, kept) = (language.map(String::from), features.to_vec());
        let key = (axes.clone(), text.into(), vertical, rtl, pre.into(), post.into(), language, seed, kept);
        if let Some(mut cache) = try_write_shared(list)
            && self.run_list(&parts.0).is_some_and(|(_, now)| now == at)
            && shape_cost(&key, run) <= (cache.budget() / RUN_PART).max(RUN_FLOOR)
        {
            let key = Shared::new(key);
            if let Some(evicted) = cache.insert(Shared::clone(&key), Arc::clone(run)) {
                self.dropped_runs(evicted);
                self.remember_run(&key, parts, run, at);
            }
            let over = cache.over();
            drop(cache);
            lru::settle(over);
        }
    }

    pub fn shaped_run_justified(
        &self, axes: &AxisKey, text: &str, vertical: bool,
        mods: &crate::daecore::daetype::jstf::JstfModLists, shrink: bool,
    ) -> Arc<crate::daecore::daekit::text::shape::ShapedRun> {
        crate::daecore::daekit::text::shape::shape_run_justified(self, axes, text, vertical, mods, shrink)
    }

    pub(crate) fn cmap_index(&self) -> Option<&crate::daecore::daetype::format::index::SparseIndex> {
        self.font.cmap.index.get()?.as_deref()
    }

    // The index first: one is built only from the tables, so it answers without looking them up. Without it, a code
    // looked up again is answered by `recent`. A glyph past 16 bits refuses both, so its code reads the tables.
    pub fn glyph_id(&self, codepoint: u32) -> Option<u32> {
        if let Some(Some(index)) = self.font.cmap.index.get() {
            return Some(u32::from(index.lookup(codepoint))).filter(|&g| g != 0);
        }
        let held = codepoint <= 0x10_FFFF;
        if held && let Some(glyph) = self.font.cmap.recent.get(codepoint) {
            return Some(glyph).filter(|&g| g != 0);
        }
        let (map, tables) = self.char_map();
        let glyph = map.glyph_id(tables, codepoint);
        if held {
            self.font.cmap.recent.put(codepoint, glyph.unwrap_or(0));
        }
        glyph
    }

    // `glyph_id` inlined for the API's per-character loops (whole and forced, the shaper took 3% longer), each path
    // filtering its own answer: shaped otherwise, Rust's loop or C's took 5% to 17% longer.
    #[inline(always)]
    pub(crate) fn inline_glyph_id(&self, codepoint: u32) -> Option<u32> {
        if let Some(Some(index)) = self.font.cmap.index.get() {
            return Some(u32::from(index.lookup(codepoint))).filter(|&g| g != 0);
        }
        if codepoint <= 0x10_FFFF && let Some(glyph) = self.font.cmap.recent.get(codepoint) {
            return Some(glyph).filter(|&g| g != 0);
        }
        Some(self.glyph_id(codepoint).unwrap_or(0)).filter(|&g| g != 0)
    }

    // The subtables shaping reads, chosen at the first lookup needing them, kept with their tables (DMAP only where
    // it reads): by tag they took 453 of Inter's 1,080 instructions a lookup. Inlined, as a call cost up to 18 more.
    #[inline(always)]
    fn char_map(&self) -> (&CharMap, CharTables<'_>) {
        let build = || {
            let table = |tag: &str| self.table(tag);
            let bytes = |tag: &str| table(tag).map(|t| &t[..]);
            let map = CharMap::new(subsetter::char_tables(bytes));
            let dmap = table("DMAP").filter(|_| map.reads_dmap()).cloned();
            (map, [table("cmap").cloned(), dmap])
        };
        let (map, [cmap, dmap]) = once_slot(&self.font.cmap.lookup, build, |_| {});
        (map, CharTables { cmap: cmap.as_deref().unwrap_or_default(), dmap: dmap.as_deref().unwrap_or_default() })
    }

    // Every code the font maps, ascending.
    pub(crate) fn cmap_coverage(&self) -> Vec<(u32, u32)> {
        let (map, tables) = self.char_map();
        map.entries(tables)
    }

    // The font's, built once whichever location asks first.
    pub(crate) fn warm_cmap_index(&self) {
        let build = || {
            let (map, tables) = self.char_map();
            let mut runs = crate::daecore::daetype::format::index::Runs::default();
            map.each_entry(tables, &mut runs);
            self.font.build_index(&|e| { runs.put_to(e); Some(()) }, 0).map(Shared::new)
        };
        once_slot(&self.font.cmap.index, build, |_| {});
    }

    pub fn variation_glyph_id(&self, base: u32, selector: u32) -> Option<u32> {
        let (map, tables) = self.char_map();
        match map.variation(tables, base, selector)? {
            UvsLookup::Explicit(gid) => Some(gid),
            UvsLookup::UseDefault => self.glyph_id(base),
        }
    }
}

// A plan's key on the stack, where it nearly always fits; a longer one moves to the heap.
struct KeyBytes {
    inline: [u8; KEY_INLINE],
    len: usize,
    spilled: Vec<u8>,
}

const KEY_INLINE: usize = 192;

impl KeyBytes {
    fn put(&mut self, bytes: &[u8]) {
        let end = self.len + bytes.len();
        if self.spilled.is_empty() && end <= KEY_INLINE {
            self.inline[self.len..end].copy_from_slice(bytes);
            self.len = end;
        } else {
            if self.spilled.is_empty() {
                self.spilled.extend_from_slice(&self.inline[..self.len]);
            }
            self.spilled.extend_from_slice(bytes);
        }
    }

    fn get(&self) -> &[u8] {
        if self.spilled.is_empty() { &self.inline[..self.len] } else { &self.spilled }
    }
}

#[allow(clippy::too_many_arguments, reason = "everything a plan is keyed on, and no fewer")]
fn plan_key(
    script: Option<crate::daecore::daeshaper::unicode::Script>,
    direction: crate::daecore::daeshaper::run::Direction,
    script_tags: Option<&[crate::daecore::daeshaper::ot::tag::Tag]>,
    language: Option<&str>,
    user_features: &[crate::daecore::daeshaper::plan::FeatureSetting],
    lookup_overrides: &[crate::daecore::daeshaper::plan::LookupSwitch],
    variations: Option<&crate::daecore::daeshaper::face::LayoutVariation>,
    exact_ranges: bool,
) -> KeyBytes {
    use crate::daecore::daeshaper::ot::tag::canonical_language_bytes;
    use crate::daecore::daeshaper::plan::Table;
    use crate::daecore::daeshaper::run::Direction;

    let mut k = KeyBytes { inline: [0; KEY_INLINE], len: 0, spilled: Vec::new() };
    match script {
        Some(s) => {
            k.put(&[1]);
            k.put(&s.0.to_le_bytes());
        }
        None => k.put(&[0]),
    }
    k.put(&[match direction {
        Direction::LeftToRight => 0,
        Direction::RightToLeft => 1,
        Direction::TopToBottom => 2,
        Direction::BottomToTop => 3,
    }]);
    k.put(&[u8::from(script_tags.is_some())]);
    if let Some(tags) = script_tags {
        k.put(&(tags.len() as u32).to_le_bytes());
        for t in tags {
            k.put(&t.0.to_le_bytes());
        }
    }
    // Ended by a NUL, which the canonical form never holds.
    let language = language.and_then(|l| canonical_language_bytes(l.as_bytes()));
    k.put(&[u8::from(language.is_some())]);
    if let Some(bytes) = language {
        bytes.for_each(|b| k.put(&[b]));
        k.put(&[0]);
    }
    k.put(&(user_features.len() as u32).to_le_bytes());
    for f in user_features {
        k.put(&f.tag.0.to_le_bytes());
        k.put(&f.value.to_le_bytes());
        // An OpenType plan takes a feature's range from each shape, so it is keyed only on whether the feature is
        // global; morx builds its flags for the ranges into the plan.
        if exact_ranges {
            k.put(&f.start.to_le_bytes());
            k.put(&f.end.to_le_bytes());
        } else {
            k.put(&[u8::from(f.covers_all())]);
        }
    }
    k.put(&(lookup_overrides.len() as u32).to_le_bytes());
    for o in lookup_overrides {
        k.put(&[match o.table { Table::Gsub => 0, Table::Gpos => 1 }]);
        k.put(&o.lookup.to_le_bytes());
        k.put(&[u8::from(o.on)]);
    }
    if let Some(variations) = variations {
        for (record, conditions) in variations.records.iter().zip(&variations.conditions) {
            k.put(&record.map_or(0, |v| u64::from(v) + 1).to_le_bytes());
            k.put(&(conditions.len() as u32).to_le_bytes());
            conditions.iter().for_each(|c| k.put(&c.to_le_bytes()));
        }
    }
    k
}

// The first plans a font is asked for, each kept with its key and read without a lock. They are never emptied, so
// past them plans go to the memo, as does a morx plan built for one call's ranges.
const KEPT_PLANS: usize = 32;

type KeptPlan = (alloc::boxed::Box<[u8]>, crate::daecore::daeshaper::plan::Plan);

pub(crate) type KeptPlans = [Once<alloc::boxed::Box<KeptPlan>>; KEPT_PLANS];

pub(crate) enum PlanRef<'a> {
    Kept(&'a crate::daecore::daeshaper::plan::Plan),
    Memo(Shared<crate::daecore::daeshaper::plan::Plan>),
}

impl core::ops::Deref for PlanRef<'_> {
    type Target = crate::daecore::daeshaper::plan::Plan;

    fn deref(&self) -> &Self::Target {
        match self {
            PlanRef::Kept(plan) => plan,
            PlanRef::Memo(plan) => plan,
        }
    }
}

// The kept plan for `key`, built into the first empty slot when none has it. Slots fill in order, so a key that is
// kept is found before any slot still being built, and only a miss waits on another thread's build.
fn kept_plan<'a>(
    slots: &'a KeptPlans,
    key: &[u8],
    build: impl Fn() -> crate::daecore::daeshaper::plan::Plan,
) -> Option<&'a crate::daecore::daeshaper::plan::Plan> {
    slots.iter().find_map(|slot| {
        let kept = slot.get_or_init(|| alloc::boxed::Box::new((key.into(), build())));
        (*kept.0 == *key).then_some(&kept.1)
    })
}

impl FontCache {
    // `script_tags` stand in for the script's own and `language` is the caller's tag; the tags of both are worked out
    // only to build a plan, so a cached one never pays for them.
    #[allow(clippy::too_many_arguments, reason = "everything a plan is keyed on, and no fewer")]
    pub(crate) fn shape_plan_cached(
        &self,
        script: Option<crate::daecore::daeshaper::unicode::Script>,
        face: &crate::daecore::daeshaper::face::Face,
        direction: crate::daecore::daeshaper::run::Direction,
        script_tags: Option<&[crate::daecore::daeshaper::ot::tag::Tag]>,
        language: Option<&str>,
        user_features: &[crate::daecore::daeshaper::plan::FeatureSetting],
        lookup_overrides: &[crate::daecore::daeshaper::plan::LookupSwitch],
    ) -> PlanRef<'_> {
        use crate::daecore::daeshaper::{ot::tag, plan::{Plan, PlanRequest}};
        use crate::daecore::daetype::format::sanitize::AatTable;
        let exact_ranges = face.aat(AatTable::Morx).is_some();
        let key = plan_key(
            script, direction, script_tags, language, user_features, lookup_overrides, face.variation_key(),
            exact_ranges,
        );
        let build = || {
            let own = script.map(tag::script_tags);
            let scripts = script_tags.unwrap_or_else(|| own.as_ref().map_or(&[], |t| t.as_slice()));
            let language = language.and_then(|l| tag::canonical_language(l.as_bytes()));
            let systems = language.as_deref().map(tag::language_tags);
            let systems = systems.as_ref().map_or(&[][..], |t| t.as_slice());
            let request = PlanRequest {
                script,
                direction,
                script_tags: scripts,
                language_tags: systems,
                language: language.as_deref(),
                features: user_features,
                switches: lookup_overrides,
            };
            Plan::new(face, &request)
        };
        if !(exact_ranges && user_features.iter().any(|f| !f.covers_all())) {
            let slots = self.kept_plans.get_or_init(|| alloc::boxed::Box::new(core::array::from_fn(|_| Once::new())));
            if let Some(plan) = kept_plan(slots, key.get(), build) {
                return PlanRef::Kept(plan);
            }
        }
        let plans = self.plans();
        if let Some(hit) = read(plans).get(key.get()) {
            return PlanRef::Memo(Shared::clone(hit));
        }
        // Built outside the lock, so racing callers may both build, and the first to keep its plan wins.
        let built = Shared::new(build());
        let mut plans = write(plans);
        if let Some(raced) = plans.get(key.get()) {
            return PlanRef::Memo(Shared::clone(raced));
        }
        plans.insert(key.get().into(), Shared::clone(&built));
        let over = plans.over();
        drop(plans);
        lru::settle(over);
        PlanRef::Memo(built)
    }
}

// A plan as the memo holds it: its key twice, as the list holds every key, its nodes and the plan.
pub(super) fn plan_cost(key: &alloc::boxed::Box<[u8]>, plan: &Shared<crate::daecore::daeshaper::plan::Plan>) -> usize {
    use crate::daecore::daeshaper::plan::Plan;
    use crate::daecore::daekit::base::sync::{btree_entry_of, shared_of};
    type Key = alloc::boxed::Box<[u8]>;
    btree_entry_of::<Key, (Shared<Plan>, u64)>() + btree_entry_of::<u64, Key>() + 2 * key.len()
        + shared_of::<Plan>() + plan.heap_bytes()
}

#[cfg(feature = "threading")]
std::thread_local! {
    // Each thread's spare, which any font's shape may take: a run holds nothing of a font once reset.
    static SPARE_BUFFER: core::cell::Cell<Option<crate::daecore::daeshaper::run::Run>> =
        const { core::cell::Cell::new(None) };
}

impl FontCache {
    pub(crate) fn take_buffer(&self) -> crate::daecore::daeshaper::run::Run {
        #[cfg(feature = "threading")]
        let spare = SPARE_BUFFER.try_with(core::cell::Cell::take).ok().flatten();
        #[cfg(not(feature = "threading"))]
        let spare = write(&self.spare_buffer).take();
        spare.unwrap_or_default()
    }

    pub(crate) fn give_buffer(&self, mut buf: crate::daecore::daeshaper::run::Run) {
        if buf.glyph_capacity() > SPARE_BUFFER_GLYPHS {
            return;
        }
        buf.reset();
        // A thread past its locals' teardown drops the buffer instead.
        #[cfg(feature = "threading")]
        let _ = SPARE_BUFFER.try_with(|spare| spare.set(Some(buf)));
        #[cfg(not(feature = "threading"))]
        {
            *write(&self.spare_buffer) = Some(buf);
        }
    }
}

type Run = crate::daecore::daekit::text::shape::ShapedRun;

// Under `threading` a thread rereads its recent runs without the run cache's lock, which every hit would take, each
// read's mark owed until its next turn there: one thread drops just what it would have, several may drop otherwise.
#[cfg(feature = "threading")]
const FRONT_RUNS: usize = 32;

// How many lists a font's runs are split over, under `threading`, each its own lock: one lock's count, written by
// every read, took 6× as long a hit at 8 threads.
#[cfg(feature = "threading")]
const RUN_SHARDS: usize = 16;

// A budget too small for this a list is one list, where runs go in the order of their use however they hash.
#[cfg(feature = "threading")]
const RUN_SHARD_BYTES: usize = 256 * 1024;

#[cfg(feature = "threading")]
pub(super) struct RunShards {
    // Each on lines of its own, as a shard is 120 bytes and a reader's lock written beside another's slowed both.
    lists: [Lined<Shareable<RunCache>>; RUN_SHARDS],
    // How many hold runs: none, under a budget of 0, one or all.
    in_use: crate::daecore::daekit::base::sync::Counter,
    // The cache's number, which its runs in a front name.
    id: Id,
}

// A pool evicting pays the marks its thread's front owes the list first, as the cache's own eviction does, and lets go
// of the thread's slots of the cache after. Another thread may serve a run a pool dropped until a run lands on its
// slot, which holds it all the while: counting the drops as the cache does would put one more load on every hit.
#[cfg(feature = "threading")]
impl lru::Member for RunShards {
    fn evict(&self, pool: &lru::Pool, at: usize, stamp: u64) {
        let Some(list) = self.lists.get(at) else { return };
        let mut list = write_shared(list);
        stamp_front(self.id, &list, at);
        if list.evict_listed(pool, stamp) > 0 {
            forget_front(self.id);
        }
    }
}

// A run this thread read through a run cache: the cache's number and how many runs it had dropped then, the key as
// the cache holds it, which list holds it, the run, and whether a read of it owes the cache its mark.
#[cfg(feature = "threading")]
struct FrontRun {
    cache: Id,
    list: usize,
    dropped: usize,
    key: Shared<ShapeKey>,
    run: Arc<Run>,
    owed: bool,
}

#[cfg(feature = "threading")]
std::thread_local! {
    // The thread's front. A run stays, outside the cache's budget, until another lands on its slot, its cache drops a
    // run on this thread or the thread ends.
    static FRONT: core::cell::RefCell<[Option<FrontRun>; FRONT_RUNS]> =
        const { core::cell::RefCell::new([const { None }; FRONT_RUNS]) };
}

#[cfg(feature = "threading")]
pub(crate) fn release_thread() {
    let _ = SPARE_BUFFER.try_with(core::cell::Cell::take);
    let _ = FRONT.try_with(|front| front.try_borrow_mut().ok().map(|mut slots| core::mem::take(&mut *slots)));
}

// Lets go of this thread's slots of cache `id`, which then can't be read.
#[cfg(feature = "threading")]
fn forget_front(id: Id) {
    let _ = FRONT.try_with(|front| {
        let Ok(mut front) = front.try_borrow_mut() else { return };
        front.iter_mut().filter(|s| s.as_ref().is_some_and(|s| s.cache == id)).for_each(|s| *s = None);
    });
}

// Pays the marks this thread's front owes list `at` of cache `id`, with the list's lock held.
#[cfg(feature = "threading")]
fn stamp_front(id: Id, cache: &RunCache, at: usize) {
    let _ = FRONT.try_with(|front| {
        let Ok(mut front) = front.try_borrow_mut() else { return };
        let mine = |s: &&mut FrontRun| s.cache == id && s.list == at && s.owed;
        for slot in front.iter_mut().flatten().filter(mine) {
            slot.owed = false;
            cache.get(&slot.key);
        }
    });
}

// A run's hash, from its cache, its text's length and ends and the context nearest them, one multiply a hit pays for:
// fonts given one text land apart, as one character shaped in many contexts does.
#[cfg(feature = "threading")]
fn run_hash(cache: Id, head: &RunHead<'_>) -> u64 {
    let word = |bytes: &[u8], end: Option<&[u8; 8]>| match end {
        Some(end) => u64::from_le_bytes(*end),
        None => bytes.iter().fold(0, |w, &b| w << 8 | u64::from(b)),
    };
    let (text, before, after) = (head.1, head.4.as_bytes(), head.5.as_bytes());
    let ends = [
        word(text, text.first_chunk()),
        word(text, text.last_chunk()).rotate_left(16),
        word(before, before.last_chunk()).rotate_left(32),
        word(after, after.first_chunk()).rotate_left(48),
    ];
    let seed = text.len() as u64 ^ cache;
    ends.iter().fold(seed, |h, w| h ^ w).wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

// A run's slot in the front, from the hash's top bits.
#[cfg(feature = "threading")]
fn front_slot(cache: Id, head: &RunHead<'_>) -> usize {
    (run_hash(cache, head) >> (64 - FRONT_RUNS.trailing_zeros())) as usize
}

// A run's list of `in_use`, from the hash's bits after the front's, so the runs of one slot don't share one.
#[cfg(feature = "threading")]
fn list_of(cache: Id, head: &RunHead<'_>, in_use: usize) -> usize {
    let bits = FRONT_RUNS.trailing_zeros() + RUN_SHARDS.trailing_zeros();
    ((run_hash(cache, head) >> (64 - bits)) as usize) & (in_use - 1)
}

#[cfg(feature = "threading")]
impl FontCache {
    fn run_shards(&self) -> &Shared<RunShards> {
        let build = || {
            let clock = Shared::new(super::lru::Clock::new(0));
            let list = |_| {
                let list: RunCache = lru::ByteLru::new(SHAPE_CACHE_BYTES / RUN_SHARDS, |key, run| shape_cost(key, run));
                Lined(Shareable::new(list.stamped_by(Shared::clone(&clock))))
            };
            let in_use = crate::daecore::daekit::base::sync::Counter::new(RUN_SHARDS);
            Shared::new(RunShards { lists: core::array::from_fn(list), in_use, id: self.runs_id })
        };
        self.run_shards.get_or_init(build)
    }

    // The list that holds a run and which it is, or None under a budget of 0: threads shaping past the cache then
    // meet at no lock, where one took twice as long a shape at 8 threads.
    fn run_list(&self, head: &RunHead<'_>) -> Option<(&Shareable<RunCache>, usize)> {
        let shards = self.run_shards();
        let in_use = shards.in_use.get();
        (in_use > 0).then(|| {
            let at = list_of(self.runs_id, head, in_use);
            (&shards.lists[at].0, at)
        })
    }

    // Each list takes its share of `runs` of a shape budget of `bytes`, which sets how many lists. Where the count
    // changes, runs move oldest first with their stamps, so a lower ceiling keeps those read again, then the newest,
    // and fronts drop theirs.
    pub(super) fn set_runs_bytes(&self, bytes: usize, runs_bytes: usize) {
        let shards = self.run_shards();
        let mut lists: Vec<_> = shards.lists.iter().map(|list| write_shared(list)).collect();
        lists.iter().enumerate().for_each(|(at, list)| self.stamp_front(list, at));
        let in_use = match bytes {
            0 => 0,
            _ if bytes / RUN_SHARDS >= RUN_SHARD_BYTES => RUN_SHARDS,
            _ => 1,
        };
        let held = in_use.max(1);
        let mut evicted = 0;
        if in_use == shards.in_use.get() {
            lists[..held].iter_mut().for_each(|list| evicted += list.set_budget(runs_bytes / held));
        } else {
            let mut runs: Vec<_> = lists.iter_mut().flat_map(|list| list.take_entries()).collect();
            runs.sort_unstable_by_key(|&(used, ..)| used);
            shards.in_use.set(in_use);
            lists.iter_mut().for_each(|list| evicted += list.set_budget(runs_bytes / held));
            for (used, key, run) in runs {
                let at = list_of(self.runs_id, &key.parts().0, held);
                evicted += lists[at].insert_used(used, key, run).unwrap_or(1);
            }
            evicted += 1;
        }
        self.dropped_runs(evicted);
    }

    pub(super) fn run_stats(&self) -> (usize, usize) {
        let Some(shards) = self.run_shards.get() else { return (0, 0) };
        let each = shards.lists.iter().map(|list| read_shared(list));
        each.fold((0, 0), |(n, bytes), list| (n + list.len(), bytes + list.bytes()))
    }

    pub(super) fn clear_runs(&self) {
        if let Some(shards) = self.run_shards.get() {
            shards.lists.iter().for_each(|list| write_shared(list).clear());
            self.dropped_runs(1);
        }
    }

    // Each run keeps its list, its marks paid first.
    pub(super) fn set_runs_pool(&self, pool: Option<&Shared<lru::Pool>>) {
        let shards = self.run_shards();
        let me = lru::member(shards);
        let mut lists: Vec<_> = shards.lists.iter().map(|list| write_shared(list)).collect();
        lists.iter().enumerate().for_each(|(at, list)| self.stamp_front(list, at));
        let held = shards.in_use.get().max(1);
        let evicted = lru::rejoin(&mut lists, pool, &me, |key| list_of(self.runs_id, &key.parts().0, held));
        self.dropped_runs(evicted);
    }

    pub(super) fn runs_pool(&self) -> Option<Shared<lru::Pool>> {
        read_shared(&self.run_shards().lists[0]).pool()
    }

    // A run with features is left to the cache, so no slot has any, and a hit compares the rest.
    fn front_run(&self, parts: &RunParts<'_>) -> Option<Arc<Run>> {
        let head = &parts.0;
        let dropped = self.runs_dropped.get();
        let read = FRONT.try_with(|front| {
            let mut front = front.try_borrow_mut().ok()?;
            let slot = front[front_slot(self.runs_id, head)].as_mut()?;
            let held = slot.cache == self.runs_id && slot.dropped == dropped && slot.key.parts().0 == *head;
            if !held || !parts.1.is_empty() {
                return None;
            }
            slot.owed = true;
            Some(Arc::clone(&slot.run))
        });
        read.ok().flatten()
    }

    fn stamp_front(&self, cache: &RunCache, at: usize) {
        stamp_front(self.runs_id, cache, at);
    }

    // Puts a run list `at` holds in this thread's front, with the list's lock held so the count of drops is the run's
    // own. A slot owing another cache's stamp is left: its cache's lock isn't held here to pay it.
    fn remember_run(&self, key: &Shared<ShapeKey>, parts: &RunParts<'_>, run: &Arc<Run>, at: usize) {
        if !parts.1.is_empty() {
            return;
        }
        let dropped = self.runs_dropped.get();
        let _ = FRONT.try_with(|front| {
            let Ok(mut front) = front.try_borrow_mut() else { return };
            let slot = &mut front[front_slot(self.runs_id, &parts.0)];
            if slot.as_ref().is_some_and(|s| s.cache != self.runs_id && s.owed) {
                return;
            }
            let (key, run) = (Shared::clone(key), Arc::clone(run));
            *slot = Some(FrontRun { cache: self.runs_id, list: at, dropped, key, run, owed: false });
        });
    }

    // Counts runs the cache dropped, with its lock held, so no front serves them again, and lets go of this thread's
    // slots of the cache, which then can't be read: other threads let go of theirs as runs land on them.
    pub(super) fn dropped_runs(&self, dropped: usize) {
        if dropped == 0 {
            return;
        }
        self.runs_dropped.add(dropped);
        forget_front(self.runs_id);
    }
}

// Without `threading` the cache's lock is a cell no other thread takes, so there's no front.
#[cfg(not(feature = "threading"))]
impl FontCache {
    fn front_run(&self, _: &RunParts<'_>) -> Option<Arc<Run>> {
        None
    }

    fn run_list(&self, _: &RunHead<'_>) -> Option<(&Shareable<RunCache>, usize)> {
        Some((&**self.shape_list(), 0))
    }

    fn stamp_front(&self, _: &RunCache, _: usize) {}

    fn remember_run(&self, _: &Shared<ShapeKey>, _: &RunParts<'_>, _: &Arc<Run>, _: usize) {}

    pub(super) fn dropped_runs(&self, _: usize) {}
}

// A run's key as its parts, borrowed: the cache keeps owned keys and is asked with borrowed ones, ordered alike, a
// string as its bytes.
type RunHead<'a> = (&'a AxisKey, &'a [u8], bool, Option<bool>, &'a str, &'a str, Option<&'a str>, Option<u16>);
type RunParts<'a> = (RunHead<'a>, &'a [Feature]);

trait RunKey {
    fn parts(&self) -> RunParts<'_>;
}

impl RunKey for ShapeKey {
    fn parts(&self) -> RunParts<'_> {
        let (axes, text, vertical, rtl, before, after, language, seed, features) = self;
        ((axes, text.as_bytes(), *vertical, *rtl, before, after, language.as_deref(), *seed), features)
    }
}

impl RunKey for RunParts<'_> {
    fn parts(&self) -> RunParts<'_> {
        *self
    }
}

impl<'a> core::borrow::Borrow<dyn RunKey + 'a> for Shared<ShapeKey> {
    fn borrow(&self) -> &(dyn RunKey + 'a) {
        &**self
    }
}

impl PartialEq for dyn RunKey + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.parts() == other.parts()
    }
}

impl Eq for dyn RunKey + '_ {}

impl PartialOrd for dyn RunKey + '_ {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for dyn RunKey + '_ {
    // Features on a tie alone and out of line, as a search compares at every node and few runs have any.
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        let (a, b) = (self.parts(), other.parts());
        match a.0.cmp(&b.0) {
            core::cmp::Ordering::Equal if !(a.1.is_empty() && b.1.is_empty()) => features_order(a.1, b.1),
            order => order,
        }
    }
}

#[cold]
#[inline(never)]
fn features_order(a: &[Feature], b: &[Feature]) -> core::cmp::Ordering {
    a.cmp(b)
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/cache/shaping.rs"]
mod tests;
