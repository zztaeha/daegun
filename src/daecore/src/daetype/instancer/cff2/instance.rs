#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use crate::daecore::daetype::decoder::read_u16_be;
use crate::daecore::daetype::instancer::{compute_location, finish, vary_metrics};
use crate::daecore::daetype::subsetter::cff::build::{cff_index_size, cff_index_flat_size, append_cff_index_chunks, encode_cff_index, encode_cff_index_flat, encode_cff_int};
use super::parse::parse_cff2;
use crate::daecore::daetype::format::cff::{decode_cff_number, decode_cff_real, encode_dict_number};
use crate::daecore::daetype::format::ivs::{region_scalars, ItemVariationStore};
use crate::daecore::daetype::format::round::ot_round;
use crate::daecore::daetype::TableBytes;

pub(crate) fn instance_cff2_from_map<'a>(
    table_map:   &'a BTreeMap<String, TableBytes>,
    axis_values: &[(String, f64)],
    extents:     bool,
) -> Result<BTreeMap<String, Cow<'a, [u8]>>, String> {
    // Without a readable fvar the font is static, and its default instance is the only one. glyf can
    // be drawn as it is, but CFF2 has to become CFF either way.
    let located   = compute_location(table_map, axis_values);
    let is_static = located.is_err();
    let location  = located.unwrap_or_default();

    let cff2_data = table_map.get("CFF2").ok_or("missing CFF2")?;
    let cff2 = parse_cff2(cff2_data)?;
    // CFF counts glyphs in a Card16, and maxp says how many there are.
    let maxp_glyphs = table_map.get("maxp").and_then(|m| read_u16_be(m, 4)).map_or(usize::MAX, usize::from);
    let n_glyphs = cff2.charstrings.len().min(maxp_glyphs).min(0xFFFF);

    let mut cff2_budget =
        u32::try_from(cff2_data.len().saturating_mul(64)).unwrap_or(u32::MAX);

    let mut scratch = super::charstring::Scratch::default();
    let (chunks, charstring_ends) =
        resolve_all_charstrings(&cff2, &location, n_glyphs, &mut cff2_budget, &mut scratch)?;

    let chunk_refs: Vec<&[u8]> = chunks.iter().map(|t| t.as_slice()).collect();
    let privates: Vec<Vec<u8>> = cff2.fds.iter()
        .map(|fd| resolve_private(fd.private, fd.vsindex, cff2.vstore.as_ref(), &location))
        .collect();
    let cff1 = build_cff(&chunk_refs, &charstring_ends, &cff2, &privates, &instance_font_name(axis_values))?;

    let num_glyphs_hint = table_map.get("maxp").and_then(|m| read_u16_be(m, 4)).unwrap_or(0) as usize;
    let metrics = vary_metrics(table_map, num_glyphs_hint, &location, None)?;
    let bounds = (extents && location.iter().any(|&v| v != 0.0)).then(|| cff_bounds(&cff1, &cff2, n_glyphs)).flatten();
    let mut out_map = finish(table_map, &location, axis_values, is_static, metrics, bounds.as_deref());
    out_map.insert("CFF ".to_string(), Cow::Owned(cff1));
    Ok(out_map)
}

// Each glyph's box, its extremes rounded outward as fontTools rounds a CFF font's. Drawn in the
// ranges the charstrings were resolved in, where that was done in parallel.
fn cff_bounds(cff: &[u8], cff2: &super::parse::Cff2Font<'_>, n_glyphs: usize) -> Option<Vec<Option<[i16; 4]>>> {
    use crate::daecore::daetype::outline::CffOutlines;
    let outlines = CffOutlines::parse(cff).ok()?;
    let boxes = |range: core::ops::Range<usize>| range.map(|gid| glyph_box(&outlines, cff, gid)).collect::<Vec<_>>();
    #[cfg(feature = "threading")]
    if let Some(ranges) = parallel_ranges(cff2, n_glyphs) {
        let boxes = &boxes;
        let parts = std::thread::scope(|scope| {
            let handles: Vec<_> = ranges.into_iter().map(|range| scope.spawn(move || boxes(range))).collect();
            handles.into_iter().map(|h| h.join().ok()).collect::<Option<Vec<_>>>()
        })?;
        return Some(parts.concat());
    }
    #[cfg(not(feature = "threading"))]
    let _ = cff2;
    Some(boxes(0..n_glyphs))
}

fn glyph_box(outlines: &crate::daecore::daetype::outline::CffOutlines, cff: &[u8], gid: usize) -> Option<[i16; 4]> {
    use crate::daecore::daetype::outline::{outline_cff_glyph_with, Bounds};
    let mut bounds = Bounds::default();
    outline_cff_glyph_with(outlines, cff, u16::try_from(gid).ok()?, &mut bounds).ok()?;
    let (x0, y0, x1, y1) = bounds.finish()?;
    let clamp = |v: f64| v.clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16;
    Some([clamp(x0.floor()), clamp(y0.floor()), clamp(x1.ceil()), clamp(y1.ceil())])
}

struct Resolved {
    chunks: Vec<Vec<u8>>,
    ends:   Vec<usize>,
}

const CHUNK_TARGET: usize = 128 << 10;

const CHUNK_CUT: usize = CHUNK_TARGET - (16 << 10);

fn resolve_all_charstrings(
    cff2:     &super::parse::Cff2Font<'_>,
    location: &[f64],
    n_glyphs: usize,
    budget:   &mut u32,
    scratch:  &mut super::charstring::Scratch,
) -> Result<(Vec<Vec<u8>>, Vec<usize>), String> {
    #[cfg(feature = "threading")]
    if let Some(ranges) = parallel_ranges(cff2, n_glyphs) {
        return resolve_in_parallel(cff2, location, &ranges, budget);
    }
    let r = resolve_charstrings(cff2, location, 0..n_glyphs, budget, scratch)?;
    Ok((r.chunks, r.ends))
}

#[cfg(feature = "threading")]
const PARALLEL_FLOOR: usize = 512 << 10;

#[cfg(feature = "threading")]
fn parallel_ranges(cff2: &super::parse::Cff2Font<'_>, n_glyphs: usize) -> Option<Vec<core::ops::Range<usize>>> {
    let total: usize = cff2.charstrings.iter().map(|c| c.len()).sum();
    if total < PARALLEL_FLOOR || n_glyphs < 2 {
        return None;
    }
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(n_glyphs);
    if threads < 2 {
        return None;
    }
    let mut ranges = Vec::with_capacity(threads);
    let (mut start, mut acc, mut cut) = (0usize, 0usize, 1usize);
    for gid in 0..n_glyphs {
        acc += cff2.charstrings[gid].len();
        if cut < threads && acc * threads >= cut * total {
            ranges.push(start..gid + 1);
            start = gid + 1;
            cut += 1;
        }
    }
    if start < n_glyphs {
        ranges.push(start..n_glyphs);
    }
    (ranges.len() > 1).then_some(ranges)
}

#[cfg(feature = "threading")]
fn resolve_in_parallel(
    cff2:     &super::parse::Cff2Font<'_>,
    location: &[f64],
    ranges:   &[core::ops::Range<usize>],
    budget:   &mut u32,
) -> Result<(Vec<Vec<u8>>, Vec<usize>), String> {
    let total: usize = cff2.charstrings.iter().map(|c| c.len()).sum::<usize>().max(1);
    let whole = *budget;

    let results: Vec<Result<(Resolved, u32), String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = ranges
            .iter()
            .map(|range| {
                let range = range.clone();
                let bytes: usize = cff2.charstrings[range.clone()].iter().map(|c| c.len()).sum();
                let share = ((u64::from(whole) * bytes as u64) / total as u64) as u32;
                scope.spawn(move || {
                    let mut own = share;
                    let mut scratch = super::charstring::Scratch::default();
                    resolve_charstrings(cff2, location, range, &mut own, &mut scratch)
                        .map(|r| (r, share - own))
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| Err(String::from("CFF2: charstring worker failed"))))
            .collect()
    });

    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut ends:   Vec<usize>   = Vec::with_capacity(cff2.charstrings.len());
    let mut base = 0usize;
    let mut spent = 0u32;
    for result in results {
        let (resolved, used) = result?;
        for end in &resolved.ends {
            ends.push(base + end);
        }
        base += resolved.chunks.iter().map(|t| t.len()).sum::<usize>();
        chunks.extend(resolved.chunks);
        spent = spent.saturating_add(used);
    }
    *budget -= spent.min(whole);
    Ok((chunks, ends))
}

fn resolve_charstrings(
    cff2:    &super::parse::Cff2Font<'_>,
    location: &[f64],
    range:    core::ops::Range<usize>,
    budget:   &mut u32,
    scratch:  &mut super::charstring::Scratch,
) -> Result<Resolved, String> {
    let mut ends: Vec<usize> = Vec::with_capacity(range.len());
    let mut chunks: Vec<Vec<u8>> = Vec::new();
    let mut cur: Vec<u8> = Vec::with_capacity(CHUNK_TARGET);
    let mut done = 0;
    for gid in range {
        if cur.len() >= CHUNK_CUT {
            done += cur.len();
            chunks.push(core::mem::replace(&mut cur, Vec::with_capacity(CHUNK_TARGET)));
        }
        let fd_idx = *cff2.fd_select.get(gid).unwrap_or(&0) as usize;
        let fd = cff2.fds.get(fd_idx)
            .ok_or("CFF2: FDSelect references an FD index out of range")?;
        super::charstring::evaluate_charstring_into(
            cff2.charstrings[gid],
            &cff2.global_subrs,
            &cff2.subr_sets[fd.subrs],
            fd.vsindex,
            cff2.vstore.as_ref(),
            location,
            budget,
            scratch,
            &mut cur,
        )?;
        cur.push(0x0e);
        ends.push(done + cur.len());
    }
    if cur.capacity() > cur.len().saturating_mul(2) {
        cur.shrink_to_fit();
    }
    chunks.push(cur);
    Ok(Resolved { chunks, ends })
}

fn instance_font_name(axis_values: &[(String, f64)]) -> Vec<u8> {
    let mut name = String::from("DaegunInstance");
    for (tag, v) in axis_values.iter().take(16) {
        name.push('-');
        name.extend(tag.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).take(8));
        name.push_str(&((v * 100.0).round() as i64).to_string());
    }
    name.truncate(100);
    name.into_bytes()
}

const FIRST_CUSTOM_SID: u16 = 391;

// A CFF2 Private DICT as CFF holds it: each blend taken at the location on the operands as stored
// (deltas, for the blue arrays), vsindex, blend and Subrs left out. BlueScale and ExpansionFactor
// stay fractional; every other operator here holds design units, which round.
fn resolve_private(private: &[u8], vsindex: u16, store: Option<&ItemVariationStore>, location: &[f64]) -> Vec<u8> {
    let scalars = store.and_then(|s| region_scalars(s, usize::from(vsindex), location)).unwrap_or_default();
    let (mut operands, mut out) = (Vec::<f64>::new(), Vec::new());
    let mut off = 0usize;
    while let Some(&b) = private.get(off) {
        if b > 24 {
            let read = if b == 30 { decode_cff_real(private, off) } else { decode_cff_number(private, off).ok().map(|(v, n)| (f64::from(v), n)) };
            let Some((value, width)) = read else { break };
            operands.push(value);
            off += width;
            continue;
        }
        let op = if b == 12 { 0x0C00 | u16::from(*private.get(off + 1).unwrap_or(&0)) } else { u16::from(b) };
        off += if b == 12 { 2 } else { 1 };
        match op {
            23 => {
                let k = scalars.len();
                let Some(n) = operands.pop().map(|n| n as usize) else { break };
                let Some(base) = n.checked_mul(k + 1).and_then(|need| operands.len().checked_sub(need)) else { break };
                let blended: Vec<f64> = (0..n)
                    .map(|i| {
                        let delta: f64 = (0..k).map(|r| scalars[r] * operands[base + n + i * k + r]).sum();
                        operands[base + i] + delta
                    })
                    .collect();
                operands.truncate(base);
                operands.extend(blended);
            }
            19 | 22 => operands.clear(),
            _ => {
                let real = matches!(op, 0x0C09 | 0x0C12);
                for v in operands.drain(..) {
                    out.extend(encode_dict_number(if real { v } else { f64::from(ot_round(v)) }));
                }
                if op > 0xFF { out.push(12); }
                out.push(op as u8);
            }
        }
    }
    out
}

// Name-keyed when one Private DICT serves every glyph and the custom names fit a Card16 SID;
// CID-keyed otherwise, with its FDs and FDSelect, so no glyph needs a name.
fn build_cff(chunks: &[&[u8]], ends: &[usize], cff2: &super::parse::Cff2Font<'_>, privates: &[Vec<u8>], font_name: &[u8]) -> Result<Vec<u8>, String> {
    let n_glyphs = ends.len();
    let too_many = || String::from("CFF2: more glyphs than a CFF INDEX can count");
    if privates.len() == 1 && n_glyphs.saturating_sub(1) <= usize::from(u16::MAX - FIRST_CUSTOM_SID) + 1 {
        return build_cff1(chunks, ends, cff2.font_matrix_raw.as_ref(), font_name, &privates[0]).ok_or_else(too_many);
    }
    if privates.len() > 256 {
        return Err("CFF2: more FDs than a CFF FDSelect can name".into());
    }
    build_cid_cff(chunks, ends, cff2.font_matrix_raw.as_ref(), font_name, &cff2.fd_select[..n_glyphs], privates).ok_or_else(too_many)
}

fn build_cff1(charstring_chunks: &[&[u8]], charstring_ends: &[usize], font_matrix_raw: Option<&Vec<u8>>, font_name: &[u8], private: &[u8]) -> Option<Vec<u8>> {
    let hdr_size = 4usize;
    let n_glyphs = charstring_ends.len();
    let named = n_glyphs.saturating_sub(1);

    let mut string_data: Vec<u8> = Vec::with_capacity(named * 6);
    let mut string_ends: Vec<usize> = Vec::with_capacity(named);
    let mut digits = [0u8; 20];
    for i in 1..=named {
        string_data.push(b'g');
        let mut n = i;
        let mut d = 0;
        while {
            digits[d] = b'0' + (n % 10) as u8;
            n /= 10;
            d += 1;
            n != 0
        } {}
        string_data.extend(digits[..d].iter().rev());
        string_ends.push(string_data.len());
    }
    let charset = build_charset(named);

    let name_index_bytes   = encode_cff_index(&[font_name.to_vec()])?;
    let string_index_bytes = encode_cff_index_flat(&string_data, &string_ends)?;
    let gsubr_index_bytes  = encode_cff_index(&[])?;
    let charstrings_index_len = cff_index_flat_size(charstring_ends);

    let fm_len = font_matrix_raw.map_or(0, |m| m.len());
    let top_dict_data_len   = 23 + fm_len;
    let top_dict_index_size = cff_index_size(top_dict_data_len);

    let base = hdr_size + name_index_bytes.len() + top_dict_index_size
        + string_index_bytes.len() + gsubr_index_bytes.len();
    let new_charset_off     = base;
    let new_charstrings_off = new_charset_off + charset.len();
    let new_private_off     = new_charstrings_off + charstrings_index_len;

    let mut top_dict_data = Vec::with_capacity(top_dict_data_len);
    if let Some(fm) = font_matrix_raw {
        top_dict_data.extend_from_slice(fm);
    }
    top_dict_data.extend(encode_cff_int(new_charset_off as i32));
    top_dict_data.push(15);
    top_dict_data.extend(encode_cff_int(new_charstrings_off as i32));
    top_dict_data.push(17);
    top_dict_data.extend(encode_cff_int(private.len() as i32));
    top_dict_data.extend(encode_cff_int(new_private_off as i32));
    top_dict_data.push(18);

    let top_dict_index = encode_cff_index(&[top_dict_data])?;
    debug_assert_eq!(top_dict_index.len(), top_dict_index_size);

    let mut out = Vec::with_capacity(new_private_off + private.len());
    out.extend_from_slice(&[1, 0, hdr_size as u8, 4]);
    out.extend_from_slice(&name_index_bytes);
    out.extend_from_slice(&top_dict_index);
    out.extend_from_slice(&string_index_bytes);
    out.extend_from_slice(&gsubr_index_bytes);
    out.extend_from_slice(&charset);
    append_cff_index_chunks(&mut out, charstring_chunks, charstring_ends)?;
    out.extend_from_slice(private);
    Some(out)
}

// ROS Adobe-Identity-0, each glyph its own CID, and one Font DICT per CFF2 FD.
fn build_cid_cff(chunks: &[&[u8]], ends: &[usize], font_matrix_raw: Option<&Vec<u8>>, font_name: &[u8], fd_select: &[u16], privates: &[Vec<u8>]) -> Option<Vec<u8>> {
    let n_glyphs = ends.len();
    let name_index = encode_cff_index(&[font_name.to_vec()])?;
    let string_index = encode_cff_index(&[b"Adobe".to_vec(), b"Identity".to_vec()])?;
    let gsubr_index = encode_cff_index(&[])?;
    let charset = match n_glyphs {
        0 | 1 => alloc::vec![0],
        n => {
            let mut c = alloc::vec![2, 0, 1];
            c.extend_from_slice(&u16::try_from(n - 2).unwrap_or(u16::MAX).to_be_bytes());
            c
        }
    };
    let mut fd_ranges: Vec<(u16, u8)> = Vec::new();
    for (gid, &fd) in fd_select.iter().enumerate() {
        if fd_ranges.last().is_none_or(|&(_, last)| u16::from(last) != fd) {
            fd_ranges.push((gid as u16, fd as u8));
        }
    }
    let mut fd_select_bytes = alloc::vec![3];
    fd_select_bytes.extend_from_slice(&(fd_ranges.len() as u16).to_be_bytes());
    for (first, fd) in &fd_ranges {
        fd_select_bytes.extend_from_slice(&first.to_be_bytes());
        fd_select_bytes.push(*fd);
    }
    fd_select_bytes.extend_from_slice(&(n_glyphs as u16).to_be_bytes());

    let fm_len = font_matrix_raw.map_or(0, |m| m.len());
    // ROS (5 + 5 + 5 + 2), CIDCount (5 + 2), charset, FDSelect, CharStrings and FDArray offsets.
    let top_len = 17 + 7 + fm_len + 6 + 7 + 6 + 7;
    let base = 4 + name_index.len() + cff_index_size(top_len) + string_index.len() + gsubr_index.len();
    let charset_off = base;
    let fd_select_off = charset_off + charset.len();
    let charstrings_off = fd_select_off + fd_select_bytes.len();
    let fd_array_off = charstrings_off + cff_index_flat_size(ends);
    let fd_array_len = encode_cff_index(&alloc::vec![alloc::vec![0u8; 11]; privates.len()])?.len();
    let mut private_off = fd_array_off + fd_array_len;
    let font_dicts: Vec<Vec<u8>> = privates
        .iter()
        .map(|p| {
            let mut d = encode_cff_int(p.len() as i32);
            d.extend(encode_cff_int(private_off as i32));
            d.push(18);
            private_off += p.len();
            d
        })
        .collect();

    let mut top = Vec::with_capacity(top_len);
    top.extend(encode_cff_int(i32::from(FIRST_CUSTOM_SID)));
    top.extend(encode_cff_int(i32::from(FIRST_CUSTOM_SID) + 1));
    top.extend(encode_cff_int(0));
    top.extend_from_slice(&[12, 30]);
    top.extend(encode_cff_int(n_glyphs as i32));
    top.extend_from_slice(&[12, 34]);
    if let Some(fm) = font_matrix_raw {
        top.extend_from_slice(fm);
    }
    top.extend(encode_cff_int(charset_off as i32));
    top.push(15);
    top.extend(encode_cff_int(fd_select_off as i32));
    top.extend_from_slice(&[12, 37]);
    top.extend(encode_cff_int(charstrings_off as i32));
    top.push(17);
    top.extend(encode_cff_int(fd_array_off as i32));
    top.extend_from_slice(&[12, 36]);
    debug_assert_eq!(top.len(), top_len);

    let mut out = Vec::with_capacity(private_off);
    out.extend_from_slice(&[1, 0, 4, 4]);
    out.extend_from_slice(&name_index);
    out.extend_from_slice(&encode_cff_index(&[top])?);
    out.extend_from_slice(&string_index);
    out.extend_from_slice(&gsubr_index);
    out.extend_from_slice(&charset);
    out.extend_from_slice(&fd_select_bytes);
    append_cff_index_chunks(&mut out, chunks, ends)?;
    out.extend_from_slice(&encode_cff_index(&font_dicts)?);
    privates.iter().for_each(|p| out.extend_from_slice(p));
    Some(out)
}

fn build_charset(named: usize) -> Vec<u8> {
    if named == 0 {
        return vec![0];
    }
    let mut out = vec![2u8];
    out.extend_from_slice(&FIRST_CUSTOM_SID.to_be_bytes());
    out.extend_from_slice(&u16::try_from(named - 1).unwrap_or(u16::MAX).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::parse::{Cff2Fd, Cff2Font};

    // Source Han's charstrings pass the parallel floor, so its boxes are drawn in ranges at once:
    // each glyph must get its own, as one thread drawing them in order gives it.
    #[cfg(feature = "threading")]
    #[test]
    fn boxes_drawn_in_parallel_are_each_glyphs_own() {
        let path = alloc::format!("{}/assets/test-fonts/source-han-sans/SourceHanSansJP-VF.otf", env!("CARGO_MANIFEST_DIR"));
        let map = crate::daecore::daetype::decoder::extract_ttf_tables(&std::fs::read(&path).expect("a fixture")).expect("it parses");
        let cff2 = parse_cff2(map.get("CFF2").expect("CFF2")).expect("CFF2 parses");
        let n = cff2.charstrings.len();
        assert!(parallel_ranges(&cff2, n).is_some(), "the font no longer passes the parallel floor");
        let instance = instance_cff2_from_map(&map, &[("wght".into(), 700.0)], false).expect("an instance");
        let cff = instance.get("CFF ").expect("CFF");
        let outlines = crate::daecore::daetype::outline::CffOutlines::parse(cff).expect("outlines");
        let in_order: Vec<_> = (0..n).map(|gid| glyph_box(&outlines, cff, gid)).collect();
        assert_eq!(cff_bounds(cff, &cff2, n), Some(in_order));
    }

    fn converted(n_glyphs: usize, fds: usize) -> Vec<u8> {
        let chunk = alloc::vec![0x0e; n_glyphs];
        let ends: Vec<usize> = (1..=n_glyphs).collect();
        let cff2 = Cff2Font {
            charstrings: alloc::vec![&[][..]; n_glyphs],
            global_subrs: Vec::new(),
            fd_select: (0..n_glyphs).map(|g| (g % fds) as u16).collect(),
            fds: (0..fds).map(|_| Cff2Fd { vsindex: 0, subrs: 0, private: &[] }).collect(),
            subr_sets: alloc::vec![Vec::new()],
            vstore: None,
            font_matrix_raw: None,
        };
        build_cff(&[&chunk], &ends, &cff2, &alloc::vec![alloc::vec![]; fds], b"T").expect("builds")
    }

    fn cid_keyed(cff: &[u8]) -> bool {
        let top = 4 + 2 + 1 + 2 + 1 + 3 + 2 + 1;
        cff.get(top..top + 20).is_some_and(|t| t.windows(2).any(|w| w == [12, 30]))
    }

    #[test]
    fn a_font_past_what_a_cff_index_counts_is_refused() {
        let n_glyphs = 70_000;
        let chunk = alloc::vec![0x0e; n_glyphs];
        let ends: Vec<usize> = (1..=n_glyphs).collect();
        let cff2 = Cff2Font {
            charstrings: alloc::vec![&[][..]; n_glyphs],
            global_subrs: Vec::new(),
            fd_select: alloc::vec![0; n_glyphs],
            fds: alloc::vec![Cff2Fd { vsindex: 0, subrs: 0, private: &[] }],
            subr_sets: alloc::vec![Vec::new()],
            vstore: None,
            font_matrix_raw: None,
        };
        assert!(build_cff(&[&chunk], &ends, &cff2, &[alloc::vec![]], b"T").is_err());
    }

    #[test]
    fn a_font_past_the_names_cff_can_hold_is_cid_keyed() {
        assert!(!cid_keyed(&converted(65_146, 1)));
        assert!(cid_keyed(&converted(65_147, 1)));
        assert!(cid_keyed(&converted(10, 3)), "several FDs need a Font DICT each");
        use crate::daecore::daetype::outline::{outline_cff_glyph_with, Bounds, CffOutlines};
        let cff = converted(65_147, 1);
        let outlines = CffOutlines::parse(&cff).expect("parses");
        assert!(outlines_cff_last(&outlines, &cff, 65_146).is_ok() && outlines_cff_last(&outlines, &cff, 65_147).is_err());
        fn outlines_cff_last(o: &CffOutlines, cff: &[u8], gid: u16) -> Result<(), String> {
            outline_cff_glyph_with(o, cff, gid, &mut Bounds::default())
        }
    }
}
