// The Universal Shaping Engine, after Microsoft's spec for it: clusters, preprocessing, rphf and pref with what they
// formed noted, orthographic unit features, one reordering, then topographical and typographic features.
use super::arabic::Joining;
use super::syllabic::{self, Circle, Step, Syllable};
use super::syllable;
use super::{MarkAdvances, Model, ModelData, Normalization, Shaper};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::generated::syllable_tables::{
    lana_accept, use_accept, UseSyllable, LANA_TRANSITIONS, USE_TRANSITIONS,
};
use crate::daecore::daeshaper::generated::unicode_tables::ARABIC_JOINING;
use crate::daecore::daeshaper::generated::use_tables::{category, USE_CATEGORY};
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::plan::{FeatureUse as U, Features, Plan, Planner};
use crate::daecore::daeshaper::run::{class, Glyph, Mask, Run};
use crate::daecore::daeshaper::unicode::{self, Script};

pub(crate) struct Universal;

pub(crate) const MODEL: Universal = Universal;

const PER_CLUSTER: U = U::GLOBAL.and(U::PER_SYLLABLE);
const CLUSTER_ZWJ: U = U::GLOBAL.and(U::SEES_ZWJ).and(U::PER_SYLLABLE);
const TYPOGRAPHIC: U = U::GLOBAL.and(U::SEES_ZWJ);

// The spec's "OpenType feature application" groups. rphf and pref each run on glyphs whose substitution flag was
// cleared, so the pause after each reads what it formed; rphf's mask reaches only a cluster's first glyphs.
const PLAN: &[Step] = &[
    Step::End(Some(find_clusters)),
    Step::Together(&[(b"locl", PER_CLUSTER), (b"ccmp", PER_CLUSTER), (b"nukt", PER_CLUSTER), (b"akhn", CLUSTER_ZWJ)]),
    Step::End(Some(forget_substitutions)),
    Step::Together(&[(b"rphf", U::SEES_ZWJ.and(U::PER_SYLLABLE))]),
    Step::End(Some(note_reph)),
    Step::End(Some(forget_substitutions)),
    Step::Together(&[(b"pref", CLUSTER_ZWJ)]),
    Step::End(Some(note_pref)),
    Step::Together(&[
        (b"rkrf", CLUSTER_ZWJ),
        (b"abvf", CLUSTER_ZWJ),
        (b"blwf", CLUSTER_ZWJ),
        (b"half", CLUSTER_ZWJ),
        (b"pstf", CLUSTER_ZWJ),
        (b"vatu", CLUSTER_ZWJ),
        (b"cjct", CLUSTER_ZWJ),
    ]),
    Step::End(Some(reorder)),
    Step::End(Some(syllable::clear)),
    Step::Together(&[(b"isol", U::PLAIN), (b"init", U::PLAIN), (b"medi", U::PLAIN), (b"fina", U::PLAIN)]),
    Step::End(None),
    Step::Together(&[
        (b"abvs", TYPOGRAPHIC),
        (b"blws", TYPOGRAPHIC),
        (b"haln", TYPOGRAPHIC),
        (b"pres", TYPOGRAPHIC),
        (b"psts", TYPOGRAPHIC),
    ]),
];

const CIRCLE: Circle =
    Circle { broken: UseSyllable::BrokenCluster as u8, class: category::B, position: None, after: Some(category::R) };

// Classes that stand after the base: finals, medials, vowels and vowel modifiers, the pre-base ones included as they
// follow the base in the text.
const AFTER_BASE: [u8; 18] = [
    category::F_ABV,
    category::F_BLW,
    category::F_PST,
    category::FM_ABV,
    category::FM_BLW,
    category::FM_PST,
    category::M_ABV,
    category::M_BLW,
    category::M_PST,
    category::M_PRE,
    category::V_ABV,
    category::V_BLW,
    category::V_PST,
    category::V_PRE,
    category::VM_ABV,
    category::VM_BLW,
    category::VM_PST,
    category::VM_PRE,
];

// A virama, explicit or invisible: reordering doesn't pass one.
const VIRAMAS: [u8; 3] = [category::H, category::VOWEL_VIRAMA, category::IS];

// The cluster kinds whose repha and pre-base glyphs move.
const REORDERED: [UseSyllable; 5] = [
    UseSyllable::ViramaTerminatedCluster,
    UseSyllable::SakotTerminatedCluster,
    UseSyllable::StandardCluster,
    UseSyllable::SymbolCluster,
    UseSyllable::BrokenCluster,
];

struct Masks {
    rphf: Mask,
    // isol, init, medi and fina's, in that order; 0 for one that is on every glyph already.
    topographical: [Mask; 4],
    joining: Joining,
}

// A cluster's topographical form, as the spec's topographical features name them.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Form {
    Isol,
    Init,
    Medi,
    Fina,
}

impl Model for Universal {
    fn kind(&self) -> Shaper {
        Shaper::Universal
    }

    fn plan_features(&self, planner: &mut Planner<'_>, _: Option<Tag>) {
        syllabic::declare(planner, PLAN);
    }

    fn plan_data(&self, features: &Features, _: &Face, _: Option<Script>, _: Option<Tag>) -> Option<ModelData> {
        let topographical = syllabic::bits(features, [b"isol", b"init", b"medi", b"fina"])
            .map(|bit| if bit == features.global() { 0 } else { bit });
        let rphf = features.on_bit(Tag::new(b"rphf"));
        Some(alloc::boxed::Box::new(Masks { rphf, topographical, joining: Joining::new(features) }))
    }

    fn mark_advances(&self) -> MarkAdvances {
        MarkAdvances::ZeroBeforeGpos
    }

    fn takes_fallbacks(&self) -> bool {
        false
    }

    fn normalization(&self) -> Normalization {
        Normalization::Recomposed
    }

    fn compose(&self, a: char, b: char, _: bool) -> Option<char> {
        (!unicode::general_category(a).is_mark()).then(|| unicode::compose(a, b)).flatten()
    }

    fn prepare(&self, run: &mut Run, _: &Plan, _: &Face) {
        syllabic::circle_vowel_lookalikes(run);
    }

    fn setup_masks(&self, run: &mut Run, plan: &Plan, _: &Face) {
        if let Some(masks) = plan.data::<Masks>()
            && joins(run.script)
        {
            masks.joining.apply(run);
        }
        let len = run.len;
        for glyph in &mut run.glyphs[..len] {
            glyph.script_class = class_of(glyph.id);
        }
    }
}

// Whether the grammar can take `c` in a cluster but not as the one that opens it.
pub(crate) fn is_dependent(c: u32) -> bool {
    use category::*;
    !matches!(class_of(c), O | B | N | GB | CGJ | WJ | CS | G | J | SB | SE | HM | HR)
}

fn class_of(c: u32) -> u8 {
    if c > 0x10_FFFF { category::O } else { USE_CATEGORY.get(c).category }
}

// A script joining as Arabic does takes its letters' forms from the joining types; the rest, by cluster.
fn joins(script: Option<Script>) -> bool {
    script.is_some_and(|s| ARABIC_JOINING.contains(&s))
}

// The spec's grammar reads a CGJ as nothing, and a ZWNJ as nothing where a mark follows it.
fn class_in_grammar(glyphs: &[Glyph], i: usize) -> Option<u8> {
    let here = glyphs[i].script_class;
    let read = match here {
        category::CGJ => false,
        category::ZWNJ => {
            let next = glyphs[i + 1..].iter().find(|g| g.script_class != category::CGJ);
            next.is_none_or(|g| !g.is_mark_char())
        }
        _ => true,
    };
    read.then_some(here)
}

fn find_clusters(run: &mut Run, plan: &Plan, _: &Face) -> bool {
    let broken = UseSyllable::BrokenCluster.into();
    // Tai Tham writes a cluster's vowels in orders the spec's pattern forbids, so it has a grammar of its own.
    match run.script == Some(Script::TAI_THAM) {
        true => syllable::setup(run, &LANA_TRANSITIONS, class_in_grammar, lana_accept, broken),
        false => syllable::setup(run, &USE_TRANSITIONS, class_in_grammar, use_accept, broken),
    }
    if let Some(masks) = plan.data::<Masks>() {
        scope_reph(run, masks.rphf);
        give_forms(run, &masks.topographical);
    }
    false
}

// rphf reaches a cluster's first three glyphs, or its repha alone.
fn scope_reph(run: &mut Run, rphf: Mask) {
    if rphf == 0 {
        return;
    }
    syllable::for_each(run, |run, start, end| {
        let reach = if run.glyphs[start].script_class == category::R { 1 } else { (end - start).min(3) };
        run.glyphs[start..start + reach].iter_mut().for_each(|g| g.mask |= rphf);
    });
}

// Each cluster of a script that doesn't join by type takes isol, or fina after another cluster, which then turns to
// init or medi; a hieroglyph or a non-cluster breaks the chain.
fn give_forms(run: &mut Run, masks: &[Mask; 4]) {
    let all = masks.iter().fold(0, |all, &m| all | m);
    if all == 0 || joins(run.script) {
        return;
    }
    let set = |glyphs: &mut [Glyph], form: Form| {
        glyphs.iter_mut().for_each(|g| g.mask = g.mask & !all | masks[form as usize]);
    };
    let mut previous: Option<(usize, Form)> = None;
    syllable::for_each(run, |run, start, end| {
        let kind = run.glyphs[start].syllable_kind();
        let breaks = kind == UseSyllable::HieroglyphCluster as u8 || kind == UseSyllable::NonCluster as u8;
        previous = match (breaks, previous) {
            (true, _) => None,
            (false, Some((at, form))) => {
                set(&mut run.glyphs[at..start], if form == Form::Fina { Form::Medi } else { Form::Init });
                set(&mut run.glyphs[start..end], Form::Fina);
                Some((start, Form::Fina))
            }
            (false, None) => {
                set(&mut run.glyphs[start..end], Form::Isol);
                Some((start, Form::Isol))
            }
        };
    });
}

fn forget_substitutions(run: &mut Run, _: &Plan, _: &Face) -> bool {
    let len = run.len;
    run.glyphs[..len].iter_mut().for_each(|g| g.class &= !class::SUBSTITUTED);
    false
}

// A glyph rphf formed at a cluster's start reorders as a repha.
fn note_reph(run: &mut Run, plan: &Plan, _: &Face) -> bool {
    let rphf = plan.data::<Masks>().map_or(0, |m| m.rphf);
    if rphf != 0 {
        syllable::for_each(run, |run, start, end| {
            let mut scoped = run.glyphs[start..end].iter_mut().take_while(|g| g.mask & rphf != 0);
            if let Some(formed) = scoped.find(|g| g.glyph_is(class::SUBSTITUTED)) {
                formed.script_class = category::R;
            }
        });
    }
    false
}

// The first glyph pref formed in a cluster reorders as a pre-base vowel.
fn note_pref(run: &mut Run, _: &Plan, _: &Face) -> bool {
    syllable::for_each(run, |run, start, end| {
        if let Some(formed) = run.glyphs[start..end].iter_mut().find(|g| g.glyph_is(class::SUBSTITUTED)) {
            formed.script_class = category::V_PRE;
        }
    });
    false
}

fn reorder(run: &mut Run, _: &Plan, face: &Face) -> bool {
    let circled = syllabic::circle_broken_syllables(face, run, &CIRCLE);
    let reordered = |kind| REORDERED.iter().any(|&k| k as u8 == kind);
    syllabic::each_syllable(run, reordered, |s| {
        move_repha(s);
        move_pre_base(s);
    });
    circled
}

// The spec's "glyph reordering": a repha goes after the base, before the first glyph that stands after it or a
// virama; with none, to the cluster's end.
fn move_repha(s: &mut Syllable) {
    let n = s.len();
    if n < 2 || s.class(0) != category::R {
        return;
    }
    let after = (1..n).find(|&i| AFTER_BASE.contains(&s.class(i)) || s.is(i, &VIRAMAS));
    s.carry(0, after.map_or(n - 1, |i| i - 1));
}

// Pre-base vowels and vowel modifiers go to the cluster's start, or just after the last virama before them; each one
// moved goes before those moved already. A piece of a ligature stays.
fn move_pre_base(s: &mut Syllable) {
    let mut front = 0;
    for i in 0..s.len() {
        if s.is(i, &VIRAMAS) {
            front = i + 1;
        } else if matches!(s.class(i), category::V_PRE | category::VM_PRE) && s[i].component() == 0 && front < i {
            s.carry(i, front);
        }
    }
}
