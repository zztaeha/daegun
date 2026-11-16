use alloc::vec::Vec;
use super::super::decoder::{clamp_i16, read_u16_be, read_i16_be};
use super::super::format::round::ot_round_f32;
use super::super::format::glyf::{Components, ARG_1_AND_2_ARE_WORDS};

// On cache lines of its own under `threading`, as a thread's scratch is written at every point: beside another thread's
// block, drawing at a location with a font each took up to 38% longer.
#[derive(Default)]
#[cfg_attr(feature = "threading", repr(align(128)))]
pub struct GlyphCoords {
    pub x_coords:   Vec<i32>,
    pub y_coords:   Vec<i32>,
    pub flags:      Vec<u8>,
    pub end_pts:    Vec<usize>,
    pub num_points: usize,
    // The glyph whose points these are as stored, where a drawn origin decoded them: the draw that follows reads them
    // rather than decoding them again.
    pub(crate) decoded: Option<u32>,
}

impl GlyphCoords {
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        let GlyphCoords { x_coords, y_coords, flags, end_pts, num_points: _, decoded: _ } = self;
        heap_of(x_coords) + heap_of(y_coords) + heap_of(flags) + heap_of(end_pts)
    }

    fn reset(&mut self) {
        self.x_coords.clear();
        self.y_coords.clear();
        self.flags.clear();
        self.end_pts.clear();
        self.num_points = 0;
        self.decoded = None;
    }
}

// A simple glyph's points. False, with nothing kept, where the end points do not increase or the
// flags or coordinates run out before the points do.
pub fn extract_coords_into(data: &[u8], n_contours: usize, out: &mut GlyphCoords) -> bool {
    let GlyphCoords { x_coords, y_coords, flags, end_pts, num_points, decoded } = out;
    *decoded = None;
    let read = extract_points_into(data, n_contours, x_coords, y_coords, flags, end_pts);
    *num_points = x_coords.len();
    read
}

// As `extract_coords_into`, into vectors the caller holds: cleared first, and left empty where the glyph doesn't
// decode. Each is its own argument, so the decode loops know that writing one leaves the others alone.
pub(crate) fn extract_points_into(
    data: &[u8], n_contours: usize, x: &mut Vec<i32>, y: &mut Vec<i32>, flags: &mut Vec<u8>, ends: &mut Vec<usize>,
) -> bool {
    x.clear();
    y.clear();
    flags.clear();
    ends.clear();
    let decoded = 'decode: {
        ends.reserve(n_contours);
        for i in 0..n_contours {
            let off = 10 + i * 2;
            let v = match read_u16_be(data, off) { Some(v) => v, None => break 'decode false };
            // `iup` indexes by these, so a non-monotonic sequence would panic rather than skip the glyph.
            if let Some(&prev) = ends.last()
                && v as usize <= prev { break 'decode false; }
            ends.push(v as usize);
        }
        let num_points = ends.last().map_or(0, |&e| e + 1);

        let instr_len_off = 10 + n_contours * 2;
        let instr_len = match read_u16_be(data, instr_len_off) { Some(v) => v as usize, None => break 'decode false };
        let mut pos = instr_len_off + 2 + instr_len;

        flags.reserve(num_points);
        while flags.len() < num_points {
            let f = match data.get(pos) { Some(&b) => { pos += 1; b } None => break };
            flags.push(f);
            if f & 0x08 != 0 {
                let r = match data.get(pos) { Some(&b) => { pos += 1; b as usize } None => break };
                let take = r.min(num_points - flags.len());
                flags.resize(flags.len() + take, f);
            }
        }
        if flags.len() < num_points { break 'decode false; }

        // Extended over the flags rather than written over zeros, which cost a pass of their own; a coordinate past
        // the data stops the reads and empties the glyph after.
        let mut read = true;
        let mut cur = 0i32;
        x.extend(flags[..num_points].iter().map(|&f| {
            if !read {
            } else if f & 0x02 != 0 {
                match data.get(pos) {
                    Some(&b) => {
                        pos += 1;
                        cur = cur.saturating_add(if f & 0x10 != 0 { b as i32 } else { (b as i32).saturating_neg() });
                    }
                    None => read = false,
                }
            } else if f & 0x10 == 0 {
                match read_i16_be(data, pos) {
                    Some(v) => { cur = cur.saturating_add(v as i32); pos += 2; }
                    None => read = false,
                }
            }
            cur
        }));
        let mut cur = 0i32;
        y.extend(flags[..num_points].iter().map(|&f| {
            if !read {
            } else if f & 0x04 != 0 {
                match data.get(pos) {
                    Some(&b) => {
                        pos += 1;
                        cur = cur.saturating_add(if f & 0x20 != 0 { b as i32 } else { (b as i32).saturating_neg() });
                    }
                    None => read = false,
                }
            } else if f & 0x20 == 0 {
                match read_i16_be(data, pos) {
                    Some(v) => { cur = cur.saturating_add(v as i32); pos += 2; }
                    None => read = false,
                }
            }
            cur
        }));
        read
    };
    if !decoded {
        x.clear();
        y.clear();
        flags.clear();
        ends.clear();
    }
    decoded
}

// The glyph with its points at `points`, rounded, written onto `out`, its box for the caller to set. `read` is each
// point's first-read flags, whose on-curve, overlap and, for GLYF (`extended`), cubic bits carry over.
pub fn write_simple_glyph(
    (glyph, extended): (&[u8], bool),
    n_contours: usize,
    points: &[(f32, f32)],
    read: &[u8],
    cc: &mut GlyphCoords,
    flags: &mut Vec<u8>,
    out: &mut Vec<u8>,
) {
    let instr_len_off = 10 + n_contours * 2;
    let Some(instruction_len) = read_u16_be(glyph, instr_len_off) else { return out.extend_from_slice(glyph) };
    let header_size = instr_len_off + 2 + usize::from(instruction_len);
    let Some(header) = glyph.get(..header_size) else { return out.extend_from_slice(glyph) };

    let n = read.len();
    cc.reset();
    cc.x_coords.extend(points.iter().take(n).map(|&(x, _)| ot_round_f32(x)));
    cc.y_coords.extend(points.iter().take(n).map(|&(_, y)| ot_round_f32(y)));
    let (xs, ys) = (&cc.x_coords[..], &cc.y_coords[..]);

    flags.clear();
    let (mut prev_x, mut prev_y) = (0i32, 0i32);
    for i in 0..n {
        let dxp = xs[i].wrapping_sub(prev_x) as i16;
        let dyp = ys[i].wrapping_sub(prev_y) as i16;
        prev_x = xs[i]; prev_y = ys[i];
        let mut f = read[i] & if extended { 0xC1 } else { 0x41 };
        if dxp == 0 { f |= 0x10; }
        else if (-255..=255).contains(&dxp) { f |= 0x02; if dxp > 0 { f |= 0x10; } }
        if dyp == 0 { f |= 0x20; }
        else if (-255..=255).contains(&dyp) { f |= 0x04; if dyp > 0 { f |= 0x20; } }
        flags.push(f);
    }

    out.reserve(header_size + n * 5);
    out.extend_from_slice(header);
    let mut i = 0;
    while i < n {
        // A flag the next one repeats is written twice, as a repeat byte would save nothing; a count starts at two.
        let rep = flags[i + 1..].iter().take(255).take_while(|&&f| f == flags[i]).count();
        if rep > 1 {
            out.extend_from_slice(&[flags[i] | 0x08, rep as u8]);
        } else {
            out.extend(core::iter::repeat_n(flags[i], rep + 1));
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

// The glyph with each component's offset at `offsets`, rounded, written onto the end of `out`. An offset that no
// longer fits a byte widens to a word; point numbers and glyph ids keep their width.
pub fn write_composite_glyph(glyph: &[u8], extended: bool, offsets: &[(f32, f32)], out: &mut Vec<u8>) {
    if glyph.len() < 10 { return out.extend_from_slice(glyph); }
    out.extend_from_slice(&glyph[..10]);
    let mut records = Components::new(glyph, extended);
    for (i, c) in records.by_ref().map_while(Result::ok).enumerate() {
        let (x, y) = match offsets.get(i) {
            Some(&(x, y)) if c.has_offset() => (ot_round_f32(x), ot_round_f32(y)),
            _ => c.args,
        };
        let byte = |v: i32| (-128..=127).contains(&v);
        let words = c.args_len() == 4 || (c.has_offset() && !(byte(x) && byte(y)));
        out.extend_from_slice(&(if words { c.flags | ARG_1_AND_2_ARE_WORDS } else { c.flags }).to_be_bytes());
        let args = c.at + 2 + c.gid_len();
        out.extend_from_slice(&glyph[c.at + 2..args]);
        for v in [x, y] {
            match (words, c.has_offset()) {
                (true, true) => out.extend_from_slice(&clamp_i16(v).to_be_bytes()),
                (true, false) => out.extend_from_slice(&(v as u16).to_be_bytes()),
                (false, _) => out.push(v as u8),
            }
        }
        out.extend_from_slice(&glyph[args + c.args_len()..c.next]);
    }
    out.extend_from_slice(&glyph[records.end()..]);
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/coords.rs"]
mod tests;
