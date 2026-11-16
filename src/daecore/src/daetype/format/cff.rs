use alloc::vec::Vec;
use core::ops::Range;
use super::super::decoder::{read_u16_be, read_u32_be};
#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daekit::float::FloatExt;
use crate::daecore::Error;

// A real reads as 0: no DICT value a caller takes as a number is a real, and FontMatrix is copied as bytes.
pub(crate) fn decode_cff_number(data: &[u8], off: usize) -> Result<(i32, usize), Error> {
    if off >= data.len() {
        return Err(Error::malformed("CFF DICT: truncated number"));
    }
    let b0 = data[off] as i32;
    match b0 {
        28 => {
            if off + 3 > data.len() { return Err(Error::malformed("CFF DICT: short 3-byte int")); }
            let v = ((data[off + 1] as i32) << 8) | data[off + 2] as i32;
            Ok((if v & 0x8000 != 0 { v | !0xFFFF } else { v }, 3))
        }
        29 => {
            if off + 5 > data.len() { return Err(Error::malformed("CFF DICT: short 5-byte int")); }
            let v = ((data[off + 1] as i32) << 24)
                  | ((data[off + 2] as i32) << 16)
                  | ((data[off + 3] as i32) << 8)
                  |   data[off + 4] as i32;
            Ok((v, 5))
        }
        30 => {
            let mut p = off + 1;
            loop {
                if p >= data.len() { return Err(Error::malformed("CFF DICT: unterminated real")); }
                if p - off > 64 { return Err(Error::malformed("CFF DICT: real number too long")); }
                let b = data[p]; p += 1;
                if (b & 0x0F) == 0x0F || (b >> 4) == 0x0F { break; }
            }
            Ok((0, p - off))
        }
        32..=246  => Ok((b0 - 139, 1)),
        247..=250 => {
            if off + 2 > data.len() { return Err(Error::malformed("CFF DICT: short 2-byte int")); }
            Ok(((b0 - 247) * 256 + data[off + 1] as i32 + 108, 2))
        }
        251..=254 => {
            if off + 2 > data.len() { return Err(Error::malformed("CFF DICT: short 2-byte int")); }
            Ok((-(b0 - 251) * 256 - data[off + 1] as i32 - 108, 2))
        }
        _ => Err(Error::owned(
            crate::daecore::ErrorKind::Malformed,
            format!("CFF DICT: unexpected byte 0x{:02X} at offset {}", b0, off),
        )),
    }
}

// The shortest of the integer forms a DICT and a charstring share: one byte, two, or 28 and 16 bits.
// False past 16 bits, where a DICT goes on to 29 and a charstring to 255.
#[inline]
pub(crate) fn push_short_int(out: &mut Vec<u8>, n: i32) -> bool {
    match n {
        -107..=107 => out.push((n + 139) as u8),
        108..=1131 => out.extend([((n - 108) / 256 + 247) as u8, ((n - 108) % 256) as u8]),
        -1131..=-108 => out.extend([((-n - 108) / 256 + 251) as u8, ((-n - 108) % 256) as u8]),
        -32768..=32767 => out.extend([28, (n >> 8) as u8, n as u8]),
        _ => return false,
    }
    true
}

pub(crate) fn encode_cff_int(n: i32) -> Vec<u8> {
    alloc::vec![29, (n >> 24) as u8, (n >> 16) as u8, (n >> 8) as u8, n as u8]
}

// The shortest DICT encoding of an integer.
pub(crate) fn encode_cff_int_short(n: i32) -> Vec<u8> {
    let mut out = Vec::with_capacity(3);
    if push_short_int(&mut out, n) { out } else { encode_cff_int(n) }
}

// A charstring number as an f64, which holds every 16.16 value exactly.
#[inline]
pub(crate) fn decode_charstring_number(data: &[u8], off: usize) -> Result<(f64, usize), Error> {
    match decode_charstring_number_fx(data, off) {
        Some((v, width)) => Ok((v as f64 / 65536.0, width)),
        None => Err(charstring_number_error(data, off)),
    }
}

// A charstring number in 16.16: an integer shifted up, or 255's value as stored, which unlike a DICT's
// is 16.16 fixed point.
#[inline]
pub(crate) fn decode_charstring_number_fx(data: &[u8], off: usize) -> Option<(i64, usize)> {
    let b0 = *data.get(off)? as i32;
    match b0 {
        28 => {
            let b1 = *data.get(off + 1)? as i32;
            let b2 = *data.get(off + 2)? as i32;
            let v = ((b1 << 8) | b2) as i16;
            Some(((v as i64) << 16, 3))
        }
        32..=246 => Some((((b0 - 139) as i64) << 16, 1)),
        247..=250 => {
            let b1 = *data.get(off + 1)? as i32;
            Some(((((b0 - 247) * 256 + b1 + 108) as i64) << 16, 2))
        }
        251..=254 => {
            let b1 = *data.get(off + 1)? as i32;
            Some((((-(b0 - 251) * 256 - b1 - 108) as i64) << 16, 2))
        }
        255 => {
            let b = data.get(off + 1..off + 5)?;
            Some((i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as i64, 5))
        }
        _ => None,
    }
}

#[cold]
#[inline(never)]
pub(crate) fn charstring_number_error(data: &[u8], off: usize) -> Error {
    let Some(&b0) = data.get(off) else { return Error::malformed("CFF charstring: truncated number") };
    match b0 {
        28 => Error::malformed("CFF charstring: truncated 3-byte int"),
        247..=254 => Error::malformed("CFF charstring: truncated 2-byte int"),
        255 => Error::malformed("CFF charstring: truncated Fixed16.16"),
        _ => Error::owned(
            crate::daecore::ErrorKind::Malformed,
            format!("CFF charstring: byte 0x{b0:02X} is not a valid number encoding"),
        ),
    }
}

pub(crate) fn subr_bias(count: usize) -> i32 {
    if count < 1240 { 107 } else if count < 33900 { 1131 } else { 32768 }
}

// The subroutine a callsubr or callgsubr operand names among `count`, its bias added; None past them.
pub(crate) fn subr_index(operand: i32, bias: i32, count: usize) -> Option<usize> {
    usize::try_from(i64::from(operand) + i64::from(bias)).ok().filter(|&i| i < count)
}

// A CFF2 blend's value: its default plus each delta times its region's 16.16 weight, each value taken in 16.16, each
// product rounded there and the sum wrapping in 32 bits as charstring arithmetic does.
pub(crate) fn blend_value(default: f64, deltas: &[f64], weights: &[i64]) -> f64 {
    let fixed = |v: f64| (v * 65536.0) as i64 as i32;
    let product = |w: i64, d: f64| super::super::instancer::drawn_mul_fix(w, i64::from(fixed(d))) as i32;
    let sum = weights.iter().zip(deltas).fold(fixed(default), |sum, (&w, &d)| sum.wrapping_add(product(w, d)));
    f64::from(sum) / 65536.0
}

// Type 2's limit on nested subroutine calls (TN 5177, appendix B), for drawing and instancing alike.
pub(crate) const MAX_SUBR_DEPTH: usize = 10;

// An FDSelect read in place: format 0 by where its bytes start, 3 and 4 by the ranges they accept (passing over one
// that starts early or is empty), FD 0 for a glyph in none. Format 3 ranges are u16 and u8, CFF2's 4 u32 and u16.
#[derive(Clone, Default)]
pub(crate) struct FdSelect {
    glyphs: usize,
    kind: FdKind,
}

#[derive(Clone, Default)]
enum FdKind {
    // No FDSelect, so every glyph reads FD 0.
    #[default]
    Zero,
    Bytes(usize),
    // First glyph, end and FD, in order and apart.
    Ranges(Vec<(u32, u32, u16)>),
}

impl FdSelect {
    pub(crate) fn zero(glyphs: usize) -> FdSelect {
        FdSelect { glyphs, kind: FdKind::Zero }
    }

    // The FDSelect at `off` and where it ends.
    pub(crate) fn read(table: &[u8], off: usize, n_glyphs: usize, kind: DictKind) -> Result<(FdSelect, usize), Error> {
        let format = *table.get(off).ok_or(Error::malformed("CFF FDSelect: offset out of bounds"))?;
        let wide = match (format, kind) {
            (0, _) => {
                let end = off + 1 + n_glyphs;
                table.get(off + 1..end).ok_or(Error::malformed("CFF FDSelect format 0: truncated"))?;
                return Ok((FdSelect { glyphs: n_glyphs, kind: FdKind::Bytes(off + 1) }, end));
            }
            (3, _) => false,
            (4, DictKind::Cff2) => true,
            _ => {
                return Err(Error::owned(
                    crate::daecore::ErrorKind::Unsupported,
                    format!("CFF FDSelect: unsupported format {format}"),
                ));
            }
        };
        let truncated =
            || Error::owned(crate::daecore::ErrorKind::Malformed, format!("CFF FDSelect format {format}: truncated"));
        let (glyph, record) = if wide { (4, 6) } else { (2, 3) };
        let read_glyph = |at: usize| match wide {
            true => read_u32_be(table, at).map(|v| v as usize),
            false => read_u16_be(table, at).map(usize::from),
        };
        let read_fd = |at: usize| if wide { read_u16_be(table, at) } else { table.get(at).map(|&fd| u16::from(fd)) };
        let n_ranges = read_glyph(off + 1).ok_or_else(truncated)?;
        let ranges = off + 1 + glyph;
        let mut accepted = Vec::new();
        let mut prev_end = 0usize;
        for i in 0..n_ranges {
            let at = i.checked_mul(record).and_then(|r| r.checked_add(ranges)).ok_or_else(truncated)?;
            let first = read_glyph(at).ok_or_else(truncated)?;
            let fd = at.checked_add(glyph).and_then(read_fd).ok_or_else(truncated)?;
            let next_first = at.checked_add(record).and_then(read_glyph).ok_or_else(truncated)?;
            if first < prev_end || next_first <= first {
                continue;
            }
            let end = next_first.min(n_glyphs);
            prev_end = end;
            if first < end {
                accepted.push((first as u32, end as u32, fd));
            }
        }
        let end = n_ranges.checked_mul(record).and_then(|r| r.checked_add(ranges + glyph));
        let end = end.filter(|&end| end <= table.len()).ok_or_else(truncated)?;
        accepted.shrink_to_fit();
        Ok((FdSelect { glyphs: n_glyphs, kind: FdKind::Ranges(accepted) }, end))
    }

    // Glyph `gid`'s FD in the table `read` read, None past its glyphs.
    pub(crate) fn get(&self, table: &[u8], gid: usize) -> Option<u16> {
        if gid >= self.glyphs {
            return None;
        }
        match &self.kind {
            FdKind::Zero => Some(0),
            FdKind::Bytes(at) => table.get(at + gid).map(|&fd| u16::from(fd)),
            FdKind::Ranges(ranges) => {
                let next = ranges.partition_point(|&(first, ..)| first as usize <= gid);
                let range = next.checked_sub(1).map(|i| ranges[i]).filter(|&(_, end, _)| gid < end as usize);
                Some(range.map_or(0, |(.., fd)| fd))
            }
        }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        match &self.kind {
            FdKind::Ranges(ranges) => crate::daecore::daekit::base::sync::heap_of(ranges),
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DictOp {
    Single(u8),
    Escaped(u8),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DictKind {
    Cff1,
    Cff2,
}

impl DictKind {
    fn is_operator(self, b: u8) -> bool {
        match self {
            DictKind::Cff1 => b <= 21,
            // CFF2 adds vsindex (22), blend (23) and vstore (24). A blend leaves its defaults for the
            // next operator, which no caller reads, so it is reported and its operands dropped.
            DictKind::Cff2 => b <= 24,
        }
    }
}

pub(crate) fn walk_cff_dict<F>(dict: &[u8], kind: DictKind, mut on_op: F)
where
    F: FnMut(DictOp, &[i32], usize, usize),
{
    let mut operands = Vec::<i32>::new();
    let mut off = 0usize;
    let mut operand_start = 0usize;

    while off < dict.len() {
        let b = dict[off];

        if kind.is_operator(b) {
            let (op, width) = if b == 12 {
                match dict.get(off + 1) {
                    Some(&b1) => (DictOp::Escaped(b1), 2),
                    None => break,
                }
            } else {
                (DictOp::Single(b), 1)
            };
            on_op(op, &operands, operand_start, off);
            operands.clear();
            off += width;
            operand_start = off;
        } else {
            match decode_cff_number(dict, off) {
                Ok((v, sz)) => { operands.push(v); off += sz; }
                Err(_) => break,
            }
        }
    }
}

// The DICT fields daetype reads, from a Top, Font or Private DICT of either version. Each is its
// operator's last occurrence, absent when that one's operands give no value; an offset of 0 is none.
#[derive(Clone, Debug, Default)]
pub(crate) struct Dict {
    pub charset: Option<usize>,
    pub charstrings: Option<usize>,
    pub private: Option<Range<usize>>,
    pub subrs: Option<usize>,
    pub vsindex: u16,
    pub vstore: Option<usize>,
    pub ros: bool,
    pub font_matrix: Option<Range<usize>>,
    pub fd_array: Option<usize>,
    pub fd_select: Option<usize>,
}

pub(crate) fn read_dict(dict: &[u8], kind: DictKind) -> Dict {
    let mut d = Dict::default();
    walk_cff_dict(dict, kind, |op, operands, operand_start, op_off| {
        let value = |v: Option<&i32>| v.and_then(|&v| usize::try_from(v).ok());
        let offset = value(operands.last()).filter(|&v| v > 0);
        match op {
            DictOp::Single(15) => d.charset = value(operands.last()),
            DictOp::Single(17) => d.charstrings = offset,
            DictOp::Single(18) => {
                let size = value(operands.len().checked_sub(2).and_then(|i| operands.get(i)));
                d.private = offset.zip(size).and_then(|(at, size)| Some(at..at.checked_add(size)?));
            }
            DictOp::Single(19) => d.subrs = offset,
            DictOp::Single(22) => {
                d.vsindex = value(operands.last()).map_or(0, |v| u16::try_from(v).unwrap_or(u16::MAX))
            }
            DictOp::Single(24) => d.vstore = offset,
            DictOp::Escaped(7) => d.font_matrix = Some(operand_start..op_off),
            DictOp::Escaped(30) => d.ros = operands.len() >= 3,
            DictOp::Escaped(36) => d.fd_array = offset,
            DictOp::Escaped(37) => d.fd_select = offset,
            _ => {}
        }
    });
    d
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Charset {
    Predefined(u8),
    At(usize),
}

impl Dict {
    // A Top DICT's charset: 0 to 2 a predefined one, ISOAdobe when it names none, and past 2 an offset.
    pub(crate) fn charset(&self) -> Charset {
        match self.charset {
            Some(at) if at > 2 => Charset::At(at),
            id => Charset::Predefined(id.map_or(0, |id| id as u8)),
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/cff.rs"]
mod tests;
