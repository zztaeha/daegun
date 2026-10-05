#[cfg(not(feature = "threading"))]
mod imp {
    use alloc::rc::Rc;
    use core::cell::RefCell;

    pub type Shared<T> = Rc<T>;

    pub type Mutable<T> = RefCell<T>;

    pub fn read<T>(cell: &Mutable<T>) -> core::cell::Ref<'_, T> {
        cell.borrow()
    }

    pub fn write<T>(cell: &Mutable<T>) -> core::cell::RefMut<'_, T> {
        cell.borrow_mut()
    }
}

#[cfg(feature = "threading")]
mod imp {
    use alloc::sync::Arc;

    pub type Shared<T> = Arc<T>;
    pub type Mutable<T> = Lock<T>;

    pub fn read<T>(cell: &Mutable<T>) -> LockGuard<'_, T> {
        cell.lock()
    }

    pub fn write<T>(cell: &Mutable<T>) -> LockGuard<'_, T> {
        cell.lock()
    }

    // `threading` enables `std`, so this is the only lock there is. One cannot be written for
    // `no_std` under `forbid(unsafe_code)`, and a spin lock deadlocks on a single core.
    pub use std_lock::{Lock, LockGuard};

    mod std_lock {
        pub struct Lock<T>(std::sync::Mutex<T>);

        pub type LockGuard<'a, T> = std::sync::MutexGuard<'a, T>;

        impl<T> Lock<T> {
            pub fn new(value: T) -> Lock<T> {
                Lock(std::sync::Mutex::new(value))
            }

            pub(crate) fn lock(&self) -> LockGuard<'_, T> {
                // Poisoning is ignored on purpose: propagating it turns one thread's panic into
                // every later call failing. Most callers hold this only across a cache insert or
                // lookup, so the map is either updated or not – but `hint_glyph_cached` and
                // `try_autohint` hold theirs across the hint itself, and that is theirs to keep.
                self.0.lock().unwrap_or_else(|e| e.into_inner())
            }
        }
    }

}

pub use imp::{read, write, Mutable, Shared};

#[cfg(not(feature = "threading"))]
pub(crate) struct Counter(core::cell::Cell<usize>);

#[cfg(not(feature = "threading"))]
impl Counter {
    pub fn new(v: usize) -> Counter {
        Counter(core::cell::Cell::new(v))
    }
    pub(crate) fn get(&self) -> usize {
        self.0.get()
    }
    pub(crate) fn set(&self, v: usize) {
        self.0.set(v)
    }
}

// An atomic, never a bare `Cell`: `Cell` is `Send` but not `Sync`, so one anywhere in `FontCache`
// withdraws `Sync` from `Font` for the whole crate. `Relaxed` is correct rather than a shortcut: this
// is a budget nothing else is ordered against, and two threads racing it overspend by one index,
// which loosens a bound rather than losing it.
#[cfg(feature = "threading")]
pub(crate) struct Counter(core::sync::atomic::AtomicUsize);

#[cfg(feature = "threading")]
impl Counter {
    pub fn new(v: usize) -> Counter {
        Counter(core::sync::atomic::AtomicUsize::new(v))
    }
    pub(crate) fn get(&self) -> usize {
        self.0.load(core::sync::atomic::Ordering::Relaxed)
    }
    pub(crate) fn set(&self, v: usize) {
        self.0.store(v, core::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(not(feature = "threading"))]
pub fn mutable<T>(value: T) -> Mutable<T> {
    core::cell::RefCell::new(value)
}

#[cfg(feature = "threading")]
pub fn mutable<T>(value: T) -> Mutable<T> {
    imp::Lock::new(value)
}
