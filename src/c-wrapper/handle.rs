// The machinery every entry point is built from, so each keeps daegun.h's rules mechanically: C has
// no borrow checker, so a mistake here is undefined behavior. A Rust struct is built or taken apart
// with every field named, never `..`, and `_` for one a sibling call carries, so a field Rust gains
// does not build until C carries it too.

use alloc::boxed::Box;
use core::ffi::c_char;

// The values are frozen: a C caller compares against the constants in `daegun.h`, so renumbering
// silently breaks every compiled consumer rather than failing to build. New codes append.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Ok = 0,
    Null = -1,
    Parse = -2,
    Range = -3,
    Absent = -4,
    // -5 was Unsupported, which only the GPU backends returned. Never reuse it.
}

#[inline]
pub unsafe fn deliver<T>(out: *mut *mut T, value: T) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = Box::into_raw(Box::new(value)) };
    Status::Ok
}

// Tolerating null is a safety property, not politeness: a caller freeing in a cleanup path after a
// failed open would otherwise guard every call, and the guard it forgets is a crash.
#[inline]
pub unsafe fn release<T>(handle: *mut T) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

// A pointer and a count: empty for a count of 0 whatever the pointer, and None for NULL with a count,
// which rule 2 answers DAEGUN_NULL rather than reading as nothing.
#[inline]
pub unsafe fn slice_of<'a, T>(data: *const T, len: usize) -> Option<&'a [T]> {
    if len == 0 {
        return Some(&[]);
    }
    if data.is_null() {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts(data, len) })
}

#[inline]
pub unsafe fn str_of<'a>(s: *const c_char) -> Option<&'a str> {
    if s.is_null() {
        return None;
    }
    unsafe { core::ffi::CStr::from_ptr(s) }.to_str().ok()
}

#[inline]
pub unsafe fn borrow<'a, T>(handle: *const T) -> Option<&'a T> {
    if handle.is_null() {
        None
    } else {
        Some(unsafe { &*handle })
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bytes {
    pub data: *const u8,
    pub len: usize,
}

const _: () = assert!(size_of::<Bytes>() == 2 * size_of::<usize>());
const _: () = assert!(align_of::<Bytes>() == align_of::<usize>());

impl Bytes {
    pub const EMPTY: Bytes = Bytes { data: core::ptr::null(), len: 0 };

    pub fn of(slice: &[u8]) -> Bytes {
        Bytes { data: slice.as_ptr(), len: slice.len() }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Str {
    pub data: *const c_char,
    pub len: usize,
}

const _: () = assert!(size_of::<Str>() == 2 * size_of::<usize>());
const _: () = assert!(align_of::<Str>() == align_of::<usize>());

impl Str {
    pub const EMPTY: Str = Str { data: c"".as_ptr(), len: 0 };
}

pub struct OwnedStr(alloc::ffi::CString);

impl OwnedStr {
    // Interior NULs are replaced rather than refused – `CString::new` rejects them, which would turn
    // a font with an odd name into a failed call. `len` still describes the whole string.
    pub fn new(s: &str) -> OwnedStr {
        let cleaned: alloc::string::String =
            s.chars().map(|c| if c == '\0' { '\u{fffd}' } else { c }).collect();
        OwnedStr(alloc::ffi::CString::new(cleaned).unwrap_or_default())
    }

    pub fn as_str(&self) -> Str {
        let bytes = self.0.as_bytes();
        Str { data: self.0.as_ptr(), len: bytes.len() }
    }
}
