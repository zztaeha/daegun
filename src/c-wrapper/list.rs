// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Anything the caller must uphold beyond that is noted at its site.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_char;

use crate::ffi::handle::{OwnedStr, Status, Str, borrow, release};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Axis {
    pub tag: *const c_char,
    pub value: f64,
}

const _: () = assert!(size_of::<Axis>() == size_of::<usize>() + 8);

pub unsafe fn axes_of<'a>(axes: *const Axis, len: usize) -> Option<Vec<(&'a str, f64)>> {
    let slice = unsafe { crate::ffi::handle::slice_of(axes, len) }?;
    let axes = slice
        .iter()
        .filter_map(|a| {
            if a.tag.is_null() {
                return None;
            }
            unsafe { core::ffi::CStr::from_ptr(a.tag) }.to_str().ok().map(|t| (t, a.value))
        })
        .collect();
    Some(axes)
}

macro_rules! pod_list {
    ($name:ident, $elem:ty, $data:ident, $free:ident) => {
        pub struct $name(pub Vec<$elem>);

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $data(
            list: *const $name,
            out_count: *mut usize,
        ) -> *const $elem {
            let Some(list) = (unsafe { borrow(list) }) else { return core::ptr::null() };
            if out_count.is_null() {
                return core::ptr::null();
            }
            unsafe { *out_count = list.0.len() };
            list.0.as_ptr()
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $free(list: *mut $name) {
            unsafe { release(list) }
        }
    };
}

pod_list!(U16List, u16, daegun_u16_list_data, daegun_u16_list_free);
pod_list!(U32List, u32, daegun_u32_list_data, daegun_u32_list_free);
pod_list!(I32List, i32, daegun_i32_list_data, daegun_i32_list_free);
pod_list!(F64List, f64, daegun_f64_list_data, daegun_f64_list_free);
pod_list!(Blob, u8, daegun_blob_data, daegun_blob_free);
pod_list!(UsizeList, usize, daegun_usize_list_data, daegun_usize_list_free);
pod_list!(GlyphValueList, GlyphValue, daegun_glyph_value_list_data, daegun_glyph_value_list_free);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlyphValue {
    pub glyph: u16,
    pub value: u16,
}

const _: () = assert!(size_of::<GlyphValue>() == 4);
const _: () = assert!(align_of::<GlyphValue>() == 2);

pub struct Text(OwnedStr);

impl Text {
    pub fn new(s: &str) -> Text {
        Text(OwnedStr::new(s))
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_text_str(text: *const Text, out: *mut Str) -> Status {
    let Some(text) = (unsafe { borrow(text) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = text.0.as_str() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_text_free(text: *mut Text) {
    unsafe { release(text) }
}

pub struct StrList(Vec<OwnedStr>);

impl StrList {
    pub fn new<I: IntoIterator<Item = String>>(items: I) -> StrList {
        StrList(items.into_iter().map(|s| OwnedStr::new(&s)).collect())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_str_list_count(list: *const StrList, out: *mut usize) -> Status {
    let Some(list) = (unsafe { borrow(list) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = list.0.len() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_str_list_at(
    list: *const StrList,
    index: usize,
    out: *mut Str,
) -> Status {
    let Some(list) = (unsafe { borrow(list) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(item) = list.0.get(index) else { return Status::Range };
    unsafe { *out = item.as_str() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_str_list_free(list: *mut StrList) {
    unsafe { release(list) }
}
