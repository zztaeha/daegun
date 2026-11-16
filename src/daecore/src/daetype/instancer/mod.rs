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
mod drawn;
mod measured;
mod varc;

use alloc::borrow::Cow;
use alloc::collections::BTreeMap;
use super::decoder::{
    build_ttf, bump_u16, clamp_i16, copies_held, f2dot14, glyf_tags, num_glyphs, parse_fvar_axes, read_u16_be,
    read_i16_be, write_u16_be, write_i16_be,
};
use super::format::round::ot_round;
use super::format::ivs::round_fixed;
use super::format::sanitize::Reads;
use super::subsetter::otl::gdef::{mark_glyph_set_count, sets_of_every_glyph};
use axis::normalize_axis;
use super::format::glyf::{
    glyf_glyph_count, glyph_record, loca_format, loca_table, simple_point_count, ComponentPath, Components, Stack,
    MAX_VISITS,
};
use super::format::metrics::{bearing_at, compacted, narrowed, shaped_tags, MetricTags, MetricsTable};
use gvar::apply_gvar;
use cvar::apply_cvar;
use crate::daecore::daetype::{TableBytes, Tables};

pub(crate) use mvar::{apply_mvar, MvarTargets};
pub(crate) use strip_var_stores::strip_colr_var_store;
use strip_var_stores::{bake_base, bake_gdef, bake_math};
pub(crate) use gpos::apply_gpos_var;
pub(crate) use gvar::GvarHeader;
pub(crate) use hvar::{advance_views, VariedAdvances, VariedOrigins};
pub(crate) use feature_variations::resolve_layout;
pub use axis::AxisTag;
pub use loca::parse_loca;
pub(crate) use coords::{extract_coords_into, extract_points_into, write_simple_glyph};
pub use coords::GlyphCoords;
pub(crate) use drawn::{
    coordinates as drawn_coordinates, fixed_to_f26dot6, fixed_to_int, Cff2Store, DeltaRoom,
    DeltaSource, DrawnAdvances, DrawnLocation, Gvar,
};
pub(crate) use drawn::{div_fix, mul_fix as drawn_mul_fix, ONE as FIXED_ONE};
pub(crate) use measured::{GlyfTables, MeasuredGlyphs};
pub(crate) use cff2::{pinned_cff2_tables, Cff2Source, Flattened};
use crate::daecore::Error;

pub fn compute_location(
    table_map:   &Tables,
    axis_values: &[(AxisTag, f64)],
) -> Result<Vec<f64>, Error> {
    Ok(Normalizer::read(table_map)?.location(axis_values))
}

// What a location's normalized coordinates are worked out from, fvar's axes and avar's maps, read once for all the
// locations a font's cache works out.
pub(crate) struct Normalizer {
    axes: Vec<super::decoder::Axis>,
    avar: Option<axis::Avar>,
}

impl Normalizer {
    pub(crate) fn read(table_map: &Tables) -> Result<Normalizer, Error> {
        let axes = parse_fvar_axes(table_map)?;
        Ok(Normalizer { axes, avar: table_map.get("avar").and_then(|a| axis::Avar::read(a)) })
    }

    pub(crate) fn location(&self, axis_values: &[(AxisTag, f64)]) -> Vec<f64> {
        // A tag named twice takes its last value.
        let mut fixed: Vec<i32> = self.axes
            .iter()
            .map(|axis| match axis_values.iter().rev().find(|p| p.0 == axis.tag) {
                Some(pair) => normalize_axis(pair.1, axis.min, axis.default, axis.max),
                None => 0,
            })
            .collect();
        axis::apply_avar(self.avar.as_ref(), &mut fixed);
        fixed.into_iter().map(f2dot14).collect()
    }
}

// Each takes the font's tables with the sanitizer's verdicts on them (`Reads::of_tables`), the same at every location.
pub(crate) fn instance_font_from_map(
    table_map:   &Tables,
    reads:       &Reads,
    axis_values: &[(AxisTag, f64)],
) -> Result<Vec<u8>, Error> {
    build_ttf(&instance_tables_from_map(table_map, reads, axis_values)?)
}

pub(crate) fn instance_tables_from_map<'a>(
    table_map:   &'a Tables,
    reads:       &Reads,
    axis_values: &[(AxisTag, f64)],
) -> Result<BTreeMap<String, alloc::borrow::Cow<'a, [u8]>>, Error> {
    instance_tables(table_map, reads, axis_values, Instancing::Whole, true)
}

// As `instance_tables_from_map`, VARC left as stored for a subset to keep or draw glyph by glyph.
pub(crate) fn instance_tables_for_subset<'a>(
    table_map:   &'a Tables,
    reads:       &Reads,
    axis_values: &[(AxisTag, f64)],
) -> Result<BTreeMap<String, alloc::borrow::Cow<'a, [u8]>>, Error> {
    instance_tables(table_map, reads, axis_values, Instancing::Subset, false)
}

// The tables a described location varies, at `axis_values`: it holds the rest as the font's kept tables.
pub(crate) fn instance_tables_described<'a>(
    table_map:   &'a Tables,
    reads:       &Reads,
    axis_values: &[(AxisTag, f64)],
) -> Result<BTreeMap<String, alloc::borrow::Cow<'a, [u8]>>, Error> {
    instance_tables(table_map, reads, axis_values, Instancing::Described, false)
}

// Whether a described location's MVAR or style pass may vary `tag`, or it is a hybrid's maxp: all else it holds as the
// font does, less those its instance drops. Inlined, a tag known as the code is built folds to an answer.
#[inline(always)]
pub(crate) fn described_varies(tag: &str) -> bool {
    matches!(tag, "OS/2" | "hhea" | "vhea" | "HHEA" | "VHEA" | "post" | "gasp" | "head" | "STAT" | "maxp")
}

// The font's tables a described location holds, failing where `instance_tables_described` fails given its
// `normalizer`: all but those its instance drops, the ones it varies as the font's until it builds its own.
pub(crate) fn described_kept(
    table_map: &Tables, reads: &Reads, normalizer: Result<&Normalizer, &Error>,
) -> Result<Tables, Error> {
    let cff2 = table_map.contains_key("CFF2");
    let tags = glyf_tags(table_map);
    if !cff2 {
        normalizer.map_err(Error::clone)?;
        let head = table_map.get("head").ok_or(Error::missing("missing head"))?;
        read_i16_be(head, 50).ok_or(Error::malformed("head: truncated"))?;
        num_glyphs(table_map).ok_or(Error::missing("maxp: missing or truncated"))?;
        table_map.get(tags.loca).ok_or(Error::missing("missing loca"))?;
        table_map.get(tags.glyf).ok_or(Error::missing("missing glyf"))?;
    }
    let layout = reads.reading(|tag| table_map.get(tag).map(|t| &t[..]));
    let unread = if tags.extended { ["glyf", "loca"] } else { ["GLYF", "LOCA"] };
    Ok(table_map.filtered(|tag| match tag {
        "CFF2" => cff2,
        "name" => false,
        _ => !STRIPPED.contains(&tag) && layout(tag).is_some() && (cff2 || !unread.contains(&tag)),
    }))
}

// What an instance is for: Whole and Subset measure varied outlines for boxes and extremes, Subset leaving GPOS's
// VariationIndex for the subset to drop; Described reads metrics and style, leaving the rest for shaping to vary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Instancing {
    Whole,
    Subset,
    Described,
}

// A whole instance decides VARC where `varc`, keeping it or holding each glyph it draws.
fn instance_tables<'a>(
    table_map:   &'a Tables,
    reads:       &Reads,
    axis_values: &[(AxisTag, f64)],
    instancing:  Instancing,
    varc:        bool,
) -> Result<BTreeMap<String, alloc::borrow::Cow<'a, [u8]>>, Error> {
    let varc = varc && instancing == Instancing::Whole;
    // A described location holds the font's tables as they are; the others are written out.
    if instancing != Instancing::Described {
        copies_held(table_map)?;
    }
    if table_map.contains_key("CFF2") {
        let instanced_map = cff2::instance_cff2_from_map(table_map, reads, axis_values, instancing, varc)?;
        return Ok(instanced_map);
    }

    let location   = compute_location(table_map, axis_values)?;

    let head        = table_map.get("head").ok_or(Error::missing("missing head"))?;
    read_i16_be(head, 50).ok_or(Error::malformed("head: truncated"))?;
    let loca_format = loca_format(head);

    let glyph_count = (num_glyphs(table_map).ok_or(Error::missing("maxp: missing or truncated"))?) as usize;
    let tags = glyf_tags(table_map);
    let needs_var = location.iter().any(|&v| v != 0.0);
    let stored = instancing == Instancing::Described;
    // A described location moves no glyph, so it asks only that loca be there, where reading it took Inter's 1.7 µs.
    let glyph_offsets = match stored {
        true => table_map.get(tags.loca).ok_or(Error::missing("missing loca")).map(|_| Vec::new())?,
        false => {
            let mut offsets = parse_loca(table_map, loca_format.unwrap_or(0), glyph_count)?;
            let loca_len = table_map.get(tags.loca).map_or(0, |l| l.len());
            offsets.truncate(loca_format.map_or(0, |format| glyf_glyph_count(loca_len, format, glyph_count)) + 1);
            offsets
        }
    };

    let mut phantoms = None;
    let glyf_src = table_map.get(tags.glyf).ok_or(Error::missing("missing glyf"))?.as_slice();
    // A gvar that fails to read, or a glyf holding no glyph, leaves the outlines and advances as stored.
    let vary = needs_var && glyph_offsets.len() > 1 && !stored;
    let varied = vary.then(|| apply_gvar(table_map, glyf_src, &glyph_offsets, &location).ok()).flatten();
    let (glyf_data, new_loca) = match varied {
        Some(result) => {
            phantoms = Some(Phantoms {
                advance: result.advances,
                lsb: result.lsb_new,
                vadvance: result.vadvances,
                tsb: result.tsb_new,
            });
            (Cow::Owned(result.glyf_data), Some(result.new_loca))
        }
        None => (Cow::Borrowed(glyf_src), None),
    };
    // A whole instance holds each glyph a VARC it can't keep draws, its side bearing at its xMin, since drawing
    // shifts a glyf outline by any difference.
    let varc = varc.then(|| varc::varc_instance(table_map, axis_values)).transpose()?.flatten();
    let drawn: BTreeMap<u32, Vec<u8>> = varc.iter().flat_map(|v| &v.drawn)
        .filter_map(|(&gid, path)| Some((gid, super::varc::flatten::glyf(path, tags.extended)?)))
        .collect();
    let (glyf_data, new_loca) = match drawn.is_empty() {
        true => (glyf_data, new_loca),
        false => {
            let loca = new_loca.as_deref().unwrap_or(&glyph_offsets);
            let (glyf, loca) = varc::with_drawn(&glyf_data, loca, glyph_count, &drawn)?;
            (Cow::Owned(glyf), Some(loca))
        }
    };

    let mut metrics = metrics_for(table_map, &location, phantoms.as_ref(), stored)?;
    if let Some(metrics) = metrics.as_mut().filter(|_| !drawn.is_empty()) {
        let mut bearings = alloc::vec![None; glyph_count];
        for (&gid, glyph) in &drawn {
            bearings[gid as usize] = read_i16_be(glyph, 2).map(i32::from);
        }
        write_bearings(&mut metrics.hmtx, &bearings, metrics.h_long);
    }
    let mut cvt = table_map.get("cvt ").map(TableBytes::to_owned_vec);
    // cvar fails only before its first write, so a cvt it can't vary stays as stored.
    if needs_var && !stored && let Some(cvt) = &mut cvt {
        let _ = apply_cvar(table_map, cvt, &location);
    }
    let loca = new_loca.as_deref().unwrap_or(&glyph_offsets);
    let measures = needs_var || !drawn.is_empty();
    let bounds = (!stored && measures).then(|| glyf_bounds(&glyf_data, loca, glyph_count));
    let counts = bounds.as_ref().zip(metrics.as_ref()).and_then(|(bounds, m)| {
        let hmtx = MetricsTable::new(&m.hmtx, m.h_long);
        GlyfCounts::of(bounds.iter().copied().enumerate(), &glyf_data, tags.extended, loca, &hmtx)
    });

    let mut out_map = finish(table_map, reads, &location, axis_values, false, metrics, bounds.as_deref(), instancing);
    if let Some(counts) = counts {
        if let Some(head) = out_map.get_mut("head") {
            counts.write_head(head.to_mut());
        }
        for (tag, wide) in [("maxp", false), ("MAXP", true)] {
            if let Some(maxp) = out_map.get(tag).and_then(|m| counts.maxp(m, wide)) {
                out_map.insert(tag.to_string(), Cow::Owned(maxp));
            }
        }
    }
    if let Some(new_loca) = new_loca {
        let (loca, format) = loca_table(&new_loca).ok_or(Error::malformed("instance: glyf past 4 GB"))?;
        out_map.insert(tags.loca.to_string(), Cow::Owned(loca));
        if Some(format) != loca_format
            && let Some(head_out) = out_map.get_mut("head")
            && head_out.len() >= 52
        {
            write_i16_be(head_out.to_mut(), 50, format);
        }
    }
    out_map.insert(tags.glyf.to_string(), glyf_data);
    // Only the pair it reads is written: its twin, left as the source had it, would disagree with the instance.
    for tag in if tags.extended { ["glyf", "loca"] } else { ["GLYF", "LOCA"] } {
        out_map.remove(tag);
    }
    if let Some(cvt) = cvt { out_map.insert("cvt ".to_string(), shared_if_same(table_map, "cvt ", cvt)); }
    if let Some(varc) = varc {
        varc.apply(&mut out_map);
    }
    Ok(out_map)
}

// A table that came out as the font's own is the font's, shared where it would be kept twice.
fn shared_if_same<'a>(table_map: &'a Tables, tag: &str, table: Vec<u8>) -> Cow<'a, [u8]> {
    match table_map.get(tag) {
        Some(source) if source[..] == table[..] => Cow::Borrowed(&source[..]),
        _ => Cow::Owned(table),
    }
}

// A described location's name, which `finish` leaves out: the style pass's, cut to the records read. None where the
// location names itself as the font does. MVAR, which moves no field the names are made from, is left out.
pub(crate) fn described_name(
    table_map: &Tables, axis_values: &[(AxisTag, f64)],
) -> Option<Vec<u8>> {
    if table_map.contains_key("CFF2") && compute_location(table_map, axis_values).is_err() {
        return None;
    }
    let mut os2 = table_map.get("OS/2").map(TableBytes::to_owned_vec);
    let os2 = os2.as_deref_mut().unwrap_or_default();
    let style = style::apply_style_metadata(table_map, axis_values, os2, &mut [], true);
    let name = style.name?;
    Some(super::decoder::read_records_only(&name).unwrap_or(name))
}

// The instance's metrics, or None `by_glyph`, which leaves them as stored for its cache to vary glyph by glyph.
pub(crate) fn metrics_for(
    table_map: &Tables,
    location: &[f64],
    phantoms: Option<&Phantoms>,
    by_glyph: bool,
) -> Result<Option<Metrics>, Error> {
    match by_glyph {
        true => Ok(None),
        false => vary_metrics(table_map, location, phantoms).map(Some),
    }
}

// What gvar's phantom points give each glyph glyf holds: advances for a font with no HVAR or VVAR, and side bearings.
pub(crate) struct Phantoms {
    advance: Vec<i32>,
    lsb: Vec<Option<i32>>,
    vadvance: Vec<i32>,
    tsb: Vec<Option<i32>>,
}

pub(crate) struct Metrics {
    hmtx: Vec<u8>,
    h_long: usize,
    vmtx: Option<Vec<u8>>,
    v_long: usize,
}

// Advances at the location from HVAR and VVAR, else the phantom points, long records cut to the last advance that
// changes. Metrics with no long records give every glyph the default advance anywhere, so they stay as they are.
pub(crate) fn vary_metrics(
    table_map: &Tables,
    location: &[f64],
    phantoms: Option<&Phantoms>,
) -> Result<Metrics, Error> {
    let needs_var = location.iter().any(|&v| v != 0.0);
    let side = |vertical: bool, var: &str, phantom: Option<(&[i32], &[Option<i32>])>| {
        let tags = shaped_tags(table_map, vertical);
        let (src, stated) = (table_map.get(tags.mtx)?, tags.long(table_map));
        let Some(table) = tags.table(table_map).filter(|t| needs_var && t.has_data()) else {
            return Some(compacted(src.to_owned_vec(), stated));
        };
        let deltas = hvar::advance_deltas(table_map, var);
        let count = match &deltas {
            Some(deltas) => hvar::varied_count(&table, deltas.extent()),
            None => table.len().max(phantom.map_or(0, |(advances, _)| advances.len())),
        };
        let (mut mtx, long) = match table.long() >= count {
            true => (src.to_owned_vec(), stated),
            false => (hvar::expand_metrics(&table, count), count),
        };
        if let Some(deltas) = &deltas {
            let scalars = deltas.scalars(location);
            let mut delta = deltas.glyph_deltas(&scalars);
            for gid in 0..count {
                bump_u16(&mut mtx, gid * 4, round_fixed(delta(gid)));
            }
        } else if let Some((advances, _)) = phantom {
            // A glyph past glyf's has no phantom points to measure, and no advance.
            for gid in 0..count {
                match advances.get(gid) {
                    Some(&advance) => { write_u16_be(&mut mtx, gid * 4, advance.clamp(0, u16::MAX.into()) as u16); }
                    None => { write_u16_be(&mut mtx, gid * 4, 0); }
                }
            }
        }
        match phantom {
            Some((_, bearings)) => write_bearings(&mut mtx, bearings, long),
            // It fails only before its first write, so bearings it can't vary stay as stored.
            None => {
                let _ = hvar::apply_bearing_var(table_map, var, &mut mtx, count, long, location);
            }
        }
        Some(compacted(mtx, long))
    };
    let horizontal = phantoms.map(|p| (&p.advance[..], &p.lsb[..]));
    let (hmtx, h_long) = side(false, "HVAR", horizontal).ok_or(Error::missing("missing hmtx"))?;
    let (vmtx, v_long) = match side(true, "VVAR", phantoms.map(|p| (&p.vadvance[..], &p.tsb[..]))) {
        Some((vmtx, v_long)) => (Some(vmtx), v_long),
        None => (None, 0),
    };
    Ok(Metrics { hmtx, h_long, vmtx, v_long })
}

fn write_bearings(mtx: &mut [u8], bearings: &[Option<i32>], long: usize) {
    for (gid, bearing) in bearings.iter().enumerate() {
        if let (Some(bearing), Some(off)) = (*bearing, bearing_at(long, gid)) {
            write_i16_be(mtx, off, clamp_i16(bearing));
        }
    }
}

// Each glyph's box from its glyf header; None for an empty glyph, of 0 contours or under its header, or one glyf
// doesn't hold.
pub(crate) fn glyf_bounds(glyf: &[u8], loca: &[usize], count: usize) -> Vec<Option<[i16; 4]>> {
    (0..count)
        .map(|gid| {
            let header: &[u8] = u32::try_from(gid).ok().and_then(|g| glyph_record(glyf, loca.into(), g))?.get(..10)?;
            let field = |i: usize| read_i16_be(header, 2 + 2 * i);
            (read_i16_be(header, 0)? != 0).then_some([field(0)?, field(1)?, field(2)?, field(3)?])
        })
        .collect()
}

// What an instance's glyf gives head and maxp over its glyphs: head's flag bit 1, set where every inked glyph's side
// bearing is its xMin, and maxp's six maxima over the inked glyphs.
pub(crate) struct GlyfCounts {
    lsb_at_x_min: bool,
    maxima: [usize; 6],
    extended: bool,
}

impl GlyfCounts {
    // `glyphs` holds each glyph's id with its box where inked, `hmtx` their side bearings; `extended` for GLYF. Each
    // count may visit MAX_VISITS glyphs, so all share 64 visits a byte of glyf and a glyph, of which real fonts spend
    // under 1%; None past them, leaving head and maxp as stored.
    pub(crate) fn of(
        glyphs: impl Iterator<Item = (usize, Option<[i16; 4]>)>, glyf: &[u8], extended: bool, loca: &[usize],
        hmtx: &MetricsTable,
    ) -> Option<GlyfCounts> {
        let mut counts = GlyfCounts { lsb_at_x_min: true, maxima: [0; 6], extended };
        let mut left = glyf.len().saturating_add(loca.len()).saturating_mul(64);
        let mut walk = (Stack::default(), ComponentPath::default());
        for (gid, bounds) in glyphs {
            let (Some(bounds), Ok(id)) = (bounds, u32::try_from(gid)) else { continue };
            counts.lsb_at_x_min &= hmtx.bearing(gid) == bounds[0];
            let mut visits = 0;
            counts.count(glyf, loca, id, &mut walk, &mut [0; 2], &mut visits);
            left = left.checked_sub(visits)?;
        }
        Some(counts)
    }

    // A glyph and the glyphs it's built from, each simple one's points and contours added to `below`. A component
    // already on its path is skipped, so a cycle is passed over, and the walk stops at the visits an assembled glyph
    // may take, each component read spending one as each glyph does. `walk`, empty between glyphs, holds composites.
    fn count<'a>(
        &mut self, glyf: &'a [u8], loca: &[usize], gid: u32, walk: &mut (Stack<Components<'a>, 8>, ComponentPath),
        below: &mut [usize; 2], visits: &mut usize,
    ) {
        let (inside, path) = walk;
        let (mut next, mut components, mut composite) = (Some(gid), 0, false);
        loop {
            if let Some(gid) = next.take() {
                let depth = path.depth();
                *visits += 1;
                if *visits <= MAX_VISITS {
                    self.maxima[5] = self.maxima[5].max(depth);
                    let glyph = glyph_record(glyf, loca.into(), gid).unwrap_or_default();
                    let contours = glyph.get(..10).and_then(|h| read_i16_be(h, 0)).unwrap_or(0);
                    if contours > 0 {
                        let n = usize::from(contours.unsigned_abs());
                        let points = simple_point_count(glyph, n).unwrap_or(0);
                        if depth == 0 {
                            self.maxima[0] = self.maxima[0].max(points);
                            self.maxima[1] = self.maxima[1].max(n);
                        }
                        below[0] += points;
                        below[1] += n;
                    } else if contours < 0 {
                        composite |= depth == 0;
                        path.enter(gid);
                        inside.push(Components::new(glyph, self.extended));
                    }
                }
            }
            let Some(records) = inside.last_mut() else { break };
            let depth = path.depth() - 1;
            let Some(Ok(c)) = records.next() else {
                inside.pop();
                path.leave();
                continue;
            };
            components += usize::from(depth == 0);
            *visits += 1;
            // Past the visits a component below the top is read no further; the top's are still counted.
            if *visits > MAX_VISITS {
                if depth > 0 {
                    inside.pop();
                    path.leave();
                }
            } else if !path.holds(c.gid) {
                next = Some(c.gid);
            }
        }
        if composite {
            self.maxima[2] = self.maxima[2].max(below[0]);
            self.maxima[3] = self.maxima[3].max(below[1]);
            self.maxima[4] = self.maxima[4].max(components);
        }
    }

    pub(crate) fn write_head(&self, head: &mut [u8]) {
        if let Some(flags) = read_u16_be(head, 16) {
            write_u16_be(head, 16, if self.lsb_at_x_min { flags | 2 } else { flags & !2 });
        }
    }

    // A version 1 maxp with the maxima written (MAXP's a byte on where `wide`), None where it has no room for them or
    // they change nothing.
    pub(crate) fn maxp(&self, maxp: &[u8], wide: bool) -> Option<Vec<u8>> {
        let end = 32 + usize::from(wide);
        let mut out = maxp.get(..end).filter(|m| read_u16_be(m, 0) == Some(1))?.to_vec();
        for (at, max) in [6, 8, 10, 12, 28, 30].into_iter().zip(self.maxima) {
            write_u16_be(&mut out, at + usize::from(wide), max.min(0xFFFF) as u16);
        }
        out.extend_from_slice(&maxp[end..]);
        (out != maxp).then_some(out)
    }
}

// post's header: version, italicAngle, the underline, isFixedPitch and the four memory hints.
const POST_HEADER: usize = 32;

const STRIPPED: &[&str] = &["fvar", "gvar", "GVAR", "HVAR", "MVAR", "avar", "cvar", "VVAR", "CFF2"];

// Every table both drivers instance alike after the outlines: metrics, MVAR, VORG, the style pass and extents over
// `bounds` where outlines changed; layout tables and COLR too unless described.
#[expect(clippy::too_many_arguments, reason = "the font, its verdicts, the location and what the driver built")]
pub(crate) fn finish<'a>(
    table_map: &'a Tables,
    reads: &Reads,
    location: &[f64],
    axis_values: &[(AxisTag, f64)],
    is_static: bool,
    metrics: Option<Metrics>,
    bounds: Option<&[Option<[i16; 4]>]>,
    instancing: Instancing,
) -> BTreeMap<String, Cow<'a, [u8]>> {
    let needs_var = location.iter().any(|&v| v != 0.0);
    let glyph_count = num_glyphs(table_map).map_or(0, |n| n as usize);
    let owned = |tag: &str| table_map.get(tag).map(TableBytes::to_owned_vec);
    let [h, v] = [false, true].map(|vertical| shaped_tags(table_map, vertical));
    let described = instancing == Instancing::Described;
    let (mut os2, mut hhea) = (owned("OS/2"), owned(h.hea));
    // A described location reads post's header alone, its glyph names read from the font's.
    let mut post = match described {
        true => table_map.get("post").map(|p| p[..p.len().min(POST_HEADER)].to_vec()),
        false => owned("post"),
    };
    let (mut vhea, mut gasp) = (owned(v.hea), owned("gasp"));
    // Beside the beyond-64K pair a hybrid's hhea and hmtx (vhea and vmtx) take the instance's metrics too, so either
    // pair reads as the instance.
    let lower = |tags: MetricTags, vertical: bool| {
        let lower = MetricTags::lower(vertical);
        (tags.wide && table_map.contains_key(lower.mtx)).then(|| owned(lower.hea)).flatten()
    };
    let (mut lower_hhea, mut lower_vhea) = (lower(h, false), lower(v, true));
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
        if lower_hhea.is_some() || lower_vhea.is_some() {
            let targets = MvarTargets {
                hhea: lower_hhea.as_deref_mut().unwrap_or_default(),
                vhea: lower_vhea.as_deref_mut().unwrap_or_default(),
                os2: &mut [],
                post: &mut [],
                gasp: &mut [],
            };
            let _ = apply_mvar(table_map, targets, location);
        }
    }
    let style = (!is_static).then(|| {
        style::apply_style_metadata(
            table_map,
            axis_values,
            os2.as_deref_mut().unwrap_or_default(),
            post.as_deref_mut().unwrap_or_default(),
            !described,
        )
    });

    let font = |tag: &str| table_map.get(tag).map(|t| &t[..]);
    let layout = reads.reading(font);
    // A described location holds what it doesn't vary as the font's kept tables, so it's given only what it may.
    let mut out: BTreeMap<String, Cow<[u8]>> = table_map
        .iter()
        .filter(|(tag, _)| !STRIPPED.contains(tag) && layout(tag).is_some() && (!described || described_varies(tag)))
        .map(|(tag, data)| (String::from(tag), Cow::Borrowed(data.as_slice())))
        .collect();
    // A hybrid's maxp counts the instance's glyphs too, MAXP's count capped at 65,535, the most its 16 bits hold.
    if table_map.contains_key("MAXP") && let Some(maxp) = out.get_mut("maxp").filter(|m| m.len() >= 6) {
        let count = glyph_count.min(0xFFFF) as u16;
        if read_u16_be(maxp, 4) != Some(count) {
            write_u16_be(maxp.to_mut(), 4, count);
        }
    }
    if !described {
        // No GDEF the sanitizer rejects is read at any location, so its store varies nothing.
        let read_gdef = layout("GDEF");
        // A whole instance's at the default too, as GDEF's store goes: its VariationIndex tables would name nothing.
        let varies = needs_var || instancing == Instancing::Whole;
        let patched = varies.then(|| apply_gpos_var(layout("GPOS")?, read_gdef?, location)).flatten();
        let store = read_gdef.and_then(super::subsetter::otl::gdef::var_store);
        let rebuilt = [
            ("GPOS", resolve_layout(layout("GPOS"), patched, location, store.as_ref())),
            ("GSUB", resolve_layout(layout("GSUB"), None, location, store.as_ref())),
            ("GDEF", read_gdef.and_then(|t| bake_gdef(t, location))),
            ("BASE", layout("BASE").and_then(|t| bake_base(t, location))),
            ("MATH", layout("MATH").and_then(|t| bake_math(t, store.as_ref(), location))),
            ("COLR", layout("COLR").and_then(|t| super::colr_v1::instance_colr_v1(t, location))),
        ];
        for (tag, table) in rebuilt {
            if let Some(table) = table { out.insert(tag.to_string(), Cow::Owned(table)); }
        }
        // Such a GDEF's instance keeps only its mark glyph sets, which filter by none.
        if let Some(gdef) = font("GDEF").filter(|_| read_gdef.is_none()) {
            match sets_of_every_glyph(mark_glyph_set_count(gdef), glyph_count) {
                Some(sets) => out.insert("GDEF".to_string(), Cow::Owned(sets)),
                None => out.remove("GDEF"),
            };
        }
        super::format::sanitize::read_as_judged(reads, &mut out);
    }
    if let Some(vorg) = (needs_var && metrics.is_some()).then(|| hvar::vary_vorg(table_map, location)).flatten() {
        out.insert("VORG".to_string(), Cow::Owned(vorg));
    }

    let (mut head, name, stat) = match style {
        Some(style) => (style.head, style.name, style.stat),
        None => (None, None, None),
    };
    // A described location builds its names on the first read there (`described_name`), so most hold none.
    if described {
        out.remove("name");
    }
    let mut lower_mtx = [None, None];
    if let Some(metrics) = &metrics {
        if let (Some(vhea), Some(_)) = (vhea.as_mut(), &metrics.vmtx) {
            v.write_long(vhea, metrics.v_long);
        }
        if let Some(hhea) = hhea.as_mut() {
            h.write_long(hhea, metrics.h_long);
        }
        let sides = [
            (false, Some(&metrics.hmtx), metrics.h_long, &mut lower_hhea),
            (true, metrics.vmtx.as_ref(), metrics.v_long, &mut lower_vhea),
        ];
        for (slot, (vertical, mtx, long, hea)) in lower_mtx.iter_mut().zip(sides) {
            let (Some(mtx), Some(hea)) = (mtx, hea.as_mut()) else { continue };
            let (mtx, long) = narrowed(mtx.clone(), long);
            MetricTags::lower(vertical).write_long(hea, long);
            *slot = Some(mtx);
        }
        if let Some(bounds) = bounds {
            let head = head.get_or_insert_with(|| owned("head").unwrap_or_default());
            let vmtx = metrics.vmtx.as_deref().map(|v| MetricsTable::new(v, metrics.v_long));
            let (glyphs, hmtx) = (bounds.iter().copied().enumerate(), MetricsTable::new(&metrics.hmtx, metrics.h_long));
            let (hea, vea, os) = (hhea.as_deref_mut(), vhea.as_deref_mut(), os2.as_deref_mut());
            recompute_extents(glyphs.clone(), &hmtx, vmtx.as_ref(), head, hea, vea, os);
            if lower_hhea.is_some() || lower_vhea.is_some() {
                let (hea, vea) = (lower_hhea.as_deref_mut(), lower_vhea.as_deref_mut());
                recompute_extents(glyphs, &hmtx, vmtx.as_ref(), &mut [], hea, vea, None);
            }
        }
    }

    // A post written as it was, its header alone or whole, leaves the font's.
    let post = post.filter(|post| !font("post").is_some_and(|font| font.starts_with(post)));
    let (hmtx, vmtx) = metrics.map_or((None, None), |m| (Some(m.hmtx), m.vmtx));
    let [lower_hmtx, lower_vmtx] = lower_mtx;
    let tables = [(h.mtx, hmtx), (v.mtx, vmtx), (v.hea, vhea), (h.hea, hhea),
        ("hmtx", lower_hmtx), ("vmtx", lower_vmtx), ("hhea", lower_hhea), ("vhea", lower_vhea),
        ("OS/2", os2), ("post", post), ("gasp", gasp), ("head", head), ("name", name), ("STAT", stat)];
    for (tag, table) in tables {
        if let Some(table) = table.filter(|t| !t.is_empty()) {
            out.insert(tag.to_string(), shared_if_same(table_map, tag, table));
        }
    }
    out
}

// head's box, hhea's and vhea's extremes and OS/2's average width over `glyphs` (ids with their inked boxes). Uninked
// glyphs count only toward advances; with none inked, head's box is 0.
pub(crate) fn recompute_extents(
    glyphs: impl Iterator<Item = (usize, Option<[i16; 4]>)> + Clone,
    hmtx: &MetricsTable,
    vmtx: Option<&MetricsTable>,
    head: &mut [u8],
    hhea: Option<&mut [u8]>,
    vhea: Option<&mut [u8]>,
    os2: Option<&mut [u8]>,
) {
    let inked = || glyphs.clone().filter_map(|(gid, b)| b.map(|b| (gid, b.map(i32::from))));
    if head.len() >= 44 {
        let union = |a: [i32; 4], b: [i32; 4]| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])];
        let bbox = inked().map(|(_, b)| b).reduce(union).unwrap_or_default();
        for (i, v) in bbox.into_iter().enumerate() { write_i16_be(head, 36 + 2 * i, v as i16); }
    }
    // hhea's over x and vhea's over y, each box's low edge at `low` and its high one two past it.
    for (hea, mtx, low) in [(hhea, Some(hmtx), 0), (vhea, vmtx, 1)] {
        let (Some(hea), Some(mtx)) = (hea.filter(|h| h.len() >= 18), mtx) else { continue };
        write_u16_be(hea, 10, glyphs.clone().map(|(g, _)| mtx.advance(g)).max().unwrap_or(0));
        let extremes = inked().map(|(gid, b)| {
            let (advance, bearing) = mtx.pair(gid);
            let extent = i32::from(bearing) + b[low + 2] - b[low];
            (i32::from(bearing), i32::from(advance) - extent, extent)
        });
        if let Some((lead, trail, extent)) = extremes.reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2))) {
            for (at, v) in [(12, lead), (14, trail), (16, extent)] { write_i16_be(hea, at, clamp_i16(v)); }
        }
    }
    // Over every glyph with a non-zero advance, in any version.
    if let Some(os2) = os2.filter(|o| o.len() >= 4) {
        let widths = glyphs.map(|(g, _)| u64::from(hmtx.advance(g))).filter(|&w| w > 0);
        let (sum, count) = widths.fold((0, 0u64), |(sum, count), w| (sum + w, count + 1));
        write_i16_be(os2, 2, if count == 0 { 0 } else { clamp_i16(ot_round(sum as f64 / count as f64)) });
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/mod.rs"]
mod tests;
