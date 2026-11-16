//! The libFuzzer targets' shared parts: the target macro and the font-aware mutator.

pub use libfuzzer_sys;

pub mod mutator;

/// A target calling the harness function of its name; `font` targets mutate inputs table by table, `raw` ones as
/// libFuzzer does.
#[macro_export]
macro_rules! target {
    ($name:ident, font) => {
        $crate::libfuzzer_sys::fuzz_target!(|data: &[u8]| {
            daegun_fuzz_harness::bounded(|| daegun_fuzz_harness::$name(data))
        });
        $crate::libfuzzer_sys::fuzz_mutator!(|data: &mut [u8], size: usize, max_size: usize, seed: u32| {
            $crate::mutator::mutate(data, size, max_size, seed)
        });
        $crate::libfuzzer_sys::fuzz_crossover!(|a: &[u8], b: &[u8], out: &mut [u8], seed: u32| {
            $crate::mutator::cross(a, b, out, seed)
        });
    };
    ($name:ident, raw) => {
        $crate::libfuzzer_sys::fuzz_target!(|data: &[u8]| {
            daegun_fuzz_harness::bounded(|| daegun_fuzz_harness::$name(data))
        });
    };
}
