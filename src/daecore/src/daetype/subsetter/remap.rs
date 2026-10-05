use crate::daecore::daetype::subsetter::GlyphSet;
use super::*;
use super::super::decoder::records_fit;

pub fn fix_post_table(mut post: Vec<u8>) -> Vec<u8> {
    if post.len() < 32 { return post; }
    post.truncate(32);
    write_u32_be(&mut post, 0, 0x0003_0000);
    post
}

// A record whose glyph has no place in the subset is left out, the rest kept.
pub fn remap_vorg(vorg: &[u8], gid_map: &[u16], active: &GlyphSet) -> Option<Vec<u8>> {
    if vorg.len() < 8 { return None; }
    let major = read_u16_be(vorg, 0)?;
    let minor = read_u16_be(vorg, 2)?;
    let default = read_i16_be(vorg, 4)?;
    let count = read_u16_be(vorg, 6)? as usize;

    let mut kept: Vec<(u16, i16)> = Vec::with_capacity(count.min(active.len()));
    for i in 0..count {
        let rec = 8 + i * 4;
        let orig_gid = read_u16_be(vorg, rec)?;
        let y = read_i16_be(vorg, rec + 2)?;
        if let Some(gid) = kept_gid(orig_gid, gid_map, active) { kept.push((gid, y)); }
    }

    let mut out = vec![0u8; 8 + kept.len() * 4];
    write_u16_be(&mut out, 0, major);
    write_u16_be(&mut out, 2, minor);
    write_i16_be(&mut out, 4, default);
    write_u16_be(&mut out, 6, kept.len() as u16);
    for (i, &(gid, y)) in kept.iter().enumerate() {
        write_u16_be(&mut out, 8 + i * 4, gid);
        write_i16_be(&mut out, 8 + i * 4 + 2, y);
    }
    Some(out)
}

fn kept_gid(gid: u16, gid_map: &[u16], active: &GlyphSet) -> Option<u16> {
    if !active.contains(&gid) { return None; }
    gid_map.get(usize::from(gid)).copied()
}

// The format 0 and 2 subtables the shaper applies, left with the kept glyphs' pairs or classes. Only the
// last can pass 64 KB: read to the table's end, as HarfBuzz reads it, and written wrapped, as fontTools does.
pub fn remap_kern(kern: &[u8], gid_map: &[u16], active: &GlyphSet) -> Option<Vec<u8>> {
    let apple = read_u16_be(kern, 0)? == 1 && read_u16_be(kern, 2)? == 0;
    let (n_tables, mut at) = if apple {
        (read_u32_be(kern, 4)? as usize, 8)
    } else {
        if read_u16_be(kern, 0)? != 0 { return None; }
        (read_u16_be(kern, 2)? as usize, 4)
    };
    let header_len = if apple { 8 } else { 6 };

    let mut subtables: Vec<Vec<u8>> = Vec::new();
    for i in 0..n_tables {
        let (length, coverage) = if apple {
            (read_u32_be(kern, at)? as usize, read_u16_be(kern, at + 4)?)
        } else {
            (usize::from(read_u16_be(kern, at + 2)?), read_u16_be(kern, at + 4)?)
        };
        let end = if !apple && i + 1 == n_tables { kern.len() } else { at.checked_add(length)? };
        if end < at + header_len || end > kern.len() {
            return None;
        }
        let sub = &kern[at..end];
        let format = if apple { coverage & 0x00FF } else { coverage >> 8 };
        let variation = apple && coverage & 0x2000 != 0;
        let body = match format {
            _ if variation => None,
            0 => remap_kern_format0(sub, header_len, gid_map, active),
            2 => remap_kern_format2(sub, header_len, gid_map, active),
            _ => None,
        };
        if let Some(body) = body {
            let mut rebuilt = sub[..header_len].to_vec();
            rebuilt.extend(body);
            subtables.push(rebuilt);
        }
        at = end;
    }

    if subtables.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    if apple {
        out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        out.extend_from_slice(&u32::try_from(subtables.len()).ok()?.to_be_bytes());
    } else {
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&u16::try_from(subtables.len()).ok()?.to_be_bytes());
    }
    for mut sub in subtables {
        if apple {
            let length = u32::try_from(sub.len()).ok()?;
            write_u32_be(&mut sub, 0, length);
        } else {
            let length = sub.len() as u16;
            write_u16_be(&mut sub, 2, length);
        }
        out.extend_from_slice(&sub);
    }
    Some(out)
}

fn remap_kern_format0(sub: &[u8], body: usize, gid_map: &[u16], active: &GlyphSet) -> Option<Vec<u8>> {
    let n_pairs = read_u16_be(sub, body)? as usize;
    if !records_fit(body + 8, n_pairs, 6, sub.len()) { return None; }

    let mut kept: Vec<(u16, u16, i16)> = Vec::with_capacity(n_pairs.min(active.len()));
    for i in 0..n_pairs {
        let rec = body + 8 + i * 6;
        let (Some(left), Some(right)) = (read_u16_be(sub, rec), read_u16_be(sub, rec + 2)) else { continue };
        let (Some(left), Some(right)) = (kept_gid(left, gid_map, active), kept_gid(right, gid_map, active)) else { continue };
        kept.push((left, right, read_i16_be(sub, rec + 4)?));
    }
    if kept.is_empty() {
        return None;
    }
    // Readers binary-search the pairs, which the font may list out of order or twice: sorted here, each
    // pair kept once with its first value.
    kept.sort_by_key(|&(left, right, _)| (left, right));
    kept.dedup_by_key(|&mut (left, right, _)| (left, right));

    let pairs = u16::try_from(kept.len()).ok()?;
    let entry_selector = u32::from(pairs).ilog2();
    let search_range = ((1u32 << entry_selector) * 6) & 0xFFFF;
    let range_shift = (u32::from(pairs) * 6).wrapping_sub(search_range) & 0xFFFF;
    let mut out = Vec::with_capacity(8 + kept.len() * 6);
    for v in [u32::from(pairs), search_range, entry_selector, range_shift] {
        out.extend_from_slice(&(v as u16).to_be_bytes());
    }
    for &(left, right, value) in &kept {
        out.extend_from_slice(&left.to_be_bytes());
        out.extend_from_slice(&right.to_be_bytes());
        out.extend_from_slice(&value.to_be_bytes());
    }
    Some(out)
}

// A class table's first glyph and its values, read whole.
fn kern_classes(sub: &[u8], off: usize) -> Option<(u16, Vec<u16>)> {
    let first = read_u16_be(sub, off)?;
    let count = usize::from(read_u16_be(sub, off + 2)?);
    if !records_fit(off + 4, count, 2, sub.len()) { return None; }
    Some((first, (0..count).filter_map(|i| read_u16_be(sub, off + 4 + i * 2)).collect()))
}

// Class tables keyed by the kept glyphs, new ids in place of old, and the kerning array copied whole.
// A left class names its row by offset from the subtable's start, so each moves with the array.
fn remap_kern_format2(sub: &[u8], body: usize, gid_map: &[u16], active: &GlyphSet) -> Option<Vec<u8>> {
    let row_width = read_u16_be(sub, body)?;
    let array = usize::from(read_u16_be(sub, body + 6)?);
    if array > sub.len() { return None; }
    let classes = |at: usize, row: bool| -> Option<Vec<(u16, u16)>> {
        let (first, values) = kern_classes(sub, usize::from(read_u16_be(sub, body + at)?))?;
        let mut kept: Vec<(u16, u16)> = values.iter().enumerate()
            .filter_map(|(i, &v)| {
                let gid = kept_gid(first.checked_add(u16::try_from(i).ok()?)?, gid_map, active)?;
                // A row before the array kerns nothing, as the shaper reads it.
                (!row || usize::from(v) >= array).then_some((gid, v))
            })
            .filter(|&(_, v)| v != 0)
            .collect();
        kept.sort_unstable();
        Some(kept)
    };
    let (left, right) = (classes(2, true)?, classes(4, false)?);
    let (Some(&(left_first, _)), Some(&(left_last, _))) = (left.first(), left.last()) else { return None };
    let (Some(&(right_first, _)), Some(&(right_last, _))) = (right.first(), right.last()) else { return None };

    let table = |first: u16, last: u16, entries: &[(u16, u16)], shift: isize| -> Option<Vec<u8>> {
        let mut values = vec![0u16; usize::from(last - first) + 1];
        for &(gid, v) in entries {
            values[usize::from(gid - first)] = u16::try_from(v as isize + shift).ok()?;
        }
        Some([first.to_be_bytes(), u16::try_from(values.len()).ok()?.to_be_bytes()].concat().into_iter()
            .chain(values.iter().flat_map(|v| v.to_be_bytes())).collect())
    };
    let left_at = body + 8;
    let right_len = 4 + 2 * (usize::from(right_last - right_first) + 1);
    let left_len = 4 + 2 * (usize::from(left_last - left_first) + 1);
    let new_array = left_at + left_len + right_len;
    let left_table = table(left_first, left_last, &left, new_array as isize - array as isize)?;
    let right_table = table(right_first, right_last, &right, 0)?;

    let mut out = Vec::with_capacity(new_array - body + sub.len() - array);
    for v in [row_width, u16::try_from(left_at).ok()?, u16::try_from(left_at + left_len).ok()?, u16::try_from(new_array).ok()?] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend(left_table);
    out.extend(right_table);
    out.extend_from_slice(&sub[array..]);
    Some(out)
}

pub fn metric_pair(mtx: &[u8], num_long: usize, last_advance: u16, gid: usize) -> (u16, i16) {
    if gid < num_long {
        let off = gid * 4;
        let adv = if off + 2 <= mtx.len() { read_u16_be(mtx, off).unwrap_or(0) } else { 0 };
        let bearing = if off + 4 <= mtx.len() { read_i16_be(mtx, off + 2).unwrap_or(0) } else { 0 };
        (adv, bearing)
    } else {
        let off = num_long * 4 + (gid - num_long) * 2;
        let bearing = if off + 2 <= mtx.len() { read_i16_be(mtx, off).unwrap_or(0) } else { 0 };
        (last_advance, bearing)
    }
}

pub fn rebuild_metrics(mtx: &[u8], num_long: usize, active_sorted: &[u16]) -> Vec<u8> {
    let last_advance = if num_long > 0 && num_long * 4 <= mtx.len() {
        read_u16_be(mtx, (num_long - 1) * 4).unwrap_or(0)
    } else { 0 };

    let mut out = vec![0u8; active_sorted.len() * 4];
    for (compact, &orig_gid) in active_sorted.iter().enumerate() {
        let (adv, bearing) = metric_pair(mtx, num_long, last_advance, orig_gid as usize);
        write_u16_be(&mut out, compact * 4, adv);
        write_i16_be(&mut out, compact * 4 + 2, bearing);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Format 0 pairs out of order, and one pair twice: the subset is sorted, each pair kept once, first
    // value first, as a reader's binary search needs.
    #[test]
    fn kern_pairs_out_of_order_come_back_sorted() {
        let kern: Vec<u8> = [0u16, 1, 0, 32, 1, 3, 12, 1, 6, 2, 1, 0xFFF6, 1, 2, 0xFFEC, 1, 2, 0xFFE2]
            .iter().flat_map(|w| w.to_be_bytes()).collect();
        let mut active = GlyphSet::new();
        (0..3).for_each(|g| { active.insert(g); });
        let out = remap_kern(&kern, &[0, 1, 2], &active).expect("a subset kern");
        let pairs: Vec<(u16, u16, i16)> = (0..usize::from(read_u16_be(&out, 10).expect("a pair count")))
            .map(|i| (read_u16_be(&out, 18 + 6 * i).unwrap(), read_u16_be(&out, 20 + 6 * i).unwrap(), read_i16_be(&out, 22 + 6 * i).unwrap()))
            .collect();
        assert_eq!(pairs, [(1, 2, -20), (2, 1, -10)], "the pairs did not come back sorted, once each");
    }

    fn u16s(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    // Glyphs 2 and 3 in rows 0 and 1, 5 and 6 in columns 0 and 1, of a 2 by 2 array: a kern table of
    // one format 2 subtable, OpenType or Apple. Offsets count from the subtable's start.
    fn format2(apple: bool) -> Vec<u8> {
        let h: u16 = if apple { 8 } else { 6 };
        let array = h + 24;
        let body = [u16s(&[4, h + 8, h + 16, array, 2, 2, array, array + 4, 5, 2, 0, 2]), u16s(&[0xFFF6, 0xFFEC, 0xFFE2, 0xFFD8])].concat();
        let length = usize::from(h) + body.len();
        let header = if apple { [u16s(&[1, 0, 0, 1]), (length as u32).to_be_bytes().to_vec(), u16s(&[2, 0])].concat() }
            else { u16s(&[0, 1, 0, length as u16, 0x0201]) };
        [header, body].concat()
    }

    // The shaper's reading: the left class names a row by offset, the right a column; a row before
    // the array kerns nothing.
    fn kerning(kern: &[u8], apple: bool, left: u16, right: u16) -> i16 {
        let sub = if apple { 8 } else { 4 };
        let body = sub + if apple { 8 } else { 6 };
        let at = |k: usize| usize::from(read_u16_be(kern, k).unwrap_or(0));
        let class = |table: usize, g: u16| {
            let (first, n) = (at(sub + table) as u16, at(sub + table + 2) as u16);
            if g < first || g >= first + n { 0 } else { at(sub + table + 4 + 2 * usize::from(g - first)) }
        };
        let row = class(at(body + 2), left);
        if row < at(body + 6) { return 0; }
        read_i16_be(kern, sub + row + class(at(body + 4), right)).unwrap_or(0)
    }

    #[test]
    fn class_kerning_keeps_its_values_on_the_kept_glyphs() {
        let mut active = GlyphSet::new();
        [0, 3, 5, 6].into_iter().for_each(|g| { active.insert(g); });
        let gid_map = [0, 0, 0, 1, 0, 2, 3];
        for apple in [false, true] {
            let kern = format2(apple);
            let out = remap_kern(&kern, &gid_map, &active).expect("the class subtable is kept");
            for (left, right) in [(3, 5), (3, 6), (5, 3), (3, 3)] {
                assert_eq!(kerning(&out, apple, gid_map[left], gid_map[right]), kerning(&kern, apple, left as u16, right as u16), "apple {apple}: {left} {right}");
            }
            assert_eq!(kerning(&out, apple, 1, 2), -30);
        }
    }

    // The last subtable, 11,000 pairs, wraps its 16-bit length and is read to the table's end.
    #[test]
    fn the_last_kern_subtable_may_pass_64_kb() {
        let pairs = |n: u16| [u16s(&[0, (14 + 6 * u32::from(n)) as u16, 0x0001, n, 0, 0, 0]), (0..n).flat_map(|i| u16s(&[1, i + 1, 5])).collect()].concat();
        let mut active = GlyphSet::new();
        (0..=11_000).for_each(|g| { active.insert(g); });
        let gid_map: Vec<u16> = (0..=11_000).collect();
        let kern = [u16s(&[0, 2]), pairs(2), pairs(11_000)].concat();
        let out = remap_kern(&kern, &gid_map, &active).expect("a kern");
        let second = 4 + usize::from(read_u16_be(&out, 6).expect("length"));
        assert_eq!([read_u16_be(&out, 10), read_u16_be(&out, second + 6)], [Some(2), Some(11_000)]);
        assert_eq!(out.len(), kern.len());
    }

    // A record whose glyph the subset holds no place for is left out; the rest stay.
    #[test]
    fn a_vorg_record_without_a_place_is_left_out() {
        let vorg = u16s(&[1, 0, 880, 2, 36, 900, 6_000, 910]);
        let mut active = GlyphSet::new();
        [0, 36, 6_000].into_iter().for_each(|g| { active.insert(g); });
        let mut gid_map = vec![0; 37];
        gid_map[36] = 1;
        assert_eq!(remap_vorg(&vorg, &gid_map, &active), Some(u16s(&[1, 0, 880, 1, 1, 900])));
    }
}
