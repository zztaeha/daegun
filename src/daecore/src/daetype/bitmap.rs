#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::vec::Vec;
use super::decoder::{clamp_i16, num_glyphs, read_i16_be, read_u16_be, read_u32_be};
use super::format::{eblc, sbix};
use crate::daecore::daetype::Tables;

/// A bitmap glyph's image.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub enum BitmapImage<'a> {
    /// A PNG, as the font stores it, borrowed from the font: daegun doesn't decode images.
    Png(&'a [u8]),
    /// A monochrome or grayscale strike's image, decoded.
    Coverage {
        /// Its width in pixels.
        width: u16,
        /// Its height in pixels.
        height: u16,
        /// One byte of coverage a pixel, 0 to 255, rows top to bottom.
        pixels: Vec<u8>,
    },
}

/// A glyph's bitmap from the strike nearest the size asked for, and where it sits.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphBitmap<'a> {
    /// The image.
    pub image: BitmapImage<'a>,
    /// The strike's size in pixels per em, which may not be the size asked for.
    pub ppem: u16,
    /// The image's left edge, in pixels at `ppem` right of the glyph's origin.
    pub left: i16,
    /// The image's top edge, in pixels at `ppem` above the glyph's origin.
    pub top: i16,
    /// Whether to draw it mirrored left to right, as Apple's sbix 'flip' asks.
    pub mirrored: bool,
}

// The tables holding strikes, location and data, in the order they are looked for: Apple's bloc and bdat
// are EBLC and EBDT under older names.
const STRIKE_TABLES: [(&str, &str); 3] = [("CBLC", "CBDT"), ("EBLC", "EBDT"), ("bloc", "bdat")];

// From sbix, CBDT, EBDT or bdat, the strike nearest the size first. A strike without the glyph defers to
// the next nearest, as the sbix spec asks: a glyph with a bitmap in any strike is drawn from one.
pub fn glyph_bitmap(table_map: &Tables, gid: u32, target_ppem: u16) -> Option<GlyphBitmap<'_>> {
    sbix_bitmap(table_map, gid, target_ppem).or_else(|| {
        STRIKE_TABLES.iter().find_map(|&(loc, data)| strike_bitmap(table_map, loc, data, gid, target_ppem))
    })
}

// The first answer `f` gives over strikes nearest first: at or above the size ascending, then below it
// descending. The first serves most glyphs, so only a glyph it lacks pays for sorting the rest.
fn nearest_first<T: Copy, R>(
    strikes: impl Iterator<Item = (u16, T)> + Clone, target_ppem: u16, mut f: impl FnMut(u16, T) -> Option<R>,
) -> Option<R> {
    let rank = |&(ppem, _): &(u16, T)| if ppem >= target_ppem { u32::from(ppem) } else { 0x1_0000 | u32::from(!ppem) };
    let (ppem, strike) = strikes.clone().min_by_key(rank)?;
    if let Some(found) = f(ppem, strike) {
        return Some(found);
    }
    let mut rest: Vec<(u16, T)> = strikes.collect();
    rest.sort_by_key(rank);
    rest.into_iter().skip(1).find_map(|(ppem, strike)| f(ppem, strike))
}

// A color glyph's image box in font units, (x_bearing, y_bearing, width, height) with y up and the height
// negative: sbix, else CBDT, at the largest strike holding the glyph.
pub(crate) fn color_bitmap_extents(table_map: &Tables, gid: u32, upm: u16) -> Option<[i32; 4]> {
    let scaled = |v: f64, ppem: u16| (v * f64::from(upm) / f64::from(ppem)).round() as i32;
    if let Some((ppem, image)) = sbix_image(table_map, gid, u16::MAX).filter(|&(ppem, _)| ppem != 0) {
        let s = |v: f64| scaled(v, ppem);
        let (width, height) = (f64::from(image.width), f64::from(image.height));
        return Some([s(image.left.into()), s(f64::from(image.bottom) + height), s(width), s(-height)]);
    }
    strike_glyph(table_map, &STRIKE_TABLES[..1], gid, upm).map(|g| g.extents)
}

// A monochrome or grayscale glyph's box as `color_bitmap_extents` reads a color one, from EBDT or bdat: the
// extents of a glyph with no outline.
pub(crate) fn mono_bitmap_extents(table_map: &Tables, gid: u32, upm: u16) -> Option<[i32; 4]> {
    strike_glyph(table_map, &STRIKE_TABLES[1..], gid, upm).map(|g| g.extents)
}

// A font with no outline table, which draws from its strikes alone, as Apple's bitmap fonts do. Only such a
// font is sized by its strikes; an outline font's metrics stay its own.
pub(crate) fn bitmap_only(table_map: &Tables) -> bool {
    !["glyf", "GLYF", "CFF ", "CFF2"].into_iter().any(|t| table_map.contains_key(t))
}

// The advance a glyph's bitmap states, 0 included, in font units: what a glyph of a bitmap-only font
// without hmtx, or vmtx, advances by. None for a glyph no strike holds, and down for small metrics.
pub(crate) fn bitmap_advance(
    table_map: &Tables, gid: u32, upm: u16, vertical: bool,
) -> Option<u32> {
    let g = strike_glyph(table_map, &STRIKE_TABLES, gid, upm)?;
    u32::try_from(if vertical { g.vertical_advance? } else { g.advance }).ok()
}

// The largest strike's ascender and descender in font units, as its sbitLineMetrics state them.
pub(crate) fn strike_line_extents(
    table_map: &Tables, upm: u16, vertical: bool,
) -> Option<(i32, i32)> {
    let loc = STRIKE_TABLES.iter().find_map(|&(loc, _)| table_map.get(loc))?;
    let size = eblc::bitmap_sizes(loc)?.iter().max_by_key(|size| eblc::ppem(size).1)?;
    let (x_ppem, y_ppem) = eblc::ppem(size);
    let (line, ppem) = if vertical { (&size[28..40], x_ppem) } else { (&size[16..28], y_ppem) };
    if ppem == 0 { return None; }
    let units = |at: usize| (f64::from(line[at] as i8) * f64::from(upm) / f64::from(ppem)).round() as i32;
    Some((units(0), units(1)))
}

// A glyph's metrics in font units at the largest strike holding it, from the first of `pairs` that does.
struct StrikeGlyph {
    // x_bearing, y_bearing, width and height, y up and the height negative.
    extents: [i32; 4],
    advance: i32,
    vertical_advance: Option<i32>,
}

fn strike_glyph(
    table_map: &Tables, pairs: &[(&str, &str)], gid: u32, upm: u16,
) -> Option<StrikeGlyph> {
    pairs.iter().find_map(|&(loc_tag, data_tag)| {
        let (loc, data) = (table_map.get(loc_tag)?, table_map.get(data_tag)?);
        let strikes = eblc::bitmap_sizes(loc)?.iter().map(|size| (u16::from(eblc::ppem(size).0), size));
        let mut work = WORK_PER_BYTE.saturating_mul(loc.len() + data.len());
        nearest_first(strikes, u16::MAX, |_, size| {
            let (x_ppem, y_ppem) = eblc::ppem(size);
            if x_ppem == 0 || y_ppem == 0 { return None; }
            let (m, _) = Strike { loc, data, size }.locate(gid, &mut work)?.metrics()?;
            let x = |v: f64| (v * f64::from(upm) / f64::from(x_ppem)).round() as i32;
            let y = |v: f64| (v * f64::from(upm) / f64::from(y_ppem)).round() as i32;
            Some(StrikeGlyph {
                extents: [x(m.left.into()), y(m.top.into()), x(m.width.into()), y(-f64::from(m.height))],
                advance: x(m.advance.into()),
                vertical_advance: m.vertical_advance.map(|v| y(v.into())),
            })
        })
    })
}

fn sbix_bitmap(table_map: &Tables, gid: u32, target_ppem: u16) -> Option<GlyphBitmap<'_>> {
    let (ppem, image) = sbix_image(table_map, gid, target_ppem)?;
    Some(GlyphBitmap {
        image: BitmapImage::Png(image.png),
        ppem,
        left: image.left,
        top: clamp_i16(i32::from(image.bottom).saturating_add_unsigned(image.height)),
        mirrored: image.mirrored,
    })
}

// A glyph's PNG as an sbix strike holds it: its left and bottom edges (originOffsetX and Y) and its size.
struct SbixImage<'a> {
    png: &'a [u8],
    left: i16,
    bottom: i16,
    width: u32,
    height: u32,
    mirrored: bool,
}

fn sbix_image(table_map: &Tables, gid: u32, target_ppem: u16) -> Option<(u16, SbixImage<'_>)> {
    let sbix = table_map.get("sbix")?;
    let n_glyphs = num_glyphs(table_map).map_or(0, |n| n as usize);
    let strikes = sbix::strikes(sbix)?.filter_map(|strike| Some((read_u16_be(sbix, strike)?, strike)));
    nearest_first(strikes, target_ppem, |ppem, strike| Some((ppem, sbix_glyph(sbix, strike, gid, n_glyphs)?)))
}

// 'dupe' and Apple's 'flip' hand over to another glyph's data, followed four deep.
fn sbix_glyph(sbix: &[u8], strike: usize, mut gid: u32, n_glyphs: usize) -> Option<SbixImage<'_>> {
    let mut mirrored = false;
    for _ in 0..=4 {
        let data = sbix::record(sbix, strike, n_glyphs, gid)?;
        match &data[4..8] {
            b"png " => {
                let png = &data[8..];
                let (width, height) = png_size(png)?;
                return Some(SbixImage {
                    png,
                    left: read_i16_be(data, 0)?,
                    bottom: read_i16_be(data, 2)?,
                    width,
                    height,
                    mirrored,
                });
            }
            b"flip" => {
                mirrored = !mirrored;
                gid = read_u16_be(data, 8)?.into();
            }
            b"dupe" => gid = read_u16_be(data, 8)?.into(),
            _ => return None,
        }
    }
    None
}

// The width and height from a PNG's IHDR chunk, which the format requires to come first.
fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.get(..8)? != b"\x89PNG\r\n\x1a\n" || png.get(12..16)? != b"IHDR" { return None; }
    let fits = |v: u32| i32::try_from(v).is_ok();
    Some((read_u32_be(png, 16)?, read_u32_be(png, 20)?)).filter(|&(w, h)| fits(w) && fits(h))
}

// Work one bitmap may do for each byte of its strike tables, over every strike it's looked for in: a pixel decoded or
// an index record scanned is one. Composites naming composites can draw a glyph exponentially often, and strikes can
// share one long array of index records, so the input bounds it; a glyph's own pixels take 8 a byte at most.
const WORK_PER_BYTE: usize = 64;
// A component costs the 4 bytes of its record, so a bitmap visits no more components than the tables hold.
const COMPONENT_WORK: usize = 4 * WORK_PER_BYTE;

fn strike_bitmap<'t>(
    table_map: &'t Tables, loc_tag: &str, data_tag: &str, gid: u32, target_ppem: u16,
) -> Option<GlyphBitmap<'t>> {
    let (loc, data) = (table_map.get(loc_tag)?, table_map.get(data_tag)?);
    let strikes = eblc::bitmap_sizes(loc)?.iter().map(|size| (u16::from(eblc::ppem(size).0), size));
    let mut work = WORK_PER_BYTE.saturating_mul(loc.len() + data.len());
    nearest_first(strikes, target_ppem, |ppem, size| Strike { loc, data, size }.bitmap(gid, ppem, &mut work))
}

#[derive(Clone, Copy)]
struct Metrics {
    height: u8,
    width: u8,
    left: i8,
    top: i8,
    advance: u8,
    // Big metrics' alone: small metrics state no advance down.
    vertical_advance: Option<u8>,
}

impl Metrics {
    // Small and big metrics start alike: height, width, the horizontal bearings and advance. Only big metrics
    // go on to the vertical ones.
    fn read(data: &[u8], big: bool) -> Option<Metrics> {
        let b = data.get(..if big { 8 } else { 5 })?;
        let vertical_advance = big.then(|| b[7]);
        Some(Metrics { height: b[0], width: b[1], left: b[2] as i8, top: b[3] as i8, advance: b[4], vertical_advance })
    }
}

// A glyph's image data, its format, and the metrics its index subtable holds, if it holds any.
struct Located<'a> {
    format: u16,
    data: &'a [u8],
    metrics: Option<Metrics>,
}

impl Located<'_> {
    // The glyph's metrics, from its data or its index subtable, and where the bytes past them start.
    fn metrics(&self) -> Option<(Metrics, usize)> {
        match self.format {
            1 | 2 | 8 | 17 => Some((Metrics::read(self.data, false)?, 5)),
            6 | 7 | 9 | 18 => Some((Metrics::read(self.data, true)?, 8)),
            5 | 19 => Some((self.metrics?, 0)),
            _ => None,
        }
    }
}

struct Strike<'a> {
    loc: &'a [u8],
    data: &'a [u8],
    size: &'a [u8; 48],
}

impl<'a> Strike<'a> {
    // A glyph's image as the first index subtable whose range covers the glyph holds it, or none.
    fn locate(&self, gid: u32, work: &mut usize) -> Option<Located<'a>> {
        let records = eblc::index_records(self.loc, self.size)?;
        *work = work.checked_sub(records.count())?;
        let (first, last, at) = records.covering(gid)?;
        let index = eblc::IndexSubtable::new(self.loc, self.data, at, first, last)?;
        let data = index.locate(gid, work)?;
        let metrics = index.fixed().and_then(|(_, m)| Metrics::read(m, true));
        Some(Located { format: index.image_format, data, metrics })
    }

    fn bitmap(&self, gid: u32, ppem: u16, work: &mut usize) -> Option<GlyphBitmap<'a>> {
        let at = self.locate(gid, work)?;
        let (m, image) = if (17..=19).contains(&at.format) {
            let (m, len_at) = at.metrics()?;
            let len = read_u32_be(at.data, len_at)? as usize;
            let png = at.data.get(len_at + 4..(len_at + 4).checked_add(len)?)?;
            (m, BitmapImage::Png(png))
        } else {
            let (m, pixels) = self.coverage(&at, gid, work)?;
            (m, BitmapImage::Coverage { width: m.width.into(), height: m.height.into(), pixels })
        };
        Some(GlyphBitmap { image, ppem, left: m.left.into(), top: m.top.into(), mirrored: false })
    }

    // A raw image as one byte a pixel. A composite draws its components straight into its box at their offsets, each
    // clipped to every composite it's inside, as drawing each into its own composite first would; maxima commute, so
    // the order doesn't matter. A component naming a composite it's already inside is passed over.
    fn coverage(&self, at: &Located<'a>, gid: u32, work: &mut usize) -> Option<(Metrics, Vec<u8>)> {
        let (m, image) = self.image(at)?;
        let (w, h) = (usize::from(m.width), usize::from(m.height));
        *work = work.checked_sub(w * h)?;
        let mut pixels = alloc::vec![0u8; w * h];
        let mut parts = match image {
            Image::Raw(body) => {
                self.draw_raw(at.format, m, body, (0, 0), [0, 0, w as isize, h as isize], &mut pixels, w)?;
                return Some((m, pixels));
            }
            Image::Composite => {
                alloc::vec![(eblc::components(at.format, at.data)?, (0, 0), [0, 0, w as isize, h as isize], gid)]
            }
        };
        // Components name 16-bit glyphs, so the glyphs on the path are a bit each of 2^16.
        let mut inside = alloc::vec![0u64; 1 << 10];
        let flip = |inside: &mut [u64], g: u32| {
            if let Some(word) = inside.get_mut(g as usize >> 6) {
                *word ^= 1 << (g & 63);
            }
        };
        flip(&mut inside, gid);
        while let Some((next, origin, clip, _)) = parts.last_mut() {
            let Some((_, g, dx, dy)) = next.next() else {
                let (.., g) = parts.pop()?;
                flip(&mut inside, g);
                continue;
            };
            let Some(left) = work.checked_sub(COMPONENT_WORK) else { break };
            *work = left;
            let Some(part) = self.locate(g.into(), work) else { continue };
            let Some((pm, image)) = self.image(&part) else { continue };
            let at = (origin.0 + isize::from(dx), origin.1 + isize::from(dy));
            let (pw, ph) = (isize::from(pm.width), isize::from(pm.height));
            let clip = [clip[0].max(at.0), clip[1].max(at.1), clip[2].min(at.0 + pw), clip[3].min(at.1 + ph)];
            if clip[0] >= clip[2] || clip[1] >= clip[3] {
                continue;
            }
            match image {
                Image::Composite if inside[usize::from(g) >> 6] & 1 << (g & 63) != 0 => {}
                Image::Composite => {
                    let Some(next) = eblc::components(part.format, part.data) else { continue };
                    flip(&mut inside, g.into());
                    parts.push((next, at, clip, g.into()));
                }
                Image::Raw(body) => {
                    let shown = ((clip[2] - clip[0]) * (clip[3] - clip[1])) as usize;
                    let Some(left) = work.checked_sub(shown) else { break };
                    *work = left;
                    let _ = self.draw_raw(part.format, pm, body, at, clip, &mut pixels, w);
                }
            }
        }
        Some((m, pixels))
    }

    // An image's metrics and what it draws: its raw bits or, a composite, its components. None for a PNG, metrics
    // that don't read, or a component list running past the image.
    fn image(&self, at: &Located<'a>) -> Option<(Metrics, Image<'a>)> {
        if (17..=19).contains(&at.format) {
            return None;
        }
        let (m, body_at) = at.metrics()?;
        Some((m, match at.format {
            8 | 9 => eblc::components(at.format, at.data).map(|_| Image::Composite)?,
            _ => Image::Raw(at.data.get(body_at..)?),
        }))
    }

    // A raw image's pixels within `clip`, placed with its top left at `at` in an image `w` wide, each the greater of
    // it and what's there. None, drawing nothing, where the strike's depth or the image's bytes don't decode.
    #[allow(clippy::too_many_arguments)]
    fn draw_raw(
        &self, format: u16, m: Metrics, body: &[u8], at: (isize, isize), clip: [isize; 4], pixels: &mut [u8], w: usize,
    ) -> Option<()> {
        // Raw data is 1, 2, 4 or 8 bits a pixel; a color strike's 32 is for PNG only.
        let depth = self.size[46];
        if !matches!(depth, 1 | 2 | 4 | 8) { return None; }
        let depth = usize::from(depth);
        let (pw, ph) = (usize::from(m.width), usize::from(m.height));
        let row_bits = pw * depth;
        let byte_rows = ph * row_bits.div_ceil(8);
        // Formats 2 and 7 are bit-aligned, but some fonts (AppleMyungJo) store byte-aligned rows
        // under them; a body of exactly the byte-aligned size is taken as that.
        let byte_aligned = match format {
            1 | 6 => true,
            2 | 7 => (ph * row_bits).div_ceil(8) < byte_rows && body.len() == byte_rows,
            _ => false,
        };
        let need = if byte_aligned { byte_rows } else { (ph * row_bits).div_ceil(8) };
        if body.len() < need { return None; }
        let max = (1u32 << depth) - 1;
        for y in clip[1]..clip[3] {
            let row = (y - at.1) as usize;
            let row_start = if byte_aligned { row * row_bits.div_ceil(8) * 8 } else { row * row_bits };
            for x in clip[0]..clip[2] {
                let bit = row_start + (x - at.0) as usize * depth;
                let v = u32::from(body[bit / 8] >> (8 - depth - bit % 8)) & max;
                let p = &mut pixels[y as usize * w + x as usize];
                *p = (*p).max((v * 255 / max) as u8);
            }
        }
        Some(())
    }
}

enum Image<'a> {
    Raw(&'a [u8]),
    Composite,
}

#[cfg(test)]
#[path = "../../tests/unit/daetype/bitmap.rs"]
mod tests;
