// A plan's features turned into mask bits and lookup stages. Within a stage a table's lookups run in LookupList
// order whatever features named them (OpenType chapter 2).
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cell::Cell;

use super::features::{FeatureUse, Pause, Planner, Request};
use super::Table;
use crate::daecore::daekit::base::sync::Shared;
use crate::daecore::daeshaper::face::LayoutVariation;
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::ot::{LayoutTable, LookupIndex};
use crate::daecore::daeshaper::run::{flag, Mask};

// An on/off feature on for every glyph shares this bit, which every glyph starts with.
const SHARED_SHIFT: u32 = 31;
const SHARED_BIT: Mask = 1 << SHARED_SHIFT;
// A feature's value takes at most this many bits.
const MOST_VALUE_BITS: u32 = 8;
// How the required feature's lookups and those JSTF switches on run: on every glyph, joiners passed over.
const ALWAYS: StageLookup =
    StageLookup { index: 0, mask: SHARED_BIT, skips_zwnj: true, skips_zwj: true, random: false, per_syllable: false };

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct ScriptChoice {
    // Whether the table has one of the run's own script tags, rather than a default script.
    pub(crate) found: bool,
    pub(crate) tag: Option<Tag>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct FeatureSlot {
    tag: Tag,
    mask: Mask,
    shift: u32,
    index: [Option<u16>; 2],
    stage: [u16; 2],
    how: FeatureUse,
}

impl FeatureSlot {
    fn lookup_use(&self) -> StageLookup {
        StageLookup {
            index: 0,
            mask: self.mask,
            skips_zwnj: !self.how.has(FeatureUse::SEES_ZWNJ),
            skips_zwj: !self.how.has(FeatureUse::SEES_ZWJ),
            random: self.how.has(FeatureUse::RANDOM),
            per_syllable: self.how.has(FeatureUse::PER_SYLLABLE),
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct Features {
    // By tag.
    slots: Vec<FeatureSlot>,
    // By tag, the features kept for a fallback that no table has, asked on or off: a model's fallback rests on the
    // font, so a feature off everywhere weighs as one off over every glyph.
    tableless: Vec<Tag>,
    global: Mask,
    scripts: [ScriptChoice; 2],
}

impl Features {
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        heap_of(&self.slots) + heap_of(&self.tableless)
    }

    fn slot(&self, tag: Tag) -> Option<&FeatureSlot> {
        self.slots.binary_search_by_key(&tag, |s| s.tag).ok().map(|i| &self.slots[i])
    }

    pub(crate) fn mask(&self, tag: Tag) -> Mask {
        self.slot(tag).map_or(0, |s| s.mask)
    }

    pub(crate) fn shift(&self, tag: Tag) -> u32 {
        self.slot(tag).map_or(0, |s| s.shift)
    }

    // The feature's value 1 as a mask.
    pub(crate) fn on_bit(&self, tag: Tag) -> Mask {
        self.slot(tag).map_or(0, |s| s.mask & 1 << s.shift)
    }

    // Every glyph's mask before ranges and models set theirs.
    pub(crate) fn global(&self) -> Mask {
        self.global
    }

    pub(crate) fn lacks_table(&self, tag: Tag) -> bool {
        self.tableless.binary_search(&tag).is_ok()
    }

    pub(crate) fn index(&self, table: Table, tag: Tag) -> Option<u16> {
        self.slot(tag)?.index[table as usize]
    }

    pub(crate) fn stage(&self, table: Table, tag: Tag) -> Option<u16> {
        self.slot(tag).map(|s| s.stage[table as usize])
    }

    pub(crate) fn script(&self, table: Table) -> ScriptChoice {
        self.scripts[table as usize]
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct StageLookup {
    pub(crate) index: u16,
    pub(crate) mask: Mask,
    // Context matching passes over ZWNJ.
    pub(crate) skips_zwnj: bool,
    // Input matching passes over ZWJ.
    pub(crate) skips_zwj: bool,
    pub(crate) random: bool,
    pub(crate) per_syllable: bool,
}

#[derive(Default)]
pub(crate) struct Schedule {
    lookups: Vec<StageLookup>,
    // Each stage's end in `lookups`, and what runs after it.
    stages: Vec<(usize, Option<Pause>)>,
    // What shaping reads of each lookup, by lookup index; built with the plan.
    pub(crate) indexes: Vec<Option<Shared<LookupIndex>>>,
}

impl Schedule {
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        heap_of(&self.lookups) + heap_of(&self.stages) + heap_of(&self.indexes)
    }

    pub(crate) fn stages(&self) -> impl Iterator<Item = (&[StageLookup], Option<Pause>)> {
        let starts = core::iter::once(0).chain(self.stages.iter().map(|&(end, _)| end));
        starts.zip(&self.stages).map(|(start, &(end, pause))| (&self.lookups[start..end], pause))
    }

    pub(crate) fn stage(&self, n: u16) -> &[StageLookup] {
        self.stages().nth(usize::from(n)).map_or(&[], |(lookups, _)| lookups)
    }

    pub(crate) fn lookups(&self) -> &[StageLookup] {
        &self.lookups
    }
}

impl Planner<'_> {
    pub(crate) fn compile(mut self, variation: &LayoutVariation) -> (Features, [Schedule; 2]) {
        let requests = settle(core::mem::take(&mut self.requests));
        let (features, required_stage) = self.assign_bits(&requests);
        self.end_stages();
        let schedules = [Table::Gsub, Table::Gpos].map(|t| self.gather(t, &features, required_stage, variation));
        (features, schedules)
    }

    // Bits for each feature found in a table or kept for a fallback, the stage of the table's required feature.
    fn assign_bits(&self, requests: &[Request]) -> (Features, [u16; 2]) {
        let required = [Table::Gsub, Table::Gpos].map(|t| self.required(t));
        let mut required_stage = [0; 2];
        let mut features = Features { global: SHARED_BIT, scripts: self.scripts, ..Features::default() };
        let mut next = flag::ALL.count_ones();
        for request in requests {
            let shared = request.how.has(FeatureUse::GLOBAL) && request.most == 1;
            let width = if shared { 0 } else { (u32::BITS - request.most.leading_zeros()).min(MOST_VALUE_BITS) };
            let planned = request.most > 0 && next + width < SHARED_SHIFT;
            let fallback = request.how.has(FeatureUse::FALLBACK);
            if !planned && !fallback {
                continue;
            }
            let mut index = [Table::Gsub, Table::Gpos].map(|t| self.find(t, request.tag));
            if index == [None; 2] && request.how.has(FeatureUse::ANY_LANGUAGE) {
                index = self.tables.map(|t| t.and_then(|t| t.find_feature_globally(request.tag)));
            }
            let found = index != [None; 2];
            if !found && fallback {
                features.tableless.push(request.tag);
            }
            if !planned {
                continue;
            }
            for t in [Table::Gsub, Table::Gpos] {
                let named = required[t as usize].and_then(|f| self.tables[t as usize]?.feature_tag(f));
                if named == Some(request.tag) {
                    required_stage[t as usize] = request.stage[t as usize];
                }
            }
            if !found && !fallback {
                continue;
            }
            let (shift, mask) = match shared {
                true => (SHARED_SHIFT, SHARED_BIT),
                false => {
                    let mask = ((1 << width) - 1) << next;
                    features.global |= (request.initial << next) & mask;
                    next += width;
                    (next - width, mask)
                }
            };
            let (tag, stage, how) = (request.tag, request.stage, request.how);
            features.slots.push(FeatureSlot { tag, mask, shift, index, stage, how });
        }
        (features, required_stage)
    }

    // A table's lookups stage by stage, each stage's in index order with duplicates merged, then JSTF's switches.
    fn gather(&self, table: Table, features: &Features, required_at: [u16; 2], varied: &LayoutVariation) -> Schedule {
        let t = table as usize;
        let mut schedule = Schedule::default();
        let required = self.required(table);
        let left = Cell::new(self.tables[t].map_or(0, LayoutTable::variation_reads));
        let varied = Varied { record: varied.records[t], conditions: &varied.conditions[t], left };
        let mut stage = Vec::new();
        for (n, &pause) in self.pauses[t].iter().enumerate() {
            if let Some(layout) = self.tables[t] {
                if let Some(feature) = required.filter(|_| usize::from(required_at[t]) == n) {
                    varied.push_lookups(layout, feature, ALWAYS, &mut stage);
                }
                for slot in features.slots.iter().filter(|s| usize::from(s.stage[t]) == n) {
                    if let Some(feature) = slot.index[t] {
                        varied.push_lookups(layout, feature, slot.lookup_use(), &mut stage);
                    }
                }
                stage.sort_by_key(|l: &StageLookup| l.index);
                stage.dedup_by(|later, kept| {
                    let same = later.index == kept.index;
                    if same {
                        kept.mask |= later.mask;
                        kept.skips_zwnj &= later.skips_zwnj;
                        kept.skips_zwj &= later.skips_zwj;
                    }
                    same
                });
                schedule.lookups.append(&mut stage);
            }
            schedule.stages.push((schedule.lookups.len(), pause));
        }
        if let Some(layout) = self.tables[t] {
            self.apply_switches(table, layout, &mut schedule);
        }
        schedule
    }

    // A lookup switched off leaves every stage; one switched on and not yet there joins the last, on everywhere.
    fn apply_switches(&self, table: Table, layout: &LayoutTable, schedule: &mut Schedule) {
        let mut wanted = BTreeMap::new();
        for s in self.switches.iter().filter(|s| s.table == table) {
            *wanted.entry(s.lookup).or_insert(false) |= s.on;
        }
        if wanted.is_empty() {
            return;
        }
        if wanted.values().any(|&on| !on) {
            let mut kept = Vec::with_capacity(schedule.lookups.len());
            let mut start = 0;
            for (end, _) in &mut schedule.stages {
                let stage = &schedule.lookups[start..*end];
                kept.extend(stage.iter().filter(|l| wanted.get(&l.index) != Some(&false)));
                start = *end;
                *end = kept.len();
            }
            schedule.lookups = kept;
        }
        let count = layout.lookup_count();
        let present: alloc::collections::BTreeSet<u16> = schedule.lookups.iter().map(|l| l.index).collect();
        let before = schedule.lookups.len();
        for (&index, _) in wanted.iter().filter(|&(index, &on)| on && *index < count && !present.contains(index)) {
            schedule.lookups.push(StageLookup { index, ..ALWAYS });
        }
        if schedule.lookups.len() > before
            && let Some(last) = schedule.stages.last_mut()
        {
            last.0 = schedule.lookups.len();
        }
    }
}

// Requests by tag, each tag's merged into its first (stable, so later requests stay later).
fn settle(mut requests: Vec<Request>) -> Vec<Request> {
    requests.sort_by_key(|r| r.tag);
    requests.dedup_by(|later, kept| {
        let same = later.tag == kept.tag;
        if same {
            kept.absorb(later);
        }
        same
    });
    requests
}

// The location's FeatureVariations record for a table and its lookup conditions, and the reads its features' lookup
// variations share, as features may all name one table.
struct Varied<'v> {
    record: Option<u32>,
    conditions: &'v [u32],
    left: Cell<usize>,
}

impl Varied<'_> {
    // A feature's lookups as the variation gives them: an alternate feature table where its first matching record
    // substitutes one (OpenType chapter 2), and lookups its conditions add.
    fn push_lookups(&self, table: &LayoutTable, feature: u16, use_: StageLookup, out: &mut Vec<StageLookup>) {
        let substitute = self.record.and_then(|r| table.variation_substitute(r, feature));
        let count = table.lookup_count();
        let mut push = |index: u16| {
            if index < count {
                out.push(StageLookup { index, ..use_ });
            }
        };
        let mut left = self.left.get();
        let varied = table.varied_lookups(feature, self.conditions, substitute, &mut left);
        self.left.set(left);
        match varied {
            Some(lookups) => lookups.into_iter().for_each(&mut push),
            None => table.feature_lookups(feature, substitute).for_each(push),
        }
    }
}
