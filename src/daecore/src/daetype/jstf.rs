use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::decoder::{read_u16_be, records_fit};
use crate::daecore::daetype::TableBytes;

fn find_jstf_script_table(jstf: &[u8], script_tag: &str) -> Option<usize> {
    if jstf.len() < 6 { return None; }
    let script_count = read_u16_be(jstf, 4)? as usize;
    for i in 0..script_count {
        let rec = 6 + i * 6;
        if jstf.get(rec..rec + 4)? != script_tag.as_bytes() { continue; }
        return Some(read_u16_be(jstf, rec + 4)? as usize);
    }
    None
}

pub fn jstf_extender_glyphs(table_map: &BTreeMap<String, TableBytes>, script_tag: &str) -> Option<Vec<u16>> {
    let jstf = table_map.get("JSTF")?;
    let jstf_script_table = find_jstf_script_table(jstf, script_tag)?;

    let extender_off = read_u16_be(jstf, jstf_script_table)? as usize;
    if extender_off == 0 { return None; }
    let extender_table = jstf_script_table + extender_off;

    let glyph_count = read_u16_be(jstf, extender_table)? as usize;
    if !records_fit(extender_table + 2, glyph_count, 2, jstf.len()) { return None; }
    let mut glyphs = Vec::with_capacity(glyph_count);
    for i in 0..glyph_count {
        glyphs.push(read_u16_be(jstf, extender_table + 2 + i * 2)?);
    }
    Some(glyphs)
}

// A priority level's GSUB and GPOS lookups to enable and disable. Its JstfMax, limits on spacing a
// justifier adds itself, is not read: no shipping software applies it.
#[derive(Debug, PartialEq)]
pub struct JstfModLists {
    pub shrinkage_enable_gsub:  Option<Vec<u16>>,
    pub shrinkage_disable_gsub: Option<Vec<u16>>,
    pub shrinkage_enable_gpos:  Option<Vec<u16>>,
    pub shrinkage_disable_gpos: Option<Vec<u16>>,
    pub extension_enable_gsub:  Option<Vec<u16>>,
    pub extension_disable_gsub: Option<Vec<u16>>,
    pub extension_enable_gpos:  Option<Vec<u16>>,
    pub extension_disable_gpos: Option<Vec<u16>>,
}

fn read_mod_list(
    jstf: &[u8],
    priority_table: usize,
    field_off: usize,
    indices_left: &mut usize,
) -> Result<Option<Vec<u16>>, ()> {
    let off = read_u16_be(jstf, priority_table + field_off).ok_or(())? as usize;
    if off == 0 { return Ok(None); }
    let list_table = priority_table + off;
    let count = read_u16_be(jstf, list_table).ok_or(())? as usize;
    if !records_fit(list_table + 2, count, 2, jstf.len()) { return Err(()); }
    *indices_left = indices_left.checked_sub(count).ok_or(())?;
    let mut indices = Vec::with_capacity(count);
    for i in 0..count {
        indices.push(read_u16_be(jstf, list_table + 2 + i * 2).ok_or(())?);
    }
    Ok(Some(indices))
}

fn read_jstf_priority(jstf: &[u8], priority_table: usize, indices_left: &mut usize) -> Option<JstfModLists> {
    Some(JstfModLists {
        shrinkage_enable_gsub:  read_mod_list(jstf, priority_table, 0, indices_left).ok()?,
        shrinkage_disable_gsub: read_mod_list(jstf, priority_table, 2, indices_left).ok()?,
        shrinkage_enable_gpos:  read_mod_list(jstf, priority_table, 4, indices_left).ok()?,
        shrinkage_disable_gpos: read_mod_list(jstf, priority_table, 6, indices_left).ok()?,
        extension_enable_gsub:  read_mod_list(jstf, priority_table, 10, indices_left).ok()?,
        extension_disable_gsub: read_mod_list(jstf, priority_table, 12, indices_left).ok()?,
        extension_enable_gpos:  read_mod_list(jstf, priority_table, 14, indices_left).ok()?,
        extension_disable_gpos: read_mod_list(jstf, priority_table, 16, indices_left).ok()?,
    })
}

pub fn jstf_priorities(
    table_map: &BTreeMap<String, TableBytes>,
    script_tag: &str,
    lang_sys_tag: Option<&str>,
) -> Option<Vec<JstfModLists>> {
    let jstf = table_map.get("JSTF")?;
    let jstf_script_table = find_jstf_script_table(jstf, script_tag)?;

    let def_lang_sys_off = read_u16_be(jstf, jstf_script_table + 2)? as usize;
    let lang_sys_count   = read_u16_be(jstf, jstf_script_table + 4)? as usize;

    // A language without its own record takes the script's default, which the spec says applies in
    // the absence of language-specific data.
    let language = lang_sys_tag.and_then(|tag| {
        (0..lang_sys_count)
            .map(|i| jstf_script_table + 6 + i * 6)
            .find(|&rec| jstf.get(rec..rec + 4) == Some(tag.as_bytes()))
            .and_then(|rec| read_u16_be(jstf, rec + 4))
            .filter(|&off| off != 0)
    });
    let jstf_lang_sys_table = match language {
        Some(off) => jstf_script_table + usize::from(off),
        None if def_lang_sys_off != 0 => jstf_script_table + def_lang_sys_off,
        None => return None,
    };

    let priority_count = read_u16_be(jstf, jstf_lang_sys_table)? as usize;
    if !records_fit(jstf_lang_sys_table + 2, priority_count, 2, jstf.len()) {
        return None;
    }
    // Each level tried costs a reshape of the line; the first are the highest priorities.
    const MAX_PRIORITY_LEVELS: usize = 64;
    let priority_count = priority_count.min(MAX_PRIORITY_LEVELS);
    let mut indices_left = jstf.len() / 2;

    let mut levels = Vec::with_capacity(priority_count);
    for i in 0..priority_count {
        let off = read_u16_be(jstf, jstf_lang_sys_table + 2 + i * 2)? as usize;
        if off == 0 { return None; }
        let priority_table = jstf_lang_sys_table + off;
        levels.push(read_jstf_priority(jstf, priority_table, &mut indices_left)?);
    }
    Some(levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A JSTF of the one script `tag`, with no language records and a default of `levels` priority
    // levels that all name one level enabling nothing.
    fn jstf(tag: &[u8; 4], levels: u16) -> BTreeMap<String, TableBytes> {
        let mut t: Vec<u8> = [1u16, 0, 1].iter().flat_map(|v| v.to_be_bytes()).collect();
        t.extend(tag);
        t.extend([12u16, 0, 6, 0, levels].iter().flat_map(|v| v.to_be_bytes()));
        (0..levels).for_each(|_| t.extend((2 + 2 * levels).to_be_bytes()));
        t.extend([0u8; 20]);
        [(String::from("JSTF"), TableBytes::from(t))].into_iter().collect()
    }

    // The spec's default applies in the absence of language data, so a language asked for by name that
    // has none takes the script's default levels.
    #[test]
    fn a_language_without_data_takes_the_scripts_default() {
        let map = jstf(b"latn", 1);
        assert_eq!(jstf_priorities(&map, "latn", Some("ENG ")).map(|l| l.len()), Some(1));
    }

    // Tags compare as bytes, so one that is not UTF-8 never matches the empty tag.
    #[test]
    fn an_empty_tag_names_no_script() {
        assert_eq!(jstf_priorities(&jstf(&[0xFF; 4], 1), "", None), None);
    }

    // Past 64 levels a font keeps its first 64, the highest priorities.
    #[test]
    fn past_64_levels_the_first_64_stay() {
        assert_eq!(jstf_priorities(&jstf(b"latn", 65), "latn", None).map(|l| l.len()), Some(64));
    }
}
