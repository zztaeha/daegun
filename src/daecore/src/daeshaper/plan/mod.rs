// What shaping a run reads of a font for a script, direction, language and set of features, built once and cached:
// the model, the features' mask bits, each table's lookup stages and the tables that run.
mod compile;
mod features;
mod models;

use alloc::vec::Vec;

use super::face::Face;
use super::ot::morx_flags::RangeFlags;
use super::ot::tag::Tag;
use super::ot::{aat::SubtableSets, gpos, gsub, reverse_of, spend_work, LayoutTable, LookupIndex};
use super::run::{Direction, Mask, Run};
use super::script::{self, Model, ModelData};
use super::unicode::Script;
use crate::daecore::daekit::base::sync::Shared;
use crate::daecore::daekit::cache::shaping::IndexEntries;
use crate::daecore::daetype::format::sanitize::AatTable;
pub(crate) use compile::{Features, Schedule, ScriptChoice, StageLookup};
pub(crate) use features::{FeatureUse, Pause, Planner, RANDOM_PICK};
pub(crate) use models::{Positioning, Substitution};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Table {
    Gsub = 0,
    Gpos = 1,
}

// A feature's value over the clusters from `start` to `end`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FeatureSetting {
    pub(crate) tag: Tag,
    pub(crate) start: u32,
    pub(crate) end: u32,
    pub(crate) value: u32,
}

impl FeatureSetting {
    // Over all the text, so a global feature rather than one a range's masks turn on.
    pub(crate) fn covers_all(&self) -> bool {
        (self.start, self.end) == (0, u32::MAX)
    }
}

// A lookup JSTF turns on or off.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct LookupSwitch {
    pub(crate) table: Table,
    pub(crate) lookup: u16,
    pub(crate) on: bool,
}

// Everything a plan is keyed on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlanRequest<'a> {
    pub(crate) direction: Direction,
    pub(crate) features: &'a [FeatureSetting],
    pub(crate) switches: &'a [LookupSwitch],
    pub(crate) script: Option<Script>,
    // The OpenType script tags to look for, best first.
    pub(crate) script_tags: &'a [Tag],
    // The language's canonical tag, which morx's features read, and the language system tags it gives.
    pub(crate) language: Option<&'a str>,
    pub(crate) language_tags: &'a [Tag],
}

// The masks the pipeline sets itself.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CoreMasks {
    pub(crate) frac: Mask,
    pub(crate) numr: Mask,
    pub(crate) dnom: Mask,
    pub(crate) rtlm: Mask,
    pub(crate) kern: Mask,
}

// The AAT sets a plan runs, kept from the font cache so a shape reads them without asking it.
#[derive(Default)]
pub(crate) struct AatPlan {
    pub(crate) morx_flags: Vec<Vec<RangeFlags>>,
    pub(crate) morx_sets: Vec<Option<Shared<SubtableSets>>>,
    pub(crate) kerx_sets: Vec<Option<Shared<SubtableSets>>>,
    pub(crate) kern_sets: Vec<Option<Shared<SubtableSets>>>,
}

pub(crate) struct Plan {
    pub(crate) model: &'static dyn Model,
    pub(crate) direction: Direction,
    pub(crate) features: Features,
    pub(crate) gsub: Schedule,
    pub(crate) gpos: Schedule,
    pub(crate) masks: CoreMasks,
    pub(crate) has_fractions: bool,
    pub(crate) has_vert: bool,
    // A mark feature is found, so composing a mark away would lose what positions it.
    pub(crate) marks_positioned: bool,
    // The font has no GDEF classes, so glyphs take theirs from their characters.
    pub(crate) synthesized_classes: bool,
    pub(crate) substitution: Substitution,
    pub(crate) positioning: Positioning,
    data: Option<ModelData>,
    pub(crate) aat: AatPlan,
}

impl Plan {
    // Its own lists and its model's data as it lies; the lookup indexes and AAT sets it shares are the font's, and what
    // a model draws from the index allowance is counted there.
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        let AatPlan { morx_flags, morx_sets, kerx_sets, kern_sets } = &self.aat;
        self.features.heap_bytes() + self.gsub.heap_bytes() + self.gpos.heap_bytes()
            + self.data.as_deref().map_or(0, core::mem::size_of_val)
            + heap_of(morx_flags) + morx_flags.iter().map(heap_of).sum::<usize>()
            + heap_of(morx_sets) + heap_of(kerx_sets) + heap_of(kern_sets)
    }

    pub(crate) fn new(face: &Face, request: &PlanRequest) -> Plan {
        let PlanRequest { direction, features, switches, script, script_tags, language_tags, .. } = *request;
        let [gsub, gpos] = [Table::Gsub, Table::Gpos].map(|table| face.layout(table));
        let morx = face.aat(AatTable::Morx);
        // morx shapes where it has data, unless vertical text has GSUB. mort isn't read, so a mort font shapes
        // through GSUB.
        let substitution = match morx.is_some() && !(direction.is_vertical() && gsub.is_some()) {
            true => Substitution::Morx,
            false => Substitution::Gsub,
        };
        let first_tag = script_tags.first().copied();

        let mut planner = Planner::new([gsub.as_ref(), gpos.as_ref()], script_tags, language_tags);
        let gsub_script = planner.script(Table::Gsub).tag;
        let script_model = script::select(script, direction.is_horizontal(), gsub_script, first_tag);
        // morx does what a script's model would, so only the general model keeps its own.
        let model = match substitution == Substitution::Morx && script_model.kind() != script::Shaper::Default {
            true => &script::MORX,
            false => script_model,
        };
        request_features(&mut planner, direction, features, model, first_tag);
        switches.iter().for_each(|&s| planner.switch(s));
        let (compiled, [mut gsub_schedule, mut gpos_schedule]) = planner.compile(&face.layout_variation());
        gsub_schedule.indexes = index_lookups(face, gsub.as_ref(), &gsub_schedule, Table::Gsub);
        gpos_schedule.indexes = index_lookups(face, gpos.as_ref(), &gpos_schedule, Table::Gpos);

        let tag = |name: &[u8; 4]| Tag::new(name);
        let masks = CoreMasks {
            frac: compiled.on_bit(tag(b"frac")),
            numr: compiled.on_bit(tag(b"numr")),
            dnom: compiled.on_bit(tag(b"dnom")),
            rtlm: compiled.on_bit(tag(b"rtlm")),
            kern: compiled.mask(tag(if direction.is_horizontal() { b"kern" } else { b"vkrn" })),
        };
        let inputs = models::Inputs {
            face,
            features: &compiled,
            horizontal: direction.is_horizontal(),
            has_gsub: gsub.is_some(),
            has_gpos: gpos.is_some(),
            substitution,
            model,
            script_model,
        };
        let positioning = models::positioning(&inputs);
        let data = model.plan_data(&compiled, face, script, first_tag);
        let aat = aat_plan(face, morx.filter(|_| substitution == Substitution::Morx), &positioning, request);

        Plan {
            model,
            direction,
            has_fractions: masks.frac != 0 || (masks.numr != 0 && masks.dnom != 0),
            has_vert: compiled.on_bit(tag(b"vert")) != 0,
            marks_positioned: compiled.on_bit(tag(b"mark")) != 0,
            synthesized_classes: !face.gives_classes(),
            features: compiled,
            gsub: gsub_schedule,
            gpos: gpos_schedule,
            masks,
            substitution,
            positioning,
            data,
            aat,
        }
    }

    pub(crate) fn schedule(&self, table: Table) -> &Schedule {
        match table {
            Table::Gsub => &self.gsub,
            Table::Gpos => &self.gpos,
        }
    }

    // The model's own data, built with the plan.
    pub(crate) fn data<T: 'static>(&self) -> Option<&T> {
        let data: &dyn core::any::Any = self.data.as_deref()?;
        data.downcast_ref()
    }

    pub(crate) fn probe<'a>(&'a self, face: &Face<'a>) -> Probe<'a> {
        let gsub = face.layout(Table::Gsub);
        Probe { gsub, plan: self }
    }
}

// The features every run takes, in the order their stages and merging depend on, the model's between daegun's two
// hooks, which let a font run lookups before and after the script's own.
fn request_features(
    p: &mut Planner,
    direction: Direction,
    settings: &[FeatureSetting],
    model: &dyn Model,
    script_tag: Option<Tag>,
) {
    use FeatureUse as U;
    let tag = |name: &[u8; 4]| Tag::new(name);
    // Variation alternates come first, before localized forms and any script's shaping.
    p.on(tag(b"rvrn"), U::PLAIN);
    p.end_stage(None);
    match direction {
        Direction::LeftToRight => [b"ltra", b"ltrm"].into_iter().for_each(|t| p.on(tag(t), U::PLAIN)),
        Direction::RightToLeft => {
            p.on(tag(b"rtla"), U::PLAIN);
            p.add(tag(b"rtlm"), U::PLAIN);
        }
        _ => {}
    }
    [b"frac", b"numr", b"dnom"].into_iter().for_each(|t| p.add(tag(t), U::PLAIN));
    p.add_value(tag(b"rand"), U::GLOBAL.and(U::RANDOM), RANDOM_PICK);
    // A mixed-case tag reads to font tools as required, an all-capital one as discretionary.
    [b"Dae1", b"DAE1"].into_iter().for_each(|t| p.on(tag(t), U::PLAIN));
    model.plan_features(p, script_tag);
    [b"Dae2", b"DAE2"].into_iter().for_each(|t| p.on(tag(t), U::PLAIN));
    for name in [b"abvm", b"blwm", b"ccmp", b"locl", b"mark", b"mkmk", b"rlig"] {
        let sees = if matches!(name, b"mark" | b"mkmk") { U::SEES_JOINERS } else { U::PLAIN };
        p.on(tag(name), sees);
    }
    if direction.is_horizontal() {
        for name in [b"calt", b"clig", b"curs", b"dist", b"kern", b"liga", b"rclt"] {
            p.on(tag(name), if name == b"kern" { U::FALLBACK } else { U::PLAIN });
        }
    } else {
        p.on(tag(b"vert"), U::ANY_LANGUAGE);
    }
    // The caller's, a feature over all the text global and one over a range off but where its masks turn it on.
    for s in settings {
        p.add_value(s.tag, if s.covers_all() { U::GLOBAL } else { U::PLAIN }, s.value);
    }
    model.plan_late_features(p);
}

// Each scheduled lookup's index, then each lookup their context rules name, from the font cache, so its allowance
// goes to the lookups the plan runs first.
fn index_lookups(
    face: &Face,
    table: Option<&LayoutTable>,
    schedule: &Schedule,
    which: Table,
) -> Vec<Option<Shared<LookupIndex>>> {
    let Some(table) = table else { return Vec::new() };
    let count = usize::from(table.lookup_count());
    let kinds = match which {
        Table::Gsub => &gsub::KINDS,
        Table::Gpos => &gpos::KINDS,
    };
    let mut indexes = alloc::vec![None; count];
    // Telling a lookup's type reads every subtable, and costs the plan that whether the font keeps its index or not,
    // so which lookups apply doesn't hang on what other plans indexed first. Indexing them spends work of its own.
    let mut work = super::ot::index_work(table.bytes().len());
    let mut reading = work;
    let mut pending: Vec<u16> = schedule.lookups().iter().map(|l| l.index).collect();
    let mut next = 0;
    while let Some(&lookup) = pending.get(next) {
        next += 1;
        let slot = usize::from(lookup);
        if slot >= count || indexes[slot].is_some() {
            continue;
        }
        // A lookup that applies nothing gets an empty index, whose digest turns every glyph away: one past the plan's
        // reading, this plan's alone and so not kept, one the table doesn't hold, or one `reverse_of` refuses.
        let found = table.lookup(lookup);
        if found.as_ref().is_some_and(|found| !spend_work(&mut reading, found.subtable_count.into())) {
            indexes[slot] = Some(Shared::new(LookupIndex::default()));
            continue;
        }
        let built = face.lookup_index(which as usize, count, lookup, |shared| {
            let Some(found) = found else { return LookupIndex::default() };
            let Some(reverse) = reverse_of(&found, kinds) else { return LookupIndex::default() };
            // Each subtable's entry is drawn from the allowance with the rest, and past it the lookup goes unindexed.
            let entries = usize::from(found.subtable_count) * size_of::<super::ot::SubtableIndex>();
            if !face.reserve_index_bytes(entries) {
                return super::ot::unindexed(reverse);
            }
            let build = |entries: &IndexEntries, absent| face.build_index(entries, absent);
            let share = |data: &[u8], class| face.shared_index(shared, found.start_of(data)?, data, class);
            let reserve = |bytes| face.reserve_index_bytes(bytes);
            super::ot::lookup_index(&found, kinds, reverse, build, share, reserve, &mut work)
        });
        pending.extend_from_slice(&built.nested);
        indexes[slot] = Some(built);
    }
    indexes
}

fn aat_plan(face: &Face, morx: Option<&[u8]>, positioning: &Positioning, request: &PlanRequest) -> AatPlan {
    let horizontal = request.direction.is_horizontal();
    let (morx_flags, morx_sets) = match morx {
        Some(morx) => {
            let flags = super::ot::morx_flags::chain_flags(face, morx, request.features, request.language);
            let sets = super::ot::morx::plan_sets(face, morx, &flags);
            (flags, sets)
        }
        None => (Vec::new(), Vec::new()),
    };
    AatPlan {
        morx_flags,
        morx_sets,
        kerx_sets: if positioning.kerx { super::ot::kerx::kerx_sets(face, horizontal) } else { Vec::new() },
        kern_sets: if positioning.kern { super::ot::kern::plan_sets(face, horizontal) } else { Vec::new() },
    }
}

// Whether a GSUB stage's lookups would substitute a glyph sequence, as a script's model asks while reordering.
pub(crate) struct Probe<'a> {
    plan: &'a Plan,
    gsub: Option<LayoutTable<'a>>,
}

impl Probe<'_> {
    // Lookups whose digest rules out the first glyph are passed over; the rest are tried by their coverage. `run`
    // pays for each lookup, and past its edits nothing substitutes.
    pub(crate) fn would_substitute(&self, run: &mut Run, stage: u16, glyphs: &[u32], zero_context: bool) -> bool {
        let Some(gsub) = &self.gsub else { return false };
        let first = glyphs.first().copied().unwrap_or(0);
        let indexes = &self.plan.gsub.indexes;
        let may_start = |l: &&StageLookup| {
            let index = indexes.get(usize::from(l.index)).and_then(Option::as_deref);
            index.is_none_or(|i| i.digest.may_have_folded(first))
        };
        let lookups = self.plan.gsub.stage(stage);
        run.spend(lookups.len())
            && lookups.iter().filter(may_start).any(|l| gsub::would_apply(gsub, l.index, glyphs, zero_context, run))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/plan/mod.rs"]
mod tests;
