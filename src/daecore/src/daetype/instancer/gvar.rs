use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{pad4, read_u16_be, read_u32_be, read_i16_be};
use super::super::format::ivs::axis_factor;
use super::super::format::round::ot_round;
use super::coords::{GlyphCoords, extract_coords_into, iup, apply_simple_glyph_deltas, count_composite_components, apply_composite_glyph_deltas};
use crate::daecore::daetype::TableBytes;

const MAX_INSTANCED_GLYF_SIZE: usize = 1024 * 1024 * 1024;

pub struct GvarResult {
    pub glyf_data: Vec<u8>,
    pub new_loca:  Vec<usize>,
    pub advance_deltas: Vec<f64>,
    pub lsb_new: Vec<Option<i32>>,
    pub vadvance_deltas: Vec<f64>,
    pub tsb_new: Vec<Option<i32>>,
}

fn read_side_bearing(mtx: &[u8], long_metrics: usize, gid: usize) -> i32 {
    super::super::subsetter::metric_pair(mtx, long_metrics, 0, gid).1 as i32
}

pub fn apply_gvar(
    table_map:     &BTreeMap<String, TableBytes>,
    glyf_data:     &[u8],
    glyph_offsets: &[usize],
    num_glyphs:    usize,
    location:      &[f64],
    axis_count:    usize,
) -> Result<GvarResult, String> {
    let gvar = table_map.get("gvar").ok_or("missing gvar")?;

    let gvar_axis_count    = read_u16_be(gvar, 4).ok_or("gvar: header truncated")? as usize;
    let shared_tuple_count = read_u16_be(gvar, 6).ok_or("gvar: header truncated")? as usize;
    let shared_tuples_off  = read_u32_be(gvar, 8).ok_or("gvar: header truncated")? as usize;
    let glyph_count        = read_u16_be(gvar, 12).ok_or("gvar: header truncated")? as usize;
    let flags              = read_u16_be(gvar, 14).ok_or("gvar: header truncated")?;
    let var_array_off      = read_u32_be(gvar, 16).ok_or("gvar: header truncated")? as usize;
    // The spec requires both to match, and FreeType refuses a gvar where they do not.
    if gvar_axis_count != axis_count || glyph_count != num_glyphs {
        return Err("gvar: axis or glyph count differs from fvar's and maxp's".into());
    }

    if shared_tuple_count > 0 {
        let shared_bytes = shared_tuple_count
            .checked_mul(gvar_axis_count)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| shared_tuples_off.checked_add(n));
        if shared_bytes.is_none_or(|end| end > gvar.len()) {
            return Err("gvar: shared tuple array does not fit the table".into());
        }
    }

    let mut shared_tuples: Vec<Vec<f64>> = Vec::with_capacity(shared_tuple_count);
    for i in 0..shared_tuple_count {
        let mut coords = Vec::with_capacity(gvar_axis_count);
        for j in 0..gvar_axis_count {
            let v = read_i16_be(gvar, shared_tuples_off + (i * gvar_axis_count + j) * 2)
                .ok_or("gvar: shared tuple truncated")?;
            coords.push(v as f64 / 16384.0);
        }
        shared_tuples.push(coords);
    }

    let use_words = (flags & 1) != 0;
    let glyph_var_offset = |gid: usize| -> Option<usize> {
        if use_words {
            read_u32_be(gvar, 20 + gid * 4).map(|v| v as usize)
        } else {
            read_u16_be(gvar, 20 + gid * 2).map(|v| v as usize * 2)
        }
    };

    let metrics = |mtx: &str, header: &str| {
        table_map.get(mtx).zip(table_map.get(header).and_then(|h| read_u16_be(h, 34)).map(usize::from))
    };
    let horizontal = metrics("hmtx", "hhea");
    let vertical = metrics("vmtx", "vhea");

    let mut state = GlyphVariation {
        gvar,
        glyf: glyf_data,
        location,
        axis_count,
        gvar_axis_count,
        var_array_off,
        shared_tuple_scalars: vec![None; shared_tuple_count],
        shared_tuples,
        font_work_left: gvar.len().saturating_mul(64),
        buffers: Buffers::default(),
    };

    let mut new_glyf: Vec<u8> = Vec::with_capacity(glyf_data.len() + glyf_data.len() / 8);
    let mut new_loca: Vec<usize>          = vec![0; num_glyphs + 1];
    let mut advance_deltas: Vec<f64>      = vec![0.0; num_glyphs];
    let mut lsb_new: Vec<Option<i32>>     = vec![None; num_glyphs];
    let mut vadvance_deltas: Vec<f64>     = vec![0.0; num_glyphs];
    let mut tsb_new: Vec<Option<i32>>     = vec![None; num_glyphs];
    let mut composites: Vec<(usize, f64, f64)> = Vec::new();

    for gid in 0..num_glyphs {
        let glyph_start  = glyph_offsets[gid];
        let glyph_end    = glyph_offsets[gid + 1];
        let var_off      = glyph_var_offset(gid).ok_or("gvar: glyph variation offset truncated")?;
        let var_off_next = glyph_var_offset(gid + 1).ok_or("gvar: glyph variation offset truncated")?;

        let at = new_glyf.len();
        let varied = if var_off == var_off_next {
            None
        } else {
            match state.vary(glyph_start, glyph_end, var_off, &mut new_glyf) {
                Ok(varied) => varied,
                Err(GlyphError::Budget(e)) => return Err(e),
                // One glyph's bad variation data leaves that glyph at its default, as FreeType does.
                Err(GlyphError::Malformed) => None,
            }
        };
        match varied {
            None => {
                let raw = glyf_data.get(glyph_start..glyph_end).ok_or("gvar: glyph data range out of bounds")?;
                new_glyf.extend_from_slice(raw);
            }
            Some(v) => {
                // The advance runs between the two horizontal phantom points, so it moves by their difference.
                advance_deltas[gid] = v.phantom[1].0 - v.phantom[0].0;
                vadvance_deltas[gid] = v.phantom[2].1 - v.phantom[3].1;
                if v.composite {
                    composites.push((gid, v.phantom[0].0, v.phantom[2].1));
                } else {
                    let (lsb, tsb) = bearings(glyf_data, glyph_start, &new_glyf[at..], gid, horizontal, vertical, v.phantom[0].0, v.phantom[2].1);
                    lsb_new[gid] = lsb;
                    tsb_new[gid] = tsb;
                }
            }
        }
        new_glyf.resize(pad4(new_glyf.len()), 0);
        new_loca[gid + 1] = new_glyf.len();
        if new_glyf.len() > MAX_INSTANCED_GLYF_SIZE {
            return Err("gvar: instanced glyf size exceeds sanity limit".to_string());
        }
    }

    recompute_composite_bboxes(&mut new_glyf, &new_loca, num_glyphs);
    // A composite's box is known only once every component is, so its bearings come last.
    for (gid, pp1, pp3) in composites {
        let glyph = new_glyf.get(new_loca[gid]..new_loca[gid + 1]).unwrap_or_default();
        let (lsb, tsb) = bearings(glyf_data, glyph_offsets[gid], glyph, gid, horizontal, vertical, pp1, pp3);
        lsb_new[gid] = lsb;
        tsb_new[gid] = tsb;
    }

    Ok(GvarResult { glyf_data: new_glyf, new_loca, advance_deltas, lsb_new, vadvance_deltas, tsb_new })
}

// A varied glyph's bearings: the new box against phantom points 1 and 3, each moved by its delta.
#[allow(clippy::too_many_arguments, reason = "the glyph's old and new forms, both metrics tables and both phantom deltas")]
fn bearings(
    glyf: &[u8],
    glyph_start: usize,
    varied: &[u8],
    gid: usize,
    horizontal: Option<(&TableBytes, usize)>,
    vertical: Option<(&TableBytes, usize)>,
    pp1: f64,
    pp3: f64,
) -> (Option<i32>, Option<i32>) {
    let field = |data: &[u8], at: usize| read_i16_be(data, at).map(i32::from);
    let lsb = horizontal.and_then(|(hmtx, long)| {
        let (old, new) = (field(glyf, glyph_start + 2)?, field(varied, 2)?);
        Some(new - old + read_side_bearing(hmtx, long, gid) - ot_round(pp1))
    });
    let tsb = vertical.and_then(|(vmtx, long)| {
        let (old, new) = (field(glyf, glyph_start + 8)?, field(varied, 8)?);
        Some(old - new + read_side_bearing(vmtx, long, gid) + ot_round(pp3))
    });
    (lsb, tsb)
}

struct Varied {
    phantom: [(f64, f64); 4],
    composite: bool,
}

enum GlyphError {
    Budget(String),
    Malformed,
}

#[derive(Default)]
struct Buffers {
    xr: Vec<f64>,
    yr: Vec<f64>,
    dx: Vec<f64>,
    dy: Vec<f64>,
    fdx: Vec<f64>,
    fdy: Vec<f64>,
    coords: GlyphCoords,
    flags: Vec<u8>,
    touched: Vec<bool>,
    order: Vec<usize>,
    peak: Vec<f64>,
    start: Vec<f64>,
    end: Vec<f64>,
    shared_points: Vec<usize>,
    private_points: Vec<usize>,
}

struct GlyphVariation<'a> {
    gvar: &'a [u8],
    glyf: &'a [u8],
    location: &'a [f64],
    axis_count: usize,
    gvar_axis_count: usize,
    var_array_off: usize,
    shared_tuples: Vec<Vec<f64>>,
    shared_tuple_scalars: Vec<Option<f64>>,
    font_work_left: usize,
    buffers: Buffers,
}

impl GlyphVariation<'_> {
    // The varied glyph goes onto the end of `out`, which is left alone where it errs.
    fn vary(&mut self, glyph_start: usize, glyph_end: usize, var_off: usize, out: &mut Vec<u8>) -> Result<Option<Varied>, GlyphError> {
        let (gvar, glyf) = (self.gvar, self.glyf);
        let (n_contours, num_points) = if glyph_start == glyph_end {
            (0i16, 0usize)
        } else {
            let nc = read_i16_be(glyf, glyph_start).ok_or(GlyphError::Malformed)?;
            match nc {
                1.. => {
                    let last = read_u16_be(glyf, glyph_start + 10 + (nc as usize - 1) * 2).ok_or(GlyphError::Malformed)?;
                    (nc, usize::from(last) + 1)
                }
                ..=-1 => (nc, count_composite_components(glyf, glyph_start, glyph_end)),
                0 => (0, 0),
            }
        };

        let total_points = num_points + 4;
        let gvd_base = self.var_array_off.checked_add(var_off).ok_or(GlyphError::Malformed)?;
        let raw_tuple_count = read_u16_be(gvar, gvd_base).ok_or(GlyphError::Malformed)?;
        let has_shared_pts = (raw_tuple_count & 0x8000) != 0;
        let tup_count = (raw_tuple_count & 0x0FFF) as usize;
        let serialized_start = gvd_base + read_u16_be(gvar, gvd_base + 2).ok_or(GlyphError::Malformed)? as usize;

        let b = &mut self.buffers;
        let mut serialized_pos = serialized_start;
        let mut shared_points = false;
        if has_shared_pts {
            (shared_points, serialized_pos) = parse_packed_points_into(gvar, serialized_pos, &mut b.shared_points);
        }

        b.dx.clear(); b.dx.resize(total_points, 0.0);
        b.dy.clear(); b.dy.resize(total_points, 0.0);
        if n_contours > 0 {
            extract_coords_into(glyf, glyph_start, n_contours as usize, &mut b.coords);
        }

        let mut header_pos       = gvd_base + 4;
        let mut private_data_pos = serialized_pos;

        const MAX_TUPLE_POINT_WORK: usize = 16_777_216;
        let mut tuple_work_left = MAX_TUPLE_POINT_WORK;

        for _ in 0..tup_count {
            let var_data_size    = read_u16_be(gvar, header_pos).ok_or(GlyphError::Malformed)? as usize;
            let tuple_index_word = read_u16_be(gvar, header_pos + 2).ok_or(GlyphError::Malformed)?;
            header_pos += 4;

            let has_peak         = (tuple_index_word & 0x8000) != 0;
            let has_intermediate = (tuple_index_word & 0x4000) != 0;
            let has_private_pts  = (tuple_index_word & 0x2000) != 0;
            let shared_idx       = (tuple_index_word & 0x0FFF) as usize;

            let peak: &[f64] = if has_peak {
                read_tuple(gvar, &mut header_pos, self.gvar_axis_count, &mut b.peak)?;
                &b.peak
            } else {
                self.shared_tuples.get(shared_idx).ok_or(GlyphError::Malformed)?
            };
            if has_intermediate {
                read_tuple(gvar, &mut header_pos, self.gvar_axis_count, &mut b.start)?;
                read_tuple(gvar, &mut header_pos, self.gvar_axis_count, &mut b.end)?;
            }

            let scalar = if !has_peak && !has_intermediate {
                *self.shared_tuple_scalars[shared_idx]
                    .get_or_insert_with(|| compute_tuple_scalar(self.location, peak, None, None, self.axis_count))
            } else {
                let (start, end) = if has_intermediate { (Some(&b.start[..]), Some(&b.end[..])) } else { (None, None) };
                compute_tuple_scalar(self.location, peak, start, end, self.axis_count)
            };
            let tuple_end = private_data_pos + var_data_size;

            if scalar.abs() > 1e-10 {
                tuple_work_left = tuple_work_left
                    .checked_sub(total_points.max(1))
                    .ok_or_else(|| GlyphError::Budget("gvar: per-glyph tuple work budget exhausted".into()))?;
                self.font_work_left = self.font_work_left
                    .checked_sub(total_points.max(1))
                    .ok_or_else(|| GlyphError::Budget("gvar: font-wide tuple work budget exhausted".into()))?;
                let points = if has_private_pts {
                    let named;
                    (named, private_data_pos) = parse_packed_points_into(gvar, private_data_pos, &mut b.private_points);
                    named.then_some(&b.private_points[..])
                } else {
                    shared_points.then_some(&b.shared_points[..])
                };

                let n_delta = points.map_or(total_points, |p| p.len());
                private_data_pos = parse_packed_deltas_into(gvar, private_data_pos, n_delta, &mut b.xr);
                parse_packed_deltas_into(gvar, private_data_pos, n_delta, &mut b.yr);

                match points {
                    None => {
                        for i in 0..total_points {
                            b.dx[i] += scalar * b.xr[i];
                            b.dy[i] += scalar * b.yr[i];
                        }
                    }
                    Some(pts) => {
                        b.fdx.clear(); b.fdx.resize(total_points, 0.0);
                        b.fdy.clear(); b.fdy.resize(total_points, 0.0);
                        // A point named twice takes both deltas, as the spec requires.
                        for (i, &p) in pts.iter().enumerate() {
                            if p < total_points {
                                b.fdx[p] += b.xr[i];
                                b.fdy[p] += b.yr[i];
                            }
                        }
                        if n_contours > 0 {
                            iup(&mut b.fdx, &mut b.fdy, pts, &b.coords, &mut b.touched, &mut b.order);
                        }
                        for i in 0..total_points {
                            b.dx[i] += scalar * b.fdx[i];
                            b.dy[i] += scalar * b.fdy[i];
                        }
                    }
                }
            }
            private_data_pos = tuple_end;
        }

        let phantom: [(f64, f64); 4] = core::array::from_fn(|k| (b.dx[num_points + k], b.dy[num_points + k]));
        if n_contours > 0 {
            apply_simple_glyph_deltas(glyf, glyph_start, glyph_end, n_contours as usize, &b.dx, &b.dy, &mut b.coords, &mut b.flags, out);
        } else if n_contours < 0 {
            apply_composite_glyph_deltas(glyf, glyph_start, glyph_end, &b.dx, &b.dy, num_points, out);
        } else {
            out.extend_from_slice(glyf.get(glyph_start..glyph_end).ok_or(GlyphError::Malformed)?);
        }
        Ok(Some(Varied { phantom, composite: n_contours < 0 }))
    }
}

#[derive(Default)]
struct BboxPen {
    min: (f32, f32),
    max: (f32, f32),
    any: bool,
}

impl BboxPen {
    fn add(&mut self, x: f32, y: f32) {
        if !self.any {
            self.any = true;
            self.min = (x, y);
            self.max = (x, y);
            return;
        }
        self.min = (self.min.0.min(x), self.min.1.min(y));
        self.max = (self.max.0.max(x), self.max.1.max(y));
    }
}

impl crate::daecore::daetype::outline::OutlinePen for BboxPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.add(cx, cy);
        self.add(x, y);
    }
    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        self.add(c1x, c1y);
        self.add(c2x, c2y);
        self.add(x, y);
    }
    fn close(&mut self) {}
}

fn recompute_composite_bboxes(glyf: &mut [u8], loca: &[usize], num_glyphs: usize) {
    use crate::daecore::daetype::decoder::write_i16_be;

    let mut scratch = GlyphCoords::default();
    for gid in 0..num_glyphs {
        let (Some(&start), Some(&end)) = (loca.get(gid), loca.get(gid + 1)) else { break };
        if end.saturating_sub(start) < 10 {
            continue;
        }
        if read_i16_be(glyf, start).is_none_or(|n| n >= 0) {
            continue;
        }

        let mut pen = BboxPen::default();
        if crate::daecore::daetype::outline::outline_glyf_glyph_reusing_bytes(glyf, loca, gid as u16, &mut scratch, &mut pen).is_err() {
            continue;
        }
        if !pen.any {
            continue;
        }
        let clamp = |v: f32| ot_round(f64::from(v)).clamp(i16::MIN.into(), i16::MAX.into()) as i16;
        write_i16_be(glyf, start + 2, clamp(pen.min.0));
        write_i16_be(glyf, start + 4, clamp(pen.min.1));
        write_i16_be(glyf, start + 6, clamp(pen.max.0));
        write_i16_be(glyf, start + 8, clamp(pen.max.1));
    }
}

pub(crate) fn compute_tuple_scalar(
    location:    &[f64],
    peak:        &[f64],
    start_tuple: Option<&[f64]>,
    end_tuple:   Option<&[f64]>,
    axis_count:  usize,
) -> f64 {
    let mut scalar = 1.0f64;
    for i in 0..axis_count.min(peak.len()) {
        let p = peak[i];
        // With no intermediate region, a tuple reaches from the default to its peak and no further.
        let start = start_tuple.map_or(p.min(0.0), |s| s[i]);
        let end   = end_tuple.map_or(p.max(0.0), |e| e[i]);
        scalar *= axis_factor(location[i], start, p, end);
        if scalar == 0.0 {
            return 0.0;
        }
    }
    scalar
}

// One tuple record's coordinates.
fn read_tuple(gvar: &[u8], at: &mut usize, axes: usize, out: &mut Vec<f64>) -> Result<(), GlyphError> {
    out.clear();
    for _ in 0..axes {
        out.push(f64::from(read_i16_be(gvar, *at).ok_or(GlyphError::Malformed)?) / 16384.0);
        *at += 2;
    }
    Ok(())
}

pub(crate) fn parse_packed_points(buf: &[u8], pos: usize) -> (Option<Vec<usize>>, usize) {
    let mut points = Vec::new();
    let (named, next) = parse_packed_points_into(buf, pos, &mut points);
    (named.then_some(points), next)
}

// Every point number as encoded, out of range or repeated ones included: the deltas that follow are
// counted by them, and the caller drops a number past its points. False where the data means all points.
pub(crate) fn parse_packed_points_into(buf: &[u8], pos: usize, points: &mut Vec<usize>) -> (bool, usize) {
    points.clear();
    let mut pos = pos;
    let mut count = match buf.get(pos) {
        Some(&b) => { pos += 1; b as usize }
        None => return (false, pos),
    };
    if count == 0 { return (false, pos); }
    if count & 0x80 != 0 {
        let next = match buf.get(pos) {
            Some(&b) => { pos += 1; b as usize }
            None => return (false, pos),
        };
        count = ((count & 0x7F) << 8) | next;
    }

    points.reserve(count);
    let mut idx = 0usize;
    while points.len() < count {
        let ctrl = match buf.get(pos) {
            Some(&b) => { pos += 1; b }
            None => break,
        };
        let words = (ctrl & 0x80) != 0;
        let len   = (ctrl & 0x7F) as usize + 1;
        for _ in 0..len {
            if points.len() >= count { break; }
            if words {
                let hi = buf.get(pos).copied().unwrap_or(0) as usize;
                let lo = buf.get(pos + 1).copied().unwrap_or(0) as usize;
                pos += 2;
                idx += (hi << 8) | lo;
            } else {
                idx += buf.get(pos).copied().unwrap_or(0) as usize;
                pos += 1;
            }
            points.push(idx);
        }
    }
    (true, pos)
}

pub(crate) fn parse_packed_deltas(buf: &[u8], pos: usize, count: usize) -> (Vec<f64>, usize) {
    let mut deltas = Vec::new();
    let next = parse_packed_deltas_into(buf, pos, count, &mut deltas);
    (deltas, next)
}

pub(crate) fn parse_packed_deltas_into(buf: &[u8], pos: usize, count: usize, deltas: &mut Vec<f64>) -> usize {
    deltas.clear();
    deltas.resize(count, 0.0);
    let mut i = 0;
    let mut pos = pos;
    'outer: while i < count {
        let ctrl = match buf.get(pos) { Some(&b) => { pos += 1; b } None => break };
        let len  = (ctrl & 0x3F) as usize + 1;
        if ctrl & 0x80 != 0 {
            i += len;
        } else if ctrl & 0x40 != 0 {
            for _ in 0..len {
                if i >= count { break; }
                let hi = match buf.get(pos) { Some(&b) => { pos += 1; b as u16 } None => break 'outer };
                let lo = match buf.get(pos) { Some(&b) => { pos += 1; b as u16 } None => break 'outer };
                let raw = (hi << 8) | lo;
                deltas[i] = if raw >= 0x8000 { raw as f64 - 0x10000 as f64 } else { raw as f64 };
                i += 1;
            }
        } else {
            for _ in 0..len {
                if i >= count { break; }
                let raw = match buf.get(pos) { Some(&b) => { pos += 1; b } None => break 'outer };
                deltas[i] = if raw >= 0x80 { raw as f64 - 0x100 as f64 } else { raw as f64 };
                i += 1;
            }
        }
    }
    pos
}

#[cfg(test)]
mod tests {
    use super::compute_tuple_scalar;

    // Values from fontTools 4.63, which HarfBuzz and FreeType agree with.
    #[test]
    fn a_tuple_reaches_from_the_default_to_its_peak() {
        assert_eq!(compute_tuple_scalar(&[0.25], &[0.5], None, None, 1), 0.5);
        assert_eq!(compute_tuple_scalar(&[0.75], &[0.5], None, None, 1), 0.0);
        assert_eq!(compute_tuple_scalar(&[-0.75], &[-0.5], None, None, 1), 0.0);
    }

    #[test]
    fn a_malformed_intermediate_region_ignores_its_axis() {
        assert_eq!(compute_tuple_scalar(&[0.75], &[0.25], Some(&[0.5]), Some(&[1.0]), 1), 1.0);
        assert_eq!(compute_tuple_scalar(&[0.25], &[1.0], Some(&[-1.0]), Some(&[1.0]), 1), 1.0);
        assert_eq!(compute_tuple_scalar(&[0.75], &[0.5], Some(&[0.25]), Some(&[1.0]), 1), 0.5);
    }
}
