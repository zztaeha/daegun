use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use super::super::decoder::{write_u16_be, write_i16_be, write_u32_be};

// Format 4 addresses the BMP only. A supplementary-plane codepoint therefore
// needs a format 12 subtable beside it, or the subset cannot express a mapping
// it otherwise carries the glyph for.
pub fn build_unicode_cmap(mappings: &[(u32, u16)]) -> Vec<u8> {
    let mut sorted: Vec<(u32, u16)> = mappings.iter().copied().filter(|&(cp, _)| cp <= 0xFFFF).collect();
    sorted.sort_by_key(|&(cp, _)| cp);

    struct Seg { start: u16, end: u16, delta: i16 }
    let mut segs: Vec<Seg> = sorted.iter().map(|&(cp, gid)| {
        let cp16 = cp as u16;
        let delta = (gid.wrapping_sub(cp16)) as i16;
        Seg { start: cp16, end: cp16, delta }
    }).collect();
    segs.push(Seg { start: 0xFFFF, end: 0xFFFF, delta: 1 });

    let seg_count = segs.len();

    let header_len = 14usize;
    let arrays_len = seg_count * 2 * 4 + 2;
    let subtable_len = header_len + arrays_len;
    if subtable_len > u16::MAX as usize || seg_count * 2 > u16::MAX as usize {
        return Vec::new();
    }

    let floor_log2 = 31 - (seg_count as u32).leading_zeros();
    let search_range = (2 * (1u32 << floor_log2)) as u16;
    let entry_selector = floor_log2 as u16;
    let range_shift = (seg_count as u16) * 2 - search_range;

    let mut sub = vec![0u8; subtable_len];
    write_u16_be(&mut sub, 0, 4);
    write_u16_be(&mut sub, 2, subtable_len as u16);
    write_u16_be(&mut sub, 4, 0);
    write_u16_be(&mut sub, 6, (seg_count * 2) as u16);
    write_u16_be(&mut sub, 8, search_range);
    write_u16_be(&mut sub, 10, entry_selector);
    write_u16_be(&mut sub, 12, range_shift);

    let mut p = header_len;
    for s in &segs { write_u16_be(&mut sub, p, s.end); p += 2; }
    p += 2;
    for s in &segs { write_u16_be(&mut sub, p, s.start); p += 2; }
    for s in &segs { write_i16_be(&mut sub, p, s.delta); p += 2; }
    for _ in &segs { write_u16_be(&mut sub, p, 0); p += 2; }

    // (0,3) and (3,1) select the BMP subtable; (0,4) and (3,10) the full-range
    // one, which is emitted only when something actually sits above the BMP
    let wide = build_format12(mappings);
    let records: &[(u16, u16, bool)] = if wide.is_empty() {
        &[(0, 3, false), (3, 1, false)]
    } else {
        &[(0, 3, false), (3, 1, false), (0, 4, true), (3, 10, true)]
    };

    let cmap_header_len = 4 + records.len() * 8;
    let fmt4_off = cmap_header_len as u32;
    let fmt12_off = fmt4_off + sub.len() as u32;

    let mut cmap = vec![0u8; cmap_header_len + sub.len() + wide.len()];
    write_u16_be(&mut cmap, 0, 0);
    write_u16_be(&mut cmap, 2, records.len() as u16);
    for (i, &(platform_id, encoding_id, is_wide)) in records.iter().enumerate() {
        let off = 4 + i * 8;
        write_u16_be(&mut cmap, off, platform_id);
        write_u16_be(&mut cmap, off + 2, encoding_id);
        write_u32_be(&mut cmap, off + 4, if is_wide { fmt12_off } else { fmt4_off });
    }
    cmap[cmap_header_len..cmap_header_len + sub.len()].copy_from_slice(&sub);
    cmap[cmap_header_len + sub.len()..].copy_from_slice(&wide);
    cmap
}

// Covers the whole mapping, BMP included, so the two subtables agree wherever
// they overlap. Empty when nothing needs it, to keep existing output byte-identical.
fn build_format12(mappings: &[(u32, u16)]) -> Vec<u8> {
    if !mappings.iter().any(|&(cp, _)| cp > 0xFFFF) {
        return Vec::new();
    }
    let mut sorted: Vec<(u32, u16)> = mappings.to_vec();
    sorted.sort_by_key(|&(cp, _)| cp);
    sorted.dedup_by_key(|&mut (cp, _)| cp);

    let mut groups: Vec<(u32, u32, u16)> = Vec::new();
    for &(cp, gid) in &sorted {
        match groups.last_mut() {
            // a run continues only while codepoint and glyph id advance together
            Some(g) if cp == g.1 + 1 && u32::from(gid) == u32::from(g.2) + (g.1 - g.0) + 1 => {
                g.1 = cp;
            }
            _ => groups.push((cp, cp, gid)),
        }
    }

    let len = 16 + groups.len() * 12;
    let mut sub = vec![0u8; len];
    write_u16_be(&mut sub, 0, 12);
    write_u16_be(&mut sub, 2, 0);
    write_u32_be(&mut sub, 4, len as u32);
    write_u32_be(&mut sub, 8, 0);
    write_u32_be(&mut sub, 12, groups.len() as u32);
    for (i, &(start, end, gid)) in groups.iter().enumerate() {
        let off = 16 + i * 12;
        write_u32_be(&mut sub, off, start);
        write_u32_be(&mut sub, off + 4, end);
        write_u32_be(&mut sub, off + 8, u32::from(gid));
    }
    sub
}

fn utf16be(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
}

fn postscript_safe(s: &str) -> String {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
    cleaned.chars().take(63).collect()
}

pub fn build_name_table(base_name: &str) -> Vec<u8> {
    let ps_name = postscript_safe(base_name);
    let records: [(u16, String); 5] = [
        (1, base_name.to_string()),
        (2, "Regular".to_string()),
        (3, format!("{ps_name};daegun-subset")),
        (4, base_name.to_string()),
        (6, ps_name),
    ];

    let strings: Vec<Vec<u8>> = records.iter().map(|(_, v)| utf16be(v)).collect();
    let header_len = 6;
    let record_len = 12;
    let string_storage_offset = header_len + records.len() * record_len;

    let mut str_off = 0usize;
    let mut offsets = Vec::with_capacity(records.len());
    for s in &strings {
        offsets.push(str_off);
        str_off += s.len();
    }

    if str_off > u16::MAX as usize {
        return Vec::new();
    }

    let total_len = string_storage_offset + str_off;
    let mut buf = vec![0u8; total_len];
    write_u16_be(&mut buf, 0, 0);
    write_u16_be(&mut buf, 2, records.len() as u16);
    write_u16_be(&mut buf, 4, string_storage_offset as u16);

    for (i, (name_id, _)) in records.iter().enumerate() {
        let off = header_len + i * record_len;
        write_u16_be(&mut buf, off, 3);
        write_u16_be(&mut buf, off + 2, 1);
        write_u16_be(&mut buf, off + 4, 0x0409);
        write_u16_be(&mut buf, off + 6, *name_id);
        write_u16_be(&mut buf, off + 8, strings[i].len() as u16);
        write_u16_be(&mut buf, off + 10, offsets[i] as u16);
    }
    for (i, s) in strings.iter().enumerate() {
        let start = string_storage_offset + offsets[i];
        buf[start..start + s.len()].copy_from_slice(s);
    }
    buf
}
