use super::*;
use crate::daecore::daetype::format::sanitize::OtlTable;
use crate::daecore::daetype::subsetter::otl::gdef::var_store;
use crate::daecore::daetype::instancer::{VariedAdvances, VariedOrigins};
use crate::daecore::daetype::vorg::Vorg;
use crate::daecore::daekit::base::sync::GlyphSlots;
use crate::daecore::daetype::format::metrics::MetricsTable;
use crate::daecore::daetype::format::glyf::{
    glyph_record, simple_point_count, ComponentPath, Components, Stack, GLYPH_WORK, MAX_VISITS,
    MAX_POINTS, USE_MY_METRICS,
};

/// The flags a PDF font descriptor sets, as PDF numbers their bits.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct PdfFlags(u32);

impl PdfFlags {
    /// Every glyph has the same width: post's isFixedPitch.
    pub const FIXED_PITCH: PdfFlags = PdfFlags(0x01);
    /// The glyphs have serifs, by OS/2's family class.
    pub const SERIF: PdfFlags = PdfFlags(0x02);
    /// A symbol font, or one that maps a character outside the Adobe standard Latin set.
    pub const SYMBOLIC: PdfFlags = PdfFlags(0x04);
    /// The glyphs look handwritten, by OS/2's family class.
    pub const SCRIPT: PdfFlags = PdfFlags(0x08);
    /// The font maps no character outside the Adobe standard Latin set.
    pub const NONSYMBOLIC: PdfFlags = PdfFlags(0x20);
    /// post gives an italic angle.
    pub const ITALIC: PdfFlags = PdfFlags(0x40);

    /// The flags as the number a PDF writes.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: PdfFlags) -> bool {
        self.0 & other.0 == other.0
    }

    /// No flags.
    #[must_use]
    pub const fn empty() -> PdfFlags {
        PdfFlags(0)
    }

    /// Whether no flag is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl core::ops::BitAnd for PdfFlags {
    type Output = PdfFlags;
    fn bitand(self, other: PdfFlags) -> PdfFlags {
        PdfFlags(self.0 & other.0)
    }
}

impl core::ops::BitAndAssign for PdfFlags {
    fn bitand_assign(&mut self, other: PdfFlags) {
        self.0 &= other.0;
    }
}

impl core::fmt::Binary for PdfFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Binary::fmt(&self.0, f)
    }
}

impl core::fmt::Octal for PdfFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Octal::fmt(&self.0, f)
    }
}

impl core::fmt::LowerHex for PdfFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::LowerHex::fmt(&self.0, f)
    }
}

impl core::fmt::UpperHex for PdfFlags {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::UpperHex::fmt(&self.0, f)
    }
}

impl core::ops::BitOr for PdfFlags {
    type Output = PdfFlags;
    fn bitor(self, other: PdfFlags) -> PdfFlags {
        PdfFlags(self.0 | other.0)
    }
}

impl core::ops::BitOrAssign for PdfFlags {
    fn bitor_assign(&mut self, other: PdfFlags) {
        self.0 |= other.0;
    }
}

/// A font's line spacing in font units, y up: `descent` lies below the baseline, so is usually negative.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Default)]
#[doc(alias("TypoLineMetrics"))]
pub struct LineMetrics {
    /// From the baseline up to the top of a line.
    pub ascent: f64,
    /// From the baseline down to the bottom of a line, negative below it.
    pub descent: f64,
    /// The gap a font asks for between one line's descent and the next one's ascent.
    pub line_gap: f64,
}

impl LineMetrics {
    /// From one baseline to the next: ascent less descent, plus the gap.
    pub fn line_height(&self) -> f64 {
        self.ascent - self.descent + self.line_gap
    }
}

// A glyph's box in font units: the top left corner, then a width and a height that runs down, so negative.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct GlyphExtents {
    pub(crate) x_bearing: i32,
    pub(crate) y_bearing: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

impl GlyphExtents {
    // Extents from a drawn box, each edge rounded by `round` and the size taken from the rounded edges, so the box
    // keeps whole-unit edges, one of no area included, as glyf stores it.
    pub(crate) fn from_bounds(bounds: [f64; 4], round: fn(f64) -> f64) -> GlyphExtents {
        let [x_min, y_min, x_max, y_max] = bounds.map(round);
        GlyphExtents {
            x_bearing: x_min as i32,
            y_bearing: y_max as i32,
            width: (x_max - x_min) as i32,
            height: (y_min - y_max) as i32,
        }
    }
}

// hhea's ascender, descender and line gap in font units, or vhea's, from the header shaping reads (HHEA's where whole);
// a bitmap-only font without one, as Apple's are, takes its largest strike's line metrics.
pub(super) fn line_extents_of(
    table_map: &Tables, upm: u16, bitmap_only: bool, vertical: bool,
) -> Option<(i32, i32, i32)> {
    let Some(hea) = crate::daecore::daetype::format::metrics::shaped_header(table_map, vertical) else {
        let strike = || crate::daecore::daetype::bitmap::strike_line_extents(table_map, upm, vertical);
        return bitmap_only.then(strike).flatten().map(|(a, d)| (a, d, 0));
    };
    let gap = read_i16_be(hea, 8).map_or(0, i32::from);
    Some((read_i16_be(hea, 4)?.into(), read_i16_be(hea, 6)?.into(), gap))
}

impl FontCache {
    pub fn font_ascender(&self) -> f64 {
        self.line_extents(false).map_or(0.0, |(a, _, _)| f64::from(a))
    }

    pub fn font_descender(&self) -> f64 {
        self.line_extents(false).map_or(0.0, |(_, d, _)| f64::from(d))
    }

    fn line_extents(&self, vertical: bool) -> Option<(i32, i32, i32)> {
        let read = || line_extents_of(self.tables(), self.upm, self.bitmap_only, vertical);
        *self.line_extents[usize::from(vertical)].get_or_init(read)
    }

    pub fn font_cap_height(&self) -> f64 {
        match self.os2().and_then(|o| o.s_cap_height) {
            Some(cap) if cap != 0 => f64::from(cap),
            _ => self.font_ascender(),
        }
    }

    // None where head is too short to hold the box.
    pub fn font_bbox(&self) -> Option<crate::daecore::Bounds> {
        let field = |at| font_header(self.tables()).and_then(|h| read_i16_be(h, at)).map(f64::from);
        Some(crate::daecore::Bounds { x_min: field(36)?, y_min: field(38)?, x_max: field(40)?, y_max: field(42)? })
    }

    pub fn font_flags(&self) -> PdfFlags {
        let mut flags = if self.is_symbolic() { PdfFlags::SYMBOLIC } else { PdfFlags::NONSYMBOLIC };

        let is_fixed_pitch = self.table("post")
            .and_then(|p| read_u32_be(p, 12))
            .is_some_and(|v| v != 0);
        if is_fixed_pitch { flags |= PdfFlags::FIXED_PITCH; }

        let family_class = self.os2()
            .and_then(|o| o.s_family_class)
            .unwrap_or(0);
        let class_id = (family_class >> 8) as u8;
        let is_serif  = matches!(class_id, 1 | 2 | 3 | 4 | 5 | 7);
        let is_script = class_id == 10;
        if is_serif  { flags |= PdfFlags::SERIF; }
        if is_script { flags |= PdfFlags::SCRIPT; }

        if self.font_italic_angle() != 0.0 { flags |= PdfFlags::ITALIC; }

        flags
    }

    // PDF's rule: a Windows symbol font, or one that maps a character outside the Adobe standard Latin set.
    // A control code names no character, so mapping one doesn't count.
    fn is_symbolic(&self) -> bool {
        let Some(cmap) = self.table("cmap") else { return false };
        let outside_latin = |code: u32| {
            let standard =
                u16::try_from(code).is_ok_and(|c| super::standard_latin::STANDARD_LATIN.binary_search(&c).is_ok());
            !standard && !char::from_u32(code).is_some_and(char::is_control)
        };
        crate::daecore::daetype::subsetter::cmap_is_symbol(cmap)
            || self.cmap_coverage().into_iter().any(|(code, _)| outside_latin(code))
    }

    pub fn font_italic_angle(&self) -> f64 {
        self.table("post")
            .filter(|p| p.len() >= 16)
            .and_then(|p| crate::daecore::daetype::decoder::read_fixed(p, 4))
            .unwrap_or(0.0)
    }

    pub fn font_num_glyphs(&self) -> u32 {
        self.num_glyphs
    }

    pub fn cff(&self) -> Option<&crate::daecore::daetype::TableBytes> {
        self.cff.as_ref()
    }

    pub(crate) fn glyf(&self) -> Option<&crate::daecore::daetype::TableBytes> {
        self.glyf.as_ref()
    }

    pub fn os2(&self) -> Option<crate::daecore::daetype::decoder::Os2Fields> {
        *self.os2.get_or_init(|| crate::daecore::daetype::decoder::parse_os2(self.tables()))
    }

    pub fn glyph_in_range(&self, gid: u32) -> Option<u32> {
        (gid < self.num_glyphs).then_some(gid)
    }

    pub fn font_upm(&self) -> u16 {
        self.upm
    }

    pub fn line_metrics(&self, vertical: bool) -> LineMetrics {
        let upm = self.font_upm() as f64;

        let hea = self.line_extents(vertical).filter(|&(a, d, _)| a - d > 0);
        let hea = hea.map(|(a, d, g)| (f64::from(a), f64::from(d), f64::from(g)));
        if vertical {
            let (a, d, g) = hea.unwrap_or((upm / 2.0, -upm / 2.0, 0.0));
            return LineMetrics { ascent: a, descent: d, line_gap: g };
        }

        let os2 = self.os2();
        let typo = os2.and_then(|o| o.line_metrics).filter(|m| m.ascender as i32 - m.descender as i32 > 0);
        let (a, d, g) = if os2.is_some_and(|o| o.use_typo_metrics()) {
            typo.map(|m| (m.ascender as f64, m.descender as f64, m.line_gap as f64))
        } else {
            None
        }
        .or(hea)
        .or_else(|| typo.map(|m| (m.ascender as f64, m.descender as f64, m.line_gap as f64)))
        .or_else(|| {
            os2.and_then(|o| o.win_metrics)
                .filter(|w| w.ascent as u32 + w.descent as u32 > 0)
                .map(|w| (w.ascent as f64, -(w.descent as f64), 0.0))
        })
        .unwrap_or((upm * 0.8, -upm * 0.2, 0.0));

        LineMetrics { ascent: a, descent: d, line_gap: g }
    }

    pub(crate) fn vertical_origin_units(&self, gid: u32) -> i32 {
        self.vertical_origins().of(gid)
    }

    // Where vertical origins come from, settled once a pass: VORG; for a glyf font with a glyph and vertical metrics,
    // the top phantom point, measured off the default; else the box centered between ascender and descender.
    pub(crate) fn vertical_origins(&self) -> VerticalOrigins<'_> {
        if let Some(vorg) = self.vorg_origins() {
            return vorg;
        }
        let vmtx = self.metrics_table(true).filter(|v| v.has_data());
        if let (Some(vmtx), Some(glyf), Some(loca)) = (vmtx, &self.glyf, self.loca_offsets())
            && loca.len() > 1
        {
            return match self.measured.as_deref() {
                Some(measured) => VerticalOrigins::Measured(measured),
                None => {
                    let extended = self.glyf_extended;
                    VerticalOrigins::Phantom { glyf, extended, loca, vmtx, upm: i32::from(self.upm) }
                }
            };
        }
        let (ascender, descender) = self.font_extents_units();
        VerticalOrigins::Boxed { extents: self.extents_source(), ascender, descender }
    }

    // The font extents, ascender up and descender down whatever their signs: OS/2's typographic ones under
    // USE_TYPO_METRICS, else hhea's or the strike's standing in for it; with neither, 0.8 em above and the rest below.
    fn font_extents_units(&self) -> (i32, i32) {
        let typo = self.os2().filter(|o| o.use_typo_metrics()).and_then(|o| o.line_metrics);
        let hhea = self.line_extents(false).map(|(a, d, _)| (a, d));
        match typo.map(|m| (i32::from(m.ascender), i32::from(m.descender))).or(hhea) {
            Some((a, d)) => (a.abs(), -d.abs()),
            None => {
                let ascender = i32::from(self.upm) * 4 / 5;
                (ascender, ascender - i32::from(self.upm))
            }
        }
    }

    pub(crate) fn glyph_extents(&self, gid: u32) -> Option<GlyphExtents> {
        self.extents_source().of(gid)
    }

    // What answers glyph extents, looked up once for a run of glyphs; a table it skips would answer none.
    pub(crate) fn extents_source(&self) -> ExtentsSource<'_> {
        let has = |tag| self.has_table(tag);
        ExtentsSource {
            cache: self,
            color_bitmaps: has("sbix") || has("CBLC"),
            colr: self.colr_drawn(),
            varc: self.varc_loads(),
            measured: self.measured.as_deref().or_else(|| self.stored_measured()),
        }
    }

    // glyf's glyphs measured as stored, as a location measures them, so a header box that's stale goes unread.
    fn stored_measured(&self) -> Option<&crate::daecore::daetype::instancer::MeasuredGlyphs> {
        use crate::daecore::daetype::instancer::MeasuredGlyphs;
        let build = || self.glyf_tables().map(|tables| MeasuredGlyphs::new(tables.clone(), &[]));
        self.drawn.is_none().then(|| once_slot(&self.stored_measured, build, |_| {}).as_ref()).flatten()
    }

    // The box of the points the glyph's outline draws, unhinted at this cache's location: None inside where it draws
    // none, None where it doesn't draw. What drawing it took, drawn or not, and the points it drew add to `work`; at a
    // glyf location, what its walks spent.
    pub(crate) fn drawn_bounds(&self, gid: u32, work: &mut usize) -> Option<Option<[f64; 4]>> {
        use crate::daecore::daetype::outline::{outline_cff_glyph_counted, PointBounds};
        if let Some(measured) = self.measured.as_deref() {
            return measured.bounds(gid, work);
        }
        if self.cff2 {
            return self.cff2_bounds_at(gid, self.location.as_deref().map_or(&[][..], |l| &l.coords[..]), work);
        }
        let mut pen = PointBounds::default();
        let drawn = match (self.cff.as_ref(), self.cff_outlines()) {
            (Some(cff), Some(outlines)) => outline_cff_glyph_counted(outlines, cff, gid, work, &mut pen).is_ok(),
            (Some(_), None) => false,
            (None, _) => {
                self.loca_offsets().is_some_and(|loca| self.draw_glyf(loca.into(), gid, work, &mut pen).is_ok())
            }
        };
        *work = work.saturating_add(pen.points);
        drawn.then(|| pen.finish().map(|(x0, y0, x1, y1)| [x0, y0, x1, y1].map(f64::from)))
    }

    // A glyf glyph's points as TrueType numbers them, a composite's components placed in order, from where it's drawn
    // from: its drawn origin at the default, or off it as measured there.
    pub(crate) fn contour_points(&self, gid: u32) -> Option<(Vec<f64>, Vec<f64>)> {
        use crate::daecore::daetype::{instancer::GlyphCoords, outline::Stored};
        if let Some(measured) = self.measured.as_deref() {
            return measured.points(gid);
        }
        let (glyf, loca) = (self.glyf.as_ref()?, self.loca_offsets()?);
        let points = crate::daecore::daetype::outline::glyph_points(glyf, self.glyf_extended, loca.into(), gid);
        let (mut xs, ys) = points.ok()?;
        let origin = self.origin(glyf, loca.into(), gid, &Stored, &mut GlyphCoords::default()).ok()?;
        xs.iter_mut().for_each(|x| *x -= f64::from(origin));
        Some((xs, ys))
    }

    // As the advance reader has it: a vmtx with no vhea to count its entries gives every height as 0.
    pub(crate) fn has_vertical_metrics(&self) -> bool {
        self.metrics_table(true).is_some_and(|v| v.has_data())
    }

    // A vertical advance as read, 0 included: vmtx's, or a bitmap-only font's big metrics. None for a glyph
    // with neither, which advances `vertical_fallback_advance`.
    pub(crate) fn stated_vertical_advance(&self, gid: u32, read: u32) -> Option<u32> {
        let bitmap = || crate::daecore::daetype::bitmap::bitmap_advance(self.tables(), gid, self.upm, true).is_some();
        (read != 0 || self.has_vertical_metrics() || self.bitmap_only && bitmap()).then_some(read)
    }

    // The advance down a line without vmtx: the font extents' height, 0 included.
    pub(crate) fn vertical_fallback_advance(&self) -> u32 {
        let (ascender, descender) = self.font_extents_units();
        ascender.abs_diff(descender)
    }

    // A glyf font's own tables for measuring glyphs off the default, shared by every location. None where glyf holds no
    // readable glyph, a head of unknown format too: advances and origins then come from metrics.
    pub(crate) fn glyf_tables(&self) -> Option<&Shared<crate::daecore::daetype::instancer::GlyfTables>> {
        use crate::daecore::{daetype::instancer::GlyfTables, daekit::base::sync::shared_of};
        let build = || {
            let loca = self.loca_offsets().filter(|l| l.len() > 1)?;
            GlyfTables::load(self.tables(), loca.clone()).map(Shared::new)
        };
        let charge = |kept: &Option<Shared<GlyfTables>>| {
            self.held.built.add(kept.as_deref().map_or(0, |t| shared_of::<GlyfTables>() + t.heap_bytes()));
        };
        once_slot(&self.glyf_tables, build, charge).as_ref()
    }

    // The tables glyph names come from: post, and a CFF's charset for the glyphs its CharStrings count.
    pub(crate) fn name_tables(&self) -> (Option<&[u8]>, Option<&[u8]>) {
        (self.table("post").map(|t| t.as_slice()), self.cff.as_ref().map(|t| t.as_slice()))
    }

    // Where the font's names lie, kept in `slot` from the first name asked for and charged to the font. The slot is
    // the font's own, not every location's, as names don't vary.
    pub(crate) fn name_index<'a>(
        &self, slot: &'a Once<crate::daecore::daetype::glyph_names::NameIndex>,
    ) -> &'a crate::daecore::daetype::glyph_names::NameIndex {
        let build = || {
            let (post, cff) = self.name_tables();
            crate::daecore::daetype::glyph_names::NameIndex::new(post, cff)
        };
        once_slot(slot, build, |kept| self.held.built.add(kept.heap_bytes()))
    }

    // Where each glyph glyf holds starts, and where the last ends: loca within its length, by indexToLocFormat 0 or 1,
    // no more glyphs than maxp counts. Behind one pointer, so the slot costs a font without loca no more.
    pub fn loca_offsets(&self) -> Option<&GlyfLoca> {
        use crate::daecore::daekit::base::sync::shared_of;
        let build = || self.loca_list().map(Shared::new);
        // The list reads loca where it lies, so the slot holds only the list's own fields.
        let charge = |kept: &Option<Shared<GlyfLoca>>| {
            if kept.is_some() {
                self.held.built.add(shared_of::<GlyfLoca>());
            }
        };
        once_slot(&self.loca, build, charge).as_deref()
    }

    // Whether drawing `gid` reads all of it before its first pen call, so a glyph that fails draws nothing: one of
    // contours, in a font whose outlines glyf alone draws.
    pub(crate) fn draws_whole(&self, gid: u32) -> bool {
        use crate::daecore::daetype::{decoder::read_i16_be, format::glyf::glyph_bytes};
        let (Some(glyf), Some(loca)) = (&self.glyf, self.loca_offsets()) else { return false };
        !self.font.varc
            && self.cff.is_none()
            && !self.cff2
            && glyph_bytes(glyf, loca.into(), gid).and_then(|g| read_i16_be(g, 0)).is_some_and(|n| n > 0)
    }

    // Whether glyf draws `gid` and flags it as possibly overlapping.
    pub(crate) fn overlap_flagged(&self, gid: u32) -> bool {
        use crate::daecore::daetype::format::glyf::overlap_flagged;
        let (Some(glyf), Some(loca)) = (&self.glyf, self.loca_offsets()) else { return false };
        let mut visits = MAX_VISITS;
        self.cff.is_none()
            && !self.cff2
            && !self.varc_draws(gid)
            && overlap_flagged(glyf, loca.into(), gid, self.glyf_extended, &mut visits)
    }

    // Whether Auto hints the whole font with the autohinter: TrueType outlines with no font program and a CVT program
    // of at most 7 bytes, room for a dropout setting alone. Any other font is left to its own hints.
    pub fn autohints_under_auto(&self) -> bool {
        let len = |tag| self.table(tag).map_or(0, |t| t.len());
        len("fpgm") == 0 && len("prep") <= 7 && self.cff.is_none() && !self.cff2 && self.glyf.is_some()
            && self.loca_offsets().is_some_and(|l| !l.is_empty())
    }

    fn loca_list(&self) -> Option<GlyfLoca> {
        use crate::daecore::daetype::format::glyf::{glyf_glyph_count, loca_format};
        let format = loca_format(self.table("head")?)?;
        let loca = self.table(crate::daecore::daetype::decoder::glyf_tags(self.tables()).loca)?;
        let entries = glyf_glyph_count(loca.len(), format, self.num_glyphs as usize) + 1;
        Some(GlyfLoca::new(loca.clone(), format, entries))
    }

    pub fn cff_outlines(&self) -> Option<&crate::daecore::daetype::outline::CffOutlines> {
        use crate::daecore::daetype::{hinting::cff::EmBox, outline::CffOutlines};
        use crate::daecore::daekit::base::sync::shared_of;
        let parse = |cff| CffOutlines::parse_in(cff, self.upm, EmBox::read(&self.font.tables, self.upm)).ok();
        let build = || self.cff.as_ref().and_then(|cff| parse(cff)).map(Shared::new);
        let charge = |kept: &Option<Shared<CffOutlines>>| {
            self.held.built.add(kept.as_deref().map_or(0, |o| shared_of::<CffOutlines>() + o.heap_bytes()));
        };
        once_slot(&self.cff_outlines_cache, build, charge).as_deref()
    }

    // A CFF2 font's charstrings, whether or not drawing reads them. The font keeps them, so no location is charged.
    pub(crate) fn cff2_parsed(&self) -> Option<&crate::daecore::daetype::outline::Cff2Outlines> {
        use crate::daecore::{daetype::hinting::cff::EmBox, daetype::outline::Cff2Outlines};
        let parse = |cff2| Cff2Outlines::parse_in(cff2, self.upm, EmBox::read(&self.font.tables, self.upm)).ok();
        let build = || self.table("CFF2").and_then(|t| parse(t)).map(Shared::new);
        once_slot(&self.font.cff2_outlines, build, |_| {}).as_deref()
    }

    // The charstrings as drawing reads them, where their FDSelect and variation store load.
    pub(crate) fn cff2_outlines(&self) -> Option<&crate::daecore::daetype::outline::Cff2Outlines> {
        self.cff2_parsed().filter(|o| o.drawable())
    }

    // A CFF2 glyph's box as drawn at a normalized `location`; None where the font doesn't draw or the glyph fails. The
    // charstring steps it took, drawn or not, and the points it drew add to `work`.
    pub(super) fn cff2_bounds_at(&self, gid: u32, location: &[f64], work: &mut usize) -> Option<Option<[f64; 4]>> {
        use crate::daecore::daetype::{format::ivs::fixed_coords, outline::{outline_cff2_glyph_counted, PointBounds}};
        let (outlines, cff2) = (self.cff2_outlines()?, self.table("CFF2")?);
        let coords = fixed_coords(location);
        let coords = coords.iter().any(|&c| c != 0).then_some(&coords[..]);
        let mut pen = PointBounds::default();
        let drawn = outline_cff2_glyph_counted(outlines, cff2, gid, coords, None, work, &mut pen);
        *work = work.saturating_add(pen.points);
        drawn.ok()?;
        Some(pen.finish().map(|(x0, y0, x1, y1)| [x0, y0, x1, y1].map(f64::from)))
    }

    // A CFF2 glyph drawn at this cache's location, and its hints when asked for. Drawn without them, its outline is
    // kept, and replayed at its next draw. The charstring steps a draw takes add to `steps`.
    pub(crate) fn draw_cff2(
        &self,
        gid: u32,
        hints: Option<crate::daecore::daetype::outline::CffHints>,
        steps: &mut usize,
        pen: &mut dyn crate::daecore::daetype::outline::OutlinePen,
    ) -> Option<Result<Option<crate::daecore::daetype::outline::CffHints>, Error>> {
        use crate::daecore::daetype::outline::outline_cff2_glyph_counted;
        let kept = match &self.cff2_at {
            Some(at) => Some(&*at.kept),
            None => self.font.cff2_kept.get(),
        };
        if hints.is_none()
            && let Some(path) = kept.and_then(|k| k.get(gid))
        {
            path.replay(None, pen);
            return Some(Ok(None));
        }
        let (outlines, cff2) = (self.cff2_outlines()?, self.table("CFF2")?);
        let coords = self.cff2_coords();
        if hints.is_some() {
            return Some(outline_cff2_glyph_counted(outlines, cff2, gid, coords, hints, steps, pen));
        }
        let lay_out = || super::drawn::KeptOutlines::new(self.num_glyphs);
        let kept = kept.unwrap_or_else(|| once_slot(&self.font.cff2_kept, lay_out, |_| {}));
        Some(kept.record(gid, pen, |pen| outline_cff2_glyph_counted(outlines, cff2, gid, coords, None, steps, pen)))
    }

    pub(crate) fn cff2_coords(&self) -> Option<&[i64]> {
        self.cff2_at.as_ref().map(|at| &*at.coords)
    }

    // A CFF2 glyph hinted at this cache's location; None where the font isn't CFF2.
    pub(crate) fn hint_cff2(
        &self,
        gid: u32,
        scale16: i64,
    ) -> Option<Option<crate::daecore::daetype::hinting::HintedOutline>> {
        let (outlines, cff2) = (self.cff2_outlines()?, self.table("CFF2")?);
        let coords = self.cff2_coords();
        Some(crate::daecore::daetype::outline::hint_cff2_glyph(outlines, cff2, gid, coords, scale16))
    }

    // GDEF as shaping reads it: none where the sanitizer rejects it.
    pub(crate) fn gdef(&self) -> Option<&TableBytes> {
        self.layout_table(OtlTable::Gdef)
    }

    // A layout table where its readers take it, each verdict kept, as a table is sanitized once a face.
    pub(crate) fn layout_table(&self, which: OtlTable) -> Option<&TableBytes> {
        use crate::daecore::daetype::format::sanitize::otl_passes;
        let table = self.table(which.tag())?;
        once_slot(&self.otl_reads[which as usize], || otl_passes(which, table), |_| {}).then_some(table)
    }

    // An AAT table where shaping reads it, each verdict kept, as a table is sanitized once a face.
    pub(crate) fn aat_table(&self, which: crate::daecore::daetype::format::sanitize::AatTable) -> Option<&TableBytes> {
        use crate::daecore::daetype::format::sanitize::shapes_with;
        let table = self.table(which.tag())?;
        let tables = |tag: &str| match OtlTable::of(tag) {
            Some(which) => self.layout_table(which).map(|t| t.as_slice()),
            None => self.table(tag).map(|t| t.as_slice()),
        };
        let verdict = || shapes_with(which, tables);
        once_slot(&self.aat_reads[which as usize], verdict, |_| {}).then_some(table)
    }

    // GDEF's ItemVariationStore, at the offset its header gives; None where it has none or it doesn't parse.
    pub(crate) fn gdef_var_store(&self) -> Option<Shared<crate::daecore::daetype::format::ivs::ItemVariationStore>> {
        let build = || self.gdef().and_then(|gdef| var_store(gdef)).map(Shared::new);
        once_slot(&self.gdef_var_store, build, |_| {}).clone()
    }

    // hmtx or vmtx as `MetricsTable::read` reads them, from what construction kept and the glyph count.
    pub(crate) fn metrics_table(&self, vertical: bool) -> Option<MetricsTable<'_>> {
        let (data, stated) = self.metrics[usize::from(vertical)].as_ref()?;
        MetricsTable::read(data, *stated as usize, self.num_glyphs as usize)
    }

    // A described location's advances one way, where it varies them glyph by glyph.
    pub(crate) fn varied_advances(&self, vertical: bool) -> Option<&VariedAdvances> {
        self.varied_advances[usize::from(vertical)].as_ref()
    }

    // VORG's origins, where the font has the table.
    pub(crate) fn vorg_origins(&self) -> Option<VerticalOrigins<'_>> {
        match &self.varied_origins {
            Some(varied) => Some(VerticalOrigins::Varied(varied)),
            None => Some(VerticalOrigins::Vorg(Vorg::read(self.table("VORG")?)?)),
        }
    }

    pub(crate) fn advance_font_units(&self, axes: &AxisKey, gid: u32, vertical: bool) -> u32 {
        self.described_at(axes, |at| at.advances(vertical).of(gid))
    }

    // This cache's advances one way, in font units, read where they lie: a location's own where it varies them, else
    // hmtx or vmtx; only a font without either keeps a list.
    #[inline]
    pub(crate) fn advances(&self, vertical: bool) -> Advances<'_> {
        if let Some(varied) = self.varied_advances(vertical) {
            return Advances::Varied(varied);
        }
        let v = usize::from(vertical);
        if let (Some((data, _)), Some(shape)) = (&self.metrics[v], &self.advance_shapes[v]) {
            return Advances::Table(shape.over(data, self.advances_past[v]));
        }
        Advances::Listed(self.listed_advances(vertical))
    }

    // Out of line, so a read of hmtx or vmtx carries none of the list's building.
    #[cold]
    #[inline(never)]
    fn listed_advances(&self, vertical: bool) -> &ListedAdvances {
        let build = || ListedAdvances::new(self.tables(), vertical, self.upm, self.num_glyphs);
        let charge = |kept: &ListedAdvances| self.held.built.add(kept.heap_bytes());
        once_slot(&self.listed_advances[usize::from(vertical)], build, charge)
    }
}

// A cache's advances one way, in font units: hmtx or vmtx where it lies, past whose glyphs each takes the second
// field; a location's, each glyph's worked out when first read; or a font's without metrics, as `ListedAdvances`.
#[derive(Clone, Copy)]
pub(crate) enum Advances<'a> {
    Table(TableAdvances<'a>),
    Varied(&'a VariedAdvances),
    Listed(&'a ListedAdvances),
}

impl Advances<'_> {
    pub(crate) fn of(&self, gid: u32) -> u32 {
        match self {
            Advances::Table(table) => table.of(gid),
            Advances::Varied(varied) => varied.of(gid),
            Advances::Listed(listed) => listed.of(gid),
        }
    }
}

// A table's advances as `MetricsTable::advance` reads them, worked out once so shaping's read per glyph is one
// bounds check and one load: a long record's own, the last one's up to `len`, then `past`.
#[derive(Clone, Copy)]
pub(crate) struct TableAdvances<'a> {
    records: &'a [[u8; 4]],
    last: u32,
    len: usize,
    past: u32,
}

// What `TableAdvances` takes from its table, the same for every read: how many long records there are, the last
// one's advance and how many glyphs have a metric. None for a table with none, which `advances` reads as missing.
#[derive(Clone, Copy)]
pub(crate) struct AdvanceShape {
    long: usize,
    last: u32,
    len: usize,
}

impl AdvanceShape {
    pub(crate) fn of(data: &[u8], stated: usize, glyphs: usize) -> Option<AdvanceShape> {
        let mtx = MetricsTable::read(data, stated, glyphs).filter(MetricsTable::has_data)?;
        Some(AdvanceShape { long: mtx.long(), last: u32::from(mtx.advance(mtx.long() - 1)), len: mtx.len() })
    }

    #[inline]
    fn over<'a>(&self, data: &'a [u8], past: u32) -> TableAdvances<'a> {
        let records = data.get(..4 * self.long).unwrap_or_default().as_chunks().0;
        TableAdvances { records, last: self.last, len: self.len, past }
    }
}

impl TableAdvances<'_> {
    pub(crate) fn of(&self, gid: u32) -> u32 {
        match self.records.get(gid as usize) {
            Some(&[hi, lo, ..]) => u32::from(u16::from_be_bytes([hi, lo])),
            _ if (gid as usize) < self.len => self.last,
            _ => self.past,
        }
    }
}

// The advances of a font with neither hmtx nor vmtx to read: a bitmap-only font's as its strikes state them, each
// worked out when first read, then `past` for every id beyond; any other font's glyphs each `past`, half an em across
// and 0 down.
pub(crate) struct ListedAdvances {
    // A bitmap-only font's tables, and each glyph's advance plus one, 0 until read; threads filling one write the same.
    // Boxed, as every location's cache holds a list and most fonts have no strikes.
    strikes: Option<alloc::boxed::Box<(Tables, GlyphSlots)>>,
    upm: u16,
    vertical: bool,
    past: u32,
}

impl ListedAdvances {
    // `upm` and `n_glyphs` were checked at load.
    fn new(map: &Tables, vertical: bool, upm: u16, n_glyphs: u32) -> ListedAdvances {
        let past = if vertical { 0 } else { u32::from(upm) / 2 };
        let bitmap = crate::daecore::daetype::bitmap::bitmap_only(map);
        let strikes = bitmap.then(|| alloc::boxed::Box::new((map.clone(), GlyphSlots::new(n_glyphs as usize))));
        ListedAdvances { strikes, upm, vertical, past }
    }

    #[inline]
    pub(crate) fn of(&self, gid: u32) -> u32 {
        let Some((tables, read)) = self.strikes.as_deref() else { return self.past };
        match read.get(gid) {
            0 if gid as usize >= read.len() => self.past,
            0 => self.fill(tables, read, gid),
            held => held - 1,
        }
    }

    // Out of line, so a glyph read again carries none of the strikes' search.
    #[cold]
    #[inline(never)]
    fn fill(&self, tables: &Tables, read: &GlyphSlots, gid: u32) -> u32 {
        use crate::daecore::daetype::bitmap::bitmap_advance;
        let advance = bitmap_advance(tables, gid, self.upm, self.vertical).unwrap_or(self.past);
        read.set(gid, advance + 1);
        advance
    }

    fn heap_bytes(&self) -> usize {
        self.strikes.as_deref().map_or(0, |(tables, read)| {
            size_of::<(Tables, GlyphSlots)>() + tables.heap_bytes() + read.heap_bytes()
        })
    }
}

pub(crate) struct ExtentsSource<'a> {
    cache: &'a FontCache,
    color_bitmaps: bool,
    colr: bool,
    varc: bool,
    measured: Option<&'a crate::daecore::daetype::instancer::MeasuredGlyphs>,
}

// Each glyph's extents, in pages of glyphs made as one of theirs is first asked for.
pub(crate) struct KeptExtents(alloc::boxed::Box<[Once<alloc::boxed::Box<ExtentsPage>>]>);

const EXTENTS_PAGE: usize = 32;
type ExtentsPage = [Once<Option<GlyphExtents>>; EXTENTS_PAGE];

impl FontCache {
    // `work_out`'s answer for the glyph, kept once worked out; one past the font's count is worked out each time.
    fn kept_extents(&self, gid: u32, work_out: impl FnOnce() -> Option<GlyphExtents>) -> Option<GlyphExtents> {
        let build = || {
            let pages = (0..(self.num_glyphs as usize).div_ceil(EXTENTS_PAGE)).map(|_| Once::new()).collect();
            alloc::boxed::Box::new(KeptExtents(pages))
        };
        let kept = once_slot(&self.kept_extents, build, |k| self.held.built.add(size_of_val(&*k.0)));
        let Some(page) = kept.0.get(gid as usize / EXTENTS_PAGE) else { return work_out() };
        let new_page = || alloc::boxed::Box::new([const { Once::new() }; EXTENTS_PAGE]);
        let page = once_slot(page, new_page, |_| self.held.built.add(size_of::<ExtentsPage>()));
        *once_slot(&page[gid as usize % EXTENTS_PAGE], work_out, |_| {})
    }
}

impl ExtentsSource<'_> {
    // glyf's measured glyphs keep their own extents; any other source's are kept once worked out.
    pub(crate) fn of(&self, gid: u32) -> Option<GlyphExtents> {
        match self.measured.is_some() && !(self.color_bitmaps || self.colr || self.varc) {
            true => self.work_out(gid),
            false => self.cache.kept_extents(gid, || self.work_out(gid)),
        }
    }

    // The first of: a color bitmap's box, a color glyph's, VARC's, glyf's drawn outline's from its origin, another
    // outline's drawn box, a bitmap's for a glyph with no outline. None for a glyf glyph that fails.
    fn work_out(&self, gid: u32) -> Option<GlyphExtents> {
        use crate::daecore::daetype::bitmap::{color_bitmap_extents, mono_bitmap_extents};
        let cache = self.cache;
        let boxed =
            |[x_bearing, y_bearing, width, height]: [i32; 4]| GlyphExtents { x_bearing, y_bearing, width, height };
        let color = self.color_bitmaps.then(|| color_bitmap_extents(cache.tables(), gid, cache.upm));
        let colored = color.flatten().map(boxed).or_else(|| self.colr.then(|| cache.colr_extents(gid)).flatten());
        if colored.is_some() {
            return colored;
        }
        if let Some(varied) = self.varc.then(|| cache.varc_extents(gid)).flatten() {
            return Some(varied);
        }
        if let Some(measured) = self.measured.and_then(|m| m.of(gid)) {
            return measured.extents.map(boxed);
        }
        let drawn = || {
            // Edges round half away from 0, as a color glyph's do, so a mirrored outline's box mirrors.
            let bounds = cache.drawn_bounds(gid, &mut 0)?;
            Some(bounds.map_or_else(GlyphExtents::default, |b| GlyphExtents::from_bounds(b, |v| v.round())))
        };
        drawn().or_else(|| mono_bitmap_extents(cache.tables(), gid, cache.upm).map(boxed))
    }
}

pub(crate) enum VerticalOrigins<'a> {
    Vorg(Vorg<'a>),
    Varied(&'a VariedOrigins),
    Phantom { glyf: &'a [u8], extended: bool, loca: &'a GlyfLoca, vmtx: MetricsTable<'a>, upm: i32 },
    Measured(&'a crate::daecore::daetype::instancer::MeasuredGlyphs),
    Boxed { extents: ExtentsSource<'a>, ascender: i32, descender: i32 },
}

impl VerticalOrigins<'_> {
    // A glyph's origin in font units; 0 past glyf's glyphs, and the ascender for a glyph with no box.
    pub(crate) fn of(&self, gid: u32) -> i32 {
        match self {
            VerticalOrigins::Vorg(vorg) => i32::from(vorg.origin(gid)),
            VerticalOrigins::Varied(varied) => i32::from(varied.origin(gid)),
            VerticalOrigins::Measured(measured) => measured.of(gid).map_or(0, |m| m.top),
            VerticalOrigins::Phantom { glyf, extended, loca, vmtx, upm } => match gid as usize + 1 < loca.len() {
                true => {
                    let loca = (*loca).into();
                    let mut walk = TopWalk { glyf, extended: *extended, loca, vmtx, edges: 0, work: GLYPH_WORK };
                    let mut gathered = 0;
                    let top = walk.top(gid, &mut gathered);
                    top.filter(|_| walk.spend(gathered).is_some()).unwrap_or(*upm)
                }
                false => 0,
            },
            VerticalOrigins::Boxed { extents, ascender, descender } => {
                let advance = ascender.saturating_sub(*descender);
                match extents.of(gid) {
                    Some(e) => e.y_bearing.saturating_add(advance.saturating_add(e.height) >> 1),
                    None => *ascender,
                }
            }
        }
    }
}

// A glyph's top phantom walk: yMax plus the top bearing, 0 for a glyph glyf doesn't hold, or the last USE_MY_METRICS
// component's; past bounds or unreadable end points the em answers. It counts work and points as `phantoms` does.
struct TopWalk<'a> {
    glyf: &'a [u8],
    extended: bool,
    loca: LocaRef<'a>,
    vmtx: &'a MetricsTable<'a>,
    edges: usize,
    work: usize,
}

impl TopWalk<'_> {
    fn spend(&mut self, work: usize) -> Option<()> {
        self.work = self.work.checked_sub(work)?;
        Some(())
    }

    // Composites are walked on the heap: each one's components left, its top so far, and what was gathered before
    // the component being read.
    fn top(&mut self, gid: u32, gathered: &mut usize) -> Option<i32> {
        let (mut inside, mut path) = (Stack::<(Components, i32, usize), 8>::default(), ComponentPath::default());
        let mut gid = gid;
        loop {
            if self.edges >= MAX_VISITS {
                return None;
            }
            self.edges += 1;
            let mut read = None;
            match glyph_record(self.glyf, self.loca, gid) {
                None => {
                    self.spend(16)?;
                    *gathered += 4;
                    read = Some(0);
                }
                Some(glyph) => {
                    let header = |at: usize| glyph.get(..10).and_then(|h| read_i16_be(h, at)).unwrap_or(0);
                    let n_contours = header(0);
                    let simple = match usize::try_from(n_contours) {
                        Ok(n) if n > 0 => simple_point_count(glyph, n)?,
                        _ => 0,
                    };
                    self.spend(16 + simple)?;
                    *gathered += simple;
                    if *gathered + 4 > MAX_POINTS {
                        return None;
                    }
                    let top = i32::from(header(8)) + i32::from(self.vmtx.bearing(gid as usize));
                    if n_contours < 0 {
                        path.enter(gid);
                        inside.push((Components::new(glyph, self.extended), top, 0));
                    } else {
                        *gathered += 4;
                        read = Some(top);
                    }
                }
            }
            // The next USE_MY_METRICS component to read, each glyph read whole handing its top to its composite.
            gid = loop {
                if let Some(top) = read.take() {
                    let Some((_, held, before)) = inside.last_mut() else { return Some(top) };
                    *held = top;
                    *gathered -= 4;
                    let more = *gathered - *before;
                    self.spend(more)?;
                    if *gathered + 4 > MAX_POINTS {
                        return None;
                    }
                }
                let (components, _, before) = inside.last_mut()?;
                let mut down = None;
                while let Some(Ok(c)) = components.next() {
                    self.spend(1)?;
                    if path.holds(c.gid) {
                        continue;
                    }
                    if c.flags & USE_MY_METRICS != 0 {
                        (*before, down) = (*gathered, Some(c.gid));
                        break;
                    }
                    if *gathered + 4 > MAX_POINTS {
                        return None;
                    }
                }
                if let Some(down) = down {
                    break down;
                }
                let (_, top, _) = inside.pop()?;
                path.leave();
                *gathered += 4;
                read = Some(top);
            };
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/cache/metrics.rs"]
mod tests;
