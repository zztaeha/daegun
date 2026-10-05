// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use core::mem::offset_of;

use crate::Font;
use crate::api::Unprepared;

use crate::ffi::handle::{Status, borrow};
use crate::ffi::list::{Axis, axes_of};
use crate::ffi::options::OutlineOptionsC;
use crate::ffi::pen::{Pen, PenBridge};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PreparedGlyphC {
    pub advance_width: f32,
    pub advance_height: f32,
    pub hinted: i32,
}

// Mirrored by `_Static_assert` in `daegun.h`; a reordered field passes a size check alone.
const _: () = assert!(size_of::<PreparedGlyphC>() == 12);
const _: () = {
    assert!(offset_of!(PreparedGlyphC, advance_width) == 0);
    assert!(offset_of!(PreparedGlyphC, advance_height) == 4);
    assert!(offset_of!(PreparedGlyphC, hinted) == 8);
};

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments, reason = "the C form of a method that takes six inputs and a pen")]
pub unsafe extern "C" fn daegun_font_prepared_outline(
    font: *const Font,
    gid: u16,
    px: f32,
    axes: *const Axis,
    axes_len: usize,
    opts: *const OutlineOptionsC,
    pen: *const Pen,
    out: *mut PreparedGlyphC,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(pen) = (unsafe { borrow(pen) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let opts = unsafe { borrow(opts) }.copied().unwrap_or(OutlineOptionsC::DEFAULT);
    let finite = |v: &f32| v.is_finite();
    if !(px.is_finite() && px > 0.0)
        || (opts.has_transform != 0 && !opts.transform.iter().all(finite))
        || (opts.has_oblique != 0 && !opts.oblique.is_finite())
        || (opts.has_stroke != 0 && !opts.stroke_width.is_finite())
    {
        return crate::ffi::range("px must be finite and above zero, and the options finite");
    }
    let mut bridge = PenBridge(*pen);
    let prepared = font.prepare(gid, px, &location, &opts.to_rust(), &mut bridge);
    let crate::PreparedGlyph { advance_width, advance_height, hinted } = match prepared {
        Ok(g) => g,
        Err(Unprepared::NoOutline) => return Status::Absent,
        Err(Unprepared::BadInput) => return crate::ffi::range("an advance, width or point overflows once scaled"),
        Err(Unprepared::TooManyPoints) => {
            crate::ffi::set_error("the stroke or embolden would take more than DAEGUN_MAX_FLATTEN_POINTS points");
            return Status::Range;
        }
    };
    if !out.is_null() {
        unsafe { *out = PreparedGlyphC { advance_width, advance_height, hinted: i32::from(hinted) } };
    }
    Status::Ok
}
