// Myanmar, after Microsoft's Myanmar script development spec: each syllable sorted into the spec's visual order, then
// the basic and the presentation forms. Zawgyi text isn't in Unicode's order, so it gets none of that.
use super::indic_category::{self, category, position};
use super::syllabic::{self, Circle, Step, Syllable};
use super::syllable;
use super::{MarkAdvances, Model, Normalization, Shaper};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::generated::syllable_tables::{myanmar_accept, MyanmarSyllable, MYANMAR_TRANSITIONS};
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::plan::{FeatureUse as U, Plan, Planner};
use crate::daecore::daeshaper::run::{Glyph, Run};

pub(crate) struct Myanmar;

pub(crate) const MODEL: Myanmar = Myanmar;

// Myanmar in the Zawgyi encoding, whose characters aren't Unicode's order, so nothing reorders or recomposes them.
pub(crate) struct Zawgyi;

pub(crate) const ZAWGYI: Zawgyi = Zawgyi;

const PER_SYLLABLE: U = U::GLOBAL.and(U::PER_SYLLABLE);
const BASIC: U = U::GLOBAL.and(U::SEES_ZWJ).and(U::PER_SYLLABLE);
const PRESENTATION: U = U::GLOBAL.and(U::SEES_ZWJ);

const PLAN: &[Step] = &[
    Step::End(Some(find_syllables)),
    Step::Together(&[(b"locl", PER_SYLLABLE), (b"ccmp", PER_SYLLABLE)]),
    Step::End(Some(reorder)),
    Step::OneByOne(&[(b"rphf", BASIC), (b"pref", BASIC), (b"blwf", BASIC), (b"pstf", BASIC)]),
    Step::End(Some(syllable::clear)),
    Step::Together(&[
        (b"pres", PRESENTATION),
        (b"abvs", PRESENTATION),
        (b"blws", PRESENTATION),
        (b"psts", PRESENTATION),
    ]),
];

const CIRCLE: Circle = Circle {
    broken: MyanmarSyllable::BrokenCluster as u8,
    class: category::DOTTED_CIRCLE,
    position: None,
    after: None,
};

// What can stand as a syllable's base.
const BASES: [u8; 6] =
    [category::C, category::STACKER, category::RA, category::V, category::PLACEHOLDER, category::DOTTED_CIRCLE];

impl Model for Myanmar {
    fn kind(&self) -> Shaper {
        Shaper::Myanmar
    }

    fn plan_features(&self, planner: &mut Planner<'_>, _: Option<Tag>) {
        syllabic::declare(planner, PLAN);
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

    fn setup_masks(&self, run: &mut Run, _: &Plan, _: &Face) {
        let len = run.len;
        for glyph in &mut run.glyphs[..len] {
            (glyph.script_class, glyph.script_slot) = indic_category::lookup(glyph.id);
        }
    }
}

impl Model for Zawgyi {
    fn kind(&self) -> Shaper {
        Shaper::MyanmarZawgyi
    }

    fn mark_advances(&self) -> MarkAdvances {
        MarkAdvances::Keep
    }

    fn takes_fallbacks(&self) -> bool {
        false
    }

    fn normalization(&self) -> Normalization {
        Normalization::Unrecomposed
    }
}

fn find_syllables(run: &mut Run, _: &Plan, _: &Face) -> bool {
    let class = |glyphs: &[Glyph], i: usize| Some(glyphs[i].script_class);
    syllable::setup(run, &MYANMAR_TRANSITIONS, class, myanmar_accept, MyanmarSyllable::BrokenCluster.into());
    false
}

fn reorder(run: &mut Run, _: &Plan, face: &Face) -> bool {
    let circled = syllabic::circle_broken_syllables(face, run, &CIRCLE);
    const CONSONANT: u8 = MyanmarSyllable::ConsonantSyllable as u8;
    const BROKEN: u8 = MyanmarSyllable::BrokenCluster as u8;
    syllabic::each_syllable(run, |kind| matches!(kind, CONSONANT | BROKEN), arrange);
    circled
}

// Gives each glyph its place in the spec's order, then sorts the syllable by them.
fn arrange(s: &mut Syllable) {
    let n = s.len();
    let kinzi = n >= 3 && [category::RA, category::AS, category::H].iter().enumerate().all(|(i, &c)| s.class(i) == c);
    let lead = if kinzi { 3 } else { 0 };
    let base = (lead..n).find(|&i| s.is(i, &BASES)).unwrap_or(0);

    // Kinzi, then what stands before the base, then the base.
    (0..lead).for_each(|i| s.set_pos(i, position::AFTER_BASE));
    let mut next = lead;
    while next < base {
        s.set_pos(next, position::PRE_BASE);
        next += 1;
    }
    if next < n {
        s.set_pos(next, position::BASE);
        next += 1;
    }

    // After the base, the zone moves on from after-main through below-base to after the below-base vowels.
    let mut zone = position::AFTER_BASE;
    for i in next..n {
        let place = match s.class(i) {
            category::MR => position::PRE_BASE,
            category::V_PRE => position::PRE_MATRA,
            category::VS => s.pos(i - 1),
            category::V_BLW if zone <= position::BELOW => {
                zone = position::BELOW;
                zone
            }
            category::A if zone == position::BELOW => position::BEFORE_BELOW,
            _ if zone == position::BELOW => {
                zone = position::AFTER_BELOW;
                zone
            }
            _ => zone,
        };
        s.set_pos(i, place);
    }

    let span = s.start..s.start + n;
    s.run.sort_span_by(span, |g| g.script_slot);
    let first = (0..n).find(|&i| s.pos(i) == position::PRE_MATRA);
    let last = (0..n).rfind(|&i| s.pos(i) == position::PRE_MATRA);
    if let (Some(first), Some(last)) = (first, last) {
        s.flip_pre_base(first..last + 1, &[category::V_PRE]);
    }
}
