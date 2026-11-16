// base.h's owned results: bytes, strings, and one owned string.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_char;

use crate::ffi::handle::{OwnedStr, Status, Str, borrow, free, invalid, or_return, past_end, put, reused_box, text_of};

// A run of bytes: one a call built, or an instance its location's cache holds, lent rather than copied.
pub enum Blob {
    Owned(Vec<u8>),
    Shared(Arc<[u8]>),
}

impl core::ops::Deref for Blob {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Blob::Owned(bytes) => bytes,
            Blob::Shared(bytes) => bytes,
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_tag_from_string(text: *const c_char, text_len: usize, out: *mut u32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let text = or_return!(unsafe { text_of(text, text_len) });
    match text.parse::<crate::Tag>() {
        Ok(tag) => unsafe { put(out, tag.0) },
        Err(e) => invalid(e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_blob_data(blob: *const Blob, out_count: *mut usize) -> *const u8 {
    unsafe { crate::ffi::handle::view(blob, out_count, |b| Some(&b[..])) }
}

reused_box!(daegun_blob_free, Blob, Blob::Owned(Vec::new()));

pub use crate::raw::format::GlyphValue;

const _: () = assert!(size_of::<GlyphValue>() == 8);

// One owned string, its handle the string's own bytes rather than a box around them: with no NUL inside, its length
// is found again from them, as `CString::from_raw` finds it.
pub struct Text {
    _bytes: [u8; 0],
}

impl Text {
    pub fn new(s: alloc::string::String) -> *mut Text {
        OwnedStr::from_string(s).into_raw().cast()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_text_str(text: *const Text, out: *mut Str) -> Status {
    if text.is_null() {
        return Status::Null;
    }
    let s = unsafe { core::ffi::CStr::from_ptr(text.cast()) };
    unsafe { put(out, Str { data: s.as_ptr(), len: s.count_bytes() }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_text_free(text: *mut Text) {
    if !text.is_null() {
        drop(unsafe { alloc::ffi::CString::from_raw(text.cast()) });
    }
}

// Every string in one buffer, each followed by its NUL, and where each ends: two allocations for any count, which
// beats one a string for the many short ones names and glyph names are.
#[derive(Default)]
pub struct StrList {
    text: Vec<u8>,
    ends: Vec<usize>,
}

// What a list makes of a string holding a NUL, which would end it early as a C string.
#[derive(Clone, Copy)]
pub enum OnNul {
    // Each NUL as U+FFFD, as OwnedStr does.
    Replace,
    // The string empty, as a glyph name holding one is none.
    Empty,
}

impl StrList {
    // Sized by a first pass over the strings, so the buffer is allocated once.
    pub fn new<'a>(items: impl Iterator<Item = &'a str> + Clone, on_nul: OnNul) -> StrList {
        let (bytes, count) = items.clone().fold((0, 0), |(bytes, count), s| (bytes + s.len() + 1, count + 1));
        let mut list = StrList::with_capacity(bytes, count);
        items.for_each(|s| list.push_str(s));
        list.finish(on_nul)
    }

    pub fn with_capacity(bytes: usize, count: usize) -> StrList {
        StrList { text: Vec::with_capacity(bytes), ends: Vec::with_capacity(count) }
    }

    pub fn reserve(&mut self, bytes: usize, count: usize) {
        self.text.reserve(bytes);
        self.ends.reserve(count);
    }

    pub fn push_str(&mut self, s: &str) {
        self.push(s.as_bytes());
    }

    // One count of every NUL, summed in a byte over each 255 so it vectorizes, costs next to nothing where looking in
    // each string cost a fifth of the call; a string holding a NUL of its own is rare.
    pub fn finish(self, on_nul: OnNul) -> StrList {
        let in_byte = |c: &[u8]| usize::from(c.iter().map(|&b| u8::from(b == 0)).sum::<u8>());
        if self.text.chunks(255).map(in_byte).sum::<usize>() == self.ends.len() {
            return self;
        }
        let mut mended = StrList::with_capacity(self.text.len(), self.ends.len());
        for s in (0..self.ends.len()).filter_map(|i| self.bytes(i)) {
            match (s.contains(&0), on_nul) {
                (false, _) => mended.push(s),
                (true, OnNul::Replace) => {
                    mended.push(&s.split(|&b| b == 0).collect::<Vec<_>>().join("\u{fffd}".as_bytes()));
                }
                (true, OnNul::Empty) => mended.push(&[]),
            }
        }
        mended
    }

    fn push(&mut self, s: &[u8]) {
        self.text.extend_from_slice(s);
        self.ends.push(self.text.len());
        self.text.push(0);
    }

    // A string and the NUL after it.
    fn with_nul(&self, index: usize) -> Option<&[u8]> {
        let end = *self.ends.get(index)?;
        let start = index.checked_sub(1).and_then(|i| self.ends.get(i)).map_or(0, |e| e + 1);
        self.text.get(start..=end)
    }

    fn bytes(&self, index: usize) -> Option<&[u8]> {
        self.with_nul(index)?.split_last().map(|(_, s)| s)
    }

    // The bytes its strings hold, their NULs left out.
    pub fn bytes_held(&self) -> usize {
        self.text.len() - self.ends.len()
    }

    pub fn len(&self) -> usize {
        self.ends.len()
    }

    // Emptied, its room kept; the bytes that room holds.
    pub fn emptied(&mut self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        self.text.clear();
        self.ends.clear();
        heap_of(&self.text) + heap_of(&self.ends)
    }

    // The pointer is the NUL's too, as C may read it.
    pub fn get(&self, index: usize) -> Option<Str> {
        let s = self.with_nul(index)?;
        Some(Str { data: s.as_ptr().cast(), len: s.len() - 1 })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_str_list_count(list: *const StrList, out: *mut usize) -> Status {
    let Some(list) = (unsafe { borrow(list) }) else { return Status::Null };
    unsafe { put(out, list.ends.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_str_list_at(list: *const StrList, index: usize, out: *mut Str) -> Status {
    let Some(list) = (unsafe { borrow(list) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(item) = list.get(index) else { return past_end("string", index, list.ends.len()) };
    unsafe { put(out, item) }
}

free!(daegun_str_list_free, StrList);

pub(crate) fn release_thread() {
    crate::ffi::handle::drop_spare::<Blob>();
}
