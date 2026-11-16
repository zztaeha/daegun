// The machinery every entry point is built from, so each keeps base.h's rules mechanically: C has no borrow
// checker, so a mistake here is undefined behavior.

use alloc::boxed::Box;
use core::ffi::c_char;

use crate::{Error, ErrorKind};

// The values are frozen, as base.h's: a compiled consumer carries the numbers. New codes append; -5 is never reused.
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    Ok = 0,
    Null = -1,
    Malformed = -2,
    InvalidArgument = -3,
    Absent = -4,
    Unsupported = -6,
    Missing = -7,
    LimitExceeded = -8,
    NoOutline = -9,
    Overflow = -10,
}

// A failure's status, its message left for daegun_last_error.
#[cold]
pub fn failed(e: Error) -> Status {
    crate::ffi::set_error(e.message());
    match e.kind() {
        ErrorKind::Malformed => Status::Malformed,
        ErrorKind::Unsupported => Status::Unsupported,
        ErrorKind::Missing => Status::Missing,
        ErrorKind::InvalidArgument => Status::InvalidArgument,
        ErrorKind::LimitExceeded => Status::LimitExceeded,
        ErrorKind::NoOutline => Status::NoOutline,
        ErrorKind::Overflow => Status::Overflow,
    }
}

// DAEGUN_INVALID_ARGUMENT with the reason.
#[cold]
pub fn invalid(reason: impl core::fmt::Display) -> Status {
    crate::ffi::set_error(reason);
    Status::InvalidArgument
}

#[cold]
pub fn past_end(what: &str, index: usize, count: usize) -> Status {
    invalid(format_args!("{what} {index} is past the end, as there are {count}"))
}

#[cold]
pub fn too_small(out_len: usize, count: usize) -> Status {
    invalid(format_args!("the buffer holds {out_len}, and {count} are needed"))
}

// Rule 5 lets a caller share a handle across threads and free it on another, so each must be Send and Sync.
#[inline]
pub unsafe fn deliver<T: Send + Sync>(out: *mut *mut T, value: T) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = new(value) };
    Status::Ok
}

#[inline]
pub fn new<T: Send + Sync>(value: T) -> *mut T {
    Box::into_raw(Box::new(value))
}

// A handle out, NULL until the call delivers into it: rule 1 promises NULL on every failure.
#[inline]
pub unsafe fn cleared<T>(out: *mut *mut T) -> Option<*mut *mut T> {
    (!out.is_null()).then(|| {
        unsafe { *out = core::ptr::null_mut() };
        out
    })
}

// A result delivered into a handle out, or its failure's status.
#[inline]
pub unsafe fn deliver_result<T: Send + Sync>(out: *mut *mut T, value: crate::Result<T>) -> Status {
    match value {
        Ok(value) => unsafe { deliver(out, value) },
        Err(e) => failed(e),
    }
}

// An out the call must have. `value` is worked out before `out` is looked at, so a call whose value can fail
// or takes real work checks `out` first itself.
#[inline]
pub unsafe fn put<T>(out: *mut T, value: T) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { out.write(value) };
    Status::Ok
}

// The same for a value that may be missing: DAEGUN_ABSENT for None, with the out left as it was.
#[inline]
pub unsafe fn put_some<T>(out: *mut T, value: Option<T>) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(value) = value else { return Status::Absent };
    unsafe { out.write(value) };
    Status::Ok
}

// An out the call may go without, skipped for NULL.
#[inline]
pub unsafe fn fill<T>(out: *mut T, value: T) {
    if !out.is_null() {
        unsafe { out.write(value) };
    }
}

// Tolerating NULL is a safety property: a caller freeing in a cleanup path after a failed call would otherwise
// guard every free, and the guard it forgets is a crash.
#[inline]
pub unsafe fn release<T>(handle: *mut T) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

// A pointer and a count: empty for a count of 0 whatever the pointer, and None for NULL with a count, which
// rule 2 answers DAEGUN_NULL rather than reading as nothing.
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
pub unsafe fn slice_mut_of<'a, T>(data: *mut T, len: usize) -> Option<&'a mut [T]> {
    if len == 0 {
        return Some(&mut []);
    }
    if data.is_null() {
        return None;
    }
    Some(unsafe { core::slice::from_raw_parts_mut(data, len) })
}

// The value a conversion gives, or the status it answered, returned from the entry point.
macro_rules! or_return {
    ($e:expr) => {
        match $e {
            Ok(value) => value,
            Err(status) => return status,
        }
    };
}
pub(crate) use or_return;

// Text as base.h's TEXT gives it: UTF-8 and a count of its bytes. DAEGUN_NULL for NULL with a count, and
// DAEGUN_INVALID_ARGUMENT, naming the byte, where it stops being UTF-8.
#[inline]
pub unsafe fn text_of<'a>(data: *const c_char, len: usize) -> Result<&'a str, Status> {
    let bytes = unsafe { slice_of(data.cast::<u8>(), len) }.ok_or(Status::Null)?;
    core::str::from_utf8(bytes).map_err(not_utf8)
}

#[cold]
pub fn not_utf8(e: core::str::Utf8Error) -> Status {
    invalid(format_args!("text is not UTF-8 past byte {}", e.valid_up_to()))
}

// A NUL-terminated identifier the call may go without, NULL being none.
#[inline]
pub unsafe fn ident_of<'a>(s: *const c_char) -> Result<Option<&'a str>, Status> {
    if s.is_null() {
        return Ok(None);
    }
    unsafe { core::ffi::CStr::from_ptr(s) }.to_str().map(Some).map_err(not_utf8)
}

#[inline]
pub unsafe fn borrow<'a, T>(handle: *const T) -> Option<&'a T> {
    unsafe { handle.as_ref() }
}

#[inline]
pub unsafe fn borrow_mut<'a, T>(handle: *mut T) -> Option<&'a mut T> {
    unsafe { handle.as_mut() }
}

// A C array whose length the header fixes, None for NULL.
#[inline]
pub unsafe fn array_of<T: Copy, const N: usize>(p: *const T) -> Option<[T; N]> {
    (!p.is_null()).then(|| unsafe { p.cast::<[T; N]>().read() })
}

// A borrowed view: the items `of` finds in the handle, with their count. NULL, the count left as it was, for a
// NULL handle or count, or where `of` finds none.
pub unsafe fn view<'a, H: 'a, T: 'a>(
    handle: *const H,
    out_count: *mut usize,
    of: impl FnOnce(&'a H) -> Option<&'a [T]>,
) -> *const T {
    let Some(items) = unsafe { borrow(handle) }.and_then(of) else { return core::ptr::null() };
    if out_count.is_null() {
        return core::ptr::null();
    }
    unsafe { *out_count = items.len() };
    items.as_ptr()
}

// An exported view of one field of a handle.
macro_rules! field_view {
    ($name:ident, $handle:ty, $field:tt, $elem:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(handle: *const $handle, out_count: *mut usize) -> *const $elem {
            unsafe { crate::ffi::handle::view(handle, out_count, |h| Some(&h.$field[..])) }
        }
    };
}
pub(crate) use field_view;

// A handle the thread that freed it keeps, emptied with its lists' room kept, for its next of the kind: a caller
// building and freeing one in a loop allocates nothing, as Rust's paint graphs reuse their lists.
pub(crate) trait Reused: Sized + 'static {
    fn empty() -> Self;
    // Emptied, its lists' room kept; the bytes that room holds.
    fn emptied(&mut self) -> usize;
    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<Box<Self>>>>;
}

// One holding more is dropped rather than kept, as the core's spares are.
pub(crate) const SPARE_BYTES: usize = 64 << 10;

pub(crate) fn drop_spare<T: Reused>() {
    let _ = T::spare().try_with(core::cell::Cell::take);
}

pub(crate) fn reused<T: Reused>() -> Box<T> {
    T::spare().try_with(core::cell::Cell::take).ok().flatten().unwrap_or_else(|| Box::new(T::empty()))
}

// Built with `--cfg daegun_no_spares`, each goes back to the allocator, so a sanitizer sees a use after its free.
pub(crate) fn keep_spare<T: Reused>(mut handle: Box<T>) {
    if cfg!(not(daegun_no_spares)) && handle.emptied() <= SPARE_BYTES {
        let _ = T::spare().try_with(|slot| slot.set(Some(handle)));
    }
}

// A reused handle's free.
pub(crate) unsafe fn give_back<T: Reused>(handle: *mut T) {
    if !handle.is_null() {
        keep_spare(unsafe { Box::from_raw(handle) });
    }
}

// A value handed out in the box its kind's last freed one left, as `deliver` hands one out in a new box.
pub(crate) unsafe fn deliver_reused<T: Reused + Send + Sync>(out: *mut *mut T, value: T) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let mut handle = reused::<T>();
    *handle = value;
    unsafe { *out = Box::into_raw(handle) };
    Status::Ok
}

// A handle whose value each call makes afresh, and its free: the box is kept for the thread's next, holding
// `$empty`, as a new box cost C's small calls 10 to 17 ns.
macro_rules! reused_box {
    ($free:ident, $handle:ty, $empty:expr) => {
        impl crate::ffi::handle::Reused for $handle {
            fn empty() -> $handle {
                $empty
            }

            fn emptied(&mut self) -> usize {
                *self = $empty;
                0
            }

            fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<alloc::boxed::Box<$handle>>>> {
                std::thread_local! {
                    static SPARE: core::cell::Cell<Option<alloc::boxed::Box<$handle>>> =
                        const { core::cell::Cell::new(None) };
                }
                &SPARE
            }
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $free(handle: *mut $handle) {
            unsafe { crate::ffi::handle::give_back(handle) }
        }
    };
}
pub(crate) use reused_box;

// The free of a handle type.
macro_rules! free {
    ($name:ident, $handle:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(handle: *mut $handle) {
            unsafe { crate::ffi::handle::release(handle) }
        }
    };
}
pub(crate) use free;

// The bytes `len` items at `p` take, none for NULL, so `apart` can tell a call's buffers apart.
pub fn span<T>(p: *const T, len: usize) -> (usize, usize) {
    (p.addr(), if p.is_null() { 0 } else { len.saturating_mul(size_of::<T>()) })
}

// BUFFERS: what a call reads and what it writes share no byte, as Rust's borrows of them can't. Checked before
// anything is written, the first span what's read and the rest written.
pub fn apart(spans: &[(usize, usize)]) -> Result<(), Status> {
    let meet = |(a, n): (usize, usize), (b, m): (usize, usize)| {
        n > 0 && m > 0 && a < b.saturating_add(m) && b < a.saturating_add(n)
    };
    match spans.iter().enumerate().any(|(i, &x)| spans[i + 1..].iter().any(|&y| meet(x, y))) {
        true => Err(invalid("a buffer the call writes shares bytes with one it reads or writes")),
        false => Ok(()),
    }
}

// Whether a result of `count` items goes into the caller's buffers, as base.h's BUFFERS says: the count set, none
// given asking for it alone, and too little room answering DAEGUN_INVALID_ARGUMENT.
pub unsafe fn room_for(count: usize, given: bool, out_len: usize, out_count: *mut usize) -> Result<bool, Status> {
    if out_count.is_null() {
        return Err(Status::Null);
    }
    unsafe { *out_count = count };
    if !given {
        return Ok(false);
    }
    if count > out_len {
        return Err(too_small(out_len, count));
    }
    Ok(true)
}

// The results a caller's buffer takes, as `room_for` sets out.
pub unsafe fn into_buffer<T>(
    items: impl ExactSizeIterator<Item = T>,
    out: *mut T,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    // Rule 2: a buffer with room is required, and a failure leaves the count as it was.
    if out.is_null() && out_len > 0 {
        return Status::Null;
    }
    or_return!(apart(&[span(out_count, 1), span(out, out_len)]));
    let count = items.len();
    if !or_return!(unsafe { room_for(count, !out.is_null(), out_len, out_count) }) {
        return Status::Ok;
    }
    let Some(dst) = (unsafe { slice_mut_of(out, count) }) else { return Status::Null };
    for (slot, item) in dst.iter_mut().zip(items) {
        *slot = item;
    }
    Status::Ok
}

// Two results of one count into the caller's buffers, either NULL to leave its half out, as `room_for` sets out.
pub unsafe fn into_buffers<A, B>(
    items: impl ExactSizeIterator<Item = (A, B)>,
    out_a: *mut A,
    out_b: *mut B,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    or_return!(apart(&[span(out_count, 1), span(out_a, out_len), span(out_b, out_len)]));
    let count = items.len();
    if !or_return!(unsafe { room_for(count, !out_a.is_null() || !out_b.is_null(), out_len, out_count) }) {
        return Status::Ok;
    }
    let (mut a, mut b) = unsafe { (opt_slice_mut(out_a, count), opt_slice_mut(out_b, count)) };
    for (i, (x, y)) in items.enumerate() {
        if let Some(a) = a.as_deref_mut() {
            a[i] = x;
        }
        if let Some(b) = b.as_deref_mut() {
            b[i] = y;
        }
    }
    Status::Ok
}

// A buffer the caller may leave out, NULL being none.
unsafe fn opt_slice_mut<'a, T>(data: *mut T, len: usize) -> Option<&'a mut [T]> {
    if data.is_null() { None } else { unsafe { slice_mut_of(data, len) } }
}

// An options struct's mirror: 2.0's size, and its defaults set on a zeroed struct, which keeps every byte between
// fields zero, as base.h promises a _default leaves them. Every field reads soundly from any bytes: bools are u8.
pub trait Options: Copy {
    // A struct grown past 2.0 sets this to the size it had then, so a 2.0 caller's struct still reads. A field added
    // goes at the end and grows `sizeof`: in padding, it would read what an older caller left there.
    const FIRST: usize = size_of::<Self>();

    // Only fields whose default isn't zero, each written in place.
    unsafe fn set_defaults(p: *mut Self);

    // The defaults whole, their size set, read where a call is given NULL or a struct older than this one.
    fn defaults() -> Self {
        let mut zeroed = core::mem::MaybeUninit::<Self>::zeroed();
        unsafe {
            Self::set_defaults(zeroed.as_mut_ptr());
            zeroed.as_mut_ptr().cast::<usize>().write(size_of::<Self>());
            zeroed.assume_init()
        }
    }
}

// An options struct as base.h's OPTIONS reads it: its first `size` bytes over the defaults, NULL being the
// defaults. Below 2.0's size answers DAEGUN_INVALID_ARGUMENT, and past what this library knows DAEGUN_UNSUPPORTED.
pub unsafe fn options_of<T: Options>(opts: *const T) -> Result<T, Status> {
    if opts.is_null() {
        return Ok(T::defaults());
    }
    let size = unsafe { opts.cast::<usize>().read() };
    // A struct of this header's size, every caller's in 2.0, read whole: checked and copied by `memcpy`, it took
    // C's justify 5 ns over Rust's.
    if size == size_of::<T>() {
        return Ok(unsafe { opts.read() });
    }
    sized::<T>(size)?;
    let mut out = T::defaults();
    unsafe { core::ptr::copy_nonoverlapping(opts.cast::<u8>(), (&raw mut out).cast::<u8>(), size) };
    Ok(out)
}

// The defaults written into the caller's struct of `size` bytes, its size set, as each _default does.
pub unsafe fn defaults_into<T: Options>(out: *mut T, size: usize) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    if let Err(status) = sized::<T>(size) {
        return status;
    }
    // Zeroed where it lies and its defaults set there: moved by value, even as `MaybeUninit::zeroed`'s result, a
    // struct leaves its padding behind. An older caller's smaller struct takes the first `size` bytes of a whole one.
    unsafe {
        if size == size_of::<T>() {
            out.write_bytes(0, 1);
            T::set_defaults(out);
        } else {
            let mut whole = core::mem::MaybeUninit::<T>::uninit();
            whole.as_mut_ptr().write_bytes(0, 1);
            T::set_defaults(whole.as_mut_ptr());
            core::ptr::copy_nonoverlapping(whole.as_ptr().cast::<u8>(), out.cast::<u8>(), size);
        }
        out.cast::<usize>().write(size);
    }
    Status::Ok
}

fn sized<T: Options>(size: usize) -> Result<(), Status> {
    if size < T::FIRST {
        return Err(invalid(format_args!("an options struct of {size} bytes is below 2.0's {}", T::FIRST)));
    }
    if size > size_of::<T>() {
        let known = size_of::<T>();
        crate::ffi::set_error(format_args!("an options struct of {size} bytes is past this library's {known}"));
        return Err(Status::Unsupported);
    }
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Bytes {
    pub data: *const u8,
    pub len: usize,
}

const _: () = assert!(size_of::<Bytes>() == 2 * size_of::<usize>());

impl Bytes {
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

impl Str {
    pub const EMPTY: Str = Str { data: c"".as_ptr(), len: 0 };

    // A static string's view, which a NUL-terminated literal gives.
    pub fn of_static(s: &'static core::ffi::CStr) -> Str {
        Str { data: s.as_ptr(), len: s.count_bytes() }
    }
}

pub struct OwnedStr(alloc::ffi::CString);

impl OwnedStr {
    // Interior NULs are replaced rather than refused, so a font with an odd name reads; `len` still describes
    // the whole string.
    pub fn new(s: &str) -> OwnedStr {
        let cleaned = match s.contains('\0') {
            true => alloc::borrow::Cow::Owned(s.replace('\0', "\u{fffd}")),
            false => alloc::borrow::Cow::Borrowed(s),
        };
        let mut bytes = alloc::vec::Vec::with_capacity(cleaned.len() + 1);
        bytes.extend_from_slice(cleaned.as_bytes());
        OwnedStr(alloc::ffi::CString::new(bytes).unwrap_or_default())
    }

    // Rust's string moved in, its NUL in the byte the core's names spare, so nothing is copied.
    pub fn from_string(s: alloc::string::String) -> OwnedStr {
        match alloc::ffi::CString::new(s) {
            Ok(s) => OwnedStr(s),
            Err(held) => OwnedStr::new(&alloc::string::String::from_utf8_lossy(&held.into_vec())),
        }
    }

    pub fn into_raw(self) -> *mut c_char {
        self.0.into_raw()
    }
}
