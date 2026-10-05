#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;

// Ties toward +infinity (fontTools' otRound) for gvar points, cvar deltas and MVAR; Rust's `round`
// goes away from zero and differs at interior locations. Half-to-even below is for CFF2 blend.
pub fn ot_round(v: f64) -> i32 {
    (v + 0.5).floor() as i32
}

// CFF2 charstring coordinates are relative, so rounding a blended delta to a whole unit
// accumulates error along the outline. The caller scales to 16.16 before rounding.
pub(crate) fn banker_round_i64(v: f64) -> i64 {
    v.round_ties_even() as i64
}

