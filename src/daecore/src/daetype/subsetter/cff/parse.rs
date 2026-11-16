use alloc::vec::Vec;
use core::ops::Range;
use crate::daecore::daetype::decoder::{read_u16_be, read_u32_be};
use crate::daecore::daetype::format::cff::{read_dict, Dict, DictKind};
use crate::daecore::Error;

// An INDEX's count and offSize, and where its offsets and data start (both its end when it is empty).
fn index_header(data: &[u8], off: usize, count_is_32bit: bool) -> Result<(usize, usize, usize, usize), Error> {
    if data.len() > u32::MAX as usize {
        return Err(Error::malformed("CFF INDEX: table larger than a CFF table can be"));
    }
    let count_size = if count_is_32bit { 4 } else { 2 };
    if off.checked_add(count_size).is_none_or(|end| end > data.len()) {
        return Err(Error::malformed("CFF INDEX: truncated"));
    }
    let count = if count_is_32bit {
        read_u32_be(data, off).ok_or(Error::malformed("CFF INDEX: truncated"))? as usize
    } else {
        read_u16_be(data, off).ok_or(Error::malformed("CFF INDEX: truncated"))? as usize
    };
    if count == 0 {
        return Ok((0, 0, off + count_size, off + count_size));
    }
    if off + count_size + 1 > data.len() {
        return Err(Error::missing("CFF INDEX: missing offSize"));
    }
    let off_size = data[off + count_size] as usize;
    if !(1..=4).contains(&off_size) {
        return Err(Error::owned(
            crate::daecore::ErrorKind::Malformed,
            format!("CFF INDEX: invalid offSize {}", off_size),
        ));
    }
    let offsets_start = off + count_size + 1;

    let offsets_count = count.checked_add(1).ok_or(Error::malformed("CFF INDEX: count overflows"))?;
    let data_start = offsets_count
        .checked_mul(off_size)
        .and_then(|len| offsets_start.checked_add(len))
        .ok_or(Error::malformed("CFF INDEX: offsets array overflows the address space"))?;
    if data_start > data.len() {
        return Err(Error::malformed("CFF INDEX: offsets array does not fit in the buffer"));
    }
    Ok((count, off_size, offsets_start, data_start))
}

// `index_header` holds `off_size` to 1 through 4; read by width rather than byte by byte, as drawing reads two a glyph.
fn index_offset(data: &[u8], at: usize, off_size: usize) -> usize {
    let b = &data[at..at + off_size];
    match *b {
        [a] => usize::from(a),
        [a, b] => usize::from(u16::from_be_bytes([a, b])),
        [a, b, c] => usize::from(a) << 16 | usize::from(b) << 8 | usize::from(c),
        [a, b, c, d] => u32::from_be_bytes([a, b, c, d]) as usize,
        _ => b.iter().fold(0, |v, &x| (v << 8) | usize::from(x)),
    }
}

// Where an INDEX ends, read from its last offset alone, for one whose objects are not wanted.
pub fn cff_index_end(data: &[u8], off: usize, count_is_32bit: bool) -> Result<usize, Error> {
    let (count, off_size, offsets_start, data_start) = index_header(data, off, count_is_32bit)?;
    if count == 0 {
        return Ok(data_start);
    }
    let data_len = index_offset(data, offsets_start + count * off_size, off_size).saturating_sub(1);
    data_start
        .checked_add(data_len)
        .filter(|&end| end <= data.len())
        .ok_or(Error::malformed("CFF INDEX: data truncated"))
}

// An INDEX's object count, from its header alone, whose offsets the bytes hold.
pub(crate) fn cff_index_count(data: &[u8], off: usize, count_is_32bit: bool) -> Result<usize, Error> {
    index_header(data, off, count_is_32bit).map(|(count, ..)| count)
}

// One object of an INDEX, read through its two offsets without the others.
pub(crate) fn cff_index_entry(data: &[u8], off: usize, i: usize) -> Option<&[u8]> {
    let (count, off_size, offsets_start, data_start) = index_header(data, off, false).ok()?;
    if i >= count { return None; }
    let at = |k: usize| index_offset(data, offsets_start + k * off_size, off_size).checked_sub(1);
    data.get(data_start.checked_add(at(i)?)?..data_start.checked_add(at(i + 1)?)?)
}

pub fn cff_index_spans(data: &[u8], off: usize, count_is_32bit: bool) -> Result<(Vec<(u32, u32)>, usize), Error> {
    let (index, end) = CffIndex::read(data, off, count_is_32bit)?;
    Ok(((0..index.len()).filter_map(|i| index.span(data, i)).collect(), end))
}

// An INDEX read where it lies: `read` checks every object's offsets once, so a span reads two of them.
#[derive(Clone, Copy, Default)]
pub(crate) struct CffIndex {
    count: usize,
    off_size: usize,
    offsets: usize,
    data: usize,
}

impl CffIndex {
    // The INDEX at `off` and where it ends, refused as `cff_index_spans` refuses it.
    pub(crate) fn read(data: &[u8], off: usize, count_is_32bit: bool) -> Result<(CffIndex, usize), Error> {
        let (count, off_size, offsets, data_start) = index_header(data, off, count_is_32bit)?;
        let index = CffIndex { count, off_size, offsets, data: data_start };
        if count == 0 {
            return Ok((index, data_start));
        }
        let data_len = index.offset(data, count);
        if data_start.checked_add(data_len).is_none_or(|end| end > data.len()) {
            return Err(Error::malformed("CFF INDEX: data truncated"));
        }
        for i in 0..count {
            let (start, end) = (index.offset(data, i), index.offset(data, i + 1));
            if start > end || end > data.len() - data_start {
                return Err(Error::owned(
                    crate::daecore::ErrorKind::Malformed,
                    format!("CFF INDEX: object {} out of bounds", i),
                ));
            }
        }
        Ok((index, data_start + data_len))
    }

    pub(crate) fn len(&self) -> usize {
        self.count
    }

    // Object `i`'s start and end in the table `read` checked, its two offsets read from one slice.
    pub(crate) fn span(&self, data: &[u8], i: usize) -> Option<(u32, u32)> {
        if i >= self.count {
            return None;
        }
        let at = self.offsets + i * self.off_size;
        let (start, end) = data.get(at..at + 2 * self.off_size)?.split_at(self.off_size);
        let place = |offset: &[u8]| (self.data + index_offset(offset, 0, self.off_size).saturating_sub(1)) as u32;
        Some((place(start), place(end)))
    }

    fn offset(&self, data: &[u8], i: usize) -> usize {
        index_offset(data, self.offsets + i * self.off_size, self.off_size).saturating_sub(1)
    }
}

pub fn parse_cff_index_refs(
    data: &[u8],
    off: usize,
    count_is_32bit: bool,
) -> Result<(Vec<&[u8]>, usize), Error> {
    let (spans, end) = cff_index_spans(data, off, count_is_32bit)?;
    let mut objects = Vec::with_capacity(spans.len());
    for (s, e) in spans {
        objects.push(data.get(s as usize..e as usize).ok_or(Error::malformed("CFF INDEX: object out of bounds"))?);
    }
    Ok((objects, end))
}

// A CFF table's header and the four INDEXes after it, Name, Top DICT, String and Global Subrs: where
// each starts, where the last ends, and the one Top DICT's fields.
pub(crate) struct Header<'a> {
    pub names: usize,
    pub top_index: usize,
    pub strings: usize,
    pub gsubrs: usize,
    pub end: usize,
    pub top_dict: &'a [u8],
    pub top: Dict,
    pub charstrings: usize,
}

pub(crate) fn read_header(cff: &[u8]) -> Result<Header<'_>, Error> {
    if cff.len() < 4 {
        return Err(Error::malformed("CFF: file too short"));
    }
    let names = usize::from(cff[2]);
    let top_index = cff_index_end(cff, names, false)?;
    let strings = cff_index_end(cff, top_index, false)?;
    let gsubrs = cff_index_end(cff, strings, false)?;
    let end = cff_index_end(cff, gsubrs, false)?;
    let top_dict = cff_index_entry(cff, top_index, 0).ok_or(Error::malformed("CFF: empty Top DICT INDEX"))?;
    let top = read_dict(top_dict, DictKind::Cff1);
    let charstrings = top.charstrings.ok_or(Error::missing("CFF Top DICT: missing CharStrings offset"))?;
    if top.private.is_none() && top.fd_array.is_none() {
        return Err(Error::missing("CFF Top DICT: missing Private DICT and FDArray"));
    }
    Ok(Header { names, top_index, strings, gsubrs, end, top_dict, top, charstrings })
}

// Whether a CFF is read to subset its glyphs: not where its CharStrings count other than maxp's glyphs.
pub(crate) fn cff_readable(cff: &[u8], num_glyphs: u32) -> bool {
    let Ok(header) = read_header(cff) else { return true };
    cff_index_count(cff, header.charstrings, false).is_ok_and(|n| n == num_glyphs as usize)
}

// A Private DICT, read one way for drawing, subsetting and instancing: outside the table it is empty,
// and its local Subrs start at its last nonzero Subrs offset when that lies within the table.
pub(crate) struct Private<'a> {
    pub dict: &'a [u8],
    pub vsindex: u16,
    pub subrs: Option<usize>,
}

pub(crate) fn read_private(table: &[u8], at: Option<Range<usize>>, kind: DictKind) -> Private<'_> {
    let (start, dict) = at.and_then(|at| Some((at.start, table.get(at)?))).unwrap_or((0, &[]));
    let fields = read_dict(dict, kind);
    let subrs = fields.subrs.and_then(|rel| start.checked_add(rel)).filter(|&abs| abs < table.len());
    Private { dict, vsindex: fields.vsindex, subrs }
}

// A local Subrs INDEX, read where it lies. One that doesn't read as an INDEX has none, so only the glyphs that
// call into it fail.
pub(crate) fn read_subrs(table: &[u8], at: usize, kind: DictKind) -> CffIndex {
    CffIndex::read(table, at, kind == DictKind::Cff2).map_or_else(|_| CffIndex::default(), |(index, _)| index)
}

pub fn walk_charset<F>(cff: &[u8], off: usize, n_glyphs: usize, mut on_entry: F) -> Result<usize, Error>
where
    F: FnMut(u16, u16),
{
    if off >= cff.len() { return Err(Error::malformed("CFF charset: offset out of range")); }
    if n_glyphs <= 1 { return Ok(off); }
    let format = cff[off];
    let mut pos = off + 1;

    match format {
        0 => {
            for gid in 1..n_glyphs {
                if pos + 2 > cff.len() { return Err(Error::malformed("CFF charset format 0: truncated")); }
                let sid = read_u16_be(cff, pos).ok_or(Error::malformed("CFF charset format 0: truncated"))?;
                pos += 2;
                on_entry(gid as u16, sid);
            }
        }
        1 | 2 => {
            let mut gid = 1usize;
            while gid < n_glyphs {
                let need = if format == 1 { 3 } else { 4 };
                if pos + need > cff.len() {
                    return Err(Error::owned(
                        crate::daecore::ErrorKind::Malformed,
                        format!("CFF charset format {}: truncated", format),
                    ));
                }
                let first = read_u16_be(cff, pos).ok_or(Error::malformed("CFF charset: truncated"))?;
                let n_left = if format == 1 {
                    let v = *cff.get(pos + 2).ok_or(Error::malformed("CFF charset format 1: truncated"))? as usize;
                    pos += 3;
                    v
                } else {
                    let v =
                        read_u16_be(cff, pos + 2).ok_or(Error::malformed("CFF charset format 2: truncated"))? as usize;
                    pos += 4;
                    v
                };
                let span = (n_left + 1).min(n_glyphs - gid);
                for k in 0..span {
                    on_entry((gid + k) as u16, first.wrapping_add(k as u16));
                }
                gid += span;
            }
        }
        _ => {
            return Err(Error::owned(
                crate::daecore::ErrorKind::Unsupported,
                format!("CFF charset: unknown format {}", format),
            ));
        }
    }
    Ok(pos)
}

// The SIDs of `kept`, ascending glyphs past 0, with `walk_charset`'s errors and wrap: its errors rest on format 0's
// one bound or the range headers, so only those are read, not an entry a glyph.
pub fn kept_sids(cff: &[u8], off: usize, n_glyphs: usize, kept: &[u32]) -> Result<Vec<u16>, Error> {
    if off >= cff.len() { return Err(Error::malformed("CFF charset: offset out of range")); }
    if n_glyphs <= 1 { return Ok(alloc::vec![0; kept.len()]); }
    let mut out = Vec::with_capacity(kept.len());
    match cff[off] {
        0 => {
            if off + 1 + 2 * (n_glyphs - 1) > cff.len() {
                return Err(Error::malformed("CFF charset format 0: truncated"));
            }
            out.extend(kept.iter().map(|&g| read_u16_be(cff, off + 2 * g as usize - 1).unwrap_or(0)));
        }
        format @ (1 | 2) => {
            let need = if format == 1 { 3 } else { 4 };
            let (mut pos, mut gid, mut next) = (off + 1, 1, kept.iter().peekable());
            while gid < n_glyphs {
                if pos + need > cff.len() {
                    return Err(Error::owned(
                        crate::daecore::ErrorKind::Malformed,
                        format!("CFF charset format {format}: truncated"),
                    ));
                }
                let first = read_u16_be(cff, pos).unwrap_or(0);
                let n_left = match format {
                    1 => usize::from(cff[pos + 2]),
                    _ => usize::from(read_u16_be(cff, pos + 2).unwrap_or(0)),
                };
                pos += need;
                let span = n_left + 1;
                while let Some(g) = next.next_if(|&&g| (g as usize) < gid + span) {
                    out.push(first.wrapping_add(((*g) as usize - gid) as u16));
                }
                gid += span;
            }
        }
        format => {
            return Err(Error::owned(
                crate::daecore::ErrorKind::Unsupported,
                format!("CFF charset: unknown format {format}"),
            ));
        }
    }
    Ok(out)
}

pub fn parse_charset_sids(cff: &[u8], off: usize, n_glyphs: usize) -> Result<usize, Error> {
    walk_charset(cff, off, n_glyphs, |_, _| {})
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/cff/parse.rs"]
mod tests;
