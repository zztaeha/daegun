use alloc::vec::Vec;
use crate::daecore::daetype::decoder::write_u16_be;

pub(crate) fn cff_index_size(data_len: usize) -> usize {
    let max_off  = data_len + 1;
    let off_size = if max_off <= 0xFF { 1 } else if max_off <= 0xFFFF { 2 } else if max_off <= 0xFF_FFFF { 3 } else { 4 };
    2 + 1 + 2 * off_size + data_len
}

// A CFF INDEX counts its objects in 16 bits: past 65,535 there is none to write.
pub fn encode_cff_index(objects: &[Vec<u8>]) -> Option<Vec<u8>> {
    let refs: Vec<&[u8]> = objects.iter().map(|t| t.as_slice()).collect();
    encode_cff_index_refs(&refs)
}

pub(crate) fn encode_cff_index_refs(objects: &[&[u8]]) -> Option<Vec<u8>> {
    if objects.is_empty() {
        return Some(vec![0, 0]);
    }
    let count = u16::try_from(objects.len()).ok()?;
    let data_size: usize = objects.iter().map(|o| o.len()).sum();
    let max_off = data_size + 1;
    let off_size = if max_off <= 0xFF { 1 } else if max_off <= 0xFFFF { 2 } else if max_off <= 0xFF_FFFF { 3 } else { 4 };

    let mut out = Vec::with_capacity(2 + 1 + (objects.len() + 1) * off_size + data_size);
    out.extend_from_slice(&count.to_be_bytes());
    out.push(off_size as u8);
    let mut at = 1usize;
    out.extend_from_slice(&(at as u32).to_be_bytes()[4 - off_size..]);
    for obj in objects {
        at += obj.len();
        out.extend_from_slice(&(at as u32).to_be_bytes()[4 - off_size..]);
    }
    for obj in objects {
        out.extend_from_slice(obj);
    }
    Some(out)
}

pub(crate) fn encode_cff_index_flat(data: &[u8], ends: &[usize]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(cff_index_flat_size(ends));
    append_cff_index_chunks(&mut out, core::slice::from_ref(&data), ends)?;
    Some(out)
}

pub(crate) fn cff_index_flat_size(ends: &[usize]) -> usize {
    if ends.is_empty() {
        return 2;
    }
    let data_len = ends[ends.len() - 1];
    2 + 1 + (ends.len() + 1) * flat_off_size(ends) + data_len
}

fn flat_off_size(ends: &[usize]) -> usize {
    let max_off = ends[ends.len() - 1] + 1;
    if max_off <= 0xFF { 1 } else if max_off <= 0xFFFF { 2 } else if max_off <= 0xFF_FFFF { 3 } else { 4 }
}

pub(crate) fn append_cff_index_chunks(out: &mut Vec<u8>, chunks: &[&[u8]], ends: &[usize]) -> Option<()> {
    if ends.is_empty() {
        out.extend_from_slice(&[0, 0]);
        return Some(());
    }
    let count = u16::try_from(ends.len()).ok()?;
    let off_size = flat_off_size(ends);
    let start = out.len();
    let header_size = 2 + 1 + (ends.len() + 1) * off_size;
    out.resize(start + header_size, 0);

    write_u16_be(out, start, count);
    out[start + 2] = off_size as u8;

    let write_off = |out: &mut Vec<u8>, i: usize, o: usize| {
        let pos = start + 3 + i * off_size;
        for j in 0..off_size {
            out[pos + off_size - 1 - j] = ((o >> (j * 8)) & 0xFF) as u8;
        }
    };
    write_off(out, 0, 1);
    for (i, &end) in ends.iter().enumerate() {
        write_off(out, i + 1, end + 1);
    }

    for chunk in chunks {
        out.extend_from_slice(chunk);
    }
    debug_assert_eq!(out.len() - start, cff_index_flat_size(ends));
    Some(())
}

pub fn encode_cff_int(n: i32) -> Vec<u8> {
    vec![29, (n >> 24) as u8, (n >> 16) as u8, (n >> 8) as u8, n as u8]
}

// The shortest DICT encoding of an integer.
pub(crate) fn encode_cff_int_short(n: i32) -> Vec<u8> {
    match n {
        -107..=107 => vec![(n + 139) as u8],
        108..=1131 => vec![((n - 108) / 256 + 247) as u8, ((n - 108) % 256) as u8],
        -1131..=-108 => vec![((-n - 108) / 256 + 251) as u8, ((-n - 108) % 256) as u8],
        -32768..=32767 => vec![28, (n >> 8) as u8, n as u8],
        _ => encode_cff_int(n),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_past_65535_objects_is_refused() {
        assert!(encode_cff_index(&vec![vec![1]; 70_000]).is_none());
        assert!(encode_cff_index_flat(&[1; 70_000], &(1..=70_000).collect::<Vec<_>>()).is_none());
        assert_eq!(encode_cff_index(&vec![vec![1]; 65_535]).map(|i| (i[0], i[1])), Some((0xFF, 0xFF)));
    }
}
