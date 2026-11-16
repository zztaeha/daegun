use alloc::string::String;
use alloc::vec::Vec;
use crate::cache::FontCache;
use crate::daecore::{GlyphId, NameId, Tag};

pub use crate::daecore::daetype::decoder::{Axis, FontStyle, LocalizedName, NamedInstance};

pub use crate::daecore::daekit::base::error::{Error, ErrorKind, Result};

/// What GDEF says a glyph is.
#[expect(clippy::exhaustive_enums, reason = "GDEF defines these four")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphClass {
    /// A base glyph, one marks attach to.
    Base,
    /// A ligature of several characters.
    Ligature,
    /// A mark, attached to another glyph.
    Mark,
    /// A part of a glyph built from several.
    Component,
}

// The decoding a glyph's outline took, drawn or not, adds to `work`: VARC's decodes, charstring steps, and glyf's
// glyphs read and points decoded.
fn draw_glyph_outline(
    cache: &FontCache, gid: u32, work: &mut usize, pen: &mut dyn crate::daecore::daetype::outline::OutlinePen,
) -> Result<(), Error> {
    if cache.draw_varc(gid, work, pen).is_some() {
        return Ok(());
    }
    if let Some(cff) = cache.cff() {
        let outlines = cache.cff_outlines().ok_or(Error::malformed("CFF: the table does not parse"))?;
        return crate::daecore::daetype::outline::outline_cff_glyph_counted(outlines, cff, gid, work, pen);
    }
    if let Some(drawn) = cache.draw_cff2(gid, None, work, pen) {
        return drawn.map(drop);
    }
    if cache.glyf().is_some() {
        let loca = cache.loca_offsets().ok_or(Error::missing("loca: missing or malformed"))?;
        return cache.draw_glyf_reusing(loca.into(), gid, work, pen);
    }
    Err(Error::new(crate::daecore::ErrorKind::NoOutline, "the font has no outlines"))
}

/// An open font: one face of a file, read lazily, with the caches that make calls on it again cheap. Shared across
/// threads with the `threading` feature.
pub struct Font {
    cache: FontCache,
    // Each made at its first use, so an open allocates neither.
    outlines: crate::sync::Once<crate::sync::Shared<Outlines>>,
    rasters: crate::sync::Once<crate::sync::Shared<crate::sync::Mutable<RasterCache>>>,
    // The raster cache's budget, kept in step under its lock, so a draw with the cache off takes no lock.
    raster_budget: crate::sync::Counter,
    names: crate::sync::Once<crate::daecore::daetype::glyph_names::NameIndex>,
    // Which font a `Location` was made by: its own are read as made, another's resolved again.
    token: crate::sync::Id,
}

impl core::fmt::Debug for Font {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Font")
            .field("units_per_em", &self.units_per_em())
            .field("glyph_count", &self.glyph_count())
            .finish_non_exhaustive()
    }
}

// What may cross threads, held per feature set: every result and option always, and a font with its locations under
// `threading`, which without it share by `Rc`.
const _: () = {
    const fn shareable<T: Send + Sync>() {}
    shareable::<Error>();
    shareable::<shape::ShapeOptions<'static>>();
    shareable::<layout::LayoutOptions<'static>>();
    shareable::<layout::JustifyOptions>();
    shareable::<outline::OutlineOptions>();
    shareable::<color::SceneOptions>();
    shareable::<shape::ShapedRun>();
    shareable::<shape::BidiRun>();
    shareable::<layout::TextLayout>();
    shareable::<layout::Justified>();
    shareable::<outline::Path>();
    shareable::<outline::HintedOutline>();
    shareable::<color::DisplayList>();
    shareable::<color::PaintGraph>();
    shareable::<tables::Subset>();
    #[cfg(feature = "threading")]
    shareable::<Font>();
    #[cfg(feature = "threading")]
    shareable::<Location>();
};

// What a `catch_unwind` may hold without `AssertUnwindSafe`: every result always, and a font with its locations under
// `threading`, whose locks ignore poisoning, where without it they hold `RefCell`s.
const _: () = {
    const fn unwinds<T: core::panic::UnwindSafe + core::panic::RefUnwindSafe>() {}
    unwinds::<Error>();
    unwinds::<shape::ShapedRun>();
    unwinds::<shape::BidiRun>();
    unwinds::<layout::TextLayout>();
    unwinds::<layout::Justified>();
    unwinds::<outline::Path>();
    unwinds::<outline::HintedOutline>();
    unwinds::<color::DisplayList>();
    unwinds::<color::PaintGraph>();
    unwinds::<tables::Subset>();
    #[cfg(feature = "threading")]
    unwinds::<Font>();
    #[cfg(feature = "threading")]
    unwinds::<Location>();
};

// A font is moved whole as it opens and wherever a caller keeps it; `FontCache` holds its own limit.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Font>() <= 2304);

/// Lets go of what this thread keeps between calls: the locations it last read at, its hinters, its recent runs and
/// the scratch it reuses. What it last read and hinted with holds those fonts' bytes, so a pool's worker calls this
/// as it goes idle or once a font is dropped. Other threads keep theirs, and this thread's next call builds again what
/// it needs. Without `std` no thread keeps anything, and this does nothing.
pub fn release_thread() {
    #[cfg(feature = "std")]
    {
        crate::sync::release_thread();
        outline::release_thread();
    }
}

mod font;
pub use font::{Cache, CachePool, CacheStats};
mod location;
pub use location::Location;

pub mod metrics;
mod glyphs;
#[cfg(feature = "capi")]
pub(crate) use glyphs::AdvanceList;
pub mod shape;
pub mod layout;
pub mod unicode;
pub mod outline;
#[doc(alias("paint"))]
pub mod color;
pub mod math;
pub mod tables;
pub mod raw;
pub mod raster;

type OutlineCache = crate::daecore::daekit::cache::lru::ByteLru<
    (u32, crate::sync::Shared<crate::sync::Lined<crate::daecore::daekit::cache::ResolvedAxes>>),
    crate::sync::Shared<crate::daecore::daetype::outline::Path>,
>;

type RasterCache = crate::daecore::daekit::cache::raster::RasterCache;

const OUTLINE_CACHE_BYTES: usize = 4 * 1024 * 1024;

// The outline cache, behind one lock a pool reaches too, and how many outlines it keeps, kept in step under that lock:
// a draw reads the count, not the lock, as nearly every font has none.
struct Outlines {
    list: crate::sync::Mutable<OutlineCache>,
    prewarmed: crate::sync::Counter,
}

impl crate::daecore::daekit::cache::lru::Member for Outlines {
    fn evict(&self, pool: &crate::daecore::daekit::cache::lru::Pool, _: usize, stamp: u64) {
        let mut list = crate::sync::write(&self.list);
        list.evict_listed(pool, stamp);
        self.prewarmed.set(list.len());
    }
}

// What a kept outline costs beyond its path: the Arc around it and its place in the cache's two maps,
// 174 to 176 bytes measured on Inter, EB Garamond and STIX.
const OUTLINE_ENTRY_BYTES: usize = 176;

impl Font {
    // Inlined, as the cache's constructors are, so an open builds the font in place: 2,250 cycles, then 2,030.
    #[inline(always)]
    fn wrap(cache: FontCache) -> Font {
        Font {
            cache,
            outlines: crate::sync::Once::new(),
            rasters: crate::sync::Once::new(),
            raster_budget: crate::sync::Counter::default(),
            names: crate::sync::Once::new(),
            token: crate::daecore::daekit::base::sync::next_id(),
        }
    }

    fn outlines(&self) -> &crate::sync::Shared<Outlines> {
        self.outlines.get_or_init(|| {
            let list = crate::sync::mutable(OutlineCache::new(OUTLINE_CACHE_BYTES, |_, p| {
                p.cost() + OUTLINE_ENTRY_BYTES
            }));
            crate::sync::Shared::new(Outlines { list, prewarmed: crate::sync::Counter::default() })
        })
    }

    fn rasters(&self) -> &crate::sync::Shared<crate::sync::Mutable<RasterCache>> {
        let list = crate::daecore::daekit::cache::raster::raster_cache;
        self.rasters.get_or_init(|| crate::sync::Shared::new(crate::sync::mutable(list())))
    }
}
