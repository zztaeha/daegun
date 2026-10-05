use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be, write_u32_be};
use super::super::super::format::aat::Lookup;
use super::super::otl::remap_gid;
use super::lookup::{build_aat_lookup, build_aat_offset_lookup};

const CARET_HEADER: usize = 6;

// lcar and opbd: a lookup from glyph to a record, its value the record's offset from the table's
// start. A record many glyphs name, or two that match byte for byte, is written once.
fn subset_offset_lookup(
    table: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
    record_len: impl Fn(&[u8], usize) -> Option<usize>,
) -> Option<Vec<u8>> {
    let lookup = Lookup::parse(table.get(CARET_HEADER..)?, num_glyphs)?;
    let mut kept: Vec<(u16, &[u8])> = Vec::new();
    for (g, value) in lookup.entries() {
        let Some(ng) = remap_gid(active, gid_map, g) else { continue };
        let from = usize::from(value);
        kept.push((ng, table.get(from..from.checked_add(record_len(table, from)?)?)?));
    }
    if kept.is_empty() { return None; }

    let glyphs: Vec<(u16, u16)> = kept.iter().map(|&(g, _)| (g, 0)).collect();
    let records_at = CARET_HEADER + build_aat_offset_lookup(&glyphs)?.len();
    let mut records: Vec<u8> = Vec::new();
    let mut placed: BTreeMap<&[u8], u16> = BTreeMap::new();
    let mut entries: Vec<(u16, u16)> = Vec::with_capacity(kept.len());
    for (g, bytes) in kept {
        let at = match placed.get(bytes) {
            Some(&at) => at,
            None => {
                let at = u16::try_from(records_at + records.len()).ok()?;
                records.extend_from_slice(bytes);
                placed.insert(bytes, at);
                at
            }
        };
        entries.push((g, at));
    }

    let mut out = table.get(..CARET_HEADER)?.to_vec();
    out.extend_from_slice(&build_aat_offset_lookup(&entries)?);
    out.extend_from_slice(&records);
    Some(out)
}

pub fn subset_lcar(
    lcar: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    subset_offset_lookup(lcar, active, gid_map, num_glyphs, |d, at| Some(2 + read_u16_be(d, at)? as usize * 2))
}

pub fn subset_opbd(
    opbd: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    subset_offset_lookup(opbd, active, gid_map, num_glyphs, |_, _| Some(8))
}

pub fn subset_ankr(
    ankr: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let lookup_off = read_u32_be(ankr, 4)? as usize;
    let data_off = read_u32_be(ankr, 8)? as usize;

    let header_len = 12usize;
    let lookup = Lookup::parse(ankr.get(lookup_off..)?, num_glyphs)?;
    let kept: Vec<(u16, u16)> = lookup.entries().into_iter()
        .filter_map(|(g, v)| remap_gid(active, gid_map, g).map(|ng| (ng, v)))
        .collect();
    if kept.is_empty() { return None; }

    // Values count from the glyph data's own start, so the lookup's size cannot move them.
    let mut data: Vec<u8> = Vec::new();
    let mut placed: BTreeMap<&[u8], u16> = BTreeMap::new();
    let mut entries: Vec<(u16, u16)> = Vec::with_capacity(kept.len());
    for (g, value) in kept {
        let from = data_off.checked_add(value as usize)?;
        let len = (read_u32_be(ankr, from)? as usize).checked_mul(4)?.checked_add(4)?;
        let bytes = ankr.get(from..from.checked_add(len)?)?;
        let at = match placed.get(bytes) {
            Some(&at) => at,
            None => {
                let at = u16::try_from(data.len()).ok()?;
                data.extend_from_slice(bytes);
                placed.insert(bytes, at);
                at
            }
        };
        entries.push((g, at));
    }

    let lookup_bytes = build_aat_lookup(&entries)?;
    let mut out = ankr.get(..4)?.to_vec();
    out.resize(header_len, 0);
    write_u32_be(&mut out, 4, header_len as u32);
    write_u32_be(&mut out, 8, u32::try_from(header_len + lookup_bytes.len()).ok()?);
    out.extend_from_slice(&lookup_bytes);
    out.extend_from_slice(&data);
    Some(out)
}

pub fn subset_bsln(
    bsln: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let format = read_u16_be(bsln, 4)?;
    let (fixed_len, std_glyph_at) = match format {
        0 | 1 => (6 + 2 + 32 * 2, None),
        2 | 3 => (6 + 2 + 2 + 32 * 2, Some(8usize)),
        _ => return None,
    };

    let mut out = bsln.get(..fixed_len)?.to_vec();
    if let Some(at) = std_glyph_at {
        let g = read_u16_be(bsln, at)?;
        let new = remap_gid(active, gid_map, g)?;
        out.get_mut(at..at + 2)?.copy_from_slice(&new.to_be_bytes());
    }
    if format == 0 || format == 2 { return Some(out); }

    let lookup = Lookup::parse(bsln.get(fixed_len..)?, num_glyphs)?;
    let kept: Vec<(u16, u16)> = lookup.entries().into_iter()
        .filter_map(|(g, v)| remap_gid(active, gid_map, g).map(|ng| (ng, v)))
        .collect();
    if kept.is_empty() {
        out.get_mut(4..6)?.copy_from_slice(&(format - 1).to_be_bytes());
        return Some(out);
    }
    out.extend_from_slice(&build_aat_lookup(&kept)?);
    Some(out)
}

pub fn subset_fmtx(fmtx: &[u8], active: &GlyphSet, gid_map: &[u16]) -> Option<Vec<u8>> {
    let glyph = read_u32_be(fmtx, 4)?;
    let new = remap_gid(active, gid_map, u16::try_from(glyph).ok()?)?;
    let mut out = fmtx.to_vec();
    write_u32_be(&mut out, 4, new as u32);
    Some(out)
}

// The glyph bsln formats 2 and 3 and fmtx each take their measures from, which the closure keeps:
// without it the table is dropped from the subset.
pub fn metrics_glyphs<'a>(table: impl Fn(&str) -> Option<&'a [u8]>) -> Vec<u16> {
    let bsln = table("bsln").filter(|t| matches!(read_u16_be(t, 4), Some(2 | 3))).and_then(|t| read_u16_be(t, 8));
    let fmtx = table("fmtx").and_then(|t| read_u32_be(t, 4)).and_then(|g| u16::try_from(g).ok());
    bsln.into_iter().chain(fmtx).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daecore::daetype::decoder::read_i16_be;

    fn u16s(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    // Glyphs 0 to 8 kept as 0, 1 and 2 from 0, 5 and 7.
    fn kept() -> (GlyphSet, Vec<u16>) {
        let mut active = GlyphSet::new();
        [0, 5, 7].iter().for_each(|&g| { active.insert(g); });
        let mut gid_map = alloc::vec![0u16; 9];
        (gid_map[5], gid_map[7]) = (1, 2);
        (active, gid_map)
    }

    // A caret table: its header, a lookup from glyph to record offset, then the records.
    fn caret_table(records: &[(u16, Vec<u8>)]) -> Vec<u8> {
        let lookup_len = build_aat_offset_lookup(&records.iter().map(|&(g, _)| (g, 0)).collect::<Vec<_>>()).expect("a lookup").len();
        let mut at = CARET_HEADER + lookup_len;
        let mut entries = Vec::new();
        for (g, r) in records {
            entries.push((*g, at as u16));
            at += r.len();
        }
        let records: Vec<u8> = records.iter().flat_map(|(_, r)| r.clone()).collect();
        [u16s(&[1, 0, 0]), build_aat_offset_lookup(&entries).expect("a lookup"), records].concat()
    }

    fn record_of(table: &[u8], glyph: u16, len: usize) -> &[u8] {
        let at = usize::from(Lookup::parse(&table[CARET_HEADER..], u16::MAX).expect("a lookup").value(glyph).expect("an entry"));
        &table[at..at + len]
    }

    #[test]
    fn caret_and_bound_records_follow_their_glyphs() {
        let (active, gid_map) = kept();
        let caret = |v: u16| u16s(&[2, v, v + 1]);
        let lcar = caret_table(&[(5, caret(10)), (6, caret(20)), (7, caret(30))]);
        let out = subset_lcar(&lcar, &active, &gid_map, 9).expect("a subset lcar");
        assert_eq!((record_of(&out, 1, 6), record_of(&out, 2, 6)), (&caret(10)[..], &caret(30)[..]));

        let bounds = |v: u16| u16s(&[v, v, v, v]);
        let opbd = caret_table(&[(5, bounds(1)), (7, bounds(1)), (8, bounds(2))]);
        let out = subset_opbd(&opbd, &active, &gid_map, 9).expect("a subset opbd");
        assert_eq!(record_of(&out, 1, 8), &bounds(1)[..]);
        let shared = Lookup::parse(&out[CARET_HEADER..], 3).expect("a lookup");
        assert_eq!(shared.value(1), shared.value(2), "a record two glyphs share was written twice");
    }

    #[test]
    fn anchor_points_follow_their_glyphs() {
        let (active, gid_map) = kept();
        let points = |n: u32, x: i16| [n.to_be_bytes().to_vec(), (0..n).flat_map(|i| [(x + i as i16).to_be_bytes(), 0i16.to_be_bytes()].concat()).collect()].concat();
        let data = [points(1, 5), points(2, 7)].concat();
        let lookup = build_aat_lookup(&[(5, 0), (7, 8)]).expect("a lookup");
        let ankr = [u16s(&[0, 0, 0, 12]), (12 + lookup.len() as u32).to_be_bytes().to_vec(), lookup, data].concat();
        let out = subset_ankr(&ankr, &active, &gid_map, 9).expect("a subset ankr");
        let (table, data) = (read_u32_be(&out, 4).expect("lookup") as usize, read_u32_be(&out, 8).expect("data") as usize);
        let at = data + usize::from(Lookup::parse(&out[table..], 3).expect("a lookup").value(2).expect("glyph 7"));
        assert_eq!((read_u32_be(&out, at), read_i16_be(&out, at + 8)), (Some(2), Some(8)));
    }

    #[test]
    fn baselines_follow_their_glyphs() {
        let (active, gid_map) = kept();
        let deltas = alloc::vec![0u8; 64];
        let format1 = [u16s(&[1, 0, 1, 0]), deltas.clone(), build_aat_lookup(&[(5, 2), (8, 1)]).expect("a lookup")].concat();
        let out = subset_bsln(&format1, &active, &gid_map, 9).expect("a subset bsln");
        assert_eq!(Lookup::parse(&out[72..], 3).expect("a lookup").entries(), [(1, 2)]);

        let format2 = [u16s(&[1, 0, 2, 0, 7]), deltas].concat();
        let out = subset_bsln(&format2, &active, &gid_map, 9).expect("a subset bsln");
        assert_eq!(read_u16_be(&out, 8), Some(2), "the standard glyph was not renumbered");
    }

    #[test]
    fn the_font_metrics_glyph_follows() {
        let (active, gid_map) = kept();
        let fmtx = [0x0002_0000u32, 7, 0, 0].map(u32::to_be_bytes).concat();
        let out = subset_fmtx(&fmtx, &active, &gid_map).expect("a subset fmtx");
        assert_eq!(read_u32_be(&out, 4), Some(2));
    }

    // bsln's standard glyph and fmtx's glyph join the closure: without them both tables are dropped.
    #[test]
    fn the_glyphs_bsln_and_fmtx_measure_by_are_named_to_the_closure() {
        let bsln = [u16s(&[1, 0, 3, 0, 7]), alloc::vec![0; 64]].concat();
        let fmtx = [0x0002_0000u32, 4, 0, 0].map(u32::to_be_bytes).concat();
        let found = metrics_glyphs(|tag| match tag { "bsln" => Some(&bsln[..]), "fmtx" => Some(&fmtx[..]), _ => None });
        assert_eq!(found, [7, 4]);
        let format1 = [u16s(&[1, 0, 1, 0]), alloc::vec![0; 64]].concat();
        assert!(metrics_glyphs(|tag| (tag == "bsln").then_some(&format1[..])).is_empty());
    }
}
