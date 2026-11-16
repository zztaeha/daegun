use alloc::vec::Vec;
use super::decoder::{read_fixed, read_i16_be, read_u16_be, read_u32_be, records_fit};
use crate::daecore::daetype::Tables;

// 0.0 is the registry's "normal", and the only track asked for: a shaper with no interface for
// choosing one has nothing else it could honestly select.
const NORMAL_TRACK: f64 = 0.0;

/// The point size trak is read at when the caller names none, or one not finite and above zero.
pub const DEFAULT_POINT_SIZE: f64 = 12.0;

// A trak is read at version 1.0, the one Apple's manual defines.
pub(crate) fn has_data(trak: &[u8]) -> bool {
    trak.len() >= 12 && read_u16_be(trak, 0) == Some(1)
}

// The normal track at `ptem` in font units, reckoned in f64, in which every Fixed and FWord is exact.
pub fn tracking(table_map: &Tables, ptem: f64, horizontal: bool) -> f64 {
    let Some(trak) = table_map.get("trak").filter(|t| has_data(t)) else { return 0.0 };
    let at = if horizontal { 6 } else { 8 };
    let Some(data) = read_u16_be(trak, at).map(usize::from).filter(|&o| o != 0) else {
        return 0.0;
    };
    // A size of 0 or less, NaN or infinite names no point size, so it takes the default's.
    let ptem = Some(ptem).filter(|p| p.is_finite() && *p > 0.0).unwrap_or(DEFAULT_POINT_SIZE);
    TrackData::read(trak, data).map_or(0.0, |d| d.at(ptem))
}

struct TrackData<'a> {
    trak: &'a [u8],
    tracks: usize,
    n_tracks: usize,
    sizes: usize,
    n_sizes: usize,
}

impl<'a> TrackData<'a> {
    // The size table, or the track records, absent where its count leaves the table; `present` says which tracks
    // hold their values.
    fn read(trak: &'a [u8], at: usize) -> Option<Self> {
        let sizes = read_u32_be(trak, at + 4)? as usize;
        let counted = |n: usize, start: usize, size: usize| if records_fit(start, n, size, trak.len()) { n } else { 0 };
        let n_sizes = counted(usize::from(read_u16_be(trak, at + 2)?), sizes, 4);
        let n_tracks = counted(usize::from(read_u16_be(trak, at)?), at + 8, 8);
        Some(TrackData { trak, tracks: at + 8, n_tracks, sizes, n_sizes })
    }

    // A track is absent where its values for the sizes leave the table.
    fn present(&self, k: usize) -> bool {
        self.values(k).is_some_and(|v| v.checked_add(2 * self.n_sizes).is_some_and(|end| end <= self.trak.len()))
    }

    fn values(&self, k: usize) -> Option<usize> {
        read_u16_be(self.trak, self.tracks + 8 * k + 6).map(usize::from)
    }

    fn track(&self, k: usize) -> f64 {
        read_fixed(self.trak, self.tracks + 8 * k).unwrap_or(0.0)
    }

    fn size(&self, i: usize) -> f64 {
        read_fixed(self.trak, self.sizes + 4 * i).unwrap_or(0.0)
    }

    fn value(&self, k: usize, i: usize) -> f64 {
        let values = self.values(k).unwrap_or(0);
        read_i16_be(self.trak, values + 2 * i).map_or(0.0, f64::from)
    }

    // The normal track, or a straight line between the present tracks either side of it.
    fn at(&self, ptem: f64) -> f64 {
        let present: Vec<usize> = (0..self.n_tracks).filter(|&k| self.present(k)).collect();
        let n = present.len();
        if n <= 1 {
            return present.first().map_or(0.0, |&k| self.at_size(k, ptem));
        }
        let (mut lo, mut hi) = (0, n - 1);
        while lo + 1 < n && self.track(present[lo + 1]) <= NORMAL_TRACK {
            lo += 1;
        }
        while hi > 0 && self.track(present[hi - 1]) >= NORMAL_TRACK {
            hi -= 1;
        }
        let (lo, hi) = (present[lo], present[hi]);
        let a = self.at_size(lo, ptem);
        let (t0, t1) = (self.track(lo), self.track(hi));
        // Two tracks of one value would divide by zero; the first stands.
        if lo == hi || t0 == t1 {
            return a;
        }
        a + (NORMAL_TRACK - t0) / (t1 - t0) * (self.at_size(hi, ptem) - a)
    }

    // The end sizes' values past the range, and a straight line between sizes.
    fn at_size(&self, k: usize, ptem: f64) -> f64 {
        let n = self.n_sizes;
        if n <= 1 {
            return if n == 0 { 0.0 } else { self.value(k, 0) };
        }
        let Some(i) = (0..n).find(|&i| self.size(i) >= ptem) else { return self.value(k, n - 1) };
        if i == 0 || self.size(i) == ptem {
            return self.value(k, i);
        }
        let (s0, v0) = (self.size(i - 1), self.value(k, i - 1));
        v0 + (ptem - s0) / (self.size(i) - s0) * (self.value(k, i) - v0)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/daetype/trak.rs"]
mod tests;
