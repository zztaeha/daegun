use super::*;
use crate::daecore::Error;
use super::super::format::sanitize::{read_as_judged, AatTable, Reads};
use crate::daecore::daetype::Tables;
use crate::daecore::daekit::base::sync::Shared;
use super::super::format::metrics::{bearing_at, shaped_tags, MetricTags, MetricsTable};
use super::super::instancer::glyf_bounds;
use super::super::instancer::GvarHeader;
use super::super::varc::flatten::{self, Draw};
use super::super::varc::subset::{covered_glyphs, subset, Subset as VarcSubset};
use super::gvar::subset_gvar;

#[cfg(test)]
pub fn subset_ttf(ttf: &[u8], requested: &[u32]) -> Result<Subset, Error> {
    let draw = |gids: &[u32]| {
        let map = super::super::decoder::extract_ttf_tables(ttf);
        map.map_or_else(|_| Ok(BTreeMap::new()), |map| crate::daecore::daekit::cache::varc_drawn_from(&map, &[], gids))
    };
    subset_ttf_at(ttf, requested, None, false, &draw)
}

// The cmap holds `text`'s mappings and sequences (source glyph IDs) when given, else every source mapping
// whose glyph the subset keeps. The font is an instance where `instanced`; `draw` draws VARC's glyphs.
fn subset_ttf_at(
    ttf: &[u8], requested: &[u32], text: Option<TextCmap>, instanced: bool, draw: Draw,
) -> Result<Subset, Error> {
    let dir = ttf_table_ranges(ttf)?;
    if slice_table(ttf, &dir, "fvar").is_some() {
        return subset_default_instance(&super::super::decoder::extract_ttf_tables(ttf)?, requested, text);
    }
    let table = |tag: &str| slice_table(ttf, &dir, tag);
    let entries = || Shared::new(CmapEntries::read(char_tables(table)));
    subset_glyf(table, None, requested, text, &entries, instanced, draw)
}

// A subset of a font's tables, as `subset_ttf_at` subsets them written out, with the font's verdicts and cmap
// entries. `instanced` where the tables are an instance's; `draw` draws VARC's glyphs where the subset stands.
pub(crate) fn subset_glyf_tables(
    map: &Tables, reads: &FaceReads, requested: &[u32], text: Option<TextCmap>,
    entries: &dyn Fn() -> Shared<CmapEntries>, instanced: bool, draw: Draw,
) -> Result<Subset, Error> {
    if map.contains_key("fvar") {
        return subset_default_instance(map, requested, text);
    }
    subset_glyf(|tag| map.get(tag).map(|t| t.as_slice()), Some(reads), requested, text, entries, instanced, draw)
}

// A subset carries no variations, so a variable font is subset at its default instance, with the variation data its
// other tables name resolved rather than left naming an fvar that is gone. VARC's glyphs are drawn from the font.
fn subset_default_instance(
    map: &Tables, requested: &[u32], text: Option<TextCmap>,
) -> Result<Subset, Error> {
    let instanced =
        build_ttf(&super::super::instancer::instance_tables_for_subset(map, &Reads::of_tables(map), &[])?)?;
    if ttf_table_ranges(&instanced)?.contains_key("fvar") {
        return Err(Error::malformed("subset: the default instance kept its variations"));
    }
    let draw = |gids: &[u32]| crate::daecore::daekit::cache::varc_drawn_from(map, &[], gids);
    subset_ttf_at(&instanced, requested, text, true, &draw)
}

// Each glyph `loca` places in `glyf` padded to an even length, `odd` of them by a byte, moved in place last first.
fn padded_to_even(glyf: &mut Vec<u8>, loca: &mut [usize], odd: usize) {
    let n = loca.len() - 1;
    let (mut end, mut next) = (glyf.len() + odd, loca[n]);
    glyf.resize(end, 0);
    loca[n] = end;
    for g in (0..n).rev() {
        let (start, len) = (loca[g], next - loca[g]);
        let to = end - len - len % 2;
        glyf.copy_within(start..start + len, to);
        if len % 2 == 1 {
            glyf[to + len] = 0;
        }
        (next, end, loca[g]) = (start, to, to);
    }
}

// A subset of a font with no fvar read through `table`, its extents recounted where it's an instance or holds glyphs
// `draw` drew. `reads` are the caller's verdicts, None to read them here; `entries` gives the cmap's.
fn subset_glyf<'a>(
    table: impl Fn(&str) -> Option<&'a [u8]> + Copy, reads: Option<&FaceReads>, requested: &[u32],
    text: Option<TextCmap>, entries: &dyn Fn() -> Shared<CmapEntries>, instanced: bool, draw: Draw,
) -> Result<Subset, Error> {
    let (head_tag, head) = closure::font_header(table).ok_or(Error::missing("subset: missing head"))?;
    let (num_glyphs, outlines) = closure::glyf_outlines(table)?;
    let taken;
    let reads = match reads {
        Some(reads) => reads,
        None => {
            taken = FaceReads::of(table);
            &taken
        }
    };
    let closure::Closed { active, stackings, colr, varced } =
        closure::glyf_closure_of(table, reads, num_glyphs, &outlines, requested)?;
    let closed: Vec<u32> = active.iter().collect();
    let mut gid_map = vec![0u32; closed.last().copied().unwrap_or(0) as usize + 1];
    for (compact, &orig) in closed.iter().enumerate() {
        gid_map[orig as usize] = compact as u32;
    }
    // VARC is subset with all axes pinned in an instance; where it can't be, each glyph VARC draws takes its
    // drawing's place, bearing at its xMin, as drawing shifts a glyf outline by any difference.
    let new_gid = |old: u32| active.contains(&old).then(|| gid_map[old as usize]);
    let varc = reads.reading(table)("VARC").map(|varc| (varc, subset(varc, &varced, new_gid, false, instanced)));
    let extended = outlines.as_ref().is_some_and(|&(.., tags)| tags.extended);
    let drawn: BTreeMap<u32, Vec<u8>> = match varc {
        Some((varc, VarcSubset::Refused)) if outlines.is_some() => {
            let gids: Vec<u32> = covered_glyphs(varc).into_iter().filter(|g| active.contains(g)).collect();
            draw(&gids)?.iter().filter_map(|(&gid, path)| Some((gid, flatten::glyf(path, extended)?))).collect()
        }
        _ => BTreeMap::new(),
    };

    // Glyph blocks run in glyph order, so a font copies at most its glyf; loca ranges that overlap copy
    // the same bytes again and stop at the subset budget. A glyph drawn is held to its drawing's own.
    let budget = subset_budget(outlines.as_ref().map_or(0, |(glyf, ..)| glyf.len()));
    let mut new_glyf: Vec<u8> = Vec::new();
    let mut new_loca = vec![0usize; closed.len() + 1];
    let (mut copied, mut odd) = (0usize, 0usize);
    if let Some((glyf, loca, tags)) = outlines.as_ref() {
        for (compact, &gid) in closed.iter().enumerate() {
            let glyph = match drawn.get(&gid) {
                Some(glyph) => glyph.as_slice(),
                None => {
                    let own = loca.glyph(glyf, gid).unwrap_or_default();
                    copied += own.len();
                    if copied > budget {
                        return Err(Error::limit("subset: glyf ranges past the subset budget"));
                    }
                    own
                }
            };
            let (glyph, at) = (trimmed(glyph), new_glyf.len());
            new_loca[compact] = at;
            new_glyf.extend_from_slice(glyph);
            if is_composite(glyph) {
                patch_compound_gids(glyph, &mut new_glyf[at..], tags.extended, &gid_map);
            }
            odd += glyph.len() % 2;
        }
    }
    new_loca[closed.len()] = new_glyf.len();
    // Each glyph is padded to an even length where all then fit short loca, which counts in halves, else none.
    if odd > 0 && new_glyf.len() + odd < 0x1_FFFF {
        padded_to_even(&mut new_glyf, &mut new_loca, odd);
    }
    let (new_loca_bytes, loca_format) = loca_table(&new_loca).ok_or(Error::malformed("subset: glyf past 4 GB"))?;

    let mut new_head = head.to_vec();
    if outlines.is_some() {
        write_i16_be(&mut new_head, 50, loca_format);
    }

    let mut tmap: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for &tag in GLYPH_FREE_TABLES {
        if let Some(d) = table(tag) { tmap.insert(tag.to_string(), d.to_vec()); }
    }
    if let Some(base) = table("BASE").filter(|b| super::super::base::base_is_glyph_free(b)) {
        tmap.insert("BASE".to_string(), base.to_vec());
    }
    tmap.insert(head_tag.to_string(), new_head);
    let tags = outlines.as_ref().map(|&(.., tags)| tags);
    if let Some(tags) = tags {
        // A glyf of empty glyphs holds one byte, as an empty table is refused.
        if new_glyf.is_empty() {
            new_glyf.push(0);
        }
        tmap.insert(tags.glyf.to_string(), new_glyf);
        tmap.insert(tags.loca.to_string(), new_loca_bytes);
    }
    if let Some((_, VarcSubset::Written(varc))) = varc {
        tmap.insert("VARC".into(), varc);
    }
    // A font with no fvar isn't instanced, and keeps its gvar: VARC's components vary their glyphs by it.
    for (tag, wide) in [("gvar", false), ("GVAR", true)].into_iter().filter(|_| !instanced) {
        let Some(gvar) = table(tag).filter(|g| GvarHeader::read(g, num_glyphs, wide).is_ok()) else { continue };
        let kept: Vec<(usize, u32)> = closed.iter().copied().enumerate().collect();
        tmap.extend(subset_gvar(gvar, wide, num_glyphs, &kept, closed.len()).transpose()?.map(|g| (tag.into(), g)));
    }
    let kept = KeptGlyphs { num_glyphs, closed, active, gid_map, renumbered: true, instanced, stackings, colr };
    side_tables(&mut tmap, table, reads, &kept, text, entries);
    if !drawn.is_empty() {
        let bearings: Vec<(usize, i16)> = drawn.iter()
            .filter_map(|(&old, glyph)| Some((kept.gid_map[old as usize] as usize, read_i16_be(glyph, 2)?)))
            .collect();
        set_bearings(&mut tmap, &bearings);
    }
    if instanced || !drawn.is_empty() {
        let glyf = tags.and_then(|tags| tmap.get(tags.glyf)).map_or(&[][..], Vec::as_slice);
        let glyphs: Vec<_> = glyf_bounds(glyf, &new_loca, kept.closed.len()).into_iter().enumerate().collect();
        let outlines = tags.map(|tags| (glyf, tags.extended, &new_loca[..]));
        let rewritten = kept_extents(&tmap, &glyphs, outlines);
        tmap.extend(rewritten.into_iter().map(|(tag, table)| (tag.to_string(), table)));
    }
    Ok(Subset { font: build_ttf(&tmap)?, gid_map: kept.gid_map })
}

// Each of `glyphs`' side bearings (new id, bearing) in the subset's hmtx and the HMTX beside it.
fn set_bearings(tmap: &mut BTreeMap<String, Vec<u8>>, glyphs: &[(usize, i16)]) {
    for tags in [shaped_tags(tmap, false), MetricTags::lower(false)] {
        let long = tags.long(tmap);
        let Some(mtx) = tmap.get_mut(tags.mtx) else { continue };
        for &(gid, bearing) in glyphs {
            if let Some(at) = bearing_at(long, gid).filter(|&at| at + 2 <= mtx.len()) {
                mtx[at..at + 2].copy_from_slice(&bearing.to_be_bytes());
            }
        }
    }
}

// An instance subset's head, hhea, vhea, OS/2, maxp and their beyond-64K twins, recounted over `glyphs` (new id, box
// where inked); `glyf` is its outlines and loca, None for CFF2, whose head flags and maxp stay.
pub(crate) fn kept_extents<V: AsRef<[u8]>>(
    map: &BTreeMap<String, V>, glyphs: &[(usize, Option<[i16; 4]>)], glyf: Option<(&[u8], bool, &[usize])>,
) -> Vec<(&'static str, Vec<u8>)> {
    use super::super::instancer::{recompute_extents, GlyfCounts};
    let table = |tag: &str| map.get(tag).map(V::as_ref);
    let Some(mut head) = table("head").map(<[u8]>::to_vec) else { return Vec::new() };
    let [h, v] = [false, true].map(|vertical| shaped_tags(map, vertical));
    let metrics = |tags: MetricTags| table(tags.mtx).map(|data| MetricsTable::new(data, tags.long(map)));
    let (hmtx, vmtx) = (metrics(h).unwrap_or_else(|| MetricsTable::new(&[], 0)), metrics(v));
    let [mut hhea, mut vhea, mut os2] = [h.hea, v.hea, "OS/2"].map(|tag| table(tag).map(<[u8]>::to_vec));
    let (hea, vea, os) = (hhea.as_deref_mut(), vhea.as_deref_mut(), os2.as_deref_mut());
    recompute_extents(glyphs.iter().copied(), &hmtx, vmtx.as_ref(), &mut head, hea, vea, os);
    // Beside the beyond-64K pair, hhea and vhea take the same extents.
    let [mut lower_hhea, mut lower_vhea] = [false, true].map(|vertical| {
        let tag = MetricTags::lower(vertical).hea;
        table(tag).filter(|_| tag != [h.hea, v.hea][usize::from(vertical)]).map(<[u8]>::to_vec)
    });
    if lower_hhea.is_some() || lower_vhea.is_some() {
        let (hea, vea) = (lower_hhea.as_deref_mut(), lower_vhea.as_deref_mut());
        recompute_extents(glyphs.iter().copied(), &hmtx, vmtx.as_ref(), &mut [], hea, vea, None);
    }
    let maxp = glyf.and_then(|(glyf, extended, loca)| {
        let counts = GlyfCounts::of(glyphs.iter().copied(), glyf, extended, loca, &hmtx)?;
        counts.write_head(&mut head);
        Some([("maxp", false), ("MAXP", true)].map(|(tag, wide)| (tag, table(tag).and_then(|m| counts.maxp(m, wide)))))
    });
    let headers = [(h.hea, hhea), (v.hea, vhea), ("hhea", lower_hhea), ("vhea", lower_vhea), ("OS/2", os2)];
    [("head", Some(head))]
        .into_iter()
        .chain(headers)
        .chain(maxp.into_iter().flatten())
        .filter_map(|(tag, rewritten)| Some((tag, rewritten?)))
        .collect()
}

// The glyphs a subset keeps, in order and as a set, of a source holding `num_glyphs`; the id each takes, its own
// unless `renumbered`; whether the source is an instance; morx's stackings over them; and what COLR's stage keeps.
pub(crate) struct KeptGlyphs<'a> {
    pub(crate) num_glyphs: usize,
    pub(crate) closed: Vec<u32>,
    pub(crate) active: GlyphSet,
    pub(crate) gid_map: Vec<u32>,
    pub(crate) renumbered: bool,
    pub(crate) instanced: bool,
    pub(crate) stackings: Option<Stackings<'a>>,
    pub(crate) colr: colr::ColrKept,
}

// What every flavor rebuilds into `out` beside its own tables: layout, post, color, cmap, name, OS/2, then, renumbered,
// tables naming glyphs. One rebuilt to nothing goes, uncountable metrics stay; `text`, `entries` per `subset_glyf`.
pub(crate) fn side_tables<'a, V: AsRef<[u8]> + From<Vec<u8>>>(
    out: &mut BTreeMap<String, V>, table: impl Fn(&str) -> Option<&'a [u8]> + Copy, reads: &Reads, kept: &KeptGlyphs,
    text: Option<TextCmap>, entries: &dyn Fn() -> Shared<CmapEntries>,
) {
    let KeptGlyphs { num_glyphs, ref closed, ref active, ref gid_map, renumbered, instanced, ref stackings, .. } =
        *kept;
    let n = num_glyphs as u32;
    let put = |out: &mut BTreeMap<String, V>, tag: &str, rebuilt: Option<Vec<u8>>| {
        if table(tag).is_none() { return; }
        match rebuilt {
            Some(bytes) => { out.insert(tag.to_string(), bytes.into()); }
            None => { out.remove(tag); }
        }
    };
    for (tag, rebuilt) in ["GDEF", "GSUB", "GPOS"].into_iter().zip(otl::subset_layout(table, reads, active, gid_map)) {
        put(out, tag, rebuilt);
    }
    if let Some(post) = table("post") {
        out.insert("post".to_string(), fix_post_table(post.to_vec()).into());
    }
    if renumbered {
        // Each pair the font holds is subset from its own records; an instance fills hmtx and HMTX from one map, hmtx
        // cut at 65,535 long records, so an instance's hmtx is subset from HMTX.
        let pairs = [false, true].map(|v| (MetricTags::lower(v), MetricTags::upper(v)));
        for (tags, upper) in pairs.into_iter().flat_map(|(lower, upper)| [(lower, upper), (upper, upper)]) {
            let (Some(mtx), Some(hea)) = (table(tags.mtx), table(tags.hea)) else { continue };
            let (from, from_mtx, from_hea) = match (table(upper.mtx), table(upper.hea)) {
                (Some(upper_mtx), Some(upper_hea)) if instanced && !tags.wide => (upper, upper_mtx, upper_hea),
                _ => (tags, mtx, hea),
            };
            let Some(num_long) = from.stated(from_hea) else { continue };
            let (new_mtx, long) = rebuild_metrics(from_mtx, num_long, num_glyphs, closed, tags.wide);
            let mut new_hea = hea.to_vec();
            tags.write_long(&mut new_hea, long);
            out.insert(tags.mtx.to_string(), new_mtx.into());
            out.insert(tags.hea.to_string(), new_hea.into());
        }
        // maxp counts the subset's glyphs up to 65,535, MAXP to 2^24 - 1.
        if let Some(maxp) = table("maxp").filter(|m| m.len() >= 6) {
            let mut maxp = maxp.to_vec();
            write_u16_be(&mut maxp, 4, closed.len().min(0xFFFF) as u16);
            out.insert("maxp".to_string(), maxp.into());
        }
        if let Some(maxp) = table("MAXP").filter(|m| m.len() >= 7) {
            let mut maxp = maxp.to_vec();
            maxp[4..7].copy_from_slice(&(closed.len().min(0xFF_FFFF) as u32).to_be_bytes()[1..]);
            out.insert("MAXP".to_string(), maxp.into());
        }
        // MERG's class definitions name glyph ids; without it, glyphs are always merged.
        out.remove("MERG");
        for (tag, rebuilt) in [
            ("hdmx", table("hdmx").and_then(|h| device_metrics::subset_hdmx(h, num_glyphs, closed))),
            ("VORG", table("VORG").and_then(|v| remap_vorg(v, gid_map, active))),
            ("kern", table("kern").and_then(|k| remap_kern(k, gid_map, active))),
            ("MATH", table("MATH").and_then(|m| math::subset_math(m, active, gid_map))),
            ("JSTF", table("JSTF").and_then(|j| jstf::subset_jstf(j, active, gid_map))),
            ("lcar", table("lcar").and_then(|d| aat::simple::subset_lcar(d, active, gid_map, n))),
            ("opbd", table("opbd").and_then(|d| aat::simple::subset_opbd(d, active, gid_map, n))),
            ("ankr", table("ankr").and_then(|d| aat::simple::subset_ankr(d, active, gid_map, n))),
            ("bsln", table("bsln").and_then(|d| aat::simple::subset_bsln(d, active, gid_map, n))),
            ("fmtx", table("fmtx").and_then(|d| aat::simple::subset_fmtx(d, active, gid_map))),
            ("Zapf", table("Zapf").and_then(|d| aat::zapf::subset_zapf(d, num_glyphs, closed, active, gid_map))),
            // A morx shaping doesn't read is dropped below, and the closure neither walked nor charged it.
            ("morx", table("morx").filter(|_| reads.shaped(AatTable::Morx)).and_then(|d| {
                aat::morx::subset_morx_with(d, active, gid_map, n, stackings.as_ref())
            })),
            ("just", table("just").and_then(|d| aat::just::subset_just(d, active, gid_map, n))),
            ("kerx", table("kerx").and_then(|d| aat::kerx::subset_kerx(d, active, gid_map, n))),
            ("prop", table("prop").and_then(|d| aat::prop::subset_prop(d, num_glyphs, active, gid_map))),
            ("LTSH", table("LTSH").and_then(|l| device_metrics::subset_ltsh(l, closed))),
            ("sbix", table("sbix").and_then(|s| bitmap::subset_sbix(s, num_glyphs, closed, gid_map))),
        ] {
            put(out, tag, rebuilt);
        }
        for (data_tag, loc_tag) in [("CBDT", "CBLC"), ("EBDT", "EBLC"), ("bdat", "bloc")] {
            let (Some(data), Some(loc)) = (table(data_tag), table(loc_tag)) else { continue };
            out.remove(data_tag);
            out.remove(loc_tag);
            if let Some((new_loc, new_data)) = bitmap::subset_bitmap_strikes(loc, data, active, gid_map) {
                out.insert(loc_tag.to_string(), new_loc.into());
                out.insert(data_tag.to_string(), new_data.into());
            }
        }
    }
    read_as_judged(reads, out);
    if renumbered {
        // Both after the tables they read back from the subset: EBSC the kept strikes, xref the kerx and morx it keeps.
        let strikes = out.get("EBLC").map(|b| aat::descriptive::strike_sizes(b.as_ref())).unwrap_or_default();
        put(out, "EBSC", table("EBSC").and_then(|e| aat::descriptive::subset_ebsc(e, &strikes)));
        let stable = |tag: &[u8]| {
            let name = core::str::from_utf8(tag).unwrap_or_default();
            aat::descriptive::subtable_counts(tag, table(name))
                == aat::descriptive::subtable_counts(tag, out.get(name).map(AsRef::as_ref))
        };
        let xref = table("xref").and_then(|x| aat::descriptive::subset_xref(x, stable));
        put(out, "xref", xref);
    }
    // A table the sanitizer rejects goes, as any table the subset can't read does. CPAL is written from the palette
    // indices COLR's stage kept, whether or not COLR's subset is.
    let read = reads.reading(table);
    put(out, "COLR", read("COLR").and_then(|colr| colr::subset_colr(colr, &kept.colr, active, gid_map)));
    put(out, "CPAL", read("CPAL").and_then(|cpal| colr::subset_cpal(cpal, &kept.colr)));
    let palette_names = read("CPAL").map(|cpal| colr::cpal_names(cpal, &kept.colr)).unwrap_or_default();

    let new_gid = |g: u32| remap_gid(active, gid_map, g);
    let os2 = table("OS/2");
    let (mappings, sequences) = match text {
        Some(text) => remap_text(text, new_gid),
        None => entries().kept(new_gid),
    };
    let subset = |tag: &str| out.get(tag).map(AsRef::as_ref);
    let source = char_tables(table);
    let written = display_tables(source, &mappings, &sequences, table("name"), os2, subset, palette_names);
    out.insert("cmap".to_string(), written.cmap.into());
    if let Some(dmap) = written.dmap {
        out.insert("DMAP".to_string(), dmap.into());
    }
    out.insert("name".to_string(), written.name.into());
    if let Some(os2) = written.os2.or_else(|| os2.map(<[u8]>::to_vec)) {
        out.insert("OS/2".to_string(), os2.into());
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/ttf.rs"]
mod tests;
