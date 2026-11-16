#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use crate::daecore::daetype::instancer::AxisTag;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::num_glyphs;
use crate::daecore::daetype::instancer::{compute_location, finish, metrics_for, vary_metrics, Instancing};
use crate::daecore::daetype::format::sanitize::Reads;
use crate::daecore::daetype::{TableBytes, Tables};
use super::super::axis::pinned_location;
use super::super::varc::varc_instance;
use super::write::Cff2Source;
use crate::daecore::daetype::outline::Path;
use crate::daecore::daetype::varc::flatten;
use crate::daecore::Error;

pub(crate) fn instance_cff2_from_map<'a>(
    table_map:   &'a Tables,
    reads:       &Reads,
    axis_values: &[(AxisTag, f64)],
    instancing:  Instancing,
    varc:        bool,
) -> Result<BTreeMap<String, Cow<'a, [u8]>>, Error> {
    if instancing != Instancing::Described {
        let whole = |_, source: Cff2Source, coords: &[f64], n_glyphs: usize| {
            let slots: Vec<Option<u32>> = (0..n_glyphs).map(|gid| Some(gid as u32)).collect();
            source.write(coords, &slots).map(Cow::Owned)
        };
        let varc = varc.then(|| varc_instance(table_map, axis_values)).transpose()?.flatten();
        let drawn = varc.as_ref().map(|v| &v.drawn);
        let mut out_map = pinned(table_map, reads, axis_values, instancing, drawn, whole)?.0;
        if let Some(varc) = varc {
            varc.apply(&mut out_map);
        }
        return Ok(out_map);
    }
    // Without a readable fvar the font is static, and its default instance is the only one.
    let located   = compute_location(table_map, axis_values);
    let is_static = located.is_err();
    let location  = located.unwrap_or_default();
    let cff2_data = table_map.get("CFF2").ok_or(Error::missing("missing CFF2"))?;
    let metrics = metrics_for(table_map, &location, None, true)?;
    let mut out_map = finish(table_map, reads, &location, axis_values, is_static, metrics, None, instancing);
    out_map.insert("CFF2".to_string(), Cow::Borrowed(cff2_data.as_slice()));
    Ok(out_map)
}

// An instance's tables and the coordinates its CFF2 glyphs are pinned at.
type Pinned<'a> = (BTreeMap<String, Cow<'a, [u8]>>, Vec<f64>);

// The tables a whole instance at `axis_values` holds, but CFF2 left variable, and the coordinates its glyphs are
// pinned at there: a subset flattens only the glyphs it keeps.
pub(crate) fn pinned_cff2_tables<'a>(
    table_map:   &'a Tables,
    reads:       &Reads,
    axis_values: &[(AxisTag, f64)],
) -> Result<Pinned<'a>, Error> {
    pinned(table_map, reads, axis_values, Instancing::Subset, None, |data: &'a TableBytes, _, _, _| {
        Ok(Cow::Borrowed(data.as_slice()))
    })
}

// A whole instance's tables, CFF2 as `cff2` writes it from the font's at the pinned coordinates, each glyph `drawn`
// holds written and measured as drawn there.
fn pinned<'a>(
    table_map:   &'a Tables,
    reads:       &Reads,
    axis_values: &[(AxisTag, f64)],
    instancing:  Instancing,
    drawn:       Option<&BTreeMap<u32, Path>>,
    cff2:        impl FnOnce(&'a TableBytes, Cff2Source, &[f64], usize) -> Result<Cow<'a, [u8]>, Error>,
) -> Result<Pinned<'a>, Error> {
    // Without a readable fvar the font is static, and its default instance is the only one.
    let located   = compute_location(table_map, axis_values);
    let is_static = located.is_err();
    let location  = located.unwrap_or_default();
    let cff2_data = table_map.get("CFF2").ok_or(Error::missing("missing CFF2"))?;
    let n_glyphs = num_glyphs(table_map).map_or(0, |n| n as usize);
    let coords = pinned_location(table_map, axis_values);
    let varc = table_map.get("VARC").is_some_and(|t| !t.is_empty());
    let charstrings: BTreeMap<u32, Vec<u8>> = drawn.into_iter().flatten()
        .filter_map(|(&gid, path)| Some((gid, flatten::charstring(path, false)?)))
        .collect();
    let boxes: Vec<(u32, Option<[i16; 4]>)> =
        drawn.map_or(Vec::new(), |d| charstrings.keys().map(|&g| (g, flatten::measured(&d[&g]))).collect());
    let source = Cff2Source::read(cff2_data, n_glyphs, varc)?.with_drawn(charstrings);
    let cff2 = cff2(cff2_data, source, &coords, n_glyphs)?;

    let metrics = vary_metrics(table_map, &location, None)?;
    let glyphs = (0..n_glyphs).filter_map(|gid| u32::try_from(gid).ok());
    let measures = location.iter().any(|&v| v != 0.0) || !boxes.is_empty();
    let bounds = measures.then(|| {
        let mut bounds = drawn_bounds(cff2_data, &location, glyphs);
        for (gid, measured) in boxes {
            if let Some(b) = bounds.get_mut(gid as usize) {
                *b = measured;
            }
        }
        bounds
    });
    let bounds = bounds.as_deref();
    let mut out_map = finish(table_map, reads, &location, axis_values, is_static, Some(metrics), bounds, instancing);
    out_map.insert("CFF2".to_string(), cff2);
    Ok((out_map, coords))
}

// Each glyph's box as drawn at the location: edges rounded half away from 0, an unspanned axis at 0. None for a glyph
// that fails or whose edges all land on 0.
pub(crate) fn drawn_bounds(cff2: &[u8], location: &[f64], glyphs: impl Iterator<Item = u32>) -> Vec<Option<[i16; 4]>> {
    use crate::daecore::daetype::format::ivs::fixed_coords;
    use crate::daecore::daetype::outline::{outline_cff2_glyph, Cff2Outlines, PointBounds};
    let Ok(outlines) = Cff2Outlines::parse(cff2) else { return glyphs.map(|_| None).collect() };
    let coords = fixed_coords(location);
    let coords = coords.iter().any(|&c| c != 0).then_some(&coords[..]);
    let edges = |low: f32, high: f32| if low < high { [low, high].map(|v| v.round() as i32) } else { [0, 0] };
    glyphs
        .map(|gid| {
            let mut pen = PointBounds::default();
            outline_cff2_glyph(&outlines, cff2, gid, coords, None, &mut pen).ok()?;
            let (x0, y0, x1, y1) = pen.finish()?;
            let ([x_min, x_max], [y_min, y_max]) = (edges(x0, x1), edges(y0, y1));
            let bounds = [x_min, y_min, x_max, y_max];
            (bounds != [0; 4]).then(|| bounds.map(crate::daecore::daetype::decoder::clamp_i16))
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/instancer/cff2/instance.rs"]
mod tests;
