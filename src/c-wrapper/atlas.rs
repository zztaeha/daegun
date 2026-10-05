// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Anything the caller must uphold beyond that is noted at its site.

use crate::{Rect, ShelfPacker};

use crate::ffi::handle::{Status, release};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RectC {
    pub x: usize,
    pub y: usize,
    pub w: usize,
    pub h: usize,
}

const _: () = assert!(size_of::<RectC>() == 4 * size_of::<usize>());
const _: () = assert!(align_of::<RectC>() == align_of::<usize>());

#[unsafe(no_mangle)]
pub extern "C" fn daegun_shelf_packer_new(width: usize, height: usize) -> *mut ShelfPacker {
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(ShelfPacker::new(width, height)))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shelf_packer_insert(
    packer: *mut ShelfPacker,
    width: usize,
    height: usize,
    out: *mut RectC,
) -> Status {
    if packer.is_null() || out.is_null() {
        return Status::Null;
    }
    let packer = unsafe { &mut *packer };
    let Some(Rect { x, y, w, h }) = packer.insert(width, height) else { return Status::Absent };
    unsafe { *out = RectC { x, y, w, h } };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shelf_packer_reset(packer: *mut ShelfPacker) -> Status {
    if packer.is_null() {
        return Status::Null;
    }
    unsafe { &mut *packer }.reset();
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shelf_packer_free(packer: *mut ShelfPacker) {
    unsafe { release(packer) }
}
