#[allow(dead_code, reason = "mounted test files use it; the target compiles empty until they exist")]
const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts");

mod latency;

mod float_ext;

mod float_accuracy;

mod daemath;

mod disasm;

mod srgb;
