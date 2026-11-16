use alloc::vec::Vec;
use super::decoder::{read_u16_be, read_uint, records_fit};
use crate::daecore::daetype::Tables;

// The scripts a reader takes: version 1.1's scriptList2 where it names any, read in its wide layout (its records'
// offsets and its scripts' extender and default offsets 32-bit, extender glyphs 24-bit); else 1.0's list.
pub(crate) struct ScriptList {
    pub(crate) records: usize,
    pub(crate) count: usize,
    pub(crate) wide: bool,
}

impl ScriptList {
    // None for a table of a major version but 1, the one the spec defines. A scriptList2 past the table is absent,
    // and the 1.0 list read in its place.
    pub(crate) fn read(jstf: &[u8]) -> Option<ScriptList> {
        if read_u16_be(jstf, 0)? != 1 { return None; }
        let count = usize::from(read_u16_be(jstf, 4)?);
        let list2 = 6 + 6 * count;
        let count2 = match read_u16_be(jstf, 2)? {
            0 => 0,
            _ => read_u16_be(jstf, list2).map_or(0, usize::from),
        };
        if count2 != 0 {
            return Some(ScriptList { records: list2 + 2, count: count2, wide: true });
        }
        Some(ScriptList { records: 6, count, wide: false })
    }

    pub(crate) fn stride(&self) -> usize { if self.wide { 8 } else { 6 } }
    pub(crate) fn offset_width(&self) -> usize { if self.wide { 4 } else { 2 } }
    pub(crate) fn glyph_width(&self) -> usize { if self.wide { 3 } else { 2 } }

    // Record `i`'s tag and its script's place in the table, None where its offset is NULL.
    pub(crate) fn record<'a>(&self, jstf: &'a [u8], i: usize) -> Option<(&'a [u8], usize)> {
        let rec = self.records + i * self.stride();
        let script = read_uint(jstf, rec + 4, self.offset_width()).filter(|&rel| rel != 0)?;
        Some((jstf.get(rec..rec + 4)?, (script as usize).min(jstf.len())))
    }
}

fn find_jstf_script_table(jstf: &[u8], script_tag: [u8; 4]) -> Option<(usize, ScriptList)> {
    let list = ScriptList::read(jstf)?;
    let script = (0..list.count).find_map(|i| list.record(jstf, i).filter(|&(tag, _)| tag == script_tag))?.1;
    Some((script, list))
}

pub fn jstf_extender_glyphs(table_map: &Tables, script_tag: [u8; 4]) -> Option<Vec<u32>> {
    let jstf = table_map.get("JSTF")?;
    let (script, list) = find_jstf_script_table(jstf, script_tag)?;
    let (o, w) = (list.offset_width(), list.glyph_width());
    let extenders = script.checked_add(read_uint(jstf, script, o).filter(|&rel| rel != 0)? as usize)?;
    let count = usize::from(read_u16_be(jstf, extenders)?);
    if !records_fit(extenders + 2, count, w, jstf.len()) { return None; }
    (0..count).map(|i| read_uint(jstf, extenders + 2 + i * w, w)).collect()
}

/// A JSTF priority level: the GSUB and GPOS lookups, by index, it enables and disables to shrink a line or to
/// extend one. Each list is none where the level names none, which isn't a list naming no lookups.
// Its JstfMax, limits on spacing a justifier adds itself, is not read: no shipping software applies it.
#[non_exhaustive]
#[derive(Debug, PartialEq, Default, Clone)]
pub struct JstfModLists {
    /// The GSUB lookups to enable to shrink a line.
    pub shrinkage_enable_gsub: Option<Vec<u16>>,
    /// The GSUB lookups to disable to shrink a line.
    pub shrinkage_disable_gsub: Option<Vec<u16>>,
    /// The GPOS lookups to enable to shrink a line.
    pub shrinkage_enable_gpos: Option<Vec<u16>>,
    /// The GPOS lookups to disable to shrink a line.
    pub shrinkage_disable_gpos: Option<Vec<u16>>,
    /// The GSUB lookups to enable to extend a line.
    pub extension_enable_gsub: Option<Vec<u16>>,
    /// The GSUB lookups to disable to extend a line.
    pub extension_disable_gsub: Option<Vec<u16>>,
    /// The GPOS lookups to enable to extend a line.
    pub extension_enable_gpos: Option<Vec<u16>>,
    /// The GPOS lookups to disable to extend a line.
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
    table_map: &Tables,
    script_tag: [u8; 4],
    lang_sys_tag: Option<[u8; 4]>,
) -> Option<Vec<JstfModLists>> {
    let jstf = table_map.get("JSTF")?;
    let (jstf_script_table, list) = find_jstf_script_table(jstf, script_tag)?;
    let o = list.offset_width();

    let def_lang_sys_off = read_uint(jstf, jstf_script_table + o, o)? as usize;
    let lang_sys_count   = read_u16_be(jstf, jstf_script_table + 2 * o)? as usize;

    // A language without its own record takes the script's default, which the spec says applies in
    // the absence of language-specific data.
    let language = lang_sys_tag.and_then(|tag| {
        (0..lang_sys_count)
            .map(|i| jstf_script_table + 2 * o + 2 + i * 6)
            .find(|&rec| jstf.get(rec..rec + 4) == Some(&tag[..]))
            .and_then(|rec| read_u16_be(jstf, rec + 4))
            .filter(|&off| off != 0)
    });
    let jstf_lang_sys_table = match language {
        Some(off) => jstf_script_table + usize::from(off),
        None if def_lang_sys_off != 0 => jstf_script_table.checked_add(def_lang_sys_off)?,
        None => return None,
    };

    let priority_count = read_u16_be(jstf, jstf_lang_sys_table)? as usize;
    if !records_fit(jstf_lang_sys_table + 2, priority_count, 2, jstf.len()) {
        return None;
    }
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
#[path = "../../tests/unit/daetype/jstf.rs"]
mod tests;
