// glyphs.h: characters to glyphs, a glyph's box, advances, vertical origin, ligature carets, classes and names.

use core::ffi::c_char;

use crate::ffi::base::{OnNul, StrList};
use crate::ffi::font::LocationHandle;
use crate::ffi::tables::glyphs_of;
use crate::ffi::char_of;
use crate::ffi::handle::{
    Status, apart, borrow, cleared, deliver, into_buffer, into_buffers, or_return, put, put_some, room_for,
    slice_mut_of, slice_of, span, text_of,
};
use crate::{Font, GlyphClass, GlyphId};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_id(font: *const Font, codepoint: u32, out: *mut u32) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let c = or_return!(char_of(codepoint));
    unsafe { put_some(out, font.glyph_id(c).map(|g| g.0)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_has_glyph(font: *const Font, codepoint: u32, out: *mut bool) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let c = or_return!(char_of(codepoint));
    unsafe { put(out, font.has_glyph(c)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_ids(
    font: *const Font,
    text: *const c_char,
    text_len: usize,
    out_gids: *mut u32,
    out_present: *mut u8,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    if out_gids.is_null() && out_len > 0 {
        return Status::Null;
    }
    let text = or_return!(unsafe { text_of(text, text_len) });
    let read = span(text.as_ptr(), text.len());
    or_return!(apart(&[read, span(out_gids, out_len), span(out_present, out_len), span(out_count, 1)]));
    // No more characters than bytes, so a buffer as long as the text is filled as it is read, with no count first.
    let room = if out_gids.is_null() || out_len < text.len() {
        let count = text.chars().count();
        if !or_return!(unsafe { room_for(count, !out_gids.is_null(), out_len, out_count) }) {
            return Status::Ok;
        }
        count
    } else {
        text.len()
    };
    let Some(gids) = (unsafe { slice_mut_of(out_gids, room) }) else { return Status::Null };
    let count = text.chars().zip(gids.iter_mut()).map(|(c, gid)| *gid = font.glyph_id(c).map_or(0, |g| g.0)).count();
    // No glyph found is 0, so presence is read from the ids afterwards, in a pass that vectorizes.
    if let Some(present) = (!out_present.is_null()).then(|| unsafe { slice_mut_of(out_present, count) }).flatten() {
        present.iter_mut().zip(&gids[..count]).for_each(|(p, &gid)| *p = (gid != 0).into());
    }
    unsafe { *out_count = count };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_coverage(
    font: *const Font,
    out_codepoints: *mut u32,
    out_gids: *mut u32,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { into_buffers(font.coverage_codes().into_iter(), out_codepoints, out_gids, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_variation_glyph_id(
    font: *const Font,
    codepoint: u32,
    selector: u32,
    out: *mut u32,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let (c, selector) = (or_return!(char_of(codepoint)), or_return!(char_of(selector)));
    unsafe { put_some(out, font.variation_glyph_id(c, selector).map(|g| g.0)) }
}

// boxes and advances

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_bounds(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    out: *mut crate::Bounds,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { put_some(out, font.glyph_bounds(GlyphId(gid), crate::ffi::at(location))) }
}

// The caller's buffer, as long as the glyphs, each advance written into its slot.
impl crate::api::AdvanceList for [f64] {
    fn fill(&mut self, advances: impl Iterator<Item = f64>) {
        self.iter_mut().zip(advances).for_each(|(slot, advance)| *slot = advance);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_advances(
    font: *const Font,
    gids: *const u32,
    gids_len: usize,
    location: *const LocationHandle,
    orientation: i32,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(gids) = (unsafe { slice_of(gids, gids_len) }) else { return Status::Null };
    or_return!(apart(&[span(gids.as_ptr(), gids_len), span(out, gids_len)]));
    let Some(out) = (unsafe { slice_mut_of(out, gids_len) }) else { return Status::Null };
    let orientation = or_return!(crate::ffi::orientation(orientation));
    let at = unsafe { crate::ffi::at(location) };
    font.advances_into(glyphs_of(gids), at, orientation, out);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_advance(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    orientation: i32,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let orientation = or_return!(crate::ffi::orientation(orientation));
    unsafe { put(out, font.advance(GlyphId(gid), crate::ffi::at(location), orientation)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_vertical_origin(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put(out, font.vertical_origin(GlyphId(gid), crate::ffi::at(location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_default_vertical_origin(font: *const Font, out: *mut f64) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put(out, font.default_vertical_origin()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_ligature_carets(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    orientation: i32,
    out_values: *mut f64,
    out_present: *mut u8,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    let orientation = or_return!(crate::ffi::orientation(orientation));
    let carets = font.ligature_carets(GlyphId(gid), unsafe { crate::ffi::at(location) }, orientation);
    let carets = carets.into_iter().map(|c| (c.unwrap_or(0.0), u8::from(c.is_some())));
    unsafe { into_buffers(carets, out_values, out_present, out_len, out_count) }
}

// classes and names

pub const GLYPH_CLASS_BASE: i32 = 0;
pub const GLYPH_CLASS_LIGATURE: i32 = 1;
pub const GLYPH_CLASS_MARK: i32 = 2;
pub const GLYPH_CLASS_COMPONENT: i32 = 3;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_class(font: *const Font, gid: u32, out: *mut i32) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let class = font.glyph_class(GlyphId(gid)).map(|c| match c {
        GlyphClass::Base => GLYPH_CLASS_BASE,
        GlyphClass::Ligature => GLYPH_CLASS_LIGATURE,
        GlyphClass::Mark => GLYPH_CLASS_MARK,
        GlyphClass::Component => GLYPH_CLASS_COMPONENT,
    });
    unsafe { put_some(out, class) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_mark_attachment_class(font: *const Font, gid: u32, out: *mut u32) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put_some(out, font.mark_attachment_class(GlyphId(gid)).map(core::num::NonZeroU32::get)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_name(
    font: *const Font,
    gid: u32,
    out: *mut c_char,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    let Some(name) = font.glyph_name(GlyphId(gid)).filter(|n| !n.contains('\0')) else { return Status::Absent };
    // Its bytes and the NUL after them, which the buffer makes room for too.
    let bytes = name.as_bytes();
    let with_nul = (0..bytes.len() + 1).map(|i| bytes.get(i).map_or(0, |&b| b as c_char));
    unsafe { into_buffer(with_nul, out, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_names(font: *const Font, out: *mut *mut StrList) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    // No name is empty, so an empty string says none.
    let names = font.glyph_names();
    let mut list = StrList::with_capacity(0, names.size_hint().0);
    names.for_each(|n| list.push_str(n.unwrap_or("")));
    unsafe { deliver(out, list.finish(OnNul::Empty)) }
}

#[cfg(test)]
#[path = "tests/glyphs.rs"]
mod tests;
