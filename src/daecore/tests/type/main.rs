use alloc::vec::Vec;

const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts");

// A location from tags written as text, as the tests name them.
fn located(font: &crate::Font, axes: &[(&str, f64)]) -> crate::Location {
    let coords: Vec<(crate::Tag, f64)> = axes.iter().map(|&(tag, v)| (tag.parse().expect("a tag"), v)).collect();
    font.location(&coords)
}

// Horizontal `advances` into a fresh buffer, for tests comparing whole lists.
fn advances(font: &crate::Font, glyphs: &[crate::GlyphId], location: &crate::Location) -> Vec<f64> {
    let mut out = Vec::new();
    font.advances(glyphs, location, crate::Orientation::Horizontal, &mut out);
    out
}

mod cmap_coverage;

mod head_checksum;

mod outline_latency;

mod autohint_points;

mod stroke;

mod contour_start;

mod cff2_outlines;

mod font_structure;

mod hint_latency;

mod colr_latency;

mod colr_cache;

mod cffdecline;

mod fdef_scope;

#[cfg(feature = "threading")]
mod hint_threading;

mod font_decoders;
