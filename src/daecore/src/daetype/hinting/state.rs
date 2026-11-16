use alloc::vec::Vec;
use super::f26dot6::{self, F2DOT14_ONE, HALF, ONE};

/// Which hinter fits a glyph to the pixel grid, if any.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum HintMode {
    /// No hinting.
    #[default]
    None,
    /// The font's own hints: its TrueType bytecode, or its CFF or CFF2 hints. Bytecode fits y to the grid, and x
    /// only in a font that declares native ClearType (INSTCTRL selector 3); otherwise x stays as drawn.
    Font,
    /// Chosen once for the font: the autohinter for TrueType outlines with no font program and a CVT program of at
    /// most 7 bytes, else the font's own bytecode or CFF hints, a glyph they don't hint left unhinted.
    Auto,
    /// The autohinter for every glyph.
    AutoForce,
}

impl HintMode {
    pub(crate) fn runs_bytecode(self) -> bool {
        matches!(self, HintMode::Font | HintMode::Auto)
    }

    /// Whether the autohinter may take a glyph under this mode.
    pub const fn may_autohint(self) -> bool {
        matches!(self, HintMode::Auto | HintMode::AutoForce)
    }
}

// A unit vector, each component in 2.14.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Vector {
    pub x: i32,
    pub y: i32,
}

impl Vector {
    pub(crate) const X_AXIS: Vector = Vector { x: F2DOT14_ONE, y: 0 };
    pub(crate) const Y_AXIS: Vector = Vector { x: 0, y: F2DOT14_ONE };

    // The direction of (dx, dy), or turned a quarter counterclockwise. With no direction to take, the graphics state's
    // default, the x-axis.
    pub(crate) fn toward(dx: i64, dy: i64, turned: bool) -> Vector {
        let (dx, dy) = if turned { (dy.saturating_neg(), dx) } else { (dx, dy) };
        f26dot6::unit_vector(dx, dy).map_or(Vector::X_AXIS, |(x, y)| Vector { x, y })
    }
}

// Spaced `period` apart and offset by `phase`, a distance falling to the lower value until `threshold` past it: SROUND
// and S45ROUND's round state, in 26.6.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Lattice {
    pub period: i32,
    pub phase: i32,
    pub threshold: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Round {
    Grid,
    HalfGrid,
    DoubleGrid,
    DownToGrid,
    UpToGrid,
    Off,
    Lattice(Lattice),
}

impl Round {
    // SROUND's or S45ROUND's selector over a grid period of `grid` in 2.14 (a pixel, or half the square root of 2).
    // Worked in 2.14 and cut to 26.6, so the 45° period keeps its fraction until the last step.
    pub(crate) fn lattice(selector: i32, grid: i32) -> Round {
        let period = match (selector >> 6) & 3 {
            0 => grid / 2,
            2 => grid * 2,
            _ => grid,
        };
        let phase = period * ((selector >> 4) & 3) / 4;
        let fraction = selector & 0xF;
        // Selector 0 is the largest value under the period, which rounds up to it.
        let threshold = if fraction == 0 { (period >> 8) - 1 } else { ((fraction - 4) * period / 8) >> 8 };
        Round::Lattice(Lattice { period: period >> 8, phase: phase >> 8, threshold })
    }

    // A distance rounded by the state. The magnitude is rounded and the sign kept, and a value pushed past 0 takes the
    // round value nearest 0 on its own side, as the instruction set's SROUND steps say.
    pub(crate) fn apply(self, d: i32) -> i32 {
        let (rounded, nearest) = match self {
            Round::Off => return d,
            Round::Grid => (f26dot6::round_pixel(d.saturating_abs()), 0),
            Round::HalfGrid => (f26dot6::floor_pixel(d.saturating_abs()).saturating_add(HALF), HALF),
            Round::DoubleGrid => (d.saturating_abs().saturating_add(ONE / 4) & -HALF, 0),
            Round::DownToGrid => (f26dot6::floor_pixel(d.saturating_abs()), 0),
            Round::UpToGrid => (f26dot6::ceil_pixel(d.saturating_abs()), 0),
            Round::Lattice(l) => {
                let (period, phase) = (i64::from(l.period), i64::from(l.phase));
                let shifted = i64::from(d.saturating_abs()) + i64::from(l.threshold) - phase;
                let mut snapped = shifted.div_euclid(period) * period + phase;
                // Past i32, the last lattice value it holds.
                let over = snapped - i64::from(i32::MAX);
                if over > 0 {
                    snapped -= (over + period - 1) / period * period;
                }
                (snapped as i32, l.phase)
            }
        };
        let magnitude = if rounded < 0 { nearest } else { rounded };
        if d < 0 { -magnitude } else { magnitude }
    }
}

// The graphics state of the instruction set's graphics state chapter, one value per variable.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GraphicsState {
    pub projection: Vector,
    pub dual: Vector,
    pub freedom: Vector,
    // rp0, rp1 and rp2, and zp0, zp1 and zp2 (0 the twilight zone, 1 the glyph).
    pub rp: [u32; 3],
    pub zp: [usize; 3],
    pub round: Round,
    pub repeat: u32,
    pub minimum_distance: i32,
    pub cut_in: i32,
    pub single_width: i32,
    pub single_width_cut_in: i32,
    pub delta_base: u32,
    pub delta_shift: u32,
    pub auto_flip: bool,
}

impl Default for GraphicsState {
    // The chapter's defaults: the x-axis, zone 1, rounding to the grid, a pixel's minimum distance, a cut-in of 17/16.
    fn default() -> Self {
        GraphicsState {
            projection: Vector::X_AXIS,
            dual: Vector::X_AXIS,
            freedom: Vector::X_AXIS,
            rp: [0; 3],
            zp: [1; 3],
            round: Round::Grid,
            repeat: 1,
            minimum_distance: ONE,
            cut_in: 17 * ONE / 16,
            single_width: 0,
            single_width_cut_in: 0,
            delta_base: 9,
            delta_shift: 3,
            auto_flip: true,
        }
    }
}

impl GraphicsState {
    // What each glyph program starts from after the CVT program: its sizing policy (distances, cut-ins, deltas,
    // auto flip) carries, and where a program works (vectors, references, zones, loop, rounding) starts fresh.
    pub(crate) fn for_glyphs(&self) -> GraphicsState {
        GraphicsState {
            minimum_distance: self.minimum_distance,
            cut_in: self.cut_in,
            single_width: self.single_width,
            single_width_cut_in: self.single_width_cut_in,
            delta_base: self.delta_base,
            delta_shift: self.delta_shift,
            auto_flip: self.auto_flip,
            ..GraphicsState::default()
        }
    }
}

// One coordinate per point along each axis.
#[derive(Default, Clone, Debug)]
pub(crate) struct Points {
    pub x: Vec<i32>,
    pub y: Vec<i32>,
}

impl Points {
    pub(crate) fn clear(&mut self) {
        self.x.clear();
        self.y.clear();
    }

    pub(crate) fn assign(&mut self, from: &Points) {
        self.x.clone_from(&from.x);
        self.y.clone_from(&from.y);
    }

    fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        heap_of(&self.x) + heap_of(&self.y)
    }
}

// A zone's points: where they are now, where scaling put them, and where the font has them. For a glyph `unscaled` is
// in font units, for a composite's own program the assembled 26.6 points; the twilight zone has none.
#[derive(Default, Debug)]
pub(crate) struct Zone {
    pub current: Points,
    pub original: Points,
    pub unscaled: Points,
    pub flags: Vec<u8>,
    pub ends: Vec<usize>,
}

impl Clone for Zone {
    fn clone(&self) -> Zone {
        let mut zone = Zone::default();
        zone.clone_from(self);
        zone
    }

    // Into the buffers already held: the twilight zone is put back after each glyph program that wrote it.
    fn clone_from(&mut self, from: &Zone) {
        self.current.assign(&from.current);
        self.original.assign(&from.original);
        self.unscaled.assign(&from.unscaled);
        self.flags.clone_from(&from.flags);
        self.ends.clone_from(&from.ends);
    }
}

pub const FLAG_ON_CURVE: u8 = 0x01;
// A cubic control point in a hinted CFF outline or a GLYF one; every other control is a quadratic one.
pub const FLAG_CUBIC: u8 = 0x02;
pub(crate) const FLAG_TOUCHED_X: u8 = 0x08;
pub(crate) const FLAG_TOUCHED_Y: u8 = 0x10;

impl Zone {
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        self.current.heap_bytes() + self.original.heap_bytes() + self.unscaled.heap_bytes() + heap_of(&self.flags)
            + heap_of(&self.ends)
    }

    // The twilight zone of `n` points, all at the origin, in the buffers the zone holds.
    pub(crate) fn reset_twilight(&mut self, n: usize) {
        for points in [&mut self.current, &mut self.original] {
            zeroed::<i32, 16>(&mut points.x, n);
            zeroed::<i32, 16>(&mut points.y, n);
        }
        zeroed::<u8, 64>(&mut self.flags, n);
        self.unscaled.clear();
        self.ends.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.current.x.len()
    }
}

// `v` as `n` zeros. Up to a 64-byte block, as a twilight zone usually is, the block's fixed-size store is inlined,
// where a memset of that length is a library call that costs more than the zeroing.
fn zeroed<T: Copy + Default, const BLOCK: usize>(v: &mut Vec<T>, n: usize) {
    v.clear();
    if n <= BLOCK {
        v.extend_from_slice(&[T::default(); BLOCK]);
        v.truncate(n);
    } else {
        v.resize(n, T::default());
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/hinting/state.rs"]
mod tests;
