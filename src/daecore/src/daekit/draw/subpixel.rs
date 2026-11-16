use alloc::vec::Vec;
#[cfg(doc)]
use crate::ErrorKind::InvalidArgument;

// All three are copied literally into raster.h, which cannot follow a Rust constant; roundtrip.c
// checks the library against those copies.
/// The most samples a layout takes a pixel, each way.
pub const MAX_OVERSAMPLE: u8 = 4;
/// The most taps a layout's filter has, each way.
#[doc(alias("MAX_SUBPIXEL_TAPS"))]
pub const MAX_TAPS: usize = MAX_OVERSAMPLE as usize + 4;
/// The most weights a channel of a filter has: [`MAX_TAPS`] squared.
#[doc(alias("MAX_SUBPIXEL_WEIGHTS"))]
pub const MAX_WEIGHTS: usize = MAX_TAPS * MAX_TAPS;

const FIR5: [f32; 5] = [8.0 / 256.0, 77.0 / 256.0, 86.0 / 256.0, 77.0 / 256.0, 8.0 / 256.0];

/// The order of a display's subpixel stripes.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StripeOrder {
    /// Red, green, blue, left to right or top to bottom.
    Rgb,
    /// Blue, green, red.
    Bgr,
}

impl StripeOrder {
    fn centers(self) -> [usize; 3] {
        match self {
            StripeOrder::Rgb => [0, 1, 2],
            StripeOrder::Bgr => [2, 1, 0],
        }
    }
}

/// How a glyph rasterized at several samples a pixel filters down to one value a channel: plain coverage, a
/// display's stripes, or a filter of the caller's.
// Fields are deliberately private: `key` is this layout's cache identity, so a layout whose contents
// did not match it would be served another layout's bitmaps by any cache keyed on it.
#[derive(Clone, Copy)]
pub struct SubpixelLayout {
    pub(crate) key: u64,
    pub(crate) ox: u8,
    pub(crate) oy: u8,
    pub(crate) taps_x: u8,
    pub(crate) taps_y: u8,
    pub(crate) origin_x: i8,
    pub(crate) origin_y: i8,
    pub(crate) weights: [[f32; MAX_WEIGHTS]; 3],
    pub(crate) channels: u8,
}

impl Default for SubpixelLayout {
    fn default() -> Self {
        Self::GRAYSCALE
    }
}

impl core::fmt::Debug for SubpixelLayout {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SubpixelLayout")
            .field("key", &self.key)
            .field("samples", &(self.ox, self.oy))
            .field("taps", &(self.taps_x, self.taps_y))
            .field("origin", &(self.origin_x, self.origin_y))
            .field("channels", &self.channels)
            .finish_non_exhaustive()
    }
}

impl SubpixelLayout {
    const GRAYSCALE: SubpixelLayout = {
        let mut weights = [[0.0; MAX_WEIGHTS]; 3];
        weights[0][0] = 1.0;
        SubpixelLayout {
            key: 0,
            ox: 1, oy: 1, taps_x: 1, taps_y: 1, origin_x: 0, origin_y: 0,
            weights, channels: 1,
        }
        .stamped()
    };

    /// One sample a pixel and one channel: plain coverage.
    pub const fn grayscale() -> Self {
        Self::GRAYSCALE
    }

    /// Three samples across a pixel, for stripes side by side, through a five-tap filter that softens color
    /// fringes.
    pub fn horizontal(order: StripeOrder) -> Self {
        Self::striped(order, &FIR5, true)
    }

    /// Three samples down a pixel, for stripes stacked, through the same filter as [`SubpixelLayout::horizontal`].
    pub fn vertical(order: StripeOrder) -> Self {
        Self::striped(order, &FIR5, false)
    }

    /// Three samples a pixel, across for stripes side by side (`Horizontal`) or down for stacked ones, each
    /// channel its stripe's sample alone.
    pub fn unfiltered(order: StripeOrder, orientation: crate::daecore::Orientation) -> Self {
        Self::striped(order, &[1.0], !orientation.is_vertical())
    }

    /// A filter of the caller's: `taps.0 * taps.1` weights for each of three channels, row by row, over
    /// `oversample` samples a pixel each way, its window starting `origin` samples from the pixel's first.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`] for an oversample of 0 or past [`MAX_OVERSAMPLE`], taps of 0 or past [`MAX_TAPS`], a
    /// channel of another length, or a weight that isn't finite.
    pub fn from_weights(
        oversample: (u8, u8),
        taps: (u8, u8),
        origin: (i8, i8),
        weights: [&[f32]; 3],
    ) -> crate::daecore::daekit::base::error::Result<Self> {
        use crate::daecore::Error;
        let (ox, oy) = oversample;
        let (taps_x, taps_y) = taps;
        if ox == 0 || oy == 0 || ox > MAX_OVERSAMPLE || oy > MAX_OVERSAMPLE {
            return Err(Error::invalid("subpixel layout: an oversample is 0 or past MAX_OVERSAMPLE"));
        }
        if taps_x == 0 || taps_y == 0 || taps_x as usize > MAX_TAPS || taps_y as usize > MAX_TAPS {
            return Err(Error::invalid("subpixel layout: a tap count is 0 or past MAX_TAPS"));
        }
        let need = taps_x as usize * taps_y as usize;
        let mut table = [[0.0; MAX_WEIGHTS]; 3];
        for (c, src) in weights.iter().enumerate() {
            if src.len() != need {
                return Err(Error::invalid("subpixel layout: a channel's weights are not its taps across times down"));
            }
            if !src.iter().all(|w| w.is_finite()) {
                return Err(Error::invalid("subpixel layout: a weight is not finite"));
            }
            table[c][..need].copy_from_slice(src);
        }
        Ok(SubpixelLayout {
            key: 0,
            ox, oy, taps_x, taps_y,
            origin_x: origin.0, origin_y: origin.1,
            weights: table, channels: 3,
        }
        .stamped())
    }

    fn striped(order: StripeOrder, kernel: &[f32], horizontal: bool) -> Self {
        let span = 3usize;
        let reach = kernel.len() / 2;
        let taps = span + kernel.len() - 1;
        let mut weights = [[0.0; MAX_WEIGHTS]; 3];
        for (c, center) in order.centers().iter().enumerate() {
            for (k, &w) in kernel.iter().enumerate() {
                weights[c][center + k] += w;
            }
        }
        let (ox, oy) = if horizontal { (span as u8, 1) } else { (1, span as u8) };
        let (taps_x, taps_y) = if horizontal { (taps as u8, 1) } else { (1, taps as u8) };
        let (origin_x, origin_y) =
            if horizontal { (-(reach as i8), 0) } else { (0, -(reach as i8)) };
        SubpixelLayout { key: 0, ox, oy, taps_x, taps_y, origin_x, origin_y, weights, channels: 3 }
            .stamped()
    }

    const fn stamped(mut self) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let header =
            [self.ox, self.oy, self.taps_x, self.taps_y, self.origin_x as u8, self.origin_y as u8, self.channels];
        let mut i = 0;
        while i < header.len() {
            h ^= header[i] as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
            i += 1;
        }
        let active = self.taps_x as usize * self.taps_y as usize;
        let mut c = 0;
        while c < self.channels as usize {
            let mut w = 0;
            while w < active {
                let bytes = self.weights[c][w].to_bits().to_le_bytes();
                let mut b = 0;
                while b < bytes.len() {
                    h ^= bytes[b] as u64;
                    h = h.wrapping_mul(0x100_0000_01b3);
                    b += 1;
                }
                w += 1;
            }
            c += 1;
        }
        self.key = h;
        self
    }

    /// Whether it has one channel.
    pub const fn is_grayscale(&self) -> bool {
        self.channels == 1
    }

    /// Samples a pixel, (x, y).
    pub const fn oversample(&self) -> (u8, u8) {
        (self.ox, self.oy)
    }

    /// The filter's taps, (x, y).
    pub const fn taps(&self) -> (u8, u8) {
        (self.taps_x, self.taps_y)
    }

    /// Where the filter's window starts, in samples from the pixel's first, (x, y).
    pub const fn origin(&self) -> (i8, i8) {
        (self.origin_x, self.origin_y)
    }

    /// 1 for grayscale, 3 for a color layout.
    pub const fn channels(&self) -> u8 {
        self.channels
    }

    /// Its identity, for a cache key: a 64-bit hash of the parameters as given, so two filters share a key only
    /// by a collision, and one written another way (an extra zero weight, a -0.0) gets a key of its own.
    #[inline]
    pub const fn key(&self) -> u64 {
        self.key
    }

    /// A channel's `taps.0 * taps.1` weights, row by row; None past its channels.
    pub fn weights(&self, channel: usize) -> Option<&[f32]> {
        if channel >= self.channels as usize {
            return None;
        }
        let active = self.taps_x as usize * self.taps_y as usize;
        self.weights.get(channel)?.get(..active)
    }

    /// The whole pixels a bitmap needs on each side of the glyph for the filter's window, (x, y).
    pub fn pad(&self) -> (usize, usize) {
        let (x, y) = self.margin();
        (x.div_ceil(usize::from(self.ox)), y.div_ceil(usize::from(self.oy)))
    }

    // One kernel across for every channel, none of its weights negative, each on its own sample of three, red or blue
    // first: the kernel, where its first tap lies from the sample it's for, and whether blue comes first.
    pub(crate) fn stripes(&self) -> Option<([f32; MAX_TAPS], usize, isize, bool)> {
        if self.channels != 3 || self.ox != 3 || self.oy != 1 || self.taps_y != 1 {
            return None;
        }
        let tx = usize::from(self.taps_x);
        let span = |c: usize| {
            let w: &[f32] = &self.weights[c][..tx];
            Some((w.iter().position(|&v| v != 0.0)?, w.iter().rposition(|&v| v != 0.0)?))
        };
        let [(f0, l0), (f1, l1), (f2, l2)] = [span(0)?, span(1)?, span(2)?];
        let kernel = &self.weights[0][f0..=l0];
        if self.weights[1][f1..=l1] != *kernel || self.weights[2][f2..=l2] != *kernel {
            return None;
        }
        if kernel.iter().any(|&w| w < 0.0) {
            return None;
        }
        let reversed = match (f1 as isize - f0 as isize, f2 as isize - f0 as isize) {
            (1, 2) => false,
            (-1, -2) => true,
            _ => return None,
        };
        let lo = isize::from(self.origin_x) + (if reversed { f2 } else { f0 }) as isize;
        if lo.unsigned_abs() + kernel.len() >= 64 {
            return None;
        }
        let mut out = [0.0; MAX_TAPS];
        out[..kernel.len()].copy_from_slice(kernel);
        Some((out, kernel.len(), lo, reversed))
    }

    // Samples the window reaches past a pixel's own on either side, (x, y): `-origin` before it and
    // `origin + taps - oversample` past it, the farther of the two.
    pub(crate) fn margin(&self) -> (usize, usize) {
        let reach = |origin: i8, taps: u8, oversample: u8| {
            let (o, t, n) = (isize::from(origin), isize::from(taps), isize::from(oversample));
            (-o).max(o + t - n).max(0) as usize
        };
        (reach(self.origin_x, self.taps_x, self.ox), reach(self.origin_y, self.taps_y, self.oy))
    }

    /// Filters an image's samples into its pixels, three bytes each, rows packed into `out`. `samples` holds
    /// `height · oversample` rows `stride` apart, with `margin` zero samples around them on every side; `rows` is
    /// scratch for a window more than one row tall.
    // Each channel sums its weights over its window in f32, row by row and tap by tap, the same order on every CPU;
    // the three run side by side. A stretch of windows of only empty or only full samples takes that sum's result.
    pub(crate) fn filter(
        &self, samples: &[u8], stride: usize, (width, height): (usize, usize), out: &mut [u8], rows: &mut Vec<u8>,
    ) {
        if width == 0 || height == 0 {
            return;
        }
        let (mx, my) = self.margin();
        let (ox, oy) = (usize::from(self.ox), usize::from(self.oy));
        let (tx, ty) = (usize::from(self.taps_x), usize::from(self.taps_y));
        let mut weights = [[0.0f32; 4]; MAX_WEIGHTS];
        for (k, w) in weights[..tx * ty].iter_mut().enumerate() {
            *w = [self.weights[0][k], self.weights[1][k], self.weights[2][k], 0.0];
        }
        let weights = &weights[..tx * ty];
        let level = |sum: [f32; 4]| sum.map(|v| (v.clamp(0.0, 255.0) + 0.5) as u8);
        let full = level(weights.iter().fold([0.0; 4], |acc, w| add(acc, w, 255.0)));
        let full = [full[0], full[1], full[2]];
        // Weights all multiples of 1/256, their magnitudes summing to at most 256, keep every f32 product and
        // partial sum exact below 2^24, so integers in 256ths give the same bytes, faster.
        let whole = |v: f32| v.abs() <= 65536.0 && v == (v as i32) as f32;
        let exact = weights.iter().flatten().all(|w| whole(w * 256.0))
            && (0..3).all(|c| weights.iter().map(|w| w[c].abs()).sum::<f32>() <= 256.0);
        let mut whole = [[0i32; 4]; MAX_WEIGHTS];
        for (i, w) in whole.iter_mut().zip(weights) {
            *i = w.map(|v| (v * 256.0) as i32);
        }
        let whole = &whole[..tx * ty];
        let mask = if tx >= 8 { u64::MAX } else { (1 << (8 * tx)) - 1 };
        let x0 = (mx as isize + isize::from(self.origin_x)) as usize;
        let y0 = (my as isize + isize::from(self.origin_y)) as usize;
        for (y, row) in out.chunks_exact_mut(width * 3).take(height).enumerate() {
            let top = (y0 + y * oy) * stride;
            // Each column's samples down the window: whether any is set, and whether all are full.
            let (any, all): (&[u8], &[u8]) = if ty == 1 {
                let line = &samples[top..top + stride];
                (line, line)
            } else {
                rows.clear();
                rows.extend_from_slice(&samples[top..top + stride]);
                rows.extend_from_slice(&samples[top..top + stride]);
                let (any, all) = rows.split_at_mut(stride);
                for line in samples[top + stride..].chunks(stride).take(ty - 1) {
                    for ((a, b), &v) in any.iter_mut().zip(all.iter_mut()).zip(line) {
                        (*a, *b) = (*a | v, *b & v);
                    }
                }
                (any, all)
            };
            let row: &mut [[u8; 3]] = row.as_chunks_mut().0;
            let mut x = 0;
            while x < width {
                let a = x0 + x * ox;
                // An empty window starts a stretch that runs to the next sample set, a solid one to the next not
                // full; each stretch takes one value.
                let (end, fill) = if word(any, a) & mask == 0 {
                    (next(any, a, |v| v != 0), [0; 3])
                } else if word(all, a) & mask == mask {
                    (next(all, a, |v| v != u64::MAX), full)
                } else {
                    let [r, g, b, _] = if exact {
                        let mut sum = [0i32; 4];
                        for (r, taps) in whole.chunks_exact(tx).enumerate() {
                            let window = word(samples, top + r * stride + a).to_le_bytes();
                            for (w, &v) in taps.iter().zip(&window) {
                                let v = i32::from(v);
                                sum = [sum[0] + w[0] * v, sum[1] + w[1] * v, sum[2] + w[2] * v, sum[3] + w[3] * v];
                            }
                        }
                        sum.map(|v| ((v.clamp(0, 255 << 8) + 128) >> 8) as u8)
                    } else {
                        let mut sum = [0.0; 4];
                        for (r, taps) in weights.chunks_exact(tx).enumerate() {
                            let window = word(samples, top + r * stride + a).to_le_bytes();
                            for (w, &v) in taps.iter().zip(&window) {
                                sum = add(sum, w, f32::from(v));
                            }
                        }
                        level(sum)
                    };
                    row[x] = [r, g, b];
                    x += 1;
                    continue;
                };
                let n = ((end - tx - a) / ox + 1).min(width - x);
                let stretch = &mut row[x..x + n];
                if n >= 16 && fill[0] == fill[1] && fill[1] == fill[2] {
                    stretch.as_flattened_mut().fill(fill[0]);
                } else {
                    for pixel in stretch {
                        *pixel = fill;
                    }
                }
                x += n;
            }
        }
    }
}

// Each lane's sum after one more tap, its weight times the sample.
#[inline(always)]
fn add(sum: [f32; 4], w: &[f32; 4], v: f32) -> [f32; 4] {
    [sum[0] + w[0] * v, sum[1] + w[1] * v, sum[2] + w[2] * v, sum[3] + w[3] * v]
}

// Eight samples from `at` as one little-endian word, zeros past the row's end.
#[inline]
fn word(row: &[u8], at: usize) -> u64 {
    let mut bytes = [0; 8];
    match row.get(at..at + 8) {
        Some(chunk) => bytes.copy_from_slice(chunk),
        None => {
            let tail = &row[at.min(row.len())..];
            bytes[..tail.len()].copy_from_slice(tail);
        }
    }
    u64::from_le_bytes(bytes)
}

// The first column at or after `at` whose eight-sample chunk `stop` takes, then the first byte in it that differs from
// the chunk's fill; the row's length if none.
fn next(row: &[u8], at: usize, stop: impl Fn(u64) -> bool) -> usize {
    let fill = if stop(0) { u8::MAX } else { 0 };
    let mut i = at;
    while let Some(chunk) = row.get(i..i + 8) {
        let v = u64::from_ne_bytes(chunk.try_into().unwrap_or_default());
        if stop(v) {
            return i + chunk.iter().position(|&b| b != fill).unwrap_or(0);
        }
        i += 8;
    }
    i + row.get(i..).unwrap_or_default().iter().position(|&b| b != fill).unwrap_or(row.len() - i.min(row.len()))
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/draw/subpixel.rs"]
mod tests;
