use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::pen::OutlinePen;
use super::super::decoder::{read_i16_be, read_u16_be};
use super::super::instancer::{extract_coords_into, GlyphCoords};
use crate::daecore::daetype::TableBytes;
use super::super::format::glyf::{
    ARG_1_AND_2_ARE_WORDS, ARGS_ARE_XY_VALUES, MORE_COMPONENTS, SCALED_COMPONENT_OFFSET,
    UNSCALED_COMPONENT_OFFSET, WE_HAVE_AN_X_AND_Y_SCALE, WE_HAVE_A_SCALE, WE_HAVE_A_TWO_BY_TWO,
};

pub fn outline_glyf_glyph_with_loca(
    table_map: &BTreeMap<String, TableBytes>,
    loca:      &[usize],
    gid:       u16,
    pen:       &mut dyn OutlinePen,
) -> Result<(), String> {
    let glyf = table_map.get("glyf").ok_or("glyf: missing glyf table")?;
    outline_glyf_bytes(glyf, loca, gid, pen)
}

pub fn outline_glyf_bytes(
    glyf: &[u8],
    loca: &[usize],
    gid:  u16,
    pen:  &mut dyn OutlinePen,
) -> Result<(), String> {
    let mut scratch = GlyphCoords::default();
    outline_glyf_glyph_reusing_bytes(glyf, loca, gid, &mut scratch, pen)
}

pub fn outline_glyf_glyph_reusing_bytes(
    glyf: &[u8],
    loca: &[usize],
    gid: u16,
    scratch: &mut GlyphCoords,
    pen: &mut dyn OutlinePen,
) -> Result<(), String> {
    let mut budget = Budget { visits: MAX_COMPONENT_VISITS, points: MAX_COMPONENT_POINTS };
    draw_component(glyf, loca, gid, 0, &mut budget, scratch, pen)
}

// A glyph's points as TrueType numbers them, a composite's components placed in order: what a contour
// point index names.
pub(crate) fn glyph_points(glyf: &[u8], loca: &[usize], gid: u16) -> Result<(Vec<f64>, Vec<f64>), String> {
    let mut budget = Budget { visits: MAX_COMPONENT_VISITS, points: MAX_COMPONENT_POINTS };
    let mut points = Points::default();
    collect_glyph(glyf, loca, gid, 0, &mut budget, &mut GlyphCoords::default(), &mut points)?;
    Ok((points.x, points.y))
}

struct Budget {
    visits: u32,
    points: u32,
}

const MAX_COMPONENT_DEPTH: usize = 10;

const MAX_COMPONENT_VISITS: u32 = 65_536;

const MAX_COMPONENT_POINTS: u32 = 1_000_000;

impl Budget {
    fn visit(&mut self, depth: usize) -> Result<(), String> {
        if depth > MAX_COMPONENT_DEPTH {
            return Err("glyf: composite glyph nesting too deep".into());
        }
        self.visits = self.visits.checked_sub(1).ok_or("glyf: composite glyph work budget exhausted")?;
        Ok(())
    }

    fn spend(&mut self, points: usize) -> Result<(), String> {
        self.points = u32::try_from(points)
            .ok()
            .and_then(|n| self.points.checked_sub(n))
            .ok_or("glyf: composite glyph point budget exhausted")?;
        Ok(())
    }
}

// A glyph's own bytes, as loca bounds them: nothing it decodes may come from the next glyph.
fn glyph_bytes<'a>(glyf: &'a [u8], loca: &[usize], gid: u16) -> Result<&'a [u8], String> {
    let gid = usize::from(gid);
    if gid + 1 >= loca.len() { return Err("glyf: glyph index out of range".into()); }
    let (start, end) = (loca[gid], loca[gid + 1]);
    if end <= start { return Ok(&[]); }
    glyf.get(start..end.min(glyf.len())).ok_or_else(|| "glyf: glyph data out of range".into())
}

fn draw_component(
    glyf: &[u8], loca: &[usize], gid: u16, depth: usize, budget: &mut Budget,
    scratch: &mut GlyphCoords, pen: &mut dyn OutlinePen,
) -> Result<(), String> {
    budget.visit(depth)?;
    let glyph = glyph_bytes(glyf, loca, gid)?;
    if glyph.is_empty() { return Ok(()); }

    let n_contours = read_i16_be(glyph, 0).ok_or("glyf: glyph header truncated")?;
    if n_contours >= 0 {
        decode_simple(glyph, n_contours as usize, budget, scratch)?;
        draw_points(scratch, pen);
        Ok(())
    } else if needs_points(glyph) {
        let mut points = Points::default();
        collect_composite(glyf, loca, glyph, depth, budget, scratch, &mut points)?;
        points.draw(pen);
        Ok(())
    } else {
        draw_composite_glyph(glyf, loca, glyph, depth, budget, scratch, pen)
    }
}

// The contours are charged before they are read, since reading a glyph's end points costs as much
// whether or not its points then decode.
fn decode_simple(glyph: &[u8], n_contours: usize, budget: &mut Budget, coords: &mut GlyphCoords) -> Result<(), String> {
    budget.spend(n_contours)?;
    if !extract_coords_into(glyph, 0, n_contours, coords) {
        return Err("glyf: simple glyph truncated, or its end points do not increase".into());
    }
    budget.spend(coords.num_points)
}

fn draw_points(coords: &GlyphCoords, pen: &mut dyn OutlinePen) {
    let mut c_start = 0usize;
    for &c_end in &coords.end_pts {
        if c_end >= coords.num_points { break; }
        draw_contour_over(&GlyfContour { coords, start: c_start, end: c_end }, pen);
        c_start = c_end + 1;
    }
}

pub trait ContourPoints {
    fn len(&self) -> usize;
    fn get(&self, i: usize) -> (f32, f32, bool);
}

struct GlyfContour<'a> {
    coords: &'a GlyphCoords,
    start: usize,
    end: usize,
}

impl ContourPoints for GlyfContour<'_> {
    fn len(&self) -> usize { self.end - self.start + 1 }
    fn get(&self, i: usize) -> (f32, f32, bool) {
        let k = self.start + i;
        (
            self.coords.x_coords[k] as f32,
            self.coords.y_coords[k] as f32,
            self.coords.flags[k] & 0x01 != 0,
        )
    }
}

pub(crate) fn draw_contour_over<P: ContourPoints + ?Sized>(pts: &P, pen: &mut dyn OutlinePen) {
    let n = pts.len();
    if n == 0 { return; }

    // TrueType starts a contour at point 0 if it is on-curve, otherwise at the last point if that
    // one is, and only failing both at the midpoint the `None` arm implies.
    let start_idx = if pts.get(0).2 {
        Some(0)
    } else if pts.get(n - 1).2 {
        Some(n - 1)
    } else {
        None
    };
    let (start_x, start_y, walk_from) = match start_idx {
        Some(i) => { let p = pts.get(i); (p.0, p.1, (i + 1) % n) }
        None => {
            let (lx, ly, _) = pts.get(n - 1);
            let (fx, fy, _) = pts.get(0);
            ((lx + fx) * 0.5, (ly + fy) * 0.5, 0)
        }
    };
    pen.move_to(start_x, start_y);

    let steps = if start_idx.is_some() { n - 1 } else { n };
    let mut pending_ctrl: Option<(f32, f32)> = None;
    let mut idx = walk_from;
    for _ in 0..steps {
        let (x, y, on) = pts.get(idx);
        idx += 1;
        if idx == n { idx = 0; }
        if on {
            match pending_ctrl.take() {
                Some((cx, cy)) => pen.quad_to(cx, cy, x, y),
                None => pen.line_to(x, y),
            }
        } else if let Some((cx, cy)) = pending_ctrl.replace((x, y)) {
            let (mx, my) = ((cx + x) * 0.5, (cy + y) * 0.5);
            pen.quad_to(cx, cy, mx, my);
        }
    }
    if let Some((cx, cy)) = pending_ctrl {
        pen.quad_to(cx, cy, start_x, start_y);
    }
    pen.close();
}

fn f2dot14(raw: i16) -> f64 { raw as f64 / 16384.0 }

// One component record: its glyph, its two arguments as stored, the 2x2 it carries and where the
// next record starts. The scale flags are read in the spec's order, WE_HAVE_A_SCALE first.
struct Component {
    flags: u16,
    gid: u16,
    args: (i32, i32),
    matrix: (f64, f64, f64, f64),
    next: usize,
}

fn read_component(glyph: &[u8], pos: usize) -> Result<Component, String> {
    let truncated = || String::from("glyf: composite stream truncated");
    let flags = read_u16_be(glyph, pos).ok_or_else(truncated)?;
    let gid = read_u16_be(glyph, pos + 2).ok_or_else(truncated)?;
    let mut at = pos + 4;
    let signed = flags & ARGS_ARE_XY_VALUES != 0;
    let args = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
        let (a, b) = (read_u16_be(glyph, at).ok_or_else(truncated)?, read_u16_be(glyph, at + 2).ok_or_else(truncated)?);
        at += 4;
        if signed { (i32::from(a as i16), i32::from(b as i16)) } else { (i32::from(a), i32::from(b)) }
    } else {
        let (a, b) = (*glyph.get(at).ok_or_else(truncated)?, *glyph.get(at + 1).ok_or_else(truncated)?);
        at += 2;
        if signed { (i32::from(a as i8), i32::from(b as i8)) } else { (i32::from(a), i32::from(b)) }
    };
    let f2 = |at: &mut usize| -> Result<f64, String> {
        let v = read_i16_be(glyph, *at).ok_or_else(truncated)?;
        *at += 2;
        Ok(f2dot14(v))
    };
    let matrix = if flags & WE_HAVE_A_SCALE != 0 {
        let s = f2(&mut at)?;
        (s, 0.0, 0.0, s)
    } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
        let (a, d) = (f2(&mut at)?, f2(&mut at)?);
        (a, 0.0, 0.0, d)
    } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
        (f2(&mut at)?, f2(&mut at)?, f2(&mut at)?, f2(&mut at)?)
    } else {
        (1.0, 0.0, 0.0, 1.0)
    };
    Ok(Component { flags, gid, args, matrix, next: at })
}

impl Component {
    fn more(&self) -> bool {
        self.flags & MORE_COMPONENTS != 0
    }

    // An offset scales with the matrix only when SCALED_COMPONENT_OFFSET says so and its opposite does not.
    fn offset(&self) -> (f64, f64) {
        let (dx, dy) = (f64::from(self.args.0), f64::from(self.args.1));
        let (a, b, c, d) = self.matrix;
        if self.flags & SCALED_COMPONENT_OFFSET != 0 && self.flags & UNSCALED_COMPONENT_OFFSET == 0 {
            (dx * a + dy * c, dx * b + dy * d)
        } else {
            (dx, dy)
        }
    }
}

fn needs_points(glyph: &[u8]) -> bool {
    let mut pos = 10;
    while let Ok(c) = read_component(glyph, pos) {
        if c.flags & ARGS_ARE_XY_VALUES == 0 { return true; }
        if !c.more() { break; }
        pos = c.next;
    }
    false
}

#[allow(clippy::too_many_arguments, reason = "the recursion carries its budget and its scratch")]
fn draw_composite_glyph(
    glyf: &[u8], loca: &[usize], glyph: &[u8], depth: usize, budget: &mut Budget,
    scratch: &mut GlyphCoords, pen: &mut dyn OutlinePen,
) -> Result<(), String> {
    let mut pos = 10;
    loop {
        let c = read_component(glyph, pos)?;
        let (a, b, cc, d) = c.matrix;
        let (tdx, tdy) = c.offset();
        let mut tp = super::pen::TransformPen::new(pen, [a, b, cc, d, tdx, tdy]);
        draw_component(glyf, loca, c.gid, depth + 1, budget, scratch, &mut tp)?;
        if !c.more() { break; }
        pos = c.next;
    }
    Ok(())
}

// A composite's points, kept rather than streamed, for a component placed by matching one of its
// points to one of the points before it.
#[derive(Default)]
struct Points {
    x: Vec<f64>,
    y: Vec<f64>,
    on: Vec<bool>,
    ends: Vec<usize>,
}

impl Points {
    fn draw(&self, pen: &mut dyn OutlinePen) {
        let mut start = 0;
        for &end in &self.ends {
            draw_contour_over(&PointsContour { points: self, start, end }, pen);
            start = end + 1;
        }
    }
}

struct PointsContour<'a> {
    points: &'a Points,
    start: usize,
    end: usize,
}

impl ContourPoints for PointsContour<'_> {
    fn len(&self) -> usize { self.end - self.start + 1 }
    fn get(&self, i: usize) -> (f32, f32, bool) {
        let k = self.start + i;
        (self.points.x[k] as f32, self.points.y[k] as f32, self.points.on[k])
    }
}

fn collect_glyph(
    glyf: &[u8], loca: &[usize], gid: u16, depth: usize, budget: &mut Budget,
    scratch: &mut GlyphCoords, out: &mut Points,
) -> Result<(), String> {
    budget.visit(depth)?;
    let glyph = glyph_bytes(glyf, loca, gid)?;
    if glyph.is_empty() { return Ok(()); }
    let n_contours = read_i16_be(glyph, 0).ok_or("glyf: glyph header truncated")?;
    if n_contours < 0 {
        return collect_composite(glyf, loca, glyph, depth, budget, scratch, out);
    }
    decode_simple(glyph, n_contours as usize, budget, scratch)?;
    let base = out.x.len();
    for k in 0..scratch.num_points {
        out.x.push(f64::from(scratch.x_coords[k]));
        out.y.push(f64::from(scratch.y_coords[k]));
        out.on.push(scratch.flags[k] & 0x01 != 0);
    }
    out.ends.extend(scratch.end_pts.iter().map(|&e| base + e));
    Ok(())
}

// As the spec places a component by points: the child's point (its own numbering, after its matrix)
// lands on the parent's point (numbered across the components before it).
#[allow(clippy::too_many_arguments, reason = "the recursion carries its budget and its scratch")]
fn collect_composite(
    glyf: &[u8], loca: &[usize], glyph: &[u8], depth: usize, budget: &mut Budget,
    scratch: &mut GlyphCoords, out: &mut Points,
) -> Result<(), String> {
    let mut pos = 10;
    loop {
        let c = read_component(glyph, pos)?;
        let mut child = Points::default();
        collect_glyph(glyf, loca, c.gid, depth + 1, budget, scratch, &mut child)?;
        let (a, b, cc, d) = c.matrix;
        for k in 0..child.x.len() {
            let (x, y) = (child.x[k], child.y[k]);
            child.x[k] = x * a + y * cc;
            child.y[k] = x * b + y * d;
        }
        let (dx, dy) = if c.flags & ARGS_ARE_XY_VALUES != 0 {
            c.offset()
        } else {
            let (parent, own) = (c.args.0 as usize, c.args.1 as usize);
            let (Some(&px), Some(&py), Some(&cx), Some(&cy)) =
                (out.x.get(parent), out.y.get(parent), child.x.get(own), child.y.get(own))
            else {
                return Err("glyf: a matched component point is past its glyph's points".into());
            };
            (px - cx, py - cy)
        };
        let base = out.x.len();
        out.x.extend(child.x.iter().map(|x| x + dx));
        out.y.extend(child.y.iter().map(|y| y + dy));
        out.on.extend(child.on);
        out.ends.extend(child.ends.iter().map(|e| base + e));
        if !c.more() { break; }
        pos = c.next;
    }
    Ok(())
}
