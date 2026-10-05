use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetrics {
    pub ascent: f64,
    pub descent: f64,
    pub line_gap: f64,
}

impl LineMetrics {
    pub fn line_height(&self) -> f64 {
        self.ascent - self.descent + self.line_gap
    }
}

impl FontCache {
    pub fn font_ascender(&self) -> i32 {
        let scale = self.scale_factor();
        if let Some(hhea) = self.table_map.get("hhea")
            && hhea.len() >= 6
                && let Some(v) = read_i16_be(hhea, 4) {
                    return (v as f64 * scale).round() as i32;
                }
        0
    }

    pub fn font_descender(&self) -> i32 {
        let scale = self.scale_factor();
        if let Some(hhea) = self.table_map.get("hhea")
            && hhea.len() >= 8
                && let Some(v) = read_i16_be(hhea, 6) {
                    return (v as f64 * scale).round() as i32;
                }
        0
    }

    pub fn font_cap_height(&self) -> i32 {
        let scale = self.scale_factor();
        let cap = self.os2()
            .and_then(|o| o.s_cap_height)
            .unwrap_or(0);
        if cap != 0 {
            return (cap as f64 * scale).round() as i32;
        }
        self.font_ascender()
    }

    pub fn font_bbox(&self) -> Vec<i32> {
        let scale = self.scale_factor();
        if let Some(head) = self.table_map.get("head")
            && head.len() >= 44 {
                let (Some(a), Some(b), Some(c), Some(d)) = (
                    read_i16_be(head, 36), read_i16_be(head, 38),
                    read_i16_be(head, 40), read_i16_be(head, 42),
                ) else { return vec![0, 0, 0, 0] };
                return vec![
                    (a as f64 * scale).round() as i32,
                    (b as f64 * scale).round() as i32,
                    (c as f64 * scale).round() as i32,
                    (d as f64 * scale).round() as i32,
                ];
            }
        vec![0, 0, 0, 0]
    }

    pub fn font_flags(&self) -> u32 {
        let mut flags: u32 = 1 << 5;

        let is_fixed_pitch = self.table_map.get("post")
            .filter(|p| p.len() >= 16)
            .and_then(|p| read_u32_be(p, 12))
            .is_some_and(|v| v != 0);
        if is_fixed_pitch { flags |= 1; }

        let family_class = self.os2()
            .and_then(|o| o.s_family_class)
            .unwrap_or(0);
        let class_id = (family_class >> 8) as u8;
        let is_serif  = matches!(class_id, 1 | 2 | 3 | 4 | 5 | 7);
        let is_script = class_id == 10;
        if is_serif  { flags |= 1 << 1; }
        if is_script { flags |= 1 << 3; }

        if self.font_italic_angle() != 0.0 { flags |= 1 << 6; }

        flags
    }

    pub fn font_italic_angle(&self) -> f64 {
        let post = match self.table_map.get("post") {
            Some(p) if p.len() >= 16 => p,
            _ => return 0.0,
        };
        let raw = read_u32_be(post, 4).unwrap_or(0) as i32;
        raw as f64 / 65536.0
    }

    pub fn font_num_glyphs(&self) -> Option<u16> {
        self.num_glyphs
    }

    pub fn cff(&self) -> Option<&crate::daecore::daetype::TableBytes> {
        self.cff.as_ref()
    }

    pub fn os2(&self) -> Option<crate::daecore::daetype::decoder::Os2Fields> {
        self.os2
    }

    pub fn glyph_in_range(&self, gid: u16) -> Option<u16> {
        match self.font_num_glyphs() {
            Some(n) => (gid < n).then_some(gid),
            None => Some(gid),
        }
    }

    pub fn font_upm(&self) -> u16 {
        self.upm
    }

    pub fn line_metrics(&self, vertical: bool) -> LineMetrics {
        let scale = self.scale_factor();
        let upm = self.font_upm() as f64;

        if vertical {
            let vhea = self.table_map.get("vhea").filter(|v| v.len() >= 10);
            let raw = vhea.and_then(|v| {
                Some((read_i16_be(v, 4)? as f64, read_i16_be(v, 6)? as f64, read_i16_be(v, 8)? as f64))
            });
            let (a, d, g) = raw.filter(|&(a, d, _)| a - d > 0.0).unwrap_or((upm / 2.0, -upm / 2.0, 0.0));
            return LineMetrics { ascent: a * scale, descent: d * scale, line_gap: g * scale };
        }

        let os2 = self.os2();
        let typo = os2.and_then(|o| o.line_metrics).filter(|m| m.ascender as i32 - m.descender as i32 > 0);
        let hhea = self
            .table_map
            .get("hhea")
            .filter(|h| h.len() >= 10)
            .and_then(|h| Some((read_i16_be(h, 4)?, read_i16_be(h, 6)?, read_i16_be(h, 8)?)))
            .filter(|&(a, d, _)| a as i32 - d as i32 > 0);

        let (a, d, g) = if os2.is_some_and(|o| o.use_typo_metrics()) {
            typo.map(|m| (m.ascender as f64, m.descender as f64, m.line_gap as f64))
        } else {
            None
        }
        .or_else(|| hhea.map(|(a, d, g)| (a as f64, d as f64, g as f64)))
        .or_else(|| typo.map(|m| (m.ascender as f64, m.descender as f64, m.line_gap as f64)))
        .or_else(|| {
            os2.and_then(|o| o.win_metrics)
                .filter(|w| w.ascent as u32 + w.descent as u32 > 0)
                .map(|w| (w.ascent as f64, -(w.descent as f64), 0.0))
        })
        .unwrap_or((upm * 0.8, -upm * 0.2, 0.0));

        LineMetrics { ascent: a * scale, descent: d * scale, line_gap: g * scale }
    }

    // Unrounded, as every other MATH value is: the font's, else half the glyph's advance.
    pub fn math_top_accent_attachment(&self, gid: u16) -> f64 {
        let scale = self.scale_factor();
        if let Some(v) = crate::daecore::daetype::math_table::math_top_accent_attachment(&self.table_map, gid) {
            return f64::from(v) * scale;
        }
        let advance = self.advance_font_units_rs(&super::canonical_axes::<&str>(&[]), gid, false);
        f64::from(advance) * scale / 2.0
    }

    pub fn font_vertical_origin_rs(&self, axis_values: &[(String, f64)], gid: u16) -> Option<i32> {
        let key = canonical_axes(axis_values);
        if key.is_empty() {
            return self.font_vertical_origin(gid);
        }
        self.instanced_font_cache_keyed(&key).font_vertical_origin(gid)
    }

    pub fn font_vertical_origin(&self, gid: u16) -> Option<i32> {
        let scale = self.scale_factor();

        if self.table_map.contains_key("glyf") {
            let tsb  = self.vmtx_top_side_bearing(gid)?;
            let ymax = self.glyf_y_max(gid)?;
            return Some(((tsb as i32 + ymax as i32) as f64 * scale).round() as i32);
        }

        crate::daecore::daetype::vorg::vorg_origin_y(&self.table_map, gid)
            .map(|raw| (raw as f64 * scale).round() as i32)
    }

    fn vmtx_top_side_bearing(&self, gid: u16) -> Option<i16> {
        let vhea = self.table_map.get("vhea")?;
        if vhea.len() < 36 { return None; }
        let num_vm = read_u16_be(vhea, 34)? as usize;
        if num_vm == 0 { return None; }
        let vmtx = self.table_map.get("vmtx")?;
        Some(crate::daecore::daetype::subsetter::metric_pair(vmtx, num_vm, 0, gid as usize).1)
    }

    pub fn loca_offsets(&self) -> Option<Shared<Vec<usize>>> {
        {
            let cache = read(&self.loca_offsets_cache);
            if let Some(offsets) = cache.as_ref() { return Some(Shared::clone(offsets)); }
        }
        let head = self.table_map.get("head")?;
        if head.len() < 52 { return None; }
        let loca_format = read_i16_be(head, 50)?;
        let maxp = self.table_map.get("maxp")?;
        let num_glyphs = read_u16_be(maxp, 4)? as usize;
        let loca = self.table_map.get("loca")?;
        let offsets = Shared::new(crate::daecore::daetype::subsetter::parse_loca(loca, loca_format, num_glyphs));
        *write(&self.loca_offsets_cache) = Some(Shared::clone(&offsets));
        Some(offsets)
    }

    pub fn cff_outlines(&self) -> Option<Shared<crate::daecore::daetype::outline::CffOutlines>> {
        {
            let cache = read(&self.cff_outlines_cache);
            if let Some(slot) = cache.as_ref() { return slot.as_ref().map(Shared::clone); }
        }
        let cff = self.table_map.get("CFF ")?;
        let parsed = crate::daecore::daetype::outline::CffOutlines::parse(cff)
            .ok()
            .map(Shared::new);
        *write(&self.cff_outlines_cache) = Some(parsed.as_ref().map(Shared::clone));
        parsed
    }

    pub(crate) fn gdef_var_store(&self, off: usize)
        -> Option<Shared<crate::daecore::daetype::format::ivs::ItemVariationStore>>
    {
        {
            let cache = read(&self.gdef_var_store);
            if let Some(v) = cache.as_ref() { return Some(Shared::clone(v)); }
        }
        let gdef = self.table_map.get("GDEF")?;
        let parsed = Shared::new(crate::daecore::daetype::format::ivs::parse_item_variation_store(gdef, off).ok()?);
        *write(&self.gdef_var_store) = Some(Shared::clone(&parsed));
        Some(parsed)
    }

    fn glyf_y_max(&self, gid: u16) -> Option<i16> {
        let glyf = self.table_map.get("glyf")?;

        let offsets = self.loca_offsets()?;
        let gid = gid as usize;
        if gid + 1 >= offsets.len() { return None; }
        let (start, end) = (offsets[gid], offsets[gid + 1]);
        if start >= end { return Some(0); }
        read_i16_be(glyf, start + 8)
    }

    pub fn scale_factor(&self) -> f64 {
        1000.0 / self.font_upm() as f64
    }

    pub fn advance_width_rs(&self, axis_values: &[(String, f64)], gid: u16) -> u32 {
        let axes = canonical_axes(axis_values);
        let key = (axes.clone(), gid);
        {
            let cache = read(&self.adv_cache);
            if let Some(&cached) = cache.get(&key) {
                return cached;
            }
        }
        let advances = self.advances_for_axis(&axes, false);
        let adv = advances.get(gid as usize).copied().unwrap_or(0);
        let mut cache = write(&self.adv_cache);
        if cache.len() >= ADVANCE_CACHE_CAP { cache.clear(); }
        cache.insert(key, adv);
        adv
    }

    pub fn vertical_advance_rs(&self, axis_values: &[(String, f64)], gid: u16) -> u32 {
        self.advance_keyed(&canonical_axes(axis_values), gid, true)
    }

    pub fn advance_keyed(&self, axes: &AxisKey, gid: u16, vertical: bool) -> u32 {
        self.advances_for_axis(axes, vertical).get(gid as usize).copied().unwrap_or(0)
    }

    pub(crate) fn retained_bytes(&self) -> usize {
        self.table_map.values().map(|t| t.len()).sum()
    }

    pub(crate) fn advance_font_units_rs(&self, axes: &AxisKey, gid: u16, vertical: bool) -> u32 {
        let advances = self.advances_for_axis_inner(axes, vertical, false);
        advances.get(gid as usize).copied().unwrap_or(0)
    }

    pub(crate) fn advances_font_units_table(
        &self,
        axes: &AxisKey,
        vertical: bool,
    ) -> Shared<Vec<u32>> {
        self.advances_for_axis_inner(axes, vertical, false)
    }

    fn advances_for_axis(&self, axes: &AxisKey, vertical: bool) -> Shared<Vec<u32>> {
        self.advances_for_axis_inner(axes, vertical, true)
    }

    fn advances_for_axis_inner(&self, axes: &AxisKey, vertical: bool, normalize: bool) -> Shared<Vec<u32>> {
        let cache_ref = match (vertical, normalize) {
            (false, true) => &self.advances_by_axis,
            (true, true) => &self.vertical_advances_by_axis,
            (false, false) => &self.advances_by_axis_fu,
            (true, false) => &self.vertical_advances_by_axis_fu,
        };
        {
            let cache = read(cache_ref);
            if let Some(v) = cache.get(axes) { return Shared::clone(v); }
        }
        let instanced = self.instanced_font_cache(axes);
        let (mtx, hea) = if vertical { ("vmtx", "vhea") } else { ("hmtx", "hhea") };
        let advances = Shared::new(crate::daecore::daetype::subsetter::map_advances_all(
            &instanced.table_map,
            mtx,
            hea,
            normalize,
        ));
        let mut cache = write(cache_ref);
        if cache.len() >= ADVANCES_BY_AXIS_CAP { cache.clear(); }
        cache.insert(axes.clone(), Shared::clone(&advances));
        advances
    }
}
