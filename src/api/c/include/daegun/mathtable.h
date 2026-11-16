/* The MATH table: the constants, glyph metrics and stretching glyphs that math layout reads, in font units at
 * a location, each value with its variation delta there from GDEF's store, unrounded.
 * Rust's math module and Font's math calls. */

#ifndef DAEGUN_MATHTABLE_H
#define DAEGUN_MATHTABLE_H

#include <daegun/base.h>

#ifdef __cplusplus
extern "C" {
#endif

DAEGUN_HEADER_BEGIN

/** MATH's constants, which place scripts, limits, fractions, stacks, bars and radicals, each named as the
 * OpenType spec names it and in its order, as Rust's MathConstants: in font units, but for the three
 * percentages, script_percent_scale_down, script_script_percent_scale_down and
 * radical_degree_bottom_raise_percent. */
typedef struct {
    double script_percent_scale_down;
    double script_script_percent_scale_down;
    double delimited_sub_formula_min_height;
    double display_operator_min_height;
    double math_leading;
    double axis_height;
    double accent_base_height;
    double flattened_accent_base_height;
    double subscript_shift_down;
    double subscript_top_max;
    double subscript_baseline_drop_min;
    double superscript_shift_up;
    double superscript_shift_up_cramped;
    double superscript_bottom_min;
    double superscript_baseline_drop_max;
    double sub_superscript_gap_min;
    double superscript_bottom_max_with_subscript;
    double space_after_script;
    double upper_limit_gap_min;
    double upper_limit_baseline_rise_min;
    double lower_limit_gap_min;
    double lower_limit_baseline_drop_min;
    double stack_top_shift_up;
    double stack_top_display_style_shift_up;
    double stack_bottom_shift_down;
    double stack_bottom_display_style_shift_down;
    double stack_gap_min;
    double stack_display_style_gap_min;
    double stretch_stack_top_shift_up;
    double stretch_stack_bottom_shift_down;
    double stretch_stack_gap_above_min;
    double stretch_stack_gap_below_min;
    double fraction_numerator_shift_up;
    double fraction_numerator_display_style_shift_up;
    double fraction_denominator_shift_down;
    double fraction_denominator_display_style_shift_down;
    double fraction_numerator_gap_min;
    double fraction_num_display_style_gap_min;
    double fraction_rule_thickness;
    double fraction_denominator_gap_min;
    double fraction_denom_display_style_gap_min;
    double skewed_fraction_horizontal_gap;
    double skewed_fraction_vertical_gap;
    double overbar_vertical_gap;
    double overbar_rule_thickness;
    double overbar_extra_ascender;
    double underbar_vertical_gap;
    double underbar_rule_thickness;
    double underbar_extra_descender;
    double radical_vertical_gap;
    double radical_display_style_vertical_gap;
    double radical_rule_thickness;
    double radical_extra_ascender;
    double radical_kern_before_degree;
    double radical_kern_after_degree;
    double radical_degree_bottom_raise_percent;
} daegun_math_constants;

/** At a location. DAEGUN_ABSENT for a font without MATH or with constants it cuts short. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_math_constants(const daegun_font *font, const daegun_location *DAEGUN_NULLABLE location,
                           daegun_math_constants *out);

/** A glyph's italic correction at a location. DAEGUN_ABSENT for one MATH gives none. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_math_italics_correction(const daegun_font *font, uint32_t gid,
                                    const daegun_location *DAEGUN_NULLABLE location, double *out);
/** Where an accent over a glyph centers at a location, from its origin: MATH's, or half its advance there. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_math_top_accent_attachment(const daegun_font *font, uint32_t gid,
                                       const daegun_location *DAEGUN_NULLABLE location, double *out);
/** Whether MATH marks a glyph an extended shape, one whose scripts attach as to a tall glyph. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_math_is_extended_shape(const daegun_font *font, uint32_t gid, bool *out);
/** The least overlap between the parts of an assembled glyph. DAEGUN_ABSENT for a font without MATH. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_math_min_connector_overlap(const daegun_font *font, double *out);

/* Which corner of a glyph a script kerns at: a superscript's after or before it, or a subscript's. */
#define DAEGUN_MATH_KERN_TOP_RIGHT     0
#define DAEGUN_MATH_KERN_TOP_LEFT      1
#define DAEGUN_MATH_KERN_BOTTOM_RIGHT  2
#define DAEGUN_MATH_KERN_BOTTOM_LEFT   3

/** The kern at one of a glyph's corners for a script at `height`, at a location; 0 where MATH gives none.
 * An infinite height selects the first or last kern; DAEGUN_INVALID_ARGUMENT for one that isn't a number. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_math_kern(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                      int32_t corner, double height, double *out);

/* stretching glyphs */

/** One ready-made size of a glyph that stretches: the glyph of that size, and its advance along the way it
 * stretches. */
typedef struct {
    uint32_t glyph;
    double   advance;
} daegun_math_variant;

/** One part of a glyph assembled to any size, along the way it stretches: its glyph, how much of its start
 * and its end may overlap the parts on either side, its whole advance, and whether it repeats, zero or more
 * times, to fill the length. */
typedef struct {
    uint32_t glyph;
    double   start_connector_length;
    double   end_connector_length;
    double   full_advance;
    bool     extender;
} daegun_glyph_part;

/** How to build a glyph to any size: the italic correction of the built glyph, and its parts from the
 * bottom or the left, BORROWED until the construction is freed. */
typedef struct {
    double                   italics_correction;
    const daegun_glyph_part *DAEGUN_NULLABLE parts DAEGUN_COUNTED_BY(parts_len);
    size_t                   parts_len;
} daegun_glyph_assembly;

/** How a glyph stretches one way: its ready-made sizes, smallest first, and how to build it larger. */
typedef struct daegun_math_construction daegun_math_construction;

/** How a glyph stretches along an orientation, DAEGUN_HORIZONTAL or DAEGUN_VERTICAL, at a location, where its
 * assembly's italic correction may vary. DAEGUN_ABSENT for one MATH doesn't stretch that way. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_font_math_glyph_variants(const daegun_font *font, uint32_t gid, const daegun_location *DAEGUN_NULLABLE location,
                                int32_t orientation, daegun_math_construction *DAEGUN_NULLABLE *DAEGUN_NONNULL out);
/** Its sizes, smallest first, BORROWED until it is freed. */
DAEGUN_API const daegun_math_variant *DAEGUN_NULLABLE DAEGUN_LENT(*out_count) DAEGUN_CALL
daegun_math_construction_variants(const daegun_math_construction *construction, size_t *out_count);
/** DAEGUN_ABSENT where the font gives only sizes. */
DAEGUN_NODISCARD DAEGUN_API daegun_status DAEGUN_CALL
daegun_math_construction_assembly(const daegun_math_construction *construction, daegun_glyph_assembly *out);
DAEGUN_API void DAEGUN_CALL
daegun_math_construction_free(daegun_math_construction *DAEGUN_NULLABLE construction);

DAEGUN_LAYOUT_ASSERT(sizeof(daegun_math_constants) == 56 * sizeof(double), "MATH has 56 constants");
#ifdef DAEGUN_LAYOUT_64
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_math_variant) == 16, "a variant is a glyph and a double");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_glyph_part) == 40, "a part is a glyph, three doubles and a flag");
DAEGUN_LAYOUT_ASSERT(sizeof(daegun_glyph_assembly) == 24, "an assembly is a double and a view");
#endif

DAEGUN_HEADER_END

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* DAEGUN_MATHTABLE_H */
