use core::fmt;

/// A glyph's index in its font.
#[expect(clippy::exhaustive_structs, reason = "a newtype over the glyph's index")]
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct GlyphId(
    /// The index, 0 the font's `.notdef`.
    pub u32,
);

impl GlyphId {
    /// Glyph 0, what a font draws for a character it has no glyph for.
    pub const NOTDEF: GlyphId = GlyphId(0);
}

impl From<u16> for GlyphId {
    fn from(index: u16) -> GlyphId {
        GlyphId(u32::from(index))
    }
}

impl From<u32> for GlyphId {
    fn from(index: u32) -> GlyphId {
        GlyphId(index)
    }
}

impl From<GlyphId> for u32 {
    fn from(glyph: GlyphId) -> u32 {
        glyph.0
    }
}

/// The 16-bit id a font up to 65,535 glyphs stores; an error past it.
impl TryFrom<GlyphId> for u16 {
    type Error = core::num::TryFromIntError;

    fn try_from(glyph: GlyphId) -> Result<u16, Self::Error> {
        u16::try_from(glyph.0)
    }
}

impl fmt::Display for GlyphId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
