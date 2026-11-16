//! daegun's rasterizer, and what any rasterizer needs beyond outlines: subpixel layouts and the filters they sample
//! through, and an atlas packer.

pub use crate::daecore::daekit::draw::atlas::{Rect, ShelfPacker};
pub use crate::daecore::daekit::draw::subpixel::{StripeOrder, SubpixelLayout, MAX_OVERSAMPLE, MAX_TAPS, MAX_WEIGHTS};
pub use crate::daecore::daekit::draw::bitmap::{Bitmap, Image, Target, MAX_DRAW_BYTES, MAX_SIDE};
pub use crate::rasterizer::Rasterizer;
use crate::rasterizer::{Drawn, Learned, Shape, SHORT};
use crate::daecore::daekit::cache::raster::{RasterKey, OPTION_WORDS};

use alloc::collections::btree_map::Entry;
use alloc::sync::Arc;
use alloc::collections::BTreeMap;

use crate::color::{Rgba, SceneOptions, IDENTITY};
use crate::daecore::daekit::draw::srgb::darkening;
use crate::daecore::daetype::outline::simplify::{remove_overlaps, Overlaps};
use crate::daecore::daetype::outline::stroke::Join;
use crate::daecore::daetype::outline::{FillRule, Path};
use crate::outline::{checked_transform, HintMode, OutlineOptions};
use crate::{Error, Font, GlyphId, Location, Result};

/// How coverage becomes alpha.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub enum Gamma {
    /// Coverage as it is, exact: for a compositor that blends in linear light. The default.
    #[default]
    Linear,
    /// Alpha for a compositor that blends sRGB values as they are, for text of the color `text` (its alpha
    /// unread): the alpha whose blend of the text over its opposite lands where coverage would in linear light,
    /// coverage first thickened by `contrast`, 0 to 1, the more as the backdrop is lighter. Each of a subpixel
    /// layout's channels takes its own.
    Srgb {
        /// The text's color.
        text: Rgba,
        /// How much thicker, 0 for none to 1.
        contrast: f32,
    },
}

/// A distance field's kind: each pixel the signed distance from its center to the outline, inside positive, as the
/// level 127.5 + 127.5·d / `spread` rounded, so it reaches `spread` pixels each way.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Field {
    /// One channel, the distance.
    Sdf {
        /// How far it reaches, in pixels.
        spread: f32,
    },
    /// Three channels whose median is the distance, each measuring to its own edges, so corners stay sharp when it's
    /// scaled up. Where overlapping contours meet too closely to be merged, the corner they make rounds as `Sdf`'s do.
    Msdf {
        /// How far it reaches, in pixels.
        spread: f32,
    },
}

/// How [`Rasterizer::glyph`] draws: the outline's options, where the glyph sits within its first pixel, the
/// layout its coverage filters through, whether its stems darken and how its coverage becomes alpha.
#[non_exhaustive]
#[derive(Clone, Copy, Debug)]
pub struct RasterOptions<'a> {
    /// Hinting, a transform, a stroke, an embolden or a lean, as [`Font::prepared_outline`] takes them.
    pub outline: OutlineOptions,
    /// How far right and up the glyph moves before it's drawn, in pixels; a pen position's fraction, so text set
    /// between pixels keeps its spacing.
    pub offset: [f32; 2],
    /// Plain coverage, one byte a pixel, or a display's stripes.
    pub layout: &'a SubpixelLayout,
    /// Whether thin stems widen, as text drawn small reads too light otherwise: by the documented CFF curve on the
    /// font's stem in pixels, 0.4 pixels for a stem of half a pixel to none from 2⅓, the glyph emboldened by that
    /// much about its outline. Measured from `l`, else `I`, else 0.075 em; none with a stroke.
    pub darken: bool,
    /// How coverage becomes alpha: as it is by default.
    pub gamma: Gamma,
}

impl Default for RasterOptions<'_> {
    fn default() -> Self {
        RasterOptions::DEFAULT
    }
}

impl PartialEq for RasterOptions<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.outline == other.outline
            && self.offset == other.offset
            && self.layout.key() == other.layout.key()
            && self.darken == other.darken
            && self.gamma == other.gamma
    }
}

impl<'a> RasterOptions<'a> {
    /// The defaults, as `default()` gives them: the outline as the font draws it, at the pixel's corner, as plain
    /// coverage, its stems as they are and its coverage exact.
    pub const DEFAULT: RasterOptions<'static> = RasterOptions {
        outline: OutlineOptions::DEFAULT,
        offset: [0.0, 0.0],
        layout: &SubpixelLayout::grayscale(),
        darken: false,
        gamma: Gamma::Linear,
    };

    /// These options with the outline's.
    #[must_use]
    pub const fn with_outline(mut self, outline: OutlineOptions) -> Self {
        self.outline = outline;
        self
    }

    /// These options moving the glyph right and up by so many pixels.
    #[must_use]
    pub const fn with_offset(mut self, x: f32, y: f32) -> Self {
        self.offset = [x, y];
        self
    }

    /// These options filtering through a layout.
    #[must_use]
    pub const fn with_layout(mut self, layout: &'a SubpixelLayout) -> Self {
        self.layout = layout;
        self
    }

    /// These options darkening thin stems, or not.
    #[must_use]
    pub const fn with_darken(mut self, darken: bool) -> Self {
        self.darken = darken;
        self
    }

    /// These options making coverage alpha as `gamma` says.
    #[must_use]
    pub const fn with_gamma(mut self, gamma: Gamma) -> Self {
        self.gamma = gamma;
        self
    }
}

impl Rasterizer {
    /// A glyph at a location, `px` pixels to the em, as `options` say.
    ///
    /// # Errors
    ///
    /// [`Font::prepared_outline`]'s, and [`Rasterizer::path`]'s.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::raster::{RasterOptions, Rasterizer};
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let mut rasterizer = Rasterizer::new();
    /// let g = font.glyph_id('g').expect("Inter maps g");
    /// let image = rasterizer.glyph(&font, g, &Location::default(), 20.0, &RasterOptions::DEFAULT)?;
    /// // One byte of coverage a pixel, its left edge `left` pixels right of the origin and its top `top` above it.
    /// assert_eq!((image.channels, image.row(0).len()), (1, image.width as usize));
    /// assert!(image.top > 0 && image.top < image.height as i32, "g sits on the baseline and hangs below it");
    /// # Ok(())
    /// # }
    /// ```
    pub fn glyph(
        &mut self,
        font: &Font,
        glyph: GlyphId,
        location: &Location,
        px: f32,
        options: &RasterOptions,
    ) -> Result<Image<'_>> {
        let cached = font.raster_budget.get() != 0;
        let key = || bits(GLYPH, px, options, [0; 2]);
        if cached && self.hit(font, glyph, location, key) {
            return Ok(self.lent());
        }
        let outline = &self.darkened(font, location, px, options);
        // A stroke or an embolden overlaps itself by making, so it's resolved every time; a glyph's own overlaps are
        // the same at every size and under every transform, so they're learned once, in font units.
        let reshaped = outline.stroke.is_some() || outline.embolden.is_some_and(|e| e > 0.0);
        let learned = if reshaped {
            None
        } else {
            let at = self.place(font, location);
            Some(learn(&mut self.learned[at], &mut self.recency, font, glyph, location)?)
        };
        // Pixels to samples: the layout's oversampling, after the glyph moves by its offset.
        let (ox, oy) = options.layout.oversample();
        let (ox, oy) = (f64::from(ox), f64::from(oy));
        let (dx, dy) = (f64::from(options.offset[0]) * ox, f64::from(options.offset[1]) * oy);
        let exact = match learned {
            // A hinter moves the font's own points, so a hinted glyph is drawn from the font.
            Some((path, shape)) if outline.hinting == HintMode::None => {
                let s = f64::from(px / f32::from(font.units_per_em()));
                let t = checked_transform(px, outline)?.unwrap_or(IDENTITY);
                let (sx, sy) = (s * ox, s * oy);
                let m = [t[0] * sx, t[1] * sy, t[2] * sx, t[3] * sy, t[4] * sx + dx, t[5] * sy + dy];
                self.edges.begin(m.map(|v| v as f32));
                self.edges.path(path);
                *shape == Shape::Unsure
            }
            known => {
                self.edges.begin([ox as f32, 0.0, 0.0, oy as f32, dx as f32, dy as f32]);
                font.prepared_outline(glyph, location, px, outline, &mut self.edges)?;
                known.is_none_or(|(_, shape)| *shape != Shape::Plain)
            }
        };
        let drawn = self.draw(FillRule::NonZero, options.layout, exact)?;
        if let Gamma::Srgb { text, contrast } = options.gamma {
            self.correct(&drawn, text, contrast);
        }
        if cached {
            self.keep(font, glyph, location, key, &drawn);
            return Ok(self.lent());
        }
        Ok(self.image(&drawn))
    }

    /// A glyph drawn as [`glyph`](Rasterizer::glyph) draws it with a plain layout, into `target`'s pixels, one byte of
    /// coverage each, however large the target: the outline is placed and cut to the target once in f64, then worked
    /// out a band of rows at a time, each within a bounded size, so no image is held.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`](crate::ErrorKind::InvalidArgument) for a subpixel layout or pixels that don't hold the
    /// target's rows, else [`glyph`](Rasterizer::glyph)'s but for an image's size, the target's pixels then left part
    /// written.
    pub fn glyph_into(
        &mut self,
        font: &Font,
        glyph: GlyphId,
        location: &Location,
        px: f32,
        options: &RasterOptions,
        mut target: Target<'_>,
    ) -> Result<()> {
        if !options.layout.is_grayscale() {
            return Err(Error::invalid("a draw into a target takes no subpixel layout"));
        }
        if !target.holds(1) {
            return Err(SHORT);
        }
        let outline = &self.darkened(font, location, px, options);
        let reshaped = outline.stroke.is_some() || outline.embolden.is_some_and(|e| e > 0.0);
        let (dx, dy) = (f64::from(options.offset[0]), f64::from(options.offset[1]));
        let mut path = core::mem::take(&mut self.outline);
        let learned = if reshaped || outline.hinting != HintMode::None {
            None
        } else {
            let at = self.place(font, location);
            Some(learn(&mut self.learned[at], &mut self.recency, font, glyph, location).map(|(units, shape)| {
                path.clone_from(units);
                *shape
            }))
        };
        let drawn = match learned.transpose() {
            Ok(Some(shape)) => checked_transform(px, outline).map(|t| {
                let s = f64::from(px / f32::from(font.units_per_em()));
                let t = t.unwrap_or(IDENTITY);
                ([t[0] * s, t[1] * s, t[2] * s, t[3] * s, t[4] * s + dx, t[5] * s + dy], shape == Shape::Unsure)
            }),
            Ok(None) => {
                path.clear();
                let known = (!reshaped).then(|| self.place(font, location)).and_then(|at| {
                    learn(&mut self.learned[at], &mut self.recency, font, glyph, location).ok().map(|l| l.1)
                });
                font.prepared_outline(glyph, location, px, outline, &mut path)
                    .map(|_| ([1.0, 0.0, 0.0, 1.0, dx, dy], known.is_none_or(|shape| shape != Shape::Plain)))
            }
            Err(e) => Err(e),
        };
        let done = drawn.and_then(|(m, exact)| self.fill_into(&path, m, FillRule::NonZero, exact, &mut target));
        self.outline = path;
        done?;
        if let Gamma::Srgb { text, contrast } = options.gamma {
            let width = target.width as usize;
            for y in 0..target.height as usize {
                self.tone(&mut target.pixels[y * target.stride..][..width], 1, text, contrast);
            }
        }
        Ok(())
    }

    /// A color glyph's COLR scene at a location, `px` pixels to the em, in `scene`'s palette and foreground, as
    /// premultiplied sRGB RGBA composited in linear light; none for a glyph without one. It takes the options'
    /// transform, lean and offset, and its box is the glyph's clip box where it has one.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`](crate::ErrorKind::InvalidArgument) for a subpixel layout, a stroke, an embolden or
    /// hinting, [`LimitExceeded`](crate::ErrorKind::LimitExceeded) for a scene whose work would pass over each pixel
    /// it's worked out over more than 1,024 times and 2^29 besides, a fill over its box, a clip shape over its box
    /// and the frame and a layer twice over the frame, their points, cells and overlaps counted with them, and else
    /// [`Font::color_scene`]'s and [`Rasterizer::path`]'s.
    pub fn color_glyph(
        &mut self,
        font: &Font,
        glyph: GlyphId,
        location: &Location,
        px: f32,
        options: &RasterOptions,
        scene: &SceneOptions,
    ) -> Result<Option<Image<'_>>> {
        let outline = &options.outline;
        if !options.layout.is_grayscale()
            || outline.stroke.is_some()
            || outline.embolden.is_some_and(|e| e > 0.0)
            || outline.hinting != HintMode::None
        {
            return Err(Error::invalid("a color glyph takes no subpixel layout, stroke, embolden or hinting"));
        }
        let cached = font.raster_budget.get() != 0;
        let key = || bits(COLOR, px, options, [u32::from(scene.palette), u32::from_le_bytes(scene.foreground.into())]);
        if cached && self.hit(font, glyph, location, key) {
            return Ok(Some(self.lent()));
        }
        let t = checked_transform(px, outline)?.unwrap_or(IDENTITY);
        let Some(list) = font.color_scene(glyph, location, scene)? else { return Ok(None) };
        let s = f64::from(px / f32::from(font.units_per_em()));
        let (dx, dy) = (f64::from(options.offset[0]), f64::from(options.offset[1]));
        let m = [t[0] * s, t[1] * s, t[2] * s, t[3] * s, t[4] * s + dx, t[5] * s + dy];
        let clip = font.colr_clip_box(glyph, location).map(|b| [b.x_min, b.y_min, b.x_max, b.y_max]);
        let drawn = self.compose(&list, &m, clip)?;
        if cached {
            self.keep(font, glyph, location, key, &drawn);
            return Ok(Some(self.lent()));
        }
        Ok(Some(self.image(&drawn)))
    }

    /// A color glyph's scene drawn as [`color_glyph`](Rasterizer::color_glyph) draws it, into `target`'s pixels, four
    /// bytes of premultiplied sRGB RGBA each, transparent where it paints nothing, however large the target: the scene
    /// is worked out a band of the target's rows at a time, each within a bounded size. False for a glyph without a
    /// scene, the target left as it was.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`](crate::ErrorKind::InvalidArgument) for pixels that don't hold the target's rows, else
    /// [`color_glyph`](Rasterizer::color_glyph)'s but for an image's size, the target's pixels then left part written.
    #[allow(clippy::too_many_arguments, reason = "color_glyph's arguments, and the target")]
    pub fn color_glyph_into(
        &mut self,
        font: &Font,
        glyph: GlyphId,
        location: &Location,
        px: f32,
        options: &RasterOptions,
        scene: &SceneOptions,
        target: Target<'_>,
    ) -> Result<bool> {
        let outline = &options.outline;
        if !options.layout.is_grayscale()
            || outline.stroke.is_some()
            || outline.embolden.is_some_and(|e| e > 0.0)
            || outline.hinting != HintMode::None
        {
            return Err(Error::invalid("a color glyph takes no subpixel layout, stroke, embolden or hinting"));
        }
        if !target.holds(4) {
            return Err(SHORT);
        }
        let t = checked_transform(px, outline)?.unwrap_or(IDENTITY);
        let Some(list) = font.color_scene(glyph, location, scene)? else { return Ok(false) };
        let s = f64::from(px / f32::from(font.units_per_em()));
        let (dx, dy) = (f64::from(options.offset[0]), f64::from(options.offset[1]));
        let m = [t[0] * s, t[1] * s, t[2] * s, t[3] * s, t[4] * s + dx, t[5] * s + dy];
        let clip = font.colr_clip_box(glyph, location).map(|b| [b.x_min, b.y_min, b.x_max, b.y_max]);
        let region = (target.left, target.top, target.width, target.height);
        self.compose_into(&list, &m, clip, region, target.pixels, target.stride)?;
        Ok(true)
    }

    /// A glyph's distance field at a location, `px` pixels to the em, its outline as the font draws it at the origin.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`](crate::ErrorKind::InvalidArgument) for a spread or size not finite and above 0,
    /// [`LimitExceeded`](crate::ErrorKind::LimitExceeded) for an outline whose chords cross or meet each other more
    /// than 64 times a chord, else [`Font::prepared_outline`]'s and [`Rasterizer::path`]'s.
    pub fn distance_field(
        &mut self,
        font: &Font,
        glyph: GlyphId,
        location: &Location,
        px: f32,
        field: Field,
    ) -> Result<Image<'_>> {
        let (Field::Sdf { spread } | Field::Msdf { spread }) = field;
        if !(spread > 0.0 && spread.is_finite() && px > 0.0 && px.is_finite()) {
            return Err(Error::invalid("a distance field takes a spread and a size above 0 and finite"));
        }
        let multi = matches!(field, Field::Msdf { .. });
        let cached = font.raster_budget.get() != 0;
        let key = || bits(if multi { MSDF } else { SDF }, px, &RasterOptions::DEFAULT, [spread.to_bits(), 0]);
        if cached && self.hit(font, glyph, location, key) {
            return Ok(self.lent());
        }
        let at = self.place(font, location);
        learn(&mut self.learned[at], &mut self.recency, font, glyph, location)?;
        let s = f64::from(px) / f64::from(font.units_per_em());
        let drawn = self.distance(at, glyph.0, &[s, 0.0, 0.0, s, 0.0, 0.0], f64::from(spread), multi)?;
        if cached {
            self.keep(font, glyph, location, key, &drawn);
            return Ok(self.lent());
        }
        Ok(self.image(&drawn))
    }

    // Whether the font's raster cache holds an image under the key `bits` give, held to be lent if so. Cold and out of
    // line, as `keep` is, so a draw with the cache off carries neither: inline, they cost it 1% at 10 px.
    #[cold]
    fn hit(&mut self, font: &Font, glyph: GlyphId, location: &Location, bits: impl FnOnce() -> Bits) -> bool {
        let key = raster_key(font, glyph, location, bits());
        self.held = crate::sync::read(font.rasters()).get(&key).map(Arc::clone);
        self.held.is_some()
    }

    // The image just drawn, kept in the font's raster cache under the key `bits` give, and held to be lent.
    #[cold]
    fn keep(&mut self, font: &Font, glyph: GlyphId, location: &Location, bits: impl FnOnce() -> Bits, drawn: &Drawn) {
        let image = Arc::new(self.image(drawn).to_bitmap());
        let mut rasters = crate::sync::write(font.rasters());
        rasters.insert(raster_key(font, glyph, location, bits()), Arc::clone(&image));
        let over = rasters.over();
        drop(rasters);
        crate::daecore::daekit::cache::lru::settle(over);
        self.held = Some(image);
    }

    fn lent(&self) -> Image<'_> {
        self.held.as_deref().map_or_else(Image::default, Bitmap::image)
    }

    // The options' outline, emboldened by what darkening their stems takes at `px`.
    fn darkened(&mut self, font: &Font, location: &Location, px: f32, options: &RasterOptions) -> OutlineOptions {
        let mut outline = options.outline;
        if options.darken && outline.stroke.is_none() && px > 0.0 && px.is_finite() {
            let em = f32::from(font.units_per_em());
            let bold = darkening(self.stem(font, location) * px / em) * em / px;
            outline.embolden = Some(outline.embolden.filter(|&e| e > 0.0).unwrap_or(0.0) + bold);
        }
        outline
    }

    // The font's stem at a location in font units, measured once: across the middle of `l`, else `I`, else the
    // documented 0.075 em. One that doesn't draw is passed over, as one the font lacks is.
    fn stem(&mut self, font: &Font, location: &Location) -> f32 {
        let at = self.place(font, location);
        if let Some(stem) = self.learned[at].stem {
            return stem;
        }
        let mut stem = f32::from(font.units_per_em()) * 0.075;
        for c in ['l', 'I'] {
            let Some(g) = font.glyph_id(c) else { continue };
            let Ok((path, _)) = learn(&mut self.learned[at], &mut self.recency, font, g, location) else { continue };
            if let Some(across) = crate::rasterizer::across(&mut self.edges, path) {
                stem = across;
                break;
            }
        }
        self.learned[at].stem = Some(stem);
        stem
    }

    /// Sets the bytes the outlines it learned may hold: each glyph's outline in font units and what its overlaps are,
    /// learned at its first draw so later draws at any size skip both. Past it the glyphs least recently drawn go,
    /// whichever font's; 4 MB by default, and at 0 each draw learns its glyph again.
    pub fn set_cache_budget(&mut self, bytes: usize) {
        self.recency.budget = bytes;
        self.trim();
    }

    // Where a font's glyphs at a location are learned, made if new, once what's learned fits the budget.
    fn place(&mut self, font: &Font, location: &Location) -> usize {
        self.trim();
        let key = font.key(location);
        let here = |l: &Learned| l.font == font.token && l.key == *key;
        if let Some(at) = self.recency.last.filter(|&at| self.learned.get(at).is_some_and(here)) {
            return at;
        }
        let at = match self.recency.places.get(&font.token).and_then(|keys| keys.get(&*key)) {
            Some(id) => self.recency.at[id],
            None => {
                let id = crate::sync::next_id();
                self.recency.places.entry(font.token).or_default().insert((*key).clone(), id);
                self.recency.at.insert(id, self.learned.len());
                self.recency.insert((id, WHOLE), place_cost(&key));
                let (key, glyphs) = ((*key).clone(), BTreeMap::new());
                self.learned.push(Learned { id, font: font.token, key, glyphs, stem: None });
                self.learned.len() - 1
            }
        };
        self.recency.last = Some(at);
        at
    }

    // The glyphs least recently drawn let go until what's learned fits; a location goes once its last glyph has.
    fn trim(&mut self) {
        while self.recency.bytes > self.recency.budget {
            let Some(((id, glyph), cost)) = self.recency.take_oldest() else { break };
            let Some(&at) = self.recency.at.get(&id) else { continue };
            let glyphs = &mut self.learned[at].glyphs;
            match glyph {
                WHOLE if !glyphs.is_empty() => self.recency.insert((id, WHOLE), cost),
                WHOLE => self.forget(at),
                _ => {
                    glyphs.remove(&glyph);
                }
            }
        }
    }

    // A location let go, the last in its place, so every other keeps its own.
    fn forget(&mut self, at: usize) {
        let gone = self.learned.swap_remove(at);
        self.recency.at.remove(&gone.id);
        if let Some(moved) = self.learned.get(at) {
            self.recency.at.insert(moved.id, at);
        }
        if let Some(keys) = self.recency.places.get_mut(&gone.font) {
            keys.remove(&gone.key);
            if keys.is_empty() {
                self.recency.places.remove(&gone.font);
            }
        }
        self.recency.last = None;
    }
}

// What each location and glyph learned costs and when it was last drawn, by the location's number and the glyph's id,
// and the same queued by stamp: a draw only stamps its glyph, which goes round again as it comes up.
pub(crate) struct Recency {
    held: BTreeMap<(u64, u32), (usize, u64)>,
    by_age: BTreeMap<u64, (u64, u32)>,
    tick: u64,
    bytes: usize,
    budget: usize,
    // Each location's number by its font and key, where it lies in `learned` by its number, and the one placed last.
    places: BTreeMap<crate::sync::Id, BTreeMap<crate::daecore::daekit::cache::AxisKey, u64>>,
    at: BTreeMap<u64, usize>,
    last: Option<usize>,
}

// A location's own entry in `recency`, which no glyph id reaches.
const WHOLE: u32 = u32::MAX;

// About 4,500 glyphs of Source Han Sans or 6,800 of Inter, each at what it holds.
const LEARNED_BYTES: usize = 4 << 20;

impl Default for Recency {
    fn default() -> Recency {
        let (held, by_age, places, at) = (BTreeMap::new(), BTreeMap::new(), BTreeMap::new(), BTreeMap::new());
        Recency { held, by_age, tick: 0, bytes: 0, budget: LEARNED_BYTES, places, at, last: None }
    }
}

impl Recency {
    // One not held, newest.
    fn insert(&mut self, key: (u64, u32), cost: usize) {
        self.tick += 1;
        let was = self.held.insert(key, (cost, self.tick));
        debug_assert!(was.is_none(), "{key:?} was held");
        self.by_age.insert(self.tick, key);
        self.bytes += cost;
    }

    fn touch(&mut self, key: (u64, u32)) {
        self.tick += 1;
        if let Some((_, used)) = self.held.get_mut(&key) {
            *used = self.tick;
        }
    }

    // The entry least recently drawn, taken out; one drawn since it was queued is queued again at its stamp.
    fn take_oldest(&mut self) -> Option<((u64, u32), usize)> {
        loop {
            let (stamp, key) = self.by_age.pop_first()?;
            let Some(&(cost, used)) = self.held.get(&key) else { continue };
            if used > stamp {
                self.by_age.insert(used, key);
            } else {
                self.held.remove(&key);
                self.bytes -= cost;
                return Some((key, cost));
            }
        }
    }
}

// A glyph's outline as learned, its node in its location's map and its two in `recency`.
fn learned_cost(path: &Path) -> usize {
    use crate::sync::btree_entry_of;
    btree_entry_of::<u32, (Path, Shape)>() + recency_nodes() + path.cost()
}

// A location's: itself, its key twice and its nodes in `recency` and in the maps that find it.
fn place_cost(key: &crate::daecore::daekit::cache::AxisKey) -> usize {
    use crate::daecore::daekit::cache::AxisKey;
    use crate::sync::btree_entry_of;
    core::mem::size_of::<Learned>() + 2 * key.heap_bytes() + recency_nodes() + btree_entry_of::<AxisKey, u64>()
        + btree_entry_of::<u64, usize>()
}

fn recency_nodes() -> usize {
    use crate::sync::btree_entry_of;
    btree_entry_of::<(u64, u32), (usize, u64)>() + btree_entry_of::<u64, (u64, u32)>()
}

type Bits = [u32; OPTION_WORDS];

// The key a font's raster cache keeps an image under.
fn raster_key(font: &Font, glyph: GlyphId, location: &Location, bits: Bits) -> RasterKey {
    (glyph.0, font.cache.resolve_axes(&font.key(location)), bits)
}

const GLYPH: u32 = 0;
const COLOR: u32 = 1;
const SDF: u32 = 2;
const MSDF: u32 = 3;

// Every option that changes an image's pixels, as bits: the kind and the flags, then each value, 0 where unset, and
// the color glyph's scene or the field's spread.
fn bits(kind: u32, px: f32, options: &RasterOptions, tail: [u32; 2]) -> Bits {
    let o = &options.outline;
    let (join, limit) = match o.stroke.map(|s| s.join) {
        Some(Join::Miter { limit }) => (0, limit.to_bits()),
        Some(Join::Round) => (1, 0),
        Some(Join::Bevel) => (2, 0),
        None => (0, 0),
    };
    let (srgb, text, contrast) = match options.gamma {
        Gamma::Linear => (false, 0, 0),
        Gamma::Srgb { text, contrast } => (true, u32::from_le_bytes(text.into()), contrast.to_bits()),
    };
    let flags = kind
        | (o.hinting as u32) << 2
        | u32::from(o.transform.is_some()) << 5
        | u32::from(o.embolden.is_some()) << 6
        | u32::from(o.oblique.is_some()) << 7
        | u32::from(o.stroke.is_some()) << 8
        | join << 9
        | (o.stroke.map_or(0, |s| s.cap as u32)) << 11
        | u32::from(options.darken) << 13
        | u32::from(srgb) << 14;
    let t = o.transform.unwrap_or_default().map(f32::to_bits);
    let bits = |v: Option<f32>| v.map_or(0, f32::to_bits);
    let layout = options.layout.key();
    [
        flags,
        px.to_bits(),
        options.offset[0].to_bits(),
        options.offset[1].to_bits(),
        t[0],
        t[1],
        t[2],
        t[3],
        t[4],
        t[5],
        bits(o.embolden),
        bits(o.oblique),
        bits(o.stroke.map(|s| s.width)),
        limit,
        layout as u32,
        (layout >> 32) as u32,
        text,
        contrast,
        tail[0],
        tail[1],
    ]
}

// A glyph's outline in font units and what its overlaps are, learned at its first draw.
fn learn<'a>(
    learned: &'a mut Learned, recency: &mut Recency, font: &Font, glyph: GlyphId, location: &Location,
) -> Result<&'a (Path, Shape)> {
    Ok(match learned.glyphs.entry(glyph.0) {
        Entry::Occupied(known) => {
            recency.touch((learned.id, glyph.0));
            &*known.into_mut()
        }
        Entry::Vacant(slot) => {
            let mut units = Path::new();
            let em = f32::from(font.units_per_em());
            font.prepared_outline(glyph, location, em, &OutlineOptions::DEFAULT, &mut units)?;
            let (mut path, shape) = match remove_overlaps(&units) {
                Overlaps::None => (units, Shape::Plain),
                Overlaps::Removed(removed) => (removed, Shape::Removed),
                Overlaps::Unsure => (units, Shape::Unsure),
            };
            path.shrink_to_fit();
            recency.insert((learned.id, glyph.0), learned_cost(&path));
            &*slot.insert((path, shape))
        }
    })
}
