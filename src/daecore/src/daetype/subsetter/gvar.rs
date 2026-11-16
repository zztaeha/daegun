// gvar, or GVAR where `wide`, for a subset that isn't instanced: header and shared tuples copied, each kept glyph's
// data in its new place, short offsets where they fit, which store half an offset, so only they pad to even length.
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::{read_u16_be, read_u32_be};
use crate::daecore::Error;

// `kept` pairs each new id with its source glyph, in order, of a subset of `count` glyphs; `maxp` is the font's
// count, past which the source's offsets read 0. None where the header doesn't read. Ranges may overlap, so the
// copy is held to the subset budget, as glyf's is.
pub(crate) fn subset_gvar(
    gvar: &[u8], wide: bool, maxp: usize, kept: &[(usize, u32)], count: usize,
) -> Option<Result<Vec<u8>, Error>> {
    let w = if wide { 3 } else { 2 };
    let (axes, tuples, tuples_at) = (read_u16_be(gvar, 4)?, read_u16_be(gvar, 6)?, read_u32_be(gvar, 8)? as usize);
    let (flags, data_at) = (read_u16_be(gvar, 12 + w)?, read_u32_be(gvar, 14 + w)? as usize);
    let offset = |i: usize| match (i > maxp, flags & 1 != 0) {
        (true, _) => 0,
        (false, true) => read_u32_be(gvar, 18 + w + 4 * i).map_or(0, |v| v as usize),
        (false, false) => read_u16_be(gvar, 18 + w + 2 * i).map_or(0, |v| 2 * usize::from(v)),
    };
    let data = |old: u32| {
        let (start, end) = (offset(old as usize), offset(old as usize + 1));
        let from = data_at.saturating_add(start).min(gvar.len());
        let bytes = gvar.get(from..data_at.saturating_add(end).min(gvar.len())).filter(|_| end >= start);
        bytes.filter(|b| b.len() >= 4).unwrap_or_default()
    };
    let budget = super::subset_budget(gvar.len());
    let within = |n: usize| Some(n).filter(|&n| n <= budget);
    let padded = kept.iter().try_fold(0, |n, &(_, old)| within(n + data(old).len().next_multiple_of(2)));
    let Some(padded) = padded else {
        return Some(Err(Error::limit("subset: gvar ranges past the subset budget")));
    };
    let long = padded > 0x1_FFFE;
    let header = 18 + w;
    let offsets = if long { 4 } else { 2 } * (count + 1);
    let shared = usize::try_from(2 * u64::from(axes) * u64::from(tuples)).ok();
    let shared = if tuples == 0 || tuples_at == 0 { &[][..] } else { gvar.get(tuples_at..)?.get(..shared?)? };

    let mut out = Vec::with_capacity(header + offsets + shared.len() + padded);
    out.extend(0x0001_0000u32.to_be_bytes());
    out.extend(axes.to_be_bytes());
    out.extend(tuples.to_be_bytes());
    out.extend(((header + offsets) as u32).to_be_bytes());
    let glyphs = count.min((1 << (8 * w)) - 1) as u32;
    out.extend_from_slice(&glyphs.to_be_bytes()[4 - w..]);
    out.extend(u16::from(long).to_be_bytes());
    out.extend(((header + offsets + shared.len()) as u32).to_be_bytes());
    let table_at = out.len();
    out.resize(header + offsets, 0);
    out.extend_from_slice(shared);
    let mut at = 0;
    let put = |out: &mut Vec<u8>, i: usize, at: usize| match long {
        true => out[table_at + 4 * i..][..4].copy_from_slice(&(at as u32).to_be_bytes()),
        false => out[table_at + 2 * i..][..2].copy_from_slice(&((at / 2) as u16).to_be_bytes()),
    };
    let mut last = 0;
    for &(new, old) in kept {
        for i in last..new {
            put(&mut out, i + 1, at);
        }
        let bytes = data(old);
        out.extend_from_slice(bytes);
        at += bytes.len();
        if !long && bytes.len() % 2 == 1 {
            out.push(0);
            at += 1;
        }
        put(&mut out, new + 1, at);
        last = new + 1;
    }
    for i in last..count {
        put(&mut out, i + 1, at);
    }
    Some(Ok(out))
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/gvar.rs"]
mod tests;
