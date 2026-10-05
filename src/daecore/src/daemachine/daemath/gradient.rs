use alloc::vec::Vec;
#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;
use super::matrix::{concat, invert};
use super::srgb::{from_linear, to_linear};
use super::{normalize_color_line, resolve_stops, Extend, Gradient, GradientKind, Rgba, Stop, Stops};

// How a color line blends between its stops. The OpenType spec asks for linear light with alpha
// premultiplied (CPAL, "Interpolation of colors"); Chrome and Cairo blend the stored sRGB, alpha straight.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Interpolation {
    #[default]
    LinearLight,
    Srgb,
}

// What a gradient paints at each pixel. Opaque, so its form can change without breaking a caller, and
// a varying one always holds the two or more stops `resolve_stops` leaves.
pub struct Ramp(Shape);

// Each interval's channels as bias + factor·t, in f32 as tiny-skia keeps them: premultiplied linear
// light and alpha in 0..1, or sRGB levels whose bias carries the half that rounds to the nearest one.
type Span = ([f32; 4], [f32; 4]);

enum Shape {
    Flat(Option<Rgba>),
    Varying {
        kind:    GradientKind,
        stops:   Vec<Stop>,
        spans:   Vec<Span>,
        how:     Interpolation,
        extend:  Extend,
        inverse: [f64; 6],
        linear:  [f64; 3],
    },
}

impl Ramp {
    // `to_device` takes the scene's space to pixels; the gradient's own transform is applied first.
    pub fn new(g: &Gradient, to_device: &[f64; 6]) -> Ramp {
        Ramp::with_interpolation(g, to_device, Interpolation::LinearLight)
    }

    pub fn with_interpolation(g: &Gradient, to_device: &[f64; 6], how: Interpolation) -> Ramp {
        let (kind, stops) = normalize_color_line(g.kind, g.stops.clone());
        let stops = match resolve_stops(stops) {
            Stops::Nothing => return Ramp(Shape::Flat(None)),
            Stops::Solid(c) => return Ramp(Shape::Flat(Some(c))),
            Stops::Many(s) => s,
        };
        let Some(inverse) = invert(&concat(&g.transform, to_device)) else { return Ramp(Shape::Flat(None)) };
        // COLR calls a degenerate linear ill-formed and has a radial with identical circles paint
        // nothing; a sweep with coincident angles draws nothing under reflect and repeat.
        let degenerate = match kind {
            GradientKind::Linear { x0, y0, x1, y1 } => x0 == x1 && y0 == y1,
            GradientKind::Radial { x0, y0, r0, x1, y1, r1 } => x0 == x1 && y0 == y1 && r0 == r1,
            GradientKind::Sweep { start_angle, end_angle, .. } => start_angle == end_angle,
        };
        if degenerate {
            return Ramp(Shape::Flat(None));
        }
        // A linear t is affine in the pixel, so the inverse, the pixel's center and the axis fold into
        // three numbers here, where `at` would take a matrix and a divide per pixel.
        let linear = match kind {
            GradientKind::Linear { x0, y0, x1, y1 } => {
                let (ax, ay) = (f64::from(x1 - x0), f64::from(y1 - y0));
                let len2 = ax * ax + ay * ay;
                let i = &inverse;
                let cx = 0.5 * (i[0] + i[2]) + i[4] - f64::from(x0);
                let cy = 0.5 * (i[1] + i[3]) + i[5] - f64::from(y0);
                [(i[0] * ax + i[1] * ay) / len2, (i[2] * ax + i[3] * ay) / len2, (cx * ax + cy * ay) / len2]
            }
            _ => [0.0; 3],
        };
        let spans = stops.windows(2).map(|w| span(w[0], w[1], how)).collect();
        Ramp(Shape::Varying { kind, stops, spans, how, extend: g.extend, inverse, linear })
    }

    // Per pixel, as are the helpers it calls: `#[inline]` lets a caller built without LTO inline them
    // rather than call across the crate for every pixel, which halves a sample's time.
    #[inline]
    pub fn at(&self, dx: f64, dy: f64) -> Option<Rgba> {
        let (kind, stops, spans, how, extend, inv, linear) = match &self.0 {
            Shape::Flat(c) => return *c,
            Shape::Varying { kind, stops, spans, how, extend, inverse, linear } => {
                (kind, stops, spans, *how, *extend, inverse, linear)
            }
        };
        let to_gradient = || {
            let (px, py) = (dx + 0.5, dy + 0.5);
            (inv[0] * px + inv[2] * py + inv[4], inv[1] * px + inv[3] * py + inv[5])
        };

        let t = match *kind {
            GradientKind::Linear { .. } => linear[0] * dx + linear[1] * dy + linear[2],
            GradientKind::Radial { x0, y0, r0, x1, y1, r1 } => {
                let (gx, gy) = to_gradient();
                radial_t(gx, gy, x0.into(), y0.into(), r0.into(), x1.into(), y1.into(), r1.into())?
            }
            GradientKind::Sweep { cx, cy, start_angle, end_angle } => {
                let (gx, gy) = to_gradient();
                sweep_t(gx, gy, cx.into(), cy.into(), start_angle.into(), end_angle.into())?
            }
        };
        if !t.is_finite() {
            return None;
        }
        Some(sample(stops, spans, how, apply_extend(t, extend)))
    }
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn radial_t(px: f64, py: f64, x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64) -> Option<f64> {
    let (cdx, cdy, dr) = (x1 - x0, y1 - y0, r1 - r0);
    let (pdx, pdy) = (px - x0, py - y0);
    let a = cdx * cdx + cdy * cdy - dr * dr;
    let b = pdx * cdx + pdy * cdy + r0 * dr;
    let c = pdx * pdx + pdy * pdy - r0 * r0;

    if a.abs() < 1e-12 {
        if b.abs() < 1e-12 {
            return None;
        }
        let s = c / (2.0 * b);
        return (r0 + s * dr >= 0.0).then_some(s);
    }
    let disc = b * b - a * c;
    if disc < 0.0 {
        return None;
    }
    let root = disc.sqrt();
    let (s1, s2) = ((b + root) / a, (b - root) / a);
    [s1.max(s2), s1.min(s2)].into_iter().find(|&s| r0 + s * dr >= 0.0)
}

// atan(x) / 2π on 0..1 as an odd polynomial, fitted on Chebyshev nodes: within 2.9e-7 of a turn.
// tiny-skia's four terms flip pixels on a hard edge, so six.
const TURN: [f64; 6] = [
    0.159_151_733_575_121_83,
    -0.052_943_764_420_488_81,
    0.030_823_588_126_902_994,
    -0.018_565_601_740_793_823,
    0.008_407_119_163_230_718,
    -0.001_873_333_226_746_811,
];

// The angle as a fraction of a turn from `TURN` in the smaller slope, twice as fast as atan2.
#[inline]
fn sweep_t(px: f64, py: f64, cx: f64, cy: f64, start: f64, end: f64) -> Option<f64> {
    let (x, y) = (px - cx, py - cy);
    let (ax, ay) = (x.abs(), y.abs());
    let slope = ax.min(ay) / ax.max(ay);
    let s = slope * slope;
    let mut turn = slope * TURN.iter().rev().fold(0.0, |acc, &c| acc * s + c);
    if ax < ay {
        turn = 0.25 - turn;
    }
    if x < 0.0 {
        turn = 0.5 - turn;
    }
    if y < 0.0 {
        turn = 1.0 - turn;
    }
    if turn.is_nan() {
        turn = 0.0;
    }
    let deg = turn * 360.0;
    let deg = if deg >= 360.0 { deg - 360.0 } else { deg };
    Some((deg - start) / (end - start))
}

#[inline]
fn apply_extend(t: f64, extend: Extend) -> f64 {
    match extend {
        Extend::Pad => t.clamp(0.0, 1.0),
        Extend::Repeat => t - t.floor(),
        Extend::Reflect => {
            let f = t - 2.0 * (t / 2.0).floor();
            if f > 1.0 { 2.0 - f } else { f }
        }
    }
}

// A hard edge, two stops at one offset, takes the later color.
fn span(a: Stop, b: Stop, how: Interpolation) -> Span {
    let (ca, cb) = (channels(a.color, how), channels(b.color, how));
    if b.offset > a.offset {
        let factor = [0, 1, 2, 3].map(|i| (cb[i] - ca[i]) / (b.offset - a.offset));
        (factor, [0, 1, 2, 3].map(|i| ca[i] - factor[i] * a.offset))
    } else {
        ([0.0; 4], cb)
    }
}

fn channels(c: Rgba, how: Interpolation) -> [f32; 4] {
    match how {
        Interpolation::LinearLight => {
            let a = f32::from(c.a) / 255.0;
            [to_linear(c.r) * a, to_linear(c.g) * a, to_linear(c.b) * a, a]
        }
        Interpolation::Srgb => [c.r, c.g, c.b, c.a].map(|v| f32::from(v) + 0.5),
    }
}

#[inline]
fn sample(stops: &[Stop], spans: &[Span], how: Interpolation, t: f64) -> Rgba {
    let first = stops[0];
    let last = stops[stops.len() - 1];
    if t <= f64::from(first.offset) {
        return first.color;
    }
    if t >= f64::from(last.offset) {
        return last.color;
    }
    for (w, &(factor, bias)) in stops.windows(2).zip(spans) {
        if t < f64::from(w[0].offset) || t > f64::from(w[1].offset) {
            continue;
        }
        let t = t as f32;
        let c = [0, 1, 2, 3].map(|i| bias[i] + factor[i] * t);
        if how == Interpolation::Srgb {
            let c = c.map(|v| v.clamp(0.0, 255.0) as u8);
            return Rgba { r: c[0], g: c[1], b: c[2], a: c[3] };
        }
        // Un-premultiplied by the alpha the interval reached; where that is none, so is the color. An
        // opaque interval keeps its alpha at exactly 1, so it skips the division.
        let a = c[3].clamp(0.0, 1.0);
        if a <= 0.0 {
            return Rgba { r: 0, g: 0, b: 0, a: 0 };
        }
        let c = if a < 1.0 { c.map(|v| v * (1.0 / a)) } else { c };
        let alpha = (a * 255.0 + 0.5) as u8;
        return Rgba { r: from_linear(c[0]), g: from_linear(c[1]), b: from_linear(c[2]), a: alpha };
    }
    last.color
}
