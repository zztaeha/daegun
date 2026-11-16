// VARC's glyphs written as outlines of their own, for an instance or subset that leaves out a VARC it can't keep:
// each point rounded to the nearest of the whole units an outline stores.
#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::daecore::daetype::format::cff::push_short_int;
use crate::daecore::daetype::instancer::{write_simple_glyph, GlyphCoords};
use crate::daecore::daetype::outline::{Path, Point, Verb};
use crate::daecore::Error;

// Each of the glyphs asked for that VARC draws, drawn where the instance or subset stands, or why none are.
pub(crate) type Draw<'a> = &'a dyn Fn(&[u32]) -> Result<BTreeMap<u32, Path>, Error>;

const ON: u8 = 0x01;
const CUBIC: u8 = 0x80;

fn cut(p: Point) -> (i32, i32) {
    (p.x.round() as i32, p.y.round() as i32)
}

fn short(v: i32) -> bool {
    i16::try_from(v).is_ok()
}

// Each contour's points with their glyf flags. A contour whose last curve ends on its first point drops that point,
// which the contour's closing curve draws.
fn contours(path: &Path) -> Vec<Vec<((i32, i32), u8)>> {
    let (verbs, points) = path.parts();
    let (mut out, mut at): (Vec<Vec<_>>, usize) = (Vec::new(), 0);
    let mut curved = false;
    for &verb in verbs {
        let flags: &[u8] = match verb {
            Verb::Move => {
                out.push(Vec::new());
                &[ON]
            }
            Verb::Line => &[ON],
            Verb::Quad => &[0, ON],
            Verb::Cubic => &[CUBIC, CUBIC, ON],
            Verb::Close => &[],
        };
        let taken = points.get(at..at + flags.len()).unwrap_or_default();
        at += flags.len();
        if let Some(contour) = out.last_mut() {
            contour.extend(taken.iter().map(|&p| cut(p)).zip(flags.iter().copied()));
            if verb != Verb::Close {
                curved = matches!(verb, Verb::Quad | Verb::Cubic);
            }
            if verb == Verb::Close && curved && contour.len() > 1 && contour.first() == contour.last() {
                contour.pop();
            }
        }
    }
    out
}

// The path as a glyf simple glyph, its box over every point; empty for an empty path. None where it has a point past
// 16 bits, more points or contours than a glyph holds, or a cubic in a glyf that holds none.
pub(crate) fn glyf(path: &Path, extended: bool) -> Option<Vec<u8>> {
    let contours = contours(path);
    let points: Vec<((i32, i32), u8)> = contours.iter().flatten().copied().collect();
    let [x_min, y_min, x_max, y_max] = match bounds(points.iter().map(|&(p, _)| p)) {
        Some(bounds) => bounds,
        None => return Some(Vec::new()),
    };
    let held = points.iter().all(|&((x, y), f)| short(x) && short(y) && (extended || f & CUBIC == 0));
    if !held || points.len() > 0xFFFF || contours.len() > 0x7FFF {
        return None;
    }
    let mut header = (contours.len() as i16).to_be_bytes().to_vec();
    header.extend([x_min, y_min, x_max, y_max].map(i16::to_be_bytes).concat());
    let mut end = 0;
    for contour in &contours {
        end += contour.len();
        header.extend(((end - 1) as u16).to_be_bytes());
    }
    header.extend([0, 0]);
    let at: Vec<(f32, f32)> = points.iter().map(|&((x, y), _)| (x as f32, y as f32)).collect();
    let read: Vec<u8> = points.iter().map(|&(_, f)| f).collect();
    let (mut out, mut scratch) = (Vec::new(), GlyphCoords::default());
    write_simple_glyph((&header, extended), contours.len(), &at, &read, &mut scratch, &mut Vec::new(), &mut out);
    Some(out)
}

fn bounds(points: impl Iterator<Item = (i32, i32)>) -> Option<[i16; 4]> {
    points.fold(None, |b: Option<[i32; 4]>, (x, y)| {
        Some(b.map_or([x, y, x, y], |[x0, y0, x1, y1]| [x0.min(x), y0.min(y), x1.max(x), y1.max(y)]))
    }).map(|b| b.map(|v| v as i16))
}

// The path as a charstring of moves, lines and curves, a quadratic raised to a cubic; CFF's ends in endchar. None
// where a point or the step to it passes 16 bits.
pub(crate) fn charstring(path: &Path, endchar: bool) -> Option<Vec<u8>> {
    let (verbs, points) = path.parts();
    let (mut out, mut at, mut pen) = (Vec::new(), 0, (0, 0));
    let mut put = |out: &mut Vec<u8>, to: &[Point], op: u8| {
        for &p in to {
            let (x, y) = cut(p);
            let fits = short(x) && short(y) && push_short_int(out, x - pen.0) && push_short_int(out, y - pen.1);
            fits.then_some(())?;
            pen = (x, y);
        }
        out.push(op);
        Some(())
    };
    for &verb in verbs {
        let p = points.get(at..at + verb.points())?;
        at += verb.points();
        match verb {
            Verb::Move => put(&mut out, p, 21)?,
            Verb::Line => put(&mut out, p, 5)?,
            Verb::Quad => {
                let from = *points.get(at.checked_sub(3)?)?;
                let raised = |a: Point, c: Point| Point {
                    x: a.x + (c.x - a.x) * 2.0 / 3.0,
                    y: a.y + (c.y - a.y) * 2.0 / 3.0,
                };
                put(&mut out, &[raised(from, p[0]), raised(p[1], p[0]), p[1]], 8)?;
            }
            Verb::Cubic => put(&mut out, p, 8)?,
            Verb::Close => {}
        }
    }
    if endchar {
        out.push(14);
    }
    Some(out)
}

// The path's box as a charstring's is measured: over the points of each move a segment follows and of
// every segment, control points too, an axis it doesn't span at 0; None where every edge is 0.
pub(crate) fn measured(path: &Path) -> Option<[i16; 4]> {
    let (verbs, points) = path.parts();
    let mut drawn = Vec::new();
    let mut at = 0;
    for (i, &verb) in verbs.iter().enumerate() {
        let n = verb.points();
        let opens = matches!(verbs.get(i + 1), Some(Verb::Line | Verb::Quad | Verb::Cubic));
        if verb != Verb::Move || opens {
            drawn.extend(points.get(at..at + n).unwrap_or_default().iter().map(|&p| cut(p)));
        }
        at += n;
    }
    let [x0, y0, x1, y1] = bounds(drawn.into_iter())?;
    let (x, y) = (if x0 < x1 { [x0, x1] } else { [0, 0] }, if y0 < y1 { [y0, y1] } else { [0, 0] });
    let b = [x[0], y[0], x[1], y[1]];
    (b != [0; 4]).then_some(b)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/varc/flatten.rs"]
mod tests;
