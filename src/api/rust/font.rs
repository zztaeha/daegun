#[cfg(doc)]
use crate::ErrorKind::{InvalidArgument, LimitExceeded, Malformed, Missing, Unsupported};
use super::*;
use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::borrow::Borrow;
use core::iter::FusedIterator;
use crate::daecore::daetype::table_bytes::Source;
use crate::daecore::daetype::outline::Path;

/// One of a font's caches, each with a budget of its own.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Cache {
    /// Shaped runs, for text shaped again.
    Shape,
    /// Each location's tables and what is built there, under one budget.
    Instance,
    /// Outlines [`Font::prewarm`] drew ahead, replayed in place of being decoded.
    Outline,
    /// Images a [`Rasterizer`](crate::raster::Rasterizer) drew of the font's glyphs, lent again in place of being
    /// drawn. Off, at a budget of 0, until set.
    Raster,
}

/// A budget the caches of several fonts share, each joining it with [`Font::set_cache_pool`]: past it, the entry least
/// recently used across all of them goes first, whichever font's. Caches of several kinds may share one. Shared across
/// threads with the `threading` feature, as a font is.
///
/// # Examples
///
/// ```
/// use daegun::{Cache, CachePool, Font, Location};
/// use daegun::shape::ShapeOptions;
///
/// # fn main() -> daegun::Result<()> {
/// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
/// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
/// let pool = CachePool::new(1 << 20);
/// let fonts = [Font::from_bytes(&bytes)?, Font::from_bytes(&bytes)?];
/// for font in &fonts {
///     font.set_cache_pool(Cache::Shape, Some(&pool));
///     let _ = font.shape("Shared", &Location::default(), &ShapeOptions::DEFAULT);
/// }
/// // Both runs, the two fonts' caches together, held under the one budget.
/// assert!(pool.stats().entries >= 2 && pool.stats().bytes <= 1 << 20);
/// # Ok(())
/// # }
/// ```
pub struct CachePool(crate::sync::Shared<crate::daecore::daekit::cache::lru::Pool>);

impl CachePool {
    /// A pool of `bytes`, a ceiling its caches grow into together, never memory reserved.
    pub fn new(bytes: usize) -> CachePool {
        CachePool(crate::sync::Shared::new(crate::daecore::daekit::cache::lru::Pool::new(bytes)))
    }

    /// Sets the pool's budget in bytes. A lower budget applies at once, keeping what was used last across its caches.
    pub fn set_budget(&self, bytes: usize) {
        self.0.set_budget(bytes);
    }

    /// What the caches in the pool hold now, together.
    pub fn stats(&self) -> CacheStats {
        let (entries, bytes) = self.0.stats();
        CacheStats { entries, bytes }
    }
}

impl core::fmt::Debug for CachePool {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CachePool").field("stats", &self.stats()).finish_non_exhaustive()
    }
}

/// What a cache holds.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct CacheStats {
    /// How many entries.
    pub entries: usize,
    /// Their bytes, as the budget counts them.
    pub bytes: usize,
}

impl Font {
    /// The TrueType or OpenType font in `bytes`, which it copies.
    ///
    /// # Errors
    ///
    /// [`Malformed`] for bytes that aren't a font file or whose tables lie past them, and [`Missing`] for a font
    /// without head's unitsPerEm or maxp's glyph count.
    pub fn from_bytes(bytes: &[u8]) -> Result<Font> {
        let table_map = crate::daecore::daetype::decoder::extract_ttf_tables(bytes)?;
        Ok(Font::wrap(FontCache::new(table_map)?))
    }

    /// The font in `bytes`, or the one at `index` in the collection they hold, kept as given rather than copied: a
    /// collection's face reads its tables where they lie in the whole file. `index` is 0 for a font file.
    ///
    /// # Errors
    ///
    /// As [`from_collection`](Font::from_collection) gives them.
    pub fn from_vec(bytes: Vec<u8>, index: usize) -> Result<Font> {
        let table_map = crate::daecore::daetype::decoder::extract_tables_in_place(Source::Owned(bytes), index)?;
        Ok(Font::wrap(FontCache::new(table_map)?))
    }

    /// The font at `index` in the TrueType or OpenType collection in `bytes`, or a font file's own at 0, which it
    /// copies: the face's tables from a collection, the whole file from a font file, as
    /// [`from_bytes`](Font::from_bytes) does.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`] for an index past the faces [`face_count`](Font::face_count) counts, and else as
    /// [`from_bytes`](Font::from_bytes) gives them.
    #[doc(alias("from_ttc"))]
    pub fn from_collection(bytes: &[u8], index: usize) -> Result<Font> {
        use crate::daecore::daetype::decoder::{extract_ttc_tables, extract_ttf_tables, is_collection, ONE_FACE};
        let table_map = match index {
            _ if is_collection(bytes) => extract_ttc_tables(bytes, index)?,
            0 => extract_ttf_tables(bytes)?,
            _ => return Err(ONE_FACE),
        };
        Ok(Font::wrap(FontCache::new(table_map)?))
    }

    /// The font in `bytes`, or the one at `index` in the collection they hold, read where it lies: nothing is
    /// copied. `index` is 0 for a font file. A file mapped into memory for the program's life can come this way, its
    /// mapping leaked; the bytes must not change while a font reads them.
    ///
    /// # Errors
    ///
    /// As [`from_collection`](Font::from_collection) gives them.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::Font;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// // Bytes kept for the program's life, as a file mapped into memory would be.
    /// let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
    /// let font = Font::from_static(bytes, 0)?;
    /// # assert!(font.units_per_em() > 0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_static(bytes: &'static [u8], index: usize) -> Result<Font> {
        let table_map = crate::daecore::daetype::decoder::extract_tables_in_place(Source::Static(bytes), index)?;
        Ok(Font::wrap(FontCache::new(table_map)?))
    }

    /// How many fonts `bytes` hold, each opened at its index by [`from_collection`](Font::from_collection) or
    /// [`from_static`](Font::from_static): a collection's faces, 1 for a font file, 0 for bytes that are neither.
    #[doc(alias("ttc_font_count"))]
    pub fn face_count(bytes: &[u8]) -> usize {
        crate::daecore::daetype::decoder::face_count(bytes)
    }

    /// Whether fvar gives the font at least one axis.
    pub fn is_variable(&self) -> bool {
        crate::daecore::daetype::decoder::is_variable_font(self.cache.font_tables())
    }

    /// The font's variation axes in fvar's order; none for a static font or an fvar that doesn't read.
    pub fn axes(&self) -> Vec<Axis> {
        crate::daecore::daetype::decoder::parse_fvar_axes(self.cache.font_tables()).unwrap_or_default()
    }

    /// The instances fvar names, in its order; none for a static font or an fvar that doesn't read.
    pub fn named_instances(&self) -> Vec<NamedInstance> {
        crate::daecore::daetype::decoder::read_fvar_instances(self.cache.font_tables()).unwrap_or_default()
    }

    // Static to the instancer: an fvar it cannot read. One that reads, axes or none, is instanced.
    pub(crate) fn is_static(&self) -> bool {
        self.cache.is_static()
    }

    /// Font units to the em, head's unitsPerEm: what every metric and outline is measured in.
    #[doc(alias("upm"))]
    pub fn units_per_em(&self) -> u16 {
        self.cache.font_upm()
    }

    /// How many glyphs the font has: MAXP's 24-bit count where it reads whole and glyf and loca, or CFF2's
    /// CharStrings, hold that many glyphs, else maxp's numGlyphs.
    #[doc(alias("num_glyphs"))]
    pub fn glyph_count(&self) -> u32 {
        self.cache.font_num_glyphs()
    }

    /// The family name at a location, from the name table the instance there would carry: the typographic family,
    /// name ID 16, else the family, ID 1, each read as [`Font::name`] reads one.
    pub fn family_name(&self, location: &Location) -> Option<String> {
        self.cache.names_at(&self.key(location), crate::daecore::daetype::decoder::read_font_family_name)
    }

    /// Whether the font is upright, italic or oblique at a location.
    pub fn style(&self, location: &Location) -> FontStyle {
        let style = |fc: &FontCache| crate::daecore::daetype::decoder::read_font_style(fc.tables());
        self.cache.described_at(&self.key(location), style)
    }

    /// Every name the name table the instance at a location would carry, by name ID, each read as [`Font::name`]
    /// reads it.
    pub fn names(&self, location: &Location) -> BTreeMap<NameId, String> {
        self.cache.names_at(&self.key(location), crate::daecore::daetype::decoder::parse_all_name_strings::<NameId>)
    }

    /// Each name [`Font::names`] gives, in order of ID, lent to `visit` as it is read rather than collected, so no
    /// name takes a string of its own.
    pub fn visit_names(&self, location: &Location, visit: impl FnMut(NameId, &str)) {
        struct Visit<F>(F);
        impl<F: FnMut(NameId, &str)> crate::daecore::daetype::decoder::NameSink for Visit<F> {
            fn put(&mut self, name_id: u16, name: &str) {
                (self.0)(NameId(name_id), name);
            }
        }
        let each = |names: &_| crate::daecore::daetype::decoder::each_name_string(names, &mut Visit(visit));
        self.cache.names_at(&self.key(location), each);
    }

    // The same, each name lent to `out` as it is read rather than collected, which C puts in one buffer.
    #[cfg(feature = "capi")]
    pub(crate) fn names_into(&self, location: &Location, out: &mut impl crate::daecore::daetype::decoder::NameSink) {
        let each = |names: &_| crate::daecore::daetype::decoder::each_name_string(names, out);
        self.cache.names_at(&self.key(location), each);
    }

    /// One name by its ID at a location; none where the name table has no record for it. Of its records, it reads an
    /// English one for Windows, else another for Windows, else an English one for the Mac in Mac Roman, else another
    /// in Mac Roman, else a Unicode one, the first of each in the table's order, trimmed of NULs and spaces.
    /// [`Font::localized_names`] gives each language's.
    #[doc(alias("name_string"))]
    pub fn name(&self, name_id: NameId, location: &Location) -> Option<String> {
        let read = |names: &_| crate::daecore::daetype::decoder::read_name_string(names, name_id.0);
        self.cache.names_at(&self.key(location), read)
    }

    /// One name by its ID in each language the name table the instance at a location would carry records it in,
    /// sorted by language; none where it has no record for the ID.
    ///
    /// ```
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/source-han-sans/SourceHanSansJP-VF.otf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// use daegun::{Font, Location, NameId};
    ///
    /// let font = Font::from_bytes(&bytes)?;
    /// let names = font.localized_names(NameId(1), &Location::default());
    /// let japanese = names.iter().find(|n| n.language == "ja").expect("a Japanese family name");
    /// assert_eq!(japanese.name, "源ノ角ゴシック JP VF");
    /// # Ok(())
    /// # }
    /// ```
    pub fn localized_names(&self, name_id: NameId, location: &Location) -> Vec<crate::LocalizedName> {
        let read = |names: &_| crate::daecore::daetype::decoder::read_localized_names(names, name_id.0);
        self.cache.names_at(&self.key(location), read)
    }

    // The same, each lent to `put` as it is read rather than collected, which C puts in two buffers.
    #[cfg(feature = "capi")]
    pub(crate) fn localized_names_into(&self, name_id: NameId, location: &Location, put: impl FnMut(&str, &str)) {
        let each = |names: &_| crate::daecore::daetype::decoder::each_localized_name(names, name_id.0, put);
        self.cache.names_at(&self.key(location), each);
    }

    /// The font's bytes as a static font at a location; a static font is its own instance.
    ///
    /// # Errors
    ///
    /// The instancer's, for a location it can't build: [`Malformed`] for a table that doesn't read, [`Missing`] for one
    /// it needs, [`Unsupported`] for one it can't vary, and [`LimitExceeded`] past its bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::{Font, Tag};
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let bold = font.location(&[(Tag::new(b"wght"), 700.0)]);
    /// let file = font.instance(&bold)?;
    /// // A static font at wght 700, to write out or open again, shared rather than copied when asked for again.
    /// let static_bold = Font::from_vec(file.to_vec(), 0)?;
    /// # assert!(static_bold.axes().is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn instance(&self, location: &Location) -> Result<Arc<[u8]>> {
        self.cache.get_or_instance(&self.key(location))
    }

    /// The instance's tables at a location, borrowed where the location leaves one as stored. A static font, or
    /// one whose fvar doesn't read, is its own instance.
    ///
    /// # Errors
    ///
    /// As [`instance`](Font::instance) gives them.
    pub fn instance_tables(&self, location: &Location) -> Result<BTreeMap<Tag, Cow<'_, [u8]>>> {
        if self.is_static() {
            crate::daecore::daetype::decoder::copies_held(self.cache.font_tables())?;
            let tables = self.cache.font_tables().tagged();
            return Ok(tables.map(|(tag, data)| (tag, Cow::Borrowed(data.as_slice()))).collect());
        }
        let (map, reads) = (self.cache.font_tables(), self.cache.instance_reads());
        let tables = crate::daecore::daetype::instancer::instance_tables_from_map(map, reads, &self.key(location))?;
        let tag = |name: &str| crate::daecore::daekit::cache::tag_bytes(name).map(|b| Tag::new(&b));
        Ok(tables.into_iter().filter_map(|(name, data)| Some((tag(&name)?, data))).collect())
    }

    /// A table's bytes as stored, none where the font has no table of that tag.
    pub fn table(&self, tag: Tag) -> Option<&[u8]> {
        self.cache.font_tables().get_tagged(tag).map(|t| t.as_slice())
    }

    /// Every table's tag, in tag order.
    pub fn table_tags(&self) -> impl ExactSizeIterator<Item = Tag> + DoubleEndedIterator + FusedIterator + Clone {
        self.cache.font_tables().tagged().map(|(tag, _)| tag)
    }

    /// Whether the font has a table of that tag.
    pub fn has_table(&self, tag: Tag) -> bool {
        self.cache.font_tables().get_tagged(tag).is_some()
    }

    /// Sets a cache's budget in bytes: a ceiling it grows into, never memory reserved. A lower budget applies at
    /// once, keeping what was used last. The newest instance is kept whatever the budget, so one location runs on
    /// a budget of 0; and a thread keeps the location it last read until it reads another or ends. Under `threading`
    /// the shape cache keeps runs read again since they came in ahead of those that weren't, rather than strictly
    /// in the order last used; a shape budget of 4 MB or more is split over lists, each keeping its own, and one of
    /// 0 sets them aside, so threads shaping meet at no lock.
    #[doc(alias("set_instance_cache_bytes", "set_outline_cache_bytes", "set_shape_cache_bytes"))]
    pub fn set_cache_budget(&self, cache: Cache, bytes: usize) {
        match cache {
            Cache::Shape => self.cache.set_shape_cache_bytes(bytes),
            Cache::Instance => self.cache.set_instance_cache_bytes(bytes),
            Cache::Outline => {
                let at = self.outlines();
                let mut outlines = crate::sync::write(&at.list);
                outlines.set_budget(bytes);
                at.prewarmed.set(outlines.len());
            }
            Cache::Raster => {
                crate::sync::write(self.rasters()).set_budget(bytes);
                self.raster_budget.set(bytes);
            }
        }
    }

    /// What a cache holds now.
    #[doc(alias("instance_cache_stats", "outline_cache_stats", "shape_cache_stats"))]
    pub fn cache_stats(&self, cache: Cache) -> CacheStats {
        let (entries, bytes) = match cache {
            Cache::Shape => self.cache.shape_cache_stats(),
            Cache::Instance => self.cache.instance_cache_stats(),
            Cache::Outline => self.outlines.get().map_or((0, 0), |at| {
                let outlines = crate::sync::read(&at.list);
                (outlines.len(), outlines.bytes())
            }),
            Cache::Raster => self.rasters.get().map_or((0, 0), |at| {
                let rasters = crate::sync::read(at);
                (rasters.len(), rasters.bytes())
            }),
        };
        CacheStats { entries, bytes }
    }

    /// Empties a cache.
    #[doc(alias("clear_prewarm", "clear_shape_cache"))]
    pub fn clear_cache(&self, cache: Cache) {
        match cache {
            Cache::Shape => self.cache.clear_shape_cache(),
            Cache::Instance => self.cache.clear_instance_cache(),
            Cache::Outline => {
                if let Some(at) = self.outlines.get() {
                    crate::sync::write(&at.list).clear();
                    at.prewarmed.set(0);
                }
            }
            Cache::Raster => {
                if let Some(at) = self.rasters.get() {
                    crate::sync::write(at).clear();
                }
            }
        }
    }

    /// Draws glyphs at a location into the outline cache, so [`Font::outline`] replays them. Answers how many it
    /// added; a glyph already there, one that draws nothing and one past the budget add none. Takes glyphs by value
    /// or by reference, so a run's `&run.glyphs` serves as it is.
    pub fn prewarm(&self, glyphs: impl IntoIterator<Item: Borrow<GlyphId>>, location: &Location) -> usize {
        let axes = self.cache.resolve_axes(&self.key(location));
        let (cache, mut added) = (self.outlines(), 0);
        for glyph in glyphs {
            let GlyphId(gid) = *glyph.borrow();
            let key = (gid, crate::sync::Shared::clone(&axes));
            if crate::sync::write(&cache.list).get(&key).is_some() { continue; }
            let mut path = Path::default();
            if self.outline_glyph_keyed(gid, &axes.key, &mut path).is_none() { continue; }
            if path.is_empty() { continue; }
            path.shrink_to_fit();
            let mut outlines = crate::sync::write(&cache.list);
            if outlines.insert(key, crate::sync::Shared::new(path)).is_some() {
                added += 1;
            }
            cache.prewarmed.set(outlines.len());
            let over = outlines.over();
            drop(outlines);
            crate::daecore::daekit::cache::lru::settle(over);
        }
        added
    }

    /// Draws a cache's bytes from a pool the caches of other fonts share, or, given none, from this font's budget
    /// alone again. It keeps to both its own budget and the pool's: past its own it lets its oldest go, and past the
    /// pool's the oldest across every cache in the pool goes, whichever font's; a budget of `usize::MAX` leaves the
    /// pool alone in charge. What it holds moves with it, oldest first, and a font dropped gives the pool its bytes
    /// back. In a pool, every cache keeps entries read again since they came in ahead of those that weren't, as the
    /// shape cache does under `threading`, and each font's newest location stays past the pool's budget as past its
    /// own. Plans the shape cache keeps past its first few count with its runs, under a sixteenth of its budget.
    pub fn set_cache_pool(&self, cache: Cache, pool: Option<&CachePool>) {
        use crate::daecore::daekit::cache::lru;
        let pool = pool.map(|pool| &pool.0);
        match cache {
            Cache::Shape => self.cache.set_shape_pool(pool),
            Cache::Instance => self.cache.set_instance_pool(pool),
            Cache::Outline => {
                let at = self.outlines();
                let mut outlines = crate::sync::write(&at.list);
                lru::rejoin(core::slice::from_mut(&mut outlines), pool, &lru::member(at), |_| 0);
                at.prewarmed.set(outlines.len());
            }
            Cache::Raster => {
                let at = self.rasters();
                lru::rejoin(core::slice::from_mut(&mut crate::sync::write(at)), pool, &lru::member(at), |_| 0);
            }
        }
        if let Some(pool) = pool {
            pool.settle();
        }
    }

    /// Sets what every index the font and its locations build may still spend, in bytes. Unlike a cache's budget
    /// this is drawn down as an index is built, and given back when a location is dropped. It starts at 4 MiB plus
    /// eight times the bytes of the font's tables, more than any real font's indexes take.
    #[doc(alias("set_cmap_index_allowance"))]
    pub fn set_index_allowance(&self, bytes: usize) {
        self.cache.set_cmap_index_allowance(bytes);
    }

    /// What the indexes may still spend, in bytes.
    #[doc(alias("cmap_index_allowance"))]
    pub fn index_allowance(&self) -> usize {
        self.cache.cmap_index_allowance()
    }
}
