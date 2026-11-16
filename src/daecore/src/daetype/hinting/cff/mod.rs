// CFF and CFF2 hints by Adobe's model (Type 1 Font Format, chapters 5, 6.5 and 8.1; TN 5177, 4.3): y alone is
// hinted, stems aligned to the Private DICT's zones or moved onto the pixel grid, each hint set a ladder points map by.
mod dict;
pub(crate) mod hinter;
mod ladder;
mod outline;
mod placement;
mod stems;
mod zones;

pub(crate) use dict::{Private, PrivateDict};
pub(crate) use hinter::Hinter;
pub(crate) use ladder::Scale;
pub(crate) use outline::Outline;
pub(crate) use placement::{FontMatrix, Placement};
pub(crate) use stems::MAX_STEMS;
pub(crate) use zones::{EmBox, Zones};

// 16.16 values: a font unit in glyph space, a pixel in device space.
const ONE: i64 = 1 << 16;
const HALF: i64 = ONE / 2;

// v / 2^shift to the nearest, a half up.
fn round_shift(v: i128, shift: u32) -> i128 {
    (v + (1 << (shift - 1))) >> shift
}

// n / d to the nearest for a d above 0, a half up.
fn div_round(n: i128, d: i128) -> i128 {
    (n + d / 2).div_euclid(d)
}

fn saturate(v: i128) -> i64 {
    v.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}
