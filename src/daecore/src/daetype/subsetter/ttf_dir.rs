use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{read_u16_be, read_u32_be};
use crate::daecore::daetype::TableBytes;

pub struct TtfEntry { pub offset: usize, pub length: usize }

pub fn parse_ttf_dir(ttf: &[u8]) -> BTreeMap<String, TtfEntry> {
    let mut map = BTreeMap::new();
    if ttf.len() < 12 { return map; }
    let n = match read_u16_be(ttf, 4) { Some(v) => v as usize, None => return map };
    for i in 0..n {
        let d = 12 + i * 16;
        if d + 16 > ttf.len() { break; }
        let tag = core::str::from_utf8(&ttf[d..d + 4])
            .unwrap_or("    ")
            .trim_end_matches('\0')
            .to_string();
        let (Some(offset), Some(length)) = (read_u32_be(ttf, d + 8), read_u32_be(ttf, d + 12)) else { break };
        map.insert(tag, TtfEntry { offset: offset as usize, length: length as usize });
    }
    map
}

pub fn slice_table<'a>(ttf: &'a [u8], map: &BTreeMap<String, TtfEntry>, tag: &str) -> Option<&'a [u8]> {
    map.get(tag).and_then(|e| {
        let end = e.offset.checked_add(e.length)?;
        ttf.get(e.offset..end)
    })
}

pub fn owned_table(ttf: &[u8], map: &BTreeMap<String, TtfEntry>, tag: &str) -> Option<Vec<u8>> {
    slice_table(ttf, map, tag).map(|s| s.to_vec())
}

// The metrics table and its count of full entries, or None where the font has none that way.
pub fn metrics_of<'a>(
    map: &'a BTreeMap<String, TableBytes>,
    mtx_tag: &str,
    hea_tag: &str,
) -> Option<(&'a [u8], usize)> {
    let mtx = map.get(mtx_tag)?.as_slice();
    let hea = map.get(hea_tag)?.as_slice();
    if hea.len() < 36 { return None; }
    Some((mtx, read_u16_be(hea, 34)? as usize))
}

pub fn map_advances_all(
    map: &BTreeMap<String, TableBytes>,
    mtx_tag: &str,
    hea_tag: &str,
    normalize: bool,
) -> Vec<u32> {
    let get = |tag: &str| map.get(tag).map(|t| t.as_slice());
    let num_glyphs = get("maxp").and_then(|m| read_u16_be(m, 4)).map_or(0, |v| v as usize);
    let upm = get("head")
        .filter(|h| h.len() >= 20)
        .and_then(|h| read_u16_be(h, 18))
        .filter(|&v| v > 0);
    let resolved = metrics_of(map, mtx_tag, hea_tag).zip(upm).map(|((mtx, n), upm)| (mtx, n, upm));
    let Some((mtx, num_metrics, upm)) = resolved else {
        let default = match upm {
            Some(u) if !mtx_tag.starts_with('v') => {
                let (em, upm) = (u64::from(u) / 2, u64::from(u));
                if normalize { ((em * 1000 + upm / 2) / upm) as u32 } else { em as u32 }
            }
            _ => 0,
        };
        return vec![default; num_glyphs];
    };
    (0..num_glyphs)
        .map(|gid| {
            let aw_off = if gid < num_metrics { gid * 4 } else { (num_metrics.max(1) - 1) * 4 };
            advance_at(mtx, aw_off, upm, normalize)
        })
        .collect()
}

fn advance_at(mtx: &[u8], aw_off: usize, upm: u16, normalize: bool) -> u32 {
    if aw_off + 2 > mtx.len() { return 0; }
    let aw = match read_u16_be(mtx, aw_off) { Some(v) => v as u64, None => return 0 };
    if !normalize { return aw as u32; }
    ((aw * 1000 + upm as u64 / 2) / upm as u64) as u32
}
