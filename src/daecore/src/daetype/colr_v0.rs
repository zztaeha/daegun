use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::decoder::{read_u16_be, read_u32_be, records_fit, search_records};
use crate::daecore::daetype::TableBytes;

pub type ColrLayer = (u16, u8, u8, u8, u8, bool);

fn colr_base_glyph_layers(colr: &[u8], gid: u16) -> Option<(usize, usize, usize)> {
    let n_base     = read_u16_be(colr, 2)? as usize;
    let base_off   = read_u32_be(colr, 4)? as usize;
    let layers_off = read_u32_be(colr, 8)? as usize;

    let at = |i: usize| base_off.checked_add(i * 6);
    let hit = search_records(n_base, gid as u32, |i| read_u16_be(colr, at(i)?).map(u32::from))?.ok()?;
    let rec = at(hit)?;
    let first_layer = read_u16_be(colr, rec + 2)? as usize;
    let n_layers    = read_u16_be(colr, rec + 4)? as usize;
    Some((first_layer, n_layers, layers_off))
}

pub(crate) fn colr_v0_header(colr: &[u8]) -> Option<(usize, usize, usize, usize)> {
    let n_base     = read_u16_be(colr, 2)? as usize;
    let base_off   = read_u32_be(colr, 4)? as usize;
    let layers_off = read_u32_be(colr, 8)? as usize;
    let n_layers   = read_u16_be(colr, 12)? as usize;
    Some((n_base, base_off, layers_off, n_layers))
}

pub(crate) fn colr_v0_base_glyphs(colr: &[u8]) -> Vec<(u16, usize, usize)> {
    let Some((n_base, base_off, _, _)) = colr_v0_header(colr) else { return vec![] };
    if !records_fit(base_off, n_base, 6, colr.len()) { return vec![] }
    let mut out = Vec::with_capacity(n_base);
    for i in 0..n_base {
        let rec = base_off + i * 6;
        let (Some(gid), Some(first), Some(n)) = (
            read_u16_be(colr, rec), read_u16_be(colr, rec + 2), read_u16_be(colr, rec + 4),
        ) else { break };
        out.push((gid, first as usize, n as usize));
    }
    out
}

pub fn cpal_palette_count(table_map: &BTreeMap<String, TableBytes>) -> u16 {
    table_map.get("CPAL").and_then(|cpal| read_u16_be(cpal, 4)).unwrap_or(0)
}

#[derive(Debug, PartialEq)]
pub struct PaletteInfo {
    pub index:      u16,
    pub light_safe: bool,
    pub dark_safe:  bool,
    pub name_id:    Option<u16>,
}

pub fn cpal_palette_info(table_map: &BTreeMap<String, TableBytes>) -> Vec<PaletteInfo> {
    let cpal = match table_map.get("CPAL") { Some(c) => c, None => return vec![] };
    let version      = read_u16_be(cpal, 0).unwrap_or(0);
    let num_palettes = match read_u16_be(cpal, 4) { Some(v) => v, None => return vec![] };

    if version == 0 {
        return (0..num_palettes)
            .map(|i| PaletteInfo { index: i, light_safe: false, dark_safe: false, name_id: None })
            .collect();
    }

    let v1_off     = 12 + num_palettes as usize * 2;
    let types_off  = read_u32_be(cpal, v1_off).map(|v| v as usize);
    let labels_off = read_u32_be(cpal, v1_off + 4).map(|v| v as usize);

    (0..num_palettes).map(|i| {
        let (light_safe, dark_safe) = match types_off {
            Some(off) if off != 0 => {
                let flags = read_u32_be(cpal, off + i as usize * 4).unwrap_or(0);
                (flags & 0x0001 != 0, flags & 0x0002 != 0)
            }
            _ => (false, false),
        };
        let name_id = match labels_off {
            Some(off) if off != 0 => read_u16_be(cpal, off + i as usize * 2).filter(|&v| v != 0xFFFF),
            _ => None,
        };
        PaletteInfo { index: i, light_safe, dark_safe, name_id }
    }).collect()
}

// Each palette entry's name ID from a version 1 CPAL, for a picker to label the colors a palette
// sets; None for an entry without one, and nothing for a version 0 table or one with no labels.
pub fn cpal_palette_entry_labels(table_map: &BTreeMap<String, TableBytes>) -> Vec<Option<u16>> {
    let Some(cpal) = table_map.get("CPAL") else { return vec![] };
    let (Some(1), Some(n_entries), Some(num_palettes)) = (read_u16_be(cpal, 0), read_u16_be(cpal, 2), read_u16_be(cpal, 4)) else {
        return vec![];
    };
    let at = 12 + usize::from(num_palettes) * 2 + 8;
    let Some(labels) = read_u32_be(cpal, at).filter(|&v| v != 0).map(|v| v as usize) else { return vec![] };
    if !records_fit(labels, usize::from(n_entries), 2, cpal.len()) { return vec![] }
    (0..usize::from(n_entries)).map(|i| read_u16_be(cpal, labels + i * 2).filter(|&v| v != 0xFFFF)).collect()
}

#[derive(Clone, Copy)]
pub(crate) struct CpalPalette {
    records_off: usize,
    pal_start:   usize,
    n_entries:   usize,
    n_records:   usize,
}

impl CpalPalette {
    pub(crate) fn new(cpal: &[u8], palette_index: u16) -> Option<CpalPalette> {
        let n_entries    = read_u16_be(cpal, 2)? as usize;
        let num_palettes = read_u16_be(cpal, 4)? as usize;
        let n_records    = read_u16_be(cpal, 6)? as usize;
        if palette_index as usize >= num_palettes { return None; }
        let records_off = read_u32_be(cpal, 8)? as usize;
        let pal_start   = read_u16_be(cpal, 12 + palette_index as usize * 2)? as usize;
        Some(CpalPalette { records_off, pal_start, n_entries, n_records })
    }

    // An entry is refused past the palette's entries or past the table's color records, as a
    // palette that starts too late would otherwise read whatever follows them as colors.
    pub(crate) fn entry(&self, cpal: &[u8], entry_index: u16) -> Option<(u8, u8, u8, u8)> {
        let record = self.pal_start + entry_index as usize;
        if entry_index as usize >= self.n_entries || record >= self.n_records { return None; }
        let c = self.records_off.checked_add(record * 4)?;
        let b = *cpal.get(c)?;
        let g = *cpal.get(c + 1)?;
        let r = *cpal.get(c + 2)?;
        let a = *cpal.get(c + 3)?;
        Some((r, g, b, a))
    }
}

pub fn colr_layers_for_palette(
    table_map: &BTreeMap<String, TableBytes>, gid: u16, palette_index: u16,
) -> Option<Vec<ColrLayer>> {
    let colr = table_map.get("COLR")?;
    let cpal = table_map.get("CPAL")?;

    let (first_layer, n_layers, layers_off) = colr_base_glyph_layers(colr, gid)?;
    let num_palettes = read_u16_be(cpal, 4)?;
    if palette_index >= num_palettes { return None; }

    // The run ends within the layer records, not merely within the table, whose later bytes are
    // version 1 data.
    let n_layer_records = read_u16_be(colr, 12)? as usize;
    if first_layer + n_layers > n_layer_records { return None; }
    if !records_fit(layers_off.checked_add(first_layer * 4)?, n_layers, 4, colr.len()) { return None; }
    let palette = CpalPalette::new(cpal, palette_index)?;
    let mut out = Vec::with_capacity(n_layers);
    for l in 0..n_layers {
        let rec = layers_off + (first_layer + l) * 4;
        let layer_gid = read_u16_be(colr, rec)?;
        let pal_idx   = read_u16_be(colr, rec + 2)?;
        // The one index CPAL cannot resolve: it means "whatever color the text is", which is the
        // caller's to supply – hence a flag on the layer rather than a color.
        if pal_idx == 0xFFFF {
            out.push((layer_gid, 0, 0, 0, 255, true));
            continue;
        }
        let (r, g, b, a) = palette.entry(cpal, pal_idx)?;
        out.push((layer_gid, r, g, b, a, false));
    }
    Some(out)
}

pub fn colr_layers(table_map: &BTreeMap<String, TableBytes>, gid: u16) -> Option<Vec<ColrLayer>> {
    colr_layers_for_palette(table_map, gid, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A CPAL of 2 entries a palette, palettes starting at `starts`, `records` colors and, after
    // them, bytes that are not colors; version 1 with these entry labels when given.
    fn cpal(starts: &[u16], records: u16, labels: Option<&[u16]>) -> Vec<u8> {
        let v1 = labels.is_some();
        let header = 12 + 2 * starts.len() + if v1 { 12 } else { 0 };
        let mut out = [u16::from(v1), 2, starts.len() as u16, records].map(u16::to_be_bytes).concat();
        out.extend((header as u32).to_be_bytes());
        starts.iter().for_each(|s| out.extend(s.to_be_bytes()));
        let labels_at = header + 4 * usize::from(records) + 4;
        if v1 {
            out.extend([0u32, 0, labels_at as u32].map(u32::to_be_bytes).concat());
        }
        for i in 0..records {
            out.extend([i as u8, 0, 0, 255]);
        }
        out.extend([0xAB; 4]);
        labels.unwrap_or_default().iter().for_each(|l| out.extend(l.to_be_bytes()));
        out
    }

    // Palette 1 starts at record 2 of 3: its second entry would be the bytes after the records.
    #[test]
    fn an_entry_past_the_color_records_is_refused() {
        let table = cpal(&[0, 2], 3, None);
        let palette = CpalPalette::new(&table, 1).expect("palette 1");
        assert_eq!(palette.entry(&table, 0), Some((0, 0, 2, 255)));
        assert_eq!(palette.entry(&table, 1), None);
    }

    // A base glyph naming two layers when the table holds one: the second would be the bytes after.
    #[test]
    fn a_layer_run_past_the_layer_records_is_refused() {
        let mut colr = [0u16, 1].map(u16::to_be_bytes).concat();
        colr.extend([14u32, 20].map(u32::to_be_bytes).concat());
        colr.extend(1u16.to_be_bytes());
        colr.extend([5u16, 0, 2].map(u16::to_be_bytes).concat());
        colr.extend([7u16, 0, 8, 0].map(u16::to_be_bytes).concat());
        let map: BTreeMap<String, TableBytes> =
            [("COLR", colr), ("CPAL", cpal(&[0], 2, None))].into_iter().map(|(t, b)| (String::from(t), TableBytes::from(b))).collect();
        assert_eq!(colr_layers(&map, 5), None);
    }

    #[test]
    fn palette_entries_have_their_labels() {
        let map: BTreeMap<String, TableBytes> =
            [(String::from("CPAL"), TableBytes::from(cpal(&[0], 2, Some(&[300, 0xFFFF]))))].into_iter().collect();
        assert_eq!(cpal_palette_entry_labels(&map), [Some(300), None]);
        let map: BTreeMap<String, TableBytes> = [(String::from("CPAL"), TableBytes::from(cpal(&[0], 2, None)))].into_iter().collect();
        assert!(cpal_palette_entry_labels(&map).is_empty(), "a version 0 table labels nothing");
    }
}
