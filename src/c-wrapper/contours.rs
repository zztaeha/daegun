// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them.

use alloc::vec::Vec;

use crate::ffi::handle::{Status, borrow, deliver, release};
use crate::ffi::outline::Path;

// Arrays rather than the Rust side's tuples, because only an array has a layout C may rely on.
pub struct Contours(Vec<Vec<[f32; 2]>>);

impl Contours {
    fn of(contours: Vec<Vec<(f32, f32)>>) -> Contours {
        Contours(contours.into_iter().map(|c| c.into_iter().map(|(x, y)| [x, y]).collect()).collect())
    }

    fn tuples(&self) -> Vec<Vec<(f32, f32)>> {
        self.0.iter().map(|c| c.iter().map(|p| (p[0], p[1])).collect()).collect()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_flatten_max_area_for(
    px: f32,
    units_per_em: f32,
    out: *mut f32,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    if !(px.is_finite() && px > 0.0 && units_per_em.is_finite() && units_per_em > 0.0) {
        return crate::ffi::range("px and units_per_em must be finite and above zero");
    }
    unsafe { *out = crate::max_area_for(px, units_per_em) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_flatten(
    path: *const Path,
    max_area: f32,
    out: *mut *mut Contours,
) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(contours) = crate::flatten(&path.0, max_area) else {
        crate::ffi::set_error(
            "a point not finite or past FLT_MAX / 16, or past DAEGUN_MAX_FLATTEN_POINTS points kept or twice that read",
        );
        return Status::Range;
    };
    if contours.is_empty() {
        return Status::Absent;
    }
    unsafe { deliver(out, Contours::of(contours)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_contours_count(contours: *const Contours, out: *mut usize) -> Status {
    let Some(c) = (unsafe { borrow(contours) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = c.0.len() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_contours_points(
    contours: *const Contours,
    index: usize,
    out_count: *mut usize,
) -> *const f32 {
    let Some(c) = (unsafe { borrow(contours) }) else { return core::ptr::null() };
    if out_count.is_null() {
        return core::ptr::null();
    }
    let Some(points) = c.0.get(index) else { return core::ptr::null() };
    unsafe { *out_count = points.len() };
    points.as_ptr().cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_contours_resolve_overlaps(
    contours: *const Contours,
    out: *mut *mut Contours,
) -> Status {
    let Some(c) = (unsafe { borrow(contours) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    match crate::resolve_overlaps(&c.tuples()) {
        Some(resolved) => unsafe { deliver(out, Contours::of(resolved)) },
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_contours_free(contours: *mut Contours) {
    unsafe { release(contours) }
}
