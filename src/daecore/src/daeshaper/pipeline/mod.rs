// One shape of a run, stage by stage. The stages read and write only the run; the plan says which run.
mod chars;
mod finish;
mod layout;
mod position;

use super::face::Face;
use super::fallback;
use super::normalize;
use super::plan::{FeatureSetting, Plan};
use super::run::{Direction, Run};
use super::unicode;

// Shapes `run` in `direction`, `features` giving the ranges of the plan's ranged features: a cached plan stands for
// every call whose features differ only in where they apply.
pub(crate) fn shape(face: &Face, plan: &Plan, run: &mut Run, direction: Direction, features: &[FeatureSetting]) {
    run.begin();
    run.direction = direction;

    chars::read_chars(run, plan.features.global());
    chars::circle_before_leading_mark(run, face);
    chars::form_graphemes(run);
    chars::turn_to_native_direction(run);
    plan.model.prepare(run, plan, face);
    chars::mirror_and_rotate(face, plan, run, direction);
    normalize::normalize(run, face, plan.model, plan.marks_positioned);
    set_masks(face, plan, run, features);
    if plan.positioning.fallback_marks {
        fallback::recategorize_marks(run);
    }

    let digest = layout::map_glyphs(face, plan, run);
    let digest = layout::substitute(face, plan, run, digest);
    position::position(face, plan, run, digest);

    finish::drop_deletions(plan, run);
    finish::stand_in_for_selectors(run);
    finish::settle_ignorables(face, run);
    plan.model.finish(run, plan, face);
    run.settle_flags();

    run.direction = direction;
    run.end();
}

fn set_masks(face: &Face, plan: &Plan, run: &mut Run, features: &[FeatureSetting]) {
    chars::fraction_masks(plan, run);
    plan.model.setup_masks(run, plan, face);
    for setting in features.iter().filter(|f| !f.covers_all()) {
        let (mask, shift) = (plan.features.mask(setting.tag), plan.features.shift(setting.tag));
        run.mask_clusters(setting.start..setting.end, mask, setting.value << shift);
    }
}

// The run's script, where none is given, from its first character of a script of its own, and that script's
// horizontal direction, left to right for none.
pub(crate) fn settle_script(run: &mut Run) -> Direction {
    if run.script.is_none() {
        let mut scripts = run.glyphs[..run.len].iter().filter_map(|g| char::from_u32(g.id).map(unicode::script));
        run.script = scripts.find(|s| !s.is_context_dependent());
    }
    run.script.map_or(Direction::LeftToRight, unicode::horizontal_direction)
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/pipeline/mod.rs"]
mod tests;
