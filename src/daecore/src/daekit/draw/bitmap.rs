use alloc::vec::Vec;

/// The most pixels an image a rasterizer lends is across or down; a draw into a [`Target`] takes any size.
pub const MAX_SIDE: u32 = 1 << 14;

/// The most bytes a draw whose image a rasterizer lends holds in its buffers, the image's among them. An outline a few
/// pixels to the em can still reach MAX_SIDE, where it would take gigabytes, so past this it's refused instead; a
/// draw into a [`Target`] holds the caller's pixels and works in bands of rows of a bounded size.
pub const MAX_DRAW_BYTES: usize = 1 << 28;

/// Pixels a caller holds for a rasterizer to draw into: `height` rows `stride` bytes apart, `width` pixels each, its
/// left edge `left` pixels right of the glyph's origin and its top edge `top` pixels above it, as an [`Image`] lies.
/// Every pixel is written.
#[non_exhaustive]
#[derive(PartialEq, Eq, Hash, Debug)]
pub struct Target<'a> {
    /// Pixels from the origin to the left edge.
    pub left: i32,
    /// Pixels from the origin up to the top edge.
    pub top: i32,
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// Bytes from one row to the next.
    pub stride: usize,
    /// The rows, top first.
    pub pixels: &'a mut [u8],
}

impl<'a> Target<'a> {
    /// `pixels` as a target `width` by `height` pixels, rows `stride` bytes apart, its top left corner `left` pixels
    /// right of the origin and `top` above it.
    pub fn new(left: i32, top: i32, width: u32, height: u32, stride: usize, pixels: &'a mut [u8]) -> Self {
        Target { left, top, width, height, stride, pixels }
    }

    // Whether its pixels hold its rows of `channels` bytes a pixel.
    pub(crate) fn holds(&self, channels: usize) -> bool {
        let Some(row) = (self.width as usize).checked_mul(channels) else { return false };
        let last = (self.height as usize).checked_sub(1).map(|r| r.checked_mul(self.stride)?.checked_add(row));
        self.stride >= row && last.is_none_or(|end| end.is_some_and(|end| end <= self.pixels.len()))
    }
}

/// Pixels a rasterizer lent: `height` rows `stride` bytes apart, each pixel `channels` bytes, the image's left edge
/// `left` pixels right of the glyph's origin and its top edge `top` pixels above it.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Image<'a> {
    /// Pixels from the origin to the image's left edge.
    pub left: i32,
    /// Pixels from the origin up to the image's top edge.
    pub top: i32,
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// Bytes from one row to the next.
    pub stride: usize,
    /// Bytes a pixel: 1 for coverage, 3 for a subpixel layout's channels, 4 for premultiplied RGBA.
    pub channels: u8,
    /// The rows, top first.
    pub pixels: &'a [u8],
}

impl Image<'_> {
    /// A row's bytes, top first; empty past the last.
    pub fn row(&self, y: u32) -> &[u8] {
        // Checked, as the fields are the caller's to set and a 32-bit usize overflows long before u32 does.
        let row = || {
            let start = (y as usize).checked_mul(self.stride)?;
            let end = start.checked_add((self.width as usize).checked_mul(usize::from(self.channels))?)?;
            self.pixels.get(start..end)
        };
        if y < self.height { row().unwrap_or(&[]) } else { &[] }
    }

    /// A copy that owns its pixels, its rows packed.
    pub fn to_bitmap(self) -> Bitmap {
        let len = (self.width as usize).saturating_mul(usize::from(self.channels));
        let mut pixels = Vec::with_capacity(len.saturating_mul(self.height as usize).min(self.pixels.len()));
        for y in 0..self.height {
            pixels.extend_from_slice(self.row(y));
        }
        Bitmap {
            left: self.left,
            top: self.top,
            width: self.width,
            height: self.height,
            channels: self.channels,
            pixels,
        }
    }
}

/// An image that owns its pixels, its rows packed.
#[non_exhaustive]
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Bitmap {
    /// Pixels from the origin to the image's left edge.
    pub left: i32,
    /// Pixels from the origin up to the image's top edge.
    pub top: i32,
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// Bytes a pixel.
    pub channels: u8,
    /// The rows, top first, `width * channels` bytes each.
    pub pixels: Vec<u8>,
}

impl Bitmap {
    /// It as an [`Image`].
    pub fn image(&self) -> Image<'_> {
        Image {
            left: self.left,
            top: self.top,
            width: self.width,
            height: self.height,
            stride: (self.width as usize).saturating_mul(usize::from(self.channels)),
            channels: self.channels,
            pixels: &self.pixels,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/draw/bitmap.rs"]
mod tests;
