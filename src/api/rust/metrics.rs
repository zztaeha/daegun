//! A font's metrics in font units, y up: line spacing, OS/2's, post's and STAT's, and tracking.

use super::{Font, FontCache, Location};
use crate::daecore::Orientation;
use crate::daecore::daetype::decoder::{FS_BOLD, FS_ITALIC, FS_OBLIQUE, FS_REGULAR, FS_USE_TYPO_METRICS};
use alloc::string::String;
use alloc::vec::Vec;

pub use crate::cache::{LineMetrics, PdfFlags};
pub use crate::daecore::daetype::stat::{StatAxis, StatAxisValue};
pub use crate::daecore::daetype::trak::DEFAULT_POINT_SIZE;

/// What OS/2 says of the font's style and vertical metrics.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
#[doc(alias("Os2Info"))]
pub struct Os2 {
    /// The table's version.
    pub version: u16,
    /// usWeightClass, from 1 to 1000 as CSS's font-weight counts: 400 regular, 700 bold. None where the table
    /// stops before it.
    pub weight_class: Option<u16>,
    /// usWidthClass, from 1, ultra-condensed, to 9, ultra-expanded, 5 being normal. None where the table stops
    /// before it.
    pub width_class: Option<u16>,
    /// fsType, the embedding the font's license allows: 0 installable, bit 1 (0x2) restricted, bit 2 (0x4) preview
    /// and print, bit 3 (0x8) editable, bit 8 (0x100) not subset, bit 9 (0x200) bitmaps only. None where the table
    /// stops before it.
    pub fs_type: Option<u16>,
    /// sFamilyClass, none where the table is too short to hold it.
    pub family_class: Option<u16>,
    /// fsSelection, none where the table is too short to hold it.
    pub selection: Option<u16>,
    /// usWinAscent and usWinDescent, none where the table stops before them.
    pub win_metrics: Option<WinMetrics>,
    /// sTypoAscender, sTypoDescender and sTypoLineGap, none where the table stops before them.
    pub typo_metrics: Option<LineMetrics>,
}

impl Os2 {
    fn selected(&self, bit: u16) -> bool {
        self.selection.is_some_and(|v| v & bit != 0)
    }

    /// Whether fsSelection marks the font italic.
    pub fn is_italic(&self) -> bool {
        self.selected(FS_ITALIC)
    }

    /// Whether fsSelection marks the font bold.
    pub fn is_bold(&self) -> bool {
        self.selected(FS_BOLD)
    }

    /// Whether fsSelection marks the font regular.
    pub fn is_regular(&self) -> bool {
        self.selected(FS_REGULAR)
    }

    /// Whether fsSelection marks the font oblique.
    pub fn is_oblique(&self) -> bool {
        self.selected(FS_OBLIQUE)
    }

    /// Whether fsSelection asks for the typo metrics to space lines.
    pub fn uses_typo_metrics(&self) -> bool {
        self.selected(FS_USE_TYPO_METRICS)
    }
}

/// OS/2's Windows clipping metrics, both distances from the baseline, so the descent is positive below it.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct WinMetrics {
    /// usWinAscent, up from the baseline.
    pub ascent: f64,
    /// usWinDescent, down from the baseline.
    pub descent: f64,
}

/// A line drawn under or through text: its position from the baseline, y up, and its thickness.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct DecorationLine {
    /// From the baseline to the line, negative below it.
    pub position: f64,
    /// How thick it is drawn.
    pub thickness: f64,
}

/// The size and offset OS/2 gives a subscript or a superscript.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
#[doc(alias("SubSuperMetrics"))]
pub struct ScriptMetrics {
    /// The horizontal size.
    pub x_size: f64,
    /// The vertical size.
    pub y_size: f64,
    /// The horizontal offset.
    pub x_offset: f64,
    /// The vertical offset, from the baseline.
    pub y_offset: f64,
}

/// OS/2's and post's measures for setting text, each none where the font has no field for it.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct TypographicMetrics {
    /// OS/2's sxHeight, which version 2 added.
    pub x_height: Option<f64>,
    /// post's underline.
    pub underline: Option<DecorationLine>,
    /// OS/2's strikeout.
    pub strikeout: Option<DecorationLine>,
    /// OS/2's subscript.
    pub subscript: Option<ScriptMetrics>,
    /// OS/2's superscript.
    pub superscript: Option<ScriptMetrics>,
}

/// STAT: the font's design axes and the values it names on them, for putting names together.
#[non_exhaustive]
#[derive(Debug, PartialEq, Clone)]
#[doc(alias("StatInfo"))]
pub struct Stat {
    /// The design axes, in STAT's order.
    pub axes: Vec<StatAxis>,
    /// The named values.
    pub values: Vec<StatAxisValue>,
    /// The name to use where every value's name is elided.
    pub elided_fallback_name: Option<String>,
}

impl Font {
    // `f` on the font's tables at a location, as `instance` builds them there, and on its own at the default.
    fn described<T>(&self, location: &Location, f: impl FnOnce(&FontCache) -> T) -> T {
        self.cache.described_at(&self.key(location), f)
    }

    /// hhea's ascender at a location; 0 without hhea.
    pub fn ascender(&self, location: &Location) -> f64 {
        self.described(location, FontCache::font_ascender)
    }

    /// hhea's descender at a location, negative below the baseline; 0 without hhea.
    pub fn descender(&self, location: &Location) -> f64 {
        self.described(location, FontCache::font_descender)
    }

    /// OS/2's cap height at a location, else the ascender where OS/2 gives none or 0.
    #[inline]
    pub fn cap_height(&self, location: &Location) -> f64 {
        self.described(location, FontCache::font_cap_height)
    }

    /// head's box around every glyph, as stored; none where head is too short to hold it.
    #[doc(alias("bbox"))]
    pub fn bounds(&self) -> Option<crate::daecore::Bounds> {
        self.cache.font_bbox()
    }

    /// The line spacing at a location: OS/2's typo metrics where it asks for them, else hhea's or vhea's, then
    /// OS/2's, then a default of the em.
    pub fn line_metrics(&self, location: &Location, orientation: Orientation) -> LineMetrics {
        self.described(location, |fc| fc.line_metrics(orientation.is_vertical()))
    }

    /// The flags a PDF font descriptor would set for the font at a location.
    #[doc(alias("flags"))]
    pub fn pdf_flags(&self, location: &Location) -> PdfFlags {
        self.described(location, FontCache::font_flags)
    }

    /// post's italic angle at a location, in degrees counterclockwise from upright; 0 without post.
    pub fn italic_angle(&self, location: &Location) -> f64 {
        self.described(location, FontCache::font_italic_angle)
    }

    /// OS/2 at a location, none without the table.
    #[doc(alias("os2_info"))]
    pub fn os2(&self, location: &Location) -> Option<Os2> {
        self.described(location, |fc| {
            let os2 = fc.os2()?;
            Some(Os2 {
                version: os2.version,
                weight_class: os2.us_weight_class,
                width_class: os2.us_width_class,
                fs_type: os2.fs_type,
                family_class: os2.s_family_class,
                selection: os2.fs_selection,
                win_metrics: os2.win_metrics.map(|w| WinMetrics { ascent: w.ascent.into(), descent: w.descent.into() }),
                typo_metrics: os2.line_metrics.map(|m| LineMetrics {
                    ascent: m.ascender.into(),
                    descent: m.descender.into(),
                    line_gap: m.line_gap.into(),
                }),
            })
        })
    }

    /// What trak adds between glyphs at a point size, one way; 0 without trak or a track for that way. A size not
    /// finite and above zero reads at [`DEFAULT_POINT_SIZE`].
    pub fn tracking(&self, point_size: f64, orientation: Orientation) -> f64 {
        let horizontal = !orientation.is_vertical();
        crate::daecore::daetype::trak::tracking(self.cache.font_tables(), point_size, horizontal)
    }

    /// OS/2's and post's measures for setting text at a location.
    pub fn typographic_metrics(&self, location: &Location) -> TypographicMetrics {
        self.described(location, |fc| {
            let script = |[x_size, y_size, x_offset, y_offset]: [i16; 4]| ScriptMetrics {
                x_size: x_size.into(),
                y_size: y_size.into(),
                x_offset: x_offset.into(),
                y_offset: y_offset.into(),
            };
            let os2 = fc.os2();
            let block = os2.and_then(|o| o.metrics);
            let read = |p, at| crate::daecore::daetype::decoder::read_i16_be(p, at).map(f64::from);
            let underline = fc.table("post").and_then(|p| {
                Some(DecorationLine { position: read(p, 8)?, thickness: read(p, 10)? })
            });
            TypographicMetrics {
                x_height: os2.and_then(|o| o.sx_height).map(f64::from),
                underline,
                strikeout: block.map(|m| DecorationLine {
                    position: m.y_strikeout_position.into(),
                    thickness: m.y_strikeout_size.into(),
                }),
                subscript: block.map(|m| script(m.subscript)),
                superscript: block.map(|m| script(m.superscript)),
            }
        })
    }

    /// STAT, with its names as the name table at a location gives them; none without STAT or one that doesn't
    /// read.
    #[doc(alias("stat_info"))]
    pub fn stat(&self, location: &Location) -> Option<Stat> {
        let parse = crate::daecore::daetype::stat::parse_stat;
        let (axes, values, elided_fallback_name) = self.cache.names_at(&self.key(location), parse).ok()?;
        Some(Stat { axes, values, elided_fallback_name })
    }

    // The same with each axis, value and name made by C's closures, as `parse_stat_with` takes them.
    #[cfg(feature = "capi")]
    pub(crate) fn stat_with<N, F: FnMut(u16) -> Option<N>, A, V>(
        &self,
        location: &Location,
        names: impl FnOnce(&crate::daecore::daetype::Tables, usize) -> F,
        axis: impl FnMut(crate::Tag, Option<N>, u16) -> A,
        value: impl FnMut(crate::daecore::daetype::stat::ValueKind, Option<N>, bool, bool) -> V,
    ) -> Option<crate::daecore::daetype::stat::StatInfo<A, V, N>> {
        let parse = |tables: &_| crate::daecore::daetype::stat::parse_stat_with(tables, names, axis, value);
        self.cache.names_at(&self.key(location), parse).ok()
    }
}
