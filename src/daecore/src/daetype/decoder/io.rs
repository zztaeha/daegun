pub fn window<const N: usize>(data: &[u8], off: usize) -> Option<&[u8; N]> {
    // A consumer may turn overflow checks on, and on 32-bit a hostile 0xFFFFFFFF offset is usize::MAX,
    // so a plain add would panic or wrap to a wrong in-range read.
    data.get(off..off.checked_add(N)?)?.try_into().ok()
}

pub fn read_u16_be(data: &[u8], off: usize) -> Option<u16> {
    let end = off.checked_add(2)?;
    let b = data.get(off..end)?;
    Some(((b[0] as u16) << 8) | b[1] as u16)
}

pub fn read_u32_be(data: &[u8], off: usize) -> Option<u32> {
    let end = off.checked_add(4)?;
    let b = data.get(off..end)?;
    Some(((b[0] as u32) << 24) | ((b[1] as u32) << 16) | ((b[2] as u32) << 8) | b[3] as u32)
}

pub fn read_i16_be(data: &[u8], off: usize) -> Option<i16> {
    read_u16_be(data, off).map(|v| v as i16)
}

pub fn read_u24_be(data: &[u8], off: usize) -> Option<u32> {
    let b = window::<3>(data, off)?;
    Some(((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32)
}

pub fn read_offset24(data: &[u8], off: usize) -> Option<usize> {
    read_u24_be(data, off).map(|v| v as usize)
}

pub fn write_offset24(data: &mut [u8], off: usize, val: usize) {
    let Some(slot) = off.checked_add(3).and_then(|end| data.get_mut(off..end)) else { return };
    slot[0] = (val >> 16) as u8;
    slot[1] = (val >> 8) as u8;
    slot[2] = val as u8;
}

pub fn write_u16_be(data: &mut [u8], off: usize, val: u16) {
    let Some(slot) = off.checked_add(2).and_then(|end| data.get_mut(off..end)) else { return };
    slot[0] = (val >> 8) as u8;
    slot[1] = val as u8;
}

pub fn write_u32_be(data: &mut [u8], off: usize, val: u32) {
    let Some(slot) = off.checked_add(4).and_then(|end| data.get_mut(off..end)) else { return };
    slot[0] = (val >> 24) as u8;
    slot[1] = (val >> 16) as u8;
    slot[2] = (val >> 8) as u8;
    slot[3] = val as u8;
}

pub fn write_i16_be(data: &mut [u8], off: usize, val: i16) {
    write_u16_be(data, off, val as u16);
}

pub fn search_records<F>(count: usize, target: u32, key_at: F) -> Option<Result<usize, usize>>
where
    F: Fn(usize) -> Option<u32>,
{
    let (mut lo, mut hi) = (0usize, count);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let k = key_at(mid)?;
        if k == target { return Some(Ok(mid)); }
        if k < target { lo = mid + 1; } else { hi = mid; }
    }
    Some(Err(lo))
}

pub fn records_fit(start: usize, count: usize, stride: usize, len: usize) -> bool {
    count
        .checked_mul(stride)
        .and_then(|n| start.checked_add(n))
        .is_some_and(|end| end <= len)
}
