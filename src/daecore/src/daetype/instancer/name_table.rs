use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;
use super::super::decoder::{encode_name, is_english, lang_tags, name_records, write_name_table, NameKey};

pub(crate) fn rewrite_name_table(
    name: &[u8],
    updates: &[(u16, String)],
    removals: &[u16],
) -> Option<Vec<u8>> {
    if name.len() < 6 {
        return None;
    }
    let records: Vec<(NameKey, &[u8])> = name_records(name).collect();

    // A record in another language holds that language's name, which an English one cannot replace,
    // so an updated ID loses it. An ID with no English record goes to each English platform there is.
    let english = || records.iter().map(|&(key, _)| key).filter(|&key| is_english(key));
    let had: BTreeSet<u16> = english().map(|key| key.name_id).collect();
    let mut platforms: Vec<(u16, u16, u16)> = english().map(|key| (key.platform, key.encoding, key.language)).collect();
    platforms.sort_unstable();
    platforms.dedup();
    if platforms.is_empty() {
        platforms.push((3, 1, 0x0409));
    }

    // Each update encoded once in each encoding an English record can have, UTF-16 and Mac Roman, and borrowed by every
    // record it replaces, of which a table can hold 65,535.
    let encoded: Vec<[Option<Vec<u8>>; 2]> = updates
        .iter()
        .map(|(_, s)| [3, 1].map(|platform| encode_name(NameKey { platform, encoding: 0, language: 0, name_id: 0 }, s)))
        .collect();
    let update = |i: usize, key: NameKey| encoded[i][usize::from(key.platform == 1)].as_deref().map(|s| (key, s));

    let mut kept: Vec<(NameKey, &[u8])> = Vec::with_capacity(records.len());
    for &(key, string) in &records {
        if removals.contains(&key.name_id) {
            continue;
        }
        match updates.iter().position(|(id, _)| *id == key.name_id) {
            None => kept.push((key, string)),
            Some(i) if is_english(key) => kept.extend(update(i, key)),
            Some(_) => {}
        }
    }
    for (i, (name_id, _)) in updates.iter().enumerate().filter(|(_, (id, _))| !had.contains(id)) {
        for &(platform, encoding, language) in &platforms {
            kept.extend(update(i, NameKey { platform, encoding, language, name_id: *name_id }));
        }
    }
    write_name_table(kept, &lang_tags(name))
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/name_table.rs"]
mod tests;
