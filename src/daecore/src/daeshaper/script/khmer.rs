// Khmer, after Microsoft's Khmer script development spec: syllables found by the Khmer grammar, then a coeng RO and
// a pre-base vowel moved before the base, then the basic forms per syllable and the presentation forms.
use super::indic_category::{self, category};
use super::syllabic::{self, Circle, Step, Syllable};
use super::syllable;
use super::{MarkAdvances, Model, ModelData, Normalization, Shaper};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::generated::syllable_tables::{khmer_accept, KhmerSyllable, KHMER_TRANSITIONS};
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::plan::{FeatureUse as U, Features, Plan, Planner};
use crate::daecore::daeshaper::run::{Glyph, Mask, Run};
use crate::daecore::daeshaper::unicode::{self, Script};

pub(crate) struct Khmer;

pub(crate) const MODEL: Khmer = Khmer;

const PER_SYLLABLE: U = U::GLOBAL.and(U::PER_SYLLABLE);
const BASIC: U = U::SEES_JOINERS.and(U::PER_SYLLABLE);
const PRESENTATION: U = U::GLOBAL.and(U::SEES_JOINERS);

const PLAN: &[Step] = &[
    Step::End(Some(find_syllables)),
    Step::End(Some(reorder)),
    Step::Together(&[
        (b"locl", PER_SYLLABLE),
        (b"ccmp", PER_SYLLABLE),
        (b"pref", BASIC),
        (b"blwf", BASIC),
        (b"abvf", BASIC),
        (b"pstf", BASIC),
        (b"cfar", BASIC),
    ]),
    Step::End(Some(syllable::clear)),
    Step::Together(&[
        (b"pres", PRESENTATION),
        (b"abvs", PRESENTATION),
        (b"blws", PRESENTATION),
        (b"psts", PRESENTATION),
    ]),
];

const CIRCLE: Circle = Circle {
    broken: KhmerSyllable::BrokenCluster as u8,
    class: category::DOTTED_CIRCLE,
    position: None,
    after: Some(category::REPHA),
};

// The basic forms a reordered syllable's glyphs take.
struct Masks {
    pref: Mask,
    after_base: Mask,
    cfar: Mask,
}

impl Model for Khmer {
    fn kind(&self) -> Shaper {
        Shaper::Khmer
    }

    fn plan_features(&self, planner: &mut Planner<'_>, _: Option<Tag>) {
        syllabic::declare(planner, PLAN);
    }

    // After the caller's features, so they hold: Khmer's ligatures are contextual, never discretionary.
    fn plan_late_features(&self, planner: &mut Planner<'_>) {
        planner.on(Tag::new(b"clig"), U::PLAIN);
        planner.off(Tag::new(b"liga"));
    }

    fn plan_data(&self, features: &Features, _: &Face, _: Option<Script>, _: Option<Tag>) -> Option<ModelData> {
        let [pref, blwf, abvf, pstf, cfar] = syllabic::bits(features, [b"pref", b"blwf", b"abvf", b"pstf", b"cfar"]);
        Some(alloc::boxed::Box::new(Masks { pref, after_base: blwf | abvf | pstf, cfar }))
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

    // The split vowels go to their pre-base part, U+17C1, and themselves, which the font draws as the rest.
    fn decompose(&self, c: char) -> Option<(char, Option<char>)> {
        match c {
            '\u{17BE}'..='\u{17C0}' | '\u{17C4}' | '\u{17C5}' => Some(('\u{17C1}', Some(c))),
            _ => unicode::decompose(c),
        }
    }

    fn compose(&self, a: char, b: char, _: bool) -> Option<char> {
        (!unicode::general_category(a).is_mark()).then(|| unicode::compose(a, b)).flatten()
    }

    fn setup_masks(&self, run: &mut Run, _: &Plan, _: &Face) {
        let len = run.len;
        for glyph in &mut run.glyphs[..len] {
            glyph.script_class = indic_category::lookup(glyph.id).0;
        }
    }
}

fn find_syllables(run: &mut Run, _: &Plan, _: &Face) -> bool {
    let class = |glyphs: &[Glyph], i: usize| Some(glyphs[i].script_class);
    syllable::setup(run, &KHMER_TRANSITIONS, class, khmer_accept, KhmerSyllable::BrokenCluster.into());
    false
}

fn reorder(run: &mut Run, plan: &Plan, face: &Face) -> bool {
    let circled = syllabic::circle_broken_syllables(face, run, &CIRCLE);
    if let Some(masks) = plan.data::<Masks>() {
        let khmer = |kind| kind != KhmerSyllable::NonKhmerCluster as u8;
        syllabic::each_syllable(run, khmer, |s| arrange(s, masks));
    }
    circled
}

// All but the first glyph may take after-base forms; a coeng RO goes before the base as pref, opening the rest to
// `cfar`, and a pre-base vowel goes first. Coengs are read while two or fewer came before, a coeng RO counting two.
fn arrange(s: &mut Syllable, masks: &Masks) {
    let n = s.len();
    s.glyphs()[1..].iter_mut().for_each(|g| g.mask |= masks.after_base);
    let mut coengs = 0;
    for i in 1..n {
        match s.class(i) {
            category::H if coengs <= 2 && i + 1 < n => {
                coengs += 1;
                if s.class(i + 1) != category::RA {
                    continue;
                }
                s[i].mask |= masks.pref;
                s[i + 1].mask |= masks.pref;
                s.join(0..i + 2);
                s.place(i, 0);
                s.place(i + 1, 1);
                s.glyphs()[i + 2..].iter_mut().for_each(|g| g.mask |= masks.cfar);
                coengs = 2;
            }
            category::V_PRE => s.carry(i, 0),
            _ => {}
        }
    }
}
