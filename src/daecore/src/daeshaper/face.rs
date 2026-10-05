#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;
use alloc::vec::Vec;

use crate::daecore::daetype::format::ivs::{compute_ivs_delta_f64, precompute_region_scalars, ItemVariationStore};
use crate::daecore::cache::FontCache;
use super::ot::{glyph_props_from_gdef_class, Gdef};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GlyphExtents {
    pub(crate) x_bearing: i32,
    pub(crate) y_bearing: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

#[derive(Default)]
struct BboxPen {
    min_x: f32,
    min_y: f32,
    max_x: f32,
    max_y: f32,
    any: bool,
}

impl BboxPen {
    fn add(&mut self, x: f32, y: f32) {
        if !self.any {
            self.any = true;
            self.min_x = x;
            self.max_x = x;
            self.min_y = y;
            self.max_y = y;
            return;
        }
        self.min_x = self.min_x.min(x);
        self.max_x = self.max_x.max(x);
        self.min_y = self.min_y.min(y);
        self.max_y = self.max_y.max(y);
    }
}

impl crate::daecore::daetype::outline::OutlinePen for BboxPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.add(cx, cy);
        self.add(x, y);
    }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.add(c1x, c1y);
        self.add(c2x, c2y);
        self.add(x, y);
    }
    fn close(&mut self) {}
}

struct VarDeltas {
    store: crate::daecore::sync::Shared<ItemVariationStore>,
    scalars: Vec<f64>,
}

pub struct Face<'a> {
    cache: &'a FontCache,
    axes: crate::daecore::cache::AxisKey,
    gdef: Option<Gdef<'a>>,
    gdef_class_index: Option<crate::daecore::sync::Shared<crate::daecore::daetype::format::index::SparseIndex>>,
    gdef_mark_attach_index: Option<crate::daecore::sync::Shared<crate::daecore::daetype::format::index::SparseIndex>>,
    var_deltas: Option<VarDeltas>,
    cmap_index: Option<crate::daecore::sync::Shared<crate::daecore::daetype::format::index::SparseIndex>>,
    upm: u16,
    point_size: Option<f64>,
}

impl<'a> Face<'a> {
    pub fn new(cache: &'a FontCache, axes: &[(&str, f64)]) -> Self {
        cache.warm_cmap_index();
        let gdef = cache.table_map.get("GDEF").and_then(|d| Gdef::new(d));
        let axes = crate::daecore::cache::canonical_axes(axes);
        let var_deltas = gdef.as_ref().filter(|g| g.item_var_store_off != 0).and_then(|g| {
            let store = cache.gdef_var_store(g.item_var_store_off)?;
            let location = cache.compute_location_rs(&axes);
            let scalars = precompute_region_scalars(&store, &location);
            Some(VarDeltas { store, scalars })
        });
        let (gdef_class_index, gdef_mark_attach_index) = cache.gdef_class_indexes(|| {
            let g = gdef.as_ref();
            (
                g.and_then(|g| g.glyph_classes.as_ref()).and_then(|c| c.index_entries()),
                g.and_then(|g| g.mark_attach_classes.as_ref()).and_then(|c| c.index_entries()),
            )
        });

        Face {
            cache,
            axes,
            point_size: None,
            gdef,
            gdef_class_index,
            gdef_mark_attach_index,
            var_deltas,
            cmap_index: cache.cmap_index(),
            upm: cache.font_upm(),
        }
    }

    pub(crate) fn with_point_size(mut self, ptem: Option<f64>) -> Self {
        self.point_size = ptem;
        self
    }

    pub(crate) fn tracking_point_size(&self) -> f64 {
        match self.point_size {
            Some(p) if p > 0.0 => p,
            _ => crate::daecore::daetype::trak::DEFAULT_POINT_SIZE,
        }
    }

    pub(crate) fn tracking(&self, horizontal: bool) -> i32 {
        let adjust =
            crate::daecore::daetype::trak::tracking(&self.cache.table_map, self.tracking_point_size(), horizontal);
        // `ot_round`, not `f64::round`: this is a positional IVS delta, which the spec rounds the
        // same way `format/round.rs` rounds the rest of that family.
        crate::daecore::daetype::format::round::ot_round(adjust)
    }

    pub(crate) fn uses_device_tables(&self) -> bool {
        !self.axes.is_empty()
    }

    pub(crate) fn variation_delta(&self, outer: u16, inner: u16) -> i32 {
        let Some(v) = self.var_deltas.as_ref() else { return 0 };
        crate::daecore::daetype::format::round::ot_round(
            compute_ivs_delta_f64(&v.store, outer as usize, inner as usize, &v.scalars),
        )
    }

    pub(crate) fn units_per_em(&self) -> u16 {
        self.upm
    }

    pub(crate) fn num_glyphs(&self) -> u16 {
        self.table("maxp")
            .and_then(|maxp| crate::daecore::daetype::decoder::read_u16_be(maxp, 4))
            .unwrap_or(u16::MAX)
    }

    pub(crate) fn glyph_index(&self, codepoint: u32) -> Option<u16> {
        if let Some(index) = &self.cmap_index {
            return Some(index.lookup(codepoint)).filter(|&g| g != 0);
        }
        self.cache.glyph_id(codepoint)
    }

    pub(crate) fn has_glyph(&self, codepoint: u32) -> bool {
        self.glyph_index(codepoint).is_some()
    }

    pub(crate) fn glyph_variation_index(&self, base: u32, selector: u32) -> Option<u16> {
        self.cache.variation_glyph_id(base, selector)
    }

    pub(crate) fn glyph_h_advance(&self, glyph: u16) -> i32 {
        self.cache.advance_font_units_rs(&self.axes, glyph, false) as i32
    }

    pub(crate) fn advances_table(&self, vertical: bool) -> crate::daecore::sync::Shared<Vec<u32>> {
        self.cache.advances_font_units_table(&self.axes, vertical)
    }

    pub(crate) fn v_advance_or_line_height(&self, raw: u32) -> i32 {
        let advance = raw as i32;
        if advance != 0 {
            return advance;
        }
        self.line_height()
    }

    pub(crate) fn glyph_v_advance(&self, glyph: u16) -> i32 {
        let advance = self.cache.advance_font_units_rs(&self.axes, glyph, true) as i32;
        if advance != 0 {
            return advance;
        }
        self.line_height()
    }

    fn line_height(&self) -> i32 {
        let Some(hhea) = self.cache.table_map.get("hhea").filter(|t| t.len() >= 8) else {
            return i32::from(self.upm);
        };
        let ascender = crate::daecore::daetype::decoder::read_i16_be(hhea, 4).unwrap_or(0) as i32;
        let descender = crate::daecore::daetype::decoder::read_i16_be(hhea, 6).unwrap_or(0) as i32;
        let height = ascender - descender;
        if height > 0 { height } else { i32::from(self.upm) }
    }

    pub(crate) fn glyph_h_origin(&self, glyph: u16) -> i32 {
        self.glyph_h_advance(glyph) / 2
    }

    pub(crate) fn glyph_v_origin(&self, glyph: u16) -> i32 {
        if let Some(y) = crate::daecore::daetype::vorg::vorg_origin_y(&self.cache.table_map, glyph) {
            return i32::from(y);
        }
        let Some(extents) = self.glyph_extents(glyph) else {
            return self.ascender();
        };
        if self.has_table("vmtx") {
            extents.y_bearing + self.glyph_v_side_bearing(glyph)
        } else {
            let line = self.ascender().saturating_sub(self.descender());
            extents
                .y_bearing
                .saturating_add(line.saturating_sub(extents.height.saturating_neg()) >> 1)
        }
    }

    fn glyph_v_side_bearing(&self, glyph: u16) -> i32 {
        let tables = &self.cache.table_map;
        let Some(vmtx) = tables.get("vmtx") else { return 0 };
        let long = tables
            .get("vhea")
            .filter(|t| t.len() >= 36)
            .and_then(|t| crate::daecore::daetype::decoder::read_u16_be(t, 34))
            .unwrap_or(0) as usize;
        let glyph = glyph as usize;
        let at = if glyph < long {
            glyph * 4 + 2
        } else {
            long * 4 + (glyph - long) * 2
        };
        crate::daecore::daetype::decoder::read_i16_be(vmtx, at).map_or(0, i32::from)
    }

    fn ascender(&self) -> i32 {
        self.typo_metric(68, 4).unwrap_or_else(|| i32::from(self.upm) * 4 / 5)
    }

    fn descender(&self) -> i32 {
        self.typo_metric(70, 6).unwrap_or_else(|| -i32::from(self.upm) / 5)
    }

    fn typo_metric(&self, os2_off: usize, hhea_off: usize) -> Option<i32> {
        let tables = &self.cache.table_map;
        let os2 = tables.get("OS/2").filter(|t| t.len() >= 72);
        let os2_value = || os2.and_then(|t| crate::daecore::daetype::decoder::read_i16_be(t, os2_off)).map(i32::from);

        let use_typo = crate::daecore::daetype::decoder::parse_os2(tables).is_some_and(|o| o.use_typo_metrics());
        if use_typo
            && let Some(v) = os2_value() {
                return Some(v);
            }
        match tables
            .get("hhea")
            .filter(|t| t.len() >= 8)
            .and_then(|t| crate::daecore::daetype::decoder::read_i16_be(t, hhea_off))
        {
            Some(0) | None => os2_value().filter(|&v| v != 0),
            Some(v) => Some(i32::from(v)),
        }
    }

    pub(crate) fn glyph_extents(&self, glyph: u16) -> Option<GlyphExtents> {
        use crate::daecore::daetype::outline::OutlinePen;
        let mut pen = BboxPen::default();
        let tables = &self.cache.table_map;

        if let Some(stored) = self.glyf_stored_bbox(glyph) {
            return Some(stored);
        }

        let drawn = if let Some(cff) = tables.get("CFF ") {
            match self.cache.cff_outlines() {
                Some(o) => crate::daecore::daetype::outline::outline_cff_glyph_with(
                    &o, cff, glyph, &mut pen as &mut dyn OutlinePen,
                ).is_ok(),
                None => false,
            }
        } else if tables.contains_key("glyf") {
            match self.cache.loca_offsets() {
                Some(loca) => self.cache.draw_glyf_reusing(&loca, glyph, &mut pen as &mut dyn OutlinePen).is_ok(),
                None => false,
            }
        } else {
            false
        };

        if !drawn {
            return None;
        }
        if !pen.any {
            return Some(GlyphExtents { x_bearing: 0, y_bearing: 0, width: 0, height: 0 });
        }

        Some(GlyphExtents {
            x_bearing: pen.min_x.round() as i32,
            y_bearing: pen.max_y.round() as i32,
            width: (pen.max_x - pen.min_x).round() as i32,
            height: (pen.min_y - pen.max_y).round() as i32,
        })
    }

    fn glyf_stored_bbox(&self, glyph: u16) -> Option<GlyphExtents> {
        use crate::daecore::daetype::decoder::read_i16_be;

        let glyf = self.cache.table_map.get("glyf")?;
        let loca = self.cache.loca_offsets()?;
        let start = *loca.get(usize::from(glyph))?;
        let end = *loca.get(usize::from(glyph) + 1)?;
        if end <= start || end > glyf.len() || end - start < 10 {
            return None;
        }

        let x_min = read_i16_be(glyf, start + 2)?;
        let y_min = read_i16_be(glyf, start + 4)?;
        let x_max = read_i16_be(glyf, start + 6)?;
        let y_max = read_i16_be(glyf, start + 8)?;

        Some(GlyphExtents {
            x_bearing: i32::from(x_min) + self.cache.glyf_origin_shift(&loca, glyph),
            y_bearing: i32::from(y_max),
            width: i32::from(x_max) - i32::from(x_min),
            height: i32::from(y_min) - i32::from(y_max),
        })
    }

    pub(crate) fn has_glyph_classes(&self) -> bool {
        self.gdef.as_ref().is_some_and(Gdef::has_glyph_classes)
    }

    pub(crate) fn glyph_props(&self, glyph: u16) -> u16 {
        let Some(gdef) = self.gdef.as_ref() else { return 0 };
        let class = gdef.glyph_classes.as_ref().map_or(0, |c| {
            c.indexed(self.gdef_class_index.as_deref()).class_of(glyph)
        });
        let mark_attach = gdef.mark_attach_classes.as_ref().map_or(0, |c| {
            c.indexed(self.gdef_mark_attach_index.as_deref()).class_of(glyph)
        });
        glyph_props_from_gdef_class(class, mark_attach)
    }

    pub(crate) fn is_mark_glyph(&self, glyph: u16, set_index: u16) -> bool {
        self.gdef.as_ref().is_some_and(|g| g.is_mark_glyph(glyph, set_index))
    }

    pub(crate) fn lookup_digest(
        &self,
        table_index: usize,
        lookup_count: usize,
        index: u16,
        compute: impl FnOnce() -> super::ot::digest::Digest,
    ) -> super::ot::digest::Digest {
        self.cache.lookup_digest_cached(table_index, lookup_count, index, compute)
    }

    pub(crate) fn build_index(
        &self,
        entries: &[(u32, u16)],
        absent: u16,
    ) -> Option<crate::daecore::daetype::format::index::SparseIndex> {
        self.cache.build_index(entries, absent)
    }

    pub(crate) fn subtable_indexes(
        &self,
        table_index: usize,
        lookup_count: usize,
        index: u16,
        build: impl FnOnce() -> alloc::vec::Vec<super::ot::SubtableIndex>,
    ) -> Option<crate::daecore::sync::Shared<alloc::vec::Vec<super::ot::SubtableIndex>>> {
        self.cache.subtable_indexes_cached(table_index, lookup_count, index, build)
    }

    pub(crate) fn has_table(&self, tag: &str) -> bool {
        self.cache.table_map.contains_key(tag)
    }

    pub(crate) fn table(&self, tag: &str) -> Option<&'a [u8]> {
        self.cache.table_map.get(tag).map(|t| t.as_slice())
    }
}
