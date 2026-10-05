// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Anything the caller must uphold beyond that is noted at its site.

use alloc::string::String;
use alloc::vec::Vec;

use crate::Font;

use crate::ffi::handle::{Status, borrow, deliver};
use crate::ffi::list::{Axis, F64List, I32List, StrList, Text, axes_of};

macro_rules! scalar {
    ($fn_name:ident, $ty:ty, $method:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(font: *const Font, out: *mut $ty) -> Status {
            let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
            if out.is_null() {
                return Status::Null;
            }
            unsafe { *out = font.$method() };
            Status::Ok
        }
    };
}

scalar!(daegun_font_is_variable, bool, is_variable);
scalar!(daegun_font_ascender, i32, ascender);
scalar!(daegun_font_descender, i32, descender);
scalar!(daegun_font_cap_height, i32, cap_height);
scalar!(daegun_font_flags, u32, flags);
scalar!(daegun_font_italic_angle, f64, italic_angle);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_family_name(
    font: *const Font,
    out: *mut *mut Text,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    match font.family_name() {
        Some(name) => unsafe { deliver(out, Text::new(&name)) },
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_style(font: *const Font, out: *mut *mut Text) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { deliver(out, Text::new(font.style())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_name_string(
    font: *const Font,
    name_id: u16,
    out: *mut *mut Text,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    match font.name_string(name_id) {
        Some(s) => unsafe { deliver(out, Text::new(&s)) },
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_names(
    font: *const Font,
    out_ids: *mut *mut crate::ffi::list::U16List,
    out_strings: *mut *mut StrList,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let names = font.names();
    if !out_ids.is_null() {
        let ids: Vec<u16> = names.keys().copied().collect();
        let st = unsafe { deliver(out_ids, crate::ffi::list::U16List(ids)) };
        if st != Status::Ok {
            return st;
        }
    }
    if !out_strings.is_null() {
        let strings: Vec<String> = names.values().cloned().collect();
        return unsafe { deliver(out_strings, StrList::new(strings)) };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_bbox(font: *const Font, out: *mut *mut I32List) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { deliver(out, I32List(font.bbox())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_normalized_axes(
    font: *const Font,
    axes: *const Axis,
    axes_len: usize,
    out: *mut *mut F64List,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    unsafe { deliver(out, F64List(font.normalized_axes(&location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_instance(
    font: *const Font,
    axes: *const Axis,
    axes_len: usize,
    out: *mut *mut crate::ffi::list::Blob,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    unsafe { deliver(out, crate::ffi::list::Blob(font.instance(&location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_tracking(
    font: *const Font,
    ptem: f64,
    horizontal: bool,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.tracking(ptem, horizontal) };
    Status::Ok
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LineMetricsC {
    pub ascent: f64,
    pub descent: f64,
    pub line_gap: f64,
}

const _: () = assert!(size_of::<LineMetricsC>() == 24);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_line_metrics(
    font: *const Font,
    vertical: bool,
    out: *mut LineMetricsC,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let crate::LineMetrics { ascent, descent, line_gap } = font.line_metrics(vertical);
    unsafe { *out = LineMetricsC { ascent, descent, line_gap } };
    Status::Ok
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Os2InfoC {
    pub version: u16,
    pub has_family_class: u16,
    pub family_class: u16,
    pub has_selection: u16,
    pub selection: u16,
    pub has_win_metrics: u16,
    pub win_ascent: i32,
    pub win_descent: i32,
    pub has_typo_metrics: u16,
    pub typo_ascender: i32,
    pub typo_descender: i32,
    pub typo_line_gap: i32,
}
const _: () = assert!(size_of::<Os2InfoC>() == 36);

macro_rules! os2_flag {
    ($fn_name:ident, $method:ident) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(font: *const Font, out: *mut bool) -> Status {
            let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
            if out.is_null() {
                return Status::Null;
            }
            let Some(info) = font.os2_info() else { return Status::Absent };
            unsafe { *out = info.$method() };
            Status::Ok
        }
    };
}

os2_flag!(daegun_font_is_italic, is_italic);
os2_flag!(daegun_font_is_bold, is_bold);
os2_flag!(daegun_font_is_regular, is_regular);
os2_flag!(daegun_font_is_oblique, is_oblique);
os2_flag!(daegun_font_uses_typo_metrics, uses_typo_metrics);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_os2_info(font: *const Font, out: *mut Os2InfoC) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(crate::Os2Info { version, family_class, selection, win_metrics, typo_metrics }) = font.os2_info() else {
        return Status::Absent;
    };
    let (win_ascent, win_descent) =
        win_metrics.as_ref().map_or((0, 0), |&crate::WinMetrics { ascent, descent }| (ascent, descent));
    let (typo_ascender, typo_descender, typo_line_gap) = typo_metrics
        .as_ref()
        .map_or((0, 0, 0), |&crate::TypoLineMetrics { ascender, descender, line_gap }| (ascender, descender, line_gap));
    unsafe {
        *out = Os2InfoC {
            version,
            has_family_class: u16::from(family_class.is_some()),
            family_class: family_class.unwrap_or(0),
            has_selection: u16::from(selection.is_some()),
            selection: selection.unwrap_or(0),
            has_win_metrics: u16::from(win_metrics.is_some()),
            win_ascent,
            win_descent,
            has_typo_metrics: u16::from(typo_metrics.is_some()),
            typo_ascender,
            typo_descender,
            typo_line_gap,
        }
    };
    Status::Ok
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TypographicMetricsC {
    pub x_height: i32,
    pub underline_position: i32,
    pub underline_thickness: i32,
    pub strikeout_size: i32,
    pub strikeout_position: i32,
    pub subscript_x_size: i32,
    pub subscript_y_size: i32,
    pub subscript_x_offset: i32,
    pub subscript_y_offset: i32,
    pub superscript_x_size: i32,
    pub superscript_y_size: i32,
    pub superscript_x_offset: i32,
    pub superscript_y_offset: i32,
}

const _: () = assert!(size_of::<TypographicMetricsC>() == 52);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_typographic_metrics(
    font: *const Font,
    axes: *const Axis,
    axes_len: usize,
    out: *mut TypographicMetricsC,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let Some(crate::TypographicMetrics {
        x_height,
        underline_position,
        underline_thickness,
        strikeout_size,
        strikeout_position,
        subscript,
        superscript,
    }) = font.typographic_metrics(&location)
    else {
        return Status::Absent;
    };
    let crate::SubSuperMetrics {
        x_size: subscript_x_size,
        y_size: subscript_y_size,
        x_offset: subscript_x_offset,
        y_offset: subscript_y_offset,
    } = subscript;
    let crate::SubSuperMetrics {
        x_size: superscript_x_size,
        y_size: superscript_y_size,
        x_offset: superscript_x_offset,
        y_offset: superscript_y_offset,
    } = superscript;
    unsafe {
        *out = TypographicMetricsC {
            x_height,
            underline_position,
            underline_thickness,
            strikeout_size,
            strikeout_position,
            subscript_x_size,
            subscript_y_size,
            subscript_x_offset,
            subscript_y_offset,
            superscript_x_size,
            superscript_y_size,
            superscript_x_offset,
            superscript_y_offset,
        }
    };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_axes(
    font: *const Font,
    out_tags: *mut *mut StrList,
    out_ranges: *mut *mut F64List,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let axes = font.axes();
    if !out_tags.is_null() {
        let tags: Vec<String> = axes.iter().map(|a| a.tag.clone()).collect();
        let st = unsafe { deliver(out_tags, StrList::new(tags)) };
        if st != Status::Ok {
            return st;
        }
    }
    if !out_ranges.is_null() {
        let ranges: Vec<f64> = axes
            .iter()
            .flat_map(|&crate::FvarAxis { tag: _, min, default, max, flags: _, name_id: _ }| [min, default, max])
            .collect();
        return unsafe { deliver(out_ranges, F64List(ranges)) };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_axis_flags(
    font: *const Font,
    out_flags: *mut *mut crate::ffi::list::U16List,
    out_name_ids: *mut *mut crate::ffi::list::U16List,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let axes = font.axes();
    if !out_flags.is_null() {
        let flags = axes.iter().map(|a| a.flags).collect();
        let st = unsafe { deliver(out_flags, crate::ffi::list::U16List(flags)) };
        if st != Status::Ok {
            return st;
        }
    }
    if !out_name_ids.is_null() {
        let ids = axes.iter().map(|a| a.name_id).collect();
        return unsafe { deliver(out_name_ids, crate::ffi::list::U16List(ids)) };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_named_instance_count(
    font: *const Font,
    out: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.named_instances().len() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_named_instance(
    font: *const Font,
    index: usize,
    out_name: *mut *mut Text,
    out_postscript_name: *mut *mut Text,
    out_coord_tags: *mut *mut StrList,
    out_coord_values: *mut *mut F64List,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let all = font.named_instances();
    let Some(crate::NamedInstance { name, postscript_name, coords }) = all.get(index) else { return Status::Range };

    if !out_name.is_null() {
        match name {
            Some(n) => {
                let st = unsafe { deliver(out_name, Text::new(n)) };
                if st != Status::Ok {
                    return st;
                }
            }
            None => unsafe { *out_name = core::ptr::null_mut() },
        }
    }
    if !out_postscript_name.is_null() {
        match postscript_name {
            Some(n) => {
                let st = unsafe { deliver(out_postscript_name, Text::new(n)) };
                if st != Status::Ok {
                    return st;
                }
            }
            None => unsafe { *out_postscript_name = core::ptr::null_mut() },
        }
    }
    if !out_coord_tags.is_null() {
        let tags: Vec<String> = coords.iter().map(|(t, _)| t.clone()).collect();
        let st = unsafe { deliver(out_coord_tags, StrList::new(tags)) };
        if st != Status::Ok {
            return st;
        }
    }
    if !out_coord_values.is_null() {
        let values: Vec<f64> = coords.iter().map(|(_, v)| *v).collect();
        return unsafe { deliver(out_coord_values, F64List(values)) };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_line_metrics_height(
    metrics: *const LineMetricsC,
    out: *mut f64,
) -> Status {
    let Some(m) = (unsafe { borrow(metrics) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = m.ascent - m.descent + m.line_gap };
    Status::Ok
}
