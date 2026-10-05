use alloc::vec::Vec;
use super::f26dot6::{self, F2DOT14_ONE, ONE};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum HintMode {
    #[default]
    None,
    Subpixel,
    Classic,
    Auto,
    AutoForce,
}

impl HintMode {
    // Classic runs bytecode as FreeType's v35 interpreter does; Subpixel and Auto as its v40, which
    // keeps a font that has not opted out from moving points along x.
    pub(crate) fn is_v40(self) -> bool {
        !matches!(self, HintMode::Classic)
    }

    pub(crate) fn runs_bytecode(self) -> bool {
        matches!(self, HintMode::Subpixel | HintMode::Classic | HintMode::Auto)
    }

    pub fn may_autohint(self) -> bool {
        matches!(self, HintMode::Auto | HintMode::AutoForce)
    }
}

// A 2.14 unit vector.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Vector {
    pub x: i32,
    pub y: i32,
}

impl Vector {
    pub(crate) const X_AXIS: Vector = Vector { x: F2DOT14_ONE, y: 0 };
    pub(crate) const Y_AXIS: Vector = Vector { x: 0, y: F2DOT14_ONE };
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RoundState {
    ToGrid,
    ToHalfGrid,
    ToDoubleGrid,
    DownToGrid,
    UpToGrid,
    Off,
    Super { period: i32, phase: i32, threshold: i32 },
    Super45 { period: i32, phase: i32, threshold: i32 },
}

impl RoundState {
    // SROUND and S45ROUND's selector: period, phase and threshold, worked in 2.14 and kept in 26.6.
    pub(crate) fn sround(selector: i32, grid_period: i32, gray45: bool) -> RoundState {
        let period = match selector & 0xC0 {
            0x00 => grid_period / 2,
            0x80 => grid_period * 2,
            _ => grid_period,
        };
        let phase = match selector & 0x30 {
            0x00 => 0,
            0x10 => period / 4,
            0x20 => period / 2,
            _ => period * 3 / 4,
        };
        let threshold = match selector & 0x0F {
            0 => period - 1,
            t => (t - 4) * period / 8,
        };
        let (period, phase, threshold) = (period >> 8, phase >> 8, threshold >> 8);
        if gray45 {
            RoundState::Super45 { period, phase, threshold }
        } else {
            RoundState::Super { period, phase, threshold }
        }
    }

    // A distance rounded away from or toward zero by magnitude, as FreeType's Round_* functions do
    // with no engine compensation; a positive distance never rounds negative or the reverse.
    pub(crate) fn apply(self, d: i32) -> i32 {
        let magnitude = |f: &dyn Fn(i32) -> i32| {
            if d >= 0 { f(d).max(0) } else { f(d.saturating_neg()).saturating_neg().min(0) }
        };
        match self {
            RoundState::Off => d,
            RoundState::ToGrid => magnitude(&|v| v.saturating_add(ONE / 2) & !(ONE - 1)),
            RoundState::ToHalfGrid => {
                if d >= 0 {
                    f26dot6::floor_pixel(d).saturating_add(ONE / 2)
                } else {
                    f26dot6::floor_pixel(d.saturating_neg()).saturating_add(ONE / 2).saturating_neg()
                }
            }
            RoundState::ToDoubleGrid => magnitude(&|v| v.saturating_add(ONE / 4) & !(ONE / 2 - 1)),
            RoundState::DownToGrid => magnitude(&f26dot6::floor_pixel),
            RoundState::UpToGrid => magnitude(&f26dot6::ceil_pixel),
            RoundState::Super { period, phase, threshold } => {
                super_round(d, phase, |v| v.saturating_add(threshold - phase) & period.saturating_neg())
            }
            RoundState::Super45 { period, phase, threshold } => {
                super_round(d, phase, |v| v.saturating_add(threshold - phase) / period.max(1) * period)
            }
        }
    }
}

fn super_round(d: i32, phase: i32, snap: impl Fn(i32) -> i32) -> i32 {
    if d >= 0 {
        let v = snap(d).saturating_add(phase);
        if v < 0 { phase } else { v }
    } else {
        let v = snap(d.saturating_neg()).saturating_neg().saturating_sub(phase);
        if v > 0 { phase.saturating_neg() } else { v }
    }
}

#[derive(Clone)]
pub(crate) struct GraphicsState {
    pub projection: Vector,
    pub dual_projection: Vector,
    pub freedom: Vector,
    pub rp0: usize,
    pub rp1: usize,
    pub rp2: usize,
    pub zp0: usize,
    pub zp1: usize,
    pub zp2: usize,
    pub round_state: RoundState,
    pub loop_count: i32,
    pub minimum_distance: i32,
    pub control_value_cut_in: i32,
    pub single_width_cut_in: i32,
    pub single_width_value: i32,
    pub delta_base: i32,
    pub delta_shift: i32,
    pub auto_flip: bool,
    pub instruct_control: u8,
}

impl Default for GraphicsState {
    fn default() -> Self {
        GraphicsState {
            projection: Vector::X_AXIS,
            dual_projection: Vector::X_AXIS,
            freedom: Vector::X_AXIS,
            rp0: 0,
            rp1: 0,
            rp2: 0,
            zp0: 1,
            zp1: 1,
            zp2: 1,
            round_state: RoundState::ToGrid,
            loop_count: 1,
            minimum_distance: ONE,
            control_value_cut_in: 68,
            single_width_cut_in: 0,
            single_width_value: 0,
            delta_base: 9,
            delta_shift: 3,
            auto_flip: true,
            instruct_control: 0,
        }
    }
}

impl GraphicsState {
    // What every glyph program starts from: the defaults, with the fields the CVT program may set,
    // the ones FreeType carries over from it.
    pub(crate) fn retained(&self) -> GraphicsState {
        GraphicsState {
            minimum_distance: self.minimum_distance,
            control_value_cut_in: self.control_value_cut_in,
            single_width_cut_in: self.single_width_cut_in,
            single_width_value: self.single_width_value,
            delta_base: self.delta_base,
            delta_shift: self.delta_shift,
            auto_flip: self.auto_flip,
            instruct_control: self.instruct_control,
            ..GraphicsState::default()
        }
    }
}

// Points current, original and unscaled. For a glyph `orus` is in font units, for the composite
// pass the assembled 26.6 points; the twilight zone has none.
#[derive(Default)]
pub(crate) struct Zone {
    pub cur_x: Vec<i32>,
    pub cur_y: Vec<i32>,
    pub org_x: Vec<i32>,
    pub org_y: Vec<i32>,
    pub orus_x: Vec<i32>,
    pub orus_y: Vec<i32>,
    pub flags: Vec<u8>,
    pub contour_ends: Vec<usize>,
}

impl Clone for Zone {
    fn clone(&self) -> Zone {
        let mut zone = Zone::default();
        zone.clone_from(self);
        zone
    }

    // Field by field into the buffers already held: the twilight zone is restored before every
    // glyph, and the derived form allocated all eight vectors each time.
    fn clone_from(&mut self, source: &Zone) {
        let Zone { cur_x, cur_y, org_x, org_y, orus_x, orus_y, flags, contour_ends } = source;
        self.cur_x.clone_from(cur_x);
        self.cur_y.clone_from(cur_y);
        self.org_x.clone_from(org_x);
        self.org_y.clone_from(org_y);
        self.orus_x.clone_from(orus_x);
        self.orus_y.clone_from(orus_y);
        self.flags.clone_from(flags);
        self.contour_ends.clone_from(contour_ends);
    }
}

pub const FLAG_ON_CURVE: u8 = 0x01;
// A cubic control point in a hinted CFF outline, FreeType's FT_CURVE_TAG_CUBIC; every other control
// is a quadratic one.
pub const FLAG_CUBIC: u8 = 0x02;
pub(crate) const FLAG_TOUCHED_X: u8 = 0x08;
pub(crate) const FLAG_TOUCHED_Y: u8 = 0x10;

impl Zone {
    pub(crate) fn with_len(n: usize) -> Zone {
        let mut z = Zone::default();
        z.reset(n);
        z
    }

    pub(crate) fn reset(&mut self, n: usize) {
        for v in [&mut self.cur_x, &mut self.cur_y, &mut self.org_x, &mut self.org_y, &mut self.orus_x, &mut self.orus_y] {
            v.clear();
            v.resize(n, 0);
        }
        self.flags.clear();
        self.flags.resize(n, 0);
        self.contour_ends.clear();
    }

    pub(crate) fn len(&self) -> usize {
        self.cur_x.len()
    }
}
