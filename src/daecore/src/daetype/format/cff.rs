use alloc::string::String;
use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, read_u32_be};
#[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
use crate::daecore::daemachine::float::FloatExt;

// A real reads as 0: no DICT value a caller takes as a number is a real, and FontMatrix is copied as bytes.
pub(crate) fn decode_cff_number(data: &[u8], off: usize) -> Result<(i32, usize), String> {
    if off >= data.len() {
        return Err("CFF DICT: truncated number".into());
    }
    let b0 = data[off] as i32;
    match b0 {
        28 => {
            if off + 3 > data.len() { return Err("CFF DICT: short 3-byte int".into()); }
            let v = ((data[off + 1] as i32) << 8) | data[off + 2] as i32;
            Ok((if v & 0x8000 != 0 { v | !0xFFFF } else { v }, 3))
        }
        29 => {
            if off + 5 > data.len() { return Err("CFF DICT: short 5-byte int".into()); }
            let v = ((data[off + 1] as i32) << 24)
                  | ((data[off + 2] as i32) << 16)
                  | ((data[off + 3] as i32) << 8)
                  |   data[off + 4] as i32;
            Ok((v, 5))
        }
        30 => {
            let mut p = off + 1;
            loop {
                if p >= data.len() { return Err("CFF DICT: unterminated real".into()); }
                if p - off > 64 { return Err("CFF DICT: real number too long".into()); }
                let b = data[p]; p += 1;
                if (b & 0x0F) == 0x0F || (b >> 4) == 0x0F { break; }
            }
            Ok((0, p - off))
        }
        32..=246  => Ok((b0 - 139, 1)),
        247..=250 => {
            if off + 2 > data.len() { return Err("CFF DICT: short 2-byte int".into()); }
            Ok(((b0 - 247) * 256 + data[off + 1] as i32 + 108, 2))
        }
        251..=254 => {
            if off + 2 > data.len() { return Err("CFF DICT: short 2-byte int".into()); }
            Ok((-(b0 - 251) * 256 - data[off + 1] as i32 - 108, 2))
        }
        _ => Err(format!("CFF DICT: unexpected byte 0x{:02X} at offset {}", b0, off)),
    }
}

// A DICT real: binary-coded decimal, two nibbles a byte, ending at nibble 0xf.
pub(crate) fn decode_cff_real(data: &[u8], off: usize) -> Option<(f64, usize)> {
    if *data.get(off)? != 30 {
        return None;
    }
    let mut text = String::new();
    let mut p = off + 1;
    'bytes: loop {
        let b = *data.get(p)?;
        p += 1;
        for nibble in [b >> 4, b & 0x0F] {
            match nibble {
                0..=9 => text.push(char::from(b'0' + nibble)),
                0x0A => text.push('.'),
                0x0B => text.push('E'),
                0x0C => text.push_str("E-"),
                0x0E => text.push('-'),
                0x0F => break 'bytes,
                _ => return None,
            }
        }
        if p - off > 64 {
            return None;
        }
    }
    Some((text.parse().ok()?, p - off))
}

// A DICT number: a whole one as a five-byte integer, any other as a real.
pub(crate) fn encode_dict_number(v: f64) -> Vec<u8> {
    if v == v.trunc() && v >= f64::from(i32::MIN) && v <= f64::from(i32::MAX) {
        let n = v as i32;
        return alloc::vec![29, (n >> 24) as u8, (n >> 16) as u8, (n >> 8) as u8, n as u8];
    }
    let text = format!("{v}");
    let mut nibbles: Vec<u8> = text.bytes().map(|c| match c {
        b'.' => 0x0A,
        b'-' => 0x0E,
        d => d - b'0',
    }).collect();
    nibbles.push(0x0F);
    if nibbles.len() % 2 == 1 {
        nibbles.push(0x0F);
    }
    let mut out = alloc::vec![30];
    out.extend(nibbles.chunks(2).map(|n| (n[0] << 4) | n[1]));
    out
}

pub(crate) fn decode_charstring_number(data: &[u8], off: usize) -> Result<(f64, usize), String> {
    match decode_charstring_number_opt(data, off) {
        Some(v) => Ok(v),
        None => Err(charstring_number_error(data, off)),
    }
}

#[inline]
pub(crate) fn decode_charstring_number_opt(data: &[u8], off: usize) -> Option<(f64, usize)> {
    let b0 = *data.get(off)? as i32;
    match b0 {
        28 => {
            let b1 = *data.get(off + 1)? as i32;
            let b2 = *data.get(off + 2)? as i32;
            let v = ((b1 << 8) | b2) as i16;
            Some((v as f64, 3))
        }
        32..=246 => Some(((b0 - 139) as f64, 1)),
        247..=250 => {
            let b1 = *data.get(off + 1)? as i32;
            Some((((b0 - 247) * 256 + b1 + 108) as f64, 2))
        }
        251..=254 => {
            let b1 = *data.get(off + 1)? as i32;
            Some(((-(b0 - 251) * 256 - b1 - 108) as f64, 2))
        }
        255 => {
            let b = data.get(off + 1..off + 5)?;
            let raw = i32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            // Unlike a DICT, a charstring reads 255 as 16.16 fixed point.
            Some((raw as f64 / 65536.0, 5))
        }
        _ => None,
    }
}

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
pub(crate) fn charstring_number_error(data: &[u8], off: usize) -> String {
    let Some(&b0) = data.get(off) else { return "CFF charstring: truncated number".into() };
    match b0 {
        28 => "CFF charstring: truncated 3-byte int".into(),
        247..=254 => "CFF charstring: truncated 2-byte int".into(),
        255 => "CFF charstring: truncated Fixed16.16".into(),
        _ => format!("CFF charstring: byte 0x{b0:02X} is not a valid number encoding"),
    }
}

pub(crate) fn subr_bias(count: usize) -> i32 {
    if count < 1240 { 107 } else if count < 33900 { 1131 } else { 32768 }
}

pub(crate) fn resolve_fd_select(cff: &[u8], off: usize, n_glyphs: usize) -> Result<Vec<u16>, String> {
    let format = *cff.get(off).ok_or("CFF FDSelect: offset out of bounds")?;
    match format {
        0 => {
            let mut out = Vec::with_capacity(n_glyphs);
            for gid in 0..n_glyphs {
                let fd = *cff.get(off + 1 + gid).ok_or("CFF FDSelect format 0: truncated")?;
                out.push(fd as u16);
            }
            Ok(out)
        }
        3 => {
            let n_ranges = read_u16_be(cff, off + 1).ok_or("CFF FDSelect format 3: truncated")? as usize;
            let mut out = vec![0u16; n_glyphs];
            let ranges_off = off + 3;
            let mut prev_end = 0usize;
            for i in 0..n_ranges {
                let rec   = ranges_off + i * 3;
                let first = read_u16_be(cff, rec).ok_or("CFF FDSelect format 3: truncated")? as usize;
                let fd    = *cff.get(rec + 2).ok_or("CFF FDSelect format 3: truncated")?;
                let next_first = read_u16_be(cff, rec + 3).ok_or("CFF FDSelect format 3: truncated")? as usize;
                if first < prev_end || next_first <= first {
                    continue;
                }
                let end = next_first.min(n_glyphs);
                prev_end = end;
                for slot in out.iter_mut().take(end).skip(first) {
                    *slot = fd as u16;
                }
            }
            Ok(out)
        }
        4 => {
            let n_ranges = read_u32_be(cff, off + 1).ok_or("CFF FDSelect format 4: truncated")? as usize;
            let mut out = vec![0u16; n_glyphs];
            let mut prev_end = 0usize;
            for i in 0..n_ranges {
                let rec = i.checked_mul(6).and_then(|r| r.checked_add(off + 5)).ok_or("CFF FDSelect format 4: truncated")?;
                let first = read_u32_be(cff, rec).ok_or("CFF FDSelect format 4: truncated")? as usize;
                let fd = read_u16_be(cff, rec + 4).ok_or("CFF FDSelect format 4: truncated")?;
                let next_first = read_u32_be(cff, rec + 6).ok_or("CFF FDSelect format 4: truncated")? as usize;
                if first < prev_end || next_first <= first {
                    continue;
                }
                let end = next_first.min(n_glyphs);
                prev_end = end;
                for slot in out.iter_mut().take(end).skip(first) {
                    *slot = fd;
                }
            }
            Ok(out)
        }
        _ => Err(format!("CFF FDSelect: unsupported format {}", format)),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DictOp {
    Single(u8),
    Escaped(u8),
}

pub(crate) enum DictFlow {
    Continue,
    Stop,
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
    F: FnMut(DictOp, &[i32], usize, usize) -> DictFlow,
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
            let flow = on_op(op, &operands, operand_start, off);
            operands.clear();
            off += width;
            operand_start = off;
            if matches!(flow, DictFlow::Stop) { return; }
        } else {
            match decode_cff_number(dict, off) {
                Ok((v, sz)) => { operands.push(v); off += sz; }
                Err(_) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn ops(dict: &[u8], kind: DictKind) -> Vec<(DictOp, Vec<i32>)> {
        let mut out = Vec::new();
        walk_cff_dict(dict, kind, |op, operands, _, _| {
            out.push((op, operands.to_vec()));
            DictFlow::Continue
        });
        out
    }

    #[test]
    fn a_cff2_dict_walks_past_a_blend() {
        let private = [129, 149, 140, 141, 141, 23, 6, 247, 0, 19];
        assert_eq!(
            ops(&private, DictKind::Cff2),
            [(DictOp::Single(23), vec![-10, 10, 1, 2, 2]), (DictOp::Single(6), vec![]), (DictOp::Single(19), vec![108])],
        );
    }

    #[test]
    fn fdselect_maps_every_glyph_in_each_format() {
        let mut f0 = vec![0u8];
        f0.extend([2, 0, 1, 1]);
        assert_eq!(resolve_fd_select(&f0, 0, 4), Ok(vec![2, 0, 1, 1]));

        let f3 = [3, 0, 2, 0, 0, 1, 0, 3, 4, 0, 5];
        assert_eq!(resolve_fd_select(&f3, 0, 5), Ok(vec![1, 1, 1, 4, 4]));

        let mut f4 = vec![4, 0, 0, 0, 2];
        f4.extend([0, 0, 0, 0, 0, 7, 0, 0, 0, 2, 0x01, 0x02]);
        f4.extend([0, 0, 0, 5]);
        assert_eq!(resolve_fd_select(&f4, 0, 5), Ok(vec![7, 7, 0x0102, 0x0102, 0x0102]));
    }

    #[test]
    fn dict_and_charstring_numbers_read_their_values() {
        assert_eq!(decode_cff_number(&[28, 0xFF, 0x38], 0), Ok((-200, 3)));
        assert_eq!(decode_cff_number(&[29, 0xFF, 0xFF, 0xFF, 0x38], 0), Ok((-200, 5)));
        assert_eq!(decode_cff_number(&[30, 0x1A, 0x5F], 0), Ok((0, 3)), "a real reads as 0");
        assert_eq!(decode_charstring_number(&[255, 0, 1, 0x80, 1], 0), Ok((1.5 + 1.0 / 65536.0, 5)));
        assert_eq!(decode_charstring_number_fx(&[255, 0xFF, 0xFE, 0x40, 0], 0), Some((-0x1_C000, 5)));
        assert_eq!(decode_charstring_number(&[28, 0x80, 0], 0), Ok((-32768.0, 3)));
    }

    #[test]
    fn a_subroutine_bias_changes_at_its_thresholds() {
        assert_eq!([1239, 1240, 33899, 33900].map(subr_bias), [107, 1131, 1131, 32768]);
    }
}
