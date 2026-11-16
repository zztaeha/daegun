use super::*;
use crate::daecore::Error;

use aat::morx::Stackings;
use glyf::Loca;
use super::super::decoder::{glyph_count, GlyfTags};
use super::super::format::glyf::loca_format;
use super::super::format::sanitize::{AatTable, Reads};

// glyf and loca, with the tags they're read from.
pub(crate) type Outlines<'a> = Option<(&'a [u8], Loca<'a>, GlyfTags)>;

// The tables the passes after the main closure read, so their work is counted in their bytes.
const PASS_TABLES: [&str; 12] =
    ["morx", "sbix", "CBLC", "CBDT", "EBLC", "EBDT", "bloc", "bdat", "bsln", "fmtx", "COLR", "VARC"];

// Past the 64 passes any table gets, a closure may spend this much more reading them.
const CLOSURE_FLOOR: usize = 1 << 24;

// A face's verdicts, read through it as `Reads`, and the lookups GSUB's features name, where each of its closures
// starts: found again each time, they took a fifth of a Source Han Sans closure. None past their work budget.
pub(crate) struct FaceReads {
    reads: Reads,
    lookups: Option<Vec<u16>>,
}

impl FaceReads {
    pub(crate) fn of<'a>(table: impl Fn(&str) -> Option<&'a [u8]> + Copy) -> FaceReads {
        let reads = Reads::of(table);
        let lookups = reads.reading(table)("GSUB").map_or(Some(Vec::new()), otl::closure::feature_lookups);
        FaceReads { reads, lookups }
    }
}

impl core::ops::Deref for FaceReads {
    type Target = Reads;

    fn deref(&self) -> &Reads {
        &self.reads
    }
}

// What the request reaches, in rendering order: GSUB's substitutes, MATH's variants of them, what drawing reads
// (COLR, VARC, then components), then morx, bitmaps, bsln and fmtx in passes bounded by bytes read, what each adds
// drawn as the request is.
pub(crate) fn closure_loop<'a>(
    table: impl Fn(&str) -> Option<&'a [u8]>,
    reads: &FaceReads,
    num_glyphs: u32,
    requested: &[u32],
    mut reach: impl FnMut(&[u32], &mut GlyphSet) -> Result<(), Error>,
) -> Result<Closed<'a>, Error> {
    let seed = core::iter::once(0).chain(requested.iter().copied());
    let mut closed: GlyphSet = seed.filter(|&g| g < num_glyphs).collect();
    // A GSUB or COLR the sanitizer rejects reaches no glyph; the subset leaves it out. So does a morx.
    if let Some(gsub) = reads.reading(&table)("GSUB") {
        let past = Error::limit("subset: GSUB closure past its work budget");
        let lookups = reads.lookups.as_deref().ok_or(past.clone())?;
        if !otl::closure::close(gsub, lookups, &mut closed, num_glyphs, otl::closure::budget(gsub)) {
            return Err(past);
        }
    }
    if let Some(math) = table("MATH") {
        closed.extend(math::math_closure(math, &closed).into_iter().filter(|&g| g < num_glyphs));
    }
    // A VARC the sanitizer rejects reaches no glyph, and the subset leaves it out.
    let (colr, varc) = (reads.reading(&table)("COLR"), reads.reading(&table)("VARC"));
    // COLR's layers and paints, then VARC's components, added to `glyphs`; and what COLR keeps by them.
    let drawn = |glyphs: &mut GlyphSet| {
        let found = colr.map(|colr| colr::colr_closure(colr, glyphs, num_glyphs));
        glyphs.extend(found.iter().flat_map(|(found, _)| found.iter()));
        if let Some(varc) = varc && !super::super::varc::subset::closure(varc, glyphs, num_glyphs) {
            return Err(Error::limit("subset: VARC closure past its work budget"));
        }
        Ok(found.map(|(_, kept)| kept).unwrap_or_default())
    };
    let (request, mut colored) = (closed.len(), colr.map(|_| closed.clone()));
    let mut colr_kept = drawn(&mut closed)?;
    let mut active = GlyphSet::new();
    reach(&closed.iter().collect::<Vec<u32>>(), &mut active)?;

    let pass_bytes = PASS_TABLES.iter().filter_map(|&tag| table(tag)).map(<[u8]>::len).sum::<usize>().max(1);
    let mut budget = pass_bytes.saturating_mul(64).saturating_add(CLOSURE_FLOOR);
    let morx = table("morx").filter(|_| reads.shaped(AatTable::Morx));
    let mut stackings = morx.map(Stackings::new);
    // bsln's and fmtx's glyphs depend on no set, so the first pass takes them.
    let mut metrics = aat::simple::metrics_glyphs(&table);
    let past = Error::limit("subset: glyph closure past its work budget");
    loop {
        budget = budget.checked_sub(pass_bytes).ok_or(past.clone())?;
        let mut new_gids: Vec<u32> = Vec::new();
        if let Some(walked) = &mut stackings {
            let found = walked.reach(&active, num_glyphs, &mut budget).ok_or(past.clone())?;
            new_gids.extend(found.into_iter().map(u32::from));
        }
        new_gids.extend(bitmap::bitmap_closure(&table, num_glyphs as usize, &active).into_iter().map(u32::from));
        new_gids.extend(metrics.drain(..).map(u32::from));
        // Done when a pass adds nothing: a gid found again is in the set or, past the font, refused by `reach`.
        let mut added: GlyphSet = new_gids.into_iter().filter(|&g| g < num_glyphs && !active.contains(&g)).collect();
        if added.len() == 0 { break; }
        if let Some(colored) = &mut colored {
            colored.union(&added);
        }
        drawn(&mut added)?;
        closed.union(&added);
        let before = active.len();
        reach(&added.iter().collect::<Vec<u32>>(), &mut active)?;
        if active.len() == before { break; }
    }
    // What COLR keeps reads every glyph it closed over, so it's found again once the passes add some.
    if let (Some(colr), Some(colored)) = (colr, colored) && colored.len() > request {
        colr_kept = colr::colr_closure(colr, &colored, num_glyphs).1;
    }
    // Kept only for a VARC to subset, as it's 8 KiB a subset holds to its end.
    let varced = if varc.is_some() { closed } else { GlyphSet::empty() };
    Ok(Closed { active, stackings, colr: colr_kept, varced })
}

// What a closure keeps, as `closure_loop` gives it.
pub(crate) struct Closed<'a> {
    pub(crate) active: GlyphSet,
    pub(crate) stackings: Option<Stackings<'a>>,
    pub(crate) colr: colr::ColrKept,
    pub(crate) varced: GlyphSet,
}

// head, or bhed in an Apple font that has only bitmaps: the same record, read alike, with its tag.
pub(crate) fn font_header<'a>(table: impl Fn(&str) -> Option<&'a [u8]>) -> Option<(&'static str, &'a [u8])> {
    ["head", "bhed"].into_iter().find_map(|tag| Some((tag, table(tag)?)))
}

// A TrueType font's glyph count and outlines, when it has them.
pub(crate) fn glyf_outlines<'a>(
    table: impl Fn(&str) -> Option<&'a [u8]> + Copy,
) -> Result<(usize, Outlines<'a>), Error> {
    let (_, head) = font_header(table).ok_or(Error::missing("subset: missing head"))?;
    if table("maxp").or(table("MAXP")).is_none() {
        return Err(Error::missing("subset: missing maxp"));
    }
    read_i16_be(head, 50).ok_or(Error::malformed("subset: head table truncated"))?;
    let num_glyphs = glyph_count(table).ok_or(Error::malformed("subset: maxp table truncated"))? as usize;
    if num_glyphs == 0 {
        return Err(Error::malformed("subset: maxp reports zero glyphs"));
    }
    // glyf of an unknown format holds no glyph, so the subset carries none.
    let tags = GlyfTags::of(|tag| table(tag).is_some_and(|t| !t.is_empty()));
    let outlines = match (table(tags.glyf), table(tags.loca)) {
        (Some(glyf), Some(loca)) => {
            loca_format(head).map(|format| (glyf, Loca::new(loca, format, num_glyphs), tags))
        }
        (None, None) => None,
        (Some(_), None) => return Err(Error::missing("subset: font has glyf but no loca")),
        (None, Some(_)) => return Err(Error::missing("subset: font has loca but no glyf")),
    };
    Ok((num_glyphs, outlines))
}

pub(crate) fn glyf_closure_of<'a>(
    table: impl Fn(&str) -> Option<&'a [u8]>, reads: &FaceReads, num_glyphs: usize, outlines: &Outlines,
    requested: &[u32],
) -> Result<Closed<'a>, Error> {
    let mut left = subset_budget(outlines.as_ref().map_or(0, |(glyf, ..)| glyf.len()));
    closure_loop(table, reads, num_glyphs as u32, requested, |gids, set| match outlines {
        Some((glyf, loca, tags)) => active_gids_into(gids, glyf, tags.extended, *loca, num_glyphs, set, &mut left),
        None => {
            set.extend(gids.iter().copied().filter(|&g| (g as usize) < num_glyphs));
            Ok(())
        }
    })
}

pub(crate) fn glyf_closure<'a>(
    table: impl Fn(&str) -> Option<&'a [u8]> + Copy, reads: &FaceReads, requested: &[u32],
) -> Result<GlyphSet, Error> {
    let (num_glyphs, outlines) = glyf_outlines(table)?;
    glyf_closure_of(table, reads, num_glyphs, &outlines, requested).map(|closed| closed.active)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/closure.rs"]
mod tests;
