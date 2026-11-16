// daegun's CPU rasterizer: outlines to coverage, filtered for a display's stripes, composed from color layers or
// measured into distance fields, in buffers kept between calls.
mod compose;
mod coverage;
mod sdf;
mod spans;
mod sweep;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;
use crate::daecore::daekit::base::error::Result;
use crate::daecore::daekit::cache::AxisKey;
use crate::daecore::daekit::draw::bitmap::{Bitmap, Image, Target, MAX_DRAW_BYTES, MAX_SIDE};
use crate::daecore::daekit::draw::srgb::{coverage_table, from_linear, to_linear};
use crate::daecore::daekit::draw::Rgba;
use crate::daecore::daekit::draw::subpixel::SubpixelLayout;
use crate::daecore::daetype::outline::{FillRule, Path};
use crate::daecore::Error;

pub(crate) use coverage::Edges;

/// How wide a glyph's outline in font units is across the middle of its height: the first stretch it fills there
/// under the nonzero rule, through `edges`. None where it fills none.
pub(crate) fn across(edges: &mut Edges, path: &Path) -> Option<f32> {
    edges.begin([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    edges.path(path);
    edges.finish();
    let [_, low, _, high] = edges.bounds()?;
    let y = (low + high) * 0.5;
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for contour in edges.contours() {
        for (i, p) in contour.iter().enumerate() {
            let q = contour[(i + 1) % contour.len()];
            if (p[1] <= y) != (q[1] <= y) {
                crossings.push((p[0] + (y - p[1]) * (q[0] - p[0]) / (q[1] - p[1]), if q[1] > p[1] { 1 } else { -1 }));
            }
        }
    }
    crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut winding, mut start) = (0, 0.0);
    for (x, dir) in crossings {
        let was = winding;
        winding += dir;
        if was == 0 {
            start = x;
        } else if winding == 0 {
            return (x > start).then_some(x - start);
        }
    }
    None
}
use compose::Scene;
pub(crate) use compose::BAND_BYTES;
use sdf::Fields;
use coverage::{allowance, Coverage, Frame, Strips};

#[cfg(test)]
#[path = "../tests/CPU/main.rs"]
mod tests;

/// Rasterizes outlines into images it lends, keeping every buffer between calls, so once it has drawn the largest
/// image it allocates nothing. One a thread: it's `Send`, and each call takes it whole.
#[derive(Default)]
pub struct Rasterizer {
    pub(crate) edges: Edges,
    coverage: Coverage,
    pixels: Vec<u8>,
    // A subpixel layout's samples, with its filter's margin of zeros around them, and the filter's scratch rows.
    samples: Vec<u8>,
    rows: Vec<u8>,
    tables: Option<Tables>,
    scene: Scene,
    fields: Fields,
    // Each font and location drawn at, with its glyphs: each one's outline in font units, learned at its first draw,
    // and what its overlaps are; and in what order they were drawn, under a budget.
    pub(crate) learned: Vec<Learned>,
    pub(crate) recency: crate::raster::Recency,
    // With a font's raster cache on, the image a call lends in place of `pixels`, found there or just kept, held until
    // the next such call.
    pub(crate) held: Option<alloc::sync::Arc<Bitmap>>,
    // An outline drawn into a target, and its contours kept for its bands to be read from.
    pub(crate) outline: Path,
    strips: Strips,
}

// The last gamma's tables, a channel each, and what they were built for: the text's color, the contrast's bits and
// the channels.
type Tables = (([u8; 3], u32, u8), [[u8; 256]; 3]);

pub(crate) struct Learned {
    // Which it is in `recency`.
    pub(crate) id: crate::daecore::daekit::base::sync::Id,
    pub(crate) font: crate::daecore::daekit::base::sync::Id,
    pub(crate) key: AxisKey,
    pub(crate) glyphs: BTreeMap<u32, (Path, Shape)>,
    // The font's stem there in font units, once darkening has asked for it.
    pub(crate) stem: Option<f32>,
}

/// A learned outline's overlaps: none or removed, so it's summed straight at every size, or too close to call, so
/// it's resolved row by row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Shape {
    Plain,
    Removed,
    Unsure,
}

// Where an image lies; the pixels stay in the rasterizer.
pub(crate) struct Drawn {
    left: i32,
    top: i32,
    width: usize,
    height: usize,
    channels: u8,
}

impl Drawn {
    // A color glyph that paints nothing.
    pub(crate) const EMPTY_COLOR: Drawn = Drawn { left: 0, top: 0, width: 0, height: 0, channels: 4 };
}

impl core::fmt::Debug for Rasterizer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Rasterizer").finish_non_exhaustive()
    }
}

const TOO_LARGE: Error = Error::limit("the image would be wider or taller than MAX_SIDE pixels");
const TOO_MUCH: Error = Error::limit("drawing the image would hold more than MAX_DRAW_BYTES bytes");

// An image's corner, or a frame's in samples, where its i32 holds it.
pub(crate) fn corner(v: i64) -> Result<i32> {
    i32::try_from(v).map_err(|_| PAST)
}
pub(crate) const PAST: Error = Error::limit("the image would lie more than 2^31 pixels or samples from the origin");

// Whether a draw holding `bytes` fits MAX_DRAW_BYTES; counted in u64, as a 32-bit usize would wrap first.
pub(crate) fn fits(bytes: u64) -> Result<()> {
    if bytes > MAX_DRAW_BYTES as u64 { Err(TOO_MUCH) } else { Ok(()) }
}
const UNFIT: Error = Error::invalid("a point is not finite once placed");
pub(crate) const SHORT: Error = Error::invalid("the target's pixels don't hold its rows");
const TANGLED: Error =
    Error::limit("resolving the path's overlaps would take more than 2^27 passes over a pixel, 512 a point, 8 a pixel");

impl Rasterizer {
    /// A rasterizer with nothing kept yet.
    pub fn new() -> Rasterizer {
        Rasterizer::default()
    }

    /// A path filled by `rule`, after `transform` (a 2×3 matrix `[a, b, c, d, dx, dy]`) if any, in pixels, y up.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`](crate::ErrorKind::InvalidArgument) for a point not finite once transformed, and
    /// [`LimitExceeded`](crate::ErrorKind::LimitExceeded) for an image past [`MAX_SIDE`](crate::raster::MAX_SIDE) or
    /// [`MAX_DRAW_BYTES`](crate::raster::MAX_DRAW_BYTES), its flattened points counted in, or a path whose overlaps
    /// would take more than 2^27 passes over a pixel, and 512 more a point and 8 a pixel, to resolve.
    /// [`path_into`](Rasterizer::path_into) draws an image past the first two.
    pub fn path(&mut self, path: &Path, rule: FillRule, transform: Option<&[f64; 6]>) -> Result<Image<'_>> {
        self.edges.begin(transform.map_or([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], |m| m.map(|v| v as f32)));
        self.edges.path(path);
        let drawn = self.draw(rule, &SubpixelLayout::grayscale(), true)?;
        Ok(self.image(&drawn))
    }

    /// The pixels `draw` left, as an image.
    pub(crate) fn image(&self, drawn: &Drawn) -> Image<'_> {
        let stride = drawn.width * usize::from(drawn.channels);
        Image {
            left: drawn.left,
            top: drawn.top,
            width: drawn.width as u32,
            height: drawn.height as u32,
            stride,
            channels: drawn.channels,
            pixels: &self.pixels[..stride * drawn.height],
        }
    }

    /// The pixels `draw` left made alpha for a compositor blending in sRGB, for text of `text`'s color.
    pub(crate) fn correct(&mut self, drawn: &Drawn, text: Rgba, contrast: f32) {
        let len = drawn.width * drawn.height * usize::from(drawn.channels);
        let mut pixels = core::mem::take(&mut self.pixels);
        self.tone(&mut pixels[..len], drawn.channels, text, contrast);
        self.pixels = pixels;
    }

    /// Coverage made alpha in place, as `correct` makes it, `channels` bytes a pixel.
    pub(crate) fn tone(&mut self, pixels: &mut [u8], channels: u8, text: Rgba, contrast: f32) {
        let key = ([text.r, text.g, text.b], contrast.to_bits(), channels);
        if self.tables.as_ref().is_none_or(|t| t.0 != key) {
            let rgb = [text.r, text.g, text.b];
            let tables = if channels == 1 {
                let y = 0.2126 * to_linear(rgb[0]) + 0.7152 * to_linear(rgb[1]) + 0.0722 * to_linear(rgb[2]);
                let gray = coverage_table(from_linear(y), contrast);
                [gray; 3]
            } else {
                rgb.map(|v| coverage_table(v, contrast))
            };
            self.tables = Some((key, tables));
        }
        let Some((_, tables)) = &self.tables else { return };
        if channels == 1 {
            pixels.iter_mut().for_each(|v| *v = tables[0][usize::from(*v)]);
        } else {
            for pixel in pixels.as_chunks_mut::<3>().0 {
                *pixel = [0, 1, 2].map(|c| tables[c][usize::from(pixel[c])]);
            }
        }
    }

    /// `path` placed by `m` in pixels, y up, filled by `rule` into `target`'s coverage: placed and cut to the target
    /// once in f64, then a band of rows at a time within BAND_BYTES, each from the chords reaching it; resolved where
    /// contours may overlap (`exact`), as `draw` resolves, all the bands' work paid from one allowance.
    pub(crate) fn fill_into(
        &mut self, path: &Path, m: [f64; 6], rule: FillRule, exact: bool, target: &mut Target<'_>,
    ) -> Result<()> {
        let (width, height) = (target.width as usize, target.height as usize);
        if width == 0 || height == 0 {
            return Ok(());
        }
        let local = [m[0], m[1], m[2], m[3], m[4] - f64::from(target.left), m[5] - f64::from(target.top)];
        self.edges.begin_keep(local, [0.0, -(height as f64), width as f64, 0.0]);
        self.edges.path(path);
        self.edges.finish();
        if self.edges.unfit() {
            return Err(UNFIT);
        }
        let rows = (BAND_BYTES / ((width as u64 + 2) * 9)).clamp(1, height as u64) as usize;
        let mut strips = core::mem::take(&mut self.strips);
        self.edges.keep_into(&mut strips, rows, height.div_ceil(rows));
        let mut allowance = allowance(strips.points(), width as u64 * height as u64);
        let mut done = Ok(());
        for (b, y) in (0..height).step_by(rows).enumerate() {
            let down = rows.min(height - y);
            self.edges.band(&mut strips, b, down);
            let frame = Frame { left: 0, top: 0, width, height: down };
            let within = exact.then_some((usize::MAX, &mut allowance));
            let out = &mut target.pixels[y * target.stride..];
            if !self.coverage.draw(&self.edges, rule, frame, within, out, target.stride) {
                done = Err(TANGLED);
                break;
            }
        }
        self.strips = strips;
        done
    }

    /// `path` filled by `rule` into `target`, as [`path`](Rasterizer::path) fills it, one byte of coverage a pixel,
    /// however large the target: the path is placed and cut to the target once in f64, then worked out a band of rows
    /// at a time, each within a bounded size.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`](crate::ErrorKind::InvalidArgument) for pixels that don't hold the target's rows or a point
    /// not finite once transformed, and [`LimitExceeded`](crate::ErrorKind::LimitExceeded) for a path whose overlaps
    /// would take more than 2^27 passes over a pixel, and 512 more a point and 8 a pixel, to resolve, the target's
    /// pixels then left part written.
    pub fn path_into(
        &mut self, path: &Path, rule: FillRule, transform: Option<&[f64; 6]>, mut target: Target<'_>,
    ) -> Result<()> {
        if !target.holds(1) {
            return Err(SHORT);
        }
        self.fill_into(path, *transform.unwrap_or(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]), rule, true, &mut target)
    }

    /// Coverage of what was drawn onto `edges` since `begin`, filtered through `layout`: summed straight, then, where
    /// contours may overlap (`exact`), checked and resolved wherever the sums may be wrong.
    pub(crate) fn draw(&mut self, rule: FillRule, layout: &SubpixelLayout, exact: bool) -> Result<Drawn> {
        self.edges.finish();
        if self.edges.unfit() {
            return Err(UNFIT);
        }
        if self.edges.far() {
            return Err(TOO_LARGE);
        }
        if self.edges.too_many() {
            return Err(TOO_MUCH);
        }
        let Some([xmin, ymin, xmax, ymax]) = self.edges.bounds() else {
            return Ok(Drawn { left: 0, top: 0, width: 0, height: 0, channels: layout.channels() });
        };
        let (ox, oy) = layout.oversample();
        let (pad_x, pad_y) = layout.pad();
        let (ox, oy) = (f32::from(ox), f32::from(oy));
        // Saturating, as the bounds may lie anywhere f32 reaches: one that far is refused here or by `corner`.
        let left = ((xmin / ox).floor() as i64).saturating_sub(pad_x as i64);
        let right = ((xmax / ox).ceil() as i64).saturating_add(pad_x as i64);
        let bottom = ((ymin / oy).floor() as i64).saturating_sub(pad_y as i64);
        let top = ((ymax / oy).ceil() as i64).saturating_add(pad_y as i64);
        let (width, height) = (right.saturating_sub(left), top.saturating_sub(bottom));
        if width > i64::from(MAX_SIDE) || height > i64::from(MAX_SIDE) {
            return Err(TOO_LARGE);
        }
        let (left, top) = (corner(left)?, corner(top)?);
        let (width, height) = (width as usize, height as usize);
        let channels = layout.channels();
        let len = width * height * usize::from(channels);
        // Each sample's sum and mark and, oversampled, its byte, the margins and the two sums past a row's end in;
        // then the image.
        let ((ox, oy), (mx, my)) = (layout.oversample(), layout.margin());
        let samples = (width * usize::from(ox) + 2 * mx + 2) as u64 * (height * usize::from(oy) + 2 * my) as u64;
        fits(samples * 9 + len as u64)?;
        // What's left, which coverage holds the passages it checks for overlaps in, and the work it may do.
        let mut allowance = allowance(self.edges.points(), samples);
        let room = exact.then_some((MAX_DRAW_BYTES - (samples * 9 + len as u64) as usize, &mut allowance));
        // Never cleared: both paths write every pixel, and the filter every channel.
        if self.pixels.len() < len {
            self.pixels.resize(len, 0);
        }
        let pixels = &mut self.pixels[..len];
        if layout.is_grayscale() {
            let frame = Frame { left, top, width, height };
            if !self.coverage.draw(&self.edges, rule, frame, room, pixels, width) {
                return Err(TANGLED);
            }
        } else if let Some((kernel, taps, lo, reversed)) = layout.stripes().filter(|_| !exact) {
            let frame = Frame { left: corner(i64::from(left) * 3)?, top, width: width * 3, height };
            self.coverage.striped(&self.edges, frame, (&kernel[..taps], lo), pixels, width * 3);
            if reversed {
                pixels.as_chunks_mut::<3>().0.iter_mut().for_each(|p| p.swap(0, 2));
            }
        } else {
            let (ox, oy) = layout.oversample();
            let (mx, my) = layout.margin();
            let (across, down) = (width * usize::from(ox), height * usize::from(oy));
            // The frame takes in the filter's margin, so coverage writes its zeros too.
            let (stride, tall) = (across + 2 * mx, down + 2 * my);
            if self.samples.len() < stride * tall {
                self.samples.resize(stride * tall, 0);
            }
            let samples = &mut self.samples[..stride * tall];
            let sx = corner(i64::from(left) * i64::from(ox) - mx as i64)?;
            let sy = corner(i64::from(top) * i64::from(oy) + my as i64)?;
            let frame = Frame { left: sx, top: sy, width: stride, height: tall };
            if !self.coverage.draw(&self.edges, rule, frame, room, samples, stride) {
                return Err(TANGLED);
            }
            layout.filter(samples, stride, (width, height), pixels, &mut self.rows);
        }
        Ok(Drawn { left, top, width, height, channels })
    }
}
