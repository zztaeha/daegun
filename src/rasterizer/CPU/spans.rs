use super::coverage::{Level, ONE};

// A sum as a byte: its magnitude, at most a whole pixel, scaled to 255 and rounded.
#[inline]
pub(super) fn level(sum: i32) -> u8 {
    ((sum.unsigned_abs().min(ONE as u32) * 255 + (ONE as u32 >> 1)) >> 16) as u8
}

/// Reads a row's sums into coverage left to right, each added to the running one and cleared for the next row, as
/// is the one past it.
pub(super) fn coverage_row<T: Level>(acc: &mut [i32], out: &mut [T]) {
    let mut sum = 0i32;
    for (o, s) in out.iter_mut().zip(acc.iter_mut()) {
        sum = sum.wrapping_add(core::mem::take(s));
        *o = T::fixed(sum);
    }
    acc[out.len()] = 0;
}
