use alloc::vec::Vec;
#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;

pub mod atlas;
pub mod bitmap;
pub mod blend;
pub mod gradient;
pub mod matrix;
pub mod srgb;
pub mod subpixel;

/// An sRGB color with straight, not premultiplied, alpha. Laid out as C reads it.
#[expect(clippy::exhaustive_structs, reason = "a plain value: its fields are all it is")]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Rgba {
    /// Red, 0 to 255.
    pub r: u8,
    /// Green, 0 to 255.
    pub g: u8,
    /// Blue, 0 to 255.
    pub b: u8,
    /// Alpha, 0 transparent to 255 opaque.
    pub a: u8,
}

/// The bytes red, green, blue and alpha, as an RGBA pixel holds them.
impl From<[u8; 4]> for Rgba {
    fn from([r, g, b, a]: [u8; 4]) -> Rgba {
        Rgba { r, g, b, a }
    }
}

/// Red, green, blue and alpha, as an RGBA pixel holds them.
impl From<Rgba> for [u8; 4] {
    fn from(color: Rgba) -> [u8; 4] {
        [color.r, color.g, color.b, color.a]
    }
}

impl Rgba {
    /// An opaque color.
    #[must_use]
    pub const fn opaque(r: u8, g: u8, b: u8) -> Rgba {
        Rgba { r, g, b, a: 255 }
    }

    /// The color with its alpha scaled by `by`, clamped to 0 to 1 and rounded; unchanged for a `by` not finite.
    #[must_use]
    pub fn fade(self, by: f64) -> Rgba {
        if !by.is_finite() {
            return self;
        }
        Rgba { a: (f64::from(self.a) * by.clamp(0.0, 1.0)).round() as u8, ..self }
    }
}

/// How a gradient goes on past its first and last stops.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Extend {
    /// The end stops' colors carry on.
    #[default]
    Pad,
    /// The stops repeat, every other time reversed.
    Reflect,
    /// The stops repeat.
    Repeat,
}

/// One stop of a gradient. Laid out as C reads it.
#[expect(clippy::exhaustive_structs, reason = "a plain value: its fields are all it is")]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Stop {
    /// Where along the gradient the stop lies, 0 at its start and 1 at its end.
    pub offset: f32,
    /// The color there.
    pub color: Rgba,
}

/// A gradient's shape, in the space its transform takes to the scene's.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GradientKind {
    /// Along the line from (`x0`, `y0`), offset 0, to (`x1`, `y1`), offset 1.
    Linear {
        /// The start's x.
        x0: f32,
        /// The start's y.
        y0: f32,
        /// The end's x.
        x1: f32,
        /// The end's y.
        y1: f32,
    },
    /// Between two circles, offset 0 on the first and 1 on the second.
    Radial {
        /// The first circle's center x.
        x0: f32,
        /// The first circle's center y.
        y0: f32,
        /// The first circle's radius.
        r0: f32,
        /// The second circle's center x.
        x1: f32,
        /// The second circle's center y.
        y1: f32,
        /// The second circle's radius.
        r1: f32,
    },
    /// Around a center, offset 0 at one angle and 1 at another, in degrees counterclockwise.
    Sweep {
        /// The center's x.
        cx: f32,
        /// The center's y.
        cy: f32,
        /// The angle at offset 0.
        start_angle: f32,
        /// The angle at offset 1.
        end_angle: f32,
    },
}

/// A gradient: its shape, stops, extend and placement.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Gradient<'s> {
    /// Its shape.
    pub kind: GradientKind,
    /// Its stops, in order of offset.
    pub stops: &'s [Stop],
    /// How it goes on past its first and last stops.
    pub extend: Extend,
    /// A 2×3 transform `[a, b, c, d, e, f]` taking its space to the scene's: `x' = a·x + c·y + e`,
    /// `y' = b·x + d·y + f`.
    pub transform: [f64; 6],
}

impl<'s> Gradient<'s> {
    /// A gradient from its parts.
    #[must_use]
    pub const fn new(kind: GradientKind, stops: &'s [Stop], extend: Extend, transform: [f64; 6]) -> Gradient<'s> {
        Gradient { kind, stops, extend, transform }
    }
}

/// How a layer combines with what lies under it: the Porter–Duff operators, then the blend modes of W3C
/// Compositing and Blending, declared in COLR's composite-mode order.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Blend {
    /// Nothing: both cleared.
    Clear,
    /// The source alone.
    Src,
    /// The backdrop alone.
    Dest,
    /// The source over the backdrop, as layers are drawn by default.
    #[default]
    SrcOver,
    /// The backdrop over the source.
    DestOver,
    /// The source where the backdrop is.
    SrcIn,
    /// The backdrop where the source is.
    DestIn,
    /// The source where the backdrop isn't.
    SrcOut,
    /// The backdrop where the source isn't.
    DestOut,
    /// The source over the backdrop, only where the backdrop is.
    SrcAtop,
    /// The backdrop over the source, only where the source is.
    DestAtop,
    /// Each where the other isn't.
    Xor,
    /// The two added.
    Plus,
    /// The two's complements multiplied and complemented: lighter.
    Screen,
    /// Multiply or screen, as the backdrop's color is dark or light.
    Overlay,
    /// The darker of the two, channel by channel.
    Darken,
    /// The lighter of the two, channel by channel.
    Lighten,
    /// The backdrop brightened by the source.
    ColorDodge,
    /// The backdrop darkened by the source.
    ColorBurn,
    /// Multiply or screen, as the source's color is dark or light.
    HardLight,
    /// A softer hard light: darken or lighten, as the source's color is dark or light.
    SoftLight,
    /// The darker taken from the lighter.
    Difference,
    /// As difference, with less contrast.
    Exclusion,
    /// The two multiplied: darker.
    Multiply,
    /// The source's hue with the backdrop's saturation and luminosity.
    HslHue,
    /// The source's saturation with the backdrop's hue and luminosity.
    HslSaturation,
    /// The source's hue and saturation with the backdrop's luminosity.
    HslColor,
    /// The source's luminosity with the backdrop's hue and saturation.
    HslLuminosity,
}

impl Blend {
    /// The mode a COLR composite-mode number names: `Clear` for one past the last, as the spec asks.
    #[must_use]
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

/// What a gradient's stops resolve to as drawn.
#[non_exhaustive]
#[derive(Clone, PartialEq, Debug)]
pub enum Stops {
    /// Nothing to paint: no stops, or stops all at one offset under repeat or reflect.
    Nothing,
    /// One color, from a single stop.
    Solid(Rgba),
    /// Two or more stops, clamped to 0 to 1 and sorted by offset.
    Many(Vec<Stop>),
}

// What `resolve_in_place` makes of the stops, which it leaves where they lie.
pub(crate) enum Resolved {
    Nothing,
    Solid(Rgba),
    Many,
}

// COLR defines a color line over its first to last stop, which may lie past or within 0..1, and extends
// it from there. The geometry moves to those stops instead, so they span 0..1 and nothing is clamped.
pub(crate) fn normalize_color_line(kind: GradientKind, stops: &mut [Stop]) -> GradientKind {
    let offsets = stops.iter().map(|s| s.offset).filter(|o| o.is_finite());
    let (first, last) = offsets.fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), o| (a.min(o), b.max(o)));
    if first > last || (first == last && (0.0..=1.0).contains(&first)) || (first == 0.0 && last == 1.0) {
        return kind;
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
    for s in stops {
        s.offset = ((f64::from(s.offset) - a) / (b - a)) as f32;
    }
    kind
}

impl Stops {
    /// A gradient's stops as drawn: nothing, one color, or two or more clamped to 0 to 1 and sorted. Stops all at
    /// one offset draw nothing under repeat and reflect, as COLR calls them ill-formed.
    #[doc(alias("resolve_stops"))]
    pub fn resolve(mut stops: Vec<Stop>, extend: Extend) -> Stops {
        match resolve_in_place(&mut stops, extend) {
            Resolved::Nothing => Stops::Nothing,
            Resolved::Solid(c) => Stops::Solid(c),
            Resolved::Many => Stops::Many(stops),
        }
    }
}

// COLR: stops all at one offset are ill-formed under repeat and reflect, and draw nothing. Decided before
// the clamp, which would make stops past 0..1 coincide.
pub(crate) fn resolve_in_place(stops: &mut [Stop], extend: Extend) -> Resolved {
    match stops.len() {
        0 => return Resolved::Nothing,
        1 => return Resolved::Solid(stops[0].color),
        _ => {}
    }
    for s in stops.iter_mut() {
        if s.offset.is_nan() { s.offset = 0.0; }
    }
    if extend != Extend::Pad && stops.windows(2).all(|w| w[0].offset == w[1].offset) {
        return Resolved::Nothing;
    }
    for s in stops.iter_mut() {
        s.offset = s.offset.clamp(0.0, 1.0);
    }
    stops.sort_by(|a, b| a.offset.partial_cmp(&b.offset).unwrap_or(core::cmp::Ordering::Equal));
    Resolved::Many
}
