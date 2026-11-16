//! The memory oracle: Rust's heap counted as it's held, so what an input makes the engine hold is measured exactly
//! and charged to that input. A process's resident size can't be: it keeps what the allocator holds freed, and
//! libFuzzer reads its peak once a second, after the input that set it may have finished.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// The most the heap may hold at once, the 4 GB the campaign's resident limit meant. A 32-bit `usize` shifts 4 GB out
/// to 0, so there it's `isize::MAX`, the most one request can be, and a count and a request never overflow together.
pub const MEMORY_LIMIT: usize = if usize::BITS > 32 { 4 << 30 } else { isize::MAX as usize };
/// The largest one allocation may be, which libFuzzer's `-malloc_limit_mb` means but checks only under a sanitizer.
pub const ALLOCATION_LIMIT: usize = 2 << 30;

static HELD: AtomicUsize = AtomicUsize::new(0);
// The largest request refused since the input began, or 0.
static REFUSED: AtomicUsize = AtomicUsize::new(0);

struct Counted;

#[global_allocator]
static COUNTED: Counted = Counted;

// `more` bytes counted in, or the request refused and remembered: one past ALLOCATION_LIMIT, or the heap past
// MEMORY_LIMIT. The engine may meet a refusal through `try_reserve` and go on, so it fails the input afterwards.
fn take(more: usize, asked: usize) -> bool {
    let held = HELD.fetch_add(more, Relaxed) + more;
    if asked > ALLOCATION_LIMIT || held > MEMORY_LIMIT {
        HELD.fetch_sub(more, Relaxed);
        REFUSED.fetch_max(asked, Relaxed);
        return false;
    }
    true
}

// SAFETY: every call goes to the system allocator with the caller's own arguments; only the counting is added.
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !take(layout.size(), layout.size()) {
            return core::ptr::null_mut();
        }
        let p = unsafe { System.alloc(layout) };
        if p.is_null() {
            HELD.fetch_sub(layout.size(), Relaxed);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        if !take(layout.size(), layout.size()) {
            return core::ptr::null_mut();
        }
        let p = unsafe { System.alloc_zeroed(layout) };
        if p.is_null() {
            HELD.fetch_sub(layout.size(), Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        HELD.fetch_sub(layout.size(), Relaxed);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let old = layout.size();
        if new_size > old && !take(new_size - old, new_size) {
            return core::ptr::null_mut();
        }
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        match (p.is_null(), new_size > old) {
            (true, true) => _ = HELD.fetch_sub(new_size - old, Relaxed),
            (false, false) => _ = HELD.fetch_sub(old - new_size, Relaxed),
            _ => {}
        }
        p
    }
}

/// `body` run as one input, failed if anything it asked for was refused: the engine reached for more than an input
/// may hold, whether or not it took the refusal in its stride.
pub fn bounded(body: impl FnOnce()) {
    crate::guard_thread();
    REFUSED.store(0, Relaxed);
    body();
    let refused = REFUSED.load(Relaxed);
    must!(refused == 0, "memory: {refused} bytes refused, past {ALLOCATION_LIMIT} alone or the heap's {MEMORY_LIMIT}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fails(body: impl FnOnce() + std::panic::UnwindSafe) -> bool {
        std::panic::catch_unwind(|| bounded(body)).is_err()
    }

    // Refused quietly, through `try_reserve`, each still fails the input: one request past 2 GiB, and a heap that
    // three of 1.5 GiB would take past 4. Pages never touched, so the test holds none of it.
    #[test]
    #[cfg(target_pointer_width = "64")]
    fn what_an_input_may_not_hold_fails_it_though_refused_quietly() {
        assert!(fails(|| _ = Vec::<u8>::new().try_reserve(ALLOCATION_LIMIT + 1)));
        assert!(fails(|| {
            let held: Vec<Vec<u8>> = (0..3).filter_map(|_| {
                let mut v = Vec::new();
                v.try_reserve(3 << 29).ok().map(|()| v)
            }).collect();
            assert_eq!(held.len(), 2);
        }));
        assert!(!fails(|| _ = Vec::<u8>::with_capacity(1 << 30)));
    }
}
