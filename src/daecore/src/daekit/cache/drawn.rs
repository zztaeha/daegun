use super::*;
use crate::daecore::daetype::format::glyf::{
    glyph_bytes, simple_point_count, ComponentPath, Components, GLYPH_WORK, USE_MY_METRICS,
};
use crate::daecore::daetype::instancer::{
    extract_coords_into, fixed_to_f26dot6, fixed_to_int, DeltaRoom, DeltaSource, DrawnLocation, GlyphCoords, Gvar,
};
use crate::daecore::daetype::outline::{Deltas, Fine, Moves, OutlinePen, Path, Shape, Stored, Vary};

type Held<T> = Option<Shared<[T]>>;

// A location's glyphs as drawn there: each one's whole-unit deltas from its first draw, kept where it doesn't vary too,
// its 26.6 deltas from its first hint, and the cvt and coordinates hinting reads.
pub(crate) struct DrawnGlyphs {
    font: Shared<FontState>,
    location: DrawnLocation,
    glyphs: Kept<(i16, i16)>,
    fine: Kept<(i32, i32)>,
    cvt: Option<Vec<i32>>,
    coords: Vec<i32>,
    bytes: crate::daecore::daekit::base::sync::Counter,
    n_glyphs: usize,
    // Whether glyf is GLYF, whose composites can name a glyph in 24 bits.
    extended: bool,
}

// Deltas kept by glyph id, read without a lock in pages of 64 slots as `KeptOutlines` is, laid out at the first glyph
// kept: a lock for each glyph took 13× as long a draw at 8 threads.
#[cfg(feature = "threading")]
#[derive(Default)]
struct Kept<T> {
    pages: Once<alloc::boxed::Box<[KeptPage<T>]>>,
}

#[cfg(feature = "threading")]
type KeptPage<T> = Once<alloc::boxed::Box<[Once<(Held<T>, usize)>; 64]>>;

#[cfg(feature = "threading")]
impl<T> Kept<T> {
    fn get(&self, gid: u32) -> Option<(Held<T>, usize)> {
        self.pages.get()?.get(gid as usize / 64)?.get()?[gid as usize % 64].get().cloned()
    }

    // How many bytes keeping it took: none where another thread kept it first.
    fn put(&self, gid: u32, held: Held<T>, cost: usize, n_glyphs: usize, _: &FontState) -> usize {
        let mut grew = 0;
        let lay_out = || (0..n_glyphs.div_ceil(64)).map(|_| Once::new()).collect();
        let pages = once_slot(&self.pages, lay_out, |p: &alloc::boxed::Box<[_]>| grew += core::mem::size_of_val(&**p));
        let lay_out = || alloc::boxed::Box::new(core::array::from_fn(|_| Once::new()));
        let page = once_slot(&pages[gid as usize / 64], lay_out, |p| grew += core::mem::size_of_val(&**p));
        let size = held.as_ref().map_or(0, held_size);
        match page[gid as usize % 64].set((held, cost)) {
            Ok(()) => grew + size,
            Err(_) => grew,
        }
    }

    fn give_back(&self, _: &FontState) {}
}

// Deltas kept by glyph id: each glyph's place among them plus one, 0 until it's kept, so a draw finds them without
// search. The places are taken from the font's spares at the first glyph kept.
#[cfg(not(feature = "threading"))]
#[derive(Default)]
struct Kept<T>(Mutable<KeptPlaces<T>>);

#[cfg(not(feature = "threading"))]
struct KeptPlaces<T> {
    at: GlyphSlots,
    held: Vec<(Held<T>, usize)>,
}

#[cfg(not(feature = "threading"))]
impl<T> Default for KeptPlaces<T> {
    fn default() -> KeptPlaces<T> {
        KeptPlaces { at: GlyphSlots::default(), held: Vec::new() }
    }
}

#[cfg(not(feature = "threading"))]
impl<T> Kept<T> {
    fn get(&self, gid: u32) -> Option<(Held<T>, usize)> {
        let kept = read(&self.0);
        kept.held.get((kept.at.get(gid) as usize).checked_sub(1)?).cloned()
    }

    // How many bytes keeping it took.
    fn put(&self, gid: u32, held: Held<T>, cost: usize, n_glyphs: usize, font: &FontState) -> usize {
        let mut kept = write(&self.0);
        if (kept.at.get(gid) as usize).checked_sub(1).is_some_and(|at| at < kept.held.len()) {
            return 0;
        }
        let before = kept.at.heap_bytes() + heap_of(&kept.held);
        if kept.at.is_empty() {
            kept.at = font.spare.take(n_glyphs);
        }
        let size = held.as_ref().map_or(0, held_size);
        kept.held.push((held, cost));
        let place = kept.held.len() as u32;
        kept.at.set(gid, place);
        size + kept.at.heap_bytes() + heap_of(&kept.held) - before
    }

    fn give_back(&self, font: &FontState) {
        font.spare.give(core::mem::take(&mut write(&self.0).at));
    }
}

fn held_size<T>(d: &Shared<[T]>) -> usize {
    shared_of::<()>() + core::mem::size_of_val(&**d)
}

// Past a draw's work, one error wherever it ran out, as known deltas are spent whole rather than as worked out.
const OVER: Error = Error::limit("glyf: the glyph's work budget exhausted");

// `n` units of work taken from what's `left` to a draw.
fn spend(left: &mut usize, n: usize) -> Result<(), Error> {
    *left = left.checked_sub(n).ok_or(OVER)?;
    Ok(())
}

// The glyph's 16.16 deltas cut by `cut`, none where all are 0; working them out, a composite's components read
// among it, is spent from `left`. A composite's points are its components' offsets, in no contour, so none
// interpolates and where they start doesn't matter, as for the phantoms.
fn varied<T: PartialEq + Default>(
    source: DeltaSource<'_>, extended: bool, gid: u32, shape: Shape<'_>, cut: impl Fn(i64) -> T, left: &mut usize,
) -> Result<Held<(T, T)>, Error> {
    let (n, x, y, ends): (usize, &[i32], &[i32], &[usize]) = match shape {
        Shape::Empty => (0, &[], &[], &[]),
        Shape::Simple { x, y, ends } => (x.len(), x, y, ends),
        Shape::Composite(glyph) => {
            let n = Components::new(glyph, extended).try_fold(0, |n, c| c.map(|_| n + 1));
            let n = n.map_err(|_| Error::malformed("glyf: composite stream truncated"))?;
            spend(left, n)?;
            (n, &[], &[], &[])
        }
    };
    let mut room = Room::take();
    let held = source.deltas(gid as usize, n + 4, (x, y), ends, &mut room, left).map_err(|_| OVER).map(|deltas| {
        let cut = |&(x, y): &(i64, i64)| (cut(x), cut(y));
        let deltas = deltas?;
        deltas.iter().map(&cut).any(|d| d != Default::default()).then(|| deltas.iter().map(cut).collect())
    });
    Room::give(room);
    held
}

// A location only a VARC component draws at, its glyphs worked out once a leaf's load and kept for its origin and its
// points: the last apart from the rest, which only a composite leaf has.
pub(crate) struct Passing<'a> {
    pub(crate) source: DeltaSource<'a>,
    pub(crate) extended: bool,
    pub(crate) last: core::cell::RefCell<Option<(u32, Deltas)>>,
    pub(crate) before: core::cell::RefCell<BTreeMap<u32, Deltas>>,
}

impl Vary for Passing<'_> {
    // Asked only within the leaf's load that worked them out, after it did, so they cost nothing more.
    fn known(&self, gid: u32, _: &mut usize) -> Option<Result<Deltas, Error>> {
        let last = self.last.borrow().as_ref().filter(|last| last.0 == gid).map(|last| last.1.clone());
        last.or_else(|| self.before.borrow().get(&gid).cloned()).map(Ok)
    }

    fn deltas(&self, gid: u32, shape: Shape<'_>, left: &mut usize) -> Result<Deltas, Error> {
        if let Some(known) = self.known(gid, left) {
            return known;
        }
        let deltas = varied(self.source, self.extended, gid, shape, fixed_to_int, left)?.map(Moves::from);
        if let Some((older, held)) = self.last.borrow_mut().replace((gid, deltas.clone())) {
            self.before.borrow_mut().insert(older, held);
        }
        Ok(deltas)
    }
}

// Its indexes go back to the font's spares.
impl Drop for DrawnGlyphs {
    fn drop(&mut self) {
        self.glyphs.give_back(&self.font);
        self.fine.give_back(&self.font);
    }
}

impl DrawnGlyphs {
    pub(crate) fn bytes(&self) -> usize {
        self.bytes.get()
    }

    pub(crate) fn hinting(&self) -> (Option<&[i32]>, &[i32]) {
        (self.cvt.as_deref(), &self.coords)
    }

    // A glyph past the font's count is worked out at each draw. The rest are kept with their `cost`, which each use
    // spends again.
    fn keep<T>(&self, kept: &Kept<T>, gid: u32, held: Held<T>, cost: usize) -> Held<T> {
        if (gid as usize) < self.n_glyphs {
            self.bytes.add(kept.put(gid, held.clone(), cost, self.n_glyphs, &self.font));
        }
        held
    }
}

impl Vary for DrawnGlyphs {
    // Inlined into `Paying`'s, which each draw at a location goes through.
    #[inline]
    fn known(&self, gid: u32, left: &mut usize) -> Option<Result<Deltas, Error>> {
        let (held, cost) = self.glyphs.get(gid)?;
        Some(spend(left, cost).map(|()| held.map(Moves::from)))
    }

    fn deltas(&self, gid: u32, shape: Shape<'_>, left: &mut usize) -> Result<Deltas, Error> {
        if let Some(held) = self.known(gid, left) {
            return held;
        }
        let before = *left;
        let held = varied(self.location.source(), self.extended, gid, shape, fixed_to_int, left)?;
        Ok(self.keep(&self.glyphs, gid, held, before - *left).map(Moves::from))
    }

    fn located(&self) -> bool {
        true
    }

    fn deltas_f26dot6(&self, gid: u32, shape: Shape<'_>, left: &mut usize) -> Result<Fine, Error> {
        if let Some((held, cost)) = self.fine.get(gid) {
            return spend(left, cost).map(|()| held);
        }
        let before = *left;
        let fine = varied(self.location.source(), self.extended, gid, shape, fixed_to_f26dot6, left)?;
        Ok(self.keep(&self.fine, gid, fine, before - *left))
    }

    // Inlined into `Paying`'s, which each draw at a location goes through.
    #[inline]
    fn known_f26dot6(&self, gid: u32, left: &mut usize) -> Option<Result<Fine, Error>> {
        let (held, cost) = self.fine.get(gid)?;
        Some(spend(left, cost).map(|()| held))
    }

    fn advance(&self, gid: u32, vertical: bool, advance: u16) -> Option<u16> {
        let table = if vertical { &self.location.vvar } else { &self.location.hvar };
        table.as_ref().map(|t| t.adjust(gid as usize, advance))
    }
}

use crate::daecore::daekit::base::sync::{heap_of, shared_of, Counter};
#[cfg(not(feature = "threading"))]
use crate::daecore::daekit::base::sync::GlyphSlots;

// What a font keeps of its glyphs as drawn, CFF2's outlines or glyf's points, stops growing past this: subroutines
// can draw far more points than the font holds, and glyf can hold many megabytes.
const KEPT_OUTLINE_BYTES: usize = 16 * 1024 * 1024;

// Each CFF2 glyph's outline as first drawn, replayed at its next draw: pages of 64 slots, each laid out at its first
// glyph, so a draw reads its glyph without a lock.
pub(crate) struct KeptOutlines {
    pages: alloc::boxed::Box<[Once<alloc::boxed::Box<Page>>]>,
    bytes: Counter,
}

type Page = [Once<Path>; 64];

impl KeptOutlines {
    pub(crate) fn new(n_glyphs: u32) -> KeptOutlines {
        let pages: alloc::boxed::Box<[_]> = (0..(n_glyphs as usize).div_ceil(64)).map(|_| Once::new()).collect();
        let bytes = Counter::new(core::mem::size_of::<KeptOutlines>() + core::mem::size_of_val(&*pages));
        KeptOutlines { pages, bytes }
    }

    pub(crate) fn get(&self, gid: u32) -> Option<&Path> {
        self.pages.get(gid as usize / 64)?.get()?[gid as usize % 64].get()
    }

    // `draw` into `pen`, recorded as it goes and kept where it draws; once the store is full, not recorded.
    pub(crate) fn record<T>(
        &self, gid: u32, pen: &mut dyn OutlinePen, draw: impl FnOnce(&mut dyn OutlinePen) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if self.bytes.get() >= KEPT_OUTLINE_BYTES {
            return draw(pen);
        }
        let mut path = Recording::take();
        path.truncate(0, 0);
        let drawn = draw(&mut Tee { pen, path: &mut path });
        if drawn.is_ok() {
            self.keep(gid, path.clone());
        }
        Recording::give(path);
        drawn
    }

    fn keep(&self, gid: u32, path: Path) {
        let Some(page) = self.pages.get(gid as usize / 64).filter(|_| self.bytes.get() < KEPT_OUTLINE_BYTES) else {
            return;
        };
        let lay_out = || alloc::boxed::Box::new(core::array::from_fn(|_| Once::new()));
        let page = once_slot(page, lay_out, |p| self.bytes.add(core::mem::size_of_val(&**p)));
        let cost = path.cost();
        if page[gid as usize % 64].set(path).is_ok() {
            self.bytes.add(cost);
        }
    }
}

// Simple glyphs' points as decoded at their first draw at any location, copied out at every draw after that one, so
// no location decodes them again; pages of 64 slots, as `KeptOutlines`, bounded as it is.
pub(crate) struct KeptPoints {
    pages: alloc::boxed::Box<[Once<alloc::boxed::Box<PointsPage>>]>,
    bytes: Counter,
}

type PointsPage = [Once<KeptGlyph>; 64];

struct KeptGlyph {
    x: alloc::boxed::Box<[i32]>,
    y: alloc::boxed::Box<[i32]>,
    flags: alloc::boxed::Box<[u8]>,
    ends: alloc::boxed::Box<[usize]>,
}

impl KeptPoints {
    pub(crate) fn new(n_glyphs: u32) -> KeptPoints {
        let pages: alloc::boxed::Box<[_]> = (0..(n_glyphs as usize).div_ceil(64)).map(|_| Once::new()).collect();
        let bytes = Counter::new(core::mem::size_of::<KeptPoints>() + core::mem::size_of_val(&*pages));
        KeptPoints { pages, bytes }
    }

    // `glyph`'s points into `scratch` as `extract_coords_into` decodes them, copied from the store where it holds
    // them, and kept there where they decode and the store has room.
    pub(crate) fn decode_into(&self, gid: u32, glyph: &[u8], n_contours: usize, scratch: &mut GlyphCoords) -> bool {
        let page = self.pages.get(gid as usize / 64);
        if let Some(kept) = page.and_then(Once::get).and_then(|p| p[gid as usize % 64].get()) {
            let GlyphCoords { x_coords, y_coords, flags, end_pts, num_points, decoded } = scratch;
            for (to, from) in [(x_coords, &kept.x), (y_coords, &kept.y)] {
                to.clear();
                to.extend_from_slice(from);
            }
            flags.clear();
            flags.extend_from_slice(&kept.flags);
            end_pts.clear();
            end_pts.extend_from_slice(&kept.ends);
            (*num_points, *decoded) = (kept.x.len(), None);
            return true;
        }
        let read = extract_coords_into(glyph, n_contours, scratch);
        if let Some(page) = page.filter(|_| read && self.bytes.get() < KEPT_OUTLINE_BYTES) {
            let lay_out = || alloc::boxed::Box::new(core::array::from_fn(|_| Once::new()));
            let page = once_slot(page, lay_out, |p| self.bytes.add(core::mem::size_of_val(&**p)));
            let kept = KeptGlyph {
                x: scratch.x_coords.as_slice().into(),
                y: scratch.y_coords.as_slice().into(),
                flags: scratch.flags.as_slice().into(),
                ends: scratch.end_pts.as_slice().into(),
            };
            let cost = core::mem::size_of_val(&*kept.x) * 2 + kept.flags.len() + core::mem::size_of_val(&*kept.ends);
            if page[gid as usize % 64].set(kept).is_ok() {
                self.bytes.add(cost);
            }
        }
        read
    }
}

struct Tee<'a> {
    pen: &'a mut dyn OutlinePen,
    path: &'a mut Path,
}

impl OutlinePen for Tee<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.pen.move_to(x, y);
        self.path.move_to(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.pen.line_to(x, y);
        self.path.line_to(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.pen.quad_to(cx, cy, x, y);
        self.path.quad_to(cx, cy, x, y);
    }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.pen.curve_to(c1x, c1y, c2x, c2y, x, y);
        self.path.curve_to(c1x, c1y, c2x, c2y, x, y);
    }
    fn close(&mut self) {
        self.pen.close();
        self.path.close();
    }
}

// The path a first draw is recorded in, kept by its thread so that its buffers are reused.
struct Recording;

#[cfg(feature = "std")]
std::thread_local! {
    static RECORDING: core::cell::Cell<Option<Path>> = const { core::cell::Cell::new(None) };
}

impl Recording {
    fn take() -> Path {
        #[cfg(feature = "std")]
        return RECORDING.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_default();
        #[cfg(not(feature = "std"))]
        Path::default()
    }

    // A thread past its locals' teardown drops the path instead.
    fn give(path: Path) {
        #[cfg(feature = "std")]
        let _ = RECORDING.try_with(|recording| recording.set(Some(path)));
        #[cfg(not(feature = "std"))]
        drop(path);
    }
}

// Left side bearings as drawing reads them: hmtx's by hhea's count, or where hhea is missing, HMTX's by HHEA's. 0 with
// hmtx but neither header, and None without it.
pub(crate) struct DrawnBearings(Option<Option<(TableBytes, usize)>>);

impl DrawnBearings {
    fn read(table_map: &Tables) -> DrawnBearings {
        use crate::daecore::daetype::format::metrics::drawn_metrics;
        DrawnBearings(match drawn_metrics(table_map, false) {
            Some((_, Some(mtx), long)) => Some(Some((mtx.clone(), long.unwrap_or(0)))),
            _ => table_map.contains_key("hmtx").then_some(None),
        })
    }

    fn at(&self, gid: u32) -> Option<i16> {
        use crate::daecore::daetype::format::metrics::MetricsTable;
        let bearing = |(mtx, long): &(TableBytes, usize)| MetricsTable::new(mtx, *long).bearing(gid as usize);
        self.0.as_ref().map(|mtx| mtx.as_ref().map_or(0, bearing))
    }
}

// The room glyphs' deltas are worked out in, kept by its thread as the recording is.
struct Room;

#[cfg(feature = "std")]
std::thread_local! {
    static ROOM: core::cell::Cell<Option<alloc::boxed::Box<DeltaRoom>>> = const { core::cell::Cell::new(None) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = RECORDING.try_with(core::cell::Cell::take);
    let _ = ROOM.try_with(core::cell::Cell::take);
}

impl Room {
    fn take() -> alloc::boxed::Box<DeltaRoom> {
        #[cfg(feature = "std")]
        return ROOM.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_default();
        #[cfg(not(feature = "std"))]
        alloc::boxed::Box::default()
    }

    fn give(room: alloc::boxed::Box<DeltaRoom>) {
        #[cfg(feature = "std")]
        if room.kept() {
            let _ = ROOM.try_with(|kept| kept.set(Some(room)));
        }
        #[cfg(not(feature = "std"))]
        drop(room);
    }
}

// A location's coordinates a CFF2 font's blends weigh by as drawn, and its glyphs as drawn there, the outlines boxed
// so that FontCache keeps its size.
pub(crate) struct Cff2Location {
    pub(crate) coords: alloc::boxed::Box<[i64]>,
    pub(crate) kept: alloc::boxed::Box<KeptOutlines>,
}

impl Cff2Location {
    pub(crate) fn new(coords: Vec<i64>, n_glyphs: u32) -> Cff2Location {
        Cff2Location { coords: coords.into_boxed_slice(), kept: alloc::boxed::Box::new(KeptOutlines::new(n_glyphs)) }
    }

    pub(crate) fn bytes(&self) -> usize {
        core::mem::size_of_val(&*self.coords) + self.kept.bytes.get()
    }
}

impl FontCache {
    // The font's gvar as drawing reads it; Err where it refuses every location. A described location's tables leave
    // gvar out, but VARC's leaves still vary there.
    pub(crate) fn drawn_gvar(&self) -> Result<Option<&Shared<Gvar>>, ()> {
        let charge = |g: &Result<Option<Shared<Gvar>>, ()>| {
            if let Ok(Some(g)) = g {
                self.held.built.add(shared_of::<Gvar>() + g.heap_bytes());
            }
        };
        let read = || Gvar::load(&self.font.tables).map(|g| g.map(Shared::new));
        once_slot(&self.drawn_gvar, read, charge).as_ref().map(Option::as_ref).map_err(|_| ())
    }

    // HVAR's and VVAR's stores, loaded once a font from its own tables, which its drawn locations vary by and VARC's
    // leaves at every location, a described one's too.
    pub(crate) fn drawn_advances(&self) -> &crate::daecore::daetype::instancer::DrawnAdvances {
        use crate::daecore::daetype::instancer::DrawnAdvances;
        once_slot(&self.font.drawn_advances, || DrawnAdvances::load(&self.font.tables), |_| {})
    }

    // How drawing moves glyphs at `key`: None at the default, and where every glyph draws as stored there.
    pub(crate) fn drawn_glyphs(&self, key: &AxisKey) -> Option<alloc::boxed::Box<DrawnGlyphs>> {
        let gvar = self.drawn_gvar().map(|gvar| gvar.cloned());
        let location = DrawnLocation::new(self.tables(), key, gvar, self.drawn_advances())?;
        let (cvt, coords) = (location.cvt(self.tables()), location.f2dot14());
        let held = core::mem::size_of::<DrawnGlyphs>() + cvt.as_ref().map_or(0, heap_of) + heap_of(&coords)
            + location.heap_bytes();
        let bytes = crate::daecore::daekit::base::sync::Counter::new(held);
        let (glyphs, fine) = (Kept::default(), Kept::default());
        let n_glyphs = self.drawn_glyph_count() as usize;
        let extended = self.glyf_extended;
        let font = Shared::clone(&self.font);
        let drawn = DrawnGlyphs { font, location, glyphs, fine, cvt, coords, bytes, n_glyphs, extended };
        Some(alloc::boxed::Box::new(drawn))
    }

    // A glyph's left side bearing as drawing reads it, found once a font where the cache reads the font's own tables.
    fn drawn_bearing(&self, gid: u32) -> Option<i16> {
        match self.location.is_none() {
            true => once_slot(&self.font.drawn_bearings, || DrawnBearings::read(self.tables()), |_| {}).at(gid),
            false => DrawnBearings::read(self.tables()).at(gid),
        }
    }

    // The drawn origin, kept once worked out for each glyph glyf holds of the font's, its sign bit flipped: 0 stands
    // for i32::MIN, which no glyph's 16-bit fields give, and so marks one not worked out yet.
    pub(crate) fn origin(
        &self, glyf: &[u8], loca: LocaRef<'_>, gid: u32, vary: &dyn Vary, scratch: &mut GlyphCoords,
    ) -> Result<i32, Error> {
        const FLIP: u32 = 1 << 31;
        let take = || self.font.spare.take((self.num_glyphs as usize).min(loca.len().saturating_sub(1)));
        let origins = once_slot(&self.origins, take, |o| self.held.built.add(o.heap_bytes()));
        if let held @ 1.. = origins.get(gid) {
            return Ok((held ^ FLIP) as i32);
        }
        let mut left = GLYPH_WORK;
        let x = self.drawn_phantom::<false>(glyf, loca, gid, vary, scratch, 0, &mut left)?;
        origins.set(gid, x as u32 ^ FLIP);
        Ok(x)
    }

    // Phantom point 1's x, where a glyph is drawn from, or `DOWN` point 3's y: its xMin less its left side bearing, or
    // its yMax plus its top one, moved as the location moves it; a composite's last USE_MY_METRICS component's for it.
    // The components it reads, the points it decodes and the deltas it works out are spent from `left`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn drawn_phantom<const DOWN: bool>(
        &self, glyf: &[u8], loca: LocaRef<'_>, gid: u32, vary: &dyn Vary, scratch: &mut GlyphCoords, depth: usize,
        left: &mut usize,
    ) -> Result<i32, Error> {
        // Down the chain of USE_MY_METRICS components, the glyphs on it kept to refuse one naming any again.
        let (mut gid, mut path) = (gid, ComponentPath::default());
        let (glyph, n_contours) = loop {
            let glyph = glyph_bytes(glyf, loca, gid).ok_or(match depth + path.depth() {
                0 => Error::new(crate::daecore::ErrorKind::NoOutline, "glyf: glyph index out of range"),
                _ => Error::malformed("glyf: glyph index out of range"),
            })?;
            let n_contours = match glyph.is_empty() {
                true => 0,
                false => glyph
                    .get(..10)
                    .and_then(|h| read_i16_be(h, 0))
                    .ok_or(Error::malformed("glyf: glyph header truncated"))?,
            };
            if n_contours < 0 {
                let mut read = 0;
                let components =
                    Components::new(glyph, self.glyf_extended).map_while(Result::ok).inspect(|_| read += 1);
                let metrics = components.filter(|c| c.flags & USE_MY_METRICS != 0).last();
                spend(left, read)?;
                if let Some(c) = metrics {
                    path.enter(gid);
                    if path.holds(c.gid) {
                        return Err(Error::malformed("glyf: composite glyph inside itself"));
                    }
                    gid = c.gid;
                    continue;
                }
            }
            break (glyph, n_contours);
        };
        // Decoding reads every point, whether they then decode or not.
        let n = usize::try_from(n_contours).unwrap_or(0);
        let decode = if n > 0 { n + simple_point_count(glyph, n).unwrap_or(0) } else { 0 };
        let deltas = match vary.known(gid, left) {
            // Deltas a location kept from an earlier draw stand in for the decode that working them out took.
            Some(known) if vary.located() => known.and_then(|deltas| spend(left, decode).map(|()| deltas))?,
            Some(known) => known?,
            None if n_contours == 0 => vary.deltas(gid, Shape::Empty, left)?,
            None if n_contours > 0 => {
                spend(left, decode)?;
                let kept = once_slot(&self.font.glyf_points, || KeptPoints::new(self.num_glyphs), |_| {});
                if !kept.decode_into(gid, glyph, n, scratch) {
                    return Err(Error::malformed("glyf: simple glyph truncated, or its end points do not increase"));
                }
                scratch.decoded = Some(gid);
                vary.deltas(gid, Shape::simple(scratch), left)?
            }
            None => vary.deltas(gid, Shape::Composite(glyph), left)?,
        };
        let header = |at: usize| if n_contours == 0 { 0 } else { i32::from(read_i16_be(glyph, at).unwrap_or(0)) };
        let moved = |phantom: usize| deltas.as_ref().map_or((0, 0), |d| d[d.len() - 4 + phantom]);
        Ok(match DOWN {
            false => self.drawn_bearing(gid).map_or(0, |lsb| header(2) - i32::from(lsb) + i32::from(moved(0).0)),
            true => {
                let tsb = self.drawn_vmtx().map_or(0, |vmtx| i32::from(vmtx.bearing(gid as usize)));
                header(8) + tsb + i32::from(moved(2).1)
            }
        })
    }

    // vmtx by its header's count, where drawing takes a glyph's top from it: a count above 0.
    fn drawn_vmtx(&self) -> Option<crate::daecore::daetype::format::metrics::MetricsTable<'_>> {
        use crate::daecore::daetype::format::metrics::{drawn_metrics, MetricsTable};
        match drawn_metrics(self.tables(), true) {
            Some((_, Some(vmtx), Some(long))) if !vmtx.is_empty() && long > 0 => Some(MetricsTable::new(vmtx, long)),
            _ => None,
        }
    }

    // Where drawing puts a glyf glyph's vertical origin: phantom point 3 by vmtx, else the span of the outline's `ys`
    // centered in the line's height, ascender to descender, as metrics place a glyph's top without vmtx.
    pub(crate) fn drawn_vertical_origin(&self, gid: u32, ys: &[f64]) -> Option<f64> {
        if self.drawn_vmtx().is_none() {
            let (ascender, descender) = crate::daecore::daetype::format::metrics::drawn_extents(self.tables());
            let span = |(low, high): (f64, f64), &y: &f64| (low.min(y), high.max(y));
            let (low, high) = ys.split_first().map_or((0.0, 0.0), |(&y, rest)| rest.iter().fold((y, y), span));
            let height = f64::from(i32::from(ascender) - i32::from(descender));
            return Some(high + (height - (high - low)) / 2.0);
        }
        let (glyf, loca) = (self.glyf.as_ref()?, self.loca_offsets()?);
        let vary: &dyn Vary = self.drawn.as_deref().map_or(&Stored, |d| d);
        let (mut scratch, mut left) = (GlyphCoords::default(), GLYPH_WORK);
        let top = self.drawn_phantom::<true>(glyf, loca.into(), gid, vary, &mut scratch, 0, &mut left);
        top.ok().map(f64::from)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/cache/drawn.rs"]
mod tests;
