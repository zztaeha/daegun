// What a plan asks for, as the pipeline and a script's model declare it: features, the stages that split GSUB's
// lookups with a model's pause after each, and the lookups JSTF switches.
use alloc::vec::Vec;

use super::{LookupSwitch, Plan, ScriptChoice, Table};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::ot::LayoutTable;
use crate::daecore::daeshaper::run::Run;

// Run between two GSUB stages; true where it put glyphs in the run that no lookup wrote.
pub(crate) type Pause = fn(&mut Run, &Plan, &Face) -> bool;

// `rand`'s value: an alternate lookup seeing it picks one at random.
pub(crate) const RANDOM_PICK: u32 = 255;

// How a feature is used, combined with `and`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct FeatureUse(u8);

impl FeatureUse {
    // Off until a range or a script's mask turns it on.
    pub(crate) const PLAIN: FeatureUse = FeatureUse(0);
    // On for every glyph by default.
    pub(crate) const GLOBAL: FeatureUse = FeatureUse(1);
    // Kept where no table has it, for the model's own fallback.
    pub(crate) const FALLBACK: FeatureUse = FeatureUse(1 << 1);
    // Context matching stops at a ZWNJ rather than skip it.
    pub(crate) const SEES_ZWNJ: FeatureUse = FeatureUse(1 << 2);
    // Input matching stops at a ZWJ rather than skip it.
    pub(crate) const SEES_ZWJ: FeatureUse = FeatureUse(1 << 3);
    pub(crate) const SEES_JOINERS: FeatureUse = FeatureUse(1 << 2 | 1 << 3);
    // Taken from any language system of the table where the chosen one lacks it.
    pub(crate) const ANY_LANGUAGE: FeatureUse = FeatureUse(1 << 4);
    pub(crate) const RANDOM: FeatureUse = FeatureUse(1 << 5);
    // Its lookups match within one syllable, as a script's model marks them.
    pub(crate) const PER_SYLLABLE: FeatureUse = FeatureUse(1 << 6);

    #[must_use]
    pub(crate) const fn and(self, other: FeatureUse) -> FeatureUse {
        FeatureUse(self.0 | other.0)
    }

    pub(crate) const fn has(self, other: FeatureUse) -> bool {
        self.0 & other.0 == other.0
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Request {
    pub(super) tag: Tag,
    pub(super) most: u32,
    pub(super) initial: u32,
    pub(super) how: FeatureUse,
    pub(super) stage: [u16; 2],
}

impl Request {
    // A later request for the same tag: a global one sets the value, any other keeps the larger; a fallback sticks;
    // the earliest stage wins. How the lookups match stays the first request's.
    pub(super) fn absorb(&mut self, later: &Request) {
        let (global, fallback) = (FeatureUse::GLOBAL.0, FeatureUse::FALLBACK.0);
        self.how = FeatureUse((self.how.0 & !global) | (later.how.0 & (global | fallback)));
        match later.how.has(FeatureUse::GLOBAL) {
            true => (self.most, self.initial) = (later.most, later.initial),
            false => self.most = self.most.max(later.most),
        }
        self.stage = [0, 1].map(|t| self.stage[t].min(later.stage[t]));
    }
}

// A table and the script and language system chosen in it.
#[derive(Clone, Copy)]
pub(super) struct Chosen<'t> {
    pub(super) table: &'t LayoutTable<'t>,
    pub(super) script: u16,
    pub(super) language: Option<u16>,
}

pub(crate) struct Planner<'t> {
    pub(super) chosen: [Option<Chosen<'t>>; 2],
    pub(super) tables: [Option<&'t LayoutTable<'t>>; 2],
    pub(super) scripts: [ScriptChoice; 2],
    pub(super) requests: Vec<Request>,
    pub(super) pauses: [Vec<Option<Pause>>; 2],
    pub(super) switches: Vec<LookupSwitch>,
}

impl<'t> Planner<'t> {
    // Each table's script is the first of `scripts` it has, else its default; its language system the first of
    // `languages` the script has, else the default one (OpenType chapter 2).
    pub(crate) fn new(
        tables: [Option<&'t LayoutTable<'t>>; 2],
        scripts: &[Tag],
        languages: &[Tag],
    ) -> Self {
        let (requests, pauses, switches) = (Vec::new(), Default::default(), Vec::new());
        let mut planner =
            Planner { chosen: [None; 2], tables, scripts: Default::default(), requests, pauses, switches };
        for (t, table) in tables.into_iter().enumerate() {
            let Some((table, (found, index, tag))) = table.and_then(|l| Some((l, l.select_script(scripts)?))) else {
                continue;
            };
            planner.scripts[t] = ScriptChoice { found, tag: Some(tag) };
            let language = table.select_langsys(index, languages);
            planner.chosen[t] = Some(Chosen { table, script: index, language });
        }
        planner
    }

    pub(crate) fn script(&self, table: Table) -> ScriptChoice {
        self.scripts[table as usize]
    }

    // A feature of value 1.
    pub(crate) fn add(&mut self, tag: Tag, how: FeatureUse) {
        self.add_value(tag, how, 1);
    }

    pub(crate) fn add_value(&mut self, tag: Tag, how: FeatureUse, value: u32) {
        let initial = if how.has(FeatureUse::GLOBAL) { value } else { 0 };
        let stage = [0, 1].map(|t| self.pauses[t].len() as u16);
        self.requests.push(Request { tag, most: value, initial, how, stage });
    }

    // On for every glyph.
    pub(crate) fn on(&mut self, tag: Tag, how: FeatureUse) {
        self.add(tag, how.and(FeatureUse::GLOBAL));
    }

    pub(crate) fn off(&mut self, tag: Tag) {
        self.add_value(tag, FeatureUse::GLOBAL, 0);
    }

    // Ends the current GSUB stage: its features' lookups run, then `pause`.
    pub(crate) fn end_stage(&mut self, pause: Option<Pause>) {
        self.pauses[Table::Gsub as usize].push(pause);
    }

    pub(super) fn end_stages(&mut self) {
        self.pauses.iter_mut().for_each(|stages| stages.push(None));
    }

    pub(crate) fn switch(&mut self, switch: LookupSwitch) {
        self.switches.push(switch);
    }

    // Whether either table's chosen language system lists `tag`, its required feature aside.
    pub(crate) fn lists(&self, tag: Tag) -> bool {
        [Table::Gsub, Table::Gpos].into_iter().any(|t| self.find(t, tag).is_some())
    }

    pub(super) fn find(&self, table: Table, tag: Tag) -> Option<u16> {
        let chosen = self.chosen[table as usize]?;
        chosen.table.find_feature(chosen.script, chosen.language, tag)
    }

    pub(super) fn required(&self, table: Table) -> Option<u16> {
        let chosen = self.chosen[table as usize]?;
        chosen.table.required_feature(chosen.script, chosen.language)
    }
}
