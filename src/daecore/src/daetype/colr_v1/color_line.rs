use alloc::vec::Vec;
use super::super::decoder::{f2dot14, read_i16_be, read_u16_be, read_u32_be};
use super::paint::ColorStop;
use super::varfield::{resolve_var_field, VarField};
use super::Colrv1Ctx;
use crate::daecore::daekit::draw::Extend;

// The color line at `off`, its stops read into `stops` and its extend returned; drawn, an extend past reflect fails
// the line. A walk that measures reads no line: every gradient's fill counts the same whatever its stops.
pub(super) fn parse_color_line(
    ctx: &Colrv1Ctx, off: usize, is_var: bool, budget: &mut super::PaintBudget, stops: &mut Vec<ColorStop>,
) -> Option<Extend> {
    stops.clear();
    if !ctx.drawn { return Some(Extend::Pad); }
    let colr = ctx.colr;
    // COLR numbers them pad, repeat, reflect: not `Extend`'s order, which is SVG's.
    let extend = match *colr.get(off)? {
        0 => Extend::Pad,
        1 => Extend::Repeat,
        2 => Extend::Reflect,
        _ => return None,
    };
    let (num_stops, stop_size) = super::format::color_line_stops(colr, off, is_var)?;
    if !budget.spend_stops(num_stops) { return None; }
    stops.reserve_exact(num_stops);
    for i in 0..num_stops {
        let s = off + 3 + i * stop_size;
        let stop_offset_raw = read_i16_be(colr, s)?;
        let palette_index   = read_u16_be(colr, s + 2)?;
        let alpha_raw        = read_i16_be(colr, s + 4)?;

        let (stop_offset, alpha) = if is_var {
            let var_index_base = read_u32_be(colr, s + 6)?;
            (
                resolve_var_field(stop_offset_raw as i32, VarField::F2Dot14, var_index_base, 0, ctx),
                resolve_var_field(alpha_raw as i32, VarField::Alpha, var_index_base, 1, ctx),
            )
        } else {
            (f2dot14(stop_offset_raw), f2dot14(alpha_raw))
        };

        let (color, foreground) = ctx.color(palette_index, alpha)?;
        stops.push(ColorStop { offset: stop_offset, color, foreground });
    }

    Some(extend)
}
