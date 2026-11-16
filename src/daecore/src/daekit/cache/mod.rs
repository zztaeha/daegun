#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use crate::daecore::daekit::base::sync::{memo_last, mutable, once_slot, read, write, Mutable, Once, Shared};
use crate::daecore::daekit::base::sync::{read_shared, write_shared, Lined, Shareable};
#[cfg(feature = "threading")]
use crate::daecore::daekit::base::sync::Id;
use crate::daecore::daekit::base::sync::{try_read_shared, try_write, try_write_shared};
use crate::daecore::daetype::decoder::{read_i16_be, read_u16_be, read_u32_be};
use crate::daecore::daetype::subsetter::Subset;
use crate::daecore::daetype::{TableBytes, Tables};
use crate::daecore::daetype::format::glyf::{GlyfLoca, LocaRef};
use crate::daecore::daekit::text::shape::Feature;
use crate::daecore::daetype::format::metrics::shaped_tags;

pub(crate) mod color;
pub(crate) mod drawn;
pub(crate) mod instance;
pub(crate) mod key;
pub(crate) mod lru;
pub(crate) mod metrics;
pub(crate) mod raster;
pub(crate) mod shaping;
mod standard_latin;
pub mod subset;
pub(crate) mod varc;
pub(crate) use varc::varc_drawn_from;

pub use key::AxisPair;
pub(crate) use key::{AxisKey, ResolvedAxes};
pub(crate) use key::{tag_bytes, EMPTY_KEY};
pub use metrics::{LineMetrics, PdfFlags};
pub(crate) use metrics::{Advances, GlyphExtents, VerticalOrigins};
use crate::daecore::Error;

type PlanCache = lru::ByteLru<alloc::boxed::Box<[u8]>, Shared<crate::daecore::daeshaper::plan::Plan>>;

// What one location has built for drawing, shaping, metrics, the whole instance and subsets, each maybe the
// instancer's error, kept so a failing location isn't rebuilt.
#[derive(Clone, Default)]
pub(crate) struct Located {
    drawn: Option<Result<Shared<FontCache>, Error>>,
    described: Option<Result<Shared<FontCache>, Error>>,
    instance: Option<Result<alloc::sync::Arc<[u8]>, Error>>,
    subset: Option<Result<Shared<subset::SubsetFace>, Error>>,
}

impl Located {
    // Its caches and subset face charged what they hold as it comes into the budget, those in already as they were.
    fn hold(&self) {
        for at in [&self.drawn, &self.described].into_iter().flatten().flatten() {
            at.charge.hold(at.held_bytes());
        }
        if let Some(Ok(face)) = &self.subset {
            face.charge.hold(face.held_bytes());
        }
    }

    fn release(&self) {
        for at in [&self.drawn, &self.described].into_iter().flatten().flatten() {
            at.charge.release();
        }
        if let Some(Ok(face)) = &self.subset {
            face.charge.release();
        }
    }
}

// The key is held twice, as the LRU holds every key.
fn located_cost(key: &AxisKey, at: &Located) -> usize {
    use crate::daecore::daekit::base::sync::shared_of;
    let cache = |c: &Option<Result<Shared<FontCache>, Error>>| {
        c.as_ref().map_or(0, |c| c.as_ref().map_or_else(Error::heap_bytes, |c| c.charge.get()))
    };
    let instance = at.instance.as_ref().map_or(0, |i| {
        i.as_ref().map_or_else(Error::heap_bytes, |i| shared_of::<()>() + i.len())
    });
    let subset = at.subset.as_ref().map_or(0, |s| s.as_ref().map_or_else(Error::heap_bytes, |s| s.charge.get()));
    2 * (core::mem::size_of::<AxisKey>() + key.heap_bytes())
        + core::mem::size_of::<Located>()
        + cache(&at.drawn)
        + cache(&at.described)
        + instance
        + subset
}

type RunCache = lru::ByteLru<Shared<ShapeKey>, alloc::sync::Arc<crate::daecore::daekit::text::shape::ShapedRun>>;

// Keyed on `rtl`, without which a Common-script run, directed by layout's bidi level, collides with its LTR twin; on
// `CONTEXT_CHARS` codepoints of context either side, all a run reads; and on a layout's features over it.
type ShapeKey = (AxisKey, String, bool, Option<bool>, String, String, Option<String>, Option<u16>, Vec<Feature>);

#[derive(Clone, Copy, Default, Debug)]
pub struct RunContext<'a> {
    pub rtl: Option<bool>,
    pub before: &'a str,
    pub after: &'a str,
    pub language: Option<&'a str>,
    pub seed_script: Option<crate::daecore::daeshaper::unicode::Script>,
    pub features: &'a [Feature],
}

// A kept run's cost past its strings, vectors, key and run (their `Arc`s and their slots in the two maps): 111 to 118
// bytes, measured on Inter, Noto Sans Devanagari and EB Garamond, 77 on 32 bits. Never 0: a budget of 0 keeps none.
const SHAPE_ENTRY_BYTES: usize = if cfg!(target_pointer_width = "64") { 118 } else { 77 }
    + core::mem::size_of::<ShapeKey>()
    + core::mem::size_of::<crate::daecore::daekit::text::shape::ShapedRun>();

// Both sides destructured rather than read through `.`, so a new field is a compile error here: an
// undercount has no symptom until memory runs out.
fn shape_cost(key: &ShapeKey, run: &crate::daecore::daekit::text::shape::ShapedRun) -> usize {
    use crate::daecore::daekit::base::sync::heap_of;
    let crate::daecore::daekit::text::shape::ShapedRun {
        glyphs, advances, offsets, unsafe_to_break, unsafe_to_concat, safe_to_insert_tatweel,
        clusters, complete: _, has_broken_syllable: _, shaper: _,
    } = run;
    let (axes, text, _, _, before, after, language, _seed, features) = key;
    SHAPE_ENTRY_BYTES
        + axes.heap_bytes()
        + text.capacity()
        + before.capacity()
        + after.capacity()
        + language.as_ref().map_or(0, String::capacity)
        + heap_of(features)
        + heap_of(glyphs)
        + heap_of(advances)
        + heap_of(offsets)
        + heap_of(unsafe_to_break)
        + heap_of(unsafe_to_concat)
        + heap_of(safe_to_insert_tatweel)
        + heap_of(clusters)
}

// Bytes, as one location holds a font's tables: Inter at a weight 405 KB drawn, 963 KB shaped and instanced, Source Han
// Sans 14 and 28 MB. The newest stays past it, or each call at its location would instance the font again.
const INSTANCE_BUDGET_BYTES: usize = 64 * 1024 * 1024;
const SHAPE_CACHE_BYTES: usize = 8 * 1024 * 1024;

// A run is kept where it costs an eighth of its list or less, so one doesn't empty it, or else 64 KB or less, an eighth
// of a default list under `threading`.
const RUN_PART: usize = 8;
const RUN_FLOOR: usize = 64 * 1024;

// Plans past the kept slots, once a font has any or its runs join a pool, take this part of the shape budget, the runs
// the rest.
const PLAN_PART: usize = 16;

// A run longer than this is rare, and its buffer kept as the spare would hold that size until the cache or the
// thread goes.
const SPARE_BUFFER_GLYPHS: usize = 4096;

// A font's indexes may take the floor and this many times its tables' distinct bytes: of 840 real faces the most any
// takes is 4.5 times (Noto Sans Siddham), LastResort aside.
const INDEX_BYTES_PER_TABLE_BYTE: usize = 8;
// The smallest power of two holding LastResort's 2.2 MB cmap index (76 bytes mapping all of Unicode), the most past
// eight times its tables any real face's indexes take.
const INDEX_FLOOR_BYTES: usize = 4 << 20;

// The bytes a font's tables span, those two tables share counted once: sorted on the stack, so the first index
// allocates nothing for it, unless the font has more tables than any real one.
fn distinct_bytes(tables: &Tables) -> usize {
    let (mut inline, mut heap) = ([(0, 0); 64], Vec::new());
    let each = tables.iter().map(|(_, t)| (t.as_ptr() as usize, t.len()));
    let spans: &mut [(usize, usize)] = match tables.iter().len() {
        n if n <= inline.len() => {
            inline.iter_mut().zip(each).for_each(|(slot, span)| *slot = span);
            &mut inline[..n]
        }
        _ => {
            heap.extend(each);
            &mut heap
        }
    };
    spans.sort_unstable();
    let (mut total, mut reach) = (0, 0);
    for &mut (start, len) in spans {
        total += (start + len).saturating_sub(start.max(reach));
        reach = reach.max(start + len);
    }
    total
}

type HintSlot = (
    crate::daecore::daetype::hinting::f26dot6::Size,
    u16,
    crate::daecore::daetype::hinting::HintMode,
    Option<crate::daecore::daetype::hinting::HintContext>,
);

type ColrScalarSlot = (Vec<f64>, Shared<Vec<i64>>);

// Codes are looked up in the subtables shaping reads, DMAP's then cmap's, chosen at the first lookup, until a shaper
// warms the index, and again where the index is refused; `recent` holds what those lookups found.
#[derive(Default)]
pub(crate) struct CmapState {
    lookup: Once<(crate::daecore::daetype::subsetter::CharMap, [Option<TableBytes>; 2])>,
    index: Once<Option<Shared<crate::daecore::daetype::format::index::SparseIndex>>>,
    recent: RecentGlyphs,
}

// A code's glyph as last looked up, in the slot its low 8 bits pick: the code's upper 13 bits above a 16-bit glyph,
// 0 for none. A slot is one word, so a reader sees an old entry or a new one, never half of each.
pub(crate) struct RecentGlyphs([core::sync::atomic::AtomicU32; 256]);

impl Default for RecentGlyphs {
    fn default() -> Self {
        // Its upper 16 bits name no code: Unicode's last has 0x10FF there.
        RecentGlyphs(core::array::from_fn(|_| core::sync::atomic::AtomicU32::new(u32::MAX)))
    }
}

impl RecentGlyphs {
    // Only for a code within Unicode, whose upper bits never match an empty slot's.
    #[inline]
    fn get(&self, code: u32) -> Option<u32> {
        let slot = self.0[code as usize & 255].load(core::sync::atomic::Ordering::Relaxed);
        (slot >> 16 == code >> 8).then_some(slot & 0xFFFF)
    }

    #[inline]
    fn put(&self, code: u32, glyph: u32) {
        if glyph <= 0xFFFF {
            self.0[code as usize & 255].store((code >> 8) << 16 | glyph, core::sync::atomic::Ordering::Relaxed);
        }
    }
}

// What a font's own cache and its locations share, each read or built once for all of them: its axes and cmap, which
// no location varies, the index allowance, and what locations are built from.
pub(crate) struct FontState {
    // The font's own fvar, a location's tables perhaps without one, and its axes read from it once a key needs them.
    fvar: Option<TableBytes>,
    axes: Once<key::AxisRanges>,
    // Whether the font has a VARC, asked by every glyph drawn or hinted: an instance may drop it but never gains one.
    // And where its components start at the default, which the font's own cache asks for each glyph VARC draws.
    varc: bool,
    varc_default: Once<Option<Vec<i64>>>,
    cmap: CmapState,
    // What the font's indexes may still take, set at the first index or the caller's first word on it, as only fonts
    // that shape build one.
    index_allowance: Once<crate::daecore::daekit::base::sync::Counter>,
    cff2_outlines: Once<Option<Shared<crate::daecore::daetype::outline::Cff2Outlines>>>,
    cff2_kept: Once<drawn::KeptOutlines>,
    // The sanitizer's verdicts on the font's own tables, which every instance and location takes alike.
    instance_reads: Once<crate::daecore::daetype::format::sanitize::Reads>,
    // HVAR's and VVAR's stores as drawing reads them, which the font's own cache and its drawn locations share.
    drawn_advances: Once<crate::daecore::daetype::instancer::DrawnAdvances>,
    // Where drawing reads the font's own left side bearings, which every glyph drawn at a new location asks.
    drawn_bearings: Once<drawn::DrawnBearings>,
    // The font's own tables, read by its cache and its drawn locations alike and left where they are: a location's
    // copy wrote the file's count once for each table, which took 2.4× as long a new location at 8 threads.
    tables: Tables,
    // The tables a described location holds, the same at every one, or why none can be built.
    described_tables: Once<Result<Tables, Error>>,
    // Simple glyphs' points as decoded at their first draw at a location, which every location then copies.
    glyf_points: Once<drawn::KeptPoints>,
    // fvar's axes and avar's maps, which every new location's coordinates are worked out from.
    normalizer: Once<Result<crate::daecore::daetype::instancer::Normalizer, Error>>,
    // HVAR's view and VVAR's as a described location's advances read them, the font's, not a location's to charge.
    advance_views: Once<[Option<Shared<crate::daecore::daetype::format::ivs::DeltaView>>; 2]>,
    // Tables by glyph that locations gone gave back emptied, taken by new ones rather than zeroed: one location's
    // worth at most, counted with the locations and let go as their budget is set or cleared.
    spare: crate::daecore::daekit::base::sync::SpareSlots,
}

// The font's locations under the instance budget, behind one lock a pool reaches too, and how many have left it, so a
// thread knows the one it last read may be gone.
pub(crate) struct Locations {
    list: Mutable<lru::ByteLru<AxisKey, Located>>,
    dropped: crate::daecore::daekit::base::sync::Counter,
}

impl lru::Member for Locations {
    fn evict(&self, pool: &lru::Pool, _: usize, stamp: u64) {
        let mut list = write(&self.list);
        let evicted = list.evict_listed(pool, stamp);
        if evicted > 0 {
            self.dropped.add(evicted);
        }
    }
}

// What a cache built on the heap, its location's share of the instance budget: hint contexts and scratch spaces are
// replaced, not added to, each keeping one figure; plans and the shaping buffer, held by the font's own, go uncounted.
#[derive(Default)]
struct Held {
    built: crate::daecore::daekit::base::sync::Counter,
    hints: crate::daecore::daekit::base::sync::Counter,
    outline_scratch: crate::daecore::daekit::base::sync::Counter,
    autohinter: crate::daecore::daekit::base::sync::Counter,
}

impl Held {
    fn total(&self) -> usize {
        let Held { built, hints, outline_scratch, autohinter } = self;
        [built, hints, outline_scratch, autohinter].into_iter().map(|c| c.get()).sum()
    }
}

pub struct FontCache {
    // The font's tables, or what a described location keeps of them until it builds those it varies at the first read
    // of one through `tables` or `table`; `font_tables` reads them directly where a cache builds none.
    font: Shared<FontState>,
    upm: u16,
    num_glyphs: u32,
    os2: Once<Option<crate::daecore::daetype::decoder::Os2Fields>>,
    glyf: Option<crate::daecore::daetype::TableBytes>,
    // Whether glyf is GLYF, whose composites can name a glyph in 24 bits.
    glyf_extended: bool,
    // CFF as drawn, named and measured, its glyphs counted by its CharStrings whatever maxp counts.
    cff: Option<crate::daecore::daetype::TableBytes>,
    // CFF2 outlines are drawn and measured from the font's own table, and subset once instanced, the default too.
    cff2: bool,
    // No outline table: the strikes give the line and the advances hmtx, hhea or vmtx would.
    bitmap_only: bool,
    // Across, then down, as `metrics::line_extents_of` reads them: every line metric starts there.
    line_extents: [Once<Option<(i32, i32, i32)>>; 2],
    // hmtx with hhea's count of long records, then vmtx with vhea's (or the beyond-64K pair shaping reads): what
    // `metrics_table` reads, kept since an outline drawn at its origin reads its side bearing.
    metrics: [Option<(TableBytes, u32)>; 2],
    // Where each table's advances lie in it, worked out from `metrics` once for every read.
    advance_shapes: [Option<metrics::AdvanceShape>; 2],
    // The font's alone: what each location has built, under one budget, made at the first location read.
    locations: Once<Shared<Locations>>,
    // Across and down, the advances of a font without hmtx or vmtx; every other font's are read where they lie.
    listed_advances: [Once<metrics::ListedAdvances>; 2],
    // A location's advance across and down, in font units, for a glyph past its instance's metrics.
    advances_past: [u32; 2],
    // A described location's advances across and down and its VORG, where they vary glyph by glyph.
    varied_advances: [Option<crate::daecore::daetype::instancer::VariedAdvances>; 2],
    varied_origins: Option<alloc::boxed::Box<crate::daecore::daetype::instancer::VariedOrigins>>,
    // What a described location builds when first read there; None for the font's own cache and a drawn location.
    described: Option<alloc::boxed::Box<instance::Described>>,
    // A glyf font's own tables as glyphs are measured, which every location shares; and a location's glyphs measured
    // there.
    glyf_tables: Once<Option<Shared<crate::daecore::daetype::instancer::GlyfTables>>>,
    measured: Option<Shared<crate::daecore::daetype::instancer::MeasuredGlyphs>>,
    // Where its glyphs are as stored, their boxes as drawn, kept as a location's are.
    stored_measured: Once<Option<crate::daecore::daetype::instancer::MeasuredGlyphs>>,
    // A location's key and normalized coordinates, and what shaping reads of it against the font's own tables.
    location: Option<Shared<Lined<ResolvedAxes>>>,
    variation: Once<Shared<crate::daecore::daeshaper::face::Variation>>,
    #[cfg(not(feature = "threading"))]
    location_by_axis: Mutable<Vec<Shared<Lined<ResolvedAxes>>>>,
    default_axes: Once<Shared<Lined<ResolvedAxes>>>,
    // Arc in every build, not `Shared`, as `Font::shape` hands these runs out and its type can't change with
    // `threading`; split into shards there, made at the first shape, so a location holds only the slot.
    #[cfg(not(feature = "threading"))]
    shape_cache: Once<Shared<Shareable<RunCache>>>,
    #[cfg(feature = "threading")]
    run_shards: Once<Shared<shaping::RunShards>>,
    // Under `threading` what each thread's front of recent runs (`shaping.rs`) checks: this cache, never another
    // one's number, and how many runs it has dropped, so a front never serves a run dropped since.
    #[cfg(feature = "threading")]
    runs_id: Id,
    #[cfg(feature = "threading")]
    runs_dropped: crate::daecore::daekit::base::sync::Counter,
    // What the shape cache may hold, its runs and plans together, as last set.
    shape_budget: crate::daecore::daekit::base::sync::Counter,
    outline_scratch: Mutable<Option<alloc::boxed::Box<crate::daecore::daetype::instancer::GlyphCoords>>>,
    // Under `threading` each thread keeps its own (`shaping.rs`), as one shared would be taken by every shape.
    #[cfg(not(feature = "threading"))]
    spare_buffer: Mutable<Option<crate::daecore::daeshaper::run::Run>>,
    kept_plans: Once<alloc::boxed::Box<shaping::KeptPlans>>,
    plan_cache: Once<Shared<Mutable<PlanCache>>>,
    loca: Once<Option<Shared<GlyfLoca>>>,
    // gvar as drawing reads it, the font's own; and how a location's cache moves glyphs as they're drawn there,
    // boxed so that no other cache carries its few hundred bytes.
    drawn_gvar: Once<Result<Option<Shared<crate::daecore::daetype::instancer::Gvar>>, ()>>,
    drawn: Option<alloc::boxed::Box<drawn::DrawnGlyphs>>,
    // Each glyf glyph's drawn origin once worked out: a composite's takes a walk of its components.
    origins: Once<crate::daecore::daekit::base::sync::GlyphSlots>,
    // Each glyph's extents once worked out where glyf's measured glyphs don't keep them: a color glyph's paint, VARC's
    // or a drawn outline costs as much each time a run asks.
    kept_extents: Once<alloc::boxed::Box<metrics::KeptExtents>>,
    // None at the default.
    cff2_at: Option<drawn::Cff2Location>,
    // A glyf location's normalized coordinates where the font has VARC, which its components start from; None at the
    // default.
    varc_at: Option<alloc::boxed::Box<[i64]>>,
    cff_outlines_cache: Once<Option<Shared<crate::daecore::daetype::outline::CffOutlines>>>,
    gdef_var_store: Once<Option<Shared<crate::daecore::daetype::format::ivs::ItemVariationStore>>>,
    color_tables: Once<Option<Shared<color::ColorTables>>>,
    colr_v1_scalars: Mutable<Option<ColrScalarSlot>>,
    hint_context: Mutable<Vec<HintSlot>>,
    // Boxed, as most caches never autohint, and are moved whole.
    autohinter: Mutable<Option<Option<alloc::boxed::Box<crate::daecore::daetype::hinting::auto::AutoHinter>>>>,
    gdef_class_indexes: Once<GdefClassIndexes>,
    // Whether shaping reads morx, kerx, `kern` and ankr, by `AatTable`, and GSUB, GPOS and GDEF, by `OtlTable`.
    aat_reads: [Once<bool>; 4],
    otl_reads: [Once<bool>; 3],
    // GSUB's and GPOS's lookup indexes, then the coverage and class definition indexes they share by offset (the
    // bool true for a class definition), under one guard: a lookup's index builds its rule tables' with it held.
    lookup_indexes: [Mutable<(LookupIndexSlots, OtlIndexes)>; 2],
    // Each morx, kerx and `kern` subtable's glyph sets, by `AatTable` then subtable.
    aat_sets: [Mutable<Vec<Option<Shared<crate::daecore::daeshaper::ot::aat::SubtableSets>>>>; 3],
    held: Held,
    // What the font's instance budget last charged this location, so that it is charged again only on a
    // change; stored only then, as every thread at the location reads the line.
    charge: lru::Charge,
    // What this cache's indexes drew from the font's allowance.
    index_spent: crate::daecore::daekit::base::sync::Counter,
}

// A cache is moved whole as a font opens and as a location is made, so a field that would pass this is boxed.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<FontCache>() <= 2112);

// A location's indexes go with it, so their bytes go back to the allowance they were drawn from, and its tables by
// glyph go back to the font's spares.
impl Drop for FontCache {
    fn drop(&mut self) {
        if let Some(allowance) = self.font.index_allowance.get() {
            allowance.add(self.index_spent.get());
        }
        let spare = &self.font.spare;
        if let Some(origins) = self.origins.take() {
            spare.give(origins);
        }
        for advances in self.varied_advances.iter_mut().flatten() {
            spare.give(advances.take_read());
        }
        if let Some(origins) = self.varied_origins.as_deref_mut() {
            spare.give(origins.take_read());
        }
    }
}

// GDEF's glyph classes and mark attachment classes, each an index where the allowance covers it, and its mark
// glyph sets, one index each.
type GdefClassIndexes = (
    Option<Shared<crate::daecore::daetype::format::index::SparseIndex>>,
    Option<Shared<crate::daecore::daetype::format::index::SparseIndex>>,
    Option<Shared<Vec<Option<crate::daecore::daetype::format::index::SparseIndex>>>>,
);

// CFF2's variation store, and whether its stated length lets it blend.

type LookupIndexSlots = Vec<Option<Shared<crate::daecore::daeshaper::ot::LookupIndex>>>;

pub(crate) type OtlIndexes =
    BTreeMap<(usize, bool), Option<Shared<crate::daecore::daetype::format::index::SparseIndex>>>;

// An autohinter's bytes and the outline scratch's, each box's own with what it points to.
fn boxed_bytes(h: &crate::daecore::daetype::hinting::auto::AutoHinter) -> usize {
    size_of::<crate::daecore::daetype::hinting::auto::AutoHinter>() + h.heap_bytes()
}

fn scratch_bytes(s: &crate::daecore::daetype::instancer::GlyphCoords) -> usize {
    size_of::<crate::daecore::daetype::instancer::GlyphCoords>() + s.heap_bytes()
}

// The points a location's moved glyphs are decoded into, kept by its thread as the room for their deltas is: a
// location's own started empty at every new one and grew at its first glyphs.
struct Scratch;

#[cfg(feature = "std")]
std::thread_local! {
    static SCRATCH: core::cell::Cell<Option<alloc::boxed::Box<crate::daecore::daetype::instancer::GlyphCoords>>> =
        const { core::cell::Cell::new(None) };
}

// One past this is dropped rather than kept, as a glyph can hold 65,535 points.
#[cfg(feature = "std")]
const SCRATCH_KEPT_BYTES: usize = 64 * 1024;

impl Scratch {
    fn take() -> alloc::boxed::Box<crate::daecore::daetype::instancer::GlyphCoords> {
        #[cfg(feature = "std")]
        return SCRATCH.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_default();
        #[cfg(not(feature = "std"))]
        alloc::boxed::Box::default()
    }

    // Its mark cleared, as no glyph's points are in it for the next draw.
    fn give(scratch: alloc::boxed::Box<crate::daecore::daetype::instancer::GlyphCoords>) {
        #[cfg(feature = "std")]
        if scratch.heap_bytes() <= SCRATCH_KEPT_BYTES {
            let mut scratch = scratch;
            scratch.decoded = None;
            let _ = SCRATCH.try_with(|kept| kept.set(Some(scratch)));
        }
        #[cfg(not(feature = "std"))]
        drop(scratch);
    }
}

// The contexts kept of the last few sizes and modes, and autohinters of the last few locations, newest last: text at
// two sizes or weights in one frame, or a layout beside its display size, would otherwise build one at every switch.
const KEPT_CONTEXTS: usize = 4;

// What a thread hints with, under `threading`, copied from what the font keeps: sharing one under its lock took 13× as
// long a glyph at 8 threads. Each names its cache, and holds its font's bytes until replaced or the thread ends.
#[cfg(feature = "threading")]
struct ThreadContext {
    cache: Id,
    size: crate::daecore::daetype::hinting::f26dot6::Size,
    upm: u16,
    mode: crate::daecore::daetype::hinting::HintMode,
    context: Option<crate::daecore::daetype::hinting::HintContext>,
}

#[cfg(feature = "threading")]
type ThreadAutohinter = (Id, alloc::boxed::Box<crate::daecore::daetype::hinting::auto::AutoHinter>);

#[cfg(feature = "threading")]
std::thread_local! {
    static THREAD_CONTEXTS: core::cell::Cell<Vec<ThreadContext>> = const { core::cell::Cell::new(Vec::new()) };
    static THREAD_AUTOHINTERS: core::cell::Cell<Vec<ThreadAutohinter>> = const { core::cell::Cell::new(Vec::new()) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SCRATCH.try_with(core::cell::Cell::take);
    #[cfg(feature = "threading")]
    {
        let _ = THREAD_CONTEXTS.try_with(core::cell::Cell::take);
        let _ = THREAD_AUTOHINTERS.try_with(core::cell::Cell::take);
    }
}

// head, or bhed in an Apple font that has only bitmaps: the same record, read alike.
fn font_header(table_map: &Tables) -> Option<&TableBytes> {
    table_map.get("head").or_else(|| table_map.get("bhed"))
}

impl FontCache {
    #[cfg(feature = "threading")]
    pub fn try_autohint<R>(
        &self,
        pts: &crate::daecore::daetype::hinting::auto::AutoPoints,
        size: crate::daecore::daetype::hinting::f26dot6::Size,
        take: impl FnOnce(&mut crate::daecore::daetype::hinting::HintedOutline) -> R,
    ) -> Option<Option<R>> {
        let cache = self.runs_id;
        let mut mine = THREAD_AUTOHINTERS.try_with(core::cell::Cell::take).unwrap_or_default();
        let hinter = match mine.iter().position(|(c, _)| *c == cache) {
            Some(i) => Some(Some(mine.remove(i).1)),
            None => crate::daecore::daekit::base::sync::read(&self.autohinter)
                .as_ref()
                .map(|built| built.as_deref().map(|built| alloc::boxed::Box::new(built.fresh()))),
        };
        let taken = hinter.map(|hinter| {
            hinter.map(|mut hinter| {
                let taken = take(hinter.hint(pts, size));
                if mine.len() == KEPT_CONTEXTS {
                    mine.remove(0);
                }
                mine.push((cache, hinter));
                taken
            })
        });
        let _ = THREAD_AUTOHINTERS.try_with(|kept| kept.set(mine));
        taken
    }

    #[cfg(not(feature = "threading"))]
    pub fn try_autohint<R>(
        &self,
        pts: &crate::daecore::daetype::hinting::auto::AutoPoints,
        size: crate::daecore::daetype::hinting::f26dot6::Size,
        take: impl FnOnce(&mut crate::daecore::daetype::hinting::HintedOutline) -> R,
    ) -> Option<Option<R>> {
        let mut slot = crate::daecore::daekit::base::sync::write(&self.autohinter);
        let built = slot.as_mut()?;
        let hinted = built.as_mut().map(|h| take(h.hint(pts, size)));
        self.held.autohinter.set(built.as_deref().map_or(0, boxed_bytes));
        Some(hinted)
    }

    pub fn set_autohinter(&self, h: Option<crate::daecore::daetype::hinting::auto::AutoHinter>) {
        let h = h.map(alloc::boxed::Box::new);
        self.held.autohinter.set(h.as_deref().map_or(0, boxed_bytes));
        *crate::daecore::daekit::base::sync::write(&self.autohinter) = Some(h);
    }

    pub fn draw_glyf_reusing(
        &self,
        loca: LocaRef<'_>,
        gid: u32,
        work: &mut usize,
        pen: &mut dyn crate::daecore::daetype::outline::OutlinePen,
    ) -> Result<(), Error> {
        self.draw_glyf(loca, gid, work, pen)
    }

    // A glyf glyph drawn at this cache's location; the glyphs it read and the points it decoded add to `work`.
    pub(crate) fn draw_glyf(
        &self,
        loca: LocaRef<'_>,
        gid: u32,
        work: &mut usize,
        pen: &mut dyn crate::daecore::daetype::outline::OutlinePen,
    ) -> Result<(), Error> {
        use crate::daecore::daetype::outline::{outline_glyf_glyph_varied, Paying, Stored, Vary};
        let Some(glyf) = self.glyf.as_ref() else {
            return Err(Error::missing("glyf: table missing"));
        };
        let mut scratch = self.take_scratch();
        let vary: &dyn Vary = self.drawn.as_deref().map_or(&Stored, |d| d);
        let shift = self.origin(glyf, loca, gid, vary, &mut scratch).map(|x| -x);
        let drawn = shift.and_then(|shift| {
            let mut shifted;
            let pen: &mut dyn crate::daecore::daetype::outline::OutlinePen = if shift == 0 {
                pen
            } else {
                let by = [1.0, 0.0, 0.0, 1.0, f64::from(shift), 0.0];
                shifted = crate::daecore::daetype::outline::TransformPen::new(pen, by);
                &mut shifted
            };
            let paying;
            let vary: &dyn Vary = match self.drawn.as_deref() {
                Some(drawn) => {
                    paying = Paying::new(drawn, gid);
                    &paying
                }
                None => vary,
            };
            outline_glyf_glyph_varied(glyf, self.glyf_extended, loca, gid, &mut scratch, vary, work, pen)
        });
        self.give_scratch(scratch);
        drawn
    }

    // The scratch glyf glyphs are decoded into: the thread's under `threading`, as one lock took 16× as long a draw at
    // 8 threads, and for a location's moved glyphs, as its own starts empty; else the cache's, for two fewer lookups.
    #[inline(always)]
    pub(super) fn take_scratch(&self) -> alloc::boxed::Box<crate::daecore::daetype::instancer::GlyphCoords> {
        match cfg!(feature = "threading") || self.drawn.is_some() {
            true => Scratch::take(),
            false => crate::daecore::daekit::base::sync::write(&self.outline_scratch).take().unwrap_or_default(),
        }
    }

    #[inline(always)]
    pub(super) fn give_scratch(&self, mut scratch: alloc::boxed::Box<crate::daecore::daetype::instancer::GlyphCoords>) {
        if cfg!(feature = "threading") || self.drawn.is_some() {
            return Scratch::give(scratch);
        }
        scratch.decoded = None;
        self.held.outline_scratch.set(scratch_bytes(&scratch));
        *crate::daecore::daekit::base::sync::write(&self.outline_scratch) = Some(scratch);
    }

    // `take` gets the hinted outline, under the context's lock where threads don't share a font: it may move the
    // outline out, or draw it and leave the buffers for the next glyph, but not reach back into this cache.
    #[allow(clippy::too_many_arguments)]
    pub fn hint_glyph_cached<R>(
        &self,
        glyf: &[u8],
        loca: LocaRef<'_>,
        gid: u32,
        size: crate::daecore::daetype::hinting::f26dot6::Size,
        upm: u16,
        mode: crate::daecore::daetype::hinting::HintMode,
        take: impl FnOnce(&mut crate::daecore::daetype::hinting::HintedOutline) -> R,
    ) -> Option<R> {
        use crate::daecore::daetype::outline::{Paying, Stored, Vary};
        let paying;
        let vary: &dyn Vary = match self.drawn.as_deref() {
            Some(drawn) => {
                paying = Paying::new(drawn, gid);
                &paying
            }
            None => &Stored,
        };
        #[cfg(feature = "threading")]
        {
            let cache = self.runs_id;
            let mut mine = THREAD_CONTEXTS.try_with(core::cell::Cell::take).unwrap_or_default();
            match mine.iter().position(|t| t.cache == cache && t.size == size && t.upm == upm && t.mode == mode) {
                Some(i) if i + 1 < mine.len() => {
                    let hit = mine.remove(i);
                    mine.push(hit);
                }
                Some(_) => {}
                None => {
                    if mine.len() == KEPT_CONTEXTS {
                        mine.remove(0);
                    }
                    let context = self.font_context(size, upm, mode, |c| c.cloned());
                    mine.push(ThreadContext { cache, size, upm, mode, context });
                }
            }
            let hinted = mine
                .last_mut()
                .and_then(|t| t.context.as_mut())
                .and_then(|c| c.hint_glyph_varied(glyf, loca, gid, vary))
                .map(take);
            let _ = THREAD_CONTEXTS.try_with(|kept| kept.set(mine));
            hinted
        }
        #[cfg(not(feature = "threading"))]
        self.font_context(size, upm, mode, |c| c.and_then(|c| c.hint_glyph_varied(glyf, loca, gid, vary)).map(take))
    }

    // `f` on the font's context for a size and mode, built if it lacks one, with the contexts' lock held.
    fn font_context<T>(
        &self,
        size: crate::daecore::daetype::hinting::f26dot6::Size,
        upm: u16,
        mode: crate::daecore::daetype::hinting::HintMode,
        f: impl FnOnce(Option<&mut crate::daecore::daetype::hinting::HintContext>) -> T,
    ) -> T {
        let mut slots = crate::daecore::daekit::base::sync::write(&self.hint_context);
        match slots.iter().position(|(s, u, m, _)| *s == size && *u == upm && *m == mode) {
            // Moved to newest only when it is not already: a context is large, and one size glyph after
            // glyph is the common case.
            Some(i) if i + 1 < slots.len() => {
                let hit = slots.remove(i);
                slots.push(hit);
            }
            Some(_) => {}
            None => {
                if slots.len() == KEPT_CONTEXTS {
                    slots.remove(0);
                }
                let (cvt, coords) = self.drawn.as_deref().map_or((None, None), |d| {
                    let (cvt, coords) = d.hinting();
                    (cvt, Some(coords))
                });
                let built = mode.runs_bytecode().then(|| {
                    crate::daecore::daetype::hinting::HintContext::new_at(self.tables(), size, upm, cvt, coords)
                });
                let built = built.flatten();
                slots.push((size, upm, mode, built));
            }
        }
        let out = f(slots.last_mut().and_then(|s| s.3.as_mut()));
        let contexts: usize = slots.iter().filter_map(|s| s.3.as_ref()).map(|c| c.heap_bytes()).sum();
        self.held.hints.set(crate::daecore::daekit::base::sync::heap_of(&slots) + contexts);
        out
    }

    // Each is a ceiling the cache grows into, never a reservation, and dead weight for some caller: text that never
    // repeats gains nothing from the shape cache. A lower one applies at once, keeping the newest entries that fit.
    pub fn set_shape_cache_bytes(&self, bytes: usize) {
        self.shape_budget.set(bytes);
        let plans = self.plan_cache.get().map_or(0, |plans| {
            write(plans).set_budget(bytes / PLAN_PART);
            bytes / PLAN_PART
        });
        self.set_runs_bytes(bytes, bytes - plans);
    }

    pub fn shape_cache_stats(&self) -> (usize, usize) {
        let (plans, bytes) = self.plan_cache.get().map_or((0, 0), |plans| {
            let plans = read(plans);
            (plans.len(), plans.bytes())
        });
        let (runs, held) = self.run_stats();
        (runs + plans, held + bytes)
    }

    pub fn clear_shape_cache(&self) {
        if let Some(plans) = self.plan_cache.get() {
            write(plans).clear();
        }
        self.clear_runs();
    }

    // Runs and plans join a pool together, or leave it.
    pub(crate) fn set_shape_pool(&self, pool: Option<&Shared<lru::Pool>>) {
        let plans = self.plans();
        self.set_runs_pool(pool);
        lru::rejoin(core::slice::from_mut(&mut write(plans)), pool, &lru::member(plans), |_| 0);
        if let Some(pool) = pool {
            pool.settle();
        }
    }

    // Plans past the kept ones, made at the first, in the pool the runs are in, the runs then taking the rest of the
    // shape budget.
    pub(super) fn plans(&self) -> &Shared<Mutable<PlanCache>> {
        self.plan_cache.get_or_init(|| {
            let bytes = self.shape_budget.get();
            self.set_runs_bytes(bytes, bytes - bytes / PLAN_PART);
            let plans = Shared::new(mutable(lru::ByteLru::new(bytes / PLAN_PART, shaping::plan_cost)));
            if let Some(pool) = self.runs_pool() {
                lru::rejoin(core::slice::from_mut(&mut write(&plans)), Some(&pool), &lru::member(&plans), |_| 0);
            }
            plans
        })
    }

    #[cfg(not(feature = "threading"))]
    fn shape_list(&self) -> &Shared<Shareable<RunCache>> {
        let list = || -> RunCache { lru::ByteLru::new(SHAPE_CACHE_BYTES, |key, run| shape_cost(key, run)) };
        self.shape_cache.get_or_init(|| Shared::new(Shareable::new(list())))
    }

    // What the runs may hold of a shape budget of `bytes`.
    #[cfg(not(feature = "threading"))]
    fn set_runs_bytes(&self, _: usize, runs: usize) {
        let evicted = write_shared(self.shape_list()).set_budget(runs);
        self.dropped_runs(evicted);
    }

    #[cfg(not(feature = "threading"))]
    fn run_stats(&self) -> (usize, usize) {
        self.shape_cache.get().map_or((0, 0), |list| {
            let list = read_shared(list);
            (list.len(), list.bytes())
        })
    }

    #[cfg(not(feature = "threading"))]
    fn clear_runs(&self) {
        if let Some(list) = self.shape_cache.get() {
            write_shared(list).clear();
        }
    }

    #[cfg(not(feature = "threading"))]
    fn set_runs_pool(&self, pool: Option<&Shared<lru::Pool>>) {
        let list = self.shape_list();
        lru::rejoin(core::slice::from_mut(&mut write_shared(list)), pool, &lru::member(list), |_| 0);
    }

    #[cfg(not(feature = "threading"))]
    fn runs_pool(&self) -> Option<Shared<lru::Pool>> {
        read_shared(self.shape_list()).pool()
    }

    // Made at the first location read, so a font read at its default alone, and every location's own cache, holds none.
    pub(super) fn locations(&self) -> &Shared<Locations> {
        self.locations.get_or_init(|| {
            let list = lru::ByteLru::keeping_newest(INSTANCE_BUDGET_BYTES, located_cost).telling(Located::release);
            let dropped = crate::daecore::daekit::base::sync::Counter::default();
            Shared::new(Locations { list: mutable(list), dropped })
        })
    }

    pub fn set_instance_cache_bytes(&self, bytes: usize) {
        let mut locations = write(&self.locations().list);
        let evicted = locations.set_budget(bytes);
        self.dropped(evicted);
        self.font.spare.clear();
    }

    // The spare tables count with the locations that gave them back.
    pub fn instance_cache_stats(&self) -> (usize, usize) {
        let locations = self.locations.get().map_or((0, 0), |at| {
            let list = read(&at.list);
            (list.len(), list.bytes())
        });
        (locations.0, locations.1 + self.font.spare.bytes())
    }

    pub fn clear_instance_cache(&self) {
        let mut locations = write(&self.locations().list);
        let held = locations.len();
        locations.clear();
        self.dropped(held);
        self.font.spare.clear();
    }

    pub(crate) fn set_instance_pool(&self, pool: Option<&Shared<lru::Pool>>) {
        let at = self.locations();
        let mut locations = write(&at.list);
        let evicted = lru::rejoin(core::slice::from_mut(&mut locations), pool, &lru::member(at), |_| 0);
        self.dropped(evicted);
        drop(locations);
        if let Some(pool) = pool {
            pool.settle();
        }
    }

    // Unlike the others, this is what is left to spend rather than a ceiling. Building an index draws it
    // down, and a location's indexes give theirs back when the location is dropped.
    pub fn set_cmap_index_allowance(&self, bytes: usize) {
        if self.font.index_allowance.set(crate::daecore::daekit::base::sync::Counter::new(bytes)).is_err() {
            self.font.index_allowance().set(bytes);
        }
    }

    pub fn cmap_index_allowance(&self) -> usize {
        self.font.index_allowance().get()
    }

    // What this cache holds of its own: itself, the tables it doesn't share with its font, and what it has
    // built since.
    pub(crate) fn held_bytes(&self) -> usize {
        use crate::daecore::daetype::instancer::MeasuredGlyphs;
        let glyphs = self.drawn.as_deref().map_or(0, drawn::DrawnGlyphs::bytes)
            + self.measured.as_deref().map_or(0, MeasuredGlyphs::bytes)
            + self.stored_measured.get().and_then(Option::as_ref).map_or(0, MeasuredGlyphs::bytes)
            + self.cff2_at.as_ref().map_or(0, drawn::Cff2Location::bytes);
        crate::daecore::daekit::base::sync::shared_of::<FontCache>() + self.held.total() + glyphs
    }

    // A font without head's em or maxp's count, which everything measured rests on, is refused here, once.
    // Inlined with `with_header`, so the cache is built where its caller keeps it rather than copied there whole.
    #[inline(always)]
    pub fn new(table_map: Tables) -> Result<Self, Error> {
        let upm = font_header(&table_map).and_then(|h| read_u16_be(h, 18)).filter(|&v| v > 0);
        let upm = upm.ok_or(Error::missing("head: missing, or no unitsPerEm"))?;
        let num_glyphs = crate::daecore::daetype::decoder::num_glyphs(&table_map);
        let num_glyphs = num_glyphs.ok_or(Error::missing("maxp: missing, or no glyph count"))?;
        // Made where it's kept: `new_cyclic` allocates before the state is built, where `new` built its 1.4 KB on
        // the stack and copied it over. The weak handle goes unused.
        let font = Shared::new_cyclic(|_| FontState {
            fvar: table_map.get("fvar").cloned(),
            axes: Once::new(),
            varc: table_map.contains_key("VARC"),
            varc_default: Once::new(),
            cmap: CmapState::default(),
            index_allowance: Once::new(),
            cff2_outlines: Once::new(),
            cff2_kept: Once::new(),
            instance_reads: Once::new(),
            drawn_advances: Once::new(),
            drawn_bearings: Once::new(),
            tables: table_map,
            described_tables: Once::new(),
            glyf_points: Once::new(),
            normalizer: Once::new(),
            advance_views: Once::new(),
            spare: crate::daecore::daekit::base::sync::SpareSlots::default(),
        });
        Ok(FontCache::with_header(font, false, upm, num_glyphs))
    }

    // For an instance of a font already checked: instancing changes neither the em, the glyph count nor the long
    // metrics' counts, which a described location reads from the font's hhea and vhea.
    #[inline(always)]
    pub(crate) fn with_header(font: Shared<FontState>, described: bool, upm: u16, num_glyphs: u32) -> Self {
        let table_map = if described { font.described_held() } else { &font.tables };
        let tags = crate::daecore::daetype::decoder::glyf_tags(table_map);
        let glyf = table_map.get(tags.glyf).cloned();
        let cff = table_map.get("CFF ").cloned();
        let cff2 = table_map.contains_key("CFF2");
        // Asked only of a font without the outlines already in hand, which have answered it.
        let bitmap_only = glyf.is_none() && cff.is_none() && !cff2
            && crate::daecore::daetype::bitmap::bitmap_only(table_map);
        let metrics = [false, true].map(|vertical| {
            let tags = shaped_tags(table_map, vertical);
            let long = tags.stated(tags.header(table_map)?)?;
            Some((table_map.get(tags.mtx)?.clone(), u32::try_from(long).ok()?))
        });
        let advance_shapes = [0, 1].map(|v| metrics[v].as_ref().and_then(|(data, long)| {
            metrics::AdvanceShape::of(data, *long as usize, num_glyphs as usize)
        }));
        Self {
            font,
            upm,
            num_glyphs,
            os2: Once::new(),
            glyf,
            glyf_extended: tags.extended,
            cff,
            cff2,
            bitmap_only,
            line_extents: [Once::new(), Once::new()],
            metrics,
            advance_shapes,
            locations: Once::new(),
            listed_advances: core::array::from_fn(|_| Once::new()),
            advances_past: [0; 2],
            varied_advances: [None, None],
            varied_origins: None,
            described: None,
            glyf_tables: Once::new(),
            measured: None,
            stored_measured: Once::new(),
            location: None,
            variation: Once::new(),
            #[cfg(not(feature = "threading"))]
            location_by_axis: mutable(Vec::new()),
            default_axes: Once::new(),
            #[cfg(not(feature = "threading"))]
            shape_cache: Once::new(),
            #[cfg(feature = "threading")]
            run_shards: Once::new(),
            #[cfg(feature = "threading")]
            runs_id: crate::daecore::daekit::base::sync::next_id(),
            #[cfg(feature = "threading")]
            runs_dropped: crate::daecore::daekit::base::sync::Counter::default(),
            shape_budget: crate::daecore::daekit::base::sync::Counter::new(SHAPE_CACHE_BYTES),
            outline_scratch: mutable(None),
            #[cfg(not(feature = "threading"))]
            spare_buffer: mutable(None),
            kept_plans: Once::new(),
            plan_cache: Once::new(),
            loca: Once::new(),
            drawn_gvar: Once::new(),
            drawn: None,
            origins: Once::new(),
            kept_extents: Once::new(),
            cff2_at: None,
            varc_at: None,
            cff_outlines_cache: Once::new(),
            gdef_var_store: Once::new(),
            color_tables: Once::new(),
            colr_v1_scalars: mutable(None),
            hint_context: mutable(Vec::new()),
            autohinter: mutable(None),
            gdef_class_indexes: Once::new(),
            aat_reads: core::array::from_fn(|_| Once::new()),
            otl_reads: core::array::from_fn(|_| Once::new()),
            lookup_indexes: [mutable(Default::default()), mutable(Default::default())],
            aat_sets: core::array::from_fn(|_| mutable(Vec::new())),
            held: Held::default(),
            charge: lru::Charge::default(),
            index_spent: crate::daecore::daekit::base::sync::Counter::default(),
        }
    }
}
