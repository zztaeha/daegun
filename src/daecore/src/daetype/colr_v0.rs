#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::vec::Vec;
use super::decoder::{read_u16_be, read_u32_be, records_fit, search_records};
#[cfg(test)]
use super::decoder::num_glyphs;
use crate::daecore::daekit::draw::Rgba;
use crate::daecore::daetype::Tables;
use crate::daecore::{GlyphId, NameId};

/// One layer of a COLR version 0 glyph, drawn back to front: a glyph's outline filled with one color. Laid out as C
/// reads it.
#[non_exhaustive]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ColrLayer {
    /// The glyph whose outline the layer fills.
    pub glyph: GlyphId,
    /// The palette's color; for a foreground layer, black at the alpha the text's color is drawn at.
    pub color: Rgba,
    /// Whether the layer takes the text's color in place of `color`.
    pub foreground: bool,
}

// A base glyph's layer records, found by a binary search of the base records the spec sorts: none for a glyph without
// a record, or where the records' count leaves the table, which makes the array absent.
pub(crate) fn base_glyph_layers(colr: &[u8], gid: u32) -> Option<&[[u8; 4]]> {
    let (n_base, base_off, ..) = colr_v0_header(colr)?;
    let records = colr.get(base_off..)?.as_chunks::<6>().0.get(..n_base)?;
    let word = |r: &[u8; 6], at: usize| u16::from_be_bytes([r[at], r[at + 1]]);
    let rec = &records[search_records(n_base, gid, |i| Some(word(&records[i], 0).into()))?.ok()?];
    layer_run(colr, word(rec, 2).into(), word(rec, 4).into())
}

pub(crate) fn colr_v0_header(colr: &[u8]) -> Option<(usize, usize, usize, usize)> {
    let n_base     = read_u16_be(colr, 2)? as usize;
    let base_off   = read_u32_be(colr, 4)? as usize;
    let layers_off = read_u32_be(colr, 8)? as usize;
    let n_layers   = read_u16_be(colr, 12)? as usize;
    Some((n_base, base_off, layers_off, n_layers))
}

// A base glyph's run of `n` layer records from `first`: absent unless the whole run lies within the table's layer
// records, whose later bytes may be version 1 data, and within the table.
pub(crate) fn layer_run(colr: &[u8], first: usize, n: usize) -> Option<&[[u8; 4]]> {
    let (_, _, layers_off, n_layer_records) = colr_v0_header(colr)?;
    if first.checked_add(n)? > n_layer_records {
        return None;
    }
    colr.get(layers_off..)?.as_chunks::<4>().0.get(first..first + n)
}

// Each base record, as its glyph, first layer and layer count; none where the records' count leaves the table.
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

#[cfg(test)]
pub fn cpal_palette_count(table_map: &Tables) -> u16 {
    table_map.get("CPAL").and_then(|cpal| read_u16_be(cpal, 4)).unwrap_or(0)
}

/// One CPAL palette, as a picker lists it.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
#[doc(alias("PaletteInfo"))]
pub struct Palette {
    /// Its index, the `palette` argument that selects it.
    pub index:      u16,
    /// Whether the font marks it as readable on a light background.
    pub light_safe: bool,
    /// Whether the font marks it as readable on a dark background.
    pub dark_safe:  bool,
    /// Its name's ID in the name table; None where the font names it nothing.
    pub name_id:    Option<NameId>,
}

// CPAL's version 1 arrays, which later versions hold too: palette types, palette labels and entry labels, each an
// offset or 0 for none, their records read where they fit.
pub(crate) struct CpalV1<'a> {
    cpal: &'a [u8],
    pub(crate) types: usize,
    pub(crate) labels: usize,
    pub(crate) entry_labels: usize,
}

impl<'a> CpalV1<'a> {
    pub(crate) fn read(cpal: &'a [u8]) -> Option<Self> {
        if read_u16_be(cpal, 0)? == 0 {
            return None;
        }
        let tail = 12 + 2 * usize::from(read_u16_be(cpal, 4)?);
        let offset = |k: usize| read_u32_be(cpal, tail + k).map_or(0, |o| o as usize);
        Some(CpalV1 { cpal, types: offset(0), labels: offset(4), entry_labels: offset(8) })
    }

    fn record(array: usize, size: usize, i: usize) -> Option<usize> {
        array.checked_add(size * i).filter(|_| array != 0)
    }

    fn palette_type(&self, i: usize) -> u32 {
        Self::record(self.types, 4, i).and_then(|at| read_u32_be(self.cpal, at)).unwrap_or(0)
    }

    // The name ID at `i` in the label array at `array`, None where the font gives none.
    pub(crate) fn label(&self, array: usize, i: usize) -> Option<NameId> {
        read_u16_be(self.cpal, Self::record(array, 2, i)?).filter(|&id| id != 0xFFFF).map(NameId)
    }
}

// Each palette CPAL's bytes hold, none for no bytes, one at a time so C writes them into its caller's buffer.
pub fn cpal_palettes(cpal: &[u8]) -> impl ExactSizeIterator<Item = Palette> + '_ {
    let v1 = CpalV1::read(cpal);
    (0..read_u16_be(cpal, 4).unwrap_or(0)).map(move |index| {
        let i = usize::from(index);
        let flags = v1.as_ref().map_or(0, |v1| v1.palette_type(i));
        let name_id = v1.as_ref().and_then(|v1| v1.label(v1.labels, i));
        Palette { index, light_safe: flags & 0x0001 != 0, dark_safe: flags & 0x0002 != 0, name_id }
    })
}

// Each palette entry's name ID, for a picker to label the colors a palette sets; None for an entry
// without one, and nothing for a version 0 table or one with no labels.
pub fn cpal_palette_entry_labels(table_map: &Tables) -> Vec<Option<NameId>> {
    let Some(cpal) = table_map.get("CPAL") else { return vec![] };
    let (Some(v1), Some(n_entries)) = (CpalV1::read(cpal), read_u16_be(cpal, 2)) else { return vec![] };
    if v1.entry_labels == 0 {
        return vec![];
    }
    (0..usize::from(n_entries)).map(|i| v1.label(v1.entry_labels, i)).collect()
}

// A palette's colors in entry order, stopping at an entry past the table's color records; an index past the
// font's palettes reads palette 0, as drawing does.
pub(crate) fn cpal_palette_colors(cpal: &[u8], palette: u16) -> Vec<Rgba> {
    let Some(chosen) = CpalPalette::new(cpal, palette) else { return Vec::new() };
    let entries = u16::try_from(chosen.n_entries).unwrap_or(u16::MAX);
    (0..entries).map_while(|i| chosen.entry(cpal, i)).map(|(r, g, b, a)| Rgba { r, g, b, a }).collect()
}

#[derive(Clone, Copy)]
pub(crate) struct CpalPalette {
    records_off: usize,
    pal_start:   usize,
    n_entries:   usize,
    n_records:   usize,
}

impl CpalPalette {
    // An index past the font's palettes reads palette 0, the default, as CSS's base-palette does; so does one whose
    // entries run past the color records, where such a palette can't be selected.
    pub(crate) fn new(cpal: &[u8], palette_index: u16) -> Option<CpalPalette> {
        let n_entries    = read_u16_be(cpal, 2)? as usize;
        let num_palettes = read_u16_be(cpal, 4)? as usize;
        let n_records    = read_u16_be(cpal, 6)? as usize;
        if num_palettes == 0 { return None; }
        let start = |i: usize| read_u16_be(cpal, 12 + i * 2).map(usize::from);
        let index = usize::from(palette_index);
        let chosen = start(index).filter(|&s| index < num_palettes && s + n_entries <= n_records);
        let records_off = read_u32_be(cpal, 8)? as usize;
        let pal_start   = chosen.or_else(|| start(0))?;
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

// The color a palette index names with `alpha`, 0 to 1, applied to the entry's own: the exact product, rounded once
// to a byte. 0xFFFF is the text's color, the caller's, so it is flagged true, black at that alpha.
pub(crate) fn palette_color(palette: Option<(&[u8], &CpalPalette)>, index: u16, alpha: f64) -> Option<(Rgba, bool)> {
    let byte = |of: u8| (alpha.clamp(0.0, 1.0) * f64::from(of)).round() as u8;
    if index == 0xFFFF {
        return Some((Rgba { r: 0, g: 0, b: 0, a: byte(255) }, true));
    }
    let (cpal, palette) = palette?;
    let (r, g, b, a) = palette.entry(cpal, index)?;
    Some((Rgba { r, g, b, a: byte(a) }, false))
}

// Drawn as `drawn_layers`, with maxp's count of glyphs; a map without maxp limits none.
#[cfg(test)]
pub fn colr_layers_for_palette(
    table_map: &Tables, gid: u32, palette_index: u16,
) -> Option<Vec<ColrLayer>> {
    drawn_layers(table_map, gid, palette_index, num_glyphs(table_map).unwrap_or(u16::MAX.into()))
}

// A base glyph's layers as a font of `glyphs` is drawn: a run of at least one layer, those naming a glyph past the
// font's or a color past the palette absent. None for no layer or a base past the font.
pub(crate) fn drawn_layers(
    table_map: &Tables, gid: u32, palette_index: u16, glyphs: u32,
) -> Option<Vec<ColrLayer>> {
    let drawn: Vec<ColrLayer> = each_drawn_layer(table_map, gid, palette_index, glyphs)?.collect();
    (!drawn.is_empty()).then_some(drawn)
}

// The same one at a time, as a scene draws them; None where the glyph has no run of layers to read.
pub(crate) fn each_drawn_layer(
    table_map: &Tables, gid: u32, palette_index: u16, glyphs: u32,
) -> Option<impl Iterator<Item = ColrLayer> + '_> {
    if gid >= glyphs {
        return None;
    }
    let colr = table_map.get("COLR")?;
    let cpal = table_map.get("CPAL")?;
    let layers = base_glyph_layers(colr, gid)?;
    let palette = CpalPalette::new(cpal, palette_index)?;
    Some(layers.iter().filter_map(move |&[g0, g1, p0, p1]| {
        let glyph = Some(u32::from(u16::from_be_bytes([g0, g1]))).filter(|&g| g < glyphs)?;
        let (color, foreground) = palette_color(Some((cpal, &palette)), u16::from_be_bytes([p0, p1]), 1.0)?;
        Some(ColrLayer { glyph: GlyphId(glyph), color, foreground })
    }))
}

#[cfg(test)]
pub fn colr_layers(table_map: &Tables, gid: u32) -> Option<Vec<ColrLayer>> {
    colr_layers_for_palette(table_map, gid, 0)
}

#[cfg(test)]
#[path = "../../tests/unit/daetype/colr_v0.rs"]
mod tests;
