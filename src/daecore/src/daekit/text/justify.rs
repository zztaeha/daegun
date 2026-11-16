use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use alloc::vec::Vec;

use crate::daecore::daetype::jstf::JstfModLists;
use crate::daecore::daekit::cache::{AxisKey, FontCache};
use crate::daecore::daekit::text::shape::ShapedRun;
use crate::daecore::Tag;

/// A line justified to a width: the run as shaped at the JSTF level that came nearest, in font units.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Justified {
    /// The line as shaped.
    pub run: Arc<ShapedRun>,
    /// The JSTF priority level it was shaped at, by index; none where the natural width was kept.
    pub level: Option<usize>,
    /// Which way the line had to go: [`Adjustment::Shrink`] where its natural width is past the target by more than
    /// the tolerance, else [`Adjustment::Extend`].
    pub adjustment: Adjustment,
    /// Its width.
    pub width: f64,
    /// Whether no level came within the tolerance, so this is only the nearest.
    pub best_effort: bool,
}

/// Which way a JSTF level adjusts a line.
#[expect(clippy::exhaustive_enums, reason = "a line shrinks or extends")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Adjustment {
    /// Narrower, by the level's shrinkage lookups.
    Shrink,
    /// Wider, by its extension lookups.
    Extend,
}

/// What to justify a line to, and with which script's JSTF data. Built with [`JustifyOptions::new`], as a script and
/// a width have no default to stand in for them.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JustifyOptions {
    /// The OpenType script whose JSTF priorities apply; one JSTF doesn't list gives no levels, so the line keeps its
    /// natural shaping.
    pub script: Tag,
    /// The OpenType language system whose priorities apply, none for the script's default.
    pub language_system: Option<Tag>,
    /// The width to reach, in font units.
    pub target_width: f64,
    /// How far from the target a width may be and still count as reaching it.
    pub tolerance: f64,
}

impl JustifyOptions {
    /// Justifying to a width with a script's default priorities, exactly.
    #[must_use]
    pub const fn new(script: Tag, target_width: f64) -> JustifyOptions {
        JustifyOptions { script, language_system: None, target_width, tolerance: 0.0 }
    }

    /// These options with a language system's priorities.
    #[must_use]
    #[doc(alias("with_language"))]
    pub const fn with_language_system(mut self, language_system: Tag) -> JustifyOptions {
        self.language_system = Some(language_system);
        self
    }

    /// These options with a tolerance.
    #[must_use]
    pub const fn with_tolerance(mut self, tolerance: f64) -> JustifyOptions {
        self.tolerance = tolerance;
        self
    }
}

// The natural run where it is within the tolerance, else each level of `jstf`'s script and language system in turn
// until one is, or the nearest: the levels are read only past the natural width.
pub(crate) fn justify_line(
    fc: &FontCache,
    axes: &AxisKey,
    text: &str,
    vertical: bool,
    jstf: (Tag, Option<Tag>),
    (target_width, tolerance): (f64, f64),
) -> Justified {
    let natural = fc.shaped_run(axes, text, vertical);
    let natural_width = natural.width();

    if (natural_width - target_width).abs() <= tolerance {
        let adjustment = Adjustment::Extend;
        return Justified { run: natural, level: None, adjustment, width: natural_width, best_effort: false };
    }

    let shrink = natural_width > target_width;
    let adjustment = if shrink { Adjustment::Shrink } else { Adjustment::Extend };
    let mut best_error = (natural_width - target_width).abs();
    let mut best = Justified { run: natural, level: None, adjustment, width: natural_width, best_effort: true };

    let levels = levels(fc, jstf.0, jstf.1).unwrap_or_default();
    // A level naming the lists an earlier one did shapes the line as it did, so each set of lists shapes it once.
    let mut tried = BTreeSet::new();
    for (i, mods) in levels.iter().enumerate() {
        let lists = match shrink {
            true => [&mods.shrinkage_enable_gsub, &mods.shrinkage_disable_gsub],
            false => [&mods.extension_enable_gsub, &mods.extension_disable_gsub],
        };
        let positions = match shrink {
            true => [&mods.shrinkage_enable_gpos, &mods.shrinkage_disable_gpos],
            false => [&mods.extension_enable_gpos, &mods.extension_disable_gpos],
        };
        if !tried.insert((lists, positions)) {
            continue;
        }
        let run = fc.shaped_run_justified(axes, text, vertical, mods, shrink);
        let w = run.width();
        let error = (w - target_width).abs();
        if error <= tolerance {
            return Justified { run, level: Some(i), adjustment, width: w, best_effort: false };
        }
        if error < best_error {
            best_error = error;
            best = Justified { run, level: Some(i), adjustment, width: w, best_effort: true };
        }
    }

    best
}

// What a script's JSTF gives for a language system, its default where none is named or the language has none.
pub(crate) fn levels(fc: &FontCache, script: Tag, language: Option<Tag>) -> Option<Vec<JstfModLists>> {
    let table_map = fc.tables();
    crate::daecore::daetype::jstf::jstf_priorities(table_map, script.to_bytes(), language.map(Tag::to_bytes))
}
