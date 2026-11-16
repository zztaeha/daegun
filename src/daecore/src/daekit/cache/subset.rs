use alloc::borrow::Cow;
use crate::daecore::daetype::decoder::{copies_held, num_glyphs};
use crate::daecore::daetype::instancer::{Cff2Source, Flattened};
use crate::daecore::daetype::outline::{CffOutlines, SubrsGathered};
use crate::daecore::daetype::subsetter::{
    self, CharMap, CharTables, Closed, CmapEntries, FaceReads, GlyphSet, TextCmap,
};
use crate::daecore::daetype::varc::flatten::{self, Draw};
use crate::daecore::daetype::varc::subset::{covered_glyphs, subset as subset_varc, Subset as VarcSubset};
use super::*;
use crate::daecore::Error;

// Tables a CFF subset leaves out or rebuilds: variations, color, and DSIG, which signs the source's bytes.
const STRIP_FOR_CFF_SUBSET: &[&str] = &[
    "fvar", "gvar", "GVAR", "avar", "cvar", "HVAR", "VVAR", "MVAR", "CFF2", "CFF ", "COLR", "CPAL", "DSIG",
];

// `reads` are the font's verdicts.
pub(crate) fn cff_color_closure<'a>(
    cff: &[u8], outlines: Option<&CffOutlines>, source: &'a Tables, reads: &FaceReads,
    requested: &[u32],
) -> Result<(Closed<'a>, Option<SubrsGathered>), Error> {
    let mut gathered = outlines.map(CffOutlines::gather_subrs);
    let closed = crate::daecore::daetype::subsetter::closure_loop(
        |tag| source.get(tag).map(|t| t.as_slice()),
        reads,
        num_glyphs(source).unwrap_or(0),
        requested,
        |gids, set| {
            let frontier: Vec<u32> = gids.iter().copied().filter(|&g| set.insert(g)).collect();
            if let (Some(outlines), Some(gathered)) = (outlines, gathered.as_mut()) {
                crate::daecore::daetype::subsetter::close_over_seacs(outlines, cff, frontier, set, gathered);
            }
            Ok(())
        },
    )?;
    Ok((closed, gathered))
}

fn identity_gid_map_for(active: &GlyphSet) -> Vec<u32> {
    let max_active = active.iter().last().unwrap_or(0);
    (0..=max_active).collect()
}

// A subset in place as `renumbered` reads one: each kept glyph its own id, every other 0, as a dropped glyph is.
fn kept_in_place(active: &GlyphSet) -> Vec<u32> {
    let mut map = vec![0; active.iter().last().map_or(1, |g| g as usize + 1)];
    active.iter().for_each(|g| map[g as usize] = g);
    map
}

// The table a CFF-flavored subset writes: CFF with the face's outlines where they parse, or CFF2 with the coordinates
// it's pinned at and the glyphs flattened there.
#[derive(Clone, Copy)]
pub(crate) enum CffSource<'f> {
    Cff(&'f [u8], Option<&'f CffOutlines>),
    Cff2(&'f [u8], &'f [f64], &'f Mutable<Flattened>),
}

impl<'f> CffSource<'f> {
    // The table, and the outlines a closure reads seacs from: CFF2 has none.
    fn table(self) -> (&'f [u8], Option<&'f CffOutlines>) {
        match self {
            CffSource::Cff(cff, outlines) => (cff, outlines),
            CffSource::Cff2(cff2, ..) => (cff2, None),
        }
    }
}

// `text` holds the text's mappings and sequences in source ids for the subset's cmap, in place of the source's, and
// the subset may be compacted. `draw` draws VARC's glyphs where the subset stands.
pub(crate) fn subset_cff_flavored(
    source: &Tables,
    reads: &FaceReads,
    cff_source: CffSource,
    gids: &[u32],
    text: Option<TextCmap>,
    entries: &dyn Fn() -> Shared<CmapEntries>,
    draw: Draw,
) -> Result<Subset, Error> {
    copies_held(source)?;
    let n_source = num_glyphs(source).unwrap_or(0);
    let (cff, outlines) = cff_source.table();
    if matches!(cff_source, CffSource::Cff(..)) && !subsetter::cff_readable(cff, n_source) {
        return Err(Error::malformed("CFF: CharStrings count differs from maxp's, or the FDSelect fails its check"));
    }
    let (Closed { active, stackings, colr, varced }, gathered) = cff_color_closure(cff, outlines, source, reads, gids)?;
    let closed: Vec<u32> = active.iter().collect();
    const RENUMBER_SAFE: &[&str] = &[
        "CFF ", "GDEF", "GSUB", "GPOS", "MATH", "JSTF", "VORG", "kern", "hdmx", "LTSH",
        "hmtx", "hhea", "vmtx", "vhea", "maxp", "HMTX", "HHEA", "VMTX", "VHEA", "MAXP", "cmap", "DMAP", "name", "post",
        "CBDT", "CBLC", "EBDT", "EBLC", "sbix", "prop", "kerx", "just", "morx", "lcar", "opbd", "ankr", "bsln", "fmtx",
        "bdat", "bloc", "Zapf", "EBSC", "xref",
        "feat", "trak", "fdsc", "ltag",
        "head", "OS/2", "MERG", "VARC",
    ];

    let recognized = |t: &str| {
        RENUMBER_SAFE.contains(&t)
            || crate::daecore::daetype::subsetter::GLYPH_FREE_TABLES.contains(&t)
            || STRIP_FOR_CFF_SUBSET.contains(&t)
            || (t == "BASE" && source.get("BASE").is_some_and(|b| crate::daecore::daetype::base::base_is_glyph_free(b)))
    };
    let want_compact = text.is_some() && source.keys().all(recognized);
    let n = n_source as usize;
    // The outlines, each glyph `drawn` holds written as that charstring.
    let mut gathered = gathered;
    let mut write_cff = |closed: &[u32], drawn: BTreeMap<u32, Vec<u8>>| -> Result<Subset, Error> {
        match cff_source {
            CffSource::Cff2(cff2, coords, flattened) => {
                let varc = source.get("VARC").is_some_and(|t| !t.is_empty());
                let cff2 = Cff2Source::read(cff2, n, varc)?.with_drawn(drawn);
                subsetter::subset_cff2_closed(cff2, n, closed, want_compact, coords, Some(&mut *write(flattened)))
            }
            CffSource::Cff(cff, outlines) => {
                subsetter::subset_cff_closed(cff, outlines, closed, want_compact, gathered.take(), &drawn)
            }
        }
    };
    let (tag, endchar) = match cff_source {
        CffSource::Cff2(..) => ("CFF2", false),
        CffSource::Cff(..) => ("CFF ", true),
    };
    let written = write_cff(&closed, BTreeMap::new())?;
    let compacted = !written.gid_map.is_empty();
    // The closure keeps a gid past the CFF, as a cmap or GSUB entry can name; the subset holds none.
    let limit = if compacted { written.gid_map.len() } else { n };
    let closed: Vec<u32> = closed.into_iter().filter(|&g| (g as usize) < limit).collect();
    let active: GlyphSet = closed.iter().copied().collect();
    let gid_map = if compacted { written.gid_map } else { identity_gid_map_for(&active) };

    let mut out_map: BTreeMap<String, TableBytes> = source.iter()
        .filter(|(tag, _)| !STRIP_FOR_CFF_SUBSET.contains(tag))
        .map(|(tag, data)| (String::from(tag), data.clone()))
        .collect();
    out_map.insert(tag.to_string(), written.font.into());
    // VARC is subset with every axis pinned where CFF2 is, as its blends are wherever it has a store. Where VARC
    // can't be subset, each kept glyph it draws is written as its drawing.
    out_map.remove("VARC");
    let mut drawn_boxes = BTreeMap::new();
    if let Some(varc) = reads.reading(|tag| source.get(tag).map(|t| t.as_slice()))("VARC") {
        let pinned = match cff_source {
            CffSource::Cff2(cff2, coords, _) => {
                !coords.is_empty() || Cff2Source::read(cff2, n, true).is_ok_and(|s| s.blends())
            }
            CffSource::Cff(..) => false,
        };
        let new_gid = |old: u32| active.contains(&old).then(|| if compacted { gid_map[old as usize] } else { old });
        match subset_varc(varc, &varced, new_gid, !compacted, pinned) {
            VarcSubset::Written(table) => _ = out_map.insert("VARC".into(), table.into()),
            VarcSubset::Refused => {
                let gids: Vec<u32> = covered_glyphs(varc).into_iter().filter(|g| active.contains(g)).collect();
                let drawn = draw(&gids)?;
                let charstrings: BTreeMap<u32, Vec<u8>> = drawn.iter()
                    .filter_map(|(&gid, path)| Some((gid, flatten::charstring(path, endchar)?)))
                    .collect();
                drawn_boxes = charstrings.keys().map(|&gid| (gid, flatten::measured(&drawn[&gid]))).collect();
                out_map.insert(tag.to_string(), write_cff(&closed, charstrings)?.font.into());
            }
            VarcSubset::Unkept => {}
        }
    }
    let (renumbered, instanced) = (compacted, false);
    let kept = subsetter::KeptGlyphs { num_glyphs: n, closed, active, gid_map, renumbered, instanced, stackings, colr };
    subsetter::side_tables(&mut out_map, |tag| source.get(tag).map(|t| t.as_slice()), reads, &kept, text, entries);
    // CFF2 is only ever subset pinned, so its boxes are measured at the location for the glyphs kept.
    if let CffSource::Cff2(cff2, coords, flattened) = cff_source {
        let new_gid = |old: u32| (if compacted { kept.gid_map[old as usize] } else { old }) as usize;
        let boxes = write(flattened).boxes(cff2, coords, &kept.closed);
        let measured = |(&old, b): (&u32, Option<[i16; 4]>)| (new_gid(old), *drawn_boxes.get(&old).unwrap_or(&b));
        let glyphs: Vec<_> = kept.closed.iter().zip(boxes).map(measured).collect();
        let rewritten = subsetter::kept_extents(&out_map, &glyphs, None);
        out_map.extend(rewritten.into_iter().map(|(tag, table)| (tag.to_string(), table.into())));
    }
    Ok(Subset {
        font: crate::daecore::daetype::decoder::build_ttf(&out_map)?,
        gid_map: if compacted { kept.gid_map } else { kept_in_place(&kept.active) },
    })
}

// What subsets read at one location, built once: the instance's tables sharing unchanged bytes (None where the font's
// own serve), CFF2 left variable with its pinned coordinates and flattened glyphs, and the verdicts.
pub(crate) struct SubsetFace {
    tables: Option<Tables>,
    pinned: Option<Vec<f64>>,
    flattened: Mutable<Flattened>,
    reads: Once<FaceReads>,
    lookup: Once<CharMap>,
    cmap: Once<Shared<CmapEntries>>,
    owned: usize,
    pub(super) charge: super::lru::Charge,
}

impl SubsetFace {
    pub(crate) fn new(
        table_map: &Tables, reads: &crate::daecore::daetype::format::sanitize::Reads, cff2: bool, own: bool,
        key: &AxisKey,
    ) -> Result<SubsetFace, Error> {
        use crate::daecore::daetype::instancer;
        let face = |tables, pinned, owned| {
            let flattened = mutable(Flattened::default());
            let (reads, lookup, cmap) = (Once::new(), Once::new(), Once::new());
            Ok(SubsetFace { tables, pinned, flattened, reads, lookup, cmap, owned, charge: Default::default() })
        };
        if own {
            return face(None, None, 0);
        }
        let (map, pinned) = match cff2 {
            true => instancer::pinned_cff2_tables(table_map, reads, key).map(|(map, coords)| (map, Some(coords)))?,
            false => (instancer::instance_tables_for_subset(table_map, reads, key)?, None),
        };
        let mut owned = pinned.as_ref().map_or(0, crate::daecore::daekit::base::sync::heap_of);
        let mut tables = Tables::default();
        for (tag, data) in map {
            let unchanged = table_map.get(&tag).filter(|t| match &data {
                Cow::Borrowed(bytes) => core::ptr::eq(t.as_slice(), *bytes),
                Cow::Owned(_) => false,
            });
            let bytes = match unchanged {
                Some(t) => t.clone(),
                None => {
                    let data = data.into_owned();
                    owned += crate::daecore::daekit::base::sync::heap_of(&data)
                        + crate::daecore::daekit::base::sync::shared_of::<Vec<u8>>();
                    TableBytes::from_vec(data)
                }
            };
            tables.insert(tag, bytes);
        }
        owned += tables.heap_bytes();
        face(Some(tables), pinned, owned)
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::shared_of;
        let cmap = self.cmap.get().map_or(0, |c| shared_of::<CmapEntries>() + c.heap_bytes());
        self.owned + cmap + read(&self.flattened).heap_bytes()
    }

    // Itself and what it holds, as its location is charged for it.
    pub(super) fn held_bytes(&self) -> usize {
        crate::daecore::daekit::base::sync::shared_of::<SubsetFace>() + self.heap_bytes()
    }
}

// What a subset or closure at a location reads, and how its outlines are subset; the font, drawing VARC there.
struct SubsetSource<'a> {
    face: Shared<SubsetFace>,
    own: &'a Tables,
    outlines: Outlines<'a>,
    font: &'a FontCache,
    key: &'a AxisKey,
}

enum Outlines<'a> {
    Cff(Option<&'a CffOutlines>),
    Cff2,
    Glyf,
}

impl SubsetSource<'_> {
    fn map(&self) -> &Tables {
        self.face.tables.as_ref().unwrap_or(self.own)
    }

    fn table(&self, tag: &str) -> Option<&[u8]> {
        self.map().get(tag).map(|t| t.as_slice())
    }

    fn reads(&self) -> &FaceReads {
        let build = || FaceReads::of(|tag| self.table(tag));
        once_slot(&self.face.reads, build, |_| {})
    }

    fn char_tables(&self) -> CharTables<'_> {
        subsetter::char_tables(|tag| self.table(tag))
    }

    fn lookup(&self) -> &CharMap {
        let build = || CharMap::new(self.char_tables());
        once_slot(&self.face.lookup, build, |_| {})
    }

    // The character map's entries, which glyph subsets filter.
    fn entries(&self) -> Shared<CmapEntries> {
        let build = || Shared::new(CmapEntries::read(self.char_tables()));
        once_slot(&self.face.cmap, build, |_| {}).clone()
    }

    // The CFF table with the face's outlines, or the CFF2 one pinned here; None for a glyf font.
    fn cff(&self) -> Option<Result<CffSource<'_>, Error>> {
        let missing = |tag| Error::owned(crate::daecore::ErrorKind::Missing, alloc::format!("subset: no {tag} table"));
        let outlines = match &self.outlines {
            Outlines::Glyf => return None,
            Outlines::Cff(outlines) => *outlines,
            Outlines::Cff2 => None,
        };
        Some(match self.face.pinned.as_deref() {
            Some(coords) => self.table("CFF2").ok_or_else(|| missing("CFF2"))
                .map(|cff2| CffSource::Cff2(cff2, coords, &self.face.flattened)),
            None => self.table("CFF ").ok_or_else(|| missing("CFF ")).map(|cff| CffSource::Cff(cff, outlines)),
        })
    }

    fn subset(&self, gids: &[u32], text: Option<TextCmap>) -> Result<Subset, Error> {
        let (reads, entries) = (self.reads(), &|| self.entries());
        let draw = |gids: &[u32]| self.font.varc_drawn(self.key, gids);
        match self.cff() {
            None => {
                let instanced = self.face.tables.is_some();
                subsetter::subset_glyf_tables(self.map(), reads, gids, text, entries, instanced, &draw)
            }
            Some(cff) => subset_cff_flavored(self.map(), reads, cff?, gids, text, entries, &draw),
        }
    }

    fn closure(&self, gids: &[u32]) -> Result<GlyphSet, Error> {
        let (reads, map) = (self.reads(), self.map());
        match self.cff() {
            None => subsetter::glyf_closure(|tag| map.get(tag).map(|t| t.as_slice()), reads, gids),
            Some(cff) => {
                let (cff, outlines) = cff?.table();
                Ok(cff_color_closure(cff, outlines, map, reads, gids)?.0.active)
            }
        }
    }
}

impl FontCache {
    // A static font's own tables, CFF's at every location, and a variable glyf or any CFF2 font's at the location.
    fn subset_source<'a>(&'a self, key: &'a AxisKey) -> Result<SubsetSource<'a>, Error> {
        let outlines = if self.cff2 {
            Outlines::Cff2
        } else if self.has_table("CFF ") {
            Outlines::Cff(self.cff_outlines())
        } else {
            Outlines::Glyf
        };
        Ok(SubsetSource { face: self.subset_face(key)?, own: self.tables(), outlines, font: self, key })
    }

    // `f` on the subset source at `key`, whose charge there grows with what it builds.
    fn with_source<T>(&self, key: &AxisKey, f: impl FnOnce(&SubsetSource) -> Result<T, Error>) -> Result<T, Error> {
        let source = self.subset_source(key)?;
        let out = f(&source);
        self.recharge(&source.face.charge, source.face.held_bytes());
        out
    }

    pub fn subset_font(&self, key: &AxisKey, gids: &[u32]) -> Result<Subset, Error> {
        self.with_source(key, |source| source.subset(gids, None))
    }

    pub fn glyph_closure(
        &self,
        key: &AxisKey,
        gids: &[u32],
    ) -> Result<Vec<u32>, Error> {
        let closure = self.with_source(key, |source| source.closure(gids))?;
        Ok(closure.iter().filter(|&g| self.glyph_in_range(g).is_some()).collect())
    }

    pub fn subset_text(&self, key: &AxisKey, text: &str) -> Result<Subset, Error> {
        self.with_source(key, |source| self.subset_text_from(key, source, text))
    }

    fn subset_text_from(&self, key: &AxisKey, source: &SubsetSource, text: &str) -> Result<Subset, Error> {
        use crate::daecore::daetype::subsetter::{is_variation_selector, UvsLookup};
        let cmap = source.char_tables();
        if cmap.cmap.is_empty() && cmap.dmap.is_empty() {
            return Err(Error::missing("subset_text: font has no cmap"));
        }
        let lookup = source.lookup();

        // Every code shaping the text can look up, with the font's sequence for each selector and base among them; a
        // code's mirror and vertical form count as bases, as shaping reads them.
        let mut seen: alloc::collections::BTreeMap<u32, u32> =
            crate::daecore::daeshaper::derived::shaping_codes(text, |code| lookup.glyph_id(cmap, code).is_some())
            .into_iter()
            .filter_map(|code| Some((code, lookup.glyph_id(cmap, code)?)))
            .collect();
        use crate::daecore::daeshaper::unicode::{mirrored, vertical_form};
        let mut sequences: Vec<crate::daecore::daetype::subsetter::Sequence> = Vec::new();
        if text.chars().any(|c| is_variation_selector(c.into())) {
            let codes: alloc::collections::BTreeSet<u32> =
                text.chars().flat_map(|c| [Some(c), mirrored(c), vertical_form(c)]).flatten().map(u32::from).collect();
            for vs in codes.iter().copied().filter(|&c| is_variation_selector(c)) {
                for &base in &codes {
                    match lookup.variation(cmap, base, vs) {
                        Some(UvsLookup::Explicit(gid)) => sequences.push((vs, base, Some(gid))),
                        Some(UvsLookup::UseDefault) => sequences.push((vs, base, None)),
                        None => {}
                    }
                }
            }
        }
        let text_mappings: Vec<(u32, u32)> = seen.iter().map(|(&code, &glyph)| (code, glyph)).collect();
        let needs = self.arabic_fallback_needs(key, &text_mappings);
        for &code in needs.iter().flat_map(|n| &n.codes) {
            if let alloc::collections::btree_map::Entry::Vacant(slot) = seen.entry(code)
                && let Some(gid) = lookup.glyph_id(cmap, code) {
                    slot.insert(gid);
                }
        }
        let gids: Vec<u32> = seen.values().copied().chain(sequences.iter().filter_map(|s| s.2)).collect();
        let mappings: Vec<(u32, u32)> = seen.into_iter().collect();
        source.subset(&gids, Some(TextCmap { mappings: &mappings, sequences: &sequences }))
    }

    // What a subset of text mapped as `text` keeps for Arabic fallback shaping, where the font shapes Arabic so.
    fn arabic_fallback_needs(
        &self, key: &AxisKey, text: &[(u32, u32)],
    ) -> Option<crate::daecore::daeshaper::script::arabic::SubsetNeeds> {
        use crate::daecore::daeshaper::{face::Face, run::Direction, unicode};
        self.described_at(key, |at| {
            let face = Face::new(self).located(at);
            let arabic = unicode::script('\u{627}');
            let plan = self.shape_plan_cached(Some(arabic), &face, Direction::RightToLeft, None, None, &[], &[]);
            let fallback = crate::daecore::daeshaper::script::arabic::plan_fallback(&plan);
            fallback.map(|fallback| fallback.subset_needs(&plan.features, &face, text))
        })
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/cache/subset.rs"]
mod tests;
