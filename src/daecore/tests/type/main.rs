#[allow(dead_code, reason = "mounted test files use it; the target compiles empty until they exist")]
const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts");

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
