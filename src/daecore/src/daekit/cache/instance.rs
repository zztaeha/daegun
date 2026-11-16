use super::*;

// Which of a location's caches: the one drawing reads, and the one metrics, extents and shaping read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Drawn,
    Described,
}

fn slot_of(at: &Located, slot: Slot) -> &Option<Result<Shared<FontCache>, Error>> {
    match slot {
        Slot::Drawn => &at.drawn,
        Slot::Described => &at.described,
    }
}

fn slot_of_mut(at: &mut Located, slot: Slot) -> &mut Option<Result<Shared<FontCache>, Error>> {
    match slot {
        Slot::Drawn => &mut at.drawn,
        Slot::Described => &mut at.described,
    }
}

// What a described location builds when first read there: the tables its instance holds, the ones it varies built as
// the instancer builds them, and its name, None where it names itself as the font does.
pub(crate) struct Described {
    key: AxisKey,
    // Set where locations are built, so a program that reads none has no path to the instancer from a table's read.
    build: fn(&FontCache, &AxisKey) -> Tables,
    tables: Once<Tables>,
    name: Once<Option<TableBytes>>,
}

impl Described {
    // Out of line, so a read at the font's own cache carries only the check.
    #[inline(never)]
    fn tables<'a>(&'a self, at: &'a FontCache) -> &'a Tables {
        once_slot(&self.tables, || (self.build)(at, &self.key), |_| {})
    }
}

// What this thread last read at a location through one slot: the font, how many locations had left its budget
// then, the key, and the location's cache, None where it can't be built.
#[cfg(feature = "std")]
struct Recent {
    font: Shared<FontState>,
    dropped: usize,
    key: AxisKey,
    at: Option<Shared<FontCache>>,
}

#[cfg(feature = "std")]
std::thread_local! {
    // Each slot's most recent location, read again without the font's lock or a search. The thread holds it, uncounted
    // once the budget drops it, until it finds it gone or another asked for on its next read, or ends.
    static RECENT: core::cell::RefCell<[Option<Recent>; 2]> = const { core::cell::RefCell::new([None, None]) };
}

// How many resolved locations a thread keeps under `threading`, and a font without it, newest last.
const KEPT_AXES: usize = 8;

// A location a thread has resolved, under `threading`, in place of the font's map: its locks took 19× as long a
// location at 8 threads, and a thread's own copies are counted by it alone. None holds a font's bytes.
#[cfg(feature = "threading")]
struct ThreadAxes {
    cache: Id,
    at: Shared<Lined<ResolvedAxes>>,
}

// `value` in `held`'s allocation where nothing else holds it, else in a new one: one on lines of its own costs more
// to make than a plain one.
fn refill<T>(held: Option<Shared<Lined<T>>>, value: T) -> Shared<Lined<T>> {
    let Some(mut held) = held else { return Shared::new(Lined(value)) };
    match Shared::get_mut(&mut held) {
        Some(at) => at.0 = value,
        None => return Shared::new(Lined(value)),
    }
    held
}

#[cfg(feature = "threading")]
std::thread_local! {
    static RESOLVED: core::cell::Cell<Vec<ThreadAxes>> = const { core::cell::Cell::new(Vec::new()) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = RECENT.try_with(|recent| recent.try_borrow_mut().ok().map(|mut slots| core::mem::take(&mut *slots)));
    #[cfg(feature = "threading")]
    let _ = RESOLVED.try_with(core::cell::Cell::take);
}

impl FontCache {
    // The cache's tables as its readers take them: a described location's as its instance holds them, built at the
    // first read of the tables it varies.
    #[inline]
    pub(crate) fn tables(&self) -> &Tables {
        match &self.described {
            None => &self.font.tables,
            Some(described) => described.tables(self),
        }
    }

    // One table as `tables` holds it: a described location builds its instance's only for one its instance varies.
    #[inline]
    pub(crate) fn table(&self, tag: &str) -> Option<&TableBytes> {
        use crate::daecore::daetype::instancer::described_varies;
        match &self.described {
            Some(described) if described_varies(tag) => described.tables(self).get(tag),
            Some(_) => self.font.described_held().get(tag),
            None => self.font.tables.get(tag),
        }
    }

    pub(crate) fn has_table(&self, tag: &str) -> bool {
        self.table(tag).is_some()
    }

    // The font's tables as stored, which the font's own cache and a drawn location hold: read without the check a
    // described location needs.
    #[inline]
    pub(crate) fn font_tables(&self) -> &Tables {
        debug_assert!(self.described.is_none());
        &self.font.tables
    }

    // The instance's tables at `key`, those it leaves as they are sharing the font's bytes, the location charged for
    // those it changes; outside the ones it varies, they are the tables it keeps.
    fn build_described(&self, key: &AxisKey) -> Tables {
        use crate::daecore::daetype::instancer;
        use crate::daecore::daekit::base::sync::{heap_of, shared_of};
        use alloc::borrow::Cow;
        let (font, kept) = (&self.font.tables, self.font.described_held());
        let Some(reads) = self.font.instance_reads.get() else {
            debug_assert!(false, "a described location is built after the font's reads are kept");
            return kept.clone();
        };
        let Ok(built) = instancer::instance_tables_described(font, reads, key) else {
            debug_assert!(false, "a described location's instance fails as its kept tables do");
            return kept.clone();
        };
        let (mut owned, mut map) = (0, kept.clone());
        for (tag, data) in built {
            let unchanged = font.get(&tag).filter(|t| match &data {
                Cow::Borrowed(bytes) => core::ptr::eq(t.as_slice(), *bytes),
                Cow::Owned(_) => false,
            });
            let bytes = match unchanged {
                Some(t) => t.clone(),
                None => {
                    let data = data.into_owned();
                    owned += heap_of(&data) + shared_of::<Vec<u8>>();
                    TableBytes::from_vec(data)
                }
            };
            map.insert(tag, bytes);
        }
        let at = |tables: &Tables, tag: &str| tables.get(tag).map(|t| t.as_slice() as *const [u8]);
        let same = |tag: &str| instancer::described_varies(tag) || at(&map, tag) == at(kept, tag);
        debug_assert!(map.keys().chain(kept.keys()).all(same));
        self.held.built.add(owned + map.heap_bytes());
        map
    }

    // A font whose fvar is missing or doesn't parse is static: its own tables are its instance anywhere.
    pub(crate) fn is_static(&self) -> bool {
        crate::daecore::daetype::decoder::parse_fvar_axes(self.tables()).is_err()
    }

    // A static font is its own instance to subset too, unless its outlines are CFF2, which a subset takes pinned.
    fn is_own_instance(&self) -> bool {
        self.is_static() && !self.cff2
    }

    // The sanitizer's verdicts an instance takes of the font's tables, judged once, on the font's own cache: a
    // location took 23 µs to judge Inter's again.
    pub(crate) fn instance_reads(&self) -> &crate::daecore::daetype::format::sanitize::Reads {
        use crate::daecore::daetype::format::sanitize::Reads;
        debug_assert!(self.location.is_none(), "a described location's tables are its instance's");
        once_slot(&self.font.instance_reads, || Reads::of_tables(self.tables()), |_| {})
    }

    // The whole font at a location, or the instancer's error.
    pub fn get_or_instance(&self, key: &AxisKey) -> Result<alloc::sync::Arc<[u8]>, Error> {
        if let Some(held) = write(&self.locations().list).get(key).and_then(|at| at.instance.clone()) {
            return held;
        }
        use crate::daecore::daetype::decoder::{build_ttf, copies_held};
        let built = if self.is_static() {
            copies_held(self.tables()).and_then(|()| build_ttf(self.tables()))
        } else {
            crate::daecore::daetype::instancer::instance_font_from_map(self.tables(), self.instance_reads(), key)
        };
        let built = built.map(alloc::sync::Arc::from);
        self.locate(key, |at| at.instance = Some(built.clone()));
        built
    }

    // What subsets read at a location, built once there.
    pub(crate) fn subset_face(&self, key: &AxisKey) -> Result<Shared<super::subset::SubsetFace>, Error> {
        if let Some(held) = write(&self.locations().list).get(key).and_then(|at| at.subset.clone()) {
            return held;
        }
        let own = !self.cff2 && (self.has_table("CFF ") || self.is_own_instance());
        let reads = self.instance_reads();
        let built = super::subset::SubsetFace::new(self.tables(), reads, self.cff2, own, key).map(Shared::new);
        let mut kept = built.clone();
        self.locate(key, |at| kept = at.subset.get_or_insert(built).clone());
        kept
    }

    // `f` on the cache drawing reads at a location: the font's own tables with its glyphs moved as each is drawn,
    // the default's where none can be built. The location is then charged for whatever `f` built there.
    pub(crate) fn at_location<T>(&self, key: &AxisKey, f: impl FnOnce(&FontCache) -> T) -> T {
        self.located(key, Slot::Drawn, f)
    }

    // `f` on the font as measuring reads it at a location: metrics, layout and style there, glyphs measured as
    // first read. The default reads the font's own tables, which cover its whole design space, as does a failed one.
    pub(crate) fn described_at<T>(&self, key: &AxisKey, f: impl FnOnce(&FontCache) -> T) -> T {
        self.located(key, Slot::Described, f)
    }

    fn located<T>(&self, key: &AxisKey, slot: Slot, f: impl FnOnce(&FontCache) -> T) -> T {
        if key.is_empty() {
            return f(self);
        }
        self.varied(key, slot, f)
    }

    // Out of line, so a call at the default doesn't carry this path's frame and thread-local reads.
    #[inline(never)]
    fn varied<T>(&self, key: &AxisKey, slot: Slot, f: impl FnOnce(&FontCache) -> T) -> T {
        #[cfg(feature = "std")]
        let f = match self.recent(key, slot, f) {
            Ok(out) => return out,
            Err(f) => f,
        };
        let (at, dropped) = self.location_cache(key, slot);
        if let Some(dropped) = dropped {
            self.remember(key, slot, &at, dropped);
        }
        match at {
            Some(at) => self.charged(&at, f),
            None => f(self),
        }
    }

    // `f` on the location this thread last read through `slot`, if it is this one and no location has left the
    // budget since, or `f` back. Such a read takes no lock and writes nothing another thread reads.
    #[cfg(feature = "std")]
    fn recent<T, F: FnOnce(&FontCache) -> T>(&self, key: &AxisKey, slot: Slot, f: F) -> Result<T, F> {
        let dropped = self.locations().dropped.get();
        let mut f = Some(f);
        let read = RECENT.try_with(|recent| {
            let recent = recent.try_borrow().ok()?;
            let r = recent[slot as usize].as_ref()?;
            if !Shared::ptr_eq(&r.font, &self.font) || r.dropped != dropped || r.key != *key {
                return None;
            }
            let f = f.take()?;
            Some(match &r.at {
                Some(at) => self.charged(at, f),
                None => f(self),
            })
        });
        match (read, f) {
            (Ok(Some(out)), _) => Ok(out),
            (_, Some(f)) => Err(f),
            (_, None) => unreachable!("`f` is taken only by a read that answers"),
        }
    }

    #[cfg(feature = "std")]
    fn remember(&self, key: &AxisKey, slot: Slot, at: &Option<Shared<FontCache>>, dropped: usize) {
        let _ = RECENT.try_with(|recent| {
            if let Ok(mut recent) = recent.try_borrow_mut() {
                let font = Shared::clone(&self.font);
                recent[slot as usize] = Some(Recent { font, dropped, key: key.clone(), at: at.clone() });
            }
        });
    }

    #[cfg(not(feature = "std"))]
    fn remember(&self, _: &AxisKey, _: Slot, _: &Option<Shared<FontCache>>, _: usize) {}

    // The location's cache, None where it can't be built, and how many locations had left the budget by then, None
    // where it isn't kept: threads queueing on a busy list took 10× as long a new location, so a later read keeps it.
    fn location_cache(&self, key: &AxisKey, slot: Slot) -> (Option<Shared<FontCache>>, Option<usize>) {
        if let Some(locations) = try_write(&self.locations().list)
            && let Some(held) = locations.get(key).and_then(|at| slot_of(at, slot).as_ref())
        {
            return (held.as_ref().ok().cloned(), Some(self.locations().dropped.get()));
        }
        let built = self.build_location(key, slot);
        let Some(mut locations) = try_write(&self.locations().list) else { return (built.ok(), None) };
        let mut kept = None;
        let change = |at: &mut Located| kept = slot_of_mut(at, slot).get_or_insert(built).as_ref().ok().cloned();
        let dropped = self.locate_in(&mut locations, key, change);
        let over = locations.over();
        drop(locations);
        lru::settle(over);
        (kept, Some(dropped))
    }

    // `f` on a location's cache, charged then for what it grew by: no search, as the cache keeps its own charge.
    fn charged<T>(&self, at: &FontCache, f: impl FnOnce(&FontCache) -> T) -> T {
        let out = f(at);
        self.recharge(&at.charge, at.held_bytes());
        out
    }

    pub(super) fn recharge(&self, charge: &super::lru::Charge, held: usize) {
        // A busy list is charged at a later read.
        if charge.due(held)
            && let Some(mut locations) = try_write(&self.locations().list)
            && let Some(by) = charge.recharge(held)
        {
            let evicted = locations.adjust(by);
            self.dropped(evicted);
            let over = locations.over();
            drop(locations);
            lru::settle(over);
        }
    }

    // Changes what a location holds, which charges it again: how many locations have left the budget once it has.
    fn locate(&self, key: &AxisKey, change: impl FnOnce(&mut Located)) -> usize {
        let mut locations = write(&self.locations().list);
        let dropped = self.locate_in(&mut locations, key, change);
        let over = locations.over();
        drop(locations);
        lru::settle(over);
        dropped
    }

    fn locate_in(
        &self, locations: &mut lru::ByteLru<AxisKey, Located>, key: &AxisKey, change: impl FnOnce(&mut Located),
    ) -> usize {
        let mut at = locations.get(key).cloned().unwrap_or_default();
        change(&mut at);
        at.hold();
        let evicted = locations.insert(key.clone(), at).unwrap_or(0);
        self.dropped(evicted)
    }

    // Counts locations the budget evicted, called with the lock held: how many have left it so far.
    pub(super) fn dropped(&self, evicted: usize) -> usize {
        if evicted > 0 {
            self.locations().dropped.add(evicted);
        }
        self.locations().dropped.get()
    }

    // A described location holds the font's tables less those its instance drops, building the ones it varies when
    // first read. Its glyphs are measured from the font's own outlines, which every location shares.
    fn build_location(&self, key: &AxisKey, slot: Slot) -> Result<Shared<FontCache>, Error> {
        use crate::daecore::daetype::instancer;
        use crate::daecore::daekit::base::sync::shared_of;
        if slot == Slot::Drawn {
            return Ok(Shared::new(self.drawn_location(key)));
        }
        let reads = self.instance_reads();
        let kept = || instancer::described_kept(&self.font.tables, reads, self.normalizer().as_ref());
        if let Err(error) = once_slot(&self.font.described_tables, kept, |_| {}) {
            return Err(error.clone());
        }
        let build = FontCache::build_described;
        let described = Described { key: key.clone(), build, tables: Once::new(), name: Once::new() };
        let mut owned = size_of::<Described>() + described.key.heap_bytes();
        let mut at = FontCache::with_header(Shared::clone(&self.font), true, self.upm, self.num_glyphs);
        at.described = Some(alloc::boxed::Box::new(described));
        // Its glyphs lie where the font's do, loca's format read from a head its style pass changes only in macStyle.
        self.loca_offsets();
        let _ = at.loca.set(self.loca.get().cloned().flatten());
        let resolved = self.resolve_axes(key);
        let location = &resolved.coords[..];
        // The location's tables leave gvar out, so glyphs measured anywhere, as VARC's leaves are, read the font's.
        let _ = at.glyf_tables.set(self.glyf_tables().cloned());
        if location.iter().any(|&v| v != 0.0) {
            at.measured = self.glyf_tables().map(|t| Shared::new(instancer::MeasuredGlyphs::new(t.clone(), location)));
            owned += at.measured.as_ref().map_or(0, |_| shared_of::<instancer::MeasuredGlyphs>());
        }
        let measured = at.measured.as_ref();
        let views = once_slot(&self.font.advance_views, || instancer::advance_views(&self.font.tables), |_| {});
        at.varied_advances = [false, true].map(|v| {
            let view = views[usize::from(v)].as_ref();
            instancer::VariedAdvances::new(self.tables(), v, location, measured, view, &self.font.spare)
        });
        at.advances_past = at.varied_advances.each_ref().map(|v| v.as_ref().map_or(0, instancer::VariedAdvances::past));
        let origins = instancer::VariedOrigins::new(self.tables(), location, &self.font.spare);
        at.varied_origins = origins.map(alloc::boxed::Box::new);
        let advances: usize = at.varied_advances.iter().flatten().map(instancer::VariedAdvances::heap_bytes).sum();
        let origins = |o: &instancer::VariedOrigins| size_of::<instancer::VariedOrigins>() + o.heap_bytes();
        owned += advances + at.varied_origins.as_deref().map_or(0, origins);
        at.location = Some(resolved);
        at.held.built.add(owned);
        Ok(Shared::new(at))
    }

    // `f` on the tables names are read from at `key`: its name, a location's built on the first read there and
    // charged to it, and its STAT.
    pub(crate) fn names_at<T>(&self, key: &AxisKey, f: impl FnOnce(&Tables) -> T) -> T {
        use crate::daecore::daekit::base::sync::{heap_of, shared_of};
        self.described_at(key, |at| {
            let built = || {
                let name = crate::daecore::daetype::instancer::described_name(self.tables(), key)?;
                at.held.built.add(heap_of(&name) + shared_of::<Vec<u8>>());
                Some(TableBytes::from_vec(name))
            };
            let name = match &at.described {
                Some(described) => described.name.get_or_init(built).clone().or_else(|| self.table("name").cloned()),
                None => at.table("name").cloned(),
            };
            let tables = [("name", name), ("STAT", at.table("STAT").cloned())];
            f(&tables.into_iter().filter_map(|(tag, table)| Some((tag, table?))).collect())
        })
    }

    // The font's own tables, shared with the location, and its glyphs moved as each is drawn: glyf's by their deltas,
    // CFF2's blended at its coordinates and kept.
    fn drawn_location(&self, key: &AxisKey) -> FontCache {
        let mut at = FontCache::with_header(Shared::clone(&self.font), false, self.upm, self.num_glyphs);
        if self.cff2 {
            let coords = crate::daecore::daetype::instancer::drawn_coordinates(self.tables(), key);
            at.cff2_at = coords.map(|c| super::drawn::Cff2Location::new(c, self.num_glyphs));
            return at;
        }
        self.loca_offsets();
        let _ = at.loca.set(self.loca.get().cloned().flatten());
        at.drawn = self.drawn_glyphs(key);
        if self.has_table("VARC") {
            let coords = crate::daecore::daetype::instancer::drawn_coordinates(self.tables(), key);
            let coords = coords.map(Vec::into_boxed_slice);
            at.held.built.add(coords.as_deref().map_or(0, core::mem::size_of_val));
            at.varc_at = coords;
        }
        at
    }

    // The key and coordinates `axes` resolve to, in one block every location made of them holds.
    #[cfg(feature = "threading")]
    pub fn resolve_axes(&self, axes: &AxisKey) -> Shared<Lined<ResolvedAxes>> {
        let mut mine = RESOLVED.try_with(core::cell::Cell::take).unwrap_or_default();
        let held = match mine.iter().position(|t| t.cache == self.runs_id && t.at.key == *axes) {
            Some(at) => mine.remove(at),
            None => {
                // The oldest gives its place, and its block where no location holds it still.
                let old = (mine.len() == KEPT_AXES).then(|| mine.remove(0).at);
                ThreadAxes { cache: self.runs_id, at: refill(old, self.resolved(axes)) }
            }
        };
        let at = Shared::clone(&held.at);
        mine.push(held);
        let _ = RESOLVED.try_with(|slot| slot.set(mine));
        at
    }

    #[cfg(not(feature = "threading"))]
    pub fn resolve_axes(&self, axes: &AxisKey) -> Shared<Lined<ResolvedAxes>> {
        if axes.is_empty() {
            return Shared::clone(self.default_axes());
        }
        let mut kept = write(&self.location_by_axis);
        let at = match kept.iter().position(|at| at.key == *axes) {
            Some(at) => kept.remove(at),
            None => {
                let old = (kept.len() == KEPT_AXES).then(|| kept.remove(0));
                refill(old, self.resolved(axes))
            }
        };
        kept.push(Shared::clone(&at));
        at
    }

    // The default's key and coordinates, built once. Readers borrow it: a count every thread bumped would pass its
    // line between them.
    pub fn default_axes(&self) -> &Shared<Lined<ResolvedAxes>> {
        self.default_axes.get_or_init(|| Shared::new(Lined(self.resolved(&EMPTY_KEY))))
    }

    fn resolved(&self, axes: &AxisKey) -> ResolvedAxes {
        let coords = self.normalizer().as_ref().map_or_else(|_| Vec::new(), |n| n.location(axes));
        ResolvedAxes { key: axes.clone(), coords }
    }

    fn normalizer(&self) -> &Result<crate::daecore::daetype::instancer::Normalizer, Error> {
        use crate::daecore::daetype::instancer::Normalizer;
        once_slot(&self.font.normalizer, || Normalizer::read(&self.font.tables), |_| {})
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/cache/instance.rs"]
mod tests;
