#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;

/// Rounds as the OpenType spec does, ties toward positive infinity, as gvar points and cvar deltas are compiled.
// Rust's `round` goes away from zero and differs at interior locations.
pub fn ot_round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

// The same for what is computed in f32, outline points and anchors, widened first so the half is added in double: a
// value just under a half rounds down, where f32 would round it up.
pub(crate) fn ot_round_f32(v: f32) -> i32 {
    ot_round(f64::from(v))
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/round.rs"]
mod tests;
