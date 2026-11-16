use crate::daecore::daetype::subsetter::GlyphSet;
use super::*;
use super::super::decoder::{bin_search_header, midpoint_search, records_fit};
use super::super::format::metrics::{compacted, narrowed, MetricsTable};
use super::super::vorg::Vorg;

pub fn fix_post_table(mut post: Vec<u8>) -> Vec<u8> {
    if post.len() < 32 { return post; }
    post.truncate(32);
    write_u32_be(&mut post, 0, 0x0003_0000);
    post
}

// A record whose glyph has no place in the subset is left out, the rest kept, in the source's version at minor 0:
// version 2's glyphs and count in 24 bits.
pub fn remap_vorg(vorg: &[u8], gid_map: &[u32], active: &GlyphSet) -> Option<Vec<u8>> {
    let table = Vorg::read(vorg)?;
    let w = table.width;
    let kept: Vec<(u32, i16)> =
        table.records().filter_map(|(gid, y)| Some((remap_gid(active, gid_map, gid)?, y))).collect();

    let fits = |v: u32| v >> (8 * w) == 0;
    let count = u32::try_from(kept.len()).ok().filter(|&n| fits(n))?;
    let mut out = vec![0, w as u8 - 1, 0, 0];
    out.extend_from_slice(&table.default.to_be_bytes());
    out.extend_from_slice(&count.to_be_bytes()[4 - w..]);
    for (gid, y) in kept {
        if !fits(gid) {
            return None;
        }
        out.extend_from_slice(&gid.to_be_bytes()[4 - w..]);
        out.extend_from_slice(&y.to_be_bytes());
    }
    Some(out)
}

// Every subtable stays in order with its coverage, as shaping's plan counts them all: formats 0 to 3 cut to the kept
// glyphs, any other its header alone. An OpenType kern's last reads to the table's end and can pass 64 KB, wrapped.
// None once the subtables pass the subset budget.
pub fn remap_kern(kern: &[u8], gid_map: &[u32], active: &GlyphSet) -> Option<Vec<u8>> {
    let (apple, n_tables, header_len, mut at) = match read_u16_be(kern, 0)? {
        0 => (false, usize::from(read_u16_be(kern, 2)?), 6, 4),
        1 => (true, read_u32_be(kern, 4)? as usize, 8, 8),
        // Another version applies nothing, but counts as data.
        _ => return kern.get(..4).map(<[u8]>::to_vec),
    };

    let mut out = kern.get(..at)?.to_vec();
    let budget = subset_budget(kern.len());
    for i in 0..n_tables {
        let last = i + 1 == n_tables;
        let (length, coverage) = if apple {
            (read_u32_be(kern, at)? as usize, read_u16_be(kern, at + 4)?)
        } else {
            (usize::from(read_u16_be(kern, at + 2)?), read_u16_be(kern, at + 4)?)
        };
        let end = if last && !apple { kern.len() } else { at.checked_add(length)? };
        if end < at + header_len || end > kern.len() {
            return None;
        }
        let sub = &kern[at..end];
        let format = if apple { coverage & 0x00FF } else { coverage >> 8 };
        let body = match format {
            0 => remap_kern_format0(sub, header_len, gid_map, active),
            1 => remap_kern_format1(sub, header_len, gid_map, active),
            2 => remap_kern_format2(sub, header_len, gid_map, active),
            3 => remap_kern_format3(sub, header_len, gid_map, active),
            _ => Some(Vec::new()),
        }?;
        let start = out.len();
        out.extend_from_slice(&sub[..header_len]);
        out.extend(body);
        if out.len() > budget {
            return None;
        }
        let length = out.len() - start;
        if apple {
            write_u32_be(&mut out, start, u32::try_from(length).ok()?);
        } else {
            write_u16_be(&mut out, start + 2, if last { length as u16 } else { u16::try_from(length).ok()? });
        }
        at = end;
    }
    Some(out)
}

fn remap_kern_format0(sub: &[u8], body: usize, gid_map: &[u32], active: &GlyphSet) -> Option<Vec<u8>> {
    let n_pairs = read_u16_be(sub, body)? as usize;
    let pairs_at = body + 8;
    if !records_fit(pairs_at, n_pairs, 6, sub.len()) { return None; }
    let key_at = |i: usize| read_u32_be(sub, pairs_at + i * 6);

    // Each pair with the value the shaper's search finds in the font: a pair the font repeats or misorders may read
    // another copy's value there, or none.
    let (mut kept, mut lefts) = (Vec::with_capacity(n_pairs.min(active.len())), BTreeSet::new());
    for i in 0..n_pairs {
        let Some(key) = key_at(i) else { continue };
        let Some(left) = remap_gid16(active, gid_map, (key >> 16) as u16) else { continue };
        lefts.insert(left);
        let Some(right) = remap_gid16(active, gid_map, key as u16) else { continue };
        let Some(found) = midpoint_search(n_pairs, |j| key_at(j).map(|k| key.cmp(&k))) else { continue };
        kept.push((left, right, read_i16_be(sub, pairs_at + found * 6 + 4)?));
    }
    let kept = format0_pairs(kept, lefts);

    let pairs = u16::try_from(kept.len()).ok()?;
    let mut out = Vec::with_capacity(8 + kept.len() * 6);
    for v in [usize::from(pairs)].into_iter().chain(bin_search_header(kept.len(), 6)) {
        out.extend_from_slice(&(v as u16).to_be_bytes());
    }
    for &(left, right, value) in &kept {
        out.extend_from_slice(&left.to_be_bytes());
        out.extend_from_slice(&right.to_be_bytes());
        out.extend_from_slice(&value.to_be_bytes());
    }
    Some(out)
}

// Pairs sorted and once each. A subtable starts on every pair's left glyph, so a kept left glyph whose pairs all went
// keeps one to 0xFFFF, which no subset glyph meets: a subset has fewer glyphs, and deletions leave before kerning.
pub(crate) fn format0_pairs(mut pairs: Vec<(u16, u16, i16)>, mut lefts: BTreeSet<u16>) -> Vec<(u16, u16, i16)> {
    for &(left, ..) in &pairs { lefts.remove(&left); }
    pairs.extend(lefts.into_iter().map(|left| (left, 0xFFFF, 0)));
    pairs.sort_unstable_by_key(|&(left, right, _)| (left, right));
    pairs.dedup_by_key(|&mut (left, right, _)| (left, right));
    pairs
}

// A class table's first glyph and its values, read whole.
fn kern_classes(sub: &[u8], off: usize) -> Option<(u16, Vec<u16>)> {
    let first = read_u16_be(sub, off)?;
    let count = usize::from(read_u16_be(sub, off + 2)?);
    if !records_fit(off + 4, count, 2, sub.len()) { return None; }
    Some((first, (0..count).filter_map(|i| read_u16_be(sub, off + 4 + i * 2)).collect()))
}

// The machine as it stands, its class table rebuilt for the kept glyphs after the rest, so no offset, state or value
// moves. A subset keeps glyph order, so every glyph between kept ones comes from the old table with its class.
fn remap_kern_format1(sub: &[u8], body: usize, gid_map: &[u32], active: &GlyphSet) -> Option<Vec<u8>> {
    let class_at = body + usize::from(read_u16_be(sub, body + 2)?);
    let first = read_u16_be(sub, class_at)?;
    let count = usize::from(read_u16_be(sub, class_at + 2)?);
    let classes = sub.get(class_at + 4..class_at + 4 + count)?;
    let mut kept: Vec<(u16, u8)> = classes.iter().enumerate()
        .filter_map(|(i, &k)| Some((remap_gid16(active, gid_map, first.checked_add(u16::try_from(i).ok()?)?)?, k)))
        .collect();
    kept.sort_unstable();
    let new_first = kept.first().map_or(0, |k| k.0);
    let mut row = vec![1u8; kept.last().map_or(0, |k| usize::from(k.0 - new_first) + 1)];
    kept.iter().for_each(|&(gid, k)| row[usize::from(gid - new_first)] = k);

    let mut out = sub.get(body..)?.to_vec();
    let at = u16::try_from(out.len()).ok()?;
    write_u16_be(&mut out, 2, at);
    out.extend(new_first.to_be_bytes());
    out.extend(u16::try_from(row.len()).ok()?.to_be_bytes());
    out.extend(row);
    Some(out)
}

// Class tables of the kept glyphs, then the array out to the furthest word a kept pair reaches within the subtable,
// as a read past it gives 0 in both. A left class, an offset counting the array's, moves with it, never below 0 or
// onto 1, which reads as none; past 65,535 it's left out. An array past the subtable reads as one at its end.
fn remap_kern_format2(sub: &[u8], body: usize, gid_map: &[u32], active: &GlyphSet) -> Option<Vec<u8>> {
    let row_width = read_u16_be(sub, body)?;
    let array = usize::from(read_u16_be(sub, body + 6)?).min(sub.len());
    let classes = |at: usize| -> Option<Vec<(u16, u16)>> {
        let (first, values) = kern_classes(sub, usize::from(read_u16_be(sub, body + at)?))?;
        let mut kept: Vec<(u16, u16)> = values.iter().enumerate()
            .filter(|&(_, &v)| v != 1)
            .filter_map(|(i, &v)| Some((remap_gid16(active, gid_map, first.checked_add(u16::try_from(i).ok()?)?)?, v)))
            .collect();
        kept.sort_unstable();
        Some(kept)
    };
    let (left, right) = (classes(2)?, classes(4)?);
    let span = |entries: &[(u16, u16)]| match (entries.first(), entries.last()) {
        (Some(&(first, _)), Some(&(last, _))) => (first, usize::from(last - first) + 1),
        _ => (0, 0),
    };
    let highest = |entries: &[(u16, u16)]| entries.iter().map(|&(_, v)| usize::from(v)).max();
    let reach = highest(&left).zip(highest(&right)).and_then(|(l, r)| (l + r).checked_sub(array));
    let words = reach.map_or(0, |reach| reach / 2 + 1).min((sub.len() - array) / 2);

    let table = |entries: &[(u16, u16)], shift: i32| -> Option<Vec<u8>> {
        let (first, len) = span(entries);
        let mut values = vec![1u16; len];
        for &(gid, v) in entries {
            if let Ok(v) = u16::try_from(i32::from(v) + shift) {
                values[usize::from(gid - first)] = v;
            }
        }
        Some([first.to_be_bytes(), u16::try_from(len).ok()?.to_be_bytes()].concat().into_iter()
            .chain(values.iter().flat_map(|v| v.to_be_bytes())).collect())
    };
    let left_at = body + 8;
    let left_len = 4 + 2 * span(&left).1;
    let tables_end = left_at + left_len + 4 + 2 * span(&right).1;
    let mut shift = left.iter().map(|&(_, v)| -i32::from(v)).fold(tables_end as i32 - array as i32, i32::max);
    if left.iter().any(|&(_, v)| i32::from(v) + shift == 1) { shift += 2; }
    let new_array = array.checked_add_signed(shift as isize)?;
    let left_table = table(&left, shift)?;
    let right_table = table(&right, 0)?;

    let mut out = Vec::with_capacity(new_array - body + 2 * words);
    out.extend(row_width.to_be_bytes());
    for at in [left_at, left_at + left_len, new_array] {
        out.extend(u16::try_from(at).ok()?.to_be_bytes());
    }
    out.extend(left_table);
    out.extend(right_table);
    out.resize(new_array - body, 0);
    (0..words).for_each(|i| out.extend(read_i16_be(sub, array + 2 * i).unwrap_or(0).to_be_bytes()));
    Some(out)
}

// Only what kept glyphs read, each renumbered in order, so each kept pair reads the value it did. A class or index past
// its count reads 0, so it takes an in-range 0 where a count can hold one more.
fn remap_kern_format3(sub: &[u8], body: usize, gid_map: &[u32], active: &GlyphSet) -> Option<Vec<u8>> {
    let glyphs = usize::from(read_u16_be(sub, body)?);
    let count = |i: usize| sub.get(body + i).map(|&n| usize::from(n));
    let (values, lefts, rights) = (count(2)?, count(3)?, count(4)?);
    let values_at = body + 6;
    let lefts_at = values_at + values * 2;
    let rights_at = lefts_at + glyphs;
    let index_at = rights_at + glyphs;
    let index = sub.get(index_at..index_at + lefts * rights)?;
    if glyphs == 0 {
        return sub.get(body..index_at + index.len()).map(<[u8]>::to_vec);
    }

    let kept: Vec<(u16, u16)> = active.iter().filter_map(|g| {
        let g = u16::try_from(g).ok()?;
        Some((g, remap_gid16(active, gid_map, g)?))
    }).collect();
    let class = |at: usize, g: u16| if usize::from(g) < glyphs { usize::from(sub[at + usize::from(g)]) } else { 0 };
    // Each side's classes in use, ascending, and whether a kept glyph reads one past the count.
    let used = |at: usize, n: usize| {
        let classes: BTreeSet<usize> = kept.iter().map(|&(old, _)| class(at, old)).collect();
        let past = classes.last().is_some_and(|&c| c >= n);
        (classes.into_iter().filter(|&c| c < n).collect::<Vec<_>>(), past)
    };
    let ((rows, past_row), (cols, past_col)) = (used(lefts_at, lefts), used(rights_at, rights));
    let cell = |l: usize, r: usize| Some(usize::from(index[l * rights + r])).filter(|&k| k < values);
    let named: BTreeSet<usize> = rows.iter().flat_map(|&l| cols.iter().filter_map(move |&r| cell(l, r))).collect();
    let mut kept_values: Vec<usize> = named.into_iter().collect();
    // A cell reading 0 names a value of 0 already kept, else one added.
    let reads_zero = past_row || past_col || rows.iter().any(|&l| cols.iter().any(|&r| cell(l, r).is_none()));
    let value = |k: usize| read_i16_be(sub, values_at + 2 * k).unwrap_or(0);
    let zero = match kept_values.iter().position(|&k| value(k) == 0) {
        Some(z) => z,
        None => kept_values.len(),
    };
    let add_zero = reads_zero && zero == kept_values.len() && zero < 255;
    if add_zero {
        kept_values.push(usize::MAX);
    }
    // A side past its count reads a row or column of zeros, added where the count can hold it.
    let (zero_row, zero_col) = (rows.len(), cols.len());
    let new_lefts = rows.len() + usize::from(past_row && zero_row < 255);
    let new_rights = cols.len() + usize::from(past_col && zero_col < 255);

    let renumber = |at: usize, kept_classes: &[usize], past: usize| {
        let mut out = vec![0u8; kept.iter().map(|&(_, new)| usize::from(new) + 1).max().unwrap_or(0)];
        for &(old, new) in &kept {
            let c = class(at, old);
            out[usize::from(new)] = kept_classes.binary_search(&c).unwrap_or(past) as u8;
        }
        out
    };
    let lefts_out = renumber(lefts_at, &rows, zero_row);
    let rights_out = renumber(rights_at, &cols, zero_col);
    let mut out = Vec::with_capacity(6 + 2 * kept_values.len() + 2 * lefts_out.len() + new_lefts * new_rights + 1);
    out.extend(u16::try_from(lefts_out.len()).ok()?.to_be_bytes());
    out.extend([kept_values.len() as u8, new_lefts as u8, new_rights as u8, sub[body + 5]]);
    kept_values.iter().for_each(|&k| out.extend(if k == usize::MAX { 0 } else { value(k) }.to_be_bytes()));
    out.extend(&lefts_out);
    out.extend(&rights_out);
    for l in (0..new_lefts).map(|i| rows.get(i)) {
        for r in (0..new_rights).map(|i| cols.get(i)) {
            let k = l.zip(r).and_then(|(&l, &r)| cell(l, r));
            out.push(k.map_or(zero, |k| kept_values.binary_search(&k).unwrap_or(zero)) as u8);
        }
    }
    Some(out)
}

// Each kept glyph's advance and bearing as shaping reads them, trailing equal advances as bearings, with the long
// count; none without long records. Past 65,535 a glyph keeps its bearing alone unless `wide` (HMTX, VMTX).
pub fn rebuild_metrics(
    mtx: &[u8], num_long: usize, num_glyphs: usize, active_sorted: &[u32], wide: bool,
) -> (Vec<u8>, usize) {
    let Some(table) = MetricsTable::read(mtx, num_long, num_glyphs).filter(MetricsTable::has_data) else {
        return (vec![0; 2 * active_sorted.len()], 0);
    };
    let mut out = vec![0u8; active_sorted.len() * 4];
    for (compact, &orig_gid) in active_sorted.iter().enumerate() {
        let (adv, bearing) = table.pair(orig_gid as usize);
        write_u16_be(&mut out, compact * 4, adv);
        write_i16_be(&mut out, compact * 4 + 2, bearing);
    }
    let (out, long) = compacted(out, active_sorted.len());
    if wide { (out, long) } else { narrowed(out, long) }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/remap.rs"]
mod tests;
