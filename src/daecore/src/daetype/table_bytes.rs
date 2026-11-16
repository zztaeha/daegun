
use alloc::vec::Vec;
use core::ops::Deref;

use crate::daecore::daekit::base::sync::Shared;

// The bytes a font's tables are windows onto: its own, or the program's. Two kinds only, as a third (an `Arc` of a
// caller's bytes) slowed every font's table reads, and a call through `dyn AsRef` far more (logbook, A3).
pub(crate) enum Source {
    Owned(Vec<u8>),
    Static(&'static [u8]),
}

impl Deref for Source {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        match self {
            Source::Owned(bytes) => bytes,
            Source::Static(bytes) => bytes,
        }
    }
}

// A window onto the font's one shared buffer, so a table is never copied; cloning is a refcount.
#[derive(Clone)]
pub(crate) struct TableBytes {
    buf: Shared<Source>,
    start: usize,
    // 32 bits, as sfnt states a table's length, so a table takes no more than a pointer and two words.
    len: u32,
}

impl TableBytes {
    // A font can't hold a table past 4 GiB, so the window ends there.
    pub fn from_vec(bytes: Vec<u8>) -> TableBytes {
        let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        TableBytes { buf: Shared::new(Source::Owned(bytes)), start: 0, len }
    }

    pub(crate) fn slice(buf: &Shared<Source>, start: usize, len: usize) -> Option<TableBytes> {
        let end = start.checked_add(len)?;
        if end > buf.len() {
            return None;
        }
        let len = u32::try_from(len).ok()?;
        Some(TableBytes { buf: Shared::clone(buf), start, len })
    }

    // The length of the buffer this is a window onto: a font file's, or a collection face's copy.
    pub(crate) fn source_len(&self) -> usize {
        self.buf.len()
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf[self.start..self.start + self.len as usize]
    }

    pub fn to_owned_vec(&self) -> Vec<u8> {
        self.as_slice().to_vec()
    }
}

impl Deref for TableBytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl AsRef<[u8]> for TableBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl From<Vec<u8>> for TableBytes {
    fn from(bytes: Vec<u8>) -> TableBytes {
        TableBytes::from_vec(bytes)
    }
}

impl core::fmt::Debug for TableBytes {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "TableBytes({} bytes)", self.len)
    }
}

impl PartialEq for TableBytes {
    fn eq(&self, other: &TableBytes) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl Eq for TableBytes {}

impl Default for TableBytes {
    fn default() -> TableBytes {
        TableBytes::from_vec(Vec::new())
    }
}
