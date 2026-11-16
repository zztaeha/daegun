// A script's shaping model: what it adds to a plan and the hooks the pipeline calls at fixed points of a shape. Each
// script module gives one; the registry picks one for a run.
pub(crate) mod arabic;
pub(crate) mod thai;
mod hangul;
mod hebrew;
mod indic;
mod indic_category;
mod khmer;
mod myanmar;
mod syllabic;
mod syllable;
mod universal;

use alloc::boxed::Box;
use core::any::Any;
use core::ops::Range;
use core::panic::RefUnwindSafe;

use super::face::Face;
use super::generated::use_tables::USE_SCRIPTS;
use super::ot::tag::Tag;
use super::plan::{Features, Plan, Planner};
use super::run::Run;
use super::unicode::{self, Script};

// A model's own plan-time data, which it reads back with `Plan::data`.
pub(crate) type ModelData = Box<dyn Keepable>;

// Under `threading` a plan is shared across threads and kept through a caller's caught panic, so a model's data is
// too; without it nothing of a font crosses either.
#[cfg(feature = "threading")]
pub(crate) trait Keepable: Any + Send + Sync + core::panic::UnwindSafe + RefUnwindSafe {}
#[cfg(feature = "threading")]
impl<T: Any + Send + Sync + core::panic::UnwindSafe + RefUnwindSafe> Keepable for T {}
#[cfg(not(feature = "threading"))]
pub(crate) trait Keepable: Any {}
#[cfg(not(feature = "threading"))]
impl<T: Any> Keepable for T {}

// What becomes of a mark's advance: kept, or zeroed before or after GPOS by GDEF's mark class.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MarkAdvances {
    Keep,
    ZeroBeforeGpos,
    ZeroAfterGpos,
}

// How the normalizer treats the run's characters.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Normalization {
    // A character the font maps stays; any other decomposes; a mark recomposes with its base where the font maps the
    // result.
    Precomposed,
    // Every character decomposes as far as it goes, then marks recompose.
    Recomposed,
    // Only a character the font lacks decomposes, and nothing recomposes.
    Unrecomposed,
}

// The hooks run in this order in a shape: `prepare` on characters; normalization's; `setup_masks` on characters with
// their glyphs found; the pauses the model planned between GSUB stages; `finish` once glyphs are placed.
pub(crate) trait Model: Sync + RefUnwindSafe {
    fn kind(&self) -> Shaper;

    // Features before the ones every run takes, with the GSUB stages and pauses they need.
    fn plan_features(&self, _planner: &mut Planner<'_>, _script_tag: Option<Tag>) {}

    // Features after the caller's, which they can override.
    fn plan_late_features(&self, _planner: &mut Planner<'_>) {}

    fn plan_data(&self, _features: &Features, _face: &Face, _script: Option<Script>, _tag: Option<Tag>)
        -> Option<ModelData> {
        None
    }

    fn mark_advances(&self) -> MarkAdvances {
        MarkAdvances::ZeroAfterGpos
    }

    // Whether the run takes the kern table and marks placed by Unicode class where the font has no other way.
    fn takes_fallbacks(&self) -> bool {
        true
    }

    // The GPOS script the model's run needs, GPOS not running under any other.
    fn gpos_script(&self) -> Option<Tag> {
        None
    }

    fn normalization(&self) -> Normalization {
        Normalization::Precomposed
    }

    fn decompose(&self, c: char) -> Option<(char, Option<char>)> {
        unicode::decompose(c)
    }

    // `marks_positioned`: the font positions marks, so a mark it would lose in a composed form is better kept.
    fn compose(&self, a: char, b: char, _marks_positioned: bool) -> Option<char> {
        unicode::compose(a, b)
    }

    // After canonical ordering, each run of marks; it may reorder within `marks` and join their clusters.
    fn reorder_marks(&self, _run: &mut Run, _marks: Range<usize>) {}

    fn prepare(&self, _run: &mut Run, _plan: &Plan, _face: &Face) {}

    fn setup_masks(&self, _run: &mut Run, _plan: &Plan, _face: &Face) {}

    fn finish(&self, _run: &mut Run, _plan: &Plan, _face: &Face) {}
}

/// The shaping model a run goes through: its script's own, the general one, or the one for text a font's morx
/// shapes.
// C numbers each by its place here, as `ALL` lists them, so a new one goes last in both.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Shaper {
    /// The general model, for a script with none of its own.
    #[default]
    Default,
    /// Where a font's morx shapes the text: no features, masks or reordering of its own.
    Dumber,
    /// Arabic's and Syriac's joining.
    Arabic,
    /// Hangul's syllables.
    Hangul,
    /// Hebrew's marks.
    Hebrew,
    /// The Indic scripts' syllables, Devanagari's and its kin's.
    Indic,
    /// Khmer's syllables.
    Khmer,
    /// Myanmar's syllables.
    Myanmar,
    /// Myanmar in the Zawgyi encoding, asked for with the language tag `qaag`, its characters left as given.
    MyanmarZawgyi,
    /// Thai's and Lao's marks.
    Thai,
    /// The Universal Shaping Engine's syllables, for the many scripts it covers.
    Universal,
}

impl Shaper {
    /// Its name in snake case: `"default"`, `"dumber"`, `"arabic"`, `"myanmar_zawgyi"` and so on.
    pub fn name(self) -> &'static str {
        self.c_name().to_str().unwrap_or_default()
    }

    // Its name as the C ABI lends it.
    pub(crate) fn c_name(self) -> &'static core::ffi::CStr {
        match self {
            Shaper::Default => c"default",
            Shaper::Dumber => c"dumber",
            Shaper::Arabic => c"arabic",
            Shaper::Hangul => c"hangul",
            Shaper::Hebrew => c"hebrew",
            Shaper::Indic => c"indic",
            Shaper::Khmer => c"khmer",
            Shaper::Myanmar => c"myanmar",
            Shaper::MyanmarZawgyi => c"myanmar_zawgyi",
            Shaper::Thai => c"thai",
            Shaper::Universal => c"universal",
        }
    }
}

// For a script with no model of its own.
pub(crate) struct General;

impl Model for General {
    fn kind(&self) -> Shaper {
        Shaper::Default
    }
}

// For text a font's morx shapes, which does what a script's model would: nothing of its own, marks left as morx
// leaves them.
pub(crate) struct Morx;

impl Model for Morx {
    fn kind(&self) -> Shaper {
        Shaper::Dumber
    }

    fn mark_advances(&self) -> MarkAdvances {
        MarkAdvances::Keep
    }

    fn takes_fallbacks(&self) -> bool {
        false
    }
}

pub(crate) const GENERAL: General = General;
pub(crate) const MORX: Morx = Morx;

// By `Shaper` code.
pub(crate) const ALL: [&dyn Model; 11] = [
    &GENERAL,
    &MORX,
    &arabic::MODEL,
    &hangul::MODEL,
    &hebrew::MODEL,
    &indic::MODEL,
    &khmer::MODEL,
    &myanmar::MODEL,
    &myanmar::ZAWGYI,
    &thai::MODEL,
    &universal::MODEL,
];

// Whether a syllabic model can read `c` as a sign of a syllable with no base, which takes a dotted circle for one.
pub(crate) fn may_lack_its_base(c: char) -> bool {
    indic_category::is_dependent(c as u32) || universal::is_dependent(c as u32)
}

// Where a script's model comes from: its own whatever the font, or one the font's GSUB script tag decides.
#[derive(Clone, Copy)]
enum Family {
    Own(&'static dyn Model),
    // Arabic's joining, horizontal only.
    Joining,
    Myanmar,
    // The nine scripts of the Indic spec.
    Indic,
}

// The scripts Microsoft's script-specific shaping specs cover, in Unicode's block order.
const FAMILIES: [(Script, Family); 17] = [
    (Script::HEBREW, Family::Own(&hebrew::MODEL)),
    (Script::ARABIC, Family::Joining),
    (Script::SYRIAC, Family::Joining),
    (Script::DEVANAGARI, Family::Indic),
    (Script::BENGALI, Family::Indic),
    (Script::GURMUKHI, Family::Indic),
    (Script::GUJARATI, Family::Indic),
    (Script::ORIYA, Family::Indic),
    (Script::TAMIL, Family::Indic),
    (Script::TELUGU, Family::Indic),
    (Script::KANNADA, Family::Indic),
    (Script::MALAYALAM, Family::Indic),
    (Script::THAI, Family::Own(&thai::MODEL)),
    (Script::LAO, Family::Own(&thai::MODEL)),
    (Script::MYANMAR, Family::Myanmar),
    (Script::HANGUL, Family::Own(&hangul::MODEL)),
    (Script::KHMER, Family::Own(&khmer::MODEL)),
];

// A run's model, from its script, the script tag GSUB chose for it and the first tag asked for. A font filing a
// script's lookups under DFLT or latn asks for the general model; under an Indic script's third version, the USE's.
pub(crate) fn select(
    script: Option<Script>,
    horizontal: bool,
    gsub: Option<Tag>,
    asked: Option<Tag>,
) -> &'static dyn Model {
    if asked.is_some_and(|t| t.to_bytes().eq_ignore_ascii_case(b"qaag")) {
        return &myanmar::ZAWGYI;
    }
    let Some(script) = script else { return &GENERAL };
    let filed = |name: &[u8; 4]| gsub == Some(Tag::new(name));
    let generic = filed(b"DFLT") || filed(b"latn");
    match FAMILIES.iter().find(|entry| entry.0 == script).map(|entry| entry.1) {
        Some(Family::Own(model)) => model,
        // Syriac filed under DFLT takes the general model; Arabic keeps its own.
        Some(Family::Joining) if horizontal && (script == Script::ARABIC || !filed(b"DFLT")) => &arabic::MODEL,
        Some(Family::Myanmar) if !generic && !filed(b"mymr") => &myanmar::MODEL,
        Some(Family::Indic) if !generic => match gsub.is_some_and(|t| t.to_bytes()[3] == b'3') {
            true => &universal::MODEL,
            false => &indic::MODEL,
        },
        None if !generic && USE_SCRIPTS.contains(&script) => &universal::MODEL,
        _ => &GENERAL,
    }
}
