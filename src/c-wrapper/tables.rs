// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_char;

use crate::{Font, MathKernCorner};

use crate::ffi::handle::{OwnedStr, Status, Str, borrow, deliver, release, slice_of, str_of};
use crate::ffi::list::{Axis, F64List, StrList, Text, U16List, axes_of};

pub struct SubsetHandle(crate::SubsetResult);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subset_ttf(
    subset: *const SubsetHandle,
    out_len: *mut usize,
) -> *const u8 {
    let Some(SubsetHandle(crate::SubsetResult { ttf, gid_map: _ })) = (unsafe { borrow(subset) }) else {
        return core::ptr::null();
    };
    if out_len.is_null() {
        return core::ptr::null();
    }
    unsafe { *out_len = ttf.len() };
    ttf.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subset_gid_map(
    subset: *const SubsetHandle,
    out_len: *mut usize,
) -> *const u16 {
    let Some(s) = (unsafe { borrow(subset) }) else { return core::ptr::null() };
    if out_len.is_null() {
        return core::ptr::null();
    }
    unsafe { *out_len = s.0.gid_map.len() };
    s.0.gid_map.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subset_free(subset: *mut SubsetHandle) {
    unsafe { release(subset) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_subset(
    font: *const Font,
    gids: *const u16,
    gids_len: usize,
    axes: *const Axis,
    axes_len: usize,
    out: *mut *mut SubsetHandle,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(ids) = (unsafe { slice_of(gids, gids_len) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    match font.subset(ids, &location) {
        Ok(r) => unsafe { deliver(out, SubsetHandle(r)) },
        Err(e) => {
            crate::ffi::set_error(&alloc::format!("{e}"));
            Status::Parse
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_subset_text(
    font: *const Font,
    text: *const c_char,
    axes: *const Axis,
    axes_len: usize,
    out: *mut *mut SubsetHandle,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(text) = (unsafe { str_of(text) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    match font.subset_text(text, &location) {
        Ok(r) => unsafe { deliver(out, SubsetHandle(r)) },
        Err(e) => {
            crate::ffi::set_error(&alloc::format!("{e}"));
            Status::Parse
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_closure(
    font: *const Font,
    gids: *const u16,
    gids_len: usize,
    axes: *const Axis,
    axes_len: usize,
    out: *mut *mut U16List,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(ids) = (unsafe { slice_of(gids, gids_len) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    match font.glyph_closure(ids, &location) {
        Ok(v) => unsafe { deliver(out, U16List(v)) },
        Err(e) => {
            crate::ffi::set_error(&alloc::format!("{e}"));
            Status::Parse
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_constant(
    font: *const Font,
    which: i32,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(crate::MathConstants {
        script_percent_scale_down,
        script_script_percent_scale_down,
        delimited_sub_formula_min_height,
        display_operator_min_height,
        math_leading,
        axis_height,
        accent_base_height,
        flattened_accent_base_height,
        subscript_shift_down,
        subscript_top_max,
        subscript_baseline_drop_min,
        superscript_shift_up,
        superscript_shift_up_cramped,
        superscript_bottom_min,
        superscript_baseline_drop_max,
        sub_superscript_gap_min,
        superscript_bottom_max_with_subscript,
        space_after_script,
        upper_limit_gap_min,
        upper_limit_baseline_rise_min,
        lower_limit_gap_min,
        lower_limit_baseline_drop_min,
        stack_top_shift_up,
        stack_top_display_style_shift_up,
        stack_bottom_shift_down,
        stack_bottom_display_style_shift_down,
        stack_gap_min,
        stack_display_style_gap_min,
        stretch_stack_top_shift_up,
        stretch_stack_bottom_shift_down,
        stretch_stack_gap_above_min,
        stretch_stack_gap_below_min,
        fraction_numerator_shift_up,
        fraction_numerator_display_style_shift_up,
        fraction_denominator_shift_down,
        fraction_denominator_display_style_shift_down,
        fraction_numerator_gap_min,
        fraction_num_display_style_gap_min,
        fraction_rule_thickness,
        fraction_denominator_gap_min,
        fraction_denom_display_style_gap_min,
        skewed_fraction_horizontal_gap,
        skewed_fraction_vertical_gap,
        overbar_vertical_gap,
        overbar_rule_thickness,
        overbar_extra_ascender,
        underbar_vertical_gap,
        underbar_rule_thickness,
        underbar_extra_descender,
        radical_vertical_gap,
        radical_display_style_vertical_gap,
        radical_rule_thickness,
        radical_extra_ascender,
        radical_kern_before_degree,
        radical_kern_after_degree,
        radical_degree_bottom_raise_percent,
    }) = font.math_constants()
    else {
        return Status::Absent;
    };
    let value = match which {
        0 => script_percent_scale_down,
        1 => script_script_percent_scale_down,
        2 => delimited_sub_formula_min_height,
        3 => display_operator_min_height,
        4 => math_leading,
        5 => axis_height,
        6 => accent_base_height,
        7 => flattened_accent_base_height,
        8 => subscript_shift_down,
        9 => subscript_top_max,
        10 => subscript_baseline_drop_min,
        11 => superscript_shift_up,
        12 => superscript_shift_up_cramped,
        13 => superscript_bottom_min,
        14 => superscript_baseline_drop_max,
        15 => sub_superscript_gap_min,
        16 => superscript_bottom_max_with_subscript,
        17 => space_after_script,
        18 => upper_limit_gap_min,
        19 => upper_limit_baseline_rise_min,
        20 => lower_limit_gap_min,
        21 => lower_limit_baseline_drop_min,
        22 => stack_top_shift_up,
        23 => stack_top_display_style_shift_up,
        24 => stack_bottom_shift_down,
        25 => stack_bottom_display_style_shift_down,
        26 => stack_gap_min,
        27 => stack_display_style_gap_min,
        28 => stretch_stack_top_shift_up,
        29 => stretch_stack_bottom_shift_down,
        30 => stretch_stack_gap_above_min,
        31 => stretch_stack_gap_below_min,
        32 => fraction_numerator_shift_up,
        33 => fraction_numerator_display_style_shift_up,
        34 => fraction_denominator_shift_down,
        35 => fraction_denominator_display_style_shift_down,
        36 => fraction_numerator_gap_min,
        37 => fraction_num_display_style_gap_min,
        38 => fraction_rule_thickness,
        39 => fraction_denominator_gap_min,
        40 => fraction_denom_display_style_gap_min,
        41 => skewed_fraction_horizontal_gap,
        42 => skewed_fraction_vertical_gap,
        43 => overbar_vertical_gap,
        44 => overbar_rule_thickness,
        45 => overbar_extra_ascender,
        46 => underbar_vertical_gap,
        47 => underbar_rule_thickness,
        48 => underbar_extra_descender,
        49 => radical_vertical_gap,
        50 => radical_display_style_vertical_gap,
        51 => radical_rule_thickness,
        52 => radical_extra_ascender,
        53 => radical_kern_before_degree,
        54 => radical_kern_after_degree,
        55 => radical_degree_bottom_raise_percent,
        _ => return Status::Range,
    };
    unsafe { *out = value };
    Status::Ok
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_math_constant_count() -> i32 {
    56
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_italics_correction(
    font: *const Font,
    gid: u16,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(v) = font.math_italics_correction(gid) else { return Status::Absent };
    unsafe { *out = v };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_top_accent_attachment(
    font: *const Font,
    gid: u16,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.math_top_accent_attachment(gid) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_is_extended_shape(
    font: *const Font,
    gid: u16,
    out: *mut bool,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.math_is_extended_shape(gid) };
    Status::Ok
}

pub const MATH_KERN_TOP_RIGHT: i32 = 0;
pub const MATH_KERN_TOP_LEFT: i32 = 1;
pub const MATH_KERN_BOTTOM_RIGHT: i32 = 2;
pub const MATH_KERN_BOTTOM_LEFT: i32 = 3;

pub(crate) fn corner_of(code: i32) -> Option<MathKernCorner> {
    match code {
        MATH_KERN_TOP_RIGHT => Some(MathKernCorner::TopRight),
        MATH_KERN_TOP_LEFT => Some(MathKernCorner::TopLeft),
        MATH_KERN_BOTTOM_RIGHT => Some(MathKernCorner::BottomRight),
        MATH_KERN_BOTTOM_LEFT => Some(MathKernCorner::BottomLeft),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_kern(
    font: *const Font,
    gid: u16,
    corner: i32,
    height: f64,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(corner) = corner_of(corner) else { return Status::Range };
    let Some(kern) = font.math_kern(gid, corner, height) else { return Status::Range };
    unsafe { *out = kern };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_min_connector_overlap(
    font: *const Font,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(v) = font.math_min_connector_overlap() else { return Status::Absent };
    unsafe { *out = v };
    Status::Ok
}

pub struct MathConstruction {
    variant_gids: Vec<u16>,
    variant_advances: Vec<f64>,
    has_assembly: bool,
    italics_correction: f64,
    part_gids: Vec<u16>,
    part_values: Vec<f64>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_glyph_variants(
    font: *const Font,
    gid: u16,
    vertical: bool,
    out: *mut *mut MathConstruction,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(crate::MathGlyphConstruction { variants, assembly }) = font.math_glyph_variants(gid, vertical) else {
        return Status::Absent;
    };
    let (variant_gids, variant_advances) =
        variants.iter().map(|&crate::MathGlyphVariant { glyph_id, advance }| (glyph_id, advance)).unzip();
    let (has_assembly, italics_correction, parts) = match assembly {
        Some(crate::GlyphAssembly { italics_correction, parts }) => (true, italics_correction, parts),
        None => (false, 0.0, Vec::new()),
    };
    let mut part_gids = Vec::with_capacity(parts.len());
    let mut part_values = Vec::with_capacity(parts.len() * 4);
    for part in parts {
        let crate::GlyphPart { glyph_id, start_connector_length, end_connector_length, full_advance, is_extender } =
            part;
        part_gids.push(glyph_id);
        part_values.extend_from_slice(&[
            start_connector_length,
            end_connector_length,
            full_advance,
            f64::from(u8::from(is_extender)),
        ]);
    }
    let built = MathConstruction {
        variant_gids,
        variant_advances,
        has_assembly,
        italics_correction,
        part_gids,
        part_values,
    };
    unsafe { deliver(out, built) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_math_construction_variants(
    c: *const MathConstruction,
    out_count: *mut usize,
    out_gids: *mut *const u16,
    out_advances: *mut *const f64,
) -> Status {
    let Some(c) = (unsafe { borrow(c) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    unsafe {
        *out_count = c.variant_gids.len();
        if !out_gids.is_null() {
            *out_gids = c.variant_gids.as_ptr();
        }
        if !out_advances.is_null() {
            *out_advances = c.variant_advances.as_ptr();
        }
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_math_construction_assembly(
    c: *const MathConstruction,
    out_italics_correction: *mut f64,
    out_part_count: *mut usize,
    out_part_gids: *mut *const u16,
    out_part_values: *mut *const f64,
) -> Status {
    let Some(c) = (unsafe { borrow(c) }) else { return Status::Null };
    if !c.has_assembly {
        return Status::Absent;
    }
    unsafe {
        if !out_italics_correction.is_null() {
            *out_italics_correction = c.italics_correction;
        }
        if !out_part_count.is_null() {
            *out_part_count = c.part_gids.len();
        }
        if !out_part_gids.is_null() {
            *out_part_gids = c.part_gids.as_ptr();
        }
        if !out_part_values.is_null() {
            *out_part_values = c.part_values.as_ptr();
        }
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_math_construction_free(c: *mut MathConstruction) {
    unsafe { release(c) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_base_is_glyph_free(
    font: *const Font,
    out: *mut bool,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.base_is_glyph_free() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_base_info(
    font: *const Font,
    script_tag: *const c_char,
    vertical: bool,
    axes: *const Axis,
    axes_len: usize,
    out_default_baseline: *mut *mut Text,
    out_baseline_tags: *mut *mut StrList,
    out_baseline_coords: *mut *mut F64List,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(tag) = (unsafe { str_of(script_tag) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let Some(crate::BaseScriptInfo { default_baseline_tag, baseline_coords }) = font.base_info(tag, vertical, &location)
    else {
        return Status::Absent;
    };

    if !out_default_baseline.is_null() {
        match &default_baseline_tag {
            Some(t) => {
                let st = unsafe { deliver(out_default_baseline, Text::new(t)) };
                if st != Status::Ok {
                    return st;
                }
            }
            None => unsafe { *out_default_baseline = core::ptr::null_mut() },
        }
    }
    if !out_baseline_tags.is_null() {
        let tags: Vec<String> = baseline_coords.keys().cloned().collect();
        let st = unsafe { deliver(out_baseline_tags, StrList::new(tags)) };
        if st != Status::Ok {
            return st;
        }
    }
    if !out_baseline_coords.is_null() {
        let coords: Vec<f64> = baseline_coords.values().copied().collect();
        return unsafe { deliver(out_baseline_coords, F64List(coords)) };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_base_extents(
    font: *const Font,
    script_tag: *const c_char,
    language_tag: *const c_char,
    feature_tag: *const c_char,
    vertical: bool,
    axes: *const Axis,
    axes_len: usize,
    out_has_min: *mut bool,
    out_min: *mut f64,
    out_has_max: *mut bool,
    out_max: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(tag) = (unsafe { str_of(script_tag) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let (language, feature) = unsafe { (str_of(language_tag), str_of(feature_tag)) };
    let Some((min, max)) = font.base_extents(tag, language, feature, vertical, &location) else {
        return Status::Absent;
    };
    for (has, out, side) in [(out_has_min, out_min, min), (out_has_max, out_max, max)] {
        unsafe {
            if !has.is_null() {
                *has = side.is_some();
            }
            if !out.is_null() {
                *out = side.unwrap_or(0.0);
            }
        }
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_script_tags(
    font: *const Font,
    out: *mut *mut StrList,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { deliver(out, StrList::new(font.script_tags())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_language_tags(
    font: *const Font,
    script: *const c_char,
    out: *mut *mut StrList,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(script) = (unsafe { str_of(script) }) else { return Status::Null };
    unsafe { deliver(out, StrList::new(font.language_tags(script))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_feature_tags(
    font: *const Font,
    script: *const c_char,
    language: *const c_char,
    out: *mut *mut StrList,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let script = unsafe { str_of(script) };
    let language = unsafe { str_of(language) };
    unsafe { deliver(out, StrList::new(font.feature_tags(script, language))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_justification_glyphs(
    font: *const Font,
    script_tag: *const c_char,
    out: *mut *mut U16List,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(tag) = (unsafe { str_of(script_tag) }) else { return Status::Null };
    let Some(v) = font.justification_glyphs(tag) else { return Status::Absent };
    unsafe { deliver(out, U16List(v)) }
}

pub struct StatHandle {
    axis_tags: Vec<String>,
    axis_names: Vec<Option<OwnedStr>>,
    axis_orderings: Vec<u16>,
    values: Vec<StatValueC>,
    names: Vec<Option<OwnedStr>>,
    combos: Vec<AxisValueC>,
    elided_fallback: Option<String>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StatValueC {
    pub kind: i32,
    pub axis_index: u16,
    pub elidable: u8,
    pub has_name: u8,
    pub older_sibling: u8,
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub linked_value: f64,
    pub combo_start: u32,
    pub combo_count: u32,
}

const _: () = assert!(size_of::<StatValueC>() == 56);
const _: () = assert!(align_of::<StatValueC>() == 8);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AxisValueC {
    pub axis_index: u16,
    pub value: f64,
}

const _: () = assert!(size_of::<AxisValueC>() == 16);

pub const STAT_SINGLE: i32 = 0;
pub const STAT_RANGE: i32 = 1;
pub const STAT_LINKED: i32 = 2;
pub const STAT_COMBO: i32 = 3;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_stat_info(
    font: *const Font,
    out: *mut *mut StatHandle,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(crate::StatInfo { axes, values: stat_values, elided_fallback_name }) = font.stat_info() else {
        return Status::Absent;
    };
    let mut values = Vec::with_capacity(stat_values.len());
    let mut names = Vec::with_capacity(stat_values.len());
    let mut combos = Vec::new();
    for v in &stat_values {
        let (flat, name) = flatten_stat_value(v, &mut combos);
        values.push(flat);
        names.push(name.map(OwnedStr::new));
    }
    let mut axis_tags = Vec::with_capacity(axes.len());
    let mut axis_names = Vec::with_capacity(axes.len());
    let mut axis_orderings = Vec::with_capacity(axes.len());
    for crate::StatAxis { tag, name, ordering } in axes {
        axis_tags.push(tag);
        axis_names.push(name.as_deref().map(OwnedStr::new));
        axis_orderings.push(ordering);
    }
    let built = StatHandle {
        axis_tags,
        axis_names,
        axis_orderings,
        values,
        names,
        combos,
        elided_fallback: elided_fallback_name,
    };
    unsafe { deliver(out, built) }
}

fn flatten_stat_value<'a>(
    v: &'a crate::StatAxisValue,
    combos: &mut Vec<AxisValueC>,
) -> (StatValueC, Option<&'a str>) {
    use crate::StatAxisValue as V;
    let blank = StatValueC {
        kind: STAT_SINGLE,
        axis_index: 0,
        elidable: 0,
        has_name: 0,
        older_sibling: 0,
        value: 0.0,
        min: 0.0,
        max: 0.0,
        linked_value: 0.0,
        combo_start: 0,
        combo_count: 0,
    };
    match v {
        V::Single { axis_index, name, value, elidable, older_sibling } => (
            StatValueC {
                kind: STAT_SINGLE,
                axis_index: *axis_index,
                elidable: u8::from(*elidable),
                older_sibling: u8::from(*older_sibling),
                has_name: u8::from(name.is_some()),
                value: *value,
                ..blank
            },
            name.as_deref(),
        ),
        V::Range { axis_index, name, nominal, min, max, elidable, older_sibling } => (
            StatValueC {
                kind: STAT_RANGE,
                axis_index: *axis_index,
                elidable: u8::from(*elidable),
                older_sibling: u8::from(*older_sibling),
                has_name: u8::from(name.is_some()),
                value: *nominal,
                min: *min,
                max: *max,
                ..blank
            },
            name.as_deref(),
        ),
        V::Linked { axis_index, name, value, linked_value, elidable, older_sibling } => (
            StatValueC {
                kind: STAT_LINKED,
                axis_index: *axis_index,
                elidable: u8::from(*elidable),
                older_sibling: u8::from(*older_sibling),
                has_name: u8::from(name.is_some()),
                value: *value,
                linked_value: *linked_value,
                ..blank
            },
            name.as_deref(),
        ),
        V::Combo { name, values, elidable, older_sibling } => {
            let start = combos.len();
            combos.extend(
                values.iter().map(|(axis_index, value)| AxisValueC {
                    axis_index: *axis_index,
                    value: *value,
                }),
            );
            (
                StatValueC {
                    kind: STAT_COMBO,
                    elidable: u8::from(*elidable),
                    older_sibling: u8::from(*older_sibling),
                    has_name: u8::from(name.is_some()),
                    combo_start: u32::try_from(start).unwrap_or(u32::MAX),
                    combo_count: u32::try_from(values.len()).unwrap_or(0),
                    ..blank
                },
                name.as_deref(),
            )
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_axes(
    stat: *const StatHandle,
    out_count: *mut usize,
    out_tags: *mut *mut StrList,
    out_orderings: *mut *const u16,
) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    unsafe {
        if !out_count.is_null() {
            *out_count = stat.axis_tags.len();
        }
        if !out_orderings.is_null() {
            *out_orderings = stat.axis_orderings.as_ptr();
        }
    }
    if !out_tags.is_null() {
        return unsafe { deliver(out_tags, StrList::new(stat.axis_tags.clone())) };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_axis_name(
    stat: *const StatHandle,
    index: usize,
    out: *mut Str,
) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(slot) = stat.axis_names.get(index) else { return Status::Range };
    let Some(name) = slot.as_ref() else { return Status::Absent };
    unsafe { *out = name.as_str() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_value_count(
    stat: *const StatHandle,
    out: *mut usize,
) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = stat.values.len() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_elided_fallback_name(
    stat: *const StatHandle,
    out: *mut *mut Text,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    let Some(name) = &stat.elided_fallback else { return Status::Absent };
    unsafe { deliver(out, Text::new(name)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_free(stat: *mut StatHandle) {
    unsafe { release(stat) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_value_at(
    stat: *const StatHandle,
    index: usize,
    out: *mut StatValueC,
) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(v) = stat.values.get(index) else { return Status::Range };
    unsafe { *out = *v };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_value_name(
    stat: *const StatHandle,
    index: usize,
    out: *mut Str,
) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(slot) = stat.names.get(index) else { return Status::Range };
    let Some(name) = slot.as_ref() else { return Status::Absent };
    unsafe { *out = name.as_str() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_combo_values(
    stat: *const StatHandle,
    out_count: *mut usize,
) -> *const AxisValueC {
    let Some(stat) = (unsafe { borrow(stat) }) else { return core::ptr::null() };
    if out_count.is_null() {
        return core::ptr::null();
    }
    unsafe { *out_count = stat.combos.len() };
    stat.combos.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subset_new_gid(
    subset: *const SubsetHandle,
    old_gid: u16,
    out: *mut u16,
) -> Status {
    let Some(s) = (unsafe { borrow(subset) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(new) = s.0.new_gid(old_gid) else { return Status::Absent };
    unsafe { *out = new };
    Status::Ok
}
