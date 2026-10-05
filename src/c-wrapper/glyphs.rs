// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Anything the caller must uphold beyond that is noted at its site.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_char;

use crate::{Font, GlyphClass};

use crate::ffi::handle::{Status, borrow, deliver, slice_of, str_of};
use crate::ffi::list::{Axis, Blob, F64List, StrList, Text, U16List, U32List, axes_of};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_has_glyph(
    font: *const Font,
    codepoint: u32,
    out: *mut bool,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.has_glyph(codepoint) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_ids(
    font: *const Font,
    text: *const c_char,
    out_gids: *mut *mut U16List,
    out_present: *mut *mut Blob,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(text) = (unsafe { str_of(text) }) else { return Status::Null };
    let ids = font.glyph_ids(text);
    if !out_gids.is_null() {
        let gids: Vec<u16> = ids.iter().map(|o| o.unwrap_or(0)).collect();
        let st = unsafe { deliver(out_gids, U16List(gids)) };
        if st != Status::Ok {
            return st;
        }
    }
    if !out_present.is_null() {
        let present: Vec<u8> = ids.iter().map(|o| u8::from(o.is_some())).collect();
        return unsafe { deliver(out_present, Blob(present)) };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_coverage(
    font: *const Font,
    out_codepoints: *mut *mut U32List,
    out_gids: *mut *mut U16List,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let pairs = font.coverage();
    if !out_codepoints.is_null() {
        let cps: Vec<u32> = pairs.iter().map(|(c, _)| *c).collect();
        let st = unsafe { deliver(out_codepoints, U32List(cps)) };
        if st != Status::Ok {
            return st;
        }
    }
    if !out_gids.is_null() {
        let gids: Vec<u16> = pairs.iter().map(|(_, g)| *g).collect();
        return unsafe { deliver(out_gids, U16List(gids)) };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_codepoints(
    font: *const Font,
    out: *mut *mut U32List,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { deliver(out, U32List(font.codepoints())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_bounds(
    font: *const Font,
    gid: u16,
    axes: *const Axis,
    axes_len: usize,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let Some((x0, y0, x1, y1)) = font.glyph_bounds(gid, &location) else {
        return Status::Absent;
    };
    unsafe {
        *out.add(0) = x0;
        *out.add(1) = y0;
        *out.add(2) = x1;
        *out.add(3) = y1;
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_variation_glyph_id(
    font: *const Font,
    base: u32,
    selector: u32,
    out: *mut u16,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(gid) = font.variation_glyph_id(base, selector) else { return Status::Absent };
    unsafe { *out = gid };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_advance_widths(
    font: *const Font,
    gids: *const u16,
    gids_len: usize,
    axes: *const Axis,
    axes_len: usize,
    out: *mut *mut F64List,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(ids) = (unsafe { slice_of(gids, gids_len) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    unsafe { deliver(out, F64List(font.advance_widths(ids, &location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_vertical_advance(
    font: *const Font,
    gid: u16,
    axes: *const Axis,
    axes_len: usize,
    out: *mut u32,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    unsafe { *out = font.vertical_advance(gid, &location) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_vertical_origin(
    font: *const Font,
    gid: u16,
    axes: *const Axis,
    axes_len: usize,
    out: *mut i32,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let Some(v) = font.vertical_origin(gid, &location) else { return Status::Absent };
    unsafe { *out = v };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_default_vertical_origin(
    font: *const Font,
    out: *mut i32,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.default_vertical_origin() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_ligature_carets(
    font: *const Font,
    gid: u16,
    axes: *const Axis,
    axes_len: usize,
    vertical: bool,
    out_values: *mut *mut F64List,
    out_present: *mut *mut Blob,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let carets = font.ligature_carets(gid, &location, vertical);
    if !out_values.is_null() {
        let values: Vec<f64> = carets.iter().map(|c| c.unwrap_or(0.0)).collect();
        let st = unsafe { deliver(out_values, F64List(values)) };
        if st != Status::Ok {
            return st;
        }
    }
    if !out_present.is_null() {
        let present: Vec<u8> = carets.iter().map(|c| u8::from(c.is_some())).collect();
        return unsafe { deliver(out_present, Blob(present)) };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_caret_positions(
    font: *const Font,
    text: *const c_char,
    axes: *const Axis,
    axes_len: usize,
    vertical: bool,
    out: *mut *mut F64List,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(text) = (unsafe { str_of(text) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let Some(v) = font.caret_positions(text, &location, vertical) else { return Status::Absent };
    unsafe { deliver(out, F64List(v)) }
}

pub const GLYPH_CLASS_BASE: i32 = 0;
pub const GLYPH_CLASS_LIGATURE: i32 = 1;
pub const GLYPH_CLASS_MARK: i32 = 2;
pub const GLYPH_CLASS_COMPONENT: i32 = 3;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_class(
    font: *const Font,
    gid: u16,
    out: *mut i32,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(class) = font.glyph_class(gid) else { return Status::Absent };
    unsafe {
        *out = match class {
            GlyphClass::Base => GLYPH_CLASS_BASE,
            GlyphClass::Ligature => GLYPH_CLASS_LIGATURE,
            GlyphClass::Mark => GLYPH_CLASS_MARK,
            GlyphClass::Component => GLYPH_CLASS_COMPONENT,
        }
    };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_mark_attachment_class(
    font: *const Font,
    gid: u16,
    out: *mut u16,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.mark_attachment_class(gid) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_name(
    font: *const Font,
    gid: u16,
    out: *mut *mut Text,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(name) = font.glyph_name(gid) else { return Status::Absent };
    unsafe { deliver(out, Text::new(&name)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_names(
    font: *const Font,
    out_names: *mut *mut StrList,
    out_present: *mut *mut Blob,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let names = font.glyph_names();
    if !out_names.is_null() {
        let flat: Vec<String> =
            names.iter().map(|o| o.clone().unwrap_or_default()).collect();
        let st = unsafe { deliver(out_names, StrList::new(flat)) };
        if st != Status::Ok {
            return st;
        }
    }
    if !out_present.is_null() {
        let present: Vec<u8> = names.iter().map(|o| u8::from(o.is_some())).collect();
        return unsafe { deliver(out_present, Blob(present)) };
    }
    Status::Ok
}
