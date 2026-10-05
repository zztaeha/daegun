use super::*;
use crate::daecore::daetype::outline::OutlinePen;
use crate::daecore::daetype::outline::stroke::Stroked;

// The oblique's shear goes in before the caller's own transform, so a glyph leans in its own frame.
fn sheared(tangent: f32, transform: Option<[f32; 6]>) -> [f32; 6] {
    let s = [1.0f32, 0.0, tangent, 1.0, 0.0, 0.0];
    let Some(o) = transform else { return s };
    [
        s[0] * o[0] + s[1] * o[2],
        s[0] * o[1] + s[1] * o[3],
        s[2] * o[0] + s[3] * o[2],
        s[2] * o[1] + s[3] * o[3],
        s[4] * o[0] + s[5] * o[2] + o[4],
        s[4] * o[1] + s[5] * o[3] + o[5],
    ]
}

// Why prepare drew nothing, which C answers with different statuses.
pub(crate) enum Unprepared {
    BadInput,
    NoOutline,
    TooManyPoints,
}

impl Font {
    // An outline for a rasterizer of your own: pixels, y up, origin at the glyph origin. The oblique, then
    // the transform (offsets in font units), apply after hinting. None draws nothing.
    pub fn prepared_outline(
        &self,
        gid: u16,
        px: f32,
        axes: &[(&str, f64)],
        opts: &OutlineOptions,
        pen: &mut dyn OutlinePen,
    ) -> Option<PreparedGlyph> {
        self.prepare(gid, px, axes, opts, pen).ok()
    }

    pub(crate) fn prepare(
        &self,
        gid: u16,
        px: f32,
        axes: &[(&str, f64)],
        opts: &OutlineOptions,
        pen: &mut dyn OutlinePen,
    ) -> Result<PreparedGlyph, Unprepared> {
        if !(px.is_finite() && px > 0.0) {
            return Err(Unprepared::BadInput);
        }
        if opts.transform.is_some_and(|t| !t.iter().all(|v| v.is_finite()))
            || opts.stroke.is_some_and(|s| !s.width.is_finite())
        {
            return Err(Unprepared::BadInput);
        }
        let transform = match opts.oblique {
            None => opts.transform,
            Some(k) if k.is_finite() => Some(sheared(k, opts.transform)),
            Some(_) => return Err(Unprepared::BadInput),
        };
        let axes_key = crate::daecore::cache::canonical_axes(axes);
        let s = px / f32::from(self.upm());
        let hinted = self.hinted_outline(gid, px, &axes_key, opts.hinting);

        // Stroked in font units unless hinted, because the stroker's thresholds are absolute and do not
        // scale. Then into pixels on the way to the caller's pen.
        let mut path = Path::default();
        let (to_px, units_to_path, tolerance) = match (&hinted, transform) {
            (Some(out), None) => {
                draw_hinted(out, &mut path);
                (1.0, s, 0.25)
            }
            (Some(out), Some(t)) => {
                let m = [t[0], t[1], t[2], t[3], t[4] * s, t[5] * s].map(f64::from);
                draw_hinted(out, &mut TransformPen::new(&mut path, m));
                (1.0, s, 0.25)
            }
            (None, None) => {
                self.outline_glyph_keyed(gid, &axes_key, &mut path).ok_or(Unprepared::NoOutline)?;
                (s, 1.0, 0.25 * (f32::from(self.upm()) / px))
            }
            (None, Some(t)) => {
                self.outline_glyph_keyed(gid, &axes_key, &mut TransformPen::new(&mut path, t.map(f64::from)))
                    .ok_or(Unprepared::NoOutline)?;
                (s, 1.0, 0.25 * (f32::from(self.upm()) / px))
            }
        };

        // Font units, not `advance_keyed`, which is per 1000 em and would be off by upm / 1000. A font
        // with no vertical metrics answers 0 for the height, which an embolden must leave at 0.
        let bold = opts.embolden.filter(|u| u.is_finite() && *u > 0.0 && opts.stroke.is_none());
        let grow = bold.unwrap_or(0.0);
        let height = self.cache.advance_font_units_rs(&axes_key, gid, true) as f32;
        let glyph = PreparedGlyph {
            advance_width: (self.cache.advance_font_units_rs(&axes_key, gid, false) as f32 + grow) * s,
            advance_height: if self.has_vertical_metrics() { height + grow } else { height } * s,
            hinted: hinted.is_some(),
        };
        let style = match (opts.stroke, bold) {
            (Some(style), _) => Some(StrokeStyle { width: style.width * units_to_path, ..style }),
            (None, Some(units)) => {
                Some(StrokeStyle { width: units * units_to_path, join: Join::Round, cap: Cap::Round })
            }
            (None, None) => None,
        };

        // Finite options can overflow once scaled: nothing is drawn for an advance, width or point too
        // large. A scale of 1 or less cannot overflow, so a plain outline is checked once px passes upm.
        let fits = |p: &(f32, f32)| fits_scaled(p.0, to_px) && fits_scaled(p.1, to_px);
        if !(glyph.advance_width.is_finite() && glyph.advance_height.is_finite())
            || style.is_some_and(|st| !st.width.is_finite())
            || (transform.is_some() || to_px > 1.0) && !path.parts().1.iter().all(fits)
        {
            return Err(Unprepared::BadInput);
        }
        let build = if opts.stroke.is_some() { Stroked::new } else { Stroked::embolden };
        let stroked = style.map(|st| build(&path, &st, tolerance).ok_or(Unprepared::TooManyPoints)).transpose()?;
        if stroked.as_ref().is_some_and(|st| !st.points().all(fits)) {
            return Err(Unprepared::BadInput);
        }

        let mut out = TransformPen::new(pen, [f64::from(to_px), 0.0, 0.0, f64::from(to_px), 0.0, 0.0]);
        match stroked {
            Some(st) if opts.stroke.is_some() => st.draw(&mut out),
            Some(bold) => {
                path.replay(None, &mut out);
                bold.draw(&mut out);
            }
            None => path.replay(None, &mut out),
        }
        Ok(glyph)
    }
}

// What the pen gets is the point times `scale`, in f64 and then cast back to f32.
fn fits_scaled(v: f32, scale: f32) -> bool {
    (f64::from(v) * f64::from(scale)).abs() <= f64::from(f32::MAX)
}
