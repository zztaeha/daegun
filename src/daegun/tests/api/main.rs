#[allow(dead_code, reason = "mounted test files use it; the target compiles empty until they exist")]
const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts");

mod autohint;

mod stroke;

mod synthetic;

mod outline_sweep;

mod varhint;

#[cfg(feature = "threading")]
mod threading;

mod surface;

mod surface_glyphs;

mod prewarm;

mod access;

mod segmentation;

mod latency;

mod cached_facts;

mod linebreak_stretch;

mod colr_v0;

mod colr_v1;

mod layout_subset;

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

mod cff_subset;

mod subset_tables;
