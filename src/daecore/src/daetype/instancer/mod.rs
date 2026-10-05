use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
mod cff2;
mod gpos;
mod style;
mod feature_variations;
mod name_table;
mod stat_filter;
mod axis;
mod loca;
mod coords;
mod gvar;
mod hvar;
mod mvar;
mod cvar;
mod strip_var_stores;

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use super::decoder::{build_ttf, parse_fvar_axes, read_u16_be, read_i16_be, write_u16_be, write_i16_be};
use super::format::round::ot_round;
use axis::{apply_avar_all, normalize_axis, to_fixed};
use loca::build_loca_table;
use gvar::apply_gvar;
use cvar::apply_cvar;
use crate::daecore::daetype::TableBytes;

pub(crate) use mvar::{apply_mvar, MvarTargets};
pub use mvar::mvar_deltas;
pub(crate) use strip_var_stores::strip_colr_var_store;
use strip_var_stores::{bake_base, bake_gdef};
pub(crate) use gpos::apply_gpos_var;
pub(crate) use feature_variations::resolve_layout;
pub use loca::parse_loca;
pub(crate) use coords::extract_coords_into;
pub use coords::GlyphCoords;

pub fn compute_location(
    table_map:   &BTreeMap<String, TableBytes>,
    axis_values: &[(String, f64)],
) -> Result<Vec<f64>, String> {
    let axes = parse_fvar_axes(table_map)?;
    let mut fixed: Vec<i32> = axes
        .iter()
        .map(|axis| match axis_values.iter().find(|p| p.0 == axis.tag) {
            Some(pair) => normalize_axis(to_fixed(pair.1), to_fixed(axis.min), to_fixed(axis.default), to_fixed(axis.max)),
            None => 0,
        })
        .collect();
    apply_avar_all(table_map, &mut fixed);
    Ok(fixed.into_iter().map(|v| f64::from(v) / 16384.0).collect())
}

pub fn instance_font_from_map(
    table_map:   &BTreeMap<String, TableBytes>,
    axis_values: &[(String, f64)],
) -> Result<Vec<u8>, String> {
    Ok(build_ttf(&instance_tables_from_map(table_map, axis_values)?))
}

pub fn instance_tables_from_map<'a>(
    table_map:   &'a BTreeMap<String, TableBytes>,
    axis_values: &[(String, f64)],
) -> Result<BTreeMap<String, alloc::borrow::Cow<'a, [u8]>>, String> {
    instance_tables(table_map, axis_values, true)
}

// An instance only drawn from never has its box or extremes read, and for CFF2 finding them means
// drawing every glyph, so the drawing cache leaves them as stored.
pub(crate) fn instance_tables_for_drawing<'a>(
    table_map:   &'a BTreeMap<String, TableBytes>,
    axis_values: &[(String, f64)],
) -> Result<BTreeMap<String, alloc::borrow::Cow<'a, [u8]>>, String> {
    instance_tables(table_map, axis_values, false)
}

fn instance_tables<'a>(
    table_map:   &'a BTreeMap<String, TableBytes>,
    axis_values: &[(String, f64)],
    extents:     bool,
) -> Result<BTreeMap<String, alloc::borrow::Cow<'a, [u8]>>, String> {
    if table_map.contains_key("CFF2") {
        let instanced_map = cff2::instance_cff2_from_map(table_map, axis_values, extents)?;
        return Ok(instanced_map);
    }

    let location   = compute_location(table_map, axis_values)?;
    let axis_count = location.len();

    let head        = table_map.get("head").ok_or("missing head")?;
    let loca_format = read_i16_be(head, 50).ok_or("head: truncated")?;

    let maxp       = table_map.get("maxp").ok_or("missing maxp")?;
    let num_glyphs = read_u16_be(maxp, 4).ok_or("maxp: truncated")? as usize;

    let glyph_offsets = parse_loca(table_map, loca_format, num_glyphs)?;
    let needs_var = location.iter().any(|&v| v != 0.0);

    let mut out_loca_format = loca_format;
    let mut phantoms = None;
    let glyf_src = table_map.get("glyf").ok_or("missing glyf")?.as_slice();
    // A gvar that fails to read leaves the outlines as stored, as FreeType reads such a font.
    let varied = needs_var.then(|| apply_gvar(table_map, glyf_src, &glyph_offsets, num_glyphs, &location, axis_count).ok()).flatten();
    let (glyf_data, new_loca) = if let Some(result) = varied {
        if out_loca_format == 0 && result.new_loca.last().copied().unwrap_or(0) > 0xFFFF * 2 {
            out_loca_format = 1;
        }
        phantoms = Some(Phantoms {
            advance: result.advance_deltas,
            lsb: result.lsb_new,
            vadvance: result.vadvance_deltas,
            tsb: result.tsb_new,
        });
        (Cow::Owned(result.glyf_data), Some(result.new_loca))
    } else {
        (Cow::Borrowed(glyf_src), None)
    };

    let metrics = vary_metrics(table_map, num_glyphs, &location, phantoms.as_ref())?;
    let mut cvt = table_map.get("cvt ").map(TableBytes::to_owned_vec);
    if needs_var && let Some(cvt) = &mut cvt {
        let mut varied = cvt.clone();
        if apply_cvar(table_map, &mut varied, &location, axis_count).is_ok() {
            *cvt = varied;
        }
    }
    let bounds = (extents && needs_var).then(|| glyf_bounds(&glyf_data, new_loca.as_deref().unwrap_or(&glyph_offsets)));

    let mut out_map = finish(table_map, &location, axis_values, false, metrics, bounds.as_deref());
    if let Some(new_loca) = new_loca {
        out_map.insert("loca".to_string(), Cow::Owned(build_loca_table(&new_loca, out_loca_format)));
    }
    out_map.insert("glyf".to_string(), glyf_data);
    if let Some(cvt) = cvt { out_map.insert("cvt ".to_string(), Cow::Owned(cvt)); }
    if out_loca_format != loca_format
        && let Some(head_out) = out_map.get_mut("head")
        && head_out.len() >= 52
    {
        write_i16_be(head_out.to_mut(), 50, out_loca_format);
    }
    Ok(out_map)
}

// What gvar's phantom points moved: advances and side bearings for a font with no HVAR or VVAR.
pub(crate) struct Phantoms {
    advance: Vec<f64>,
    lsb: Vec<Option<i32>>,
    vadvance: Vec<f64>,
    tsb: Vec<Option<i32>>,
}

pub(crate) struct Metrics {
    hmtx: Vec<u8>,
    h_long: usize,
    vmtx: Option<Vec<u8>>,
    v_long: usize,
}

// Advances at the location, from HVAR and VVAR where they read and from the phantom points
// otherwise. A variation table that fails to read is treated as absent, as FreeType and HarfBuzz
// treat it, rather than failing the whole instance.
pub(crate) fn vary_metrics(
    table_map: &BTreeMap<String, TableBytes>,
    num_glyphs: usize,
    location: &[f64],
    phantoms: Option<&Phantoms>,
) -> Result<Metrics, String> {
    let needs_var = location.iter().any(|&v| v != 0.0);
    let long_metrics = |tag: &str| table_map.get(tag).and_then(|h| read_u16_be(h, 34)).map_or(0, usize::from);
    let expanded = |src: &TableBytes, long: usize| {
        if needs_var { hvar::expand_metrics(src, num_glyphs, long) } else { (src.to_owned_vec(), long) }
    };
    let side = |src: &TableBytes, long: usize, var: &str, deltas: Option<&[f64]>, bearings: Option<&[Option<i32>]>| {
        let (mut mtx, long) = expanded(src, long);
        if needs_var {
            let varied = table_map.contains_key(var) && hvar::apply_metric_var(table_map, var, &mut mtx, num_glyphs, long, location).is_ok();
            if !varied && let Some(deltas) = deltas {
                bump_advances(&mut mtx, deltas, long.min(num_glyphs));
            }
        }
        match bearings {
            Some(bearings) => write_bearings(&mut mtx, bearings, long),
            None if needs_var => {
                let mut varied = mtx.clone();
                if hvar::apply_bearing_var(table_map, var, &mut varied, num_glyphs, long, location).is_ok() {
                    mtx = varied;
                }
            }
            None => {}
        }
        (mtx, long)
    };
    let hmtx = table_map.get("hmtx").ok_or("missing hmtx")?;
    let (hmtx, h_long) = side(hmtx, long_metrics("hhea"), "HVAR", phantoms.map(|p| &p.advance[..]), phantoms.map(|p| &p.lsb[..]));
    let vertical = table_map.get("vmtx").map(|vmtx| {
        side(vmtx, long_metrics("vhea"), "VVAR", phantoms.map(|p| &p.vadvance[..]), phantoms.map(|p| &p.tsb[..]))
    });
    let (vmtx, v_long) = match vertical {
        Some((vmtx, v_long)) => (Some(vmtx), v_long),
        None => (None, 0),
    };
    Ok(Metrics { hmtx, h_long, vmtx, v_long })
}

fn bump_advances(mtx: &mut [u8], deltas: &[f64], long: usize) {
    for (gid, &delta) in deltas.iter().enumerate().take(long) {
        let d = ot_round(delta);
        let off = gid * 4;
        if d == 0 || off + 2 > mtx.len() { continue; }
        let advance = read_u16_be(mtx, off).unwrap_or(0) as i32;
        write_u16_be(mtx, off, advance.saturating_add(d).clamp(0, 65535) as u16);
    }
}

fn write_bearings(mtx: &mut [u8], bearings: &[Option<i32>], long: usize) {
    for (gid, bearing) in bearings.iter().enumerate() {
        let Some(bearing) = *bearing else { continue };
        let off = if gid < long { gid * 4 + 2 } else { long * 4 + (gid - long) * 2 };
        if off + 2 > mtx.len() { continue; }
        write_i16_be(mtx, off, bearing.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
    }
}

// Each glyph's box from its glyf header; None for an empty glyph.
fn glyf_bounds(glyf: &[u8], loca: &[usize]) -> Vec<Option<[i16; 4]>> {
    loca.windows(2)
        .map(|w| {
            let at = w[0];
            if w[1] <= at { return None; }
            let field = |i: usize| read_i16_be(glyf, at + 2 + 2 * i);
            Some([field(0)?, field(1)?, field(2)?, field(3)?])
        })
        .collect()
}

const STRIPPED: &[&str] = &["fvar", "gvar", "HVAR", "MVAR", "avar", "cvar", "VVAR", "CFF2"];

// Every table both drivers instance alike once the outlines are done: metrics, MVAR, the style
// pass, the layout tables, color, and the extents the new outlines changed.
pub(crate) fn finish<'a>(
    table_map: &'a BTreeMap<String, TableBytes>,
    location: &[f64],
    axis_values: &[(String, f64)],
    is_static: bool,
    metrics: Metrics,
    bounds: Option<&[Option<[i16; 4]>]>,
) -> BTreeMap<String, Cow<'a, [u8]>> {
    let needs_var = location.iter().any(|&v| v != 0.0);
    let glyph_count = table_map.get("maxp").and_then(|m| read_u16_be(m, 4)).map_or(0, usize::from);
    let owned = |tag: &str| table_map.get(tag).map(TableBytes::to_owned_vec);
    let (mut os2, mut hhea, mut post) = (owned("OS/2"), owned("hhea"), owned("post"));
    let (mut vhea, mut gasp) = (owned("vhea"), owned("gasp"));
    if needs_var {
        // As for HVAR above, an MVAR that fails to read is no MVAR.
        let targets = MvarTargets {
            hhea: hhea.as_deref_mut().unwrap_or_default(),
            vhea: vhea.as_deref_mut().unwrap_or_default(),
            os2: os2.as_deref_mut().unwrap_or_default(),
            post: post.as_deref_mut().unwrap_or_default(),
            gasp: gasp.as_deref_mut().unwrap_or_default(),
        };
        let _ = apply_mvar(table_map, targets, location);
    }
    let style = (!is_static).then(|| {
        style::apply_style_metadata(table_map, axis_values, os2.as_deref_mut().unwrap_or_default(), post.as_deref_mut().unwrap_or_default())
    });

    let mut out: BTreeMap<String, Cow<[u8]>> = table_map
        .iter()
        .filter(|(tag, _)| !STRIPPED.contains(&tag.as_str()))
        .map(|(tag, data)| (tag.clone(), Cow::Borrowed(data.as_slice())))
        .collect();
    let layout = |tag: &str| table_map.get(tag).map(|t| &t[..]);
    let patched = if needs_var { apply_gpos_var(table_map, location) } else { None };
    let rebuilt = [
        ("GPOS", resolve_layout(layout("GPOS"), patched, location)),
        ("GSUB", resolve_layout(layout("GSUB"), None, location)),
        ("GDEF", layout("GDEF").and_then(|t| bake_gdef(t, location))),
        ("BASE", layout("BASE").and_then(|t| bake_base(t, location))),
        ("COLR", layout("COLR").and_then(|t| super::colr_v1::instance_colr_v1(t, location))),
        ("VORG", needs_var.then(|| hvar::vary_vorg(table_map, glyph_count, location)).flatten()),
    ];
    for (tag, table) in rebuilt {
        if let Some(table) = table { out.insert(tag.to_string(), Cow::Owned(table)); }
    }

    let (mut head, name, stat) = match style {
        Some(style) => (style.head, style.name, style.stat),
        None => (None, None, None),
    };
    if let (Some(vhea), Some(_)) = (vhea.as_mut().filter(|v| v.len() >= 36), &metrics.vmtx) {
        write_u16_be(vhea, 34, metrics.v_long.min(0xFFFF) as u16);
    }
    if let Some(hhea) = hhea.as_mut().filter(|h| h.len() >= 36) {
        write_u16_be(hhea, 34, metrics.h_long.min(0xFFFF) as u16);
    }
    if let Some(bounds) = bounds.filter(|_| needs_var) {
        let head = head.get_or_insert_with(|| owned("head").unwrap_or_default());
        recompute_extents(bounds, &metrics, head, hhea.as_deref_mut(), vhea.as_deref_mut(), os2.as_deref_mut());
    }

    let tables = [("hmtx", Some(metrics.hmtx)), ("vmtx", metrics.vmtx), ("vhea", vhea), ("hhea", hhea),
        ("OS/2", os2), ("post", post), ("gasp", gasp), ("head", head), ("name", name), ("STAT", stat)];
    for (tag, table) in tables {
        if let Some(table) = table.filter(|t| !t.is_empty()) { out.insert(tag.to_string(), Cow::Owned(table)); }
    }
    out
}

// head's box, hhea's and vhea's extremes and OS/2's average width, from the instance's own outlines
// and advances, as the spec defines each. Glyphs with no outline count only toward the advances.
fn recompute_extents(
    bounds: &[Option<[i16; 4]>],
    metrics: &Metrics,
    head: &mut [u8],
    hhea: Option<&mut [u8]>,
    vhea: Option<&mut [u8]>,
    os2: Option<&mut [u8]>,
) {
    let last = |mtx: &[u8], long: usize| long.checked_sub(1).and_then(|i| read_u16_be(mtx, i * 4)).unwrap_or(0);
    let h_last = last(&metrics.hmtx, metrics.h_long);
    let horizontal = |gid: usize| super::subsetter::metric_pair(&metrics.hmtx, metrics.h_long, h_last, gid);
    let inked = || bounds.iter().enumerate().filter_map(|(gid, b)| b.map(|b| (gid, b.map(i32::from))));

    if head.len() >= 44 && let Some(bbox) = inked().map(|(_, b)| b).reduce(|a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]) {
        for (i, v) in bbox.into_iter().enumerate() { write_i16_be(head, 36 + 2 * i, v as i16); }
    }
    if let Some(hhea) = hhea.filter(|h| h.len() >= 18) {
        let advance_max = (0..bounds.len()).map(|g| horizontal(g).0).max().unwrap_or(0);
        let extremes = inked().map(|(gid, b)| {
            let (aw, lsb) = horizontal(gid);
            let extent = i32::from(lsb) + b[2] - b[0];
            (i32::from(lsb), i32::from(aw) - extent, extent)
        });
        if let Some((min_lsb, min_rsb, max_extent)) = extremes.reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2))) {
            write_u16_be(hhea, 10, advance_max);
            for (at, v) in [(12, min_lsb), (14, min_rsb), (16, max_extent)] { write_i16_be(hhea, at, clamp16(v)); }
        }
    }
    if let (Some(vhea), Some(vmtx)) = (vhea.filter(|v| v.len() >= 18), &metrics.vmtx) {
        let v_last = last(vmtx, metrics.v_long);
        let vertical = |gid: usize| super::subsetter::metric_pair(vmtx, metrics.v_long, v_last, gid);
        let advance_max = (0..bounds.len()).map(|g| vertical(g).0).max().unwrap_or(0);
        let extremes = inked().map(|(gid, b)| {
            let (ah, tsb) = vertical(gid);
            let extent = i32::from(tsb) + b[3] - b[1];
            (i32::from(tsb), i32::from(ah) - extent, extent)
        });
        if let Some((min_tsb, min_bsb, max_extent)) = extremes.reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2))) {
            write_u16_be(vhea, 10, advance_max);
            for (at, v) in [(12, min_tsb), (14, min_bsb), (16, max_extent)] { write_i16_be(vhea, at, clamp16(v)); }
        }
    }
    // From version 3 the average is over every glyph with a non-zero advance.
    if let Some(os2) = os2.filter(|o| o.len() >= 4 && read_u16_be(o, 0).is_some_and(|v| v >= 3)) {
        let widths: Vec<u32> = (0..bounds.len()).map(|g| u32::from(horizontal(g).0)).filter(|&w| w > 0).collect();
        if !widths.is_empty() {
            let average = ot_round(f64::from(widths.iter().sum::<u32>()) / widths.len() as f64);
            write_i16_be(os2, 2, clamp16(average));
        }
    }
}

fn clamp16(v: i32) -> i16 {
    v.clamp(i16::MIN.into(), i16::MAX.into()) as i16
}
