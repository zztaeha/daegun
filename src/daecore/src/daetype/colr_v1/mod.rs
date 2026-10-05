#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;
use alloc::string::String;
use alloc::vec::Vec;
mod format;
mod paint;
mod color_line;
mod varfield;
mod instance;
pub(crate) mod write;

use alloc::collections::BTreeMap;
use super::decoder::{read_i16_be, read_offset24, read_u16_be, read_u32_be, records_fit, search_records};
use super::format::ivs::{parse_delta_set_index_map, parse_item_variation_store, precompute_region_scalars, ItemVariationStore};
use crate::daecore::daetype::TableBytes;
use crate::daecore::daemachine::daemath::matrix::{concat, Matrix};

pub use paint::{ColorStop, Paint};
pub(crate) use format::paint_layout;
pub(crate) use instance::instance_colr_v1;

pub(crate) const MAX_PAINT_DEPTH: usize = 64;

pub(crate) const MAX_PAINT_VISITS: usize = 100_000;

pub(crate) const MAX_PAINT_STOPS: usize = 1_000_000;

// One glyph's walk: the paints from the root down to the one being read, so a paint that names one
// of them is a cycle, and what is left of the visits and stops a glyph may spend.
pub(crate) struct PaintBudget {
    path: Vec<usize>,
    visits: usize,
    stops: usize,
}

impl PaintBudget {
    pub fn new() -> Self {
        PaintBudget { path: Vec::new(), visits: MAX_PAINT_VISITS, stops: MAX_PAINT_STOPS }
    }

    #[must_use]
    pub(crate) fn spend_stops(&mut self, n: usize) -> bool {
        match self.stops.checked_sub(n) {
            Some(left) => { self.stops = left; true }
            None => false,
        }
    }

    // False for a paint past the depth limit, past the visit budget, or already on the path.
    #[must_use]
    pub(crate) fn enter(&mut self, off: usize) -> bool {
        if self.path.len() > MAX_PAINT_DEPTH || self.visits == 0 || self.path.contains(&off) {
            return false;
        }
        self.path.push(off);
        self.visits -= 1;
        true
    }

    pub(crate) fn leave(&mut self) {
        self.path.pop();
    }
}

pub struct ColrV1VarData {
    pub var_store:     Option<ItemVariationStore>,
    pub var_index_map: Option<Vec<(u32, u32)>>,
}

pub fn parse_colr_v1_var_data(colr: &[u8]) -> ColrV1VarData {
    let none = ColrV1VarData { var_store: None, var_index_map: None };
    let var_index_map_off = read_u32_be(colr, 26).unwrap_or(0);
    let ivs_off = read_u32_be(colr, 30).unwrap_or(0);
    if ivs_off == 0 {
        return none;
    }
    let Ok(var_store) = parse_item_variation_store(colr, ivs_off as usize) else { return none };
    // The implicit mapping is for a table with no map; one that is named and cannot be read leaves
    // no way to find the right deltas, so nothing varies.
    let var_index_map = match var_index_map_off {
        0 => None,
        off => match parse_delta_set_index_map(colr, off as usize) {
            Ok(map) => Some(map),
            Err(_) => return none,
        },
    };
    ColrV1VarData { var_store: Some(var_store), var_index_map }
}

pub(crate) struct Colrv1Ctx<'a> {
    pub colr:            &'a [u8],
    pub cpal:             Option<&'a [u8]>,
    pub palette:          Option<super::colr_v0::CpalPalette>,
    pub var_store:        Option<&'a ItemVariationStore>,
    pub region_scalars:   &'a [f64],
    pub var_index_map:    Option<&'a [(u32, u32)]>,
    pub base_glyph_list_off: usize,
    pub layer_list_off:   Option<usize>,
}

pub fn colr_v1_paint_graph_cached(
    table_map:     &BTreeMap<String, TableBytes>,
    gid:           u16,
    location:      &[f64],
    palette_index: u16,
    var_data:      &ColrV1VarData,
) -> Option<Paint> {
    let scalars = colr_v1_region_scalars(var_data, location);
    build(table_map, gid, palette_index, var_data, &scalars)
}

pub fn colr_v1_region_scalars(var_data: &ColrV1VarData, location: &[f64]) -> Vec<f64> {
    match &var_data.var_store {
        Some(store) => precompute_region_scalars(store, location),
        None => Vec::new(),
    }
}

pub fn colr_v1_paint_graph_with_scalars(
    table_map:      &BTreeMap<String, TableBytes>,
    gid:            u16,
    palette_index:  u16,
    var_data:       &ColrV1VarData,
    region_scalars: &[f64],
) -> Option<Paint> {
    build(table_map, gid, palette_index, var_data, region_scalars)
}

fn build(
    table_map:      &BTreeMap<String, TableBytes>,
    gid:            u16,
    palette_index:  u16,
    var_data:       &ColrV1VarData,
    region_scalars: &[f64],
) -> Option<Paint> {
    let colr = table_map.get("COLR")?;
    if colr.len() < 34 { return None; }
    if read_u16_be(colr, 0)? != 1 { return None; }

    let base_glyph_list_off = read_u32_be(colr, 14)? as usize;
    if base_glyph_list_off == 0 { return None; }

    let layer_list_raw = read_u32_be(colr, 18)?;
    let layer_list_off = if layer_list_raw == 0 { None } else { Some(layer_list_raw as usize) };

    let cpal = table_map.get("CPAL").map(|v| v.as_slice());
    let palette = cpal.and_then(|c| super::colr_v0::CpalPalette::new(c, palette_index));
    let ctx = Colrv1Ctx {
        colr, cpal, palette,
        var_store: var_data.var_store.as_ref(),
        region_scalars,
        var_index_map: var_data.var_index_map.as_deref(),
        base_glyph_list_off, layer_list_off,
    };

    let mut budget = PaintBudget::new();
    let root = lookup_base_glyph_paint_offset(colr, base_glyph_list_off, gid)?;
    if !budget.enter(root) { return None; }
    paint::parse_paint(&ctx, root, &mut budget)
}

pub(super) fn lookup_base_glyph_paint_offset(colr: &[u8], base_glyph_list_off: usize, gid: u16) -> Option<usize> {
    let num_records = read_u32_be(colr, base_glyph_list_off)? as usize;
    let records_off = base_glyph_list_off.checked_add(4)?;
    let at = |i: usize| i.checked_mul(6).and_then(|o| records_off.checked_add(o));
    let hit = search_records(num_records, gid as u32, |i| read_u16_be(colr, at(i)?).map(u32::from))?.ok()?;
    let rel_off = read_u32_be(colr, at(hit)?.checked_add(2)?)? as usize;
    base_glyph_list_off.checked_add(rel_off)
}

// Where a base glyph's ClipBox starts, from the ClipList's records sorted by first glyph.
pub(crate) fn clip_box_offset(colr: &[u8], gid: u16) -> Option<usize> {
    if colr.len() < 34 || read_u16_be(colr, 0)? != 1 { return None; }
    let list = read_u32_be(colr, 22)? as usize;
    if list == 0 || *colr.get(list)? != 1 { return None; }
    let count = read_u32_be(colr, list.checked_add(1)?)? as usize;
    let records = list.checked_add(5)?;
    if !records_fit(records, count, 7, colr.len()) { return None; }
    let start_of = |i: usize| read_u16_be(colr, records + i * 7).map(u32::from);
    let i = match search_records(count, u32::from(gid), start_of)? {
        Ok(i) => i,
        Err(0) => return None,
        Err(i) => i - 1,
    };
    let rec = records + i * 7;
    if read_u16_be(colr, rec + 2)? < gid { return None; }
    list.checked_add(read_offset24(colr, rec + 4)?)
}

// A color glyph's clip box as (x_min, y_min, x_max, y_max) in font units; a variable one is rounded
// outward, as the spec asks.
pub fn clip_box(colr: &[u8], gid: u16, var_data: &ColrV1VarData, region_scalars: &[f64]) -> Option<[i32; 4]> {
    let at = clip_box_offset(colr, gid)?;
    let raw = [1, 3, 5, 7].map(|k| at.checked_add(k).and_then(|o| read_i16_be(colr, o)));
    let [Some(x0), Some(y0), Some(x1), Some(y1)] = raw else { return None };
    match *colr.get(at)? {
        1 => Some([x0, y0, x1, y1].map(i32::from)),
        2 => {
            let vib = read_u32_be(colr, at.checked_add(9)?)?;
            let varied = |v: i16, k: u32| {
                f64::from(v) + varfield::resolve_delta_raw(var_data.var_store.as_ref(), var_data.var_index_map.as_deref(), region_scalars, vib, k)
            };
            let (fx0, fy0, fx1, fy1) = (varied(x0, 0), varied(y0, 1), varied(x1, 2), varied(y1, 3));
            Some([outward(fx0, false), outward(fy0, false), outward(fx1, true), outward(fy1, true)])
        }
        _ => None,
    }
}

// A varied clip box edge rounded outward, from the 16.16 precision deltas carry: noise in the
// last bits of a float must not move a whole unit.
pub(crate) fn outward(v: f64, up: bool) -> i32 {
    let v = (v * 65536.0).round() / 65536.0;
    (if up { v.ceil() } else { v.floor() }) as i32
}

// The matrices of COLR's transforming paints, x' = a*x + c*y + e as everywhere in daegun, each
// about an optional center. A skew leans x by -tan of its angle, as the spec defines it.
pub(crate) fn scale_matrix(sx: f64, sy: f64, center: Option<(f64, f64)>) -> Matrix {
    about(center, [sx, 0.0, 0.0, sy, 0.0, 0.0])
}

pub(crate) fn rotate_matrix(degrees: f64, center: Option<(f64, f64)>) -> Matrix {
    let (sin, cos) = crate::daecore::daemachine::float::sin_cos(degrees.to_radians());
    about(center, [cos, sin, -sin, cos, 0.0, 0.0])
}

pub(crate) fn skew_matrix(x_degrees: f64, y_degrees: f64, center: Option<(f64, f64)>) -> Matrix {
    let tan = |deg: f64| {
        let (s, c) = crate::daecore::daemachine::float::sin_cos(deg.to_radians());
        if c == 0.0 { 0.0 } else { s / c }
    };
    about(center, [1.0, tan(y_degrees), -tan(x_degrees), 1.0, 0.0, 0.0])
}

fn about(center: Option<(f64, f64)>, m: Matrix) -> Matrix {
    let Some((cx, cy)) = center else { return m };
    concat(&concat(&[1.0, 0.0, 0.0, 1.0, -cx, -cy], &m), &[1.0, 0.0, 0.0, 1.0, cx, cy])
}

#[cfg(test)]
mod tests {
    use super::*;

    // Offsets near u32::MAX, which on a 32-bit target would overflow an unchecked sum and trap in a
    // debug build: the paint is past the table, and there is none.
    #[test]
    fn offsets_near_the_limit_read_as_absent() {
        let mut colr = alloc::vec![0u8; 34];
        colr[1] = 1;
        colr[14..18].copy_from_slice(&34u32.to_be_bytes());
        colr.extend(1u32.to_be_bytes());
        colr.extend(1u16.to_be_bytes());
        colr.extend(0xFFFF_FFF0u32.to_be_bytes());
        let map: BTreeMap<String, TableBytes> = [(String::from("COLR"), TableBytes::from(colr.clone()))].into_iter().collect();
        let var = parse_colr_v1_var_data(&colr);
        assert_eq!(colr_v1_paint_graph_cached(&map, 1, &[], 0, &var), None);
        assert_eq!(clip_box(&colr, 1, &var, &[]), None);
    }

    // 2,000 glyphs sharing 60 layers walk 242,000 paints between them, past one budget of 100,000;
    // the instancer walks each paint once, so the last glyph's alpha still varies.
    #[test]
    fn instancing_reaches_every_glyph_of_a_shared_graph() {
        let colr = testing::shared_layers(2000, 60, true);
        let out = instance_colr_v1(&colr, &[1.0]).expect("an instance");
        let map: BTreeMap<String, TableBytes> = [(String::from("COLR"), TableBytes::from(out.clone()))].into_iter().collect();
        let var = parse_colr_v1_var_data(&out);
        assert!(var.var_store.is_none(), "the store is left");
        let Some(Paint::Layers(layers)) = colr_v1_paint_graph_cached(&map, 2000, &[], 0, &var) else { panic!("glyph 2000") };
        assert_eq!(layers.len(), 60);
        assert!(layers.iter().all(|l| matches!(l, Paint::Glyph { child, .. } if matches!(**child, Paint::Solid { alpha: 0, .. }))));
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use alloc::vec::Vec;

    // An item variation store over one axis whose `subtables` data subtables each hold one row:
    // `delta` at the axis's peak.
    pub(crate) fn store(subtables: u16, delta: i16) -> Vec<u8> {
        let header = 8 + 4 * usize::from(subtables);
        let mut out = 1u16.to_be_bytes().to_vec();
        out.extend((header as u32).to_be_bytes());
        out.extend(subtables.to_be_bytes());
        for _ in 0..subtables {
            out.extend((header as u32 + 10).to_be_bytes());
        }
        out.extend([1u16, 1, 0, 0x4000, 0x4000].map(u16::to_be_bytes).concat());
        out.extend([1u16, 1, 1, 0].map(u16::to_be_bytes).concat());
        out.extend(delta.to_be_bytes());
        out
    }

    // A COLR whose base glyphs 1 to `bases` share one PaintColrLayers of `layers` PaintGlyphs
    // (glyphs 1000 on) in the foreground, a PaintVarSolid whose delta takes alpha to 0 if `varied`.
    pub(crate) fn shared_layers(bases: u16, layers: u8, varied: bool) -> Vec<u8> {
        let list = 34usize;
        let colr_layers = list + 4 + 6 * usize::from(bases);
        let layer_list = colr_layers + 6;
        let paints = layer_list + 4 + 4 * usize::from(layers);
        let solid: Vec<u8> = if varied {
            [&[3u8, 0xFF, 0xFF, 0x40, 0][..], &0u32.to_be_bytes()].concat()
        } else {
            alloc::vec![2, 0xFF, 0xFF, 0x40, 0]
        };
        let paint_len = 6 + solid.len();
        let store_at = paints + paint_len * usize::from(layers);
        let mut out = alloc::vec![0u8; 34];
        out[1] = 1;
        out[14..18].copy_from_slice(&(list as u32).to_be_bytes());
        out[18..22].copy_from_slice(&(layer_list as u32).to_be_bytes());
        if varied {
            out[30..34].copy_from_slice(&(store_at as u32).to_be_bytes());
        }
        out.extend(u32::from(bases).to_be_bytes());
        for gid in 1..=bases {
            out.extend(gid.to_be_bytes());
            out.extend(((colr_layers - list) as u32).to_be_bytes());
        }
        out.extend([1, layers, 0, 0, 0, 0]);
        out.extend(u32::from(layers).to_be_bytes());
        for i in 0..usize::from(layers) {
            out.extend(((paints + i * paint_len - layer_list) as u32).to_be_bytes());
        }
        for i in 0..u16::from(layers) {
            out.extend([10, 0, 0, 6]);
            out.extend((1000 + i).to_be_bytes());
            out.extend(&solid);
        }
        if varied {
            out.extend(store(1, -16384));
        }
        out
    }
}
