//! Stack overflows reported with the input that made them. Without a sanitizer, libFuzzer's crash handler runs on the
//! stack that overflowed, so the process died with no report and no input saved: each thread a target runs on gets a
//! stack of its own for signals, and the crash handlers already installed run on it.

use std::cell::OnceCell;
use std::sync::{Once, OnceLock};

/// The stack, in KiB, the `stack` target runs on unless `DAEGUN_FUZZ_STACK` says otherwise: musl's thread default,
/// the smallest common one. Of 503 real fonts every one runs in 64 KiB but a hostile fixture, and 22 need over 32.
pub const STACK_KIB: usize = 128;

// Room for libFuzzer's crash report, which prints and writes the input from it.
const SIGNAL_STACK: usize = 256 << 10;

struct SignalStack {
    _memory: Vec<u8>,
}

// Unregistered before its memory goes, as the thread ends.
impl Drop for SignalStack {
    fn drop(&mut self) {
        let off = libc::stack_t { ss_sp: std::ptr::null_mut(), ss_size: 0, ss_flags: libc::SS_DISABLE };
        // SAFETY: disables the thread's signal stack, reading nothing back.
        unsafe { libc::sigaltstack(&off, std::ptr::null_mut()) };
    }
}

std::thread_local! {
    static OWN: OnceCell<Option<SignalStack>> = const { OnceCell::new() };
}

/// A signal stack for the calling thread where it has none, and the crash handlers set to run on it. Rust's own
/// `main` and the sanitizers give their threads one, and a sanitizer frees the one it finds as a thread ends, so
/// those are left alone.
pub fn guard_thread() {
    static HANDLERS: Once = Once::new();
    HANDLERS.call_once(|| {
        for signal in [libc::SIGSEGV, libc::SIGBUS] {
            // SAFETY: the installed action read into a zeroed one and written back with one more flag.
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(signal, std::ptr::null(), &mut action) == 0
                    && action.sa_sigaction != libc::SIG_DFL
                    && action.sa_flags & libc::SA_ONSTACK == 0
                {
                    action.sa_flags |= libc::SA_ONSTACK;
                    libc::sigaction(signal, &action, std::ptr::null_mut());
                }
            }
        }
    });
    let _ = OWN.try_with(|own| {
        own.get_or_init(|| {
            // SAFETY: the thread's current signal stack read into a zeroed one.
            let mut current: libc::stack_t = unsafe { std::mem::zeroed() };
            if unsafe { libc::sigaltstack(std::ptr::null(), &mut current) } != 0
                || current.ss_flags & libc::SS_DISABLE == 0
            {
                return None;
            }
            let mut memory = vec![0u8; SIGNAL_STACK];
            let stack = libc::stack_t { ss_sp: memory.as_mut_ptr().cast(), ss_size: memory.len(), ss_flags: 0 };
            // SAFETY: memory the thread holds in OWN, whose drop unregisters it before freeing it.
            unsafe { libc::sigaltstack(&stack, std::ptr::null_mut()) };
            Some(SignalStack { _memory: memory })
        });
    });
}

/// The stack the `stack` target's thread gets, in bytes.
pub fn stack_size() -> usize {
    static KIB: OnceLock<usize> = OnceLock::new();
    *KIB.get_or_init(|| {
        std::env::var("DAEGUN_FUZZ_STACK").ok().and_then(|v| v.parse().ok()).unwrap_or(STACK_KIB)
    }) << 10
}
