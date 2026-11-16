use alloc::collections::btree_map::{self, BTreeMap};
use alloc::vec::Vec;
use core::borrow::Borrow;
use crate::daecore::daekit::base::sync::{mutable, write, Counter, Mutable, Shared};

// Bytes held under a budget, least recently used out first. A hit only stamps its entry, or marks it where lists
// share a clock, so readers touch it at once and eviction requeues it. Keys are held twice: wrap a costly one.
pub(crate) struct ByteLru<K: Ord + Clone, V> {
    entries: BTreeMap<K, Entry<V>>,
    by_age: BTreeMap<u64, K>,
    clock: Clock,
    // Where stamps come from instead, for lists whose entries' ages compare: one cache's shards, or a pool's lists.
    shared_clock: Option<Shared<Clock>>,
    bytes: usize,
    budget: usize,
    // Asked as an entry comes and as it goes, so a value that grows in between must be charged its growth by
    // `adjust`, for the two to agree.
    cost: fn(&K, &V) -> usize,
    // Told of each value that leaves or is refused, but not of one replaced, whose replacement holds its parts.
    left: fn(&V),
    keep_newest: bool,
    pool: Option<Link>,
}

struct Entry<V> {
    value: V,
    used: Clock,
}

// A hit's mark in a stamp, where a list sharing its clock takes no tick: one tick of a clock every thread writes cost
// each shared shape hit at 8 threads a line. The stamp's other bits keep the entry's place in the queue.
const USED_AGAIN: u64 = 1 << 63;

impl<K: Ord + Clone, V> ByteLru<K, V> {
    pub(crate) fn new(budget: usize, cost: fn(&K, &V) -> usize) -> ByteLru<K, V> {
        let (entries, by_age) = (BTreeMap::new(), BTreeMap::new());
        ByteLru {
            entries,
            by_age,
            clock: Clock::new(0),
            shared_clock: None,
            bytes: 0,
            budget,
            cost,
            left: |_| {},
            keep_newest: false,
            pool: None,
        }
    }

    #[cfg(feature = "threading")]
    pub(crate) fn stamped_by(mut self, clock: Shared<Clock>) -> ByteLru<K, V> {
        self.shared_clock = Some(clock);
        self
    }

    fn now(&self) -> u64 {
        self.shared_clock.as_deref().unwrap_or(&self.clock).tick()
    }

    pub(crate) fn telling(mut self, left: fn(&V)) -> ByteLru<K, V> {
        self.left = left;
        self
    }

    // For values so costly to rebuild that the most recently used stays past any budget, alone once over.
    pub(crate) fn keeping_newest(budget: usize, cost: fn(&K, &V) -> usize) -> ByteLru<K, V> {
        let mut list = ByteLru::new(budget, cost);
        list.keep_newest = true;
        list
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn budget(&self) -> usize {
        self.budget
    }

    // The pool it draws from.
    pub(crate) fn pool(&self) -> Option<Shared<Pool>> {
        self.pool.as_ref().map(|link| Shared::clone(&link.pool))
    }

    pub(crate) fn get<Q: Ord + ?Sized>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
    {
        self.get_held(key).map(|(_, value)| value)
    }

    // As `get`, with the key as held, for one asked for borrowed.
    pub(crate) fn get_held<Q: Ord + ?Sized>(&self, key: &Q) -> Option<(&K, &V)>
    where
        K: Borrow<Q>,
    {
        let (held, entry) = self.entries.get_key_value(key)?;
        self.used_again(&entry.used);
        Some((held, &entry.value))
    }

    #[cfg(feature = "threading")]
    fn used_again(&self, used: &Clock) {
        if self.shared_clock.is_none() {
            return used.set(self.now());
        }
        let at = used.get();
        if at & USED_AGAIN == 0 {
            used.set(at | USED_AGAIN);
        }
    }

    #[cfg(not(feature = "threading"))]
    fn used_again(&self, used: &Clock) {
        used.set(self.now());
    }

    // None when the value alone is over budget and so was not kept; else how many entries made room for it.
    pub(crate) fn insert(&mut self, key: K, value: V) -> Option<usize> {
        self.enter(key, value, None)
    }

    // Everything held, each with its stamp, mark and all, leaving the list empty and telling nothing of it.
    pub(crate) fn take_entries(&mut self) -> Vec<(u64, K, V)> {
        self.by_age.clear();
        self.lose(self.bytes, self.entries.len());
        self.relist();
        let entries = core::mem::take(&mut self.entries);
        entries.into_iter().map(|(key, entry)| (entry.used.get(), key, entry.value)).collect()
    }

    // As `insert`, for an entry that keeps the stamp of its last use.
    #[cfg(feature = "threading")]
    pub(crate) fn insert_used(&mut self, used: u64, key: K, value: V) -> Option<usize> {
        self.enter(key, value, Some(used))
    }

    fn enter(&mut self, key: K, value: V, used: Option<u64>) -> Option<usize> {
        let cost = (self.cost)(&key, &value);
        if cost > self.budget && !self.keep_newest {
            (self.left)(&value);
            return None;
        }
        let now = used.unwrap_or_else(|| self.now());
        let entry = Entry { value, used: Clock::new(now) };
        let replaced = match self.entries.entry(key) {
            btree_map::Entry::Occupied(mut held) => {
                let gone = core::mem::replace(held.get_mut(), entry);
                Some((self.cost)(held.key(), &gone.value))
            }
            btree_map::Entry::Vacant(slot) => {
                self.by_age.insert(now & !USED_AGAIN, slot.key().clone());
                slot.insert(entry);
                None
            }
        };
        if let Some(was) = replaced {
            self.lose(was, 0);
        }
        self.gain(cost, usize::from(replaced.is_none()));
        Some(self.evict_to_fit())
    }

    // Charges a held value's growth, or refunds its shrinking, which may evict others: how many.
    pub(crate) fn adjust(&mut self, by: isize) -> usize {
        match usize::try_from(by) {
            Ok(grew) => self.gain(grew, 0),
            Err(_) => self.lose(by.unsigned_abs().min(self.bytes), 0),
        }
        self.evict_to_fit()
    }

    // How many entries the new budget evicted.
    pub(crate) fn set_budget(&mut self, budget: usize) -> usize {
        self.budget = budget;
        self.evict_to_fit()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.values().for_each(|entry| (self.left)(&entry.value));
        self.lose(self.bytes, self.entries.len());
        self.entries.clear();
        self.by_age.clear();
        self.relist();
    }

    fn evict_to_fit(&mut self) -> usize {
        let (keep, mut evicted) = (usize::from(self.keep_newest), 0);
        while self.bytes > self.budget && self.entries.len() > keep {
            match self.step() {
                Some(Some((_, gone))) => {
                    (self.left)(&gone);
                    evicted += 1;
                }
                Some(None) => {}
                None => break,
            }
        }
        self.relist();
        evicted
    }

    // The oldest in the queue taken out, or, used since it was queued, queued again: one marked at a fresh tick,
    // unmarked, one stamped since at its stamp. None once empty.
    fn step(&mut self) -> Option<Option<(K, V)>> {
        let (tick, key) = self.by_age.pop_first()?;
        let btree_map::Entry::Occupied(held) = self.entries.entry(key) else { return Some(None) };
        let used = held.get().used.get();
        if used > tick {
            let used = match self.shared_clock.as_deref() {
                Some(clock) if used & USED_AGAIN != 0 => clock.tick(),
                _ => used,
            };
            held.get().used.set(used);
            self.by_age.insert(used, held.key().clone());
            return Some(None);
        }
        let (key, gone) = held.remove_entry();
        self.lose((self.cost)(&key, &gone.value), 1);
        Some(Some((key, gone.value)))
    }

    fn gain(&mut self, bytes: usize, entries: usize) {
        self.bytes += bytes;
        if let Some(link) = &self.pool {
            link.pool.held.add(bytes);
            link.pool.entries.add(entries);
        }
    }

    // What a pool counts is the sum of its lists', each changed under its list's lock, so it always covers this one's.
    fn lose(&mut self, bytes: usize, entries: usize) {
        self.bytes -= bytes;
        if let Some(link) = &self.pool {
            let drawn = link.pool.held.draw(bytes) & link.pool.entries.draw(entries);
            debug_assert!(drawn, "a list gave its pool back more than it drew");
        }
    }

    // Lists the head in the pool where it may be evicted, past the newest a list keeps whatever its budget.
    fn relist(&mut self) {
        let keep = usize::from(self.keep_newest);
        let Some(link) = &mut self.pool else { return };
        let head = self.by_age.first_key_value().map(|(&at, _)| at).filter(|_| self.entries.len() > keep);
        if head == link.listed {
            return;
        }
        let mut heads = write(&link.pool.heads);
        if let Some(was) = link.listed.take() {
            heads.remove(&was);
        }
        if let Some(at) = head {
            heads.insert(at, (link.me.clone(), link.at));
        }
        link.listed = head;
    }

    // The pool over its budget, to settle once this list's lock is let go.
    pub(crate) fn over(&self) -> Option<Shared<Pool>> {
        let link = self.pool.as_ref()?;
        (link.pool.held.get() > link.pool.budget.get()).then(|| Shared::clone(&link.pool))
    }

    // `pool`'s turn here, at the head it listed at `stamp`: taken out, or queued again where read since. How many left.
    pub(crate) fn evict_listed(&mut self, pool: &Pool, stamp: u64) -> usize {
        let keep = usize::from(self.keep_newest);
        let ours = |link: &&mut Link| core::ptr::eq(&*link.pool, pool) && link.listed == Some(stamp);
        let Some(link) = self.pool.as_mut().filter(ours) else { return 0 };
        link.listed = None;
        let gone = if self.entries.len() > keep { self.step().flatten() } else { None };
        if let Some((_, value)) = &gone {
            (self.left)(value);
        }
        self.relist();
        usize::from(gone.is_some())
    }
}

// Settles the pool `over` named, once the lock of the list that named it is let go.
pub(crate) fn settle(over: Option<Shared<Pool>>) {
    if let Some(pool) = over {
        pool.settle();
    }
}

// A list dropped gives its pool back what it held.
impl<K: Ord + Clone, V> Drop for ByteLru<K, V> {
    fn drop(&mut self) {
        if self.pool.is_some() {
            self.lose(self.bytes, self.entries.len());
            self.entries.clear();
            self.relist();
        }
    }
}

// One cache's lists into `pool`, or out of any, each the `at`th of `me`: what they hold stays, each entry stamped again
// in the order last used by the clock the lists then share, the pool's or, apart, their own. How many the move evicted.
pub(crate) fn rejoin<K: Ord + Clone, V, L: core::ops::DerefMut<Target = ByteLru<K, V>>>(
    lists: &mut [L],
    pool: Option<&Shared<Pool>>,
    me: &MemberRef,
    list_of: impl Fn(&K) -> usize,
) -> usize {
    let mut held: Vec<_> = lists.iter_mut().flat_map(|list| list.take_entries()).collect();
    held.sort_unstable_by_key(|&(used, ..)| used);
    let clock = match pool {
        Some(pool) => Some(Shared::clone(&pool.clock)),
        None if lists.len() > 1 => Some(Shared::new(Clock::new(0))),
        None => None,
    };
    for (at, list) in lists.iter_mut().enumerate() {
        list.shared_clock = clock.clone();
        list.pool = pool.map(|pool| Link { pool: Shared::clone(pool), me: me.clone(), at, listed: None });
    }
    held.into_iter().map(|(_, key, value)| lists[list_of(&key)].insert(key, value).unwrap_or(1)).sum()
}

// A budget lists of several fonts' caches draw from, each list's bytes drawn from its count, all stamped by one clock
// and evicted in one order: past the budget, the entry queued longest across them goes, or goes round again if read
// since. Lock order: a list's lock, then `heads`; settling takes `heads` only to pop, then one list's lock at a time.
pub(crate) struct Pool {
    clock: Shared<Clock>,
    held: Counter,
    entries: Counter,
    budget: Counter,
    // Each list's oldest entry, by its stamp, where the list holds more than it keeps past any budget.
    heads: Mutable<BTreeMap<u64, (MemberRef, usize)>>,
}

// What holds a pooled cache's lists: list `at` asked to give up its head at `stamp` to `pool`, under its own lock, and
// told what left as the cache would be.
pub(crate) trait Member: Crossing {
    fn evict(&self, pool: &Pool, at: usize, stamp: u64);
}

// What a pool's lists are to cross threads with it under `threading`, as a font is; nothing without.
#[cfg(feature = "threading")]
pub(crate) trait Crossing: Send + Sync + core::panic::RefUnwindSafe + core::panic::UnwindSafe {}
#[cfg(feature = "threading")]
impl<T: Send + Sync + core::panic::RefUnwindSafe + core::panic::UnwindSafe> Crossing for T {}
#[cfg(not(feature = "threading"))]
pub(crate) trait Crossing {}
#[cfg(not(feature = "threading"))]
impl<T> Crossing for T {}

// Weak, so a dropped font's lists leave the pool as they go rather than the pool keeping them.
#[cfg(feature = "threading")]
pub(crate) type MemberRef = alloc::sync::Weak<dyn Member>;
#[cfg(not(feature = "threading"))]
pub(crate) type MemberRef = alloc::rc::Weak<dyn Member>;

// How a pool reaches a cache's lists.
pub(crate) fn member<T: Member + 'static>(lists: &Shared<T>) -> MemberRef {
    Shared::<T>::downgrade(lists)
}

// A list's place in its pool, and the stamp its head is listed at, None where it isn't.
struct Link {
    pool: Shared<Pool>,
    me: MemberRef,
    at: usize,
    listed: Option<u64>,
}

impl<K: Ord + Clone, V> Member for Mutable<ByteLru<K, V>>
where
    Self: Crossing,
{
    fn evict(&self, pool: &Pool, _: usize, stamp: u64) {
        write(self).evict_listed(pool, stamp);
    }
}

impl Pool {
    pub(crate) fn new(budget: usize) -> Pool {
        Pool {
            clock: Shared::new(Clock::new(0)),
            held: Counter::default(),
            entries: Counter::default(),
            budget: Counter::new(budget),
            heads: mutable(BTreeMap::new()),
        }
    }

    pub(crate) fn set_budget(&self, budget: usize) {
        self.budget.set(budget);
        self.settle();
    }

    // Entries and bytes its lists hold.
    pub(crate) fn stats(&self) -> (usize, usize) {
        (self.entries.get(), self.held.get())
    }

    // Evicts the oldest heads until the lists fit, each at its list's lock; a list that went or moved on is passed.
    pub(crate) fn settle(&self) {
        while self.held.get() > self.budget.get() {
            let Some((stamp, (member, at))) = write(&self.heads).pop_first() else { return };
            if let Some(member) = member.upgrade() {
                member.evict(self, at, stamp);
            }
        }
    }
}

// A stamp, unique as each tick is: an atomic under `threading`, where readers stamp at once, else a cell.
#[cfg(feature = "threading")]
pub(crate) struct Clock(core::sync::atomic::AtomicU64);
#[cfg(not(feature = "threading"))]
pub(crate) struct Clock(core::cell::Cell<u64>);

impl Clock {
    pub(crate) fn new(at: u64) -> Clock {
        Clock(at.into())
    }

    #[cfg(feature = "threading")]
    fn get(&self) -> u64 {
        self.0.load(core::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(feature = "threading")]
    fn set(&self, at: u64) {
        self.0.store(at, core::sync::atomic::Ordering::Relaxed)
    }

    #[cfg(feature = "threading")]
    fn tick(&self) -> u64 {
        self.0.fetch_add(1, core::sync::atomic::Ordering::Relaxed) + 1
    }

    #[cfg(not(feature = "threading"))]
    fn get(&self) -> u64 {
        self.0.get()
    }

    #[cfg(not(feature = "threading"))]
    fn set(&self, at: u64) {
        self.0.set(at)
    }

    #[cfg(not(feature = "threading"))]
    fn tick(&self) -> u64 {
        self.0.set(self.0.get() + 1);
        self.0.get()
    }
}

// What a part of a held value was last charged, or that it isn't held: changed only under the cache's lock, so the
// total stays its parts' charges, and a part a thread kept after its value left is charged nothing.
pub(crate) struct Charge(Counter);

const UNHELD: usize = usize::MAX;

impl Default for Charge {
    fn default() -> Charge {
        Charge(Counter::new(UNHELD))
    }
}

impl Charge {
    pub(crate) fn get(&self) -> usize {
        debug_assert_ne!(self.0.get(), UNHELD, "a part charged while not held");
        match self.0.get() {
            UNHELD => 0,
            held => held,
        }
    }

    // As its value comes in: one held already keeps its charge.
    pub(crate) fn hold(&self, bytes: usize) {
        if self.0.get() == UNHELD {
            self.0.set(bytes);
        }
    }

    pub(crate) fn release(&self) {
        self.0.set(UNHELD);
    }

    // Read outside the lock, so asked again under it by `recharge`.
    pub(crate) fn due(&self, bytes: usize) -> bool {
        let was = self.0.get();
        was != UNHELD && was != bytes
    }

    // How much more it is charged, where it is held and changed.
    pub(crate) fn recharge(&self, bytes: usize) -> Option<isize> {
        let was = self.0.get();
        (was != UNHELD && was != bytes).then(|| {
            self.0.set(bytes);
            bytes as isize - was as isize
        })
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/cache/lru.rs"]
mod tests;
