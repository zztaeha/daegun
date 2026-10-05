// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use alloc::vec::Vec;

use crate::{Font, Quad, QuadError, QuadraticPen};

use crate::ffi::handle::{Status, borrow, deliver, release};
use crate::ffi::list::{Axis, axes_of};
use crate::ffi::outline::Path;

pub struct Quads(Vec<Quad>);

// `daegun_quads_data` hands the curves out as one run of floats, six per curve.
const _: () = assert!(size_of::<Quad>() == 6 * size_of::<f32>());
const _: () = assert!(align_of::<Quad>() == align_of::<f32>());

unsafe fn answer(built: Result<Vec<Quad>, QuadError>, out: *mut *mut Quads) -> Status {
    match built {
        Ok(curves) => unsafe { deliver(out, Quads(curves)) },
        Err(QuadError::NoOutline) => Status::Absent,
        Err(e) => {
            crate::ffi::set_error(&alloc::format!("{e}"));
            Status::Range
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_quads(
    font: *const Font,
    gid: u16,
    axes: *const Axis,
    axes_len: usize,
    out: *mut *mut Quads,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    unsafe { answer(font.glyph_quads(gid, &location), out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_quads(
    path: *const Path,
    units_per_em: f32,
    out: *mut *mut Quads,
) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let mut pen = QuadraticPen::new(units_per_em);
    path.0.replay(None, &mut pen);
    unsafe { answer(pen.finish(), out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_quads_normalize_winding(quads: *mut Quads) -> Status {
    if quads.is_null() {
        return Status::Null;
    }
    crate::normalize_winding(unsafe { &mut (*quads).0 });
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_quads_data(quads: *const Quads, out_count: *mut usize) -> *const f32 {
    let Some(q) = (unsafe { borrow(quads) }) else { return core::ptr::null() };
    if out_count.is_null() {
        return core::ptr::null();
    }
    unsafe { *out_count = q.0.len() };
    q.0.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_quads_free(quads: *mut Quads) {
    unsafe { release(quads) }
}
