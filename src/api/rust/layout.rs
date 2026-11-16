//! Lines: text laid out and justified in font units.

use super::{Font, Location};
use crate::daecore::{GlyphId, Orientation, Tag};
use alloc::sync::Arc;
use alloc::vec::Vec;

pub use crate::daecore::daetype::jstf::JstfModLists;
pub use crate::text::justify::{Adjustment, Justified, JustifyOptions};
pub use crate::text::layout::{
    Align, BreakStrategy, LayoutLine, LayoutOptions, PositionedRun, TextLayout, TextOrientation, WritingMode,
};

impl Font {
    /// Lays text out in lines at a location.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::layout::LayoutOptions;
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let options = LayoutOptions::default().with_max_inline_size(10_000.0);
    /// let layout = font.layout("A line of text that runs on past one line.", &Location::default(), &options);
    /// for line in &layout.lines {
    ///     // line.bytes is the range of text it holds; line.runs its glyphs, each run placed at run.offset
    ///     # assert!(!line.runs.is_empty());
    /// }
    /// # assert!(layout.lines.len() > 1);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn layout(&self, text: &str, location: &Location, options: &LayoutOptions) -> TextLayout {
        crate::text::layout::layout_text(&self.cache, &self.key(location), text, options)
    }

    /// Justifies one line at a location: its natural shaping where that is within the tolerance, else the first
    /// JSTF priority level that comes within it, else the nearest.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::layout::JustifyOptions;
    /// use daegun::shape::ShapeOptions;
    /// use daegun::{Location, Orientation, Tag};
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/scheherazade-new");
    /// # let path = format!("{folder}/ScheherazadeNew-Regular.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let text = "بسم الله الرحمن الرحيم";
    /// let natural = font.shape(text, &Location::default(), &ShapeOptions::DEFAULT).width();
    /// let options = JustifyOptions::new(Tag::new(b"arab"), natural * 1.1);
    /// let line = font.justify(text, &Location::default(), Orientation::Horizontal, &options);
    /// // line.run is the line to draw, line.width its width, and line.level the JSTF level it took, if any.
    /// # assert!(line.width >= natural);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn justify(
        &self, text: &str, location: &Location, orientation: Orientation, options: &JustifyOptions,
    ) -> Justified {
        let (key, target) = (self.key(location), (options.target_width, options.tolerance));
        let (vertical, jstf) = (orientation.is_vertical(), (options.script, options.language_system));
        crate::text::justify::justify_line(&self.cache, &key, text, vertical, jstf, target)
    }

    /// Shapes text at a location with a JSTF level's lookups applied one way.
    #[must_use]
    pub fn shape_justified(
        &self, text: &str, location: &Location, orientation: Orientation, level: &JstfModLists,
        adjustment: Adjustment,
    ) -> Arc<crate::text::shape::ShapedRun> {
        let (key, vertical, shrink) = (self.key(location), orientation.is_vertical(), adjustment == Adjustment::Shrink);
        self.cache.shaped_run_justified(&key, text, vertical, level, shrink)
    }

    /// The glyphs JSTF lists for a script as extenders, which a justifier may repeat to widen a line, such as the
    /// tatweel; none where JSTF lists none.
    #[doc(alias("justification_extenders"))]
    pub fn justification_glyphs(&self, script: Tag) -> Option<Vec<GlyphId>> {
        let glyphs = crate::daecore::daetype::jstf::jstf_extender_glyphs(self.cache.font_tables(), script.to_bytes())?;
        Some(glyphs.into_iter().map(GlyphId).collect())
    }

    /// A script's JSTF priority levels for an OpenType language system, highest priority first, its default where
    /// none is named or the script has none for it; none where JSTF has no data for the script.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::layout::Adjustment;
    /// use daegun::{Location, Orientation, Tag};
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let folder = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/scheherazade-new");
    /// # let path = format!("{folder}/ScheherazadeNew-Regular.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let text = "بسم الله الرحمن الرحيم";
    /// if let Some(levels) = font.justification_priorities(Tag::new(b"arab"), None) {
    ///     // Each level's extension lookups widen a line; Font::justify tries them in this order.
    ///     let at = Location::default();
    ///     let wider = font.shape_justified(text, &at, Orientation::Horizontal, &levels[0], Adjustment::Extend);
    ///     # assert!(!wider.glyphs.is_empty());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn justification_priorities(&self, script: Tag, language_system: Option<Tag>) -> Option<Vec<JstfModLists>> {
        crate::text::justify::levels(&self.cache, script, language_system)
    }
}
