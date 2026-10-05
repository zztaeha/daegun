// All three are copied literally into daegun.h, which cannot follow a Rust constant; roundtrip.c
// checks the library against those copies.
pub const MAX_OVERSAMPLE: u8 = 4;
pub const MAX_TAPS: usize = MAX_OVERSAMPLE as usize + 4;
pub const MAX_WEIGHTS: usize = MAX_TAPS * MAX_TAPS;

const FIR5: [f32; 5] = [8.0 / 256.0, 77.0 / 256.0, 86.0 / 256.0, 77.0 / 256.0, 8.0 / 256.0];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StripeOrder {
    Rgb,
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
    pub const GRAYSCALE: SubpixelLayout = {
        let mut weights = [[0.0; MAX_WEIGHTS]; 3];
        weights[0][0] = 1.0;
        SubpixelLayout {
            key: 0,
            ox: 1, oy: 1, taps_x: 1, taps_y: 1, origin_x: 0, origin_y: 0,
            weights, channels: 1,
        }
        .stamped()
    };

    pub fn grayscale() -> Self {
        Self::GRAYSCALE
    }

    pub fn horizontal(order: StripeOrder) -> Self {
        Self::striped(order, &FIR5, true)
    }

    pub fn vertical(order: StripeOrder) -> Self {
        Self::striped(order, &FIR5, false)
    }

    pub fn unfiltered(order: StripeOrder, horizontal: bool) -> Self {
        Self::striped(order, &[1.0], horizontal)
    }

    pub fn from_weights(
        oversample: (u8, u8),
        taps: (u8, u8),
        origin: (i8, i8),
        weights: [&[f32]; 3],
    ) -> Option<Self> {
        let (ox, oy) = oversample;
        let (taps_x, taps_y) = taps;
        if ox == 0 || oy == 0 || ox > MAX_OVERSAMPLE || oy > MAX_OVERSAMPLE { return None; }
        if taps_x == 0 || taps_y == 0 || taps_x as usize > MAX_TAPS || taps_y as usize > MAX_TAPS { return None; }
        let need = taps_x as usize * taps_y as usize;
        let mut table = [[0.0; MAX_WEIGHTS]; 3];
        for (c, src) in weights.iter().enumerate() {
            if src.len() != need { return None; }
            if !src.iter().all(|w| w.is_finite()) { return None; }
            table[c][..need].copy_from_slice(src);
        }
        Some(SubpixelLayout {
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
        let header = [self.ox, self.oy, self.taps_x, self.taps_y, self.origin_x as u8, self.origin_y as u8, self.channels];
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

    pub fn is_grayscale(&self) -> bool {
        self.channels == 1
    }

    pub fn oversample(&self) -> (u8, u8) {
        (self.ox, self.oy)
    }

    pub fn taps(&self) -> (u8, u8) {
        (self.taps_x, self.taps_y)
    }

    pub fn origin(&self) -> (i8, i8) {
        (self.origin_x, self.origin_y)
    }

    pub fn channels(&self) -> u8 {
        self.channels
    }

    // A 64-bit hash of the parameters as given: two filters share a key only by a collision, and one
    // written another way (an extra zero weight, a -0.0) gets a key of its own.
    #[inline]
    pub fn key(&self) -> u64 {
        self.key
    }

    pub fn weights(&self, channel: usize) -> Option<&[f32]> {
        if channel >= self.channels as usize {
            return None;
        }
        let active = self.taps_x as usize * self.taps_y as usize;
        self.weights.get(channel)?.get(..active)
    }

    // Whole pixels a side. The window reaches `-origin` samples before the pixel and
    // `origin + taps - oversample` past it, and the pad covers the farther.
    pub fn pad(&self) -> (usize, usize) {
        let pad = |origin: i8, taps: u8, oversample: u8| {
            let (o, t, n) = (isize::from(origin), isize::from(taps), isize::from(oversample));
            ((-o).max(o + t - n).max(0) as usize).div_ceil(oversample as usize)
        };
        (pad(self.origin_x, self.taps_x, self.ox), pad(self.origin_y, self.taps_y, self.oy))
    }
}
