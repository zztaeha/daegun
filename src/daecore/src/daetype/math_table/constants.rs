use alloc::string::String;
use alloc::collections::BTreeMap;
use super::super::decoder::{read_u16_be, read_i16_be};
use super::read_math_value;
use crate::daecore::daetype::TableBytes;

pub struct MathConstants {
    pub script_percent_scale_down: i16,
    pub script_script_percent_scale_down: i16,
    pub delimited_sub_formula_min_height: u16,
    pub display_operator_min_height: u16,
    pub math_leading: i16,
    pub axis_height: i16,
    pub accent_base_height: i16,
    pub flattened_accent_base_height: i16,
    pub subscript_shift_down: i16,
    pub subscript_top_max: i16,
    pub subscript_baseline_drop_min: i16,
    pub superscript_shift_up: i16,
    pub superscript_shift_up_cramped: i16,
    pub superscript_bottom_min: i16,
    pub superscript_baseline_drop_max: i16,
    pub sub_superscript_gap_min: i16,
    pub superscript_bottom_max_with_subscript: i16,
    pub space_after_script: i16,
    pub upper_limit_gap_min: i16,
    pub upper_limit_baseline_rise_min: i16,
    pub lower_limit_gap_min: i16,
    pub lower_limit_baseline_drop_min: i16,
    pub stack_top_shift_up: i16,
    pub stack_top_display_style_shift_up: i16,
    pub stack_bottom_shift_down: i16,
    pub stack_bottom_display_style_shift_down: i16,
    pub stack_gap_min: i16,
    pub stack_display_style_gap_min: i16,
    pub stretch_stack_top_shift_up: i16,
    pub stretch_stack_bottom_shift_down: i16,
    pub stretch_stack_gap_above_min: i16,
    pub stretch_stack_gap_below_min: i16,
    pub fraction_numerator_shift_up: i16,
    pub fraction_numerator_display_style_shift_up: i16,
    pub fraction_denominator_shift_down: i16,
    pub fraction_denominator_display_style_shift_down: i16,
    pub fraction_numerator_gap_min: i16,
    pub fraction_num_display_style_gap_min: i16,
    pub fraction_rule_thickness: i16,
    pub fraction_denominator_gap_min: i16,
    pub fraction_denom_display_style_gap_min: i16,
    pub skewed_fraction_horizontal_gap: i16,
    pub skewed_fraction_vertical_gap: i16,
    pub overbar_vertical_gap: i16,
    pub overbar_rule_thickness: i16,
    pub overbar_extra_ascender: i16,
    pub underbar_vertical_gap: i16,
    pub underbar_rule_thickness: i16,
    pub underbar_extra_descender: i16,
    pub radical_vertical_gap: i16,
    pub radical_display_style_vertical_gap: i16,
    pub radical_rule_thickness: i16,
    pub radical_extra_ascender: i16,
    pub radical_kern_before_degree: i16,
    pub radical_kern_after_degree: i16,
    pub radical_degree_bottom_raise_percent: i16,
}

pub fn parse_math_constants(table_map: &BTreeMap<String, TableBytes>) -> Option<MathConstants> {
    let math = table_map.get("MATH")?;
    let constants_off = read_u16_be(math, 4).filter(|&o| o != 0)? as usize;
    let buf = math.get(constants_off..)?;

    let script_percent_scale_down = read_i16_be(buf, 0)?;
    let script_script_percent_scale_down = read_i16_be(buf, 2)?;
    let delimited_sub_formula_min_height = read_u16_be(buf, 4)?;
    let display_operator_min_height = read_u16_be(buf, 6)?;

    let mut raw = [0i16; 51];
    for (i, r) in raw.iter_mut().enumerate() {
        *r = read_math_value(buf, 8 + i * 4)?;
    }
    let [
        math_leading, axis_height, accent_base_height, flattened_accent_base_height,
        subscript_shift_down, subscript_top_max, subscript_baseline_drop_min,
        superscript_shift_up, superscript_shift_up_cramped, superscript_bottom_min,
        superscript_baseline_drop_max, sub_superscript_gap_min, superscript_bottom_max_with_subscript,
        space_after_script, upper_limit_gap_min, upper_limit_baseline_rise_min,
        lower_limit_gap_min, lower_limit_baseline_drop_min, stack_top_shift_up,
        stack_top_display_style_shift_up, stack_bottom_shift_down, stack_bottom_display_style_shift_down,
        stack_gap_min, stack_display_style_gap_min, stretch_stack_top_shift_up,
        stretch_stack_bottom_shift_down, stretch_stack_gap_above_min, stretch_stack_gap_below_min,
        fraction_numerator_shift_up, fraction_numerator_display_style_shift_up,
        fraction_denominator_shift_down, fraction_denominator_display_style_shift_down,
        fraction_numerator_gap_min, fraction_num_display_style_gap_min, fraction_rule_thickness,
        fraction_denominator_gap_min, fraction_denom_display_style_gap_min,
        skewed_fraction_horizontal_gap, skewed_fraction_vertical_gap, overbar_vertical_gap,
        overbar_rule_thickness, overbar_extra_ascender, underbar_vertical_gap,
        underbar_rule_thickness, underbar_extra_descender, radical_vertical_gap,
        radical_display_style_vertical_gap, radical_rule_thickness, radical_extra_ascender,
        radical_kern_before_degree, radical_kern_after_degree,
    ] = raw;

    let radical_degree_bottom_raise_percent = read_i16_be(buf, 8 + 51 * 4)?;

    Some(MathConstants {
        script_percent_scale_down, script_script_percent_scale_down,
        delimited_sub_formula_min_height, display_operator_min_height,
        math_leading, axis_height, accent_base_height, flattened_accent_base_height,
        subscript_shift_down, subscript_top_max, subscript_baseline_drop_min,
        superscript_shift_up, superscript_shift_up_cramped, superscript_bottom_min,
        superscript_baseline_drop_max, sub_superscript_gap_min, superscript_bottom_max_with_subscript,
        space_after_script, upper_limit_gap_min, upper_limit_baseline_rise_min,
        lower_limit_gap_min, lower_limit_baseline_drop_min, stack_top_shift_up,
        stack_top_display_style_shift_up, stack_bottom_shift_down, stack_bottom_display_style_shift_down,
        stack_gap_min, stack_display_style_gap_min, stretch_stack_top_shift_up,
        stretch_stack_bottom_shift_down, stretch_stack_gap_above_min, stretch_stack_gap_below_min,
        fraction_numerator_shift_up, fraction_numerator_display_style_shift_up,
        fraction_denominator_shift_down, fraction_denominator_display_style_shift_down,
        fraction_numerator_gap_min, fraction_num_display_style_gap_min, fraction_rule_thickness,
        fraction_denominator_gap_min, fraction_denom_display_style_gap_min,
        skewed_fraction_horizontal_gap, skewed_fraction_vertical_gap, overbar_vertical_gap,
        overbar_rule_thickness, overbar_extra_ascender, underbar_vertical_gap,
        underbar_rule_thickness, underbar_extra_descender, radical_vertical_gap,
        radical_display_style_vertical_gap, radical_rule_thickness, radical_extra_ascender,
        radical_kern_before_degree, radical_kern_after_degree,
        radical_degree_bottom_raise_percent,
    })
}
