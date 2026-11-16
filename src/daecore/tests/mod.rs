// The core's own suites, mounted in the crate since what they test is private to it.
#[path = "fingerprint/main.rs"]
mod fingerprint;
#[path = "kit/main.rs"]
mod daekit;
#[path = "shaper/main.rs"]
mod daeshaper;
#[path = "type/main.rs"]
mod daetype;

use alloc::vec::Vec;
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::plan::{Plan, PlanRequest};
use crate::daecore::daeshaper::run::{Direction, Glyph, Placement, Run};
use crate::daecore::daeshaper::unicode::Script;

// A plan with no features, language or lookup switches asked for, as the suites shape plain text.
pub(crate) fn plain_plan(face: &Face, script: Option<Script>, direction: Direction, script_tags: &[Tag]) -> Plan {
    let request = PlanRequest {
        script,
        direction,
        script_tags,
        language_tags: &[],
        language: None,
        features: &[],
        switches: &[],
    };
    Plan::new(face, &request)
}

// Each glyph of a shaped run with its placement, as the suites read a run.
pub(crate) fn placed(run: &Run) -> Vec<(Glyph, Placement)> {
    run.glyphs[..run.len].iter().copied().zip(run.places[..run.len].iter().copied()).collect()
}
