const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts");

// A location from tags written as text, as the tests name them.
fn located(font: &daegun::Font, axes: &[(&str, f64)]) -> daegun::Location {
    let coords: Vec<(daegun::Tag, f64)> = axes.iter().map(|&(tag, v)| (tag.parse().expect("a tag"), v)).collect();
    font.location(&coords)
}

// Horizontal `advances` into a fresh buffer, for tests comparing whole lists.
fn advances(font: &daegun::Font, glyphs: &[daegun::GlyphId], location: &daegun::Location) -> Vec<f64> {
    let mut out = Vec::new();
    font.advances(glyphs, location, daegun::Orientation::Horizontal, &mut out);
    out
}

// A run's offsets as pairs, x then y, for tests comparing them with literals.
fn xy(offsets: &[daegun::shape::Offset]) -> Vec<(f64, f64)> {
    offsets.iter().map(|&o| o.into()).collect()
}

// `hinted_outline` at a px it takes, so a refusal fails the test where it happens.
trait Hint {
    fn hinted(
        &self, glyph: daegun::GlyphId, location: &daegun::Location, px: f32, mode: daegun::outline::HintMode,
    ) -> Option<daegun::outline::HintedOutline>;
}

impl Hint for daegun::Font {
    fn hinted(
        &self, glyph: daegun::GlyphId, location: &daegun::Location, px: f32, mode: daegun::outline::HintMode,
    ) -> Option<daegun::outline::HintedOutline> {
        self.hinted_outline(glyph, location, px, mode).expect("a finite px above zero")
    }
}

mod autohint;

mod cff_hinting;

mod stroke;

mod synthetic;

mod outline_sweep;

mod varhint;

#[cfg(feature = "threading")]
mod threading;

mod release;

mod surface;

mod surface_glyphs;

mod prewarm;

mod access;

mod latency;

mod varc;

mod cached_facts;

mod colr_v0;

mod colr_v1;

mod layout_subset;

mod subset_text;

mod tables;

mod cache_budgets;

mod quads;

mod flatten;

mod prepared;

mod layout_variations;

mod instance_tables;

mod gvar_points;

mod cff2_gpos_instance;

mod outline_decoders;

mod stroke_coverage;

mod bytecode;

mod bitmap_subset;

mod aat_subset;

mod aat_shaping;

mod cmap_reading;

mod aat_reading;

mod arabic_fallback;

mod aat_sanitize;

mod aat_kerning;

mod aat_lookups;

mod aat_features;

mod cff_subset;

mod subset_tables;

mod layout_lookups;

mod layout_sanitize;

mod past_the_count;

mod shaping_buffer;

mod syllables;

mod reordering;

mod dotted_circles;

mod layout_build;

mod gpos_lookups;

mod layout_records;

mod attachments;

mod script_shapers;

mod unicode_properties;

mod shape_options;
mod record_searches;
mod variation_regions;
mod varied_glyphs;
mod color_load;
mod beyond_64k;
mod readme;
