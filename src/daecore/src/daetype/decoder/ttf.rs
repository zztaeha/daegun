use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::io::{read_u16_be, read_u32_be, write_u16_be, write_u32_be};
use crate::daecore::daetype::TableBytes;
use crate::daecore::sync::Shared;

fn checksum32(data: &[u8]) -> u32 {
    let (chunks, rem) = data.as_chunks::<4>();
    let mut sum = chunks.iter().fold(0u32, |acc, c| {
        // `from_be_bytes`, not a shift-or chain, is what lets LLVM see one byte-swapped load and
        // vectorize the fold: 116.7 µs against 569.5 µs over an 8.4 MB font.
        acc.wrapping_add(u32::from_be_bytes([c[0], c[1], c[2], c[3]]))
    });
    if !rem.is_empty() {
        let mut last: u32 = 0;
        for (i, &b) in rem.iter().enumerate() {
            last |= (b as u32) << ((3 - i) * 8);
        }
        sum = sum.wrapping_add(last);
    }
    sum
}

pub fn pad4(n: usize) -> usize {
    (n + 3) & !3
}

pub fn build_ttf<V: AsRef<[u8]>>(table_map: &BTreeMap<String, V>) -> Vec<u8> {
    let tags: Vec<&String> = table_map.keys().collect();
    let num_tables = tags.len();
    if num_tables == 0 { return Vec::new(); }

    let floor_log2     = (usize::BITS - num_tables.leading_zeros() - 1) as usize;
    let search_range   = (1usize << floor_log2) * 16;
    let entry_selector = floor_log2;
    let range_shift    = num_tables * 16 - search_range;

    let sfnt_hdr  = 12;
    let dir_size  = num_tables * 16;
    let mut data_off = sfnt_hdr + dir_size;

    for &tag in &tags {
        data_off += pad4(table_map[tag].as_ref().len());
    }
    let total = data_off;

    let mut out: Vec<u8> = Vec::with_capacity(total);
    out.resize(sfnt_hdr + dir_size, 0);
    let mut tbl_offsets: Vec<usize> = Vec::with_capacity(num_tables);
    let mut tbl_checksums: Vec<u32> = Vec::with_capacity(num_tables);
    for &tag in &tags {
        let raw = table_map[tag].as_ref();
        tbl_offsets.push(out.len());
        // head's directory checksum reads checkSumAdjustment as zero: that field derives from it.
        tbl_checksums.push(if tag == "head" && raw.len() >= 12 {
            let mut zeroed = raw.to_vec();
            zeroed[8..12].fill(0);
            checksum32(&zeroed)
        } else {
            checksum32(raw)
        });
        out.extend_from_slice(raw);
        out.resize(pad4(out.len()), 0);
    }

    let cff = table_map.contains_key("CFF ") || table_map.contains_key("CFF2");
    let sfnt_version = if cff { 0x4F54_544F } else { 0x0001_0000 };
    write_u32_be(&mut out, 0, sfnt_version);
    write_u16_be(&mut out, 4,  num_tables as u16);
    write_u16_be(&mut out, 6,  search_range as u16);
    write_u16_be(&mut out, 8,  entry_selector as u16);
    write_u16_be(&mut out, 10, range_shift as u16);

    let mut dir_off = sfnt_hdr;
    for (i, &tag) in tags.iter().enumerate() {
        let cs = tbl_checksums[i];
        for (j, &b) in tag.as_bytes().iter().take(4).enumerate() {
            out[dir_off + j] = b;
        }
        write_u32_be(&mut out, dir_off + 4,  cs);
        write_u32_be(&mut out, dir_off + 8,  tbl_offsets[i] as u32);
        write_u32_be(&mut out, dir_off + 12, table_map[tag].as_ref().len() as u32);
        dir_off += 16;
    }

    if let Some(pos) = tags.iter().position(|t| t.as_str() == "head") {
        let head_off = tbl_offsets[pos];
        let head_len = table_map[tags[pos]].as_ref().len();
        if head_len >= 12 && head_off + 12 <= out.len() {
            write_u32_be(&mut out, head_off + 8, 0);
            let header_cs = checksum32(&out[..sfnt_hdr + dir_size]);
            let file_cs = tbl_checksums.iter().fold(header_cs, |a, &c| a.wrapping_add(c));
            write_u32_be(&mut out, head_off + 8, 0xB1B0_AFBA_u32.wrapping_sub(file_cs));
        }
    }

    out
}

pub fn extract_ttf_tables(data: &[u8]) -> Result<BTreeMap<String, TableBytes>, String> {
    extract_ttf_tables_owned_at(Shared::new(data.to_vec()), 0)
}

pub fn extract_ttf_tables_owned(data: Vec<u8>) -> Result<BTreeMap<String, TableBytes>, String> {
    extract_ttf_tables_owned_at(Shared::new(data), 0)
}

// Tables read from a buffer already shared, such as a cached instance, without copying it.
pub(crate) fn extract_ttf_tables_shared(data: Shared<Vec<u8>>) -> Result<BTreeMap<String, TableBytes>, String> {
    extract_ttf_tables_owned_at(data, 0)
}

pub fn extract_ttc_tables(data: &[u8], index: usize) -> Result<BTreeMap<String, TableBytes>, String> {
    if read_u32_be(data, 0) != Some(0x7474_6366) {
        return Err("Not a TTC file".into());
    }
    let num_fonts = read_u32_be(data, 8).ok_or("TTC: header truncated")? as usize;
    if index >= num_fonts {
        return Err(format!("TTC: font index {} out of range ({} fonts)", index, num_fonts));
    }
    let entry = index.checked_mul(4).and_then(|i| i.checked_add(12)).ok_or("TTC: offset table truncated")?;
    let dir_off = read_u32_be(data, entry).ok_or("TTC: offset table truncated")? as usize;
    // A face holds its own tables, copied into one buffer, rather than the whole collection.
    let entries = directory(data, dir_off, |off, len| Some((off, len)))?;
    let mut compact = Vec::with_capacity(entries.values().map(|&(_, len)| len).sum());
    let mut placed: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    let starts: Vec<usize> = entries.values().map(|&(off, len)| *placed.entry((off, len)).or_insert_with(|| {
        compact.extend_from_slice(&data[off..off + len]);
        compact.len() - len
    })).collect();
    let buf = Shared::new(compact);
    entries.into_iter().zip(starts)
        .map(|((tag, (_, len)), start)| Ok((tag, TableBytes::slice(&buf, start, len).ok_or("TTC: a table out of bounds")?)))
        .collect()
}

// The faces whose offsets the file holds, whatever count its header states.
pub fn ttc_font_count(data: &[u8]) -> usize {
    if read_u32_be(data, 0) != Some(0x7474_6366) { return 0; }
    let stated = read_u32_be(data, 8).unwrap_or(0) as usize;
    stated.min(data.len().saturating_sub(12) / 4)
}

fn extract_ttf_tables_owned_at(
    buf: Shared<Vec<u8>>,
    dir_off: usize,
) -> Result<BTreeMap<String, TableBytes>, String> {
    directory(&buf, dir_off, |off, len| TableBytes::slice(&buf, off, len))
}

// The tables a directory names, each made by `table` from its offset and length as it is read, so
// the map is built once.
fn directory<T>(data: &[u8], dir_off: usize, mut table: impl FnMut(usize, usize) -> Option<T>) -> Result<BTreeMap<String, T>, String> {
    if dir_off.checked_add(12).is_none_or(|end| data.len() < end) {
        return Err("TTF/OTF: file too short".into());
    }
    let sfversion = read_u32_be(data, dir_off).ok_or("TTF/OTF: header truncated")?;
    if sfversion != 0x0001_0000 && sfversion != 0x4F54_544F && sfversion != 0x7472_7565 {
        return Err(format!("Not a TTF/OTF file (signature: 0x{:08X})", sfversion));
    }
    let num_tables = read_u16_be(data, dir_off + 4).ok_or("TTF/OTF: header truncated")? as usize;
    if (dir_off + 12).checked_add(num_tables * 16).is_none_or(|end| data.len() < end) {
        return Err("TTF/OTF: table directory truncated".into());
    }
    const MAX_EXTRACT_RATIO: usize = 4;
    const MAX_EXTRACT_FLOOR: usize = 64 * 1024;
    let extract_ceiling = data.len().saturating_mul(MAX_EXTRACT_RATIO).max(MAX_EXTRACT_FLOOR);
    let mut extracted = 0usize;

    // Read as FreeType reads it: a tag not printable or starting past the file is skipped, one running
    // past it too unless hmtx or vmtx (clipped), and of a repeated tag the first non-empty entry counts.
    let mut map = BTreeMap::new();
    for i in 0..num_tables {
        let e = dir_off + 12 + i * 16;
        let Some(tag_bytes) = data.get(e..e + 4) else { continue };
        if !tag_bytes.iter().all(|b| (0x20..=0x7E).contains(b)) {
            continue;
        }
        let tag = core::str::from_utf8(tag_bytes).unwrap_or_default().to_string();
        let (Some(offset), Some(length)) = (read_u32_be(data, e + 8), read_u32_be(data, e + 12)) else { continue };
        let (offset, mut length) = (offset as usize, length as usize);
        if offset > data.len() {
            continue;
        }
        let alloc::collections::btree_map::Entry::Vacant(slot) = map.entry(tag) else { continue };
        if length > data.len() - offset {
            if !matches!(slot.key().as_str(), "hmtx" | "vmtx") {
                continue;
            }
            length = data.len() - offset;
        }
        // A zero-length table is a missing one, as FreeType has it, but for glyf: there it is a font
        // whose glyphs are all empty.
        if length == 0 && slot.key() != "glyf" {
            continue;
        }
        extracted = extracted.saturating_add(length);
        if extracted > extract_ceiling {
            return Err(format!(
                "TTF/OTF: table directory extracts {} bytes from a {}-byte file, past the {}x ceiling: \
                 entries overlap",
                extracted,
                data.len(),
                MAX_EXTRACT_RATIO,
            ));
        }
        let value = table(offset, length).ok_or_else(|| format!("TTF/OTF: table '{}' out of bounds", slot.key()))?;
        slot.insert(value);
    }

    // A font carrying both outline formats is malformed, and sfntVersion breaks the tie:
    // 0x00010000 and 'true' mean TrueType, 'OTTO' means CFF.
    if map.contains_key("glyf") && (map.contains_key("CFF ") || map.contains_key("CFF2")) {
        if sfversion == 0x4F54_544F {
            map.remove("glyf");
            map.remove("loca");
        } else {
            map.remove("CFF ");
            map.remove("CFF2");
        }
    }

    Ok(map)
}
