// Lint levels nest, so an `#[allow(unsafe_code)]` anywhere below is E0453, a hard error, not an override: the
// core, where a hostile font's bytes are parsed, cannot hold unsafe by construction.
#![forbid(unsafe_code)]

pub mod daekit;
pub use daekit::base::{Bounds, Error, ErrorKind, GlyphId, NameId, Orientation, Tag};

pub mod daetype;
pub mod daeshaper;
