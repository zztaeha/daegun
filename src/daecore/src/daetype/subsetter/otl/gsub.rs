use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::string::String;
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::{read_u16_be, read_i16_be, write_u16_be, records_fit};
use super::{parse_coverage, build_coverage, remap_gid};
use super::lookup_list;
use super::context;
use super::generic::{self, schemas};

pub(crate) fn parse_single_subst(buf: &[u8], off: usize) -> Result<Vec<(u16, u16)>, String> {
    let format = read_u16_be(buf, off).ok_or("SingleSubst: truncated")?;
    let cov_off = read_u16_be(buf, off + 2).ok_or("SingleSubst: truncated")? as usize;
    let coverage = parse_coverage(buf, off + cov_off)?;
    match format {
        1 => {
            let delta = read_i16_be(buf, off + 4).ok_or("SingleSubst format 1: truncated")?;
            Ok(coverage.iter().map(|&g| (g, (g as i32 + delta as i32).rem_euclid(65536) as u16)).collect())
        }
        2 => {
            let count = read_u16_be(buf, off + 4).ok_or("SingleSubst format 2: truncated")? as usize;
            let mut pairs = Vec::with_capacity(count.min(coverage.len()));
            for (i, &g) in coverage.iter().enumerate().take(count) {
                let sub = read_u16_be(buf, off + 6 + i * 2).ok_or("SingleSubst format 2: substitute array truncated")?;
                pairs.push((g, sub));
            }
            Ok(pairs)
        }
        _ => Err(format!("SingleSubst: unknown format {}", format)),
    }
}

pub(crate) fn parse_coverage_indexed_glyph_arrays(buf: &[u8], off: usize) -> Result<Vec<(u16, Vec<u16>)>, String> {
    let cov_off = read_u16_be(buf, off + 2).ok_or("coverage-indexed glyph array subtable: truncated")? as usize;
    let coverage = parse_coverage(buf, off + cov_off)?;
    let count = read_u16_be(buf, off + 4).ok_or("coverage-indexed glyph array subtable: truncated")? as usize;
    let mut result = Vec::with_capacity(count.min(coverage.len()));
    for (i, &g) in coverage.iter().enumerate().take(count) {
        let arr_rel = read_u16_be(buf, off + 6 + i * 2).ok_or("coverage-indexed glyph array subtable: array offset truncated")? as usize;
        let arr_off = off + arr_rel;
        let glyph_count = read_u16_be(buf, arr_off).ok_or("coverage-indexed glyph array subtable: array truncated")? as usize;
        let mut glyphs = Vec::with_capacity(glyph_count);
        for j in 0..glyph_count {
            glyphs.push(read_u16_be(buf, arr_off + 2 + j * 2).ok_or("coverage-indexed glyph array subtable: glyph truncated")?);
        }
        result.push((g, glyphs));
    }
    Ok(result)
}

pub(crate) type LigatureSet = Vec<(u16, Vec<u16>)>;

const MAX_LIGATURE_ENTRIES: usize = 1_000_000;

pub(crate) fn parse_ligature_subst(buf: &[u8], off: usize) -> Result<Vec<(u16, LigatureSet)>, String> {
    let cov_off = read_u16_be(buf, off + 2).ok_or("LigatureSubst: truncated")? as usize;
    let coverage = parse_coverage(buf, off + cov_off)?;
    let lig_set_count = read_u16_be(buf, off + 4).ok_or("LigatureSubst: truncated")? as usize;
    if !records_fit(off + 6, lig_set_count, 2, buf.len()) {
        return Err("LigatureSubst: LigatureSet offset array does not fit".into());
    }
    let mut budget = MAX_LIGATURE_ENTRIES;
    let mut result = Vec::with_capacity(lig_set_count.min(coverage.len()).min(256));
    for (i, &first) in coverage.iter().enumerate().take(lig_set_count) {
        let ls_rel = read_u16_be(buf, off + 6 + i * 2).ok_or("LigatureSubst: LigatureSet offset truncated")? as usize;
        let ls_off = off + ls_rel;
        let lig_count = read_u16_be(buf, ls_off).ok_or("LigatureSubst: LigatureSet truncated")? as usize;
        if !records_fit(ls_off + 2, lig_count, 2, buf.len()) {
            return Err("LigatureSubst: Ligature offset array does not fit".into());
        }
        let mut ligs: LigatureSet = Vec::with_capacity(lig_count.min(256));
        for j in 0..lig_count {
            let lig_rel = read_u16_be(buf, ls_off + 2 + j * 2).ok_or("LigatureSubst: Ligature offset truncated")? as usize;
            let lig_off = ls_off + lig_rel;
            let lig_glyph = read_u16_be(buf, lig_off).ok_or("Ligature: truncated")?;
            let comp_count = read_u16_be(buf, lig_off + 2).ok_or("Ligature: truncated")? as usize;
            if comp_count == 0 { return Err("Ligature: CompCount must be at least 1".into()); }
            if !records_fit(lig_off + 4, comp_count - 1, 2, buf.len()) {
                return Err("Ligature: component array does not fit".into());
            }
            budget = budget.checked_sub(comp_count).ok_or("LigatureSubst: entry budget exhausted")?;
            let mut components = Vec::with_capacity((comp_count - 1).min(256));
            for k in 0..comp_count - 1 {
                components.push(read_u16_be(buf, lig_off + 4 + k * 2).ok_or("Ligature: component truncated")?);
            }
            ligs.push((lig_glyph, components));
        }
        result.push((first, ligs));
    }
    Ok(result)
}

struct ReverseChainSingleSubst {
    backtrack: Vec<Vec<u16>>,
    lookahead: Vec<Vec<u16>>,
    pairs: Vec<(u16, u16)>,
}

fn parse_reverse_chain_single_subst(buf: &[u8], off: usize) -> Result<ReverseChainSingleSubst, String> {
    let cov_off = read_u16_be(buf, off + 2).ok_or("ReverseChainSingleSubst: truncated")? as usize;
    let bt_count = read_u16_be(buf, off + 4).ok_or("ReverseChainSingleSubst: truncated")? as usize;
    let backtrack = context::parse_coverage_array(buf, off, off + 6, bt_count)?;
    let la_off = off + 6 + bt_count * 2;
    let la_count = read_u16_be(buf, la_off).ok_or("ReverseChainSingleSubst: truncated")? as usize;
    let lookahead = context::parse_coverage_array(buf, off, la_off + 2, la_count)?;
    let gc_off = la_off + 2 + la_count * 2;
    let glyph_count = read_u16_be(buf, gc_off).ok_or("ReverseChainSingleSubst: truncated")? as usize;
    let coverage = parse_coverage(buf, off + cov_off)?;
    let mut pairs = Vec::with_capacity(glyph_count.min(coverage.len()));
    for (i, &g) in coverage.iter().enumerate().take(glyph_count) {
        let sub = read_u16_be(buf, gc_off + 2 + i * 2).ok_or("ReverseChainSingleSubst: substitute array truncated")?;
        pairs.push((g, sub));
    }
    Ok(ReverseChainSingleSubst { backtrack, lookahead, pairs })
}

fn subset_reverse_chain_single_subst(buf: &[u8], off: usize, active: &GlyphSet, gid_map: &[u16]) -> Option<Vec<u8>> {
    let parsed = parse_reverse_chain_single_subst(buf, off).ok()?;
    let new_backtrack = context::filter_coverage_group(&parsed.backtrack, active, gid_map)?;
    let new_lookahead = context::filter_coverage_group(&parsed.lookahead, active, gid_map)?;

    let mut remapped: Vec<(u16, u16)> = parsed.pairs.into_iter()
        .filter_map(|(g, s)| match (remap_gid(active, gid_map, g), remap_gid(active, gid_map, s)) {
            (Some(ng), Some(ns)) => Some((ng, ns)),
            _ => None,
        })
        .collect();
    if remapped.is_empty() { return None; }
    // In the order the rebuilt coverage lists glyphs, one substitute each: a glyph listed twice keeps
    // its first, so the two stay the same length.
    remapped.sort_by_key(|&(g, _)| g);
    remapped.dedup_by_key(|&mut (g, _)| g);

    let bt_blobs = context::build_coverage_array_blobs(&new_backtrack);
    let la_blobs = context::build_coverage_array_blobs(&new_lookahead);
    let cov_gids: Vec<u16> = remapped.iter().map(|&(g, _)| g).collect();
    let cov_bytes = build_coverage(&cov_gids);

    let header_len = 6 + bt_blobs.len() * 2 + 2 + la_blobs.len() * 2 + 2 + remapped.len() * 2;
    let mut out = vec![0u8; header_len];
    write_u16_be(&mut out, 0, 1);
    write_u16_be(&mut out, 4, u16::try_from(bt_blobs.len()).ok()?);
    let mut pos = header_len;
    for (i, blob) in bt_blobs.iter().enumerate() {
        write_u16_be(&mut out, 6 + i * 2, u16::try_from(pos).ok()?);
        pos = pos.checked_add(blob.len())?;
    }
    let la_count_off = 6 + bt_blobs.len() * 2;
    write_u16_be(&mut out, la_count_off, u16::try_from(la_blobs.len()).ok()?);
    for (i, blob) in la_blobs.iter().enumerate() {
        write_u16_be(&mut out, la_count_off + 2 + i * 2, u16::try_from(pos).ok()?);
        pos = pos.checked_add(blob.len())?;
    }
    let gc_off = la_count_off + 2 + la_blobs.len() * 2;
    write_u16_be(&mut out, gc_off, u16::try_from(remapped.len()).ok()?);
    for (i, &(_, sub)) in remapped.iter().enumerate() { write_u16_be(&mut out, gc_off + 2 + i * 2, sub); }
    write_u16_be(&mut out, 2, u16::try_from(pos).ok()?);
    for blob in &bt_blobs { out.extend_from_slice(blob); }
    for blob in &la_blobs { out.extend_from_slice(blob); }
    out.extend(cov_bytes);
    Some(out)
}

pub(crate) fn resolve_effective_type(gsub: &[u8], lookup_type: u16, sub_off: usize) -> Option<(u16, usize)> {
    lookup_list::resolve_effective_type(gsub, 7, lookup_type, sub_off)
}

// Lookups and subtables may be named by offset many times over; each is visited once.
pub(crate) fn each_lookup_subtable(gsub: &[u8], mut f: impl FnMut(u16, &[u8], usize)) -> Option<()> {
    let lookup_off = read_u16_be(gsub, 8)? as usize;
    let count = read_u16_be(gsub, lookup_off)?;
    let (mut lookups, mut subtables) = (alloc::collections::BTreeSet::new(), alloc::collections::BTreeSet::new());
    for i in 0..count as usize {
        let rel = read_u16_be(gsub, lookup_off + 2 + i * 2)?;
        if !lookups.insert(rel) { continue; }
        let lookup_start = lookup_off + rel as usize;
        let Some(lookup_type) = read_u16_be(gsub, lookup_start) else { continue };
        let Some(sub_count) = read_u16_be(gsub, lookup_start + 4) else { continue };
        for j in 0..sub_count as usize {
            let Some(srel) = read_u16_be(gsub, lookup_start + 6 + j * 2) else { break };
            if let Some((real_type, real_off)) = resolve_effective_type(gsub, lookup_type, lookup_start + srel as usize)
                && subtables.insert((real_type, real_off))
            {
                f(real_type, gsub, real_off);
            }
        }
    }
    Some(())
}

// GSUB's substitutions as edges, read once for every closure pass. A range coverage names thousands
// of glyphs in six bytes, so the edges stop at the subset budget and the table is refused.
pub(crate) struct GsubEdges {
    edges: Vec<(u16, u16)>,
    ligatures: Vec<(u16, u16, core::ops::Range<usize>)>,
    components: Vec<u16>,
}

impl GsubEdges {
    pub(crate) fn new(gsub: &[u8]) -> Option<GsubEdges> {
        let budget = super::super::subset_budget(gsub.len());
        let mut e = GsubEdges { edges: Vec::new(), ligatures: Vec::new(), components: Vec::new() };
        let mut over = false;
        each_lookup_subtable(gsub, |lookup_type, buf, off| if !over { match lookup_type {
            1 => if let Ok(pairs) = parse_single_subst(buf, off) { e.edges.extend(pairs); },
            2 | 3 => if let Ok(entries) = parse_coverage_indexed_glyph_arrays(buf, off) {
                for (orig, outs) in entries { e.edges.extend(outs.into_iter().map(|o| (orig, o))); }
            },
            4 => if let Ok(entries) = parse_ligature_subst(buf, off) {
                for (first, ligs) in entries {
                    for (lig_glyph, components) in ligs {
                        let at = e.components.len();
                        e.components.extend(components);
                        e.ligatures.push((first, lig_glyph, at..e.components.len()));
                    }
                }
            },
            8 => if let Ok(parsed) = parse_reverse_chain_single_subst(buf, off) { e.edges.extend(parsed.pairs); },
            _ => {}
        }
        over = e.edges.len() + e.components.len() + e.ligatures.len() > budget; });
        (!over).then_some(e)
    }

    // The glyphs one substitution takes the active ones to.
    pub(crate) fn reach(&self, active: &GlyphSet) -> Vec<u16> {
        let mut found: Vec<u16> = self.edges.iter().filter(|(input, _)| active.contains(input)).map(|&(_, o)| o).collect();
        found.extend(self.ligatures.iter()
            .filter(|(first, _, components)| active.contains(first) && self.components[components.clone()].iter().all(|c| active.contains(c)))
            .map(|&(_, lig_glyph, _)| lig_glyph));
        found
    }
}

pub fn gsub_closure(gsub: &[u8], active: &GlyphSet) -> Vec<u16> {
    GsubEdges::new(gsub).map(|e| e.reach(active)).unwrap_or_default()
}

pub(crate) fn subset_gsub_subtable(
    effective_type: u16, schema: Option<&generic::schema::Schema>, buf: &[u8], off: usize, active: &GlyphSet,
    gid_map: &[u16], work: &mut generic::Work,
) -> Option<Vec<u8>> {
    if effective_type == 8 {
        return subset_reverse_chain_single_subst(buf, off, active, gid_map);
    }
    generic::subset_subtable(buf, off, schema?, active, gid_map, generic::Devices::Keep, work)
}

// A lookup keeps its mark filtering set only if it is one of the `mark_sets` the subset's GDEF holds.
pub fn subset_gsub(gsub: &[u8], active: &GlyphSet, gid_map: &[u16], mark_sets: u16) -> Option<Vec<u8>> {
    let schema_for_type = &schemas::gsub_schema_for_type;
    lookup_list::subset_lookup_table(gsub, 7, active, gid_map, mark_sets, &subset_gsub_subtable, schema_for_type)
}

#[cfg(test)]
mod shared_lookups {
    use super::*;

    // 2,000 lookup list entries naming one lookup, which names one SingleSubst (5 to 6) 2,000 times.
    fn gsub() -> Vec<u8> {
        const N: u16 = 2_000;
        let mut t = [1u16, 0, 0, 0, 10, N].map(u16::to_be_bytes).concat();
        (0..N).for_each(|_| t.extend((2 + 2 * N).to_be_bytes()));
        t.extend([1u16, 0, N].map(u16::to_be_bytes).concat());
        (0..N).for_each(|_| t.extend((6 + 2 * N).to_be_bytes()));
        t.extend([1u16, 6, 1, 1, 1, 5].map(u16::to_be_bytes).concat());
        t
    }

    // 2,000 entries naming one lookup of a 1,000-glyph SingleSubst: 8 MB of copies from 8 KB of table.
    #[test]
    fn a_lookup_named_past_the_budget_is_refused() {
        const N: u16 = 2_000;
        const K: u16 = 1_000;
        let mut t = [1u16, 0, 0, 0, 10, N].map(u16::to_be_bytes).concat();
        (0..N).for_each(|_| t.extend((2 + 2 * N).to_be_bytes()));
        t.extend([1u16, 0, 1, 8].map(u16::to_be_bytes).concat());
        t.extend([2u16, 6 + 2 * K, K].map(u16::to_be_bytes).concat());
        t.extend((1..=K).flat_map(|g| (g + K).to_be_bytes()));
        t.extend([1u16, K].map(u16::to_be_bytes).concat());
        t.extend((1..=K).flat_map(u16::to_be_bytes));
        let mut active = GlyphSet::new();
        (0..=2 * K).for_each(|g| { active.insert(g); });
        let gid_map: Vec<u16> = (0..=2 * K).collect();
        assert!(subset_gsub(&t, &active, &gid_map, 0).is_none(), "8 MB of lookup copies were written");
    }

    #[test]
    fn a_shared_subtable_is_closed_over_once() {
        let mut active = GlyphSet::new();
        active.insert(5);
        let found = gsub_closure(&gsub(), &active);
        assert_eq!(found.len(), 1, "one substitution, named four million times, found {}", found.len());
    }

    #[test]
    fn a_shared_subtable_is_rebuilt_once_a_lookup() {
        let t = gsub();
        let mut active = GlyphSet::new();
        (0..8).for_each(|g| { active.insert(g); });
        let out = subset_gsub(&t, &active, &(0..8).collect::<Vec<u16>>(), 0).expect("a subset GSUB");
        let list = usize::from(read_u16_be(&out, 8).expect("a lookup list"));
        let lookup = list + usize::from(read_u16_be(&out, list + 2).expect("a lookup"));
        assert_eq!(read_u16_be(&out, lookup + 4), Some(1), "the lookup kept its subtable more than once");
    }
}

#[cfg(test)]
mod layout {
    use super::*;
    use crate::daecore::daetype::decoder::read_u32_be;

    fn words(w: &[u16]) -> Vec<u8> {
        w.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    fn all(n: u16) -> (GlyphSet, Vec<u16>) {
        let mut active = GlyphSet::new();
        (0..n).for_each(|g| { active.insert(g); });
        (active, (0..n).collect())
    }

    fn devanagari_gsub() -> Vec<u8> {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/noto-devanagari/NotoSansDevanagari.ttf");
        let ttf = std::fs::read(path).expect("the fixture font");
        let tables = crate::daecore::daetype::decoder::extract_ttf_tables(&ttf).expect("its tables");
        tables.get("GSUB").expect("a GSUB").to_owned_vec()
    }

    // The same table at version 1.1, four header bytes longer for the FeatureVariations offset `fv`.
    fn as_1_1(gsub: &[u8], fv: u32) -> Vec<u8> {
        let mut t = words(&[1, 1]);
        for at in [4, 6, 8] {
            t.extend((read_u16_be(gsub, at).expect("a header offset") + 4).to_be_bytes());
        }
        t.extend(fv.to_be_bytes());
        t.extend(&gsub[10..]);
        t
    }

    fn lookups(t: &[u8]) -> &[u8] {
        &t[usize::from(read_u16_be(t, 8).expect("a lookup list"))..]
    }

    // Version 1.1 with no FeatureVariations, as the instancer leaves a table once it has applied them,
    // and a kilobyte after it: kept whole ahead of the lookups, the table would pass 64 KB and GSUB go.
    #[test]
    fn a_version_1_1_table_is_trimmed_as_1_0_is() {
        let gsub = devanagari_gsub();
        let (active, gid_map) = all(1117);
        let want = subset_gsub(&gsub, &active, &gid_map, 10).expect("the 1.0 table subsets");
        let mut t = as_1_1(&gsub, 0);
        t.extend([0u8; 1024]);
        let got = subset_gsub(&t, &active, &gid_map, 10).expect("the 1.1 table subsets");
        assert!(got.len() <= want.len() + 4, "{} bytes where 1.0 gives {}", got.len(), want.len());
        assert_eq!(lookups(&got), lookups(&want));
    }

    // FeatureVariations at the table's end, whose alternate for feature 0 runs lookup 0 on axis 0 from
    // 0.5 to 1, move whole after the lookups, not after a copy of the whole table.
    #[test]
    fn feature_variations_move_whole_after_the_lookups() {
        let gsub = devanagari_gsub();
        let (active, gid_map) = all(1117);
        let want = subset_gsub(&gsub, &active, &gid_map, 10).expect("the 1.0 table subsets");
        let fv = [
            words(&[1, 0, 0, 1, 0, 16, 0, 30]),
            words(&[1, 0, 6, 1, 0, 0x2000, 0x4000]),
            words(&[1, 0, 1, 0, 0, 12, 0, 1, 0]),
        ]
        .concat();
        let mut t = as_1_1(&gsub, gsub.len() as u32 + 4);
        t.extend(&fv);
        let got = subset_gsub(&t, &active, &gid_map, 10).expect("the 1.1 table subsets");
        let at = read_u32_be(&got, 10).expect("a FeatureVariations offset") as usize;
        assert_eq!(read_u16_be(&got, 2), Some(1));
        assert_eq!(got.get(at..at + fv.len()), Some(&fv[..]));
        assert!(got.len() <= want.len() + 4 + fv.len(), "{} bytes where 1.0 gives {}", got.len(), want.len());
    }

    // The FeatureVariations of `feature_variations_move_whole_after_the_lookups`, its condition in
    // `format`, ahead of a FeatureList and LookupList: a 1.1 table of 100 bytes.
    fn variations_first(format: u16) -> (Vec<u8>, Vec<u8>) {
        let fv = [
            words(&[1, 0, 0, 1, 0, 16, 0, 30]),
            words(&[1, 0, 6, format, 0, 0x2000, 0x4000]),
            words(&[1, 0, 1, 0, 0, 12, 0, 1, 0]),
        ]
        .concat();
        let mut t = words(&[1, 1, 0, 62, 76, 0, 14]);
        t.extend(&fv);
        t.extend(words(&[1, 0x6C69, 0x6761, 8, 0, 1, 0]));
        t.extend(words(&[1, 4, 1, 0, 1, 8, 1, 6, 1, 1, 1, 5]));
        (t, fv)
    }

    // Already ahead of the lookups, the FeatureVariations stay where they are, written once.
    #[test]
    fn feature_variations_in_the_kept_prefix_stay() {
        let (t, fv) = variations_first(1);
        let (active, gid_map) = all(8);
        let out = subset_gsub(&t, &active, &gid_map, 0).expect("a subset GSUB");
        assert_eq!(read_u32_be(&out, 10), Some(14));
        assert_eq!(out.windows(fv.len()).filter(|w| *w == &fv[..]).count(), 1);
    }

    // A condition format past 1 may reach further than this reader knows, so the whole source stays
    // ahead of the lookups, FeatureVariations and all it could reach in place.
    #[test]
    fn feature_variations_not_understood_keep_the_whole_source() {
        let (t, _) = variations_first(2);
        let (active, gid_map) = all(8);
        let out = subset_gsub(&t, &active, &gid_map, 0).expect("a subset GSUB");
        assert_eq!(read_u32_be(&out, 10), Some(14));
        assert_eq!(out.get(14..t.len()), Some(&t[14..]));
    }

    // ScriptList and FeatureList offsets of 0 are empty lists. Read as lists at offset 0, they would make
    // the whole table stay ahead of the lookups.
    #[test]
    fn null_lists_are_empty() {
        let mut t = words(&[1, 0, 0, 0, 10, 1, 4, 1, 0, 1, 8, 1, 6, 1, 1, 1, 5]);
        t.extend([0u8; 1024]);
        let (active, gid_map) = all(8);
        let out = subset_gsub(&t, &active, &gid_map, 0).expect("a subset GSUB");
        assert_eq!(read_u16_be(&out, 8), Some(10), "the lookups come right after the header");
    }

    // A lookup filtering on mark set 1 keeps it when GDEF holds two sets. With one, it drops the set
    // and its flag, as OTS refuses a lookup naming a set GDEF lacks.
    #[test]
    fn a_mark_filtering_set_the_gdef_lacks_is_dropped() {
        let t = words(&[1, 0, 0, 0, 10, 1, 4, 1, 0x10, 1, 10, 1, 1, 6, 1, 1, 1, 5]);
        let (active, gid_map) = all(8);
        let lookup = |out: &[u8]| 10 + usize::from(read_u16_be(out, 12).expect("a lookup"));
        let kept = subset_gsub(&t, &active, &gid_map, 2).expect("a subset GSUB");
        assert_eq!(read_u16_be(&kept, lookup(&kept) + 2), Some(0x10));
        assert_eq!(read_u16_be(&kept, lookup(&kept) + 8), Some(1));
        let dropped = subset_gsub(&t, &active, &gid_map, 1).expect("a subset GSUB");
        assert_eq!(read_u16_be(&dropped, lookup(&dropped) + 2), Some(0));
    }

    // Coverage [5, 5, 7] with substitutes 10, 11, 12: once the repeat goes, 7 is the coverage's second
    // glyph and keeps its own substitute, 12, not the second, 11.
    #[test]
    fn a_repeated_glyph_keeps_one_substitute() {
        let t = words(&[1, 16, 0, 0, 3, 10, 11, 12, 1, 3, 5, 5, 7]);
        let (active, gid_map) = all(16);
        let out = subset_reverse_chain_single_subst(&t, 0, &active, &gid_map).expect("a subset subtable");
        let coverage = usize::from(read_u16_be(&out, 2).expect("a coverage offset"));
        assert_eq!(parse_coverage(&out, coverage), Ok(alloc::vec![5, 7]));
        assert_eq!([8, 10, 12].map(|at| read_u16_be(&out, at)), [Some(2), Some(10), Some(12)]);
    }
}
