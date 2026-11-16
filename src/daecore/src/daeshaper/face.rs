#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use crate::daecore::daekit::cache::{Advances, FontCache, GlyphExtents, VerticalOrigins};
use crate::daecore::daetype::format::ivs::{round_fixed, store_delta, store_scalars, units, ItemVariationStore};
use crate::daecore::daetype::format::sanitize::OtlTable;
use super::ot::{gdef_class, ClassDef, Coverage, Gdef, LayoutTable, COVERAGE_ABSENT};
use crate::daecore::daetype::format::index::SparseIndex;
use crate::daecore::daekit::base::sync::Shared;

// What a plan takes of GSUB's and GPOS's FeatureVariations at a location: the record each takes, and the conditions
// of its lookup variations that hold there.
#[derive(Clone, Default, PartialEq)]
pub(crate) struct LayoutVariation {
    pub(crate) records: [Option<u32>; 2],
    pub(crate) conditions: [alloc::vec::Vec<u32>; 2],
}

impl LayoutVariation {
    // At `coords` in F2Dot14, a value condition read from GDEF's `store`.
    fn at(font: &FontCache, coords: &[i32], store: Option<&ItemVariationStore>) -> Self {
        let tables = [OtlTable::Gsub, OtlTable::Gpos].map(|t| font.layout_table(t).and_then(|t| LayoutTable::parse(t)));
        LayoutVariation {
            records: tables.each_ref().map(|t| t.as_ref()?.find_variation_index(coords, store)),
            conditions: tables
                .each_ref()
                .map(|t| t.as_ref().map_or_else(Default::default, |t| t.lookup_conditions(coords, store))),
        }
    }
}

// What shaping reads of a location beyond its metrics, built once there: what a plan takes of FeatureVariations,
// whether that differs from the default's, which a plan can't share, and GPOS's device deltas, none where all are 0.
pub(crate) struct Variation {
    layout: LayoutVariation,
    own_plan: bool,
    deltas: Option<Deltas>,
}

impl Variation {
    pub(crate) fn new(font: &FontCache, location: &[f64]) -> Self {
        let store = font.gdef_var_store();
        let coords: alloc::vec::Vec<i32> = location.iter().map(|&v| (v * 16384.0).round() as i32).collect();
        let layout = LayoutVariation::at(font, &coords, store.as_deref());
        let own_plan = layout != LayoutVariation::at(font, &[], None);
        let deltas = store.and_then(|store| Deltas::at(store, location));
        Variation { layout, own_plan, deltas }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.deltas.as_ref().map_or(0, |d| crate::daecore::daekit::base::sync::heap_of(&d.scalars))
    }
}

// GDEF's store and its regions' 16.16 scalars at the location.
pub(crate) struct Deltas {
    store: Shared<ItemVariationStore>,
    scalars: alloc::vec::Vec<i64>,
}

impl Deltas {
    // None where every coordinate is 0.
    pub(crate) fn at(store: Shared<ItemVariationStore>, location: &[f64]) -> Option<Deltas> {
        location.iter().any(|&v| v != 0.0).then(|| Deltas { scalars: store_scalars(&store, location), store })
    }

    // Rounded half up, as a delta baked into an instance is.
    pub(crate) fn of(&self, outer: u16, inner: u16) -> i32 {
        round_fixed(self.fixed(outer, inner))
    }

    // Unrounded, as an anchor keeps it until the difference of two is taken.
    pub(crate) fn exact(&self, outer: u16, inner: u16) -> f32 {
        units(self.fixed(outer, inner)) as f32
    }

    fn fixed(&self, outer: u16, inner: u16) -> i64 {
        store_delta(&self.store, outer.into(), inner.into(), &self.scalars)
    }
}

pub struct Face<'a> {
    cache: &'a FontCache,
    // The cache holding the location's metrics and measuring its glyphs, `cache` itself at the default.
    metrics: &'a FontCache,
    variation: Option<&'a Variation>,
    gdef: Option<Gdef<'a>>,
    gdef_class_index: Option<&'a SparseIndex>,
    gdef_mark_attach_index: Option<&'a SparseIndex>,
    mark_set_indexes: Option<&'a [Option<SparseIndex>]>,
    cmap_index: Option<&'a SparseIndex>,
    upm: u16,
    point_size: Option<f64>,
}

impl<'a> Face<'a> {
    // The cache's tables as they are: a location's cache is already instanced, so no device table varies them.
    pub fn new(cache: &'a FontCache) -> Self {
        cache.warm_cmap_index();
        let gdef = cache.gdef().and_then(|d| Gdef::new(d));
        let (classes, marks, sets) = cache.gdef_class_indexes(gdef.as_ref());

        Face {
            cache,
            metrics: cache,
            variation: None,
            point_size: None,
            gdef,
            gdef_class_index: classes.as_deref(),
            gdef_mark_attach_index: marks.as_deref(),
            mark_set_indexes: sets.as_deref().map(alloc::vec::Vec::as_slice),
            cmap_index: cache.cmap_index(),
            upm: cache.font_upm(),
        }
    }

    pub(crate) fn with_point_size(mut self, ptem: Option<f64>) -> Self {
        self.point_size = ptem;
        self
    }

    // The font's face at a location `at` describes, unless it's the font's own cache, at the default or where the
    // location can't be built.
    pub(crate) fn located(mut self, at: &'a FontCache) -> Self {
        if !core::ptr::eq(at, self.cache) {
            self.metrics = at;
            self.variation = Some(at.variation(self.cache));
        }
        self
    }

    // What a plan takes of FeatureVariations: the location's, or what holds at the default.
    pub(crate) fn layout_variation(&self) -> LayoutVariation {
        match self.variation {
            Some(v) => v.layout.clone(),
            None => LayoutVariation::at(self.cache, &[], None),
        }
    }

    // What sets a plan apart at a location: None where it takes the default's.
    pub(crate) fn variation_key(&self) -> Option<&LayoutVariation> {
        self.variation.filter(|v| v.own_plan).map(|v| &v.layout)
    }

    pub(crate) fn deltas(&self) -> Option<&Deltas> {
        self.variation?.deltas.as_ref()
    }

    pub(crate) fn tracking(&self, horizontal: bool) -> i32 {
        use crate::daecore::daetype::trak::{tracking, DEFAULT_POINT_SIZE};
        let adjust = tracking(self.cache.tables(), self.point_size.unwrap_or(DEFAULT_POINT_SIZE), horizontal);
        crate::daecore::daetype::format::round::ot_round(adjust)
    }

    pub(crate) fn upm(&self) -> u16 {
        self.upm
    }

    pub(crate) fn glyph_count(&self) -> u32 {
        self.cache.font_num_glyphs()
    }

    // The font's glyph for a character, none for one cmap leaves at .notdef.
    pub(crate) fn glyph_of(&self, c: u32) -> Option<u32> {
        match self.cmap_index {
            Some(index) => Some(u32::from(index.lookup(c))).filter(|&glyph| glyph != 0),
            None => self.cache.glyph_id(c),
        }
    }

    pub(crate) fn maps(&self, c: u32) -> bool {
        self.glyph_of(c).is_some()
    }

    // The glyph cmap's variation sequences give `base` under `selector`.
    pub(crate) fn variant_of(&self, base: u32, selector: u32) -> Option<u32> {
        self.cache.variation_glyph_id(base, selector)
    }

    pub(crate) fn advances(&self, vertical: bool) -> Advances<'a> {
        self.metrics.advances(vertical)
    }

    pub(crate) fn width(&self, glyph: u32) -> i32 {
        self.advances(false).of(glyph) as i32
    }

    // A glyph's advance down a line: vmtx's (`stated` as read), else `fallback`, for a glyph vmtx doesn't cover.
    pub(crate) fn height_or(&self, glyph: u32, stated: u32, fallback: i32) -> i32 {
        self.metrics.stated_vertical_advance(glyph, stated).map_or(fallback, |a| a as i32)
    }

    pub(crate) fn height(&self, glyph: u32) -> i32 {
        self.height_or(glyph, self.advances(true).of(glyph), self.line_height())
    }

    // Without vmtx a glyph advances the line's height, ascender to descender, so glyphs set down a line keep apart as
    // horizontal lines do.
    pub(crate) fn line_height(&self) -> i32 {
        self.metrics.vertical_fallback_advance() as i32
    }

    // A pass's vertical origins: VORG's where the font has one, which needs no outline.
    pub(crate) fn with_origins<T>(&self, f: impl FnOnce(&VerticalOrigins) -> T) -> T {
        match self.metrics.vorg_origins() {
            Some(vorg) => f(&vorg),
            None => f(&self.metrics.vertical_origins()),
        }
    }

    // Point `index` of `glyph`'s outline as TrueType numbers its points, at the face's location, from its drawn origin;
    // None for a glyph without glyf points or one past them.
    pub(crate) fn outline_point(&self, glyph: u32, index: u16) -> Option<(f64, f64)> {
        let (xs, ys) = self.metrics.contour_points(glyph)?;
        let at = usize::from(index);
        xs.get(at).zip(ys.get(at)).map(|(&x, &y)| (x, y))
    }

    pub(crate) fn extents(&self, glyph: u32) -> Option<GlyphExtents> {
        self.metrics.glyph_extents(glyph)
    }

    // Whether GDEF classes the glyphs; without it they take classes from their characters.
    pub(crate) fn gives_classes(&self) -> bool {
        self.gdef.as_ref().is_some_and(Gdef::has_glyph_classes)
    }

    // The glyph's class and mark attachment class; only a mark keeps an attachment class.
    pub(crate) fn classes(&self, glyph: u32) -> (u8, u8) {
        let Some(gdef) = self.gdef.as_ref() else { return (0, 0) };
        let read = |defs: Option<&ClassDef>, index| defs.map_or(0, |d| d.indexed(index).class_of(glyph));
        let class = read(gdef.glyph_classes.as_ref(), self.gdef_class_index);
        let attach = if class == 3 { read(gdef.mark_attach_classes.as_ref(), self.gdef_mark_attach_index) } else { 0 };
        gdef_class(class, attach)
    }

    pub(crate) fn mark_set_count(&self) -> u16 {
        self.gdef.as_ref().map_or(0, Gdef::mark_glyph_set_count)
    }

    pub(crate) fn in_mark_set(&self, glyph: u32, set: u16) -> bool {
        match self.mark_set_indexes.and_then(|s| s.get(usize::from(set))?.as_ref()) {
            Some(index) => index.lookup(glyph) != COVERAGE_ABSENT,
            None => self.gdef.as_ref().is_some_and(|g| g.is_mark_glyph(glyph, set)),
        }
    }

    // The index of the coverage or, with `class`, the class definition `data` at `start` in GSUB or GPOS, shared by
    // every subtable naming it; None for an empty one or one that can't be indexed.
    pub(crate) fn shared_index(
        &self,
        indexes: &mut crate::daecore::daekit::cache::OtlIndexes,
        start: usize,
        data: &[u8],
        class: bool,
    ) -> Option<Shared<SparseIndex>> {
        if data.is_empty() {
            return None;
        }
        self.cache.otl_index_in(indexes, (start, class), || match class {
            true => self.cache.build_index(&|e| ClassDef::new(data).each_entry(e), 0),
            false => self.cache.build_index(&|e| Coverage::new(data).each_entry(e), COVERAGE_ABSENT),
        })
    }

    pub(crate) fn build_index(
        &self,
        each: &crate::daecore::daekit::cache::shaping::IndexEntries,
        absent: u16,
    ) -> Option<SparseIndex> {
        self.cache.build_index(each, absent)
    }

    pub(crate) fn reserve_index_bytes(&self, bytes: usize) -> bool {
        self.cache.reserve_index_bytes(bytes)
    }

    pub(crate) fn draw_index_bytes(&self, bytes: usize) -> Option<crate::daecore::daekit::cache::shaping::Drawn> {
        self.cache.draw_index_bytes(bytes)
    }

    pub(crate) fn lookup_index(
        &self,
        table_index: usize,
        lookup_count: usize,
        index: u16,
        build: impl FnOnce(&mut crate::daecore::daekit::cache::OtlIndexes) -> super::ot::LookupIndex,
    ) -> Shared<super::ot::LookupIndex> {
        self.cache.lookup_index_cached(table_index, lookup_count, index, build)
    }

    pub(crate) fn subtable_sets(
        &self,
        table: crate::daecore::daetype::format::sanitize::AatTable,
        subtable_count: usize,
        index: usize,
        build: impl FnOnce() -> super::ot::aat::SubtableSets,
    ) -> Shared<super::ot::aat::SubtableSets> {
        self.cache.subtable_sets_cached(table, subtable_count, index, build)
    }

    pub(crate) fn table(&self, tag: &str) -> Option<&'a [u8]> {
        self.cache.table(tag).map(|t| t.as_slice())
    }

    // An AAT table where shaping reads it, and none where the sanitizer rejects it or finds no data.
    pub(crate) fn aat(&self, which: crate::daecore::daetype::format::sanitize::AatTable) -> Option<&'a [u8]> {
        self.cache.aat_table(which).map(|t| t.as_slice())
    }

    // GSUB or GPOS where shaping reads it, parsed; none where the sanitizer rejects it.
    pub(crate) fn layout(&self, table: super::plan::Table) -> Option<LayoutTable<'a>> {
        let which = match table {
            super::plan::Table::Gsub => OtlTable::Gsub,
            super::plan::Table::Gpos => OtlTable::Gpos,
        };
        self.cache.layout_table(which).and_then(|t| LayoutTable::parse(t.as_slice()))
    }
}
