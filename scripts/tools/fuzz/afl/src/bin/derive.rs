fn main() {
    afl::fuzz!(|data: &[u8]| daegun_fuzz_harness::bounded(|| daegun_fuzz_harness::derive(data)));
}
