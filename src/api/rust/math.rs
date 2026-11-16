//! The MATH table: the constants, glyph metrics and stretching glyphs that math layout reads, in font units at a
//! location: each value with its variation delta there from GDEF's store, unrounded.

#[cfg(doc)]
use crate::ErrorKind::InvalidArgument;
use super::{Font, Location};
use crate::daecore::daetype::math_table::{self as table, Deltas, DeviceDeltas};
use crate::daecore::daekit::base::error::Result;
use crate::daecore::{Error, GlyphId, Orientation};

pub use crate::daecore::daetype::math_table::{
    GlyphAssembly, GlyphPart, MathConstants, MathGlyphConstruction, MathGlyphVariant, MathKernCorner,
};

impl Font {
    // `read` with GDEF's deltas for MATH's values away from the default. Each call reads the default itself, making
    // no deltas: handing its answer back through a closure took `math_glyph_variants` 22% longer.
    #[inline(never)]
    fn math_at<T>(&self, location: &Location, read: impl FnOnce(Deltas<'_>) -> T) -> T {
        let key = self.key(location);
        let store = self.cache.font_tables().contains_key("MATH").then(|| self.cache.gdef_var_store()).flatten();
        let mut deltas = store.and_then(|s| DeviceDeltas::from_store(s, &self.resolved(&key).coords));
        read(deltas.as_mut())
    }

    /// MATH's constants at a location; none for a font without MATH or with constants it cuts short.
    pub fn math_constants(&self, location: &Location) -> Option<MathConstants> {
        let tables = self.cache.font_tables();
        if location.is_default() {
            return table::parse_math_constants(tables, None);
        }
        self.math_at(location, |deltas| table::parse_math_constants(tables, deltas))
    }

    /// A glyph's italic correction at a location, in font units; none for one MATH gives none.
    pub fn math_italics_correction(&self, glyph: GlyphId, location: &Location) -> Option<f64> {
        let tables = self.cache.font_tables();
        if location.is_default() {
            return table::math_italics_correction(tables, glyph.0, None);
        }
        self.math_at(location, |deltas| table::math_italics_correction(tables, glyph.0, deltas))
    }

    /// Where an accent over a glyph centers at a location, in font units from its origin: MATH's, or half its
    /// advance there.
    pub fn math_top_accent_attachment(&self, glyph: GlyphId, location: &Location) -> f64 {
        let tables = self.cache.font_tables();
        let stated = if location.is_default() {
            table::math_top_accent_attachment(tables, glyph.0, None)
        } else {
            self.math_at(location, |deltas| table::math_top_accent_attachment(tables, glyph.0, deltas))
        };
        stated.unwrap_or_else(|| self.advance(glyph, location, Orientation::Horizontal) / 2.0)
    }

    /// Whether MATH marks a glyph an extended shape, one whose scripts attach as to a tall glyph.
    pub fn math_is_extended_shape(&self, glyph: GlyphId) -> bool {
        table::math_is_extended_shape(self.cache.font_tables(), glyph.0)
    }

    /// The kern at one of a glyph's corners for a script at `height`, at a location, in font units; 0 where MATH
    /// gives none. An infinite height selects the first or last kern.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`] for a height that isn't a number.
    pub fn math_kern(&self, glyph: GlyphId, location: &Location, corner: MathKernCorner, height: f64) -> Result<f64> {
        if height.is_nan() {
            return Err(Error::invalid("MATH: a kern's height is not a number"));
        }
        let tables = self.cache.font_tables();
        if location.is_default() {
            return Ok(table::math_kern(tables, glyph.0, corner, height, None));
        }
        Ok(self.math_at(location, |deltas| table::math_kern(tables, glyph.0, corner, height, deltas)))
    }

    /// How a glyph stretches one way, horizontally or vertically, at a location, where its assembly's italic
    /// correction may vary; none for one MATH doesn't stretch that way.
    pub fn math_glyph_variants(
        &self, glyph: GlyphId, location: &Location, orientation: Orientation,
    ) -> Option<MathGlyphConstruction> {
        let (tables, vertical) = (self.cache.font_tables(), orientation.is_vertical());
        if location.is_default() {
            return table::math_glyph_construction(tables, glyph.0, vertical, None);
        }
        self.math_at(location, |deltas| table::math_glyph_construction(tables, glyph.0, vertical, deltas))
    }

    /// The least overlap between the parts of an assembled glyph, in font units; none for a font without MATH.
    pub fn math_min_connector_overlap(&self) -> Option<f64> {
        table::math_min_connector_overlap(self.cache.font_tables()).map(f64::from)
    }
}
