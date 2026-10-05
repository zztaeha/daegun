// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use crate::SubpixelLayout;
use crate::daecore::daemachine::subpixel::MAX_TAPS;

use crate::ffi::handle::{Status, borrow, deliver, release};
use crate::ffi::options::layout_of;

pub struct Layout(SubpixelLayout);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_key(layout: i32, out: *mut u64) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = layout_of(layout).key() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_new(layout: i32, out: *mut *mut Layout) -> Status {
    unsafe { deliver(out, Layout(layout_of(layout))) }
}

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments, reason = "the C form of from_weights, whose tuples are flattened")]
pub unsafe extern "C" fn daegun_subpixel_layout_from_weights(
    oversample_x: u8,
    oversample_y: u8,
    taps_x: u8,
    taps_y: u8,
    origin_x: i8,
    origin_y: i8,
    weights: *const f32,
    out: *mut *mut Layout,
) -> Status {
    if weights.is_null() || out.is_null() {
        return Status::Null;
    }
    // Checked before the buffer is read: the caller sized it for the taps it meant, not for 255 by 255.
    let per = usize::from(taps_x) * usize::from(taps_y);
    if per == 0 || usize::from(taps_x) > MAX_TAPS || usize::from(taps_y) > MAX_TAPS {
        return crate::ffi::range("taps of zero or past DAEGUN_MAX_SUBPIXEL_TAPS");
    }
    let all = unsafe { core::slice::from_raw_parts(weights, per * 3) };
    let rows = [&all[..per], &all[per..per * 2], &all[per * 2..]];
    match SubpixelLayout::from_weights(
        (oversample_x, oversample_y),
        (taps_x, taps_y),
        (origin_x, origin_y),
        rows,
    ) {
        Some(l) => unsafe { deliver(out, Layout(l)) },
        None => crate::ffi::range("an oversample of zero or past DAEGUN_MAX_OVERSAMPLE, or a weight not finite"),
    }
}

macro_rules! pair {
    ($fn_name:ident, $method:ident, $t:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(
            layout: *const Layout,
            out_x: *mut $t,
            out_y: *mut $t,
        ) -> Status {
            let Some(l) = (unsafe { borrow(layout) }) else { return Status::Null };
            if out_x.is_null() || out_y.is_null() {
                return Status::Null;
            }
            let (x, y) = l.0.$method();
            unsafe {
                *out_x = x;
                *out_y = y;
            }
            Status::Ok
        }
    };
}

pair!(daegun_subpixel_layout_oversample, oversample, u8);
pair!(daegun_subpixel_layout_taps, taps, u8);
pair!(daegun_subpixel_layout_origin, origin, i8);
pair!(daegun_subpixel_layout_pad, pad, usize);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_channels(layout: *const Layout, out: *mut u8) -> Status {
    let Some(l) = (unsafe { borrow(layout) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = l.0.channels() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_is_grayscale(
    layout: *const Layout,
    out: *mut i32,
) -> Status {
    let Some(l) = (unsafe { borrow(layout) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = i32::from(l.0.is_grayscale()) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_cache_key(
    layout: *const Layout,
    out: *mut u64,
) -> Status {
    let Some(l) = (unsafe { borrow(layout) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = l.0.key() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_weights(
    layout: *const Layout,
    channel: usize,
    out_count: *mut usize,
) -> *const f32 {
    let Some(l) = (unsafe { borrow(layout) }) else { return core::ptr::null() };
    if out_count.is_null() {
        return core::ptr::null();
    }
    let Some(w) = l.0.weights(channel) else { return core::ptr::null() };
    unsafe { *out_count = w.len() };
    w.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_free(layout: *mut Layout) {
    unsafe { release(layout) }
}
