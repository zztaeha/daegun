use super::*;

impl Font {
    pub fn glyph_quads(&self, gid: u16, axes: &[(&str, f64)]) -> Result<Vec<Quad>, QuadError> {
        let mut pen = QuadraticPen::new(f32::from(self.upm()));
        self.outline_glyph_keyed(gid, &crate::daecore::cache::canonical_axes(axes), &mut pen)
            .ok_or(QuadError::NoOutline)?;
        let mut curves = pen.finish()?;
        normalize_winding(&mut curves);
        Ok(curves)
    }
}
