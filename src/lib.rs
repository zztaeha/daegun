//! A font engine. Open a [`Font`], resolve a [`Location`] once for a variable one, and ask it for metrics, glyphs,
//! shaping and layout, outlines, color glyphs, MATH and the rest, each area a module of its own.
//!
//! ```
//! use daegun::shape::ShapeOptions;
//! use daegun::{Font, Tag};
//!
//! # fn main() -> daegun::Result<()> {
//! # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
//! # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
//! let font = Font::from_bytes(&bytes)?;
//! let bold = font.location(&[(Tag::new(b"wght"), 700.0)]);
//! let run = font.shape("Wave", &bold, &ShapeOptions::DEFAULT);
//! let px = run.width() * 16.0 / f64::from(font.units_per_em());
//! # assert!(px > 40.0 && px < 60.0, "{px}");
//! # Ok(())
//! # }
//! ```
//!
//! **Units.** Every metric, advance, offset, box and outline is in the font's own units, `f64` (an outline's points
//! `f32`), y up and unrounded; only a call that takes a pixel size, [`Font::prepared_outline`] and
//! [`Font::hinted_outline`], answers in pixels. A value past `f32`'s range on the way to a pen is an
//! [`Overflow`](ErrorKind::Overflow) or [`InvalidArgument`](ErrorKind::InvalidArgument) error, never a point drawn
//! at infinity.
//!
//! **Text.** Every position in text is a UTF-8 byte offset on a character boundary: a run's clusters, a feature's
//! range, line breaks, boundaries, bidi runs and carets alike, so `&text[range]` slices what it names.
//!
//! **Answers.** A font is checked as it opens, so its metrics are plain numbers. An `Option` means the font has none
//! of something, never a 0 or -1 standing in for none; a [`Result`] is a call that can fail for a reason to act on,
//! its [`Error`]'s [`kind`](Error::kind). A run as shaping gives it is an `Arc`, which a shape cache hit hands out
//! again without a copy; [`Font::shape_into`] fills a run of your own instead.
//!
//! **Threads.** With the `threading` feature a `Font` is `Send + Sync`, one font serving every thread through an
//! `Arc`, each thread keeping its own shaping buffer, hinters, locations and recent runs. A thread keeps what it last
//! hinted with and the locations it last read at (up to 4 hinting contexts, 4 autohinters and 2 locations), so a
//! dropped font's bytes may stay until it hints or reads at locations of other fonts, ends, or calls
//! [`release_thread`]. Without the feature a font stays on the thread that opened it. Results cross
//! threads in every build, and are `UnwindSafe`; a font is too under `threading`, and without it holds `RefCell`s, so
//! a `catch_unwind` over one takes `AssertUnwindSafe`. No code here is unsafe, so a font a panic interrupts stays
//! memory-safe to use.
//!
//! **Bytes.** [`Font::from_static`] reads a font where it lies, so its bytes must not change while a font reads them.
//!
//! **Stability.** What a minor release may grow is `#[non_exhaustive]`: every struct a call returns, every type of
//! options and every enum that may gain a variant. Build options, and the text and metrics results a test of yours
//! may need, with their constructors or `Default`; a record a font holds comes from the call that reads it. Match an
//! enum with a `_` arm. The exceptions are plain values whose fields are all they are ([`GlyphId`], [`NameId`],
//! [`Bounds`], [`outline::Point`], [`outline::Quad`], [`shape::Offset`], [`shape::Feature`], [`color::Rgba`],
//! [`color::Stop`], [`color::Center`], [`color::Premultiplied`], [`color::PathId`], [`color::GradientId`],
//! [`raster::Rect`], [`unicode::WordSegmenter`]), the records [`raw::format`] reads field for
//! field ([`Entry`](raw::format::Entry), [`GlyphValue`](raw::format::GlyphValue),
//! [`AnchorPoint`](raw::format::AnchorPoint), [`RegionAxis`](raw::format::RegionAxis),
//! [`DeltaSetIndex`](raw::format::DeltaSetIndex)), and sets a specification or geometry closes ([`Orientation`],
//! [`shape::Direction`], [`unicode::BaseDirection`], [`unicode::ScriptDirection`], [`layout::Adjustment`],
//! [`outline::FillRule`], [`outline::Verb`], [`outline::PointKind`], [`math::MathKernCorner`], [`GlyphClass`],
//! [`unicode::GeneralCategory`], [`raw::format::Substitution`]). [`outline::OutlinePen`] and
//! [`color::PaintVisitor`] are yours to implement, lent by `&mut` or as `dyn`; a method either gains comes with a
//! default.
//!
//! **Features.** `std`, on by default, takes the standard library's float math and keeps scratch lists per thread;
//! without it the crate is `no_std`, needing only `alloc` and a target with atomic compare-and-swap at pointer width,
//! as `Arc` does: an Armv7-M microcontroller has it, an Armv6-M one doesn't. `threading` makes a [`Font`]
//! `Send + Sync`. `capi` builds the C ABI and implies `threading`. `dictionaries` builds in word lists for breaking
//! Thai, Lao, Khmer and Myanmar lines, about 1 MB.
//!
//! **Speed.** daegun's own numbers are built with `lto = true` and `codegen-units = 1`, which a dependency can't set
//! for you: put them in your own `[profile.release]` where shaping or drawing is the hot path.
//!
//! **Rust.** 1.99 or newer. The minimum moves only in a minor release, which says so.


// Unconditional, including under test, so the compiler enforces it rather than a
// `--no-default-features` build nobody runs. Tests read fixtures off disk, so they get std back.
#![no_std]
// docs.rs builds with `--cfg docsrs` on nightly, where each item a feature gates is badged with it.
#![cfg_attr(docsrs, feature(doc_cfg))]
// `forbid` wherever no code needs it; `deny` where `ffi` opts back in to turn C pointers into references, and
// under test, where two probes and a counting allocator do. Every other module is `forbid` either way.
#![cfg_attr(not(any(feature = "capi", test)), forbid(unsafe_code))]
#![cfg_attr(any(feature = "capi", test), deny(unsafe_code))]
// A font is untrusted input and a panic is a denial of service that `forbid(unsafe_code)` does
// nothing about. Gated on `not(test)` so the crate's own unit tests may `expect`.
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]
#![deny(missing_docs)]
#![doc(test(attr(deny(warnings))))]

// Every build shares results through `Arc`, which a target without it fails to find, naming nothing of why.
#[cfg(not(target_has_atomic = "ptr"))]
compile_error!("daegun needs atomic compare-and-swap at pointer width, which this target lacks");

#[cfg_attr(not(test), macro_use)]
extern crate alloc;

#[path = "daecore/src/mod.rs"]
mod daecore;

#[cfg(feature = "capi")]
#[path = "api/c/translation/mod.rs"]
mod ffi;

// The core's suites, mounted here since the core forbids the unsafe two of their probes need.
#[cfg(test)]
#[path = "daecore/tests/mod.rs"]
mod tests;

// Debug builds only, so the release benches sharing this test binary time the system allocator, not its counting;
// and release ones built with `--cfg daegun_counting`, for the benches that count (`allocs`).
#[cfg(all(test, any(debug_assertions, daegun_counting)))]
#[path = "api/rust/tests/alloc/main.rs"]
mod allocations;

#[cfg(test)]
#[macro_use]
extern crate std;

#[cfg(all(feature = "std", not(test)))]
extern crate std;

pub(crate) use crate::daecore::daeshaper;
pub(crate) use crate::daecore::daekit::{base::sync, cache, text};

#[forbid(unsafe_code)]
#[path = "api/rust/mod.rs"]
mod api;

#[forbid(unsafe_code)]
#[path = "rasterizer/CPU/mod.rs"]
mod rasterizer;

pub use api::{
    Font, Error, ErrorKind, Result,
    NamedInstance,
    Axis, Cache, CachePool, CacheStats, FontStyle, GlyphClass, LocalizedName,
};
pub use crate::daecore::{Bounds, GlyphId, NameId, Orientation, Tag};
pub use api::{color, layout, math, metrics, outline, raster, raw, shape, tables, unicode, release_thread, Location};
