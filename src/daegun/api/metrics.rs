use super::*;

impl Font {
    pub fn family_name(&self) -> Option<String> {
        crate::daecore::daetype::decoder::read_font_family_name(&self.cache.table_map)
    }

    pub fn style(&self) -> &'static str {
        crate::daecore::daetype::decoder::read_font_style(&self.cache.table_map)
    }

    pub fn is_variable(&self) -> bool {
        crate::daecore::daetype::decoder::is_variable_font(&self.cache.table_map)
    }

    pub fn named_instances(&self) -> Vec<NamedInstance> {
        crate::daecore::daetype::decoder::read_fvar_instances(&self.cache.table_map).unwrap_or_default()
    }

    // Static to the instancer: an fvar it cannot read. One that reads, axes or none, is instanced.
    pub(crate) fn is_static(&self) -> bool {
        crate::daecore::daetype::decoder::parse_fvar_axes(&self.cache.table_map).is_err()
    }

    // As the advance reader has it: a vmtx with no vhea to count its entries gives every height as 0.
    pub(crate) fn has_vertical_metrics(&self) -> bool {
        crate::daecore::daetype::subsetter::metrics_of(&self.cache.table_map, "vmtx", "vhea").is_some()
    }

    pub fn axes(&self) -> Vec<FvarAxis> {
        crate::daecore::daetype::decoder::parse_fvar_axes(&self.cache.table_map).unwrap_or_default()
    }

    pub fn names(&self) -> alloc::collections::BTreeMap<u16, String> {
        crate::daecore::daetype::decoder::parse_all_name_strings(&self.cache.table_map)
    }

    pub fn name_string(&self, name_id: u16) -> Option<String> {
        crate::daecore::daetype::decoder::read_name_string(&self.cache.table_map, name_id)
    }

    pub fn upm(&self) -> u16 {
        self.cache.font_upm()
    }

    pub fn num_glyphs(&self) -> u16 {
        self.cache.font_num_glyphs().unwrap_or(0)
    }

    // A static font is its own instance, as in `instance_tables`.
    pub fn instance(&self, axes: &[(&str, f64)]) -> Vec<u8> {
        if self.is_static() {
            return crate::daecore::daetype::decoder::build_ttf(&self.cache.table_map);
        }
        (*self.cache.get_or_instance(&owned_axes(axes))).clone()
    }

    pub fn ascender(&self) -> i32 {
        self.cache.font_ascender()
    }

    pub fn descender(&self) -> i32 {
        self.cache.font_descender()
    }

    pub fn cap_height(&self) -> i32 {
        self.cache.font_cap_height()
    }

    pub fn bbox(&self) -> Vec<i32> {
        self.cache.font_bbox()
    }

    pub fn line_metrics(&self, vertical: bool) -> LineMetrics {
        self.cache.line_metrics(vertical)
    }

    pub fn flags(&self) -> u32 {
        self.cache.font_flags()
    }

    pub fn italic_angle(&self) -> f64 {
        self.cache.font_italic_angle()
    }

    pub fn os2_info(&self) -> Option<Os2Info> {
        let os2 = self.cache.os2()?;
        let scale = self.cache.scale_factor();
        Some(Os2Info {
            version: os2.version,
            family_class: os2.s_family_class,
            selection: os2.fs_selection,
            win_metrics: os2.win_metrics.map(|w| WinMetrics {
                ascent: (f64::from(w.ascent) * scale).round() as i32,
                descent: (f64::from(w.descent) * scale).round() as i32,
            }),
            typo_metrics: os2.line_metrics.map(|m| TypoLineMetrics {
                ascender: (f64::from(m.ascender) * scale).round() as i32,
                descender: (f64::from(m.descender) * scale).round() as i32,
                line_gap: (f64::from(m.line_gap) * scale).round() as i32,
            }),
        })
    }

    pub fn tracking(&self, ptem: f64, horizontal: bool) -> f64 {
        crate::daecore::daetype::trak::tracking(&self.cache.table_map, ptem, horizontal)
            * self.cache.scale_factor()
    }

    pub fn normalized_axes(&self, axes: &[(&str, f64)]) -> Vec<f64> {
        crate::daecore::daetype::instancer::compute_location(&self.cache.table_map, &owned_axes(axes))
            .unwrap_or_default()
    }

    pub fn typographic_metrics(&self, axes: &[(&str, f64)]) -> Option<TypographicMetrics> {
        let scale = self.cache.scale_factor();
        let s = |v: i16| (v as f64 * scale).round() as i32;

        let location = self.normalized_axes(axes);
        let deltas = if location.is_empty() {
            alloc::collections::BTreeMap::new()
        } else {
            crate::daecore::daetype::instancer::mvar_deltas(&self.cache.table_map, &location)
                .unwrap_or_default()
        };
        let d = |tag: &[u8; 4]| deltas.get(tag).copied().unwrap_or(0);
        let varied = |v: i16, tag: &[u8; 4]| s((i32::from(v) + d(tag)).clamp(-32768, 32767) as i16);
        let quad = |a: [i16; 4], tags: [&[u8; 4]; 4]| SubSuperMetrics {
            x_size:   varied(a[0], tags[0]),
            y_size:   varied(a[1], tags[1]),
            x_offset: varied(a[2], tags[2]),
            y_offset: varied(a[3], tags[3]),
        };

        let os2 = self.cache.os2();
        let block = os2.as_ref().and_then(|o| o.metrics);
        let x_height = os2.as_ref().and_then(|o| o.sx_height);
        let underline = self.cache.table_map.get("post")
            .filter(|p| p.len() >= 12)
            .and_then(|p| Some((
                crate::daecore::daetype::decoder::read_i16_be(p, 8)?,
                crate::daecore::daetype::decoder::read_i16_be(p, 10)?,
            )));

        if block.is_none() && x_height.is_none() && underline.is_none() {
            return None;
        }
        let (underline_position, underline_thickness) =
            underline.map_or((0, 0), |(p, t)| (varied(p, b"undo"), varied(t, b"unds")));

        Some(TypographicMetrics {
            x_height: x_height.map_or(0, |v| varied(v, b"xhgt")),
            underline_position,
            underline_thickness,
            strikeout_size:     block.map_or(0, |m| varied(m.y_strikeout_size, b"strs")),
            strikeout_position: block.map_or(0, |m| varied(m.y_strikeout_position, b"stro")),
            subscript: block.map_or_else(SubSuperMetrics::default, |m| {
                quad(m.subscript, [b"sbxs", b"sbys", b"sbxo", b"sbyo"])
            }),
            superscript: block.map_or_else(SubSuperMetrics::default, |m| {
                quad(m.superscript, [b"spxs", b"spys", b"spxo", b"spyo"])
            }),
        })
    }
}
