//! Color glyphs: bitmaps, COLR layers and palettes, paint graphs and the scenes lowered from them, and the color
//! math to sample a scene's gradients and composite its layers.

#[cfg(doc)]
use crate::ErrorKind::LimitExceeded;
use super::{Font, Location};
use alloc::vec::Vec;
use crate::daecore::daekit::cache::color::ColrAt;
use crate::daecore::daekit::cache::AxisKey;
use crate::daecore::daetype::outline::FillRule;
use crate::daecore::daekit::base::error::Result;
use crate::daecore::{Bounds, Error, GlyphId, NameId};

pub use crate::daecore::daekit::draw::blend::{Premultiplied, Rgb};
pub use crate::daecore::daekit::draw::gradient;
pub use crate::daecore::daekit::draw::matrix::{concat, invert, Matrix, IDENTITY};
pub use crate::daecore::daetype::bitmap::{BitmapImage, GlyphBitmap};
pub use crate::daecore::daetype::colr_v0::{ColrLayer, Palette};
pub use crate::daecore::daetype::colr_v1::{Center, ColorStop, PaintGraph, PaintNode, PaintRef, PaintVisitor};
pub use crate::daecore::daetype::paint::colr::lower;
pub use crate::daecore::daetype::paint::{
    Blend, ClipShape, DisplayList, Extend, Gradient, GradientId, GradientKind, Op, Paint, PathId, Rgba,
    Stop, Stops,
};

/// What [`Font::color_scene`] draws with.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct SceneOptions {
    /// The CPAL palette, by index; one past the font's reads palette 0, the default.
    pub palette: u16,
    /// The text's color, which a layer or paint asking for the foreground takes. Opaque black by default.
    pub foreground: Rgba,
}

impl Default for SceneOptions {
    fn default() -> SceneOptions {
        SceneOptions::DEFAULT
    }
}

impl SceneOptions {
    /// The defaults, as `default()` gives them: palette 0 and opaque black.
    pub const DEFAULT: SceneOptions = SceneOptions { palette: 0, foreground: Rgba { r: 0, g: 0, b: 0, a: 255 } };

    /// These options with a palette.
    #[must_use]
    pub const fn with_palette(mut self, palette: u16) -> SceneOptions {
        self.palette = palette;
        self
    }

    /// These options with the text's color.
    #[must_use]
    pub const fn with_foreground(mut self, foreground: Rgba) -> SceneOptions {
        self.foreground = foreground;
        self
    }
}

// A scene's outlines, each drawn into its list once. Past what the font's size allows, points held or decoded, the
// scene is refused: a glyph's decoding can multiply its bytes, and layers name glyphs freely.
struct Outlines<'f> {
    font: &'f Font,
    key: &'f AxisKey,
    held: usize,
    most: usize,
    over: bool,
}

impl Outlines<'_> {
    fn get(&mut self, g: u32, out: &mut DisplayList) -> Option<PathId> {
        if self.over {
            return None;
        }
        let mut work = 0;
        let id = out.draw_path(|pen| self.font.outline_glyph_counted(g, self.key, &mut work, pen).is_ok());
        self.held += work + id.and_then(|id| out.path(id)).map_or(0, |p| p.parts().1.len());
        self.over = self.held > self.most;
        id.filter(|_| !self.over)
    }
}

impl Font {
    /// A glyph's bitmap from sbix, CBDT, EBDT or bdat, from the strike nearest `ppem`, at or above it first; none
    /// for a glyph no strike holds.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::color::BitmapImage;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/noto-color-emoji/NotoColorEmoji.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let smile = font.glyph_id('😀').expect("Noto Color Emoji maps 😀");
    /// if let Some(bitmap) = font.bitmap(smile, 64) {
    ///     match bitmap.image {
    ///         // Lent from the font: decode it with a PNG library, then scale it from bitmap.ppem.
    ///         BitmapImage::Png(png) => assert!(png.starts_with(b"\x89PNG")),
    ///         // One byte of coverage a pixel, rows top to bottom.
    ///         BitmapImage::Coverage { width, height, pixels } => {
    ///             assert_eq!(pixels.len(), usize::from(width) * usize::from(height));
    ///         }
    ///         _ => {}
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias("glyph_bitmap"))]
    pub fn bitmap(&self, glyph: GlyphId, ppem: u16) -> Option<GlyphBitmap<'_>> {
        crate::daecore::daetype::bitmap::glyph_bitmap(self.cache.font_tables(), glyph.0, ppem)
    }

    /// A COLR version 0 glyph's layers in a palette, back to front, as they're drawn; none for a glyph with none.
    /// A palette past the font's reads palette 0, the default.
    #[doc(alias("colr_layers_for_palette"))]
    pub fn colr_layers(&self, glyph: GlyphId, palette: u16) -> Option<Vec<ColrLayer>> {
        let glyphs = self.cache.drawn_glyph_count();
        let layers = |map| crate::daecore::daetype::colr_v0::drawn_layers(map, glyph.0, palette, glyphs);
        self.cache.colr_drawn().then_some(self.cache.font_tables()).and_then(layers)
    }

    /// The font's CPAL palettes; none for a font without, or one whose CPAL fails sanitizing.
    #[doc(alias("palette_info"))]
    pub fn palettes(&self) -> Vec<Palette> {
        self.each_palette().collect()
    }

    /// How many palettes [`Font::palettes`] lists, without listing them.
    pub fn palette_count(&self) -> usize {
        self.each_palette().len()
    }

    // The same one at a time, which C writes into its caller's buffer.
    pub(crate) fn each_palette(&self) -> impl ExactSizeIterator<Item = Palette> + '_ {
        let cpal = self.cache.font_tables().get("CPAL").filter(|_| self.cache.palettes_read());
        crate::daecore::daetype::colr_v0::cpal_palettes(cpal.map_or(&[], |cpal| &cpal[..]))
    }

    /// Each palette entry's name ID in the name table, so a picker can label the colors a palette sets: none for an
    /// entry the font leaves unnamed, and an empty list where it names none.
    pub fn palette_entry_labels(&self) -> Vec<Option<NameId>> {
        let labels = || crate::daecore::daetype::colr_v0::cpal_palette_entry_labels(self.cache.font_tables());
        if self.cache.palettes_read() { labels() } else { Vec::new() }
    }

    /// A palette's colors in entry order, as a picker shows them and CSS's `override-colors` replaces them: none
    /// for a font without CPAL. A palette past the font's reads palette 0, the default, as drawing does.
    pub fn palette_colors(&self, palette: u16) -> Vec<Rgba> {
        let cpal = self.cache.font_tables().get("CPAL").filter(|_| self.cache.palettes_read());
        cpal.map_or_else(Vec::new, |cpal| crate::daecore::daetype::colr_v0::cpal_palette_colors(cpal, palette))
    }

    /// A color glyph as a scene to draw, from its COLR version 1 paint graph or else its version 0 layers. In font
    /// units, y up, each path held once and placed by its op's transform, at most 128 levels of clips and layers
    /// deep, a version 1 glyph inside its clip box. None for a glyph with no color description, and no ops for one
    /// that draws nothing.
    ///
    /// # Errors
    ///
    /// [`LimitExceeded`] where its outlines hold more than 16 points a byte of the font's glyf, CFF, CFF2 and VARC
    /// tables, and a million at least, each glyph held once and its decoding, drawn or not, counted with them: a
    /// charstring's step, a point or glyph glyf decodes, or a unit of VARC's work, a value it decodes as one point and
    /// a component it loads as 64.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::color::{Op, SceneOptions};
    /// use daegun::{GlyphId, Location};
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/colr-v1-test-glyphs/test_glyphs.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// if let Some(scene) = font.color_scene(GlyphId(9), &Location::default(), &SceneOptions::default())? {
    ///     for op in scene.ops() {
    ///         if let Op::Fill { path, paint, transform, .. } = op {
    ///             // Fill `scene.path(*path)` with `paint`, placed by `transform`, in font units.
    ///             # let _ = (scene.path(*path), paint, transform);
    ///         }
    ///     }
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias("colr_scene", "colr_scene_with"))]
    pub fn color_scene(
        &self, glyph: GlyphId, location: &Location, options: &SceneOptions,
    ) -> Result<Option<DisplayList>> {
        self.scene_into(glyph, location, options, DisplayList::spare())
    }

    // The same drawn into `list`, which is empty: Rust's own, or the one C's reused scene keeps with its room. Generic,
    // as through a reference Rust's frame grew and its scene took 3% longer.
    #[inline(always)]
    pub(crate) fn scene_into<L: core::borrow::BorrowMut<DisplayList>>(
        &self, glyph: GlyphId, location: &Location, options: &SceneOptions, mut list: L,
    ) -> Result<Option<L>> {
        use crate::daecore::daetype::paint::colr::{lower_tree, lower_walk};
        let (gid, &SceneOptions { palette, foreground }) = (glyph.0, options);
        let out = list.borrow_mut();
        let key = self.key(location);
        let most = self.cache.scene_points();
        let mut outlines = Outlines { font: self, key: &key, held: 0, most, over: false };
        // A version 1 glyph the spec says must not be drawn, unbounded with no clip box, leaves the
        // version 0 layers, if the font has them, to draw instead.
        let clip_box = |g| self.cache.colr_clip_box(g, ColrAt::Drawn(&key)).map(Bounds::of);
        let walked = self.cache.colr_v1_walk(gid, ColrAt::Drawn(&key), palette, |walk, root| {
            lower_walk(gid, walk, root, &clip_box, &mut |g, out| outlines.get(g, out), foreground, out)
        });
        let drawn = match walked {
            Some(Some(drawn)) => drawn,
            // Where the walk can't settle what its budget leaves out, the tree it would build decides.
            Some(None) => {
                out.clear();
                (outlines.held, outlines.over) = (0, false);
                self.cache.colr_v1_paint(gid, &key, palette).is_some_and(|paint| {
                    lower_tree(gid, &paint, &clip_box, &mut |g, out| outlines.get(g, out), foreground, out)
                })
            }
            None => false,
        };
        if !drawn {
            let layers = self.cache.colr_drawn().then_some(self.cache.font_tables()).and_then(|map| {
                crate::daecore::daetype::colr_v0::each_drawn_layer(map, gid, palette, self.cache.drawn_glyph_count())
            });
            let Some(mut layers) = layers.map(Iterator::peekable) else { return Ok(None) };
            if layers.peek().is_none() {
                return Ok(None);
            }
            let mut paths = crate::daecore::daetype::paint::colr::Interned::new();
            // Back to front, which is the order COLR v0 records them in.
            for ColrLayer { glyph, color, foreground: text } in layers {
                let path = paths.get(glyph.0).unwrap_or_else(|| {
                    let path = outlines.get(glyph.0, out);
                    paths.insert(glyph.0, path);
                    path
                });
                let Some(path) = path else { continue };
                let color = if text { foreground } else { color };
                out.push(Op::Fill { path, paint: Paint::Solid(color), rule: FillRule::NonZero, transform: IDENTITY });
            }
        }
        if outlines.over {
            return Err(Error::limit("the scene's outlines take more than 16 points a byte of the font's outlines"));
        }
        Ok(Some(list))
    }

    /// A COLR version 1 glyph's clip box in font units, which what it draws stays inside; none for a glyph without
    /// one. At a location each edge moves by its delta, rounded to a whole unit.
    pub fn colr_clip_box(&self, glyph: GlyphId, location: &Location) -> Option<Bounds> {
        self.cache.colr_clip_box(glyph.0, ColrAt::Drawn(&self.key(location))).map(Bounds::of)
    }

    /// A COLR version 1 glyph's paint graph at a location, in a palette; none for a glyph with none. A palette past
    /// the font's reads palette 0, the default.
    #[must_use]
    #[doc(alias("colr_v1_paint"))]
    pub fn paint_graph(&self, glyph: GlyphId, location: &Location, palette: u16) -> Option<PaintGraph> {
        self.cache.colr_v1_paint(glyph.0, &self.key(location), palette)
    }

    /// [`Font::paint_graph`]'s graph handed to `visitor` paint by paint, with no graph built; false for a glyph with
    /// none.
    #[doc(alias("colr_v1_visit"))]
    pub fn visit_paint_graph(
        &self, glyph: GlyphId, location: &Location, palette: u16, visitor: &mut (impl PaintVisitor + ?Sized),
    ) -> bool {
        self.cache.colr_v1_visit(glyph.0, &self.key(location), palette, visitor)
    }
}

#[cfg(test)]
#[path = "tests/unit/color.rs"]
mod tests;
