const FONTS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts");

// This thread's instructions and cycles on each core type, from the kernel's per-thread accounting: no root and no
// recording, for `tasks/tools/perf/pmu-fast.sh`. None where the call is missing or refused.
#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "the kernel's counters are only a C call away")]
fn core_counts() -> Option<[[u64; 2]; 2]> {
    unsafe extern "C" {
        fn thread_selfcounts(kind: u32, dst: *mut core::ffi::c_void, size: usize) -> i32;
    }
    let mut counts = [[0u64; 2]; 2];
    // Kind 2 fills one (instructions, cycles) pair per core type, performance first.
    let ok = unsafe { thread_selfcounts(2, counts.as_mut_ptr().cast(), size_of_val(&counts)) } == 0;
    ok.then_some(counts)
}

#[cfg(not(target_os = "macos"))]
fn core_counts() -> Option<[[u64; 2]; 2]> {
    None
}

#[cfg(any(debug_assertions, daegun_counting))]
mod allocs;
mod bench;
mod latency;
#[cfg(feature = "threading")]
mod threads;
