use super::*;

impl Font {
    pub fn subset(&self, gids: &[u16], axes: &[(&str, f64)]) -> Result<SubsetResult, FontError> {
        self.cache.subset_font_rs(&owned_axes(axes), gids).map_err(FontError)
    }

    pub fn subset_text(&self, text: &str, axes: &[(&str, f64)]) -> Result<SubsetResult, FontError> {
        self.cache.subset_text_rs(&owned_axes(axes), text).map_err(FontError)
    }

    pub fn glyph_closure(&self, gids: &[u16], axes: &[(&str, f64)]) -> Result<Vec<u16>, FontError> {
        self.cache.glyph_closure_rs(&owned_axes(axes), gids).map_err(FontError)
    }

    pub fn math_constants(&self) -> Option<MathConstants> {
        let c = crate::daecore::daetype::math_table::parse_math_constants(&self.cache.table_map)?;
        let scale = self.cache.scale_factor();
        let s = |v: i16| v as f64 * scale;
        Some(MathConstants {
            script_percent_scale_down: c.script_percent_scale_down as f64,
            script_script_percent_scale_down: c.script_script_percent_scale_down as f64,
            delimited_sub_formula_min_height: f64::from(c.delimited_sub_formula_min_height) * scale,
            display_operator_min_height: f64::from(c.display_operator_min_height) * scale,
            math_leading: s(c.math_leading),
            axis_height: s(c.axis_height),
            accent_base_height: s(c.accent_base_height),
            flattened_accent_base_height: s(c.flattened_accent_base_height),
            subscript_shift_down: s(c.subscript_shift_down),
            subscript_top_max: s(c.subscript_top_max),
            subscript_baseline_drop_min: s(c.subscript_baseline_drop_min),
            superscript_shift_up: s(c.superscript_shift_up),
            superscript_shift_up_cramped: s(c.superscript_shift_up_cramped),
            superscript_bottom_min: s(c.superscript_bottom_min),
            superscript_baseline_drop_max: s(c.superscript_baseline_drop_max),
            sub_superscript_gap_min: s(c.sub_superscript_gap_min),
            superscript_bottom_max_with_subscript: s(c.superscript_bottom_max_with_subscript),
            space_after_script: s(c.space_after_script),
            upper_limit_gap_min: s(c.upper_limit_gap_min),
            upper_limit_baseline_rise_min: s(c.upper_limit_baseline_rise_min),
            lower_limit_gap_min: s(c.lower_limit_gap_min),
            lower_limit_baseline_drop_min: s(c.lower_limit_baseline_drop_min),
            stack_top_shift_up: s(c.stack_top_shift_up),
            stack_top_display_style_shift_up: s(c.stack_top_display_style_shift_up),
            stack_bottom_shift_down: s(c.stack_bottom_shift_down),
            stack_bottom_display_style_shift_down: s(c.stack_bottom_display_style_shift_down),
            stack_gap_min: s(c.stack_gap_min),
            stack_display_style_gap_min: s(c.stack_display_style_gap_min),
            stretch_stack_top_shift_up: s(c.stretch_stack_top_shift_up),
            stretch_stack_bottom_shift_down: s(c.stretch_stack_bottom_shift_down),
            stretch_stack_gap_above_min: s(c.stretch_stack_gap_above_min),
            stretch_stack_gap_below_min: s(c.stretch_stack_gap_below_min),
            fraction_numerator_shift_up: s(c.fraction_numerator_shift_up),
            fraction_numerator_display_style_shift_up: s(c.fraction_numerator_display_style_shift_up),
            fraction_denominator_shift_down: s(c.fraction_denominator_shift_down),
            fraction_denominator_display_style_shift_down: s(c.fraction_denominator_display_style_shift_down),
            fraction_numerator_gap_min: s(c.fraction_numerator_gap_min),
            fraction_num_display_style_gap_min: s(c.fraction_num_display_style_gap_min),
            fraction_rule_thickness: s(c.fraction_rule_thickness),
            fraction_denominator_gap_min: s(c.fraction_denominator_gap_min),
            fraction_denom_display_style_gap_min: s(c.fraction_denom_display_style_gap_min),
            skewed_fraction_horizontal_gap: s(c.skewed_fraction_horizontal_gap),
            skewed_fraction_vertical_gap: s(c.skewed_fraction_vertical_gap),
            overbar_vertical_gap: s(c.overbar_vertical_gap),
            overbar_rule_thickness: s(c.overbar_rule_thickness),
            overbar_extra_ascender: s(c.overbar_extra_ascender),
            underbar_vertical_gap: s(c.underbar_vertical_gap),
            underbar_rule_thickness: s(c.underbar_rule_thickness),
            underbar_extra_descender: s(c.underbar_extra_descender),
            radical_vertical_gap: s(c.radical_vertical_gap),
            radical_display_style_vertical_gap: s(c.radical_display_style_vertical_gap),
            radical_rule_thickness: s(c.radical_rule_thickness),
            radical_extra_ascender: s(c.radical_extra_ascender),
            radical_kern_before_degree: s(c.radical_kern_before_degree),
            radical_kern_after_degree: s(c.radical_kern_after_degree),
            radical_degree_bottom_raise_percent: c.radical_degree_bottom_raise_percent as f64,
        })
    }

    pub fn math_italics_correction(&self, gid: u16) -> Option<f64> {
        crate::daecore::daetype::math_table::math_italics_correction(&self.cache.table_map, gid)
            .map(|v| v as f64 * self.cache.scale_factor())
    }

    pub fn math_top_accent_attachment(&self, gid: u16) -> f64 {
        self.cache.math_top_accent_attachment(gid)
    }

    pub fn math_is_extended_shape(&self, gid: u16) -> bool {
        crate::daecore::daetype::math_table::math_is_extended_shape(&self.cache.table_map, gid)
    }

    // The kern at a height: None for one that is not a number.
    pub fn math_kern(&self, gid: u16, corner: MathKernCorner, height: f64) -> Option<f64> {
        if !height.is_finite() { return None; }
        let scale = self.cache.scale_factor();
        let kern = crate::daecore::daetype::math_table::math_kern(&self.cache.table_map, gid, corner, height / scale);
        Some(f64::from(kern) * scale)
    }

    pub fn math_glyph_variants(&self, gid: u16, vertical: bool) -> Option<MathGlyphConstruction> {
        let c = crate::daecore::daetype::math_table::math_glyph_construction(&self.cache.table_map, gid, vertical)?;
        let scale = self.cache.scale_factor();
        let assembly = c.assembly.map(|a| GlyphAssembly {
            italics_correction: a.italics_correction as f64 * scale,
            parts: a.parts.into_iter().map(|p| GlyphPart {
                glyph_id:               p.glyph_id,
                start_connector_length: p.start_connector_length as f64 * scale,
                end_connector_length:   p.end_connector_length as f64 * scale,
                full_advance:           p.full_advance as f64 * scale,
                is_extender:            p.is_extender,
            }).collect(),
        });
        let variants = c.variants.into_iter()
            .map(|v| MathGlyphVariant { glyph_id: v.glyph_id, advance: v.advance as f64 * scale })
            .collect();
        Some(MathGlyphConstruction { assembly, variants })
    }

    pub fn math_min_connector_overlap(&self) -> Option<f64> {
        crate::daecore::daetype::math_table::math_min_connector_overlap(&self.cache.table_map)
            .map(|v| v as f64 * self.cache.scale_factor())
    }

    pub fn stat_info(&self) -> Option<StatInfo> {
        let (axes, values, elided_fallback_name) = crate::daecore::daetype::stat::parse_stat(&self.cache.table_map).ok()?;
        Some(StatInfo { axes, values, elided_fallback_name })
    }

    // A script's baselines on one axis at a location, DFLT's when the script has none.
    pub fn base_info(&self, script_tag: &str, vertical: bool, axes: &[(&str, f64)]) -> Option<BaseScriptInfo> {
        let location = self.cache.compute_location_rs(&owned_axes(axes));
        let table_map = &self.cache.table_map;
        let mut info = crate::daecore::daetype::base::base_script_info(table_map, script_tag, vertical, &location)?;
        let scale = self.cache.scale_factor();
        info.baseline_coords.values_mut().for_each(|c| *c *= scale);
        Some(info)
    }

    // The lowest and highest extent BASE gives a script on one axis at a location, for a language and a
    // feature when given; each side None where the font gives none.
    pub fn base_extents(
        &self, script_tag: &str, language: Option<&str>, feature: Option<&str>, vertical: bool, axes: &[(&str, f64)],
    ) -> Option<(Option<f64>, Option<f64>)> {
        let location = self.cache.compute_location_rs(&owned_axes(axes));
        let (min, max) = crate::daecore::daetype::base::base_min_max(
            &self.cache.table_map, script_tag, language, feature, vertical, &location,
        )?;
        let scale = self.cache.scale_factor();
        Some((min.map(|v| v * scale), max.map(|v| v * scale)))
    }

    pub fn base_is_glyph_free(&self) -> bool {
        self.cache
            .table_map
            .get("BASE")
            .is_some_and(|base| crate::daecore::daetype::base::base_is_glyph_free(base))
    }

    pub fn script_tags(&self) -> Vec<String> {
        self.layout_tags(crate::daecore::daeshaper::ot::offered::script_tags)
    }

    pub fn language_tags(&self, script: &str) -> Vec<String> {
        let Some(tag) = four_bytes(script) else { return Vec::new() };
        self.layout_tags(move |d| crate::daecore::daeshaper::ot::offered::language_tags(d, &tag))
    }

    pub fn feature_tags(&self, script: Option<&str>, language: Option<&str>) -> Vec<String> {
        let s = match script { Some(t) => match four_bytes(t) { Some(b) => Some(b), None => return Vec::new() }, None => None };
        let l = match language { Some(t) => match four_bytes(t) { Some(b) => Some(b), None => return Vec::new() }, None => None };
        self.layout_tags(move |d| {
            crate::daecore::daeshaper::ot::offered::feature_tags(d, s.as_ref(), l.as_ref())
        })
    }

    fn layout_tags(&self, read: impl Fn(&[u8]) -> Vec<[u8; 4]>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for tag in ["GSUB", "GPOS"] {
            let Some(data) = self.cache.table_map.get(tag) else { continue };
            out.extend(read(data).iter().filter_map(|t| {
                core::str::from_utf8(t).ok().map(ToString::to_string)
            }));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn justification_glyphs(&self, script_tag: &str) -> Option<Vec<u16>> {
        crate::daecore::daetype::jstf::jstf_extender_glyphs(&self.cache.table_map, script_tag)
    }

    pub fn justification_priorities(&self, script_tag: &str, lang_sys_tag: Option<&str>) -> Option<Vec<JstfModLists>> {
        crate::daecore::daetype::jstf::jstf_priorities(&self.cache.table_map, script_tag, lang_sys_tag)
    }
}
