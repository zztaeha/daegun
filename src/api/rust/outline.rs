//! Outlines: glyphs drawn in font units, hinted in pixels and prepared for a rasterizer, and the path tools around
//! them.

#[cfg(doc)]
use crate::ErrorKind::{InvalidArgument, NoOutline, Overflow};
use super::{draw_glyph_outline, Font, FontCache, Location};
use alloc::vec::Vec;
use crate::daecore::daekit::cache::AxisKey;
use crate::daecore::daetype::hinting::auto::{AutoPoints, CollectPen};
use crate::daecore::daetype::hinting::f26dot6::Size;
use crate::daecore::daetype::hinting::{FLAG_CUBIC, FLAG_ON_CURVE};
use crate::daecore::daetype::outline::stroke::{Ink, Stroke};
use crate::daecore::daekit::base::error::Result;
use crate::daecore::{Error, ErrorKind, GlyphId};

pub use crate::daecore::daetype::hinting::HintMode;
pub use crate::daecore::daetype::outline::flatten::{max_area_for, MAX_FLATTEN_POINTS};
pub use crate::daecore::daetype::outline::quadratic::{normalize_winding, Quad, QuadraticPen};
pub use crate::daecore::daetype::outline::simplify::resolve_overlaps;
pub use crate::daecore::daetype::outline::{
    Cap, CffHints, CffMask, CffStem, FillRule, Join, OutlinePen, Path, PathView, Point, StrokeStyle, TransformPen,
    Verb,
};

/// What [`Font::prepared_outline`] does to an outline, in font units: hinting, then the oblique and the transform,
/// then a stroke or an embolden. Either overlaps itself, which a rasterizer summing coverage needs
/// [`resolve_overlaps`] for.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct OutlineOptions {
    /// A 2×3 transform `[a, b, c, d, dx, dy]`, its offsets in font units, applied after hinting. It scales neither a
    /// stroke, an embolden nor the advances.
    pub transform: Option<[f32; 6]>,
    /// Which hinter, if any.
    pub hinting: HintMode,
    /// A stroke in place of the fill, its width in font units.
    pub stroke: Option<StrokeStyle>,
    /// Extra stem width in font units, which widens the advances too. Ignored with a stroke, which takes its
    /// place, and at zero or below.
    pub embolden: Option<f32>,
    /// The tangent of a lean from upright, positive to the right, applied before the transform so the glyph
    /// leans in its own frame.
    pub oblique: Option<f32>,
}

impl Default for OutlineOptions {
    fn default() -> OutlineOptions {
        OutlineOptions::DEFAULT
    }
}

impl OutlineOptions {
    /// The defaults, as `default()` gives them: the outline as the font draws it.
    pub const DEFAULT: OutlineOptions =
        OutlineOptions { transform: None, hinting: HintMode::None, stroke: None, embolden: None, oblique: None };

    /// These options with a transform.
    #[must_use]
    pub const fn with_transform(mut self, transform: [f32; 6]) -> OutlineOptions {
        self.transform = Some(transform);
        self
    }

    /// These options with a hinter.
    #[must_use]
    pub const fn with_hinting(mut self, hinting: HintMode) -> OutlineOptions {
        self.hinting = hinting;
        self
    }

    /// These options stroking the outline.
    #[must_use]
    pub const fn with_stroke(mut self, stroke: StrokeStyle) -> OutlineOptions {
        self.stroke = Some(stroke);
        self
    }

    /// These options emboldening the outline by so many font units.
    #[must_use]
    pub const fn with_embolden(mut self, units: f32) -> OutlineOptions {
        self.embolden = Some(units);
        self
    }

    /// These options leaning the outline.
    #[must_use]
    pub const fn with_oblique(mut self, tangent: f32) -> OutlineOptions {
        self.oblique = Some(tangent);
        self
    }
}

/// What [`Font::prepared_outline`] drew: the advances in pixels, an embolden widening both. Laid out as C reads it.
#[non_exhaustive]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct PreparedGlyph {
    /// The advance across.
    pub advance_width: f32,
    /// The advance down; 0 for a font without vertical metrics.
    pub advance_height: f32,
    /// Whether a hinter ran.
    pub hinted: bool,
}

/// A glyph's outline as a hinter left it, from the glyph's origin, y up.
#[derive(Clone, PartialEq, Debug)]
pub struct HintedOutline(pub(crate) crate::daecore::daetype::hinting::HintedOutline);

/// What a point of a hinted outline is: on the curve, or a control of a curve of one kind.
#[expect(clippy::exhaustive_enums, reason = "a point is on its curve or controls a quadratic or a cubic")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PointKind {
    /// On the curve.
    #[doc(alias("FLAG_ON_CURVE"))]
    OnCurve,
    /// A quadratic curve's control, as TrueType's are.
    Quadratic,
    /// One of a cubic curve's two controls, as CFF's are.
    #[doc(alias("FLAG_CUBIC"))]
    Cubic,
}

impl HintedOutline {
    /// Each point in pixels, with what it is.
    pub fn points(
        &self,
    ) -> impl ExactSizeIterator<Item = (Point, PointKind)> + DoubleEndedIterator + core::iter::FusedIterator + Clone {
        let px = |v: i32| v as f32 / 64.0;
        let points = self.0.x.iter().zip(&self.0.y).map(move |(&x, &y)| Point { x: px(x), y: px(y) });
        let kind = |&flags: &u8| match flags {
            _ if flags & FLAG_ON_CURVE != 0 => PointKind::OnCurve,
            _ if flags & FLAG_CUBIC != 0 => PointKind::Cubic,
            _ => PointKind::Quadratic,
        };
        points.zip(self.0.flags.iter().map(kind))
    }

    /// Where each contour ends, as the index of its last point.
    pub fn contours(&self) -> &[usize] {
        &self.0.contour_ends
    }

    /// Draws it onto a pen, in pixels.
    #[doc(alias("draw_hinted"))]
    pub fn draw(&self, pen: &mut (impl OutlinePen + ?Sized)) {
        crate::daecore::daetype::hinting::draw_hinted(&self.0, pen);
    }
}

const BAD_INPUT: Error = Error::invalid("px must be finite and above zero, and the options finite");

// The transform `opts` give an outline in font units, its lean first, if any; px and the options checked.
pub(crate) fn checked_transform(px: f32, opts: &OutlineOptions) -> Result<Option<[f64; 6]>> {
    if !(px.is_finite() && px > 0.0) {
        return Err(BAD_INPUT);
    }
    if opts.transform.is_some_and(|t| !t.iter().all(|v| v.is_finite()))
        || opts.stroke.is_some_and(|s| !s.width.is_finite())
        || opts.embolden.is_some_and(|u| !u.is_finite())
    {
        return Err(BAD_INPUT);
    }
    // The oblique's shear goes in before the caller's own transform, so a glyph leans in its own frame.
    let given = opts.transform.map(|t| t.map(f64::from));
    match opts.oblique {
        None => Ok(given),
        Some(k) if k.is_finite() => {
            use crate::color::{concat, IDENTITY};
            Ok(Some(concat(&[1.0, 0.0, f64::from(k), 1.0, 0.0, 0.0], &given.unwrap_or(IDENTITY))))
        }
        Some(_) => Err(BAD_INPUT),
    }
}
const OVERFLOW: Error = Error::new(ErrorKind::Overflow, "an advance, width or point overflows once scaled");

impl Font {
    /// Draws a glyph's outline at a location onto a pen, in font units, y up, from its origin.
    ///
    /// # Errors
    ///
    /// The decoder's, for a glyph that doesn't decode, by which time the pen may hold part of it.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::outline::Path;
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// if let Some(g) = font.glyph_id('g') {
    ///     let mut path = Path::default();
    ///     font.outline(g, &Location::default(), &mut path)?;
    ///     // `path` holds g's contours in font units; `path.replay` draws them onto a pen of yours.
    ///     # assert!(!path.is_empty());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias("outline_glyph", "outline_glyph_instanced"))]
    pub fn outline(&self, glyph: GlyphId, location: &Location, pen: &mut dyn OutlinePen) -> Result<()> {
        self.outline_glyph_drawn(glyph.0, &self.key(location), pen)
    }

    /// A glyph hinted at `px` pixels to the em at a location; none where no hinter takes it: [`HintMode::None`], a
    /// px past 65,535, a glyph too large to hint at that size, or one hinting leaves where scaling puts it, as one
    /// its font's own hints leave alone. Draw it unhinted then, as [`Font::prepared_outline`] does.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`] unless px is finite and above zero, as [`Font::prepared_outline`] gives it.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::outline::{HintMode, Path};
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let g = font.glyph_id('g').expect("Inter maps g");
    /// let mut path = Path::default();
    /// match font.hinted_outline(g, &Location::default(), 13.0, HintMode::Auto)? {
    ///     // In pixels, y up, fitted to the grid at 13 pixels to the em.
    ///     Some(hinted) => hinted.draw(&mut path),
    ///     // No hinter takes it: draw it unhinted, as prepared_outline does.
    ///     None => {}
    /// }
    /// # assert!(!path.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    #[doc(alias("hinted_glyph"))]
    pub fn hinted_outline(
        &self,
        glyph: GlyphId,
        location: &Location,
        px: f32,
        mode: HintMode,
    ) -> Result<Option<HintedOutline>> {
        if !(px > 0.0 && px.is_finite()) {
            return Err(BAD_INPUT);
        }
        Ok(self.hint_with(glyph.0, px, &self.key(location), mode, core::mem::take).map(HintedOutline))
    }

    /// A CFF glyph's stem hints and masks at a location, a CFF2 font's blended there as its outline is; none for a
    /// glyph that isn't CFF or doesn't decode.
    pub fn cff_hints(&self, glyph: GlyphId, location: &Location) -> Option<CffHints> {
        self.cache.at_location(&self.key(location), |at| cff_hinted_points(at, glyph.0).map(|(hints, _)| hints))
    }

    /// A glyph's outline at a location as quadratic curves in font units, cubics split to within 1/4096 em, wound by
    /// [`normalize_winding`].
    ///
    /// # Errors
    ///
    /// The decoder's, or [`QuadraticPen::finish`]'s: [`NoOutline`] for a glyph that draws nothing, such as a space.
    #[doc(alias("glyph_quads"))]
    pub fn quads(&self, glyph: GlyphId, location: &Location) -> Result<Vec<Quad>> {
        let mut pen = QuadraticPen::new(self.quad_tolerance());
        self.outline(glyph, location, &mut pen)?;
        let mut curves = pen.finish()?;
        normalize_winding(&mut curves);
        Ok(curves)
    }

    /// The same into `curves`, whose room is reused, so a list kept between glyphs allocates only to grow; emptied
    /// on an error.
    ///
    /// # Errors
    ///
    /// As [`quads`](Font::quads) gives them.
    pub fn quads_into(&self, glyph: GlyphId, location: &Location, curves: &mut Vec<Quad>) -> Result<()> {
        let mut pen = QuadraticPen::reusing(self.quad_tolerance(), core::mem::take(curves));
        let drawn = self.outline(glyph, location, &mut pen);
        let (kept, finished) = pen.finish_apart();
        *curves = kept;
        let verdict = drawn.and(finished);
        match verdict {
            Ok(()) => normalize_winding(curves),
            Err(_) => curves.clear(),
        }
        verdict
    }

    /// Whether glyf flags the glyph as one whose contours or components may overlap: OVERLAP_SIMPLE on it or a glyph
    /// it's built from, or OVERLAP_COMPOUND on a component, which a rasterizer may take as a cue to antialias with
    /// more care. False for a glyph CFF, CFF2 or VARC draws, which carry no such flag, and for one glyf leaves
    /// unflagged, whose contours may overlap all the same.
    pub fn has_overlap_flag(&self, glyph: GlyphId) -> bool {
        self.cache.overlap_flagged(glyph.0)
    }

    fn quad_tolerance(&self) -> f32 {
        f32::from(self.units_per_em()) * crate::daecore::daetype::outline::quadratic::CUBIC_TOLERANCE
    }

    /// A glyph's outline at a location for a rasterizer of your own: hinted, leaned, transformed, stroked or
    /// emboldened as the options say, then onto the pen in pixels at `px` to the em, y up, from the glyph's
    /// origin.
    ///
    /// # Errors
    ///
    /// [`InvalidArgument`] unless px is finite and above zero and the options finite, [`Overflow`] for anything past
    /// f32 once scaled, drawing nothing, and else the decoder's.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::outline::{HintMode, OutlineOptions, Path};
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let gid = font.glyph_id('a').expect("the font maps a");
    /// let mut path = Path::new();
    /// let options = OutlineOptions::default().with_hinting(HintMode::Auto);
    /// let glyph = font.prepared_outline(gid, &Location::default(), 16.0, &options, &mut path)?;
    /// // path holds the hinted outline in pixels; glyph.advance_width is the pen's step after it
    /// # assert!(glyph.hinted && !path.is_empty());
    /// # Ok(())
    /// # }
    /// ```
    pub fn prepared_outline(
        &self,
        glyph: GlyphId,
        location: &Location,
        px: f32,
        options: &OutlineOptions,
        pen: &mut (impl OutlinePen + ?Sized),
    ) -> Result<PreparedGlyph> {
        // Generic, so a pen's calls inline into the scaling pen: 41 ns of a 336 ns glyph for a Rust pen, and most of
        // C's 24 ns over Rust's for its bridge.
        let mut path = take_path();
        let prepared = self.prepare_in(glyph.0, px, &self.key(location), options, pen, &mut path);
        give_path(path);
        prepared
    }

    pub(crate) fn prewarmed_outline(
        &self,
        gid: u32,
        axes: &crate::sync::Shared<crate::sync::Lined<crate::daecore::daekit::cache::ResolvedAxes>>,
    ) -> Option<crate::sync::Shared<Path>> {
        crate::sync::write(&self.outlines().list)
            .get(&(gid, crate::sync::Shared::clone(axes)))
            .map(crate::sync::Shared::clone)
    }

    pub(crate) fn outline_glyph_keyed(&self, gid: u32, axes: &AxisKey, pen: &mut dyn OutlinePen) -> Option<()> {
        self.outline_glyph_drawn(gid, axes, pen).ok()
    }

    // The decoder's reason when a glyph does not draw. The pen may already hold part of it.
    pub(crate) fn outline_glyph_drawn(&self, gid: u32, axes: &AxisKey, pen: &mut dyn OutlinePen) -> Result<()> {
        self.outline_glyph_counted(gid, axes, &mut 0, pen)
    }

    // As `outline_glyph_drawn`, the decoding it took, drawn or not, added to `work`.
    pub(crate) fn outline_glyph_counted(
        &self, gid: u32, axes: &AxisKey, work: &mut usize, pen: &mut dyn OutlinePen,
    ) -> Result<()> {
        // A prewarmed outline replays instead of decoding. Nothing is interned unless something was
        // prewarmed, so a caller who never prewarms pays one read of a count.
        if self.outlines.get().is_some_and(|o| o.prewarmed.get() != 0)
            && let Some(path) = self.prewarmed_outline(gid, &self.cache.resolve_axes(axes))
        {
            path.replay(None, pen);
            return Ok(());
        }
        self.cache.at_location(axes, |at| draw_glyph_outline(at, gid, work, pen))
    }

    fn prepare_in<P: OutlinePen + ?Sized>(
        &self,
        gid: u32,
        px: f32,
        axes: &AxisKey,
        opts: &OutlineOptions,
        pen: &mut P,
        path: &mut Path,
    ) -> Result<PreparedGlyph> {
        let transform = checked_transform(px, opts)?;
        let s = px / f32::from(self.units_per_em());

        // Stroked in font units unless hinted, as the stroker's thresholds are absolute, then scaled into pixels on
        // the way to the pen. A hinted outline is drawn as the hinter left it, into this path alone.
        let hinted = self
            .hint_with(gid, px, axes, opts.hinting, |out| match transform {
                None => crate::daecore::daetype::hinting::draw_hinted(out, path),
                Some(t) => {
                    let m = [t[0], t[1], t[2], t[3], t[4] * f64::from(s), t[5] * f64::from(s)];
                    crate::daecore::daetype::hinting::draw_hinted(out, &mut TransformPen::new(path, m));
                }
            })
            .is_some();
        let (to_px, units_to_path, tolerance) = match (hinted, transform) {
            (true, _) => (1.0, s, 0.25),
            (false, None) => {
                // Where nothing needs the whole path (no stroke or embolden, no scale past 1 to check), a glyph that
                // draws whole goes straight to the pen, as the path would replay it, and the path stays empty.
                let unstyled = opts.stroke.is_none() && !opts.embolden.is_some_and(|u| u > 0.0);
                let drawn = if unstyled && s <= 1.0 && self.cache.at_location(axes, |at| at.draws_whole(gid)) {
                    let scale = f64::from(s);
                    let mut scaled = TransformPen::new(&mut *pen, [scale, 0.0, 0.0, scale, 0.0, 0.0]);
                    self.outline_glyph_drawn(gid, axes, &mut scaled)
                } else {
                    self.outline_glyph_drawn(gid, axes, path)
                };
                drawn?;
                (s, 1.0, 0.25 * (f32::from(self.units_per_em()) / px))
            }
            (false, Some(t)) => {
                self.outline_glyph_drawn(gid, axes, &mut TransformPen::new(path, t))?;
                (s, 1.0, 0.25 * (f32::from(self.units_per_em()) / px))
            }
        };

        // A font with no vertical metrics answers 0 for the height, which an embolden must leave at 0.
        let bold = opts.embolden.filter(|u| *u > 0.0 && opts.stroke.is_none());
        let grow = bold.unwrap_or(0.0);
        let both = |at: &FontCache| (at.advances(false).of(gid), at.advances(true).of(gid));
        let (width, height) = self.cache.described_at(axes, both);
        let height = height as f32;
        let glyph = PreparedGlyph {
            advance_width: (width as f32 + grow) * s,
            advance_height: if self.cache.has_vertical_metrics() { height + grow } else { height } * s,
            hinted,
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
        let fits = |(x, y): (f32, f32)| fits_scaled(x, to_px) && fits_scaled(y, to_px);
        if !(glyph.advance_width.is_finite() && glyph.advance_height.is_finite())
            || style.is_some_and(|st| !st.width.is_finite())
            || (transform.is_some() || to_px > 1.0) && !path.parts().1.iter().all(|&p| fits(p.into()))
        {
            return Err(OVERFLOW);
        }
        let ink = if opts.stroke.is_some() { Ink::Stroke } else { Ink::Embolden };
        let stroked = style.map(|st| Stroke::new(path, &st, tolerance, ink, fits).ok_or(OVERFLOW)).transpose()?;

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

    // What `take` makes of the glyph hinted, by whichever hinter takes it. The bytecode hinter's and the autohinter's
    // outlines are their own, lent for the call: `take` moves it out or draws it, but doesn't call back into the font.
    pub(super) fn hint_with<R>(
        &self, gid: u32, px: f32, axes: &AxisKey, mode: HintMode,
        take: impl FnMut(&mut crate::daecore::daetype::hinting::HintedOutline) -> R,
    ) -> Option<R> {
        if mode == HintMode::None { return None; }
        let size = Size::from_px(px)?;
        self.cache.at_location(axes, |cache| self.hinted_at(cache, gid, size, axes, mode, take))
    }

    fn hinted_at<R>(
        &self,
        cache: &FontCache,
        gid: u32,
        size: Size,
        axes: &AxisKey,
        mode: HintMode,
        mut take: impl FnMut(&mut crate::daecore::daetype::hinting::HintedOutline) -> R,
    ) -> Option<R> {
        let upm = self.units_per_em();
        // A glyph VARC draws is never hinted, as VARC holds no hints for the glyph it composes; one it covers but
        // leaves to its own outline is hinted as that outline is.
        if cache.varc_draws(gid) {
            return None;
        }
        let mode = match mode {
            HintMode::Auto if cache.autohints_under_auto() => HintMode::AutoForce,
            mode => mode,
        };
        if mode != HintMode::AutoForce
            && let Some(glyf) = cache.glyf()
            && let Some(loca) = cache.loca_offsets()
            && let Some(out) = cache.hint_glyph_cached(glyf, loca.into(), gid, size, upm, mode, &mut take)
        {
            return Some(out);
        }

        if matches!(mode, HintMode::Font | HintMode::Auto) {
            return Self::cff_hinted_outline(cache, gid, size, upm).map(|mut out| take(&mut out));
        }
        if mode != HintMode::AutoForce { return None; }

        let pts = self.collected_points(gid, axes)?;
        if pts.len() > crate::daecore::daetype::format::glyf::MAX_POINTS || !scales_inside(&pts, size, upm) {
            return None;
        }
        if let Some(out) = cache.try_autohint(&pts, size, &mut take) { return out; }
        self.ensure_autohinter(cache, axes);
        cache.try_autohint(&pts, size, &mut take)?
    }

    // A CFF or CFF2 glyph as the CFF hinter places it; None for one that draws nothing.
    fn cff_hinted_outline(cache: &FontCache, gid: u32, size: Size, upm: u16)
        -> Option<crate::daecore::daetype::hinting::HintedOutline>
    {
        let scale16 = size.scale16(upm);
        let out = match cache.hint_cff2(gid, scale16) {
            Some(out) => out,
            None => {
                let outlines = cache.cff_outlines()?;
                crate::daecore::daetype::outline::hint_cff_glyph(outlines, cache.cff()?, gid, scale16)
            }
        };
        out.filter(|o| !o.x.is_empty())
    }

    fn ensure_autohinter(&self, cache: &FontCache, axes: &AxisKey) {
        use crate::daecore::daetype::hinting::auto::AutoHinter;
        let upm = self.units_per_em();
        let mut resolve = |c: char| cache.glyph_id(c as u32);
        let mut outline_of = |gid: u32| self.collected_points(gid, axes);
        let zones = AutoHinter::compute_zones(upm, &mut resolve, &mut outline_of);
        cache.set_autohinter(AutoHinter::from_zones(zones, upm));
    }

    // A glyph's points at `axes` as the autohinter reads them; None where it draws nothing.
    fn collected_points(&self, gid: u32, axes: &AxisKey) -> Option<AutoPoints> {
        let mut pen = CollectPen::new();
        self.outline_glyph_keyed(gid, axes, &mut pen)?;
        let pts = pen.finish();
        (!pts.is_empty()).then_some(pts)
    }
}

// A CFF glyph's hints, and the pen that collected the points they apply to, unfinished for a caller
// that wants only the hints.
fn cff_hinted_points(cache: &FontCache, gid: u32) -> Option<(CffHints, CollectPen)> {
    let mut pen = CollectPen::new();
    if let Some(drawn) = cache.draw_cff2(gid, Some(CffHints::default()), &mut 0, &mut pen) {
        return Some((drawn.ok()?.unwrap_or_default(), pen));
    }
    let (cff, outlines) = (cache.cff()?, cache.cff_outlines()?);
    let hints =
        crate::daecore::daetype::outline::outline_cff_glyph_hinted(outlines, cff, gid, &mut pen).ok()?;
    Some((hints, pen))
}

// As `fits` answers over the points, the farthest found on the bits in one vector walk, as a float's magnitude orders
// as its bits do and a NaN's past infinity's.
fn scales_inside(pts: &AutoPoints, size: Size, upm: u16) -> bool {
    let reach = pts.x.iter().chain(&pts.y).fold(0, |r, v| r.max(v.to_bits() & 0x7FFF_FFFF));
    crate::daecore::daetype::hinting::f26dot6::fits([f64::from(f32::from_bits(reach))], size, upm)
}

// The path the thread's last prepared outline grew, lent to its next; one past `SPARE_PATH_BYTES` is dropped.
#[cfg(feature = "std")]
std::thread_local! {
    static SPARE_PATH: core::cell::Cell<Option<Path>> = const { core::cell::Cell::new(None) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE_PATH.try_with(core::cell::Cell::take);
}

#[cfg(feature = "std")]
const SPARE_PATH_BYTES: usize = 64 << 10;

fn take_path() -> Path {
    #[cfg(feature = "std")]
    return SPARE_PATH.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_default();
    #[cfg(not(feature = "std"))]
    Path::default()
}

// A thread past its locals' teardown drops the path instead.
fn give_path(mut path: Path) {
    path.clear();
    #[cfg(feature = "std")]
    if path.heap_bytes() <= SPARE_PATH_BYTES {
        let _ = SPARE_PATH.try_with(|spare| spare.set(Some(path)));
    }
}

// What the pen gets is the point times `scale`, in f64 and then cast back to f32.
fn fits_scaled(v: f32, scale: f32) -> bool {
    (f64::from(v) * f64::from(scale)).abs() <= f64::from(f32::MAX)
}

#[cfg(test)]
#[path = "tests/unit/outline.rs"]
mod tests;
