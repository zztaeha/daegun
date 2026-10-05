use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, read_i16_be, write_i16_be};
use super::super::format::round::ot_round;
use super::super::format::glyf::{scale_len, ARGS_ARE_XY_VALUES, ARG_1_AND_2_ARE_WORDS, MORE_COMPONENTS};

#[derive(Default)]
pub struct GlyphCoords {
    pub x_coords:   Vec<i32>,
    pub y_coords:   Vec<i32>,
    pub flags:      Vec<u8>,
    pub end_pts:    Vec<usize>,
    pub num_points: usize,
}

impl GlyphCoords {
    fn reset(&mut self) {
        self.x_coords.clear();
        self.y_coords.clear();
        self.flags.clear();
        self.end_pts.clear();
        self.num_points = 0;
    }
}

// A simple glyph's points. False, with nothing kept, where the end points do not increase or the
// flags or coordinates run out before the points do.
pub fn extract_coords_into(data: &[u8], start: usize, n_contours: usize, out: &mut GlyphCoords) -> bool {
    out.reset();
    let empty = |out: &mut GlyphCoords| { out.reset(); false };
    out.end_pts.reserve(n_contours);
    for i in 0..n_contours {
        let off = start + 10 + i * 2;
        let v = match read_u16_be(data, off) { Some(v) => v, None => return empty(out) };
        // `iup` indexes by these, so a non-monotonic sequence would panic rather than skip the glyph.
        if let Some(&prev) = out.end_pts.last()
            && v as usize <= prev { return empty(out); }
        out.end_pts.push(v as usize);
    }
    let num_points = out.end_pts.last().map_or(0, |&e| e + 1);

    let instr_len_off = start + 10 + n_contours * 2;
    let instr_len = match read_u16_be(data, instr_len_off) { Some(v) => v as usize, None => return empty(out) };
    let mut pos = instr_len_off + 2 + instr_len;

    out.flags.reserve(num_points);
    while out.flags.len() < num_points {
        let f = match data.get(pos) { Some(&b) => { pos += 1; b } None => break };
        out.flags.push(f);
        if f & 0x08 != 0 {
            let r = match data.get(pos) { Some(&b) => { pos += 1; b as usize } None => break };
            let take = r.min(num_points - out.flags.len());
            out.flags.resize(out.flags.len() + take, f);
        }
    }
    if out.flags.len() < num_points { return empty(out); }

    out.x_coords.resize(num_points, 0);
    {
        let flags = &out.flags[..num_points];
        let xs = &mut out.x_coords[..num_points];
        let mut cur = 0i32;
        for (x, &f) in xs.iter_mut().zip(flags) {
            if f & 0x02 != 0 {
                let Some(&b) = data.get(pos) else { return empty(out) };
                pos += 1;
                cur = cur.saturating_add(if f & 0x10 != 0 { b as i32 } else { (b as i32).saturating_neg() });
            } else if f & 0x10 == 0 {
                let Some(v) = read_i16_be(data, pos) else { return empty(out) };
                cur = cur.saturating_add(v as i32); pos += 2;
            }
            *x = cur;
        }
    }

    out.y_coords.resize(num_points, 0);
    {
        let flags = &out.flags[..num_points];
        let ys = &mut out.y_coords[..num_points];
        let mut cur = 0i32;
        for (y, &f) in ys.iter_mut().zip(flags) {
            if f & 0x04 != 0 {
                let Some(&b) = data.get(pos) else { return empty(out) };
                pos += 1;
                cur = cur.saturating_add(if f & 0x20 != 0 { b as i32 } else { (b as i32).saturating_neg() });
            } else if f & 0x20 == 0 {
                let Some(v) = read_i16_be(data, pos) else { return empty(out) };
                cur = cur.saturating_add(v as i32); pos += 2;
            }
            *y = cur;
        }
    }

    out.num_points = num_points;
    true
}

// Moves the points a tuple leaves untouched between the touched ones around them, contour by contour.
// `touched` and `order` are scratch.
pub fn iup(
    dx: &mut [f64], dy: &mut [f64],
    touched_points: &[usize],
    cc: &GlyphCoords,
    touched: &mut Vec<bool>, order: &mut Vec<usize>,
) {
    touched.clear();
    touched.resize(cc.num_points, false);
    for &p in touched_points { if p < cc.num_points { touched[p] = true; } }

    let mut start = 0;
    for &end in &cc.end_pts {
        let n = end + 1 - start;
        order.clear();
        if let Some(first) = (start..=end).find(|&i| touched[i]) {
            order.extend((0..n).map(|step| (first - start + step) % n + start).filter(|&i| touched[i]));
            iup_axis(dx, &cc.x_coords, order, start, n);
            iup_axis(dy, &cc.y_coords, order, start, n);
        }
        start = end + 1;
    }
}

// `touched_pos` lists a contour's touched points in order, starting from its first.
fn iup_axis(delta: &mut [f64], coords: &[i32], touched_pos: &[usize], start: usize, n: usize) {
    for t in 0..touched_pos.len() {
        let prev_idx   = touched_pos[t];
        let next_idx   = touched_pos[(t + 1) % touched_pos.len()];
        let prev_delta = delta[prev_idx];
        let next_delta = delta[next_idx];
        let prev_coord = coords[prev_idx] as f64;
        let next_coord = coords[next_idx] as f64;

        let mut cur = (prev_idx - start + 1) % n + start;
        while cur != next_idx {
            let cur_coord = coords[cur] as f64;
            delta[cur] = if prev_coord == next_coord {
                if prev_delta == next_delta { prev_delta } else { 0.0 }
            } else if cur_coord <= prev_coord.min(next_coord) {
                if prev_coord < next_coord { prev_delta } else { next_delta }
            } else if cur_coord >= prev_coord.max(next_coord) {
                if prev_coord > next_coord { prev_delta } else { next_delta }
            } else {
                prev_delta + (next_delta - prev_delta) * (cur_coord - prev_coord) / (next_coord - prev_coord)
            };
            cur = (cur - start + 1) % n + start;
        }
    }
}

// The glyph with its points moved, written onto the end of `out`. `cc` holds its points as read
// and is left moved; `flags` is scratch.
#[allow(clippy::too_many_arguments, reason = "the glyph, its deltas and the three buffers it reuses")]
pub fn apply_simple_glyph_deltas(
    data: &[u8], start: usize, end: usize,
    n_contours: usize,
    dx: &[f64], dy: &[f64],
    cc: &mut GlyphCoords, flags: &mut Vec<u8>, out: &mut Vec<u8>,
) {
    let raw = |out: &mut Vec<u8>| {
        let s = start.min(data.len());
        let e = end.min(data.len()).max(s);
        out.extend_from_slice(&data[s..e]);
    };
    let instr_len_off = start + 10 + n_contours * 2;
    let Some(instruction_len) = read_u16_be(data, instr_len_off) else { return raw(out) };
    let header_size = instr_len_off + 2 + usize::from(instruction_len) - start;
    let Some(header) = data.get(start..start + header_size) else { return raw(out) };

    let n = cc.num_points;
    let (xs, ys) = (&mut cc.x_coords[..n], &mut cc.y_coords[..n]);
    for i in 0..n {
        xs[i] = xs[i].saturating_add(ot_round(dx[i]));
        ys[i] = ys[i].saturating_add(ot_round(dy[i]));
    }
    let range = |v: &[i32]| (v.iter().copied().min().unwrap_or(0), v.iter().copied().max().unwrap_or(0));
    let ((x_min, x_max), (y_min, y_max)) = (range(xs), range(ys));

    flags.clear();
    let (mut prev_x, mut prev_y) = (0i32, 0i32);
    for i in 0..n {
        let dxp = xs[i].wrapping_sub(prev_x) as i16;
        let dyp = ys[i].wrapping_sub(prev_y) as i16;
        prev_x = xs[i]; prev_y = ys[i];
        let mut f = cc.flags[i] & 0xC1;
        if dxp == 0 { f |= 0x10; }
        else if (-255..=255).contains(&dxp) { f |= 0x02; if dxp > 0 { f |= 0x10; } }
        if dyp == 0 { f |= 0x20; }
        else if (-255..=255).contains(&dyp) { f |= 0x04; if dyp > 0 { f |= 0x20; } }
        flags.push(f);
    }

    let at = out.len();
    out.reserve(header_size + n * 5);
    out.extend_from_slice(header);
    write_i16_be(out, at + 2, x_min as i16);
    write_i16_be(out, at + 4, y_min as i16);
    write_i16_be(out, at + 6, x_max as i16);
    write_i16_be(out, at + 8, y_max as i16);
    let mut i = 0;
    while i < n {
        let rep = flags[i + 1..].iter().take(255).take_while(|&&f| f == flags[i]).count();
        if rep > 0 {
            out.extend_from_slice(&[flags[i] | 0x08, rep as u8]);
        } else {
            out.push(flags[i]);
        }
        i += 1 + rep;
    }
    push_deltas(out, flags, xs, 0x02, 0x10);
    push_deltas(out, flags, ys, 0x04, 0x20);
}

// One axis as glyf stores it: each point's step from the one before, a byte where `short` is set
// (`same` then giving its sign), else a word unless `same` says it repeats.
fn push_deltas(out: &mut Vec<u8>, flags: &[u8], coords: &[i32], short: u8, same: u8) {
    let mut prev = 0i32;
    for (&f, &c) in flags.iter().zip(coords) {
        let d = c.wrapping_sub(prev) as i16;
        prev = c;
        if f & short != 0 { out.push(d.unsigned_abs() as u8); }
        else if f & same == 0 { out.extend_from_slice(&d.to_be_bytes()); }
    }
}

// Bounded by the glyph's own end, as apply_composite_glyph_deltas reads it.
pub fn count_composite_components(data: &[u8], start: usize, end: usize) -> usize {
    let glyph = data.get(..end).unwrap_or(data);
    let mut pos = start + 10;
    let mut count = 0;
    loop {
        let f = match read_u16_be(glyph, pos) { Some(v) if pos + 4 <= end => v, _ => break };
        pos += 4; count += 1;
        pos += if f & ARG_1_AND_2_ARE_WORDS != 0 { 4 } else { 2 };
        pos += scale_len(f);
        if f & MORE_COMPONENTS == 0 { break; }
    }
    count
}

// The glyph with its component offsets moved, written onto the end of `out`.
pub fn apply_composite_glyph_deltas(
    data: &[u8], start: usize, end: usize,
    dx: &[f64], dy: &[f64], num_components: usize, out: &mut Vec<u8>,
) {
    let Some(src) = data.get(start..end) else { return };
    if src.len() < 10 { return out.extend_from_slice(src); }

    out.extend_from_slice(&src[..10]);
    let mut pos = 10usize;
    let mut comp = 0usize;
    while comp < num_components {
        let f = match read_u16_be(src, pos) { Some(v) => v, None => break };
        let glyph_idx = match read_u16_be(src, pos + 2) { Some(v) => v, None => break };
        pos += 4;

        let words_in = f & ARG_1_AND_2_ARE_WORDS != 0;
        let (a0, a1) = if words_in {
            let x = match read_i16_be(src, pos)     { Some(v) => v as i32, None => break };
            let y = match read_i16_be(src, pos + 2) { Some(v) => v as i32, None => break };
            pos += 4;
            (x, y)
        } else {
            let x = match src.get(pos)     { Some(&b) => b as i8 as i32, None => break };
            let y = match src.get(pos + 1) { Some(&b) => b as i8 as i32, None => break };
            pos += 2;
            (x, y)
        };

        let (na0, na1) = if f & ARGS_ARE_XY_VALUES != 0 {
            (a0.saturating_add(ot_round(dx.get(comp).copied().unwrap_or(0.0))),
             a1.saturating_add(ot_round(dy.get(comp).copied().unwrap_or(0.0))))
        } else {
            (a0, a1)
        };

        let words_out = words_in || !(-128..=127).contains(&na0) || !(-128..=127).contains(&na1);
        let nf = if words_out { f | ARG_1_AND_2_ARE_WORDS } else { f };
        out.extend_from_slice(&nf.to_be_bytes());
        out.extend_from_slice(&glyph_idx.to_be_bytes());
        if words_out {
            out.extend_from_slice(&(na0.clamp(-32768, 32767) as i16).to_be_bytes());
            out.extend_from_slice(&(na1.clamp(-32768, 32767) as i16).to_be_bytes());
        } else {
            out.push(na0 as i8 as u8);
            out.push(na1 as i8 as u8);
        }

        let t_len = scale_len(f);
        match src.get(pos..pos + t_len) {
            Some(t) => out.extend_from_slice(t),
            None => break,
        }
        pos += t_len;
        comp += 1;
        if f & MORE_COMPONENTS == 0 { break; }
    }
    if pos < src.len() { out.extend_from_slice(&src[pos..]); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    // One component that says more follow, then the next glyph's bytes: the count stops at the
    // glyph's own end.
    #[test]
    fn a_component_count_stops_at_the_glyphs_end() {
        let mut glyf = vec![0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0];
        glyf.extend([0x00, 0x20 | 0x02, 0, 1, 0, 0]);
        let end = glyf.len();
        for _ in 0..1000 {
            glyf.extend([0x00, 0x22, 0, 1, 0, 0]);
        }
        assert_eq!(count_composite_components(&glyf, 0, end), 1);
    }
}
