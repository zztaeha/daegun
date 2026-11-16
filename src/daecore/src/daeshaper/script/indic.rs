// The nine scripts of Microsoft's Indic script development specs, Devanagari to Malayalam, through the specs' shaping
// engine: syllables, consonant forms read from the font, reordering, basic forms, final reordering, presentation forms.
use alloc::boxed::Box;
use core::sync::atomic::{AtomicU8, Ordering};

use super::indic_category::{self, category as cat, position as at};
use super::syllabic::{self, Circle, Step, Syllable};
use super::syllable;
use super::{MarkAdvances, Model, ModelData, Normalization, Shaper};
use crate::daecore::daekit::cache::shaping::Drawn;
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::generated::syllable_tables::{indic_accept, IndicSyllable, INDIC_TRANSITIONS};
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::plan::{FeatureUse as U, Features, Plan, Planner, Probe, Table};
use crate::daecore::daeshaper::run::{class, Glyph, Mask, Run};
use crate::daecore::daeshaper::unicode::{self, GeneralCategory, Script};

pub(crate) struct Indic;

pub(crate) const MODEL: Indic = Indic;

const PER_SYLLABLE: U = U::GLOBAL.and(U::PER_SYLLABLE);
// Every feature from the basic forms on sees joiners and stays within a syllable; those a syllable's position turns
// on are off until then.
const ON: U = U::GLOBAL.and(U::SEES_JOINERS).and(U::PER_SYLLABLE);
const MASKED: U = U::SEES_JOINERS.and(U::PER_SYLLABLE);

const PLAN: &[Step] = &[
    Step::End(Some(find_syllables)),
    Step::Together(&[(b"locl", PER_SYLLABLE), (b"ccmp", PER_SYLLABLE)]),
    Step::End(Some(reorder_initially)),
    Step::OneByOne(&[
        (b"nukt", ON),
        (b"akhn", ON),
        (b"rphf", MASKED),
        (b"rkrf", ON),
        (b"pref", MASKED),
        (b"blwf", MASKED),
        (b"abvf", MASKED),
        (b"half", MASKED),
        (b"pstf", MASKED),
        (b"vatu", ON),
        (b"cjct", ON),
    ]),
    Step::End(Some(reorder_finally)),
    Step::Together(&[
        (b"init", MASKED),
        (b"pres", ON),
        (b"abvs", ON),
        (b"blws", ON),
        (b"psts", ON),
        (b"haln", ON),
    ]),
];

const CIRCLE: Circle = Circle {
    broken: IndicSyllable::BrokenCluster as u8,
    class: cat::DOTTED_CIRCLE,
    position: Some(at::LAST),
    after: Some(cat::REPHA),
};

// What stands as a consonant: consonants, independent vowels and placeholders.
const CONSONANTS: [u8; 7] = [cat::C, cat::STACKER, cat::RA, cat::MEDIAL, cat::V, cat::PLACEHOLDER, cat::DOTTED_CIRCLE];
const JOINERS: [u8; 2] = [cat::ZWJ, cat::ZWNJ];
// Read by class alone: a matra counts as one even ligated.
const MATRAS: [u8; 2] = [cat::M, cat::M_POST];
const MATRAS_OR_VIRAMA: [u8; 3] = [cat::M, cat::M_POST, cat::H];

// How a syllable opening with RA and a virama shows the RA as a reph.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reph {
    // RA, virama: the reph forms where rphf substitutes the pair.
    Implicit,
    // RA, virama, ZWJ (Telugu).
    Explicit,
    // A character of its own, Malayalam's dot reph.
    Logical,
}

// Where the final reordering puts the reph, by each spec's reph position: after the main consonant, before or after
// the below-base forms, before or after the post-base forms.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RephGoes {
    AfterMain,
    BeforeBelow,
    AfterBelow,
    BeforePost,
    AfterPost,
}

// A script's rules, from its spec.
#[derive(Clone, Copy)]
struct Rules {
    virama: u32,
    reph: Reph,
    reph_goes: RephGoes,
    // The spec has a first version, for fonts with no GSUB script tag of the second.
    first_spec: bool,
    // Below-base forms come before the base too, not only after it.
    below_before_base: bool,
    half_forms: bool,
    // Kannada: RA, virama, ZWJ opening a syllable reads as RA, ZWJ, virama.
    ra_zwj_virama: bool,
    // Kannada: a virama after the base doesn't move past another virama.
    one_virama: bool,
    // Devanagari's first version: RA and virama before the base take the below-base form, its eyelash RA aside.
    eyelash_ra: bool,
    // Malayalam: a below-base consonant past the base with no form of its own stands as a base.
    unformed_below_is_base: bool,
}

const SECOND_SPEC: Rules = Rules {
    virama: 0,
    reph: Reph::Implicit,
    reph_goes: RephGoes::BeforePost,
    first_spec: false,
    below_before_base: true,
    half_forms: true,
    ra_zwj_virama: false,
    one_virama: false,
    eyelash_ra: false,
    unformed_below_is_base: false,
};

// Each script's virama, reph and below-base rules, as its spec gives them.
fn rules_of(script: Option<Script>) -> Rules {
    let spec = |virama, reph, reph_goes, below_before_base| Rules {
        virama,
        reph,
        reph_goes,
        below_before_base,
        first_spec: true,
        ..SECOND_SPEC
    };
    use Reph::{Explicit, Implicit, Logical};
    use RephGoes::{AfterBelow, AfterMain, AfterPost, BeforeBelow, BeforePost};
    match script {
        Some(Script::DEVANAGARI) => Rules { eyelash_ra: true, ..spec(0x094D, Implicit, BeforePost, true) },
        Some(Script::BENGALI) => spec(0x09CD, Implicit, AfterBelow, true),
        Some(Script::GURMUKHI) => spec(0x0A4D, Implicit, BeforeBelow, true),
        Some(Script::GUJARATI) => spec(0x0ACD, Implicit, BeforePost, true),
        Some(Script::ORIYA) => spec(0x0B4D, Implicit, AfterMain, true),
        Some(Script::TAMIL) => Rules { half_forms: false, ..spec(0x0BCD, Implicit, AfterPost, true) },
        Some(Script::TELUGU) => spec(0x0C4D, Explicit, AfterPost, false),
        Some(Script::KANNADA) => {
            Rules { ra_zwj_virama: true, one_virama: true, ..spec(0x0CCD, Implicit, AfterPost, false) }
        }
        Some(Script::MALAYALAM) => Rules {
            half_forms: false,
            unformed_below_is_base: true,
            ..spec(0x0D4D, Logical, AfterMain, true)
        },
        _ => SECOND_SPEC,
    }
}

struct Masks {
    rphf: Mask,
    pref: Mask,
    blwf: Mask,
    abvf: Mask,
    half: Mask,
    pstf: Mask,
    init: Mask,
}

// The GSUB stages whose lookups say what forms a consonant takes.
struct Stages {
    rphf: Option<u16>,
    pref: Option<u16>,
    blwf: Option<u16>,
    pstf: Option<u16>,
    vatu: Option<u16>,
}

// Each glyph's consonant position as the font's forms place it, kept once asked: 0 for not yet, else the position
// plus one. Threads racing on a slot store the one answer the font gives, so relaxed order is enough.
struct Positions {
    slots: Box<[AtomicU8]>,
    // The font's allowance the slots are drawn from, given back with the plan.
    _drawn: Option<Drawn>,
}

impl Positions {
    // A slot for every glyph where the font has a virama to ask with and its allowance holds them.
    fn for_font(face: &Face, asked: bool) -> Positions {
        let count = face.glyph_count() as usize;
        let drawn = if asked { face.draw_index_bytes(count) } else { None };
        let slots = match drawn {
            Some(_) => core::iter::repeat_with(AtomicU8::default).take(count).collect(),
            None => Box::default(),
        };
        Positions { slots, _drawn: drawn }
    }
}

struct Data {
    rules: Rules,
    // The font follows the spec's first version.
    first_spec: bool,
    // A second-version font's basic forms match with nothing around them; Malayalam's of either version read it.
    bare: bool,
    masks: Masks,
    stages: Stages,
    virama: Option<u32>,
    positions: Positions,
}

impl Model for Indic {
    fn kind(&self) -> Shaper {
        Shaper::Indic
    }

    fn plan_features(&self, planner: &mut Planner<'_>, _: Option<Tag>) {
        syllabic::declare(planner, PLAN);
    }

    // Ligatures belong to the presentation forms; the syllables are forgotten once those ran.
    fn plan_late_features(&self, planner: &mut Planner<'_>) {
        planner.off(Tag::new(b"liga"));
        planner.end_stage(Some(syllable::clear));
    }

    fn plan_data(&self, features: &Features, face: &Face, script: Option<Script>, _: Option<Tag>)
        -> Option<ModelData> {
        let rules = rules_of(script);
        // Only a GSUB script tag of the second version ('dev2' and its kin) asks for it.
        let second_tag = features.script(Table::Gsub).tag.is_some_and(|t| t.to_bytes().ends_with(b"2"));
        let first_spec = rules.first_spec && !second_tag;
        let [rphf, pref, blwf, abvf, half, pstf, init] =
            syllabic::bits(features, [b"rphf", b"pref", b"blwf", b"abvf", b"half", b"pstf", b"init"]);
        let stage = |tag: &[u8; 4]| features.stage(Table::Gsub, Tag::new(tag));
        let virama = Some(rules.virama).filter(|&v| v != 0).and_then(|v| face.glyph_of(v));
        Some(Box::new(Data {
            rules,
            first_spec,
            bare: !first_spec && script != Some(Script::MALAYALAM),
            masks: Masks { rphf, pref, blwf, abvf, half, pstf, init },
            stages: Stages {
                rphf: stage(b"rphf"),
                pref: stage(b"pref"),
                blwf: stage(b"blwf"),
                pstf: stage(b"pstf"),
                vatu: stage(b"vatu"),
            },
            virama,
            positions: Positions::for_font(face, virama.is_some()),
        }))
    }

    fn mark_advances(&self) -> MarkAdvances {
        MarkAdvances::Keep
    }

    fn takes_fallbacks(&self) -> bool {
        false
    }

    fn normalization(&self) -> Normalization {
        Normalization::Recomposed
    }

    // These stay whole, as fonts draw them: Devanagari RRA, Bengali RRA and RHA, Tamil AU.
    fn decompose(&self, c: char) -> Option<(char, Option<char>)> {
        match c {
            '\u{0931}' | '\u{09DC}' | '\u{09DD}' | '\u{0B94}' => None,
            _ => unicode::decompose(c),
        }
    }

    // Nothing composes onto a mark; Bengali YA and nukta give YYA, which composition excludes.
    fn compose(&self, a: char, b: char, _: bool) -> Option<char> {
        match (a, b) {
            _ if unicode::general_category(a).is_mark() => None,
            ('\u{09AF}', '\u{09BC}') => Some('\u{09DF}'),
            _ => unicode::compose(a, b),
        }
    }

    fn prepare(&self, run: &mut Run, _: &Plan, _: &Face) {
        syllabic::circle_vowel_lookalikes(run);
    }

    fn setup_masks(&self, run: &mut Run, _: &Plan, _: &Face) {
        let len = run.len;
        for glyph in &mut run.glyphs[..len] {
            (glyph.script_class, glyph.script_slot) = indic_category::lookup(glyph.id);
        }
    }
}

fn find_syllables(run: &mut Run, _: &Plan, _: &Face) -> bool {
    let class = |glyphs: &[Glyph], i: usize| Some(glyphs[i].script_class);
    syllable::setup(run, &INDIC_TRANSITIONS, class, indic_accept, IndicSyllable::BrokenCluster.into());
    false
}

// A symbol or a run of non-Indic characters isn't reordered.
fn reorders(kind: u8) -> bool {
    kind != IndicSyllable::SymbolCluster as u8 && kind != IndicSyllable::NonIndicCluster as u8
}

fn reorder_initially(run: &mut Run, plan: &Plan, face: &Face) -> bool {
    let Some(font) = FontForms::of(plan, face) else { return false };
    font.place_consonants(run);
    let circled = syllabic::circle_broken_syllables(face, run, &CIRCLE);
    syllabic::each_syllable(run, reorders, |s| font.initially(s));
    circled
}

fn reorder_finally(run: &mut Run, plan: &Plan, face: &Face) -> bool {
    if let Some(font) = FontForms::of(plan, face) {
        syllabic::each_syllable(run, |_| true, |s| font.finally(s));
    }
    false
}

// The plan's data and what the font's GSUB forms of a glyph sequence, as a pause reads them.
struct FontForms<'a> {
    data: &'a Data,
    probe: Probe<'a>,
}

impl<'a> FontForms<'a> {
    fn of(plan: &'a Plan, face: &'a Face<'a>) -> Option<Self> {
        Some(FontForms { data: plan.data::<Data>()?, probe: plan.probe(face) })
    }

    // `run` pays for each ask, and past its edits nothing forms.
    fn forms(&self, run: &mut Run, stage: Option<u16>, glyphs: &[u32]) -> bool {
        stage.is_some_and(|stage| self.probe.would_substitute(run, stage, glyphs, self.data.bare))
    }

    // The spec's "dynamic character properties": a consonant is below-base where blwf or vatu forms it with the
    // virama, post-base where pstf or pref does, else a base.
    fn ask_position(&self, run: &mut Run, consonant: u32, virama: u32) -> u8 {
        // Either order: the first version writes the consonant first and the second the virama first, and fonts carry
        // one's lookups under the other's tags.
        let mut either =
            |stage| self.forms(run, stage, &[virama, consonant]) || self.forms(run, stage, &[consonant, virama]);
        let stages = &self.data.stages;
        if either(stages.blwf) || either(stages.vatu) {
            at::BELOW
        } else if either(stages.pstf) || either(stages.pref) {
            at::POST
        } else {
            at::BASE
        }
    }

    // `ask_position`, kept from the first ask where the plan has room for it and the run could pay for all of it.
    fn position(&self, run: &mut Run, consonant: u32, virama: u32) -> u8 {
        let slot = self.data.positions.slots.get(consonant as usize);
        match slot.map(|s| s.load(Ordering::Relaxed)) {
            Some(kept) if kept != 0 => kept - 1,
            _ => {
                let asked = self.ask_position(run, consonant, virama);
                if run.ops_left() >= 0 {
                    slot.inspect(|s| s.store(asked + 1, Ordering::Relaxed));
                }
                asked
            }
        }
    }

    fn place_consonants(&self, run: &mut Run) {
        let Some(virama) = self.data.virama else { return };
        // Taken out while the asks, which read none of them, charge the run, so the loop keeps to the slice.
        let (len, mut glyphs) = (run.len, core::mem::take(&mut run.glyphs));
        for glyph in glyphs[..len].iter_mut().filter(|g| g.script_slot == at::BASE) {
            glyph.script_slot = self.position(run, glyph.id, virama);
        }
        run.glyphs = glyphs;
    }

    // The spec's "reorder characters": find the base, give every glyph its place, sort, then mark which basic forms
    // each part may take.
    fn initially(&self, s: &mut Syllable) {
        let rules = &self.data.rules;
        if rules.ra_zwj_virama && s.len() >= 3 && s.is(0, &[cat::RA]) && s.is(1, &[cat::H]) && s.is(2, &[cat::ZWJ]) {
            s.join(1..3);
            s.glyphs().swap(1, 2);
        }
        let (base, reph) = self.base_of(s);
        for i in 0..base {
            s.set_pos(i, s.pos(i).min(at::PRE_BASE));
        }
        if base < s.len() {
            s.set_pos(base, at::BASE);
        }
        if reph {
            s.set_pos(0, at::REPH);
        }
        if self.data.first_spec {
            self.trail_virama(s, base);
        }
        place_marks(s, base);
        let base = self.sort(s);
        self.mask_forms(s, base);
    }

    // Where the reph's RA and virama end (with the joiners after them), if the syllable opens with a reph.
    fn reph_end(&self, s: &mut Syllable) -> Option<usize> {
        let n = s.len();
        let past_joiners = |s: &Syllable, from: usize| (from..n).find(|&i| !s.is(i, &JOINERS)).unwrap_or(n);
        match self.data.rules.reph {
            Reph::Logical => (s.class(0) == cat::REPHA).then(|| past_joiners(s, 1)),
            form => {
                if self.data.masks.rphf == 0 || n < 3 {
                    return None;
                }
                let third = match form {
                    Reph::Explicit => s.class(2) == cat::ZWJ,
                    _ => !s.is(2, &JOINERS),
                };
                if !third {
                    return None;
                }
                let opening = [s[0].id, s[1].id, s[2].id];
                let rphf = self.data.stages.rphf;
                let formed = self.forms(s.run, rphf, &opening[..2])
                    || (form == Reph::Explicit && self.forms(s.run, rphf, &opening));
                formed.then(|| past_joiners(s, 2))
            }
        }
    }

    // The spec's "find base consonant": back from the end past below-base forms, and post-base ones until a below-base
    // is passed, stopped by a virama and ZWJ or the reph. A reph that ends by glyph 2 with no consonant after is none.
    fn base_of(&self, s: &mut Syllable) -> (usize, bool) {
        let n = s.len();
        let reph_end = self.reph_end(s);
        let floor = reph_end.unwrap_or(0).min(n - 1);
        let mut base = if reph_end.is_some() { 0 } else { n };
        let mut below_seen = false;
        for i in (floor..n).rev() {
            if !s.is(i, &CONSONANTS) {
                if i > 0 && s.class(i) == cat::ZWJ && s.class(i - 1) == cat::H {
                    break;
                }
                continue;
            }
            base = i;
            match s.pos(i) {
                at::BELOW => below_seen = true,
                at::POST if !below_seen => {}
                _ => break,
            }
        }
        let bare_reph = base == 0 && reph_end.is_some_and(|end| end <= 2);
        (base, reph_end.is_some() && !bare_reph)
    }

    // A first-version font wants the first virama after the base moved past the consonants after it (in Kannada, not
    // past another virama).
    fn trail_virama(&self, s: &mut Syllable, base: usize) {
        let n = s.len();
        let Some(virama) = (base + 1..n).find(|&i| s.class(i) == cat::H) else { return };
        let one_virama = self.data.rules.one_virama;
        let stop = (virama + 1..n).rev().find(|&i| s.is(i, &CONSONANTS) || (one_virama && s.class(i) == cat::H));
        if let Some(last) = stop.filter(|&i| s.class(i) != cat::H) {
            s.place(virama, last);
        }
    }

    // Sorts by position, the syllable byte lent to note each glyph's start: glyphs on one cycle of the permutation
    // crossed, so from the base on their clusters join (all of them, in the first version). Gives the base's index.
    fn sort(&self, s: &mut Syllable) -> usize {
        let n = s.len();
        debug_assert!(n <= 64, "a syllable pass takes at most 64 glyphs");
        let serial = s[0].syllable;
        s.glyphs().iter_mut().zip(0u8..).for_each(|(g, i)| g.syllable = i);
        s.glyphs().sort_by_key(|g| g.script_slot);

        let base = (0..n).find(|&i| s.pos(i) == at::BASE).unwrap_or(n);
        let first = (0..base).find(|&i| s.pos(i) == at::PRE_MATRA);
        let last = (0..base).rfind(|&i| s.pos(i) == at::PRE_MATRA);
        if let (Some(first), Some(last)) = (first, last) {
            s.flip_pre_base(first..last + 1, &MATRAS);
        }

        if self.data.first_spec {
            s.join(base..n);
        } else {
            let mut done = 0u64;
            for i in base..n {
                if done & 1 << i != 0 {
                    continue;
                }
                let (mut low, mut high, mut j) = (i, i, usize::from(s[i].syllable));
                while j != i {
                    (low, high) = (low.min(j), high.max(j));
                    done |= 1 << j;
                    j = usize::from(s[j].syllable);
                }
                s.join(base.max(low)..high + 1);
            }
        }
        s.glyphs().iter_mut().for_each(|g| g.syllable = serial);
        base
    }

    // The basic forms each part may take: rphf the reph; half, and blwf in the second version, before the base; blwf,
    // abvf, pstf after it; pref the first pair that forms it. A ZWNJ bars half forms back to the consonant before it.
    fn mask_forms(&self, s: &mut Syllable, base: usize) {
        let (m, rules) = (&self.data.masks, &self.data.rules);
        let n = s.len();
        let reph = (0..n).take_while(|&i| s.pos(i) == at::REPH).count();
        s.glyphs()[..reph].iter_mut().for_each(|g| g.mask |= m.rphf);

        let below = if !self.data.first_spec && rules.below_before_base { m.blwf } else { 0 };
        s.glyphs()[..base].iter_mut().for_each(|g| g.mask |= m.half | below);
        if base + 1 < n {
            s.glyphs()[base + 1..].iter_mut().for_each(|g| g.mask |= m.blwf | m.abvf | m.pstf);
        }

        if self.data.first_spec && rules.eyelash_ra {
            for i in 0..base.saturating_sub(1) {
                let eyelash = i + 2 < base && s.class(i + 2) == cat::ZWJ;
                if s.class(i) == cat::RA && s.class(i + 1) == cat::H && !eyelash {
                    s[i].mask |= m.blwf;
                    s[i + 1].mask |= m.blwf;
                }
            }
        }

        if m.pref != 0 && base + 2 < n {
            let formed = (base + 1..n - 1).find(|&i| {
                let pair = [s[i].id, s[i + 1].id];
                self.forms(s.run, self.data.stages.pref, &pair)
            });
            if let Some(i) = formed {
                s[i].mask |= m.pref;
                s[i + 1].mask |= m.pref;
            }
        }

        for i in 1..n {
            if !s.is(i, &JOINERS) {
                continue;
            }
            let zwnj = s.class(i) == cat::ZWNJ;
            for j in (0..i).rev() {
                if zwnj {
                    s[j].mask &= !m.half;
                }
                if j == 0 || s.is(j, &CONSONANTS) {
                    break;
                }
            }
        }
    }

    // The spec's "final reordering", on what the basic forms made of the syllable.
    fn finally(&self, s: &mut Syllable) {
        if let Some(virama) = self.data.virama {
            // A virama a ligature split back out reads as a virama again.
            let split = class::LIGATED | class::MULTIPLIED;
            for g in s.glyphs().iter_mut().filter(|g| g.id == virama && g.class & split == split) {
                g.script_class = cat::H;
                g.class &= !split;
            }
        }
        let mut pref_left = self.data.masks.pref != 0;
        let mut base = self.final_base(s, &mut pref_left);
        self.move_pre_base_matras(s, &mut base);
        self.move_reph(s, &mut base);
        if pref_left {
            self.move_pref(s, base);
        }
        if s.pos(0) == at::PRE_MATRA {
            self.init_or_join(s);
        }
    }

    // The base again, now that forms are made: the first glyph placed at the base or after it, moved on to a glyph
    // pref didn't form (a pre-base RA left whole), and back off a nukta or virama.
    fn final_base(&self, s: &mut Syllable, pref_left: &mut bool) -> usize {
        let n = s.len();
        let base = 'found: {
            let Some(mut base) = (0..n).find(|&i| s.pos(i) >= at::BASE) else { break 'found n };
            if *pref_left && base + 1 < n {
                let pref = self.data.masks.pref;
                if let Some(i) = (base + 1..n).find(|&i| s[i].mask & pref != 0) {
                    let formed = s[i].glyph_is(class::SUBSTITUTED) && s[i].only_ligated();
                    if !formed {
                        base = (i..n).find(|&k| !s.is(k, &[cat::H])).unwrap_or(n);
                        if base < n {
                            s.set_pos(base, at::BASE);
                        }
                        *pref_left = false;
                    }
                }
                if base == n {
                    break 'found n;
                }
            }
            if self.data.rules.unformed_below_is_base {
                base = self.unformed_below_base(s, base);
            }
            if base > 0 && s.pos(base) > at::BASE {
                base -= 1;
            }
            base
        };
        // With no base, a ZWJ ending the syllable stands as one; a base never ends on its nukta or virama.
        if base == n {
            return n - usize::from(s.is(n - 1, &[cat::ZWJ]));
        }
        base - (1..=base).rev().take_while(|&k| s.is(k, &[cat::N, cat::H])).count()
    }

    // Malayalam: a virama then a below-base consonant after the base, joiners between them passed over; each such
    // consonant becomes the base in turn.
    fn unformed_below_base(&self, s: &mut Syllable, mut base: usize) -> usize {
        let n = s.len();
        let past_joiners = |s: &Syllable, from: usize| (from..n).find(|&i| !s.is(i, &JOINERS)).unwrap_or(n);
        let mut i = base + 1;
        while i < n {
            i = past_joiners(s, i);
            if i == n || !s.is(i, &[cat::H]) {
                break;
            }
            i = past_joiners(s, i + 1);
            if i < n && s.is(i, &CONSONANTS) && s.pos(i) == at::BELOW {
                base = i;
                s.set_pos(base, at::BASE);
            }
            i += 1;
        }
        base
    }

    // A pre-base matra moves to just after the last virama left standing, passing one a ZWJ follows (a half form), or
    // before the base in a script without half forms; with neither it stays. Its cluster joins the base's.
    fn move_pre_base_matras(&self, s: &mut Syllable, base: &mut usize) {
        let n = s.len();
        if n < 2 || *base == 0 {
            return;
        }
        // Just before the base, or the last glyph where there's none.
        let mut to = *base - 1 - usize::from(*base == n);
        if self.data.rules.half_forms {
            loop {
                while to > 0 && !s.is(to, &MATRAS_OR_VIRAMA) {
                    to -= 1;
                }
                if !(s.is(to, &[cat::H]) && s.pos(to) != at::PRE_MATRA) {
                    to = 0;
                    break;
                }
                let zwj_after = to + 1 < n && s.class(to + 1) == cat::ZWJ;
                if to == 0 || !zwj_after {
                    break;
                }
                to -= 1;
            }
        }
        let through_base = |base: usize| n.min(base + 1);
        if to == 0 || s.pos(to) == at::PRE_MATRA {
            if let Some(first) = (0..*base).find(|&i| s.pos(i) == at::PRE_MATRA) {
                s.join(first..through_base(*base));
            }
            return;
        }
        // Back from `to`, each matra found moves there, and the place before it is the next one's.
        let mut i = to;
        while i > 0 {
            i -= 1;
            if s.pos(i) != at::PRE_MATRA {
                continue;
            }
            if i < *base && *base <= to {
                *base -= 1;
            }
            s.place(i, to);
            s.join(to..through_base(*base));
            to -= 1;
        }
    }

    // A reph the font formed (a ligated RA, or a dot reph left whole) moves to where its script's spec puts it.
    fn move_reph(&self, s: &mut Syllable, base: &mut usize) {
        if s.len() < 2 || s.pos(0) != at::REPH {
            return;
        }
        if (s.class(0) == cat::REPHA) == s[0].only_ligated() {
            return;
        }
        let to = self.reph_target(s, *base);
        s.carry(0, to);
        if 0 < *base && *base <= to {
            *base -= 1;
        }
    }

    // The spec's reph steps: after the first virama before the base and its joiner; else past the main consonant or the
    // below-base forms, per script; else the end before final signs, less one per matra past the base if it's a virama.
    fn reph_target(&self, s: &Syllable, base: usize) -> usize {
        let n = s.len();
        let virama = (1..base).find(|&i| s.is(i, &[cat::H]));
        if let Some(v) = virama {
            return v + usize::from(v + 1 < base && s.is(v + 1, &JOINERS));
        }
        let through = |stays: &dyn Fn(u8) -> bool| {
            let count = (base + 1..n).take_while(|&i| stays(s.pos(i))).count();
            Some(base + count).filter(|&k| k < n)
        };
        let settled = match self.data.rules.reph_goes {
            RephGoes::AfterMain => through(&|p| p <= at::AFTER_BASE),
            RephGoes::AfterBelow => through(&|p| !matches!(p, at::POST | at::AFTER_POST | at::SIGNS)),
            _ => None,
        };
        if let Some(k) = settled {
            return k;
        }
        let mut end = (1..n).rev().find(|&k| s.pos(k) != at::SIGNS).unwrap_or(0);
        if s.is(end, &[cat::H]) {
            let mut i = base + 1;
            while i < end {
                end -= usize::from(MATRAS.contains(&s.class(i)));
                i += 1;
            }
        }
        end
    }

    // A glyph pref formed moves before the base: where a pre-base matra would go, else just before the base.
    fn move_pref(&self, s: &mut Syllable, base: usize) {
        let n = s.len();
        if base + 1 >= n {
            return;
        }
        let pref = self.data.masks.pref;
        let Some(i) = (base + 1..n).find(|&i| s[i].mask & pref != 0) else { return };
        if !s[i].only_ligated() {
            return;
        }
        let mut to = base;
        if self.data.rules.half_forms {
            while to > 0 && !s.is(to - 1, &MATRAS_OR_VIRAMA) {
                to -= 1;
            }
        }
        if to > 0 && s.is(to - 1, &[cat::H]) && to < n && s.is(to, &JOINERS) {
            to += 1;
        }
        s.carry(i, to);
    }

    // A syllable opening with a pre-base matra takes init where it starts a word; inside one, it can't be broken
    // from what precedes it.
    fn init_or_join(&self, s: &mut Syllable) {
        use GeneralCategory as G;
        let Some(previous) = s.start.checked_sub(1) else {
            s[0].mask |= self.data.masks.init;
            return;
        };
        let gc = s.run.glyphs[previous].category;
        match matches!(gc, G::Format | G::Unassigned | G::PrivateUse | G::Surrogate) || gc.is_letter() || gc.is_mark() {
            true => s.run.unsafe_to_break(previous..s.start + 1),
            false => s[0].mask |= self.data.masks.init,
        }
    }
}

// The spec's sign placement: a nukta, joiner or virama goes with what it follows (past pre-base matras, for a virama),
// a post-base matra takes a modifier before it along, and signs between consonants after the base go with the later.
fn place_marks(s: &mut Syllable, base: usize) {
    let n = s.len();
    let mut last = at::FIRST;
    for i in 0..n {
        let class = s.class(i);
        if matches!(class, cat::ZWJ | cat::ZWNJ | cat::N | cat::MEDIAL | cat::H) {
            let behind_matras = (class == cat::H && last == at::PRE_MATRA)
                .then(|| (0..i).rfind(|&k| s.pos(k) != at::PRE_MATRA))
                .flatten();
            s.set_pos(i, behind_matras.map_or(last, |k| s.pos(k)));
        } else if s.pos(i) != at::SIGNS {
            if class == cat::M_POST && i > 0 && s.class(i - 1) == cat::SM {
                s.set_pos(i - 1, s.pos(i));
            }
            last = s.pos(i);
        }
    }

    let mut owner = base;
    for i in base + 1..n {
        if s.is(i, &CONSONANTS) {
            let place = s.pos(i);
            for j in owner + 1..i {
                if s.pos(j) < at::SIGNS {
                    s.set_pos(j, place);
                }
            }
            owner = i;
        } else if MATRAS.contains(&s.class(i)) {
            owner = i;
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/script/indic.rs"]
mod tests;
