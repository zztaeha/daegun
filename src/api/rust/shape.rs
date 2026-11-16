//! Shaping: text to glyphs placed in font units, and the carets hit testing reads off them.

use super::{Font, Location};
use crate::daecore::daeshaper::unicode::bidi::BaseDirection;
use crate::daecore::{GlyphId, Orientation};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ops::Range;

pub use crate::daecore::daeshaper::run::{ClusterLevel, Direction};
pub use crate::daecore::daeshaper::script::Shaper;
pub use crate::text::shape::{Feature, Ignorables, Offset, ShapeOptions, ShapedRun};

/// One of [`Font::shape_bidi`]'s runs: its shaping, its bidi level and the text it covers.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BidiRun {
    /// The run as shaped, its clusters from the run's own start.
    pub run: Arc<ShapedRun>,
    /// Its bidi level: odd right to left.
    pub level: u8,
    /// The text it covers, as byte offsets.
    pub bytes: core::range::Range<usize>,
}

impl Font {
    /// Shapes text at a location. A run asked for with no more than a direction, context, a language and a seed
    /// script comes from the font's shape cache, shared with every caller that asks for it again. Threads shaping on
    /// one font don't wait for the cache: where it's busy, a run is shaped and kept by a later call.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::shape::ShapeOptions;
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let run = font.shape("Wave", &Location::default(), &ShapeOptions::DEFAULT);
    /// let mut x = 0.0;
    /// for ((glyph, advance), offset) in run.glyphs.iter().zip(&run.advances).zip(&run.offsets) {
    ///     // Draw `glyph` at (x + offset.x, offset.y), in font units: a mark's offset moves it onto its base.
    ///     # let _ = (glyph, x + offset.x, offset.y);
    ///     x += advance;
    /// }
    /// # assert!((x - run.width()).abs() < 1e-9);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[doc(alias("shape_with_features", "shape_with_language", "shape_with_options"))]
    pub fn shape(&self, text: &str, location: &Location, options: &ShapeOptions) -> Arc<ShapedRun> {
        let key = self.key(location);
        match options.cached() {
            Some((vertical, ctx)) => self.cache.shaped_run_in_context(&key, text, vertical, &ctx),
            None => crate::text::shape::shape_run(&self.cache, &key, text, false, None, options),
        }
    }

    // `shape` over text C gave, read as UTF-8 only where the cache doesn't hold its run, as a held run's text was:
    // None where it isn't UTF-8, as an `Option` returns in a register where a `Utf8Error` went through memory.
    #[cfg(feature = "capi")]
    pub(crate) fn shape_bytes(
        &self, text: &[u8], location: &Location, options: &ShapeOptions,
    ) -> Option<Arc<ShapedRun>> {
        let key = self.key(location);
        match options.cached() {
            Some((vertical, ctx)) => self.cache.shaped_bytes_in_context(&key, text, vertical, &ctx),
            None => {
                let text = core::str::from_utf8(text).ok()?;
                Some(crate::text::shape::shape_run(&self.cache, &key, text, false, None, options))
            }
        }
    }

    /// Shapes text at a location as [`Font::shape`] does, into `run`, whose vectors keep their room: shaping run
    /// after run into one allocates only where a run outgrows the last. It neither reads nor fills the shape cache.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::shape::{ShapeOptions, ShapedRun};
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let mut run = ShapedRun::default();
    /// for word in ["one", "two", "three"] {
    ///     font.shape_into(word, &Location::default(), &ShapeOptions::DEFAULT, &mut run);
    ///     // `run` holds this word's glyphs, in the room the last word left.
    ///     # assert!(!run.glyphs.is_empty());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn shape_into(&self, text: &str, location: &Location, options: &ShapeOptions, run: &mut ShapedRun) {
        crate::text::shape::shape_run_into(&self.cache, &self.key(location), text, options, run);
    }

    /// Shapes text at a location as runs in visual order: each bidi run cut where its script changes, as layout
    /// cuts lines, and shaped with its own direction, script and context. Options that name a script keep each
    /// bidi run whole, and a seed script stands in for a run with no script of its own; their direction isn't
    /// read.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::shape::ShapeOptions;
    /// use daegun::unicode::BaseDirection;
    /// use daegun::Location;
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(
    /// #     env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/scheherazade-new/ScheherazadeNew-Regular.ttf"
    /// # );
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let text = "Read مرحبا aloud";
    /// for bidi in font.shape_bidi(text, &Location::default(), BaseDirection::Auto, &ShapeOptions::DEFAULT) {
    ///     // Left to right as drawn: `&text[bidi.bytes]` is what it shows, an odd `bidi.level` right to left.
    ///     # assert!(!bidi.run.glyphs.is_empty());
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    #[doc(alias("shape_bidi_with"))]
    pub fn shape_bidi(
        &self, text: &str, location: &Location, base: BaseDirection, options: &ShapeOptions,
    ) -> Vec<BidiRun> {
        let key = self.key(location);
        self.bidi_runs(text, base, options.script.is_none(), |slice, rtl, script, before, after| {
            // The caller's seed stands in for a run with no script of its own, Common or a whole bidi run.
            let seed_script = match script {
                Some(s) if s.is_context_dependent() => options.seed_script.or(script),
                Some(_) => script,
                None => options.seed_script,
            };
            // Each run takes the direction bidi resolved for it, from the shape cache where `shape` would.
            let opts = ShapeOptions { before, after, seed_script, direction: None, ..*options };
            match opts.cached() {
                Some((_, ctx)) => {
                    let ctx = crate::daecore::daekit::cache::RunContext { rtl: Some(rtl), ..ctx };
                    self.cache.shaped_run_in_context(&key, slice, false, &ctx)
                }
                None => crate::text::shape::shape_run(&self.cache, &key, slice, false, Some(rtl), &opts),
            }
        })
        .into_iter()
        .map(|(run, level, bytes)| BidiRun { run, level, bytes: bytes.into() })
        .collect()
    }

    // Bidi runs in visual order, cut where the script changes unless the caller names one. Each shapes in logical
    // order, as the shaper reverses RTL itself, with the text around it as context so joining crosses its edges.
    pub(crate) fn bidi_runs<'t, R>(
        &self,
        text: &'t str,
        base: BaseDirection,
        by_script: bool,
        mut shape: impl FnMut(&'t str, bool, Option<crate::text::Script>, &'t str, &'t str) -> R,
    ) -> Vec<(R, u8, Range<usize>)> {
        let para = crate::daeshaper::unicode::bidi::resolve(text, base.rtl());
        let pieces = by_script.then(|| crate::text::pieces::Pieces::new(text, &para.levels, (0, text.len()), |_| true));
        let mut out = Vec::new();
        for (bytes, level) in crate::daeshaper::unicode::bidi::visual_runs(&para, text).iter() {
            let parts: Vec<(Option<crate::text::Script>, usize, usize)> = match &pieces {
                Some(pieces) => {
                    pieces.split(bytes, level).into_iter().map(|(p, s, e)| (p.map(|p| p.script), s, e)).collect()
                }
                None => alloc::vec![(None, bytes.start, bytes.end)],
            };
            for (script, s, e) in parts {
                out.push((shape(&text[s..e], level % 2 == 1, script, &text[..s], &text[e..]), level, s..e));
            }
        }
        out
    }

    /// The caret before each byte of the text at a location, then the text's trailing edge: `text.len() + 1` of
    /// them, every byte of a character at its leading edge, so any byte offset indexes it. From the text's left
    /// edge, or its top in vertical text. Text across a line is shaped as [`Font::shape_bidi`] shapes it, so the
    /// carets sit where those runs draw; vertical text is one run, top to bottom. A ligature splits at its carets.
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
    /// let text = "Wave";
    /// let carets = font.caret_positions(text, &Location::default(), Orientation::Horizontal);
    /// assert_eq!(carets.len(), text.len() + 1);
    /// // carets[2] sits between "Wa" and "ve", in font units from the left.
    /// # assert!(carets[2] > carets[1]);
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn caret_positions(&self, text: &str, location: &Location, orientation: Orientation) -> Vec<f64> {
        let key = self.key(location);
        let vertical = orientation.is_vertical();
        let ligature = |glyph: GlyphId| {
            let top = match vertical {
                true => self.cache.described_at(&key, |at| at.vertical_origin_units(glyph.0)).into(),
                false => 0.0,
            };
            (self.carets(glyph.0, &key, vertical, false), top)
        };
        if vertical {
            return run_carets(&self.cache.shaped_run(&key, text, true), text, false, true, ligature);
        }
        // Shaped as `shape_bidi` shapes, through the shape cache: hit testing asks again and again.
        let runs = self.bidi_runs(text, BaseDirection::Auto, true, |slice, rtl, seed_script, before, after| {
            let ctx = crate::daecore::daekit::cache::RunContext {
                rtl: Some(rtl), before, after, seed_script, ..Default::default()
            };
            (self.cache.shaped_run_in_context(&key, slice, false, &ctx), slice)
        });
        let n = text.len();
        let mut out = alloc::vec![0.0; n + 1];
        let mut x = 0.0;
        for ((run, slice), level, bytes) in runs {
            let carets = run_carets(&run, slice, level % 2 == 1, false, ligature);
            for (i, &c) in bytes.clone().zip(&carets) {
                out[i] = x + c;
            }
            if bytes.end == n {
                out[n] = x + carets[bytes.len()];
            }
            x += run.width();
        }
        out
    }
}

// The caret before each byte from the run's left (or top), a character's bytes at its leading edge, then the trailing
// edge. Graphemes split a cluster at a lone glyph's carets, else evenly, clamped in order.
fn run_carets(
    run: &ShapedRun, text: &str, rtl: bool, vertical: bool,
    ligature: impl Fn(GlyphId) -> (Vec<Option<f64>>, f64),
) -> Vec<f64> {
    struct Extent { left: f64, right: f64, glyphs: usize, glyph: GlyphId }
    // A run's glyphs of one cluster sit together, so each extent grows in place; a sort then puts the clusters in
    // order, and one met again apart merges, its glyph then unread as it has more than one.
    let mut starts: Vec<(usize, Extent)> = Vec::with_capacity(run.glyphs.len());
    let mut x = 0.0;
    for ((&cluster, &advance), &glyph) in run.clusters.iter().zip(&run.advances).zip(&run.glyphs) {
        let cluster = cluster as usize;
        if starts.last().is_none_or(|(c, _)| *c != cluster) {
            starts.push((cluster, Extent { left: x, right: x, glyphs: 0, glyph }));
        }
        if let Some((_, e)) = starts.last_mut() {
            e.left = e.left.min(x);
            e.right = e.right.max(x + advance);
            e.glyphs += 1;
        }
        x += advance;
    }
    starts.sort_unstable_by_key(|&(c, _)| c);
    starts.dedup_by(|(c, later), (kept_c, kept)| {
        let same = c == kept_c;
        if same {
            kept.left = kept.left.min(later.left);
            kept.right = kept.right.max(later.right);
            kept.glyphs += later.glyphs;
        }
        same
    });
    let trailing = if rtl { 0.0 } else { x };

    let n = text.len();
    let graphemes = crate::text::grapheme_boundaries(text);
    starts.truncate(starts.partition_point(|&(c, _)| c < n));
    let mut out = Vec::with_capacity(n + 1);
    // Clusters and grapheme boundaries both ascend, so one cursor finds each cluster's inner boundaries.
    let mut g = 0;
    for (k, (first, e)) in starts.iter().enumerate() {
        let next = starts.get(k + 1).map_or(n, |s| s.0);
        while graphemes.get(g).is_some_and(|&b| b <= *first) { g += 1; }
        let from = g;
        while graphemes.get(g).is_some_and(|&b| b < next) { g += 1; }
        let inner = &graphemes[from..g];
        let (carets, top) = if e.glyphs == 1 && !inner.is_empty() { ligature(e.glyph) } else { (Vec::new(), 0.0) };
        let edges = Edges { left: e.left, right: e.right, top, rtl, vertical };
        let mut edge = if rtl { e.right } else { e.left };
        // Characters no glyph stands for, before the first cluster, sit at its edge.
        out.resize(*first, edge);
        let mut part = 0;
        for i in *first..next {
            if inner.get(part) == Some(&i) {
                part += 1;
                let at = edges.boundary(part, inner.len() + 1, &carets);
                edge = if rtl { at.clamp(e.left, edge) } else { at.clamp(edge, e.right) };
            }
            out.push(edge);
        }
    }
    out.resize(n, trailing);
    out.push(trailing);
    out
}

// A ligature glyph's place in a run, for finding where its components meet.
struct Edges {
    left: f64,
    right: f64,
    top: f64,
    rtl: bool,
    vertical: bool,
}

impl Edges {
    // Where the k-th of `span` components ends: carets run up the glyph, so right to left or down a
    // line that is the k-th caret from the end. A missing caret falls evenly between the edges.
    fn boundary(&self, k: usize, span: usize, carets: &[Option<f64>]) -> f64 {
        let index = if self.rtl || self.vertical { carets.len().checked_sub(k) } else { k.checked_sub(1) };
        let even = (self.right - self.left) * k as f64 / span as f64;
        match index.and_then(|j| carets.get(j).copied().flatten()) {
            Some(c) if self.vertical => self.left + self.top - c,
            Some(c) => self.left + c,
            None if self.rtl => self.right - even,
            None => self.left + even,
        }
    }
}

#[cfg(test)]
#[path = "tests/unit/shape.rs"]
mod tests;
