use alloc::string::String;
use alloc::string::ToString;
use alloc::collections::{BTreeMap, BTreeSet};
use super::decoder::{read_u16_be, read_i16_be, read_u32_be, records_fit};
use super::format::ivs::DeviceDeltas;
use crate::daecore::daetype::TableBytes;

#[derive(Debug, PartialEq)]
pub struct BaseScriptInfo {
    pub default_baseline_tag: Option<String>,
    pub baseline_coords:      BTreeMap<String, f64>,
}

// A script's BaseScript on one axis, and the axis table: DFLT's when the script has none, as HarfBuzz
// falls back.
fn base_script(base: &[u8], vertical: bool, script_tag: &str) -> Option<(usize, usize)> {
    let axis = usize::from(read_u16_be(base, if vertical { 6 } else { 4 })?);
    if axis == 0 { return None; }
    let list_rel = usize::from(read_u16_be(base, axis + 2)?);
    if list_rel == 0 { return None; }
    let list = axis + list_rel;
    let count = usize::from(read_u16_be(base, list)?);
    if !records_fit(list + 2, count, 6, base.len()) { return None; }
    let script = |tag: &[u8]| {
        (0..count)
            .map(|i| list + 2 + i * 6)
            .find(|&rec| base.get(rec..rec + 4) == Some(tag))
            .and_then(|rec| read_u16_be(base, rec + 4))
            .filter(|&rel| rel != 0)
    };
    let rel = script(script_tag.as_bytes()).or_else(|| script(b"DFLT"))?;
    Some((axis, list + usize::from(rel)))
}

// BASE 1.1's ItemVariationStore at a location in normalized coordinates.
fn deltas(base: &[u8], location: &[f64]) -> Option<DeviceDeltas> {
    if read_u16_be(base, 2)? < 1 { return None; }
    DeviceDeltas::new(base, read_u32_be(base, 8)? as usize, location)
}

// A BaseCoord in design units: formats 1 and 2 as stored, 2's contour point refining it only at a
// hinted size, and 3 moved by its VariationIndex.
fn coordinate(base: &[u8], at: usize, deltas: &mut Option<DeviceDeltas>) -> Option<f64> {
    let value = f64::from(read_i16_be(base, at + 2)?);
    match read_u16_be(base, at)? {
        1 | 2 => Some(value),
        3 => {
            let device = usize::from(read_u16_be(base, at + 4)?);
            let delta = match deltas { Some(d) if device != 0 => d.delta(base, at + device), _ => 0.0 };
            Some(value + delta)
        }
        _ => None,
    }
}

// A script's baselines on one axis, in design units at a location in normalized coordinates.
pub fn base_script_info(
    table_map: &BTreeMap<String, TableBytes>, script_tag: &str, vertical: bool, location: &[f64],
) -> Option<BaseScriptInfo> {
    let base = table_map.get("BASE")?;
    let (axis, script) = base_script(base, vertical, script_tag)?;

    let tags_rel = usize::from(read_u16_be(base, axis)?);
    if tags_rel == 0 { return None; }
    let tag_list = axis + tags_rel;
    let tag_count = usize::from(read_u16_be(base, tag_list)?);
    if !records_fit(tag_list + 2, tag_count, 4, base.len()) { return None; }
    let tag = |i: usize| {
        let at = tag_list + 2 + i * 4;
        base.get(at..at + 4).map(|t| String::from_utf8_lossy(t).to_string())
    };

    let values_rel = usize::from(read_u16_be(base, script)?);
    if values_rel == 0 { return None; }
    let values = script + values_rel;
    let default_index = usize::from(read_u16_be(base, values)?);
    let coord_count = usize::from(read_u16_be(base, values + 2)?);

    let mut deltas = deltas(base, location);
    let mut baseline_coords = BTreeMap::new();
    for i in 0..coord_count.min(tag_count) {
        let rel = usize::from(read_u16_be(base, values + 4 + i * 2)?);
        if rel == 0 { continue; }
        if let Some(c) = coordinate(base, values + rel, &mut deltas) {
            baseline_coords.insert(tag(i)?, c);
        }
    }
    let default_baseline_tag = if default_index < tag_count { tag(default_index) } else { None };
    Some(BaseScriptInfo { default_baseline_tag, baseline_coords })
}

// A script's extent on one axis in design units at a location: the language's MinMax, else the
// default, narrowed to a feature's record if any; None for a side the font leaves out.
pub fn base_min_max(
    table_map: &BTreeMap<String, TableBytes>, script_tag: &str, language: Option<&str>,
    feature: Option<&str>, vertical: bool, location: &[f64],
) -> Option<(Option<f64>, Option<f64>)> {
    let base = table_map.get("BASE")?;
    let (_, script) = base_script(base, vertical, script_tag)?;

    let lang_count = usize::from(read_u16_be(base, script + 4)?);
    if !records_fit(script + 6, lang_count, 6, base.len()) { return None; }
    let lang = language.and_then(|tag| {
        (0..lang_count).map(|i| script + 6 + i * 6).find(|&rec| base.get(rec..rec + 4) == Some(tag.as_bytes()))
    });
    let rel = match lang {
        Some(rec) => read_u16_be(base, rec + 4)?,
        None => read_u16_be(base, script + 2)?,
    };
    if rel == 0 { return None; }
    let min_max = script + usize::from(rel);

    let feat_count = usize::from(read_u16_be(base, min_max + 4)?);
    if !records_fit(min_max + 6, feat_count, 8, base.len()) { return None; }
    let feat = feature.and_then(|tag| {
        (0..feat_count).map(|i| min_max + 6 + i * 8).find(|&rec| base.get(rec..rec + 4) == Some(tag.as_bytes()))
    });
    let [min_field, max_field] = match feat {
        Some(rec) => [rec + 4, rec + 6],
        None => [min_max, min_max + 2],
    };
    let mut deltas = deltas(base, location);
    let mut side = |field: usize| {
        let rel = usize::from(read_u16_be(base, field)?);
        if rel == 0 { return None; }
        coordinate(base, min_max + rel, &mut deltas)
    };
    let min = side(min_field);
    Some((min, side(max_field)))
}

// Whether no BaseCoord names a glyph, as format 2 does by ID, so BASE stays valid however glyphs are
// renumbered. A BaseScript or MinMax that many records share is walked once.
pub fn base_is_glyph_free(base: &[u8]) -> bool {
    let names_no_glyph = |at: usize| Some(matches!(read_u16_be(base, at)?, 1 | 3));
    let min_max_ok = |at: usize| -> Option<bool> {
        for slot in [0usize, 2] {
            let rel = read_u16_be(base, at + slot)?;
            if rel != 0 && !names_no_glyph(at + rel as usize)? { return Some(false); }
        }
        let count = read_u16_be(base, at + 4)? as usize;
        if !records_fit(at + 6, count, 8, base.len()) { return None; }
        for i in 0..count {
            for slot in [4usize, 6] {
                let rel = read_u16_be(base, at + 6 + i * 8 + slot)?;
                if rel != 0 && !names_no_glyph(at + rel as usize)? { return Some(false); }
            }
        }
        Some(true)
    };

    let mut scripts = BTreeSet::new();
    let mut min_maxes = BTreeSet::new();
    let mut walk = || -> Option<bool> {
        for axis_slot in [4usize, 6] {
            let axis = read_u16_be(base, axis_slot)? as usize;
            if axis == 0 { continue; }
            let list_rel = read_u16_be(base, axis + 2)?;
            if list_rel == 0 { continue; }
            let list = axis + list_rel as usize;

            let count = read_u16_be(base, list)? as usize;
            if !records_fit(list + 2, count, 6, base.len()) { return None; }
            for i in 0..count {
                let script = list + read_u16_be(base, list + 2 + i * 6 + 4)? as usize;
                if !scripts.insert(script) { continue; }

                let values_rel = read_u16_be(base, script)?;
                if values_rel != 0 {
                    let values = script + values_rel as usize;
                    let n = read_u16_be(base, values + 2)? as usize;
                    if !records_fit(values + 4, n, 2, base.len()) { return None; }
                    for k in 0..n {
                        let c = read_u16_be(base, values + 4 + k * 2)?;
                        if c != 0 && !names_no_glyph(values + c as usize)? { return Some(false); }
                    }
                }

                let lang_count = read_u16_be(base, script + 4)? as usize;
                if !records_fit(script + 6, lang_count, 6, base.len()) { return None; }
                let langs = (0..lang_count).map(|k| read_u16_be(base, script + 6 + k * 6 + 4));
                for rel in core::iter::once(read_u16_be(base, script + 2)).chain(langs) {
                    let min_max = script + rel? as usize;
                    if min_max != script && min_maxes.insert(min_max) && !min_max_ok(min_max)? {
                        return Some(false);
                    }
                }
            }
        }
        Some(true)
    };
    walk().unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn words(w: &[u16]) -> Vec<u8> {
        w.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    // A horizontal BASE of the one baseline tag 'romn', each script record naming the BaseScript `body`.
    fn base(scripts: &[&[u8; 4]], body: &[u8]) -> BTreeMap<String, TableBytes> {
        let mut t = words(&[1, 0, 8, 0, 4, 10, 1, 0x726F, 0x6D6E]);
        t.extend(words(&[scripts.len() as u16]));
        for tag in scripts {
            t.extend(*tag);
            t.extend(words(&[2 + 6 * scripts.len() as u16]));
        }
        t.extend(body);
        [(String::from("BASE"), TableBytes::from(t))].into_iter().collect()
    }

    // A BaseScript: baseline -120 in coordinate `format` (2 names glyph 5); default MinMax -200 to 800,
    // 'kern' -250 to 850, 'liga' -300 to 900; an 'ENG ' MinMax with only a minimum, -210.
    fn script(format: u16) -> Vec<u8> {
        let mut s = words(&[12, 26, 1, 0x454E, 0x4720, 72]);
        s.extend(words(&[0, 1, 6, format, (-120i16) as u16, 5, 0]));
        s.extend(words(&[22, 26, 2, 0x6B65, 0x726E, 30, 34, 0x6C69, 0x6761, 38, 42]));
        for v in [-200i16, 800, -250, 850, -300, 900] {
            s.extend(words(&[1, v as u16]));
        }
        s.extend(words(&[6, 0, 0, 1, (-210i16) as u16]));
        s
    }

    fn table(map: &BTreeMap<String, TableBytes>) -> &[u8] {
        map.get("BASE").expect("a BASE")
    }

    // FeatMinMax records are eight bytes; read six apart, the second would come from the first's bytes
    // and a glyph-free table read as naming glyphs.
    #[test]
    fn feature_extents_are_read_eight_bytes_apart() {
        let map = base(&[b"latn"], &script(1));
        assert!(base_is_glyph_free(table(&map)));
        let extents = |lang, feat| base_min_max(&map, "latn", lang, feat, false, &[]);
        assert_eq!(extents(None, None), Some((Some(-200.0), Some(800.0))));
        assert_eq!(extents(None, Some("kern")), Some((Some(-250.0), Some(850.0))));
        assert_eq!(extents(None, Some("liga")), Some((Some(-300.0), Some(900.0))));
        assert_eq!(extents(None, Some("smcp")), Some((Some(-200.0), Some(800.0))));
        assert_eq!(extents(Some("ENG "), None), Some((Some(-210.0), None)));
        assert_eq!(extents(Some("TRK "), None), Some((Some(-200.0), Some(800.0))));
    }

    // Format 2 gives its coordinate, and names a glyph by ID.
    #[test]
    fn a_coordinate_naming_a_glyph_is_read_and_found() {
        let map = base(&[b"latn"], &script(2));
        assert!(!base_is_glyph_free(table(&map)));
        let info = base_script_info(&map, "latn", false, &[]).expect("latn's baselines");
        assert_eq!(info.baseline_coords.get("romn"), Some(&-120.0));
    }

    #[test]
    fn dflt_stands_in_for_a_script_without_baselines() {
        let map = base(&[b"DFLT"], &script(1));
        assert_eq!(base_script_info(&map, "latn", false, &[]).map(|i| i.baseline_coords.len()), Some(1));
        assert!(base_min_max(&map, "latn", None, None, false, &[]).is_some());
    }

    // Tags compare as bytes, so one that is not UTF-8 never matches the empty tag.
    #[test]
    fn an_empty_tag_names_no_script() {
        let map = base(&[&[0xFF; 4]], &script(1));
        assert_eq!(base_script_info(&map, "", false, &[]), None);
    }

    // 32 script records naming one BaseScript, its 32 language records naming one MinMax of 65,535
    // feature records: walked per naming, 67 million records, walked once, 65,535.
    #[test]
    fn a_shared_script_and_min_max_are_walked_once() {
        let mut body = words(&[0, 6 + 6 * 32, 32]);
        (0..32).for_each(|_| body.extend(words(&[0x4142, 0x4344, 6 + 6 * 32])));
        body.extend(words(&[0, 0, u16::MAX]));
        body.extend(alloc::vec![0u8; 8 * usize::from(u16::MAX)]);
        let map = base(&[b"latn"; 32], &body);
        let started = std::time::Instant::now();
        assert!(base_is_glyph_free(table(&map)));
        assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
    }
}
