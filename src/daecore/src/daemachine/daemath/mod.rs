use alloc::vec::Vec;
#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;

pub mod blend;
pub mod gradient;
pub mod matrix;
pub mod srgb;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub fn opaque(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 255 }
    }

    pub fn fade(self, by: f64) -> Rgba {
        if !by.is_finite() {
            return self;
        }
        Rgba { a: (f64::from(self.a) * by.clamp(0.0, 1.0)).round() as u8, ..self }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Extend {
    #[default]
    Pad,
    Reflect,
    Repeat,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Stop {
    pub offset: f32,
    pub color: Rgba,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GradientKind {
    Linear { x0: f32, y0: f32, x1: f32, y1: f32 },
    Radial { x0: f32, y0: f32, r0: f32, x1: f32, y1: f32, r1: f32 },
    Sweep { cx: f32, cy: f32, start_angle: f32, end_angle: f32 },
}

#[derive(Clone, PartialEq, Debug)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<Stop>,
    pub extend: Extend,
    pub transform: [f64; 6],
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Blend {
    Clear,
    Src,
    Dest,
    #[default]
    SrcOver,
    DestOver,
    SrcIn,
    DestIn,
    SrcOut,
    DestOut,
    SrcAtop,
    DestAtop,
    Xor,
    Plus,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Multiply,
    HslHue,
    HslSaturation,
    HslColor,
    HslLuminosity,
}

impl Blend {
    pub fn from_colr(mode: u8) -> Blend {
        use Blend::*;
        const MODES: [Blend; 28] = [
            Clear, Src, Dest, SrcOver, DestOver, SrcIn, DestIn, SrcOut, DestOut, SrcAtop, DestAtop,
            Xor, Plus, Screen, Overlay, Darken, Lighten, ColorDodge, ColorBurn, HardLight, SoftLight,
            Difference, Exclusion, Multiply, HslHue, HslSaturation, HslColor, HslLuminosity,
        ];
        // The spec: "If an unrecognized value is encountered, COMPOSITE_CLEAR must be used."
        MODES.get(mode as usize).copied().unwrap_or(Clear)
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Stops {
    Nothing,
    Solid(Rgba),
    Many(Vec<Stop>),
}

// COLR defines a color line over its first to last stop, which may lie past or within 0..1, and extends
// it from there. The geometry moves to those stops instead, so they span 0..1 and nothing is clamped.
pub(crate) fn normalize_color_line(kind: GradientKind, mut stops: Vec<Stop>) -> (GradientKind, Vec<Stop>) {
    let offsets = stops.iter().map(|s| s.offset).filter(|o| o.is_finite());
    let (first, last) = offsets.fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), o| (a.min(o), b.max(o)));
    if first > last || (first == last && (0.0..=1.0).contains(&first)) || (first == 0.0 && last == 1.0) {
        return (kind, stops);
    }
    // A hard edge, every stop at one offset, has no span to move to: it keeps a length of 1 from there.
    let (a, b) = (f64::from(first), f64::from(if first == last { first + 1.0 } else { last }));
    let at = |p: f32, q: f32, t: f64| (f64::from(p) + t * (f64::from(q) - f64::from(p))) as f32;
    let kind = match kind {
        GradientKind::Linear { x0, y0, x1, y1 } => {
            GradientKind::Linear { x0: at(x0, x1, a), y0: at(y0, y1, a), x1: at(x0, x1, b), y1: at(y0, y1, b) }
        }
        GradientKind::Radial { x0, y0, r0, x1, y1, r1 } => GradientKind::Radial {
            x0: at(x0, x1, a), y0: at(y0, y1, a), r0: at(r0, r1, a),
            x1: at(x0, x1, b), y1: at(y0, y1, b), r1: at(r0, r1, b),
        },
        GradientKind::Sweep { cx, cy, start_angle, end_angle } => GradientKind::Sweep {
            cx, cy, start_angle: at(start_angle, end_angle, a), end_angle: at(start_angle, end_angle, b),
        },
    };
    for s in &mut stops {
        s.offset = ((f64::from(s.offset) - a) / (b - a)) as f32;
    }
    (kind, stops)
}

pub fn resolve_stops(mut stops: Vec<Stop>) -> Stops {
    match stops.len() {
        0 => return Stops::Nothing,
        1 => return Stops::Solid(stops[0].color),
        _ => {}
    }
    for s in &mut stops {
        s.offset = if s.offset.is_nan() { 0.0 } else { s.offset.clamp(0.0, 1.0) };
    }
    stops.sort_by(|a, b| a.offset.partial_cmp(&b.offset).unwrap_or(core::cmp::Ordering::Equal));
    for i in 1..stops.len() {
        if stops[i].offset < stops[i - 1].offset {
            stops[i].offset = stops[i - 1].offset;
        }
    }
    Stops::Many(stops)
}
