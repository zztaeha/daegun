use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cmp::Ordering;
use core::marker::PhantomData;

/// The `N` bytes at `off`; None where they run past the end.
pub fn window<const N: usize>(data: &[u8], off: usize) -> Option<&[u8; N]> {
    // A consumer may turn overflow checks on, and on 32-bit a hostile 0xFFFFFFFF offset is usize::MAX,
    // so a plain add would panic or wrap to a wrong in-range read.
    data.get(off..off.checked_add(N)?)?.try_into().ok()
}

/// The big-endian u16 at `off`; None past the end.
pub fn read_u16_be(data: &[u8], off: usize) -> Option<u16> {
    let end = off.checked_add(2)?;
    let b = data.get(off..end)?;
    Some(((b[0] as u16) << 8) | b[1] as u16)
}

/// The big-endian u32 at `off`; None past the end.
pub fn read_u32_be(data: &[u8], off: usize) -> Option<u32> {
    let end = off.checked_add(4)?;
    let b = data.get(off..end)?;
    Some(((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | b[3] as u32)
}

/// The big-endian i16 at `off`; None past the end.
pub fn read_i16_be(data: &[u8], off: usize) -> Option<i16> {
    read_u16_be(data, off).map(|v| v as i16)
}

pub(crate) fn read_i32_be(data: &[u8], off: usize) -> Option<i32> {
    read_u32_be(data, off).map(|v| v as i32)
}

// F2Dot14 and Fixed 16.16 values, from the stored integer or from one a delta has moved.
pub(crate) fn f2dot14(raw: impl Into<f64>) -> f64 {
    raw.into() / 16384.0
}

pub(crate) fn fixed(raw: impl Into<f64>) -> f64 {
    raw.into() / 65536.0
}

pub(crate) fn read_f2dot14(data: &[u8], off: usize) -> Option<f64> {
    read_i16_be(data, off).map(f2dot14)
}

pub fn read_fixed(data: &[u8], off: usize) -> Option<f64> {
    read_i32_be(data, off).map(fixed)
}

/// The big-endian 24-bit unsigned value at `off`; None past the end.
pub fn read_u24_be(data: &[u8], off: usize) -> Option<u32> {
    let b = window::<3>(data, off)?;
    Some(((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32)
}

// An unsigned field of `width` bytes: 2, 3 or 4.
#[inline(always)]
pub(crate) fn read_uint(data: &[u8], off: usize, width: usize) -> Option<u32> {
    match width {
        2 => read_u16_be(data, off).map(u32::from),
        3 => read_u24_be(data, off),
        _ => read_u32_be(data, off),
    }
}

/// The big-endian 24-bit offset at `off`; None past the end.
pub fn read_offset24(data: &[u8], off: usize) -> Option<usize> {
    read_u24_be(data, off).map(|v| v as usize)
}

// The Offset16 at `at`, resolved against `base`; a NULL offset names no table.
pub(crate) fn resolve_offset16(data: &[u8], at: usize, base: usize) -> Option<usize> {
    read_u16_be(data, at).filter(|&rel| rel != 0).and_then(|rel| base.checked_add(usize::from(rel)))
}

// The Offset16 at `field` in the subtable at `from`, resolved against it. A field that reads lies in
// `data`, so `from` is below its length and adding 16 bits to it cannot overflow.
pub(crate) fn offset16_in(data: &[u8], from: usize, field: usize) -> Option<usize> {
    match read_u16_be(data, from.checked_add(field)?)? {
        0 => None,
        rel => Some(from + usize::from(rel)),
    }
}

// The u16 count at `at`, cut to the records of `stride` bytes that fit after it.
pub(crate) fn fitting_count(data: &[u8], at: usize, stride: usize) -> usize {
    let stated = read_u16_be(data, at).map_or(0, usize::from);
    stated.min(data.len().saturating_sub(at.saturating_add(2)) / stride)
}

pub(crate) const OFFSET24_MAX: usize = 0xFF_FFFF;

/// Writes `val` big-endian in the three bytes at `off`, and whether it did: not where they run past the end or
/// `val` past 24 bits.
pub fn write_offset24(data: &mut [u8], off: usize, val: usize) -> bool {
    if val > OFFSET24_MAX {
        return false;
    }
    let Some(slot) = off.checked_add(3).and_then(|end| data.get_mut(off..end)) else { return false };
    slot[0] = (val >> 16) as u8;
    slot[1] = (val >> 8) as u8;
    slot[2] = val as u8;
    true
}

/// Writes `val` big-endian at `off`, and whether it did: not where it runs past the end.
pub fn write_u16_be(data: &mut [u8], off: usize, val: u16) -> bool {
    let Some(slot) = off.checked_add(2).and_then(|end| data.get_mut(off..end)) else { return false };
    slot[0] = (val >> 8) as u8;
    slot[1] = val as u8;
    true
}

/// Writes `val` big-endian at `off`, and whether it did: not where it runs past the end.
pub fn write_u32_be(data: &mut [u8], off: usize, val: u32) -> bool {
    let Some(slot) = off.checked_add(4).and_then(|end| data.get_mut(off..end)) else { return false };
    slot[0] = (val >> 24) as u8;
    slot[1] = (val >> 16) as u8;
    slot[2] = (val >> 8) as u8;
    slot[3] = val as u8;
    true
}

/// Writes `val` big-endian at `off`, and whether it did: not where it runs past the end.
pub fn write_i16_be(data: &mut [u8], off: usize, val: i16) -> bool {
    write_u16_be(data, off, val as u16)
}

pub(crate) fn clamp_i16(v: i32) -> i16 {
    v.clamp(i16::MIN.into(), i16::MAX.into()) as i16
}

pub(crate) fn add_i16(v: i16, delta: i32) -> i16 {
    clamp_i16(i32::from(v).saturating_add(delta))
}

// The 16-bit field at `at` moved by `delta`, saturating at its range; a field past the end is left alone.
pub(crate) fn bump_i16(data: &mut [u8], at: usize, delta: i32) {
    if let Some(v) = read_i16_be(data, at) {
        write_i16_be(data, at, add_i16(v, delta));
    }
}

pub(crate) fn bump_u16(data: &mut [u8], at: usize, delta: i32) {
    if let Some(v) = read_u16_be(data, at) {
        write_u16_be(data, at, i32::from(v).saturating_add(delta).clamp(0, u16::MAX.into()) as u16);
    }
}

/// A binary search of `count` records sorted by key, `key_at` reading record `i`'s: `Ok` with the record found,
/// `Err` with where `target` would go, and None where a key doesn't read. It stops at the first midpoint that
/// matches, which decides the record where a font repeats keys.
pub fn search_records<F>(count: usize, target: u32, key_at: F) -> Option<Result<usize, usize>>
where
    F: Fn(usize) -> Option<u32>,
{
    bisect(count, |i| key_at(i).map(|k| target.cmp(&k)))
}

// A binary search that stops at the first midpoint that matches, which decides the record read where a font repeats
// or misorders them: the index found, if any. `order` places the target against record `i`.
pub(crate) fn midpoint_search(count: usize, order: impl Fn(usize) -> Option<Ordering>) -> Option<usize> {
    bisect(count, order)?.ok()
}

// The search they share; each step reads the lower of two middle records.
fn bisect(count: usize, order: impl Fn(usize) -> Option<Ordering>) -> Option<Result<usize, usize>> {
    let (mut lo, mut hi) = (0usize, count);
    while lo < hi {
        let mid = lo + (hi - lo - 1) / 2;
        let ordering = order(mid)?;
        // Tests rather than a match, which holds the ordering as a value and tests it twice a step.
        if ordering.is_lt() {
            hi = mid;
        } else if ordering.is_gt() {
            lo = mid + 1;
        } else {
            return Some(Ok(mid));
        }
    }
    Some(Err(lo))
}

// A binary search header for `n` units of `unit` bytes: searchRange, entrySelector and rangeShift, all
// 0 for none. Each format narrows them to its own field width.
pub(crate) fn bin_search_header(n: usize, unit: usize) -> [usize; 3] {
    if n == 0 {
        return [0; 3];
    }
    let selector = n.ilog2() as usize;
    let range = unit << selector;
    [range, selector, n * unit - range]
}

// A 64-bit fingerprint of byte strings, mixed a word at a time and finished as MurmurHash3's fmix64.
#[derive(Default)]
pub(crate) struct Fingerprint(u64);

impl Fingerprint {
    pub(crate) fn mix(&mut self, part: &[u8]) {
        let (words, rest) = part.as_chunks::<8>();
        let mut last = [0u8; 8];
        last[..rest.len()].copy_from_slice(rest);
        for &word in words.iter().chain([&last]) {
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(word)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
    }

    pub(crate) fn finish(self) -> u64 {
        let mut hash = (self.0 ^ (self.0 >> 33)).wrapping_mul(0xff51_afd7_ed55_8ccd);
        hash = (hash ^ (hash >> 33)).wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        hash ^ (hash >> 33)
    }
}

// Entries found again by fingerprint: open addressing over entry numbers, at most half full. A probe
// stops after 64 slots, so entries crafted to collide cost a missed share, not quadratic time.
#[derive(Default)]
pub(crate) struct FingerprintIndex {
    slots: Vec<u32>,
    hashes: Vec<u64>,
}

const PROBES: usize = 64;

impl FingerprintIndex {
    // Room for `n` entries from the start, so pushing them never moves the slots.
    pub(crate) fn with_capacity(n: usize) -> FingerprintIndex {
        let slots = alloc::vec![u32::MAX; (n * 2).max(16).next_power_of_two()];
        FingerprintIndex { slots, hashes: Vec::with_capacity(n) }
    }

    // The first entry with fingerprint `hash` that `same` accepts.
    pub(crate) fn find(&self, hash: u64, mut same: impl FnMut(usize) -> bool) -> Option<usize> {
        let mask = self.slots.len().checked_sub(1)?;
        (0..PROBES)
            .map_while(|i| {
                let n = self.slots[(hash as usize).wrapping_add(i) & mask] as usize;
                self.hashes.get(n).map(|&h| (n, h))
            })
            .find(|&(n, h)| h == hash && same(n))
            .map(|(n, _)| n)
    }

    // Indexes the next entry, numbered by how many came before it.
    pub(crate) fn push(&mut self, hash: u64) {
        if (self.hashes.len() + 1) * 2 > self.slots.len() {
            self.slots = alloc::vec![u32::MAX; (self.slots.len() * 2).max(16)];
            for (n, &h) in self.hashes.iter().enumerate() {
                Self::put(&mut self.slots, h, n);
            }
        }
        Self::put(&mut self.slots, hash, self.hashes.len());
        self.hashes.push(hash);
    }

    fn put(slots: &mut [u32], hash: u64, n: usize) {
        let mask = slots.len() - 1;
        if let Some(i) = (0..PROBES).map(|i| (hash as usize).wrapping_add(i) & mask).find(|&i| slots[i] == u32::MAX) {
            slots[i] = n as u32;
        }
    }
}

// Records laid out one after another, each distinct one once, at 16-bit offsets counted from `base`. A record borrowed
// from the same bytes as one placed before is found by where it lies, not hashed again.
pub(crate) struct Placed<'a> {
    base: usize,
    pub(crate) bytes: Vec<u8>,
    index: FingerprintIndex,
    spans: Vec<(usize, usize)>,
    // Each record placed, by the address and length it's borrowed from, and where it went.
    sources: BTreeMap<(usize, usize), u16>,
    borrowed: PhantomData<&'a [u8]>,
}

impl<'a> Placed<'a> {
    pub(crate) fn new(base: usize) -> Self {
        let (index, sources) = (FingerprintIndex::default(), BTreeMap::new());
        Placed { base, bytes: Vec::new(), index, spans: Vec::new(), sources, borrowed: PhantomData }
    }

    // Where `record` lies, None where it would start past 16 bits.
    pub(crate) fn place(&mut self, record: &'a [u8]) -> Option<u16> {
        let source = (record.as_ptr() as usize, record.len());
        if let Some(&at) = self.sources.get(&source) {
            return Some(at);
        }
        let mut print = Fingerprint::default();
        print.mix(record);
        let hash = print.finish();
        let (bytes, spans) = (&self.bytes, &self.spans);
        let at = match self.index.find(hash, |n| bytes.get(spans[n].0..spans[n].0 + spans[n].1) == Some(record)) {
            Some(n) => u16::try_from(self.base + spans[n].0).ok()?,
            None => {
                let at = u16::try_from(self.base + self.bytes.len()).ok()?;
                self.index.push(hash);
                self.spans.push((self.bytes.len(), record.len()));
                self.bytes.extend_from_slice(record);
                at
            }
        };
        self.sources.insert(source, at);
        Some(at)
    }
}

/// Whether `count` records of `stride` bytes from `start` lie within `len` bytes, with no overflow.
pub fn records_fit(start: usize, count: usize, stride: usize, len: usize) -> bool {
    count
        .checked_mul(stride)
        .and_then(|n| start.checked_add(n))
        .is_some_and(|end| end <= len)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/decoder/io.rs"]
mod tests;
