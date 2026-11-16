use super::super::decoder::{offset16_in, read_i16_be, read_u16_be};
use super::{device_delta, Deltas, CONSTANTS_LEN};
use crate::daecore::daetype::Tables;

/// MATH's constants, which place scripts, limits, fractions, stacks, bars and radicals: in font units, but for the
/// three percentages. Laid out as C reads it.
#[non_exhaustive]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MathConstants {
    /// How far down a first-level subscript or superscript is scaled, in percent.
    pub script_percent_scale_down: f64,
    /// How far down a second-level subscript or superscript is scaled, in percent.
    pub script_script_percent_scale_down: f64,
    /// The least height of a subformula that delimiters grow around.
    pub delimited_sub_formula_min_height: f64,
    /// The least height of an n-ary operator, such as a sum or an integral, in display style.
    pub display_operator_min_height: f64,
    /// The space between the bottom of one line and the top of the next, for multiline formulas.
    pub math_leading: f64,
    /// The math axis above the baseline, which fraction bars and operators center on.
    pub axis_height: f64,
    /// The greatest height of a base that takes an accent without moving it up.
    pub accent_base_height: f64,
    /// The least height of a base that takes the flattened form of an accent.
    pub flattened_accent_base_height: f64,
    /// How far a subscript's baseline drops below the base's.
    pub subscript_shift_down: f64,
    /// The highest the top of a subscript may reach above the baseline.
    pub subscript_top_max: f64,
    /// The least drop of a subscript's baseline below the bottom of the base.
    pub subscript_baseline_drop_min: f64,
    /// How far a superscript's baseline rises above the base's.
    pub superscript_shift_up: f64,
    /// How far a superscript's baseline rises in cramped style.
    pub superscript_shift_up_cramped: f64,
    /// The lowest the bottom of a superscript may sit above the baseline.
    pub superscript_bottom_min: f64,
    /// The greatest drop of a superscript's baseline below the top of the base.
    pub superscript_baseline_drop_max: f64,
    /// The least gap between a superscript's bottom and a subscript's top.
    pub sub_superscript_gap_min: f64,
    /// The highest a superscript's bottom may sit, with a subscript, before both shift down.
    pub superscript_bottom_max_with_subscript: f64,
    /// Space added after a subscript or superscript.
    pub space_after_script: f64,
    /// The least gap between an upper limit's bottom and the operator's top.
    pub upper_limit_gap_min: f64,
    /// The least rise of an upper limit's baseline above the operator's top.
    pub upper_limit_baseline_rise_min: f64,
    /// The least gap between a lower limit's top and the operator's bottom.
    pub lower_limit_gap_min: f64,
    /// The least drop of a lower limit's baseline below the operator's bottom.
    pub lower_limit_baseline_drop_min: f64,
    /// How far a stack's top element's baseline rises above the baseline.
    pub stack_top_shift_up: f64,
    /// How far a stack's top element's baseline rises in display style.
    pub stack_top_display_style_shift_up: f64,
    /// How far a stack's bottom element's baseline drops below the baseline.
    pub stack_bottom_shift_down: f64,
    /// How far a stack's bottom element's baseline drops in display style.
    pub stack_bottom_display_style_shift_down: f64,
    /// The least gap between a stack's top element's bottom and its bottom element's top.
    pub stack_gap_min: f64,
    /// The least gap between the elements of a stack in display style.
    pub stack_display_style_gap_min: f64,
    /// How far the baseline of an element above a stretched glyph rises above the glyph's top.
    pub stretch_stack_top_shift_up: f64,
    /// How far the baseline of an element below a stretched glyph drops below the glyph's bottom.
    pub stretch_stack_bottom_shift_down: f64,
    /// The least gap between a stretched glyph and an element above it.
    pub stretch_stack_gap_above_min: f64,
    /// The least gap between a stretched glyph and an element below it.
    pub stretch_stack_gap_below_min: f64,
    /// How far a numerator's baseline rises above the baseline.
    pub fraction_numerator_shift_up: f64,
    /// How far a numerator's baseline rises in display style.
    pub fraction_numerator_display_style_shift_up: f64,
    /// How far a denominator's baseline drops below the baseline.
    pub fraction_denominator_shift_down: f64,
    /// How far a denominator's baseline drops in display style.
    pub fraction_denominator_display_style_shift_down: f64,
    /// The least gap between a numerator's bottom and the fraction bar.
    pub fraction_numerator_gap_min: f64,
    /// The least gap between a numerator's bottom and the fraction bar in display style.
    pub fraction_num_display_style_gap_min: f64,
    /// The fraction bar's thickness.
    pub fraction_rule_thickness: f64,
    /// The least gap between the fraction bar and a denominator's top.
    pub fraction_denominator_gap_min: f64,
    /// The least gap between the fraction bar and a denominator's top in display style.
    pub fraction_denom_display_style_gap_min: f64,
    /// The horizontal gap between a skewed fraction's numerator and denominator.
    pub skewed_fraction_horizontal_gap: f64,
    /// The vertical gap between a skewed fraction's numerator and denominator.
    pub skewed_fraction_vertical_gap: f64,
    /// The gap between an overbar and the top of what it covers.
    pub overbar_vertical_gap: f64,
    /// An overbar's thickness.
    pub overbar_rule_thickness: f64,
    /// Space added above an overbar.
    pub overbar_extra_ascender: f64,
    /// The gap between an underbar and the bottom of what it underlines.
    pub underbar_vertical_gap: f64,
    /// An underbar's thickness.
    pub underbar_rule_thickness: f64,
    /// Space added below an underbar.
    pub underbar_extra_descender: f64,
    /// The gap between a radical's bar and the top of the radicand.
    pub radical_vertical_gap: f64,
    /// The gap between a radical's bar and the radicand in display style.
    pub radical_display_style_vertical_gap: f64,
    /// A radical's bar's thickness.
    pub radical_rule_thickness: f64,
    /// Space added above a radical's bar.
    pub radical_extra_ascender: f64,
    /// Space before a radical's degree.
    pub radical_kern_before_degree: f64,
    /// Space after a radical's degree, negative to tuck it in.
    pub radical_kern_after_degree: f64,
    /// How high a radical's degree's bottom sits, in percent of the radical's height.
    pub radical_degree_bottom_raise_percent: f64,
}

pub fn parse_math_constants(table_map: &Tables, deltas: Deltas<'_>) -> Option<MathConstants> {
    let math = table_map.get("MATH")?;
    let at = offset16_in(math, 0, 4)?;
    let buf = math.get(at..at + CONSTANTS_LEN)?;

    let mut stored = [0i16; 51];
    for (i, r) in stored.iter_mut().enumerate() {
        *r = read_i16_be(buf, 8 + i * 4)?;
    }
    // Two branches, so the default's values convert in place: one array for both took 28% longer, copying it.
    match deltas {
        None => constants(buf, stored.map(f64::from)),
        Some(deltas) => {
            let mut raw = stored.map(f64::from);
            for (i, r) in raw.iter_mut().enumerate() {
                *r += device_delta(math, at + 8 + i * 4, at, deltas);
            }
            constants(buf, raw)
        }
    }
}

#[inline(always)]
fn constants(buf: &[u8], raw: [f64; 51]) -> Option<MathConstants> {
    let script_percent_scale_down = read_i16_be(buf, 0)?.into();
    let script_script_percent_scale_down = read_i16_be(buf, 2)?.into();
    let delimited_sub_formula_min_height = read_u16_be(buf, 4)?.into();
    let display_operator_min_height = read_u16_be(buf, 6)?.into();

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

    let radical_degree_bottom_raise_percent = read_i16_be(buf, CONSTANTS_LEN - 2)?.into();

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
