#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;
use alloc::boxed::Box;
use alloc::vec::Vec;
use super::super::decoder::{read_i16_be, read_offset24, read_u16_be, read_u32_be};
use super::color_line::parse_color_line;
use super::varfield::{decode_angle, resolve_var_angle, resolve_var_field, VarField};
use super::{Colrv1Ctx, PaintBudget};

#[derive(Debug, PartialEq)]
pub struct ColorStop {
    pub offset:        f64,
    pub is_foreground: bool,
    pub r: u8, pub g: u8, pub b: u8,
    pub alpha: u8,
}

#[derive(Debug, PartialEq)]
pub enum Paint {
    Layers(Vec<Paint>),
    Glyph { child: Box<Paint>, glyph_id: u16 },
    ColrGlyph { glyph_id: u16, child: Box<Paint> },
    Solid { is_foreground: bool, r: u8, g: u8, b: u8, alpha: u8 },
    LinearGradient { extend: u8, stops: Vec<ColorStop>, x0: f64, y0: f64, x1: f64, y1: f64, x2: f64, y2: f64 },
    RadialGradient { extend: u8, stops: Vec<ColorStop>, x0: f64, y0: f64, r0: f64, x1: f64, y1: f64, r1: f64 },
    SweepGradient { extend: u8, stops: Vec<ColorStop>, cx: f64, cy: f64, start_angle: f64, end_angle: f64 },
    Transform { child: Box<Paint>, matrix: [f64; 6] },
    Translate { child: Box<Paint>, dx: f64, dy: f64 },
    Scale { child: Box<Paint>, sx: f64, sy: f64, center: Option<(f64, f64)> },
    ScaleUniform { child: Box<Paint>, s: f64, center: Option<(f64, f64)> },
    Rotate { child: Box<Paint>, angle: f64, center: Option<(f64, f64)> },
    Skew { child: Box<Paint>, x_angle: f64, y_angle: f64, center: Option<(f64, f64)> },
    Composite { source: Box<Paint>, mode: u8, backdrop: Box<Paint> },
}

fn read_fixed_raw(data: &[u8], off: usize) -> Option<i32> {
    read_u32_be(data, off).map(|v| v as i32)
}

fn parse_affine(ctx: &Colrv1Ctx, off: usize, is_var: bool) -> Option<[f64; 6]> {
    let colr = ctx.colr;
    let mut raw = [0i32; 6];
    for (i, r) in raw.iter_mut().enumerate() {
        *r = read_fixed_raw(colr, off + i * 4)?;
    }
    if !is_var {
        return Some(raw.map(|v| v as f64 / 65536.0));
    }
    let var_index_base = read_u32_be(colr, off + 24)?;
    let mut out = [0.0f64; 6];
    for i in 0..6 {
        out[i] = resolve_var_field(raw[i], VarField::Fixed1616, var_index_base, i as u32, ctx);
    }
    Some(out)
}

fn read_center(colr: &[u8], off: usize) -> Option<(f64, f64)> {
    Some((read_i16_be(colr, off)? as f64, read_i16_be(colr, off + 2)? as f64))
}

fn resolve_var_center(ctx: &Colrv1Ctx, off: usize, var_index_base: u32, base_position: u32) -> Option<(f64, f64)> {
    let cx = read_i16_be(ctx.colr, off)? as i32;
    let cy = read_i16_be(ctx.colr, off + 2)? as i32;
    Some((
        resolve_var_field(cx, VarField::Raw, var_index_base, base_position, ctx),
        resolve_var_field(cy, VarField::Raw, var_index_base, base_position + 1, ctx),
    ))
}

// A paint that cannot be read, names one of its own ancestors or lies past the depth or visit
// limits is left out with its sub-graph, drawn as nothing, and the rest of the glyph still draws.
fn parse_child(ctx: &Colrv1Ctx, off: Option<usize>, budget: &mut PaintBudget) -> Paint {
    let Some(off) = off else { return Paint::Layers(Vec::new()) };
    if !budget.enter(off) { return Paint::Layers(Vec::new()); }
    let result = parse_paint(ctx, off, budget);
    budget.leave();
    result.unwrap_or(Paint::Layers(Vec::new()))
}

// The paint at `off` plus a 24-bit offset read at `off + at`.
fn child_at(colr: &[u8], off: usize, at: usize) -> Option<usize> {
    off.checked_add(read_offset24(colr, off.checked_add(at)?)?)
}

pub(super) fn parse_paint(ctx: &Colrv1Ctx, off: usize, budget: &mut PaintBudget) -> Option<Paint> {
    let colr = ctx.colr;
    let format = *colr.get(off)?;

    match format {
        1 => {
            let num_layers = *colr.get(off + 1)? as usize;
            let first_layer_index = read_u32_be(colr, off + 2)? as usize;
            let layer_list_off = ctx.layer_list_off?;
            let num_layers_total = read_u32_be(colr, layer_list_off)? as usize;
            let mut children = Vec::with_capacity(num_layers);
            for i in 0..num_layers {
                let entry = first_layer_index
                    .checked_add(i)
                    .filter(|&idx| idx < num_layers_total)
                    .and_then(|idx| idx.checked_mul(4)?.checked_add(layer_list_off)?.checked_add(4))
                    .and_then(|at| layer_list_off.checked_add(read_u32_be(colr, at)? as usize));
                children.push(parse_child(ctx, entry, budget));
            }
            Some(Paint::Layers(children))
        }
        2 | 3 => {
            let palette_index = read_u16_be(colr, off + 1)?;
            let alpha_raw = read_i16_be(colr, off + 3)?;
            let alpha = if format == 3 {
                let var_index_base = read_u32_be(colr, off + 5)?;
                resolve_var_field(alpha_raw as i32, VarField::F2Dot14, var_index_base, 0, ctx)
            } else {
                alpha_raw as f64 / 16384.0
            };
            let alpha_byte = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            if palette_index == 0xFFFF {
                Some(Paint::Solid { is_foreground: true, r: 0, g: 0, b: 0, alpha: alpha_byte })
            } else {
                let (cpal, palette) = (ctx.cpal?, ctx.palette.as_ref()?);
                let (r, g, b, cpal_a) = palette.entry(cpal, palette_index)?;
                let combined = (alpha_byte as f64 / 255.0) * (cpal_a as f64 / 255.0);
                Some(Paint::Solid { is_foreground: false, r, g, b, alpha: (combined * 255.0).round() as u8 })
            }
        }
        4 | 5 => {
            let is_var = format == 5;
            let (extend, stops) = parse_color_line(ctx, child_at(colr, off, 1)?, is_var, budget)?;
            let coord_off = off + 4;
            let raws = read_i16_pair3(colr, coord_off)?;
            let coords = if is_var {
                let var_index_base = read_u32_be(colr, coord_off + 12)?;
                resolve_coords6(ctx, &raws, var_index_base)
            } else {
                raws.map(|v| v as f64)
            };
            Some(Paint::LinearGradient {
                extend, stops,
                x0: coords[0], y0: coords[1], x1: coords[2], y1: coords[3], x2: coords[4], y2: coords[5],
            })
        }
        6 | 7 => {
            let is_var = format == 7;
            let (extend, stops) = parse_color_line(ctx, child_at(colr, off, 1)?, is_var, budget)?;
            let c = off + 4;
            let x0 = read_i16_be(colr, c)?;
            let y0 = read_i16_be(colr, c + 2)?;
            let r0 = read_u16_be(colr, c + 4)?;
            let x1 = read_i16_be(colr, c + 6)?;
            let y1 = read_i16_be(colr, c + 8)?;
            let r1 = read_u16_be(colr, c + 10)?;
            let (x0, y0, r0, x1, y1, r1) = if is_var {
                let vib = read_u32_be(colr, c + 12)?;
                (
                    resolve_var_field(x0 as i32, VarField::Raw, vib, 0, ctx),
                    resolve_var_field(y0 as i32, VarField::Raw, vib, 1, ctx),
                    resolve_var_field(r0 as i32, VarField::Raw, vib, 2, ctx),
                    resolve_var_field(x1 as i32, VarField::Raw, vib, 3, ctx),
                    resolve_var_field(y1 as i32, VarField::Raw, vib, 4, ctx),
                    resolve_var_field(r1 as i32, VarField::Raw, vib, 5, ctx),
                )
            } else {
                (x0 as f64, y0 as f64, r0 as f64, x1 as f64, y1 as f64, r1 as f64)
            };
            Some(Paint::RadialGradient { extend, stops, x0, y0, r0, x1, y1, r1 })
        }
        8 | 9 => {
            let is_var = format == 9;
            let (extend, stops) = parse_color_line(ctx, child_at(colr, off, 1)?, is_var, budget)?;
            let c = off + 4;
            let cx_raw = read_i16_be(colr, c)?;
            let cy_raw = read_i16_be(colr, c + 2)?;
            let start_raw = read_i16_be(colr, c + 4)?;
            let end_raw   = read_i16_be(colr, c + 6)?;
            let (cx, cy, start_angle, end_angle) = if is_var {
                let vib = read_u32_be(colr, c + 8)?;
                (
                    resolve_var_field(cx_raw as i32, VarField::Raw, vib, 0, ctx),
                    resolve_var_field(cy_raw as i32, VarField::Raw, vib, 1, ctx),
                    resolve_var_angle(start_raw, 1.0, vib, 2, ctx),
                    resolve_var_angle(end_raw, 1.0, vib, 3, ctx),
                )
            } else {
                (cx_raw as f64, cy_raw as f64, decode_angle(start_raw, 1.0), decode_angle(end_raw, 1.0))
            };
            Some(Paint::SweepGradient { extend, stops, cx, cy, start_angle, end_angle })
        }
        10 => {
            let glyph_id = read_u16_be(colr, off + 4)?;
            let child = parse_child(ctx, child_at(colr, off, 1), budget);
            Some(Paint::Glyph { child: Box::new(child), glyph_id })
        }
        11 => {
            let glyph_id = read_u16_be(colr, off + 1)?;
            let root = super::lookup_base_glyph_paint_offset(colr, ctx.base_glyph_list_off, glyph_id);
            Some(Paint::ColrGlyph { glyph_id, child: Box::new(parse_child(ctx, root, budget)) })
        }
        12 | 13 => {
            let matrix = parse_affine(ctx, child_at(colr, off, 4)?, format == 13)?;
            let child = parse_child(ctx, child_at(colr, off, 1), budget);
            Some(Paint::Transform { child: Box::new(child), matrix })
        }
        14 | 15 => {
            let dx_raw = read_i16_be(colr, off + 4)?;
            let dy_raw = read_i16_be(colr, off + 6)?;
            let (dx, dy) = if format == 15 {
                let vib = read_u32_be(colr, off + 8)?;
                (resolve_var_field(dx_raw as i32, VarField::Raw, vib, 0, ctx), resolve_var_field(dy_raw as i32, VarField::Raw, vib, 1, ctx))
            } else {
                (dx_raw as f64, dy_raw as f64)
            };
            let child = parse_child(ctx, child_at(colr, off, 1), budget);
            Some(Paint::Translate { child: Box::new(child), dx, dy })
        }
        16..=23 => {
            let uniform = matches!(format, 20..=23);
            let around_center = matches!(format, 18 | 19 | 22 | 23);
            let is_var = format % 2 == 1;

            let (sx_raw, sy_raw, after_scale_off) = if uniform {
                let s = read_i16_be(colr, off + 4)?;
                (s, s, off + 6)
            } else {
                (read_i16_be(colr, off + 4)?, read_i16_be(colr, off + 6)?, off + 8)
            };

            let (sx, sy, center, child) = if is_var {
                let (center_raw_off, vib_off) = if around_center { (after_scale_off, after_scale_off + 4) } else { (after_scale_off, after_scale_off) };
                let vib = read_u32_be(colr, vib_off)?;
                let sx = resolve_var_field(sx_raw as i32, VarField::F2Dot14, vib, 0, ctx);
                let sy = if uniform { sx } else { resolve_var_field(sy_raw as i32, VarField::F2Dot14, vib, 1, ctx) };
                let base_pos = if uniform { 1 } else { 2 };
                let center = if around_center { Some(resolve_var_center(ctx, center_raw_off, vib, base_pos)?) } else { None };
                let child = parse_child(ctx, child_at(colr, off, 1), budget);
                (sx, sy, center, child)
            } else {
                let sx = sx_raw as f64 / 16384.0;
                let sy = sy_raw as f64 / 16384.0;
                let center = if around_center { Some(read_center(colr, after_scale_off)?) } else { None };
                let child = parse_child(ctx, child_at(colr, off, 1), budget);
                (sx, sy, center, child)
            };

            if uniform {
                Some(Paint::ScaleUniform { child: Box::new(child), s: sx, center })
            } else {
                Some(Paint::Scale { child: Box::new(child), sx, sy, center })
            }
        }
        24..=27 => {
            let around_center = matches!(format, 26 | 27);
            let is_var = format % 2 == 1;
            let angle_raw = read_i16_be(colr, off + 4)?;

            let (angle, center, child) = if is_var {
                let center_off = off + 6;
                let vib_off = if around_center { center_off + 4 } else { center_off };
                let vib = read_u32_be(colr, vib_off)?;
                let angle = resolve_var_angle(angle_raw, 0.0, vib, 0, ctx);
                let center = if around_center { Some(resolve_var_center(ctx, center_off, vib, 1)?) } else { None };
                let child = parse_child(ctx, child_at(colr, off, 1), budget);
                (angle, center, child)
            } else {
                let angle = decode_angle(angle_raw, 0.0);
                let center = if around_center { Some(read_center(colr, off + 6)?) } else { None };
                let child = parse_child(ctx, child_at(colr, off, 1), budget);
                (angle, center, child)
            };
            Some(Paint::Rotate { child: Box::new(child), angle, center })
        }
        28..=31 => {
            let around_center = matches!(format, 30 | 31);
            let is_var = format % 2 == 1;
            let x_raw = read_i16_be(colr, off + 4)?;
            let y_raw = read_i16_be(colr, off + 6)?;

            let (x_angle, y_angle, center, child) = if is_var {
                let center_off = off + 8;
                let vib_off = if around_center { center_off + 4 } else { center_off };
                let vib = read_u32_be(colr, vib_off)?;
                let x_angle = resolve_var_angle(x_raw, 0.0, vib, 0, ctx);
                let y_angle = resolve_var_angle(y_raw, 0.0, vib, 1, ctx);
                let center = if around_center { Some(resolve_var_center(ctx, center_off, vib, 2)?) } else { None };
                let child = parse_child(ctx, child_at(colr, off, 1), budget);
                (x_angle, y_angle, center, child)
            } else {
                let x_angle = decode_angle(x_raw, 0.0);
                let y_angle = decode_angle(y_raw, 0.0);
                let center = if around_center { Some(read_center(colr, off + 8)?) } else { None };
                let child = parse_child(ctx, child_at(colr, off, 1), budget);
                (x_angle, y_angle, center, child)
            };
            Some(Paint::Skew { child: Box::new(child), x_angle, y_angle, center })
        }
        32 => {
            let mode = *colr.get(off + 4)?;
            let source = parse_child(ctx, child_at(colr, off, 1), budget);
            let backdrop = parse_child(ctx, child_at(colr, off, 5), budget);
            Some(Paint::Composite { source: Box::new(source), mode, backdrop: Box::new(backdrop) })
        }
        _ => None,
    }
}

fn read_i16_pair3(colr: &[u8], off: usize) -> Option<[i16; 6]> {
    let mut out = [0i16; 6];
    for (i, o) in out.iter_mut().enumerate() {
        *o = read_i16_be(colr, off + i * 2)?;
    }
    Some(out)
}

fn resolve_coords6(ctx: &Colrv1Ctx, raws: &[i16; 6], var_index_base: u32) -> [f64; 6] {
    let mut out = [0.0f64; 6];
    for i in 0..6 {
        out[i] = resolve_var_field(raws[i] as i32, VarField::Raw, var_index_base, i as u32, ctx);
    }
    out
}
