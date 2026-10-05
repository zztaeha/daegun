use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::decoder::{read_i16_be, read_u16_be, read_u32_be, records_fit};
use crate::daecore::daetype::TableBytes;

#[derive(Clone, Debug, PartialEq)]
pub enum BitmapImage {
    // As the font stores it: daegun does not decode images.
    Png(Vec<u8>),
    // One byte of coverage per pixel, 0 to 255, rows top to bottom: a monochrome or grayscale strike.
    Coverage { width: u16, height: u16, pixels: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct GlyphBitmap {
    pub image: BitmapImage,
    pub ppem: u16,
    // The image's left and top edges, in pixels at `ppem` from the glyph's origin, y up.
    pub left: i16,
    pub top: i16,
    // To be drawn mirrored left to right, as Apple's sbix 'flip' asks.
    pub mirrored: bool,
}

// From sbix, CBDT or EBDT, the strike nearest the size first. A strike without the glyph defers to
// the next nearest, as the sbix spec asks: a glyph with a bitmap in any strike is drawn from one.
pub fn glyph_bitmap(table_map: &BTreeMap<String, TableBytes>, gid: u16, target_ppem: u16) -> Option<GlyphBitmap> {
    sbix_bitmap(table_map, gid, target_ppem)
        .or_else(|| strike_bitmap(table_map, "CBLC", "CBDT", gid, target_ppem))
        .or_else(|| strike_bitmap(table_map, "EBLC", "EBDT", gid, target_ppem))
}

fn num_glyphs(table_map: &BTreeMap<String, TableBytes>) -> usize {
    table_map.get("maxp").and_then(|m| read_u16_be(m, 4)).unwrap_or(0) as usize
}

// The order to try strikes in: the smallest at or above the size, larger ones after it, then the
// largest below it and smaller ones.
fn by_preference<T>(strikes: &mut [(u16, T)], target_ppem: u16) {
    strikes.sort_by_key(|&(ppem, _)| if ppem >= target_ppem { (0, i32::from(ppem)) } else { (1, -i32::from(ppem)) });
}

fn sbix_bitmap(table_map: &BTreeMap<String, TableBytes>, gid: u16, target_ppem: u16) -> Option<GlyphBitmap> {
    let sbix = table_map.get("sbix")?;
    let n_glyphs = num_glyphs(table_map);
    let num_strikes = read_u32_be(sbix, 4)? as usize;
    if !records_fit(8, num_strikes, 4, sbix.len()) { return None; }
    // A strike whose header cannot be read is passed over rather than failing the others.
    let mut strikes: Vec<(u16, usize)> = (0..num_strikes)
        .filter_map(|i| {
            let off = read_u32_be(sbix, 8 + i * 4)? as usize;
            Some((read_u16_be(sbix, off)?, off))
        })
        .collect();
    by_preference(&mut strikes, target_ppem);
    strikes.iter().find_map(|&(ppem, strike)| sbix_glyph(sbix, strike, ppem, gid, n_glyphs))
}

// 'dupe' and Apple's 'flip' hand over to another glyph's data, which FreeType follows four deep.
fn sbix_glyph(sbix: &[u8], strike: usize, ppem: u16, mut gid: u16, n_glyphs: usize) -> Option<GlyphBitmap> {
    let mut mirrored = false;
    for _ in 0..=4 {
        if usize::from(gid) >= n_glyphs { return None; }
        let offset = |g: usize| strike.checked_add(4 + g * 4).and_then(|at| read_u32_be(sbix, at));
        let (start, end) = (offset(usize::from(gid))? as usize, offset(usize::from(gid) + 1)? as usize);
        if end <= start { return None; }
        let data = sbix.get(strike.checked_add(start)?..strike.checked_add(end)?)?;
        if data.len() < 8 { return None; }
        match &data[4..8] {
            b"png " => {
                let png = &data[8..];
                // originOffsetY is the image's bottom edge; its top is the PNG's height above.
                let top = i32::from(read_i16_be(data, 2)?) + png_height(png)?;
                return Some(GlyphBitmap {
                    image: BitmapImage::Png(png.to_vec()),
                    ppem,
                    left: read_i16_be(data, 0)?,
                    top: top.clamp(i16::MIN.into(), i16::MAX.into()) as i16,
                    mirrored,
                });
            }
            b"flip" => {
                mirrored = !mirrored;
                gid = read_u16_be(data, 8)?;
            }
            b"dupe" => gid = read_u16_be(data, 8)?,
            _ => return None,
        }
    }
    None
}

// The height from a PNG's IHDR chunk, which the format requires to come first.
fn png_height(png: &[u8]) -> Option<i32> {
    if png.get(..8)? != b"\x89PNG\r\n\x1a\n" || png.get(12..16)? != b"IHDR" { return None; }
    i32::try_from(read_u32_be(png, 20)?).ok()
}

// Pixels a bitmap may decode, composites and components together: a component can name a large
// glyph many times over.
const MAX_PIXELS: usize = 1 << 24;
const MAX_NESTING: u8 = 16;

fn strike_bitmap(
    table_map: &BTreeMap<String, TableBytes>, loc_tag: &str, data_tag: &str, gid: u16, target_ppem: u16,
) -> Option<GlyphBitmap> {
    let (loc, data) = (table_map.get(loc_tag)?, table_map.get(data_tag)?);
    let num_sizes = read_u32_be(loc, 4)? as usize;
    if !records_fit(8, num_sizes, 48, loc.len()) { return None; }
    let mut strikes: Vec<(u16, usize)> = (0..num_sizes).map(|i| (u16::from(loc[8 + i * 48 + 44]), 8 + i * 48)).collect();
    by_preference(&mut strikes, target_ppem);
    strikes.iter().find_map(|&(ppem, st)| Strike::read(loc, data, st)?.bitmap(gid, ppem))
}

#[derive(Clone, Copy)]
struct Metrics {
    height: u8,
    width: u8,
    left: i8,
    top: i8,
}

impl Metrics {
    // Small and big metrics start alike: height, width, then the horizontal bearings.
    fn read(data: &[u8]) -> Option<Metrics> {
        let b = data.get(..4)?;
        Some(Metrics { height: b[0], width: b[1], left: b[2] as i8, top: b[3] as i8 })
    }
}

// A glyph's image data, its format, and the metrics its index subtable holds, if it holds any.
struct Located<'a> {
    format: u16,
    data: &'a [u8],
    metrics: Option<Metrics>,
}

struct Strike<'a> {
    loc: &'a [u8],
    data: &'a [u8],
    list: usize,
    count: usize,
    depth: u8,
}

impl<'a> Strike<'a> {
    fn read(loc: &'a [u8], data: &'a [u8], st: usize) -> Option<Strike<'a>> {
        let list = read_u32_be(loc, st)? as usize;
        let count = read_u32_be(loc, st + 8)? as usize;
        let depth = loc[st + 46];
        records_fit(list, count, 8, loc.len()).then_some(Strike { loc, data, list, count, depth })
    }

    fn locate(&self, gid: u16) -> Option<Located<'a>> {
        let loc = self.loc;
        let rec = (0..self.count).map(|i| self.list + i * 8).find(|&rec| {
            read_u16_be(loc, rec).is_some_and(|first| first <= gid) && read_u16_be(loc, rec + 2).is_some_and(|last| gid <= last)
        })?;
        let idx = usize::from(gid - read_u16_be(loc, rec)?);
        let ist = self.list.checked_add(read_u32_be(loc, rec + 4)? as usize)?;
        let (index_format, format) = (read_u16_be(loc, ist)?, read_u16_be(loc, ist + 2)?);
        let image_data = read_u32_be(loc, ist + 4)? as usize;
        let fixed = |size: usize, k: usize| Some((size.checked_mul(k)?, size.checked_mul(k + 1)?));
        let ((start, end), metrics) = match index_format {
            1 => ((read_u32_be(loc, ist + 8 + idx * 4)? as usize, read_u32_be(loc, ist + 12 + idx * 4)? as usize), None),
            2 => (fixed(read_u32_be(loc, ist + 8)? as usize, idx)?, Some(Metrics::read(loc.get(ist + 12..)?)?)),
            3 => ((usize::from(read_u16_be(loc, ist + 8 + idx * 2)?), usize::from(read_u16_be(loc, ist + 10 + idx * 2)?)), None),
            4 => {
                let n = read_u32_be(loc, ist + 8)? as usize;
                if !records_fit(ist + 12, n.checked_add(1)?, 4, loc.len()) { return None; }
                let pair = |k: usize| ist + 12 + k * 4;
                let k = (0..n).find(|&k| read_u16_be(loc, pair(k)) == Some(gid))?;
                ((usize::from(read_u16_be(loc, pair(k) + 2)?), usize::from(read_u16_be(loc, pair(k + 1) + 2)?)), None)
            }
            5 => {
                let n = read_u32_be(loc, ist + 20)? as usize;
                if !records_fit(ist + 24, n, 2, loc.len()) { return None; }
                let k = (0..n).find(|&k| read_u16_be(loc, ist + 24 + k * 2) == Some(gid))?;
                (fixed(read_u32_be(loc, ist + 8)? as usize, k)?, Some(Metrics::read(loc.get(ist + 12..)?)?))
            }
            _ => return None,
        };
        if end <= start { return None; }
        let data = self.data.get(image_data.checked_add(start)?..image_data.checked_add(end)?)?;
        Some(Located { format, data, metrics })
    }

    fn bitmap(&self, gid: u16, ppem: u16) -> Option<GlyphBitmap> {
        let at = self.locate(gid)?;
        let placed = |image, m: Metrics| GlyphBitmap { image, ppem, left: m.left.into(), top: m.top.into(), mirrored: false };
        match at.format {
            17..=19 => {
                let (metrics, len_at) = match at.format {
                    17 => (Metrics::read(at.data)?, 5),
                    18 => (Metrics::read(at.data)?, 8),
                    _ => (at.metrics?, 0),
                };
                let len = read_u32_be(at.data, len_at)? as usize;
                let png = at.data.get(len_at + 4..(len_at + 4).checked_add(len)?)?;
                Some(placed(BitmapImage::Png(png.to_vec()), metrics))
            }
            _ => {
                let mut budget = MAX_PIXELS;
                let (m, pixels) = self.coverage(&at, 0, &mut budget)?;
                let image = BitmapImage::Coverage { width: m.width.into(), height: m.height.into(), pixels };
                Some(placed(image, m))
            }
        }
    }

    // A raw image as one byte a pixel; a composite draws its components into its own box at their
    // offsets, as FreeType does, clipped to it.
    fn coverage(&self, at: &Located, level: u8, budget: &mut usize) -> Option<(Metrics, Vec<u8>)> {
        let (m, body) = match at.format {
            1 | 2 | 8 => (Metrics::read(at.data)?, at.data.get(5..)?),
            6 | 7 | 9 => (Metrics::read(at.data)?, at.data.get(8..)?),
            5 => (at.metrics?, at.data),
            _ => return None,
        };
        let (w, h) = (usize::from(m.width), usize::from(m.height));
        *budget = budget.checked_sub(w * h)?;
        let mut pixels = alloc::vec![0u8; w * h];
        if matches!(at.format, 8 | 9) {
            let body = if at.format == 8 { body.get(1..)? } else { body };
            let n = usize::from(read_u16_be(body, 0)?);
            if level >= MAX_NESTING || !records_fit(2, n, 4, body.len()) { return None; }
            for c in 0..n {
                let rec = 2 + c * 4;
                let Some(part) = read_u16_be(body, rec).and_then(|g| self.locate(g)) else { continue };
                let Some((pm, pp)) = self.coverage(&part, level + 1, budget) else { continue };
                let (dx, dy) = (isize::from(body[rec + 2] as i8), isize::from(body[rec + 3] as i8));
                for (row, line) in pp.chunks_exact(usize::from(pm.width).max(1)).enumerate() {
                    let Ok(y) = usize::try_from(row as isize + dy) else { continue };
                    for (col, &v) in line.iter().enumerate() {
                        let Ok(x) = usize::try_from(col as isize + dx) else { continue };
                        if x < w && y < h {
                            pixels[y * w + x] = pixels[y * w + x].max(v);
                        }
                    }
                }
            }
            return Some((m, pixels));
        }

        // Raw data is 1, 2, 4 or 8 bits a pixel; a color strike's 32 is for PNG only.
        if !matches!(self.depth, 1 | 2 | 4 | 8) { return None; }
        let depth = usize::from(self.depth);
        let row_bits = w * depth;
        let byte_rows = h * row_bits.div_ceil(8);
        // Formats 2 and 7 are bit-aligned, but some fonts (AppleMyungJo) store byte-aligned rows
        // under them; FreeType takes a body of exactly the byte-aligned size as that.
        let byte_aligned = match at.format {
            1 | 6 => true,
            2 | 7 => (h * row_bits).div_ceil(8) < byte_rows && body.len() == byte_rows,
            _ => false,
        };
        let need = if byte_aligned { byte_rows } else { (h * row_bits).div_ceil(8) };
        if body.len() < need { return None; }
        let max = (1u32 << depth) - 1;
        for y in 0..h {
            let row_start = if byte_aligned { y * row_bits.div_ceil(8) * 8 } else { y * row_bits };
            for x in 0..w {
                let bit = row_start + x * depth;
                let v = u32::from(body[bit / 8] >> (8 - depth - bit % 8)) & max;
                pixels[y * w + x] = (v * 255 / max) as u8;
            }
        }
        Some((m, pixels))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(tables: &[(&str, Vec<u8>)]) -> BTreeMap<String, TableBytes> {
        tables.iter().map(|(t, b)| (String::from(*t), TableBytes::from(b.clone()))).collect()
    }

    fn maxp(n: u16) -> Vec<u8> {
        [&[0, 0, 0x50, 0][..], &n.to_be_bytes()].concat()
    }

    // A PNG as far as its IHDR, which is all daegun reads of one.
    fn png(height: u32) -> Vec<u8> {
        [&b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR"[..], &7u32.to_be_bytes(), &height.to_be_bytes(), &[0; 9]].concat()
    }

    // An sbix of three glyphs: each strike a ppem and, per glyph, its record or none.
    fn sbix(strikes: &[(u16, [Option<Vec<u8>>; 3])]) -> Vec<u8> {
        let mut out = [1u16, 1].map(u16::to_be_bytes).concat();
        out.extend((strikes.len() as u32).to_be_bytes());
        let mut at = out.len() + strikes.len() * 4;
        let mut bodies = Vec::new();
        for (ppem, glyphs) in strikes {
            out.extend((at as u32).to_be_bytes());
            let mut body = [ppem.to_be_bytes(), 72u16.to_be_bytes()].concat();
            let mut offset = 4 + 4 * 4;
            let mut data: Vec<u8> = Vec::new();
            for g in glyphs {
                body.extend((offset as u32).to_be_bytes());
                if let Some(g) = g {
                    data.extend(g);
                    offset += g.len();
                }
            }
            body.extend((offset as u32).to_be_bytes());
            body.extend(data);
            at += body.len();
            bodies.push(body);
        }
        out.extend(bodies.concat());
        out
    }

    fn record(x: i16, y: i16, kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        [&x.to_be_bytes()[..], &y.to_be_bytes(), kind, data].concat()
    }

    // An sbix image's top edge is its bottom (originOffsetY) plus its height, as CBDT's bearingY is
    // its top: the two read alike. 'dupe' and 'flip' hand over to another glyph's image.
    #[test]
    fn sbix_places_dupes_flips_and_tops_as_freetype_does() {
        let image = record(4, -27, b"png ", &png(128));
        let t = map(&[("maxp", maxp(3)), ("sbix", sbix(&[(109, [None, Some(image), Some(record(0, 0, b"dupe", &1u16.to_be_bytes()))])]))]);
        let b = glyph_bitmap(&t, 1, 109).expect("glyph 1");
        assert_eq!((b.left, b.top, b.mirrored, b.ppem), (4, 101, false, 109));
        let dupe = glyph_bitmap(&t, 2, 109).expect("glyph 2, a dupe of 1");
        assert_eq!(dupe, b);

        let image = record(4, -27, b"png ", &png(128));
        let t = map(&[("maxp", maxp(3)), ("sbix", sbix(&[(109, [None, Some(image), Some(record(0, 0, b"flip", &1u16.to_be_bytes()))])]))]);
        assert!(glyph_bitmap(&t, 2, 109).expect("glyph 2, a flip of 1").mirrored);
    }

    // A strike without the glyph defers to the next nearest, and a strike whose header cannot be
    // read is passed over rather than failing the font.
    #[test]
    fn another_strike_serves_a_glyph_the_nearest_lacks() {
        let image = || Some(record(0, 0, b"png ", &png(20)));
        let mut table = sbix(&[(40, [None, None, None]), (20, [None, image(), None])]);
        let t = map(&[("maxp", maxp(3)), ("sbix", table.clone())]);
        assert_eq!(glyph_bitmap(&t, 1, 40).map(|b| b.ppem), Some(20));
        table[8..12].copy_from_slice(&0xFFFF_FFF0u32.to_be_bytes());
        let t = map(&[("maxp", maxp(3)), ("sbix", table)]);
        assert_eq!(glyph_bitmap(&t, 1, 40).map(|b| b.ppem), Some(20), "a broken strike failed the rest");
    }

    // An EBLC of one strike at `depth` bits, with these index subtables (first glyph, last glyph,
    // the subtable's bytes after its 8-byte header, its index and image formats).
    fn eblc(depth: u8, subtables: &[(u16, u16, u16, u16, Vec<u8>)]) -> Vec<u8> {
        let list = 8 + 48;
        let mut out = [0x0002_0000u32, 1].map(u32::to_be_bytes).concat();
        let mut size = Vec::new();
        size.extend((list as u32).to_be_bytes());
        size.extend([0; 4]);
        size.extend((subtables.len() as u32).to_be_bytes());
        size.extend([0; 4 + 24]);
        size.extend([0, 0, 0xFF, 0xFF, 16, 16, depth, 1]);
        out.extend(size);
        let mut at = subtables.len() * 8;
        let mut bodies = Vec::new();
        for (first, last, index_format, image_format, rest) in subtables {
            out.extend([first.to_be_bytes(), last.to_be_bytes()].concat());
            out.extend((at as u32).to_be_bytes());
            let mut body = [index_format.to_be_bytes(), image_format.to_be_bytes()].concat();
            body.extend(4u32.to_be_bytes());
            body.extend(rest);
            at += body.len();
            bodies.push(body);
        }
        out.extend(bodies.concat());
        out
    }

    fn coverage(b: &GlyphBitmap) -> (u16, u16, &[u8]) {
        match &b.image {
            BitmapImage::Coverage { width, height, pixels } => (*width, *height, pixels),
            BitmapImage::Png(_) => panic!("a PNG"),
        }
    }

    // EBDT's raw formats decoded: byte-aligned rows (1), bit-aligned (2), metrics from a format 2
    // index (5) at 2 bits a pixel, and a composite (8) drawing two glyphs into its own box.
    #[test]
    fn ebdt_strikes_decode_to_coverage() {
        let small = |h: u8, w: u8, x: i8, y: i8| alloc::vec![h, w, x as u8, y as u8, w];
        let g1 = [small(2, 3, 1, 5), alloc::vec![0b1010_0000, 0b0100_0000]].concat();
        let g2 = [small(2, 3, 0, 2), alloc::vec![0b1010_1000]].concat();
        let g3 = [small(2, 4, 0, 5), alloc::vec![0, 0, 2], 1u16.to_be_bytes().to_vec(), alloc::vec![0, 0], 2u16.to_be_bytes().to_vec(), alloc::vec![1, 0]].concat();
        let ebdt = [&[0, 2, 0, 0][..], &g1, &g2, &g3].concat();
        let offsets = |sizes: &[usize]| -> Vec<u8> {
            let mut at = 0;
            let mut out = Vec::new();
            for s in sizes {
                out.extend((at as u32).to_be_bytes());
                at += s;
            }
            out.extend((at as u32).to_be_bytes());
            out
        };
        let mut rest = offsets(&[g1.len()]);
        let loc = eblc(1, &[(1, 1, 1, 1, rest.clone())]);
        let t = map(&[("EBLC", loc), ("EBDT", ebdt.clone())]);
        let b = glyph_bitmap(&t, 1, 16).expect("glyph 1");
        assert_eq!(coverage(&b), (3, 2, &[255, 0, 255, 0, 255, 0][..]));
        assert_eq!((b.left, b.top), (1, 5));

        // Glyphs 2 and 3 follow glyph 1's data, so their offsets start past it.
        rest = offsets(&[g2.len(), g3.len()]).chunks(4).map(|c| u32::from_be_bytes(c.try_into().unwrap()) + g1.len() as u32).flat_map(u32::to_be_bytes).collect();
        let loc = eblc(1, &[(1, 1, 1, 1, offsets(&[g1.len()])), (2, 2, 1, 2, rest[..8].to_vec()), (3, 3, 1, 8, rest[4..].to_vec())]);
        let t = map(&[("EBLC", loc), ("EBDT", ebdt)]);
        assert_eq!(coverage(&glyph_bitmap(&t, 2, 16).expect("glyph 2")), (3, 2, &[255, 0, 255, 0, 255, 0][..]));
        let composite = glyph_bitmap(&t, 3, 16).expect("glyph 3");
        assert_eq!(coverage(&composite), (4, 2, &[255, 255, 255, 255, 0, 255, 255, 0][..]));

        // Two bits a pixel, metrics in a format 2 index: 3 and 1 of 3 are 255 and 85.
        let big = alloc::vec![1, 2, 0xFE, 4, 2, 0, 0, 0];
        let loc = eblc(2, &[(7, 9, 2, 5, [&1u32.to_be_bytes()[..], &big].concat())]);
        let t = map(&[("EBLC", loc), ("EBDT", alloc::vec![0, 2, 0, 0, 0, 0b1101_0000])]);
        let b = glyph_bitmap(&t, 8, 16).expect("glyph 8");
        assert_eq!((coverage(&b), b.left, b.top), ((2, 1, &[255, 85][..]), -2, 4));
    }

    // Sparse index formats: 4 pairs glyphs with offsets, 5 lists glyphs of one size and metrics, and
    // a CBDT format 19 PNG takes its placement from that index.
    #[test]
    fn sparse_indexes_and_format_19_find_their_glyphs() {
        let image = png(6);
        let data = [&[0u8, 3, 0, 0][..], &(image.len() as u32).to_be_bytes(), &image].concat();
        let size = (data.len() - 4) as u32;
        let big = alloc::vec![6, 7, 2, 9, 8, 0, 0, 0];
        let five = [&size.to_be_bytes()[..], &big, &1u32.to_be_bytes(), &40u16.to_be_bytes(), &[0, 0]].concat();
        let t = map(&[("CBLC", eblc(32, &[(10, 50, 5, 19, five)])), ("CBDT", data)]);
        let b = glyph_bitmap(&t, 40, 16).expect("glyph 40");
        assert_eq!((&b.image, b.left, b.top), (&BitmapImage::Png(image.clone()), 2, 9));
        assert!(glyph_bitmap(&t, 41, 16).is_none(), "a glyph the index leaves out");

        let small = [&[0u8, 3, 0, 0][..], &[6, 7, 1, 3, 7], &(image.len() as u32).to_be_bytes(), &image].concat();
        let pairs = [30u16, 0, 31, (small.len() - 4) as u16].map(u16::to_be_bytes).concat();
        let t2 = map(&[("CBLC", eblc(32, &[(30, 31, 4, 17, [&1u32.to_be_bytes()[..], &pairs].concat())])), ("CBDT", small)]);
        let b = glyph_bitmap(&t2, 30, 16).expect("glyph 30");
        assert_eq!((b.left, b.top), (1, 3));
    }
}
