use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use super::super::decoder::{read_offset24, read_u16_be, read_u32_be, write_u16_be};
use super::super::colr_v0::{colr_v0_header, colr_v0_base_glyphs};
use super::super::colr_v1::paint_layout;
use super::super::colr_v1::write::{assemble, rebuild};
use super::otl::remap_gid;
use super::subset_budget;

const MAX_BASE_GLYPHS: usize = 65536;

pub fn colr_closure(colr: &[u8], active: &GlyphSet) -> Vec<u16> {
    let mut found = Vec::new();
    colr_v0_closure(colr, active, &mut found);
    colr_v1_closure(colr, active, &mut found);
    found
}

// Base glyphs may share layers, so their ranges are merged and each layer is read once.
fn colr_v0_closure(colr: &[u8], active: &GlyphSet, found: &mut Vec<u16>) {
    let Some((_, _, layers_off, n_layer_records)) = colr_v0_header(colr) else { return };
    let mut ranges: Vec<(usize, usize)> = colr_v0_base_glyphs(colr)
        .into_iter()
        .filter(|(gid, ..)| active.contains(gid))
        .map(|(_, first, n)| (first, first.saturating_add(n).min(n_layer_records)))
        .filter(|(start, end)| start < end)
        .collect();
    ranges.sort_unstable();
    let mut next = 0;
    for (start, end) in ranges {
        for idx in start.max(next)..end {
            if let Some(layer_gid) = read_u16_be(colr, layers_off + idx * 4) {
                found.push(layer_gid);
            }
        }
        next = next.max(end);
    }
}

// Every paint the kept base glyphs reach, each read once however many glyphs share it, so the walk
// is linear in the table and needs no budget.
fn colr_v1_closure(colr: &[u8], active: &GlyphSet, found: &mut Vec<u16>) {
    if colr.len() < 34 || read_u16_be(colr, 0) != Some(1) { return; }
    let Some(list) = read_u32_be(colr, 14).filter(|&v| v != 0).map(|v| v as usize) else { return };
    let layer_list = read_u32_be(colr, 18).filter(|&v| v != 0).map(|v| v as usize);
    let Some(count) = read_u32_be(colr, list) else { return };

    let mut stack: Vec<usize> = (0..(count as usize).min(MAX_BASE_GLYPHS))
        .filter_map(|i| {
            let rec = list.checked_add(4 + i * 6)?;
            active.contains(&read_u16_be(colr, rec)?).then_some(())?;
            list.checked_add(read_u32_be(colr, rec + 2)? as usize)
        })
        .collect();
    let mut seen = alloc::vec![0u64; colr.len().div_ceil(64)];
    while let Some(off) = stack.pop() {
        let Some(word) = seen.get_mut(off >> 6) else { continue };
        if *word & (1 << (off & 63)) != 0 { continue; }
        *word |= 1 << (off & 63);
        let Some(layout) = colr.get(off).and_then(|&f| paint_layout(f)) else { continue };
        for &pos in layout.glyph_ids {
            found.extend(read_u16_be(colr, off + pos));
        }
        if colr[off] == 1 {
            let (Some(n), Some(first), Some(layers)) = (colr.get(off + 1), read_u32_be(colr, off + 2), layer_list) else { continue };
            let total = read_u32_be(colr, layers).unwrap_or(0) as usize;
            let end = (first as usize).saturating_add(usize::from(*n)).min(total);
            for idx in first as usize..end {
                stack.extend(read_u32_be(colr, layers + 4 + idx * 4).and_then(|rel| layers.checked_add(rel as usize)));
            }
        } else {
            for &pos in layout.children {
                stack.extend(read_offset24(colr, off + pos).and_then(|rel| off.checked_add(rel)));
            }
        }
    }
}

// The kept base glyphs under their new IDs, v1 through the instancer's writer. A subset has no fvar,
// so it is the default master, and the variation data stays behind with the axes.
pub fn subset_colr(colr: &[u8], active: &GlyphSet, gid_map: &[u16]) -> Option<Vec<u8>> {
    let has_v0 = colr_v0_header(colr).is_some_and(|(n, ..)| n > 0);
    let is_v1 = colr.len() >= 34 && read_u16_be(colr, 0) == Some(1);
    let v1_present = is_v1 && read_u32_be(colr, 14).unwrap_or(0) != 0;
    if !has_v0 && !v1_present { return None; }

    let v0 = if has_v0 { rebuild_v0(colr, active, gid_map) } else { None };
    let keep = |g: u16| remap_gid(active, gid_map, g);
    let remap = |g: u16| gid_map.get(usize::from(g)).copied().unwrap_or(g);
    let v1 = if v1_present { rebuild(colr, &keep, &remap, None, subset_budget(colr.len())) } else { None };
    if v0.is_none() && v1.is_none() { return None; }

    let v0 = v0.as_ref().map(|(base, n, layers, m)| (base.as_slice(), *n, layers.as_slice(), *m));
    assemble(if is_v1 { 1 } else { 0 }, v0, v1)
}

// Base records and layer records for the kept glyphs. A range several base glyphs name is written
// once, and a table past u16 counts is not written.
fn rebuild_v0(colr: &[u8], active: &GlyphSet, gid_map: &[u16]) -> Option<(Vec<u8>, u16, Vec<u8>, u16)> {
    let (_, _, orig_layers_off, orig_n_layers) = colr_v0_header(colr)?;
    let mut survivors: Vec<(u16, usize, usize)> = colr_v0_base_glyphs(colr).into_iter()
        .filter_map(|(gid, first, n)| remap_gid(active, gid_map, gid).map(|ng| (ng, first, n)))
        .collect();
    if survivors.is_empty() { return None; }
    survivors.sort_unstable_by_key(|&(gid, _, _)| gid);

    let mut layers: Vec<u8> = Vec::new();
    let mut base_records: Vec<u8> = Vec::with_capacity(survivors.len() * 6);
    let mut written: alloc::collections::BTreeMap<(usize, usize), (u16, u16)> = alloc::collections::BTreeMap::new();
    for &(gid, first, n) in &survivors {
        let record = match written.get(&(first, n)) {
            Some(&record) => record,
            None => {
                let new_first = layers.len() / 4;
                let mut actual_n = 0usize;
                for idx in first..first.saturating_add(n).min(orig_n_layers) {
                    let rec = orig_layers_off + idx * 4;
                    let (Some(orig_gid), Some(pal)) = (read_u16_be(colr, rec), read_u16_be(colr, rec + 2)) else { break };
                    let mut lrec = [0u8; 4];
                    write_u16_be(&mut lrec, 0, remap_gid(active, gid_map, orig_gid).unwrap_or(orig_gid));
                    write_u16_be(&mut lrec, 2, pal);
                    layers.extend_from_slice(&lrec);
                    actual_n += 1;
                }
                if layers.len() / 4 > usize::from(u16::MAX) {
                    return None;
                }
                let record = (new_first as u16, actual_n as u16);
                written.insert((first, n), record);
                record
            }
        };
        base_records.extend([gid, record.0, record.1].map(u16::to_be_bytes).concat());
    }
    let (n, m) = (survivors.len() as u16, (layers.len() / 4) as u16);
    Some((base_records, n, layers, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    // COLR v0 whose base glyphs 1 to `bases` all name the same 40,000 layers of glyph `bases + 1`.
    fn shared(bases: u16) -> (Vec<u8>, GlyphSet, Vec<u16>) {
        const LAYERS: u16 = 40_000;
        let records = 14 + 6 * usize::from(bases);
        let mut colr = [0u16, bases].map(u16::to_be_bytes).concat();
        colr.extend(14u32.to_be_bytes());
        colr.extend((records as u32).to_be_bytes());
        colr.extend(LAYERS.to_be_bytes());
        for gid in 1..=bases {
            colr.extend([gid, 0, LAYERS].map(u16::to_be_bytes).concat());
        }
        for _ in 0..LAYERS {
            colr.extend([bases + 1, 0].map(u16::to_be_bytes).concat());
        }
        let mut active = GlyphSet::new();
        (0..=bases + 1).for_each(|g| { active.insert(g); });
        (colr, active, (0..=bases + 1).collect())
    }

    #[test]
    fn shared_layers_are_written_once() {
        let (colr, active, gid_map) = shared(2);
        let out = subset_colr(&colr, &active, &gid_map).expect("a subset COLR");
        assert_eq!(read_u16_be(&out, 12), Some(40_000), "the layer count");
        let base = read_u32_be(&out, 4).expect("base records") as usize;
        for i in 0..2 {
            let (first, n) = (read_u16_be(&out, base + 6 * i + 2), read_u16_be(&out, base + 6 * i + 4));
            assert_eq!((first, n), (Some(0), Some(40_000)), "base glyph {i} lost its layers");
        }
    }

    #[test]
    fn layers_past_u16_drop_v0_rather_than_wrap() {
        let (mut colr, active, gid_map) = shared(2);
        colr[22..26].copy_from_slice(&[0, 1, 0x9C, 0x3F]);
        let out = subset_colr(&colr, &active, &gid_map);
        assert_eq!(out.map(|t| read_u16_be(&t, 12)), None, "79,999 layers were written as a u16 count");
    }

    // 2,000 glyphs sharing 60 layers walk 242,000 paints between them, past one budget of 100,000 for
    // the whole subset: each paint is read once, and written once.
    #[test]
    fn a_shared_graph_subsets_whole() {
        let colr = super::super::super::colr_v1::testing::shared_layers(2000, 60, false);
        let mut active = GlyphSet::new();
        (0..=2000u16).for_each(|g| { active.insert(g); });
        let found = colr_closure(&colr, &active);
        assert!((1000..1060).all(|g| found.contains(&g)), "the closure lost layer glyphs");
        (1000..1060u16).for_each(|g| { active.insert(g); });
        let gid_map: Vec<u16> = (0..=2000).collect();
        let out = subset_colr(&colr, &active, &gid_map).expect("a subset COLR");
        let list = read_u32_be(&out, 14).expect("a BaseGlyphList") as usize;
        assert_eq!(read_u32_be(&out, list), Some(2000));
        assert!(out.len() < colr.len() + 2000 * 6, "the shared layers were written more than once");
    }

    #[test]
    fn the_v0_closure_reads_each_layer_once() {
        let (colr, active, _) = shared(200);
        let found = colr_closure(&colr, &active);
        assert!(found.len() <= 40_000, "200 glyphs sharing 40,000 layers found {}", found.len());
    }
}
