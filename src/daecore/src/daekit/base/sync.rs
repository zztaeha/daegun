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

    pub fn try_write<T>(cell: &Mutable<T>) -> Option<core::cell::RefMut<'_, T>> {
        cell.try_borrow_mut().ok()
    }

    pub type Shareable<T> = RefCell<T>;

    pub fn read_shared<T>(cell: &Shareable<T>) -> core::cell::Ref<'_, T> {
        cell.borrow()
    }

    pub fn write_shared<T>(cell: &Shareable<T>) -> core::cell::RefMut<'_, T> {
        cell.borrow_mut()
    }

    pub fn try_write_shared<T>(cell: &Shareable<T>) -> Option<core::cell::RefMut<'_, T>> {
        cell.try_borrow_mut().ok()
    }

    pub fn try_read_shared<T>(cell: &Shareable<T>) -> Option<core::cell::Ref<'_, T>> {
        cell.try_borrow().ok()
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

    // Read by many threads at once and changed by one, poisoning ignored as `Lock` ignores it.
    pub type Shareable<T> = std::sync::RwLock<T>;

    pub fn read_shared<T>(cell: &Shareable<T>) -> std::sync::RwLockReadGuard<'_, T> {
        cell.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn write_shared<T>(cell: &Shareable<T>) -> std::sync::RwLockWriteGuard<'_, T> {
        cell.write().unwrap_or_else(|e| e.into_inner())
    }

    // Enough for a reader's short hold to end, and few enough that a thread gives up before queueing behind others.
    const TRIES: usize = 8;

    // None where other threads hold it through every try, for work that can be skipped rather than waited for.
    pub fn try_write_shared<T>(cell: &Shareable<T>) -> Option<std::sync::RwLockWriteGuard<'_, T>> {
        for _ in 0..TRIES {
            match cell.try_write() {
                Ok(guard) => return Some(guard),
                Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => core::hint::spin_loop(),
            }
        }
        None
    }

    // As `try_write_shared`, for a lock that has no readers.
    pub fn try_write<T>(cell: &Mutable<T>) -> Option<LockGuard<'_, T>> {
        for _ in 0..TRIES {
            if let Some(guard) = cell.try_lock() {
                return Some(guard);
            }
            core::hint::spin_loop();
        }
        None
    }

    // As `try_write_shared`.
    pub fn try_read_shared<T>(cell: &Shareable<T>) -> Option<std::sync::RwLockReadGuard<'_, T>> {
        for _ in 0..TRIES {
            match cell.try_read() {
                Ok(guard) => return Some(guard),
                Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => core::hint::spin_loop(),
            }
        }
        None
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
                // Poisoning is ignored: propagating it turns one thread's panic into every later call failing. Most
                // hold this over one cache insert or lookup; the hinting caches hold theirs over the hint.
                self.0.lock().unwrap_or_else(|e| e.into_inner())
            }

            pub(crate) fn try_lock(&self) -> Option<LockGuard<'_, T>> {
                match self.0.try_lock() {
                    Ok(guard) => Some(guard),
                    Err(std::sync::TryLockError::Poisoned(e)) => Some(e.into_inner()),
                    Err(std::sync::TryLockError::WouldBlock) => None,
                }
            }
        }
    }
}

pub use imp::{read, read_shared, write, write_shared, Mutable, Shareable, Shared};
pub use imp::{try_read_shared, try_write, try_write_shared};

#[cfg(not(feature = "threading"))]
#[derive(Default)]
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
    pub(crate) fn add(&self, n: usize) {
        self.0.set(self.0.get().saturating_add(n))
    }
    pub(crate) fn draw(&self, n: usize) -> bool {
        let left = self.0.get().checked_sub(n);
        left.inspect(|&left| self.0.set(left)).is_some()
    }
}

// An atomic, as one `Cell` anywhere in `FontCache` withdraws `Sync` from `Font`. `Relaxed` is correct: nothing else is
// ordered against these budgets, and a draw takes its bytes in one step, so threads racing for the last can't both.
#[cfg(feature = "threading")]
#[derive(Default)]
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
    pub(crate) fn add(&self, n: usize) {
        use core::sync::atomic::Ordering::Relaxed;
        let _ = self.0.try_update(Relaxed, Relaxed, |v| Some(v.saturating_add(n)));
    }
    // `n` taken where at least `n` is left, else nothing: false then.
    pub(crate) fn draw(&self, n: usize) -> bool {
        use core::sync::atomic::Ordering::Relaxed;
        self.0.try_update(Relaxed, Relaxed, |v| v.checked_sub(n)).is_ok()
    }
}

// A value on cache lines of its own (128 bytes on Apple silicon), for one written at every call, as a `Shared` count
// is: each write would otherwise stall another thread reading its own value on the line.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "threading", repr(align(128)))]
pub(crate) struct Lined<T>(pub(crate) T);

impl<T> core::ops::Deref for Lined<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

// What a vector holds on the heap, as a budget charges it.
pub(crate) fn heap_of<T>(v: &alloc::vec::Vec<T>) -> usize {
    v.capacity() * core::mem::size_of::<T>()
}

// What a `Shared<T>` allocates of its own: its two counts and the value, not what the value points to.
pub(crate) const fn shared_of<T>() -> usize {
    2 * core::mem::size_of::<usize>() + core::mem::size_of::<T>()
}

// What a `BTreeMap` holds an entry in, nodes and all: leaves of 11 slots left holding 6 as appending splits them,
// and each leaf's share of a parent over 7 of them.
pub(crate) const fn btree_entry_of<K, V>() -> usize {
    use core::mem::{align_of, size_of};
    let (k, v, p) = (align_of::<K>(), align_of::<V>(), align_of::<usize>());
    let align = if k > v { k } else { v };
    let align = if align > p { align } else { p };
    let leaf = (size_of::<usize>() + 4 + 11 * (size_of::<K>() + size_of::<V>())).next_multiple_of(align);
    (7 * leaf + leaf + 12 * size_of::<usize>()) / 49
}

pub fn mutable<T>(value: T) -> Mutable<T> {
    Mutable::new(value)
}

// A value built once and read without a lock: `OnceLock` where threads share it, `OnceCell` where they can't.
#[cfg(feature = "threading")]
pub(crate) type Once<T> = std::sync::OnceLock<T>;
#[cfg(not(feature = "threading"))]
pub(crate) type Once<T> = core::cell::OnceCell<T>;

// One value, built once and read in place: a racing caller waits for the build rather than drawing a second
// copy's bytes, so `build` must not read its own cell. `stored` is told of the value, to charge for it.
pub(crate) fn once_slot<T>(cell: &Once<T>, build: impl FnOnce() -> T, stored: impl FnOnce(&T)) -> &T {
    cell.get_or_init(|| {
        let built = build();
        stored(&built);
        built
    })
}

// Every thread-local the core keeps, emptied: what the thread last read and hinted with, which hold their fonts, and
// the scratch it reuses.
#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    use crate::daecore::{daekit, daetype};
    daetype::colr_v1::release_thread();
    daetype::colr_v1::paint::release_thread();
    daetype::decoder::ttf::release_thread();
    daetype::hinting::release_thread();
    daetype::hinting::auto::points::release_thread();
    daetype::hinting::cff::hinter::release_thread();
    daetype::outline::cff_pen::release_thread();
    daetype::outline::glyf_pen::release_thread();
    daetype::outline::stroke::release_thread();
    daetype::paint::release_thread();
    daekit::cache::release_thread();
    daekit::cache::drawn::release_thread();
    daekit::cache::instance::release_thread();
    #[cfg(feature = "threading")]
    daekit::cache::shaping::release_thread();
    daekit::cache::varc::release_thread();
}

// A number nothing else has had, naming a font or a cache without holding it. 64 bits, as what threads and
// rasterizers keep is matched by it, and 32 come round within a long run of new locations.
pub(crate) type Id = u64;

#[cfg(target_has_atomic = "64")]
pub(crate) fn next_id() -> Id {
    static NEXT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
}

#[cfg(all(not(target_has_atomic = "64"), feature = "std"))]
pub(crate) fn next_id() -> Id {
    static NEXT: std::sync::Mutex<Id> = std::sync::Mutex::new(0);
    let mut next = NEXT.lock().unwrap_or_else(|e| e.into_inner());
    *next += 1;
    *next
}

// shortcut: past 2^32 fonts an id comes round, so a location may be read as another font's; nothing short of `std` or
// 64-bit atomics counts further, and a target with neither opens far fewer.
#[cfg(all(not(target_has_atomic = "64"), not(feature = "std")))]
pub(crate) fn next_id() -> Id {
    static NEXT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);
    NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed) as Id
}

// The value for the last key asked, built again when the key changes: outside the guard, as one held across `build`
// deadlocks under `threading` and panics a `RefCell`.
pub(crate) fn memo_last<Q: PartialEq + alloc::borrow::ToOwned + ?Sized, V: Clone>(
    cell: &Mutable<Option<(Q::Owned, V)>>,
    key: &Q,
    build: impl FnOnce() -> V,
) -> V {
    use core::borrow::Borrow;
    if let Some((at, hit)) = read(cell).as_ref()
        && at.borrow() == key
    {
        return hit.clone();
    }
    let built = build();
    *write(cell) = Some((key.to_owned(), built.clone()));
    built
}

// A u32 for each glyph, 0 for none, read and written without a lock. Ahead of the glyphs' slots, a count of writes
// and the glyphs written, up to `LISTED` or a sixteenth of the glyphs, so the table is emptied there and not whole.
#[derive(Default)]
pub(crate) struct GlyphSlots {
    slots: alloc::boxed::Box<[core::sync::atomic::AtomicU32]>,
    // Where the glyphs' slots start: past the count and the list.
    base: usize,
}

// A location drawn or shaped at for a line or two writes fewer; past them, or a sixteenth of the glyphs, emptying the
// table whole costs no more.
const LISTED: usize = 64;

impl GlyphSlots {
    pub(crate) fn new(glyphs: usize) -> GlyphSlots {
        let base = 1 + LISTED.min(glyphs / 16);
        let slots = (0..base + glyphs).map(|_| core::sync::atomic::AtomicU32::new(0)).collect();
        GlyphSlots { slots, base }
    }

    pub(crate) fn len(&self) -> usize {
        self.slots.len().saturating_sub(self.base)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    #[inline]
    pub(crate) fn get(&self, gid: u32) -> u32 {
        self.slots.get(self.base + gid as usize).map_or(0, |v| v.load(core::sync::atomic::Ordering::Relaxed))
    }

    // Threads writing one glyph write the same value.
    pub(crate) fn set(&self, gid: u32, value: u32) {
        use core::sync::atomic::Ordering::Relaxed;
        let Some(slot) = self.slots.get(self.base + gid as usize) else { return };
        slot.store(value, Relaxed);
        let at = 1 + self.slots[0].fetch_add(1, Relaxed) as usize;
        if at < self.base {
            self.slots[at].store(gid, Relaxed);
        }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        core::mem::size_of_val(&*self.slots)
    }

    fn empty(&mut self) {
        use core::sync::atomic::Ordering::Relaxed;
        let written = self.slots[0].swap(0, Relaxed) as usize;
        let (list, glyphs) = self.slots.split_at(self.base);
        match written < self.base {
            true => list[1..=written].iter().for_each(|gid| glyphs[gid.load(Relaxed) as usize].store(0, Relaxed)),
            false => glyphs.iter().for_each(|slot| slot.store(0, Relaxed)),
        }
    }
}

// Tables a font's locations gave back emptied, which the next ones take rather than zeroing their own.
pub(crate) struct SpareSlots(Mutable<alloc::vec::Vec<GlyphSlots>>);

impl Default for SpareSlots {
    fn default() -> SpareSlots {
        SpareSlots(mutable(alloc::vec::Vec::new()))
    }
}

// One location's worth: its drawn glyphs' two indexes and origins, its advances both ways and VORG's origins.
const SPARES: usize = 6;

impl SpareSlots {
    // A busy list is passed over, both ways: a new table costs less than the wait.
    pub(crate) fn take(&self, glyphs: usize) -> GlyphSlots {
        let mut spare = try_write(&self.0);
        let found = spare.as_mut().and_then(|spare| {
            let at = spare.iter().position(|table| table.len() == glyphs)?;
            Some(spare.swap_remove(at))
        });
        found.unwrap_or_else(|| GlyphSlots::new(glyphs))
    }

    pub(crate) fn give(&self, mut table: GlyphSlots) {
        if table.is_empty() {
            return;
        }
        table.empty();
        if let Some(mut spare) = try_write(&self.0)
            && spare.len() < SPARES
        {
            spare.push(table);
        }
    }

    pub(crate) fn bytes(&self) -> usize {
        read(&self.0).iter().map(GlyphSlots::heap_bytes).sum()
    }

    pub(crate) fn clear(&self) {
        write(&self.0).clear();
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/base/sync.rs"]
mod tests;
