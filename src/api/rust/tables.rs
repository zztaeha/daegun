//! BASE baselines, the tags GSUB and GPOS offer, and subsetting with its glyph closure.

#[cfg(doc)]
use crate::ErrorKind::{LimitExceeded, Malformed, Missing, Unsupported};
use super::{Font, Location};
use alloc::vec::Vec;
use core::borrow::Borrow;
use crate::daecore::daetype::format::sanitize::OtlTable;
use crate::daecore::daekit::base::error::Result;
use crate::daecore::{GlyphId, Orientation, Tag};

pub use crate::daecore::daetype::base::BaseScript;
pub use crate::daecore::daetype::subsetter::Subset;

/// How far a script's glyphs reach across a line, from BASE: below and above it, or left and right of a vertical
/// one, in font units.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct Extents {
    /// The lowest; none where the font leaves it out.
    pub min: Option<f64>,
    /// The highest; none where the font leaves it out.
    pub max: Option<f64>,
}

impl Font {
    /// A subset of the font, pinned to a location, holding these glyphs and every glyph they reach: components,
    /// substitutions and color layers among them. Takes glyphs by value or by reference, so a slice, a `Vec` or a
    /// set of them serves as it is.
    ///
    /// # Errors
    ///
    /// The subsetter's and, away from the default location, the instancer's: [`Malformed`] for a table that
    /// doesn't read, [`Missing`] for one it needs, [`Unsupported`] for one it can't write, and [`LimitExceeded`] past
    /// its bounds.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let glyphs: Vec<_> = font.glyph_ids("Wave").flatten().collect();
    /// let subset = font.subset(&glyphs, &Location::default())?;
    /// // subset.font is the font file; a glyph's id in it may differ from its id here.
    /// let w = subset.renumbered(glyphs[0]).expect("a glyph asked for is kept");
    /// # assert!(subset.font.len() < bytes.len() && w.0 > 0);
    /// # Ok(())
    /// # }
    /// ```
    pub fn subset(&self, glyphs: impl IntoIterator<Item: Borrow<GlyphId>>, location: &Location) -> Result<Subset> {
        let gids: Vec<u32> = glyphs.into_iter().map(|g| g.borrow().0).collect();
        self.cache.subset_font(&self.key(location), &gids)
    }

    /// A subset of the font, pinned to a location, holding what `text` maps to and every glyph that reaches, its
    /// cmap keeping only the text's characters.
    ///
    /// # Errors
    ///
    /// As [`subset`](Font::subset) gives them, a font without cmap [`Missing`].
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let subset = font.subset_text("Hello", &Location::default())?;
    /// // subset.font is a font file of its own, holding H, e, l and o and what they reach
    /// # assert!(daegun::Font::from_bytes(&subset.font)?.glyph_count() < font.glyph_count());
    /// # Ok(())
    /// # }
    /// ```
    pub fn subset_text(&self, text: &str, location: &Location) -> Result<Subset> {
        self.cache.subset_text(&self.key(location), text)
    }

    /// The glyphs a subset of these would hold: them and every glyph they reach, sorted, each once. Takes glyphs as
    /// [`subset`](Font::subset) does.
    ///
    /// # Errors
    ///
    /// As [`subset`](Font::subset) gives them.
    pub fn glyph_closure(
        &self, glyphs: impl IntoIterator<Item: Borrow<GlyphId>>, location: &Location,
    ) -> Result<Vec<GlyphId>> {
        let gids: Vec<u32> = glyphs.into_iter().map(|g| g.borrow().0).collect();
        Ok(self.cache.glyph_closure(&self.key(location), &gids)?.into_iter().map(GlyphId).collect())
    }

    /// A script's baselines one way at a location, DFLT's where BASE gives the script none; none for a font
    /// without BASE, or whose BASE has neither.
    #[doc(alias("base_info"))]
    pub fn base_script(&self, script: Tag, location: &Location, orientation: Orientation) -> Option<BaseScript> {
        let key = self.key(location);
        let location = self.resolved(&key);
        let table_map = self.cache.font_tables();
        crate::daecore::daetype::base::base_script_info(
            table_map, Some(script.to_bytes()), orientation.is_vertical(), &location.coords,
        )
    }

    /// A script's extents one way at a location, for an OpenType language system and a feature where given: the
    /// language system's, else the script's default, narrowed to the feature's where it has its own. DFLT's where
    /// BASE gives the script none; none for a font without BASE, or whose BASE has neither.
    pub fn base_extents(
        &self, script: Tag, location: &Location, orientation: Orientation, language_system: Option<Tag>,
        feature: Option<Tag>,
    ) -> Option<Extents> {
        let key = self.key(location);
        let location = self.resolved(&key);
        let language_system = language_system.map(Tag::to_bytes);
        let (min, max) = crate::daecore::daetype::base::base_min_max(
            self.cache.font_tables(), Some(script.to_bytes()), language_system, feature.map(Tag::to_bytes),
            orientation.is_vertical(), &location.coords,
        )?;
        Some(Extents { min, max })
    }

    /// Whether the font has BASE and none of its coordinates names a glyph, so renumbering glyphs leaves it valid.
    #[doc(alias("base_is_glyph_free"))]
    pub fn has_glyph_free_base(&self) -> bool {
        self.cache
            .font_tables()
            .get("BASE")
            .is_some_and(|base| crate::daecore::daetype::base::base_is_glyph_free(base))
    }

    /// The scripts GSUB and GPOS offer, sorted, each once.
    pub fn script_tags(&self) -> Vec<Tag> {
        self.layout_tags(crate::daecore::daeshaper::ot::offered::script_tags)
    }

    /// The OpenType language systems GSUB and GPOS offer for a script, sorted, each once.
    #[doc(alias("language_tags"))]
    pub fn language_system_tags(&self, script: Tag) -> Vec<Tag> {
        let tag = script.to_bytes();
        self.layout_tags(move |d| crate::daecore::daeshaper::ot::offered::language_tags(d, &tag))
    }

    /// The features GSUB and GPOS offer a script and OpenType language system, sorted, each once: with no script,
    /// the one shaping falls back to (DFLT, dflt, then latn), and for no language system or one the script lacks,
    /// its default.
    pub fn feature_tags(&self, script: Option<Tag>, language_system: Option<Tag>) -> Vec<Tag> {
        let (s, l) = (script.map(Tag::to_bytes), language_system.map(Tag::to_bytes));
        self.layout_tags(move |d| crate::daecore::daeshaper::ot::offered::feature_tags(d, s.as_ref(), l.as_ref()))
    }

    fn layout_tags(&self, read: impl Fn(&[u8]) -> Vec<[u8; 4]>) -> Vec<Tag> {
        let mut out = Vec::new();
        for which in [OtlTable::Gsub, OtlTable::Gpos] {
            let Some(data) = self.cache.layout_table(which) else { continue };
            out.extend(read(data).iter().map(Tag::new));
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}
