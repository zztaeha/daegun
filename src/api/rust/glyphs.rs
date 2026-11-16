use super::*;
use super::location::Key;
use crate::daecore::daekit::cache::AxisKey;
use crate::daecore::{Bounds, Orientation};
use core::iter::FusedIterator;

// Where a batch of advances goes: a Vec, emptied first, or a C caller's buffer as long as the glyphs.
pub(crate) trait AdvanceList {
    fn fill(&mut self, advances: impl Iterator<Item = f64>);
}

impl AdvanceList for Vec<f64> {
    // Emptied here, inside the cache's read, so the list's length is known rather than read back from memory.
    fn fill(&mut self, advances: impl Iterator<Item = f64>) {
        self.clear();
        self.extend(advances);
    }
}

impl Font {
    /// The glyph cmap maps each character of `text` to, none for one it maps to none.
    ///
    /// # Examples
    ///
    /// ```
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let ids: Vec<_> = font.glyph_ids("Wa\u{10FFFD}").collect();
    /// // One a character, none for the last, which Inter's cmap doesn't map.
    /// assert!(ids[0].is_some() && ids[2].is_none());
    /// # Ok(())
    /// # }
    /// ```
    pub fn glyph_ids(&self, text: &str) -> impl DoubleEndedIterator<Item = Option<GlyphId>> + FusedIterator + Clone {
        text.chars().map(|c| self.glyph_id(c))
    }

    /// The glyph cmap maps a character to, none where it maps it to none.
    #[inline(always)]
    pub fn glyph_id(&self, c: char) -> Option<GlyphId> {
        self.cache.inline_glyph_id(u32::from(c)).map(GlyphId)
    }

    /// Whether cmap maps a character to a glyph.
    pub fn has_glyph(&self, c: char) -> bool {
        self.glyph_id(c).is_some()
    }

    /// Every character cmap maps, in order, with its glyph. A code point that isn't a character is left out.
    #[doc(alias("codepoints"))]
    pub fn coverage(&self) -> impl DoubleEndedIterator<Item = (char, GlyphId)> + FusedIterator + use<> {
        let pairs = self.cache.cmap_coverage().into_iter();
        pairs.filter_map(|(c, gid)| Some((char::from_u32(c)?, GlyphId(gid))))
    }

    // The same for C, which counts before it writes: sorted with each code once, so the codes no `char` holds, the
    // surrogates and any past U+10FFFF, are cut where a search finds them.
    #[cfg(feature = "capi")]
    pub(crate) fn coverage_codes(&self) -> Vec<(u32, u32)> {
        let mut pairs = self.cache.cmap_coverage();
        pairs.truncate(pairs.partition_point(|p| p.0 <= 0x10_FFFF));
        let surrogates = pairs.partition_point(|p| p.0 < 0xD800)..pairs.partition_point(|p| p.0 < 0xE000);
        pairs.drain(surrogates);
        pairs
    }

    /// The glyph cmap's variation sequences give a character with a variation selector, none where they give
    /// none.
    pub fn variation_glyph_id(&self, c: char, selector: char) -> Option<GlyphId> {
        self.cache.variation_glyph_id(u32::from(c), u32::from(selector)).map(GlyphId)
    }

    /// The box a glyph's outline covers at a location, none for one that draws nothing or doesn't decode.
    pub fn glyph_bounds(&self, glyph: GlyphId, location: &Location) -> Option<Bounds> {
        let mut bounds = crate::daecore::daetype::outline::path::TightBounds::default();
        self.outline_glyph_keyed(glyph.0, &self.key(location), &mut bounds)?;
        bounds.finish()
    }

    /// Each glyph's advance at a location one way, into `out` once it's cleared, so a buffer kept between calls is
    /// allocated only to grow. Each is as [`Font::advance`] gives it.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::{Location, Orientation};
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let glyphs: Vec<_> = font.glyph_ids("Wave").flatten().collect();
    /// let mut widths = Vec::new();
    /// font.advances(&glyphs, &Location::default(), Orientation::Horizontal, &mut widths);
    /// // One a glyph in font units; `widths` keeps its room for the next line.
    /// # assert_eq!(widths.len(), 4);
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias("advance_widths"))]
    pub fn advances(&self, glyphs: &[GlyphId], location: &Location, orientation: Orientation, out: &mut Vec<f64>) {
        self.advances_into(glyphs, location, orientation, out);
    }

    // `advances` into a list of either kind: C fills its caller's buffer through it, with no Vec between.
    pub(crate) fn advances_into(
        &self,
        glyphs: &[GlyphId],
        location: &Location,
        orientation: Orientation,
        out: &mut (impl AdvanceList + ?Sized),
    ) {
        let key = self.key(location);
        let vertical = orientation.is_vertical();
        self.cache.described_at(&key, |at| {
            use crate::daecore::daekit::cache::Advances;
            if vertical {
                // The location read once for the batch, and the line a font without vmtx falls back to once too.
                let mut line = None;
                let mut fallback = || *line.get_or_insert_with(|| at.vertical_fallback_advance());
                out.fill(glyphs.iter().map(|&g| f64::from(self.vertical_advance_at(at, g.0, &mut fallback))));
                return;
            }
            // A loop for each source, as shaping has, keeps the per-glyph match out of a long batch.
            match at.advances(false) {
                Advances::Table(table) => out.fill(glyphs.iter().map(|&g| f64::from(table.of(g.0)))),
                Advances::Varied(varied) => out.fill(glyphs.iter().map(|&g| f64::from(varied.of(g.0)))),
                Advances::Listed(listed) => out.fill(glyphs.iter().map(|&g| f64::from(listed.of(g.0)))),
            }
        })
    }

    /// A glyph's advance at a location one way. Down a line, a font without vmtx advances each glyph by the
    /// height between its ascender and descender.
    #[doc(alias("vertical_advance"))]
    pub fn advance(&self, glyph: GlyphId, location: &Location, orientation: Orientation) -> f64 {
        let key = self.key(location);
        match orientation {
            Orientation::Horizontal => self.cache.advance_font_units(&key, glyph.0, false).into(),
            Orientation::Vertical => self.vertical_advance(glyph.0, &key),
        }
    }

    fn vertical_advance(&self, gid: u32, key: &AxisKey) -> f64 {
        let advance = |at: &FontCache| self.vertical_advance_at(at, gid, &mut || at.vertical_fallback_advance());
        self.cache.described_at(key, advance).into()
    }

    // vmtx's advance or a bitmap font's at `at`, the cache read there, else `fallback`.
    fn vertical_advance_at(&self, at: &FontCache, gid: u32, fallback: &mut impl FnMut() -> u32) -> u32 {
        self.cache.stated_vertical_advance(gid, at.advances(true).of(gid)).unwrap_or_else(fallback)
    }

    /// Where a glyph's vertical origin lies at a location, up from the baseline: VORG's, else the top of its
    /// box as vmtx places it, else the font's line centered on the em.
    pub fn vertical_origin(&self, glyph: GlyphId, location: &Location) -> f64 {
        self.cache.described_at(&self.key(location), |at| at.vertical_origin_units(glyph.0)).into()
    }

    /// VORG's default vertical origin, up from the baseline; 0 for a TrueType font or one without VORG.
    pub fn default_vertical_origin(&self) -> f64 {
        if self.cache.glyf().is_some() {
            return 0.0;
        }
        crate::daecore::daetype::vorg::vorg_default_origin_y(self.cache.font_tables()).map_or(0.0, f64::from)
    }

    /// A ligature's carets at a location, x across a line or y down one: one for each GDEF lists, in its order,
    /// none for one that can't be resolved.
    pub fn ligature_carets(&self, glyph: GlyphId, location: &Location, orientation: Orientation) -> Vec<Option<f64>> {
        let vertical = orientation.is_vertical();
        self.carets(glyph.0, &self.key(location), vertical, vertical)
    }

    // The carets, format 2's point y up from the baseline, or with `from_top`, from the vertical origin drawing sets.
    pub(super) fn carets(&self, gid: u32, key: &Key<'_>, vertical: bool, from_top: bool) -> Vec<Option<f64>> {
        let Some(gdef) = self.cache.gdef() else { return Vec::new() };
        let location = self.resolved(key);
        let points = || {
            let (xs, ys) = self.cache.described_at(key, |at| at.contour_points(gid))?;
            let top = match from_top {
                true => self.cache.at_location(key, |at| at.drawn_vertical_origin(gid, &ys))?,
                false => 0.0,
            };
            Some((xs, ys.iter().map(|y| y - top).collect()))
        };
        let store = || self.cache.gdef_var_store();
        crate::daecore::daetype::lig_caret::ligature_carets(gdef, gid, points, &location.coords, vertical, store)
    }

    /// GDEF's class for a glyph, none where GDEF gives it none.
    pub fn glyph_class(&self, glyph: GlyphId) -> Option<GlyphClass> {
        let gdef = self.cache.gdef()?;
        match crate::daecore::daetype::subsetter::otl::gdef::glyph_class(gdef, glyph.0) {
            1 => Some(GlyphClass::Base),
            2 => Some(GlyphClass::Ligature),
            3 => Some(GlyphClass::Mark),
            4 => Some(GlyphClass::Component),
            _ => None,
        }
    }

    /// GDEF's mark attachment class for a glyph; none where GDEF gives it none. A 24-bit ClassDef's classes reach
    /// past 16 bits.
    // NonZero, as an `Option<u32>` costs its callers a second word: 12 instructions a call where this costs none.
    pub fn mark_attachment_class(&self, glyph: GlyphId) -> Option<core::num::NonZeroU32> {
        use crate::daecore::daetype::subsetter::otl::gdef::mark_attach_class;
        core::num::NonZeroU32::new(self.cache.gdef().map_or(0, |gdef| mark_attach_class(gdef, glyph.0)))
    }

    /// A glyph's name, from post or CFF; none where neither names it. A name the font gives empty, or past post's
    /// 63 bytes, counts as none.
    pub fn glyph_name(&self, glyph: GlyphId) -> Option<&str> {
        let (post, cff) = self.cache.name_tables();
        let index = self.cache.name_index(&self.names);
        crate::daecore::daetype::glyph_names::glyph_name(index, post, cff, self.glyph_count(), glyph.0)
    }

    /// Every glyph's name, by glyph id, none for one post and CFF don't name.
    pub fn glyph_names(
        &self,
    ) -> impl ExactSizeIterator<Item = Option<&str>> + DoubleEndedIterator + FusedIterator + Clone {
        let (post, cff) = self.cache.name_tables();
        let index = self.cache.name_index(&self.names);
        crate::daecore::daetype::glyph_names::glyph_names(index, post, cff, self.glyph_count())
    }
}
