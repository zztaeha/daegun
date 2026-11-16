// The C ABI of `include/daegun/`, a file a header. SAFETY: each `unsafe` block rests on a NULL check above it or
// in the `handle.rs` helper it calls; a pointer's count is the caller's promise, which nothing here can check.

#![allow(unsafe_code)]

mod handle;
mod base;
mod color;
mod font;
mod glyphs;
mod layout;
mod math;
mod metrics;
mod outline;
mod raster;
mod raw;
mod shape;
mod tables;
mod unicode;

use crate::{Location, Tag};
use alloc::vec::Vec;
use core::cell::RefCell;
use handle::Str;

const fn abi(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 16) | (minor << 8) | patch
}

// The C ABI's own version, not the crate's; abi-parity.py holds it to the last release's.
const ABI_VERSION: u32 = abi(2, 0, 0);

#[unsafe(no_mangle)]
pub extern "C" fn daegun_abi_version() -> u32 {
    ABI_VERSION
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_version() -> Str {
    Str { data: concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast(), len: env!("CARGO_PKG_VERSION").len() }
}

std::thread_local! {
    // The reason's bytes and a NUL, empty until a failure; kept, so a failure allocates only to grow it.
    static LAST_ERROR: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

// A reason written into the slot, a NUL in it as U+FFFD, as C would read the string as ending there.
struct Reason<'a>(&'a mut Vec<u8>);

impl core::fmt::Write for Reason<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let mut parts = s.split('\0');
        self.0.extend_from_slice(parts.next().unwrap_or_default().as_bytes());
        for part in parts {
            self.0.extend_from_slice("\u{fffd}".as_bytes());
            self.0.extend_from_slice(part.as_bytes());
        }
        Ok(())
    }
}

// An atexit handler or a thread-exit destructor can run after the thread's storage is gone, where `with` would
// panic: the reason is then dropped.
#[cold]
pub(crate) fn set_error(reason: impl core::fmt::Display) {
    let _ = LAST_ERROR.try_with(|slot| {
        let mut bytes = slot.borrow_mut();
        bytes.clear();
        let _ = core::fmt::write(&mut Reason(&mut bytes), format_args!("{reason}"));
        bytes.push(0);
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_last_error() -> Str {
    // The pointer is the whole buffer's, so C may read the NUL after the reason.
    let reason = |slot: &RefCell<Vec<u8>>| match slot.borrow().as_slice() {
        [] => Str::EMPTY,
        whole => Str { data: whole.as_ptr().cast(), len: whole.len() - 1 },
    };
    LAST_ERROR.try_with(reason).unwrap_or(Str::EMPTY)
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_thread_release() {
    crate::release_thread();
    base::release_thread();
    color::release_thread();
    font::release_thread();
    math::release_thread();
    outline::release_thread();
    raster::release_thread();
    raw::release_thread();
    unicode::release_thread();
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_status_name(status: i32) -> Str {
    Str::of_static(match status {
        0 => c"DAEGUN_OK",
        -1 => c"DAEGUN_NULL",
        -2 => c"DAEGUN_MALFORMED",
        -3 => c"DAEGUN_INVALID_ARGUMENT",
        -4 => c"DAEGUN_ABSENT",
        -6 => c"DAEGUN_UNSUPPORTED",
        -7 => c"DAEGUN_MISSING",
        -8 => c"DAEGUN_LIMIT_EXCEEDED",
        -9 => c"DAEGUN_NO_OUTLINE",
        -10 => c"DAEGUN_OVERFLOW",
        _ => c"unknown",
    })
}

static DEFAULT: Location = Location::DEFAULT;

// The location a call is at: NULL is the default instance.
pub(crate) unsafe fn at<'a>(location: *const font::LocationHandle) -> &'a Location {
    unsafe { location.as_ref() }.map_or(&DEFAULT, |l| &l.location)
}

// A tag the call may go without, 0 being none.
pub(crate) fn opt_tag(tag: u32) -> Option<Tag> {
    (tag != 0).then_some(Tag(tag))
}

// A code point as Rust's char: a surrogate or one past U+10FFFF answers DAEGUN_INVALID_ARGUMENT.
pub(crate) fn char_of(codepoint: u32) -> Result<char, handle::Status> {
    char::from_u32(codepoint)
        .ok_or_else(|| handle::invalid(format_args!("U+{codepoint:04X} is not a Unicode scalar value")))
}

pub const BASE_AUTO: i32 = -1;
pub const BASE_LTR: i32 = 0;
pub const BASE_RTL: i32 = 1;

// A paragraph's base direction, as shape.h's DAEGUN_BASE_*.
pub(crate) fn base_direction(code: i32) -> Result<crate::unicode::BaseDirection, handle::Status> {
    use crate::unicode::BaseDirection;
    match code {
        BASE_AUTO => Ok(BaseDirection::Auto),
        BASE_LTR => Ok(BaseDirection::LeftToRight),
        BASE_RTL => Ok(BaseDirection::RightToLeft),
        _ => Err(handle::invalid(format_args!("base direction {code} is none of DAEGUN_BASE_*"))),
    }
}

pub const HORIZONTAL: i32 = 0;
pub const VERTICAL: i32 = 1;

// An orientation code, as base.h's DAEGUN_HORIZONTAL and DAEGUN_VERTICAL.
pub(crate) fn orientation(code: i32) -> Result<crate::Orientation, handle::Status> {
    match code {
        HORIZONTAL => Ok(crate::Orientation::Horizontal),
        VERTICAL => Ok(crate::Orientation::Vertical),
        _ => Err(handle::invalid(format_args!("orientation {code} is neither DAEGUN_HORIZONTAL nor DAEGUN_VERTICAL"))),
    }
}

#[cfg(test)]
#[path = "tests/support.rs"]
pub(crate) mod testing;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
