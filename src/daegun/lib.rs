// Unconditional, including under test, so the compiler enforces it rather than a
// `--no-default-features` build nobody runs. Tests read fixtures off disk, so they get std back.
#![no_std]
// `deny`, not `forbid`, only because `ffi` opts back in to turn C pointers into references. Every
// other module is `forbid`, which no inner `allow` can override.
#![deny(unsafe_code)]
// A font is untrusted input and a panic is a denial of service that `forbid(unsafe_code)` does
// nothing about. Gated on `not(test)` so the crate's own unit tests may `expect`.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

#[cfg_attr(not(test), macro_use)]
extern crate alloc;

#[doc(hidden)]
#[path = "../daecore/src/mod.rs"]
pub mod daecore;

#[cfg(feature = "capi")]
#[path = "../c-wrapper/mod.rs"]
mod ffi;

#[cfg(test)]
#[macro_use]
extern crate std;

#[cfg(all(feature = "std", not(test)))]
extern crate std;

pub(crate) use crate::daecore::{cache, daeshaper, sync};

#[forbid(unsafe_code)]
mod glyphcache;
#[forbid(unsafe_code)]
mod text;
#[forbid(unsafe_code)]
mod api;

// What a color scene is made of, and the math to sample its gradients and composite its layers.
#[forbid(unsafe_code)]
pub mod paint {
    pub use crate::daecore::daemachine::daemath::{blend, gradient, matrix};
    use crate::daecore::daetype::paint;
    pub use crate::daecore::daetype::paint::colr;

    pub use paint::{
        resolve_stops, Blend, ClipShape, DisplayList, Extend, Gradient, GradientKind, Op, Paint, PathId,
        Rgba, Stop, Stops,
    };
    pub use matrix::{concat, invert, Matrix, IDENTITY};

    pub use blend::{blend, composite, Rgb};

    pub use colr::lower;
}

pub use text::{
    grapheme_boundaries, line_break_opportunities, resolve_bidi, word_boundaries, BidiParagraph,
    LineBreak, ShapeOptions, Ignorables,
    line_visual_runs, script_runs, GeneralCategory, Script, ScriptRun, VisualRun,
    general_category, is_upright, vertical_form,
};
pub use daeshaper::buffer::ClusterLevel;
pub use api::{
    Font, FontError, SubsetResult, Paint, ColorStop, GlyphBitmap, BitmapImage, ColrLayer, PaletteInfo,
    BaseScriptInfo, StatAxis, StatAxisValue, StatInfo, MathKernCorner, NamedInstance, ShapedRun,
    GlyphPart, GlyphAssembly, MathGlyphVariant, MathGlyphConstruction, MathConstants, JstfModLists,
    FvarAxis, SubSuperMetrics, TypographicMetrics, GlyphClass,
    SubpixelLayout, StripeOrder, MAX_OVERSAMPLE, HintMode, Rect, ShelfPacker, Justified, JustifyOptions, BidiRun,
    Cap, Join, StrokeStyle, stroke, stroke_simplified,
    Os2Info, WinMetrics, TypoLineMetrics,
    HintedOutline, draw_hinted, FLAG_CUBIC, FLAG_ON_CURVE,
    CffHints, CffStem,
    FillRule, Path, TransformPen, Verb,
    Quad, QuadError, QuadraticPen, normalize_winding, MAX_CURVES_PER_GLYPH,
    flatten, max_area_for, resolve_overlaps, MAX_FLATTEN_POINTS, MAX_RESOLVE_EDGES,
    OutlineOptions, PreparedGlyph,
    MAX_SUBPIXEL_WEIGHTS, MAX_SUBPIXEL_TAPS,
    LineMetrics, Align, BreakStrategy, LayoutLine, LayoutOptions, PositionedRun, TextLayout,
    TextOrientation, WritingMode,
    DEFAULT_POINT_SIZE,
};
pub use crate::daecore::daetype::outline::OutlinePen;
pub use api::{outline_glyf_bytes, parse_loca};
pub use api::bytes;
pub use api::format;
pub use api::build_font;
pub use crate::paint::DisplayList as ColorScene;

// Also at the top level, beside `colr_scene_with`, whose signature takes it.
pub use crate::paint::Rgba;
