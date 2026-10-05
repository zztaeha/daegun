use alloc::string::String;
use alloc::vec::Vec;
use super::super::decoder::{mac_roman_byte, read_u16_be, write_u16_be};

struct NameRecord {
    platform: u16,
    encoding: u16,
    language: u16,
    name_id: u16,
    string: Vec<u8>,
}

fn encode_for(platform: u16, encoding: u16, s: &str) -> Option<Vec<u8>> {
    match (platform, encoding) {
        (0 | 3, _) => Some(s.encode_utf16().flat_map(u16::to_be_bytes).collect()),
        (1, 0) => s.chars().map(|c| mac_roman_byte(c as u32)).collect(),
        _ => None,
    }
}

// The records the new English strings belong in: Unicode, Windows English and Mac Roman English.
fn english(platform: u16, encoding: u16, language: u16) -> bool {
    match platform {
        0 => true,
        1 => encoding == 0 && language == 0,
        3 => language & 0x3FF == 0x009,
        _ => false,
    }
}

fn parse_records(name: &[u8]) -> Option<(Vec<NameRecord>, Vec<Vec<u8>>)> {
    let count = read_u16_be(name, 2)? as usize;
    let storage = read_u16_be(name, 4)? as usize;
    if name.len() < 6 + count * 12 {
        return None;
    }

    let mut records = Vec::with_capacity(count);
    for i in 0..count {
        let rec = 6 + i * 12;
        let platform = read_u16_be(name, rec)?;
        let encoding = read_u16_be(name, rec + 2)?;
        let language = read_u16_be(name, rec + 4)?;
        let name_id = read_u16_be(name, rec + 6)?;
        let length = read_u16_be(name, rec + 8)? as usize;
        let offset = read_u16_be(name, rec + 10)? as usize;
        let Some(string) = name.get(storage + offset..).and_then(|s| s.get(..length)) else { continue };
        records.push(NameRecord { platform, encoding, language, name_id, string: string.to_vec() });
    }

    let mut lang_tags = Vec::new();
    if read_u16_be(name, 0) == Some(1) {
        let at = 6 + count * 12;
        let lang_count = read_u16_be(name, at)? as usize;
        for i in 0..lang_count {
            let rec = at + 2 + i * 4;
            let length = read_u16_be(name, rec)? as usize;
            let offset = read_u16_be(name, rec + 2)? as usize;
            lang_tags.push(name.get(storage + offset..).and_then(|s| s.get(..length))?.to_vec());
        }
    }

    Some((records, lang_tags))
}

pub(crate) fn rewrite_name_table(
    name: &[u8],
    updates: &[(u16, String)],
    removals: &[u16],
) -> Option<Vec<u8>> {
    if name.len() < 6 {
        return None;
    }
    let (records, lang_tags) = parse_records(name)?;

    // A record in another language holds that language's name, which an English one cannot replace,
    // so an updated ID loses it. An ID with no English record goes to each English platform there is.
    let mut keys: Vec<(u16, u16, u16)> = records
        .iter()
        .filter(|r| english(r.platform, r.encoding, r.language))
        .map(|r| (r.platform, r.encoding, r.language))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    if keys.is_empty() {
        keys.push((3, 1, 0x0409));
    }

    let had: Vec<u16> = records.iter().filter(|r| english(r.platform, r.encoding, r.language)).map(|r| r.name_id).collect();
    let records_had = |id: u16| had.contains(&id);
    let mut out_records: Vec<NameRecord> = Vec::with_capacity(records.len() + updates.len() * keys.len());
    for record in records {
        if removals.contains(&record.name_id) {
            continue;
        }
        match updates.iter().find(|(id, _)| *id == record.name_id) {
            None => out_records.push(record),
            Some(_) if !english(record.platform, record.encoding, record.language) => {}
            Some((_, replacement)) => {
                if let Some(string) = encode_for(record.platform, record.encoding, replacement) {
                    out_records.push(NameRecord { string, ..record });
                }
            }
        }
    }

    for (name_id, string) in updates {
        if records_had(*name_id) {
            continue;
        }
        for &(platform, encoding, language) in &keys {
            if let Some(string) = encode_for(platform, encoding, string) {
                out_records.push(NameRecord { platform, encoding, language, name_id: *name_id, string });
            }
        }
    }

    out_records.sort_by_key(|r| (r.platform, r.encoding, r.language, r.name_id));

    let version: u16 = if lang_tags.is_empty() { 0 } else { 1 };
    let lang_tag_block = if lang_tags.is_empty() { 0 } else { 2 + lang_tags.len() * 4 };
    let header_len = 6 + out_records.len() * 12 + lang_tag_block;

    let mut out = vec![0u8; header_len];
    write_u16_be(&mut out, 0, version);
    write_u16_be(&mut out, 2, u16::try_from(out_records.len()).ok()?);
    write_u16_be(&mut out, 4, u16::try_from(header_len).ok()?);

    let mut storage: Vec<u8> = Vec::new();
    if !lang_tags.is_empty() {
        let dst = 6 + out_records.len() * 12;
        write_u16_be(&mut out, dst, u16::try_from(lang_tags.len()).ok()?);
        for (i, tag) in lang_tags.iter().enumerate() {
            write_u16_be(&mut out, dst + 2 + i * 4, u16::try_from(tag.len()).ok()?);
            write_u16_be(&mut out, dst + 2 + i * 4 + 2, u16::try_from(storage.len()).ok()?);
            storage.extend_from_slice(tag);
        }
    }

    for (i, record) in out_records.iter().enumerate() {
        let rec = 6 + i * 12;
        write_u16_be(&mut out, rec, record.platform);
        write_u16_be(&mut out, rec + 2, record.encoding);
        write_u16_be(&mut out, rec + 4, record.language);
        write_u16_be(&mut out, rec + 6, record.name_id);
        write_u16_be(&mut out, rec + 8, u16::try_from(record.string.len()).ok()?);
        write_u16_be(&mut out, rec + 10, u16::try_from(storage.len()).ok()?);
        storage.extend_from_slice(&record.string);
    }

    out.extend_from_slice(&storage);
    Some(out)
}
