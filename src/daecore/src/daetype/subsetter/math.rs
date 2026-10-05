use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{read_u16_be, read_i16_be, write_u16_be, records_fit};
use super::otl::{parse_coverage, build_coverage, device_table, remap_gid};

struct SubTable {
    fixed: Vec<u8>,
    devices: Vec<(usize, usize)>,
}

impl SubTable {
    fn new() -> Self { SubTable { fixed: Vec::new(), devices: Vec::new() } }
    fn u16(&mut self, v: u16) { self.fixed.extend_from_slice(&v.to_be_bytes()); }
    fn i16(&mut self, v: i16) { self.fixed.extend_from_slice(&v.to_be_bytes()); }
    fn slot(&mut self) -> usize { let at = self.fixed.len(); self.u16(0); at }

    // A device is noted by where it sits in the source, and copied once in `finish`.
    fn value(&mut self, math: &[u8], src: usize, src_parent: usize) {
        self.i16(read_i16_be(math, src).unwrap_or(0));
        let slot = self.slot();
        if let Some(rel) = read_u16_be(math, src + 2).filter(|&r| r != 0) {
            self.devices.push((slot, src_parent + rel as usize));
        }
    }

    // The offset written, or None, adding nothing, where the bytes would start past 16 bits: the
    // subtable holding the slot is then refused, not left naming nothing.
    fn place(&mut self, slot: usize, bytes: &[u8]) -> Option<u16> {
        let at = u16::try_from(self.fixed.len()).ok()?;
        write_u16_be(&mut self.fixed, slot, at);
        self.fixed.extend_from_slice(bytes);
        Some(at)
    }

    // The devices after the fixed part, each distinct one once; None if one would start past 16 bits.
    fn finish(mut self, math: &[u8]) -> Option<Vec<u8>> {
        let mut placed: BTreeMap<&[u8], u16> = BTreeMap::new();
        for (slot, src) in core::mem::take(&mut self.devices) {
            let Some(dev) = device_table(math, src) else { continue };
            let at = match placed.get(dev) {
                Some(&at) => at,
                None => {
                    let at = self.place(slot, dev)?;
                    placed.insert(dev, at);
                    at
                }
            };
            write_u16_be(&mut self.fixed, slot, at);
        }
        Some(self.fixed)
    }
}

fn sections(math: &[u8]) -> Option<(usize, usize, usize)> {
    Some((
        read_u16_be(math, 4)? as usize,
        read_u16_be(math, 6)? as usize,
        read_u16_be(math, 8)? as usize,
    ))
}

const MATH_CONSTANTS_LEN: usize = 8 + 51 * 4 + 2;

fn variants_direction(math: &[u8], variants: usize, vertical: bool) -> Option<(usize, usize, usize)> {
    let cov_rel = read_u16_be(math, variants + if vertical { 2 } else { 4 })? as usize;
    let count = read_u16_be(math, variants + if vertical { 6 } else { 8 })? as usize;
    let vert_count = read_u16_be(math, variants + 6)? as usize;
    let array = variants + 10 + if vertical { 0 } else { vert_count * 2 };
    Some((variants + cov_rel, count, array))
}

// Glyphs may share a construction, which is read once. Constructions at overlapping offsets read the
// same records again, so the records read stop at the subset budget.
pub fn math_closure(math: &[u8], active: &GlyphSet) -> Vec<u16> {
    let mut found = GlyphSet::new();
    let Some((_, _, variants)) = sections(math) else { return Vec::new() };
    if variants == 0 { return Vec::new(); }
    let mut read = alloc::collections::BTreeSet::new();
    let mut budget = super::subset_budget(math.len());

    'directions: for vertical in [true, false] {
        let Some((cov_off, count, array)) = variants_direction(math, variants, vertical) else { continue };
        let Ok(covered) = parse_coverage(math, cov_off) else { continue };
        if !records_fit(array, count, 2, math.len()) { continue; }

        for (i, &gid) in covered.iter().enumerate() {
            if i >= count || !active.contains(&gid) { continue; }
            let Some(rel) = read_u16_be(math, array + i * 2) else { continue };
            let construction = variants + rel as usize;
            if read.insert(construction) && collect_construction_glyphs(math, construction, &mut found, &mut budget).is_none() {
                break 'directions;
            }
        }
    }
    found.iter().collect()
}

fn collect_construction_glyphs(math: &[u8], construction: usize, out: &mut GlyphSet, budget: &mut usize) -> Option<()> {
    if let Some(count) = read_u16_be(math, construction + 2)
        && records_fit(construction + 4, count as usize, 4, math.len()) {
            *budget = budget.checked_sub(count as usize)?;
            for i in 0..count as usize {
                if let Some(g) = read_u16_be(math, construction + 4 + i * 4) { out.insert(g); }
            }
        }
    let Some(rel) = read_u16_be(math, construction).filter(|&r| r != 0) else { return Some(()) };
    let assembly = construction + rel as usize;
    let Some(parts) = read_u16_be(math, assembly + 4) else { return Some(()) };
    if !records_fit(assembly + 6, parts as usize, 10, math.len()) { return Some(()) }
    *budget = budget.checked_sub(parts as usize)?;
    for i in 0..parts as usize {
        if let Some(g) = read_u16_be(math, assembly + 6 + i * 10) { out.insert(g); }
    }
    Some(())
}

fn subset_parallel_values(
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u16],
) -> Option<Vec<u8>> {
    let cov_off = off + read_u16_be(math, off)? as usize;
    let count = read_u16_be(math, off + 2)? as usize;
    let covered = parse_coverage(math, cov_off).ok()?;
    if !records_fit(off + 4, count, 4, math.len()) { return None; }

    let kept: Vec<(u16, usize)> = covered.iter().enumerate()
        .filter(|&(i, g)| i < count && active.contains(g))
        .filter_map(|(i, g)| remap_gid(active, gid_map, *g).map(|ng| (ng, i)))
        .collect();
    if kept.is_empty() { return None; }

    let mut sub = SubTable::new();
    let cov_slot = sub.slot();
    sub.u16(kept.len() as u16);
    for &(_, i) in &kept { sub.value(math, off + 4 + i * 4, off); }
    let mut out = SubTable { fixed: sub.finish(math)?, devices: Vec::new() };
    let gids: Vec<u16> = kept.iter().map(|&(g, _)| g).collect();
    out.place(cov_slot, &build_coverage(&gids))?;
    Some(out.fixed)
}

fn subset_math_kern(math: &[u8], off: usize) -> Option<Vec<u8>> {
    let heights = read_u16_be(math, off)? as usize;
    let records = heights.checked_mul(2)?.checked_add(1)?;
    if !records_fit(off + 2, records, 4, math.len()) { return None; }
    let mut sub = SubTable::new();
    sub.u16(heights as u16);
    for i in 0..records { sub.value(math, off + 2 + i * 4, off); }
    sub.finish(math)
}

fn subset_kern_info(
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u16],
) -> Option<Vec<u8>> {
    let cov_off = off + read_u16_be(math, off)? as usize;
    let count = read_u16_be(math, off + 2)? as usize;
    let covered = parse_coverage(math, cov_off).ok()?;
    if !records_fit(off + 4, count, 8, math.len()) { return None; }

    let kept: Vec<(u16, usize)> = covered.iter().enumerate()
        .filter(|&(i, g)| i < count && active.contains(g))
        .filter_map(|(i, g)| remap_gid(active, gid_map, *g).map(|ng| (ng, i)))
        .collect();
    if kept.is_empty() { return None; }

    let mut sub = SubTable::new();
    let cov_slot = sub.slot();
    sub.u16(kept.len() as u16);
    let mut corner_slots: Vec<(usize, Option<usize>)> = Vec::new();
    for &(_, i) in &kept {
        for c in 0..4 {
            let src = read_u16_be(math, off + 4 + i * 8 + c * 2).filter(|&r| r != 0);
            corner_slots.push((sub.slot(), src.map(|r| off + r as usize)));
        }
    }
    // A MathKern many corners name is rebuilt and placed once, every slot naming the one copy.
    let mut out = SubTable { fixed: sub.finish(math)?, devices: Vec::new() };
    let mut placed: BTreeMap<usize, Option<u16>> = BTreeMap::new();
    for (slot, src) in corner_slots {
        let Some(src) = src else { continue };
        let at = match placed.get(&src) {
            Some(&at) => at,
            None => {
                let at = match subset_math_kern(math, src) {
                    Some(kern) => Some(out.place(slot, &kern)?),
                    None => None,
                };
                placed.insert(src, at);
                at
            }
        };
        if let Some(at) = at { write_u16_be(&mut out.fixed, slot, at); }
    }
    out.place(cov_slot, &build_coverage(&kept.iter().map(|&(g, _)| g).collect::<Vec<_>>()))?;
    Some(out.fixed)
}

fn subset_construction(
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u16],
) -> Option<Vec<u8>> {
    let variant_count = read_u16_be(math, off + 2)? as usize;
    if !records_fit(off + 4, variant_count, 4, math.len()) { return None; }

    let mut variants: Vec<(u16, u16)> = Vec::new();
    for i in 0..variant_count {
        let rec = off + 4 + i * 4;
        let Some(g) = read_u16_be(math, rec) else { continue };
        let Some(ng) = remap_gid(active, gid_map, g) else { continue };
        variants.push((ng, read_u16_be(math, rec + 2).unwrap_or(0)));
    }

    let assembly = read_u16_be(math, off).filter(|&r| r != 0).and_then(|rel| {
        let at = off + rel as usize;
        let parts = read_u16_be(math, at + 4)? as usize;
        if !records_fit(at + 6, parts, 10, math.len()) { return None; }
        let mut mapped: Vec<(u16, [u16; 4])> = Vec::with_capacity(parts);
        for i in 0..parts {
            let rec = at + 6 + i * 10;
            let ng = remap_gid(active, gid_map, read_u16_be(math, rec)?)?;
            let mut rest = [0u16; 4];
            for (k, slot) in rest.iter_mut().enumerate() { *slot = read_u16_be(math, rec + 2 + k * 2)?; }
            mapped.push((ng, rest));
        }
        let mut sub = SubTable::new();
        sub.value(math, at, at);
        sub.u16(mapped.len() as u16);
        for (g, rest) in &mapped {
            sub.u16(*g);
            for v in rest { sub.u16(*v); }
        }
        sub.finish(math)
    });

    if variants.is_empty() && assembly.is_none() { return None; }

    let mut sub = SubTable::new();
    let assembly_slot = sub.slot();
    sub.u16(variants.len() as u16);
    for (g, adv) in &variants { sub.u16(*g); sub.u16(*adv); }
    let mut out = SubTable { fixed: sub.finish(math)?, devices: Vec::new() };
    if let Some(a) = assembly { out.place(assembly_slot, &a)?; }
    Some(out.fixed)
}

fn subset_variants(
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u16],
) -> Option<Vec<u8>> {
    // Glyphs sharing a construction share its rebuilt bytes too, placed once. Nothing starting past
    // 16 bits can be placed, so rebuilding stops there.
    let mut built: [Vec<(u16, usize)>; 2] = [Vec::new(), Vec::new()];
    let mut constructions: Vec<Vec<u8>> = Vec::new();
    let mut total = 0usize;
    let mut index_of: BTreeMap<usize, Option<usize>> = BTreeMap::new();
    for (slot, vertical) in [(0usize, true), (1, false)] {
        let Some((cov_off, count, array)) = variants_direction(math, off, vertical) else { continue };
        let Ok(covered) = parse_coverage(math, cov_off) else { continue };
        if !records_fit(array, count, 2, math.len()) { continue; }
        for (i, &gid) in covered.iter().enumerate() {
            if i >= count || !active.contains(&gid) { continue; }
            let (Some(ng), Some(rel)) = (remap_gid(active, gid_map, gid), read_u16_be(math, array + i * 2))
            else { continue };
            let at = off + rel as usize;
            let index = match index_of.get(&at) {
                Some(&index) => index,
                None => {
                    let index = subset_construction(math, at, active, gid_map).map(|c| {
                        total += c.len();
                        constructions.push(c);
                        constructions.len() - 1
                    });
                    if total > usize::from(u16::MAX) { return None; }
                    index_of.insert(at, index);
                    index
                }
            };
            if let Some(index) = index {
                built[slot].push((ng, index));
            }
        }
    }

    let mut sub = SubTable::new();
    sub.u16(read_u16_be(math, off).unwrap_or(0));
    let vert_cov = sub.slot();
    let horiz_cov = sub.slot();
    sub.u16(built[0].len() as u16);
    sub.u16(built[1].len() as u16);
    let mut construction_slots: Vec<usize> = Vec::new();
    for dir in &built {
        for _ in dir { construction_slots.push(sub.slot()); }
    }

    let mut out = SubTable { fixed: sub.finish(math)?, devices: Vec::new() };
    let mut slots = construction_slots.into_iter();
    let mut placed: Vec<Option<u16>> = alloc::vec![None; constructions.len()];
    for dir in &built {
        for &(_, index) in dir {
            let Some(slot) = slots.next() else { break };
            match placed[index] {
                Some(at) => write_u16_be(&mut out.fixed, slot, at),
                None => placed[index] = Some(out.place(slot, &constructions[index])?),
            }
        }
    }
    for (slot, dir) in [(vert_cov, &built[0]), (horiz_cov, &built[1])] {
        if dir.is_empty() { continue; }
        out.place(slot, &build_coverage(&dir.iter().map(|(g, _)| *g).collect::<Vec<_>>()))?;
    }
    Some(out.fixed)
}

fn subset_glyph_info(
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u16],
) -> Option<Vec<u8>> {
    let italics = read_u16_be(math, off).filter(|&r| r != 0)
        .and_then(|r| subset_parallel_values(math, off + r as usize, active, gid_map));
    let top_accent = read_u16_be(math, off + 2).filter(|&r| r != 0)
        .and_then(|r| subset_parallel_values(math, off + r as usize, active, gid_map));
    let extended = read_u16_be(math, off + 4).filter(|&r| r != 0).and_then(|r| {
        let covered = parse_coverage(math, off + r as usize).ok()?;
        let kept: Vec<u16> = covered.iter().filter_map(|&g| remap_gid(active, gid_map, g)).collect();
        (!kept.is_empty()).then(|| build_coverage(&kept))
    });
    let kern = read_u16_be(math, off + 6).filter(|&r| r != 0)
        .and_then(|r| subset_kern_info(math, off + r as usize, active, gid_map));

    let mut sub = SubTable::new();
    let slots: Vec<usize> = (0..4).map(|_| sub.slot()).collect();
    let mut out = SubTable { fixed: sub.finish(math)?, devices: Vec::new() };
    for (slot, bytes) in slots.iter().zip([&italics, &top_accent, &extended, &kern]) {
        if let Some(b) = bytes { out.place(*slot, b)?; }
    }
    Some(out.fixed)
}

// Constants cut short are left out, not written as zeros: the last field is read with `?`.
fn subset_constants(math: &[u8], at: usize) -> Option<Vec<u8>> {
    let mut sub = SubTable::new();
    for k in 0..2 { sub.i16(read_i16_be(math, at + k * 2)?); }
    for k in 0..2 { sub.u16(read_u16_be(math, at + 4 + k * 2)?); }
    for k in 0..51 { sub.value(math, at + 8 + k * 4, at); }
    sub.u16(read_u16_be(math, at + MATH_CONSTANTS_LEN - 2)?);
    sub.finish(math)
}

const EMPTY_GLYPH_INFO: [u8; 8] = [0; 8];

// MathVariants with no construction: the source's minConnectorOverlap, no coverage, no glyphs.
fn empty_variants(math: &[u8], variants: usize) -> Vec<u8> {
    let overlap = if variants == 0 { 0 } else { read_u16_be(math, variants).unwrap_or(0) };
    [overlap.to_be_bytes(), [0; 2], [0; 2], [0; 2], [0; 2]].concat()
}

// The constants are font-wide, so MATH stays whenever the source has them. Glyph info and variants
// no kept glyph has are written empty, as the header's offsets may not be NULL.
pub fn subset_math(math: &[u8], active: &GlyphSet, gid_map: &[u16]) -> Option<Vec<u8>> {
    let (constants, glyph_info, variants) = sections(math)?;
    let new_constants = (constants != 0).then(|| subset_constants(math, constants)).flatten();
    let new_glyph_info = (glyph_info != 0)
        .then(|| subset_glyph_info(math, glyph_info, active, gid_map)).flatten()
        .unwrap_or_else(|| EMPTY_GLYPH_INFO.to_vec());
    let empty = empty_variants(math, variants);
    let new_variants = (variants != 0)
        .then(|| subset_variants(math, variants, active, gid_map)).flatten()
        .unwrap_or_else(|| empty.clone());
    if new_constants.is_none() && new_glyph_info == EMPTY_GLYPH_INFO && new_variants == empty { return None; }

    let mut sub = SubTable::new();
    sub.u16(1);
    sub.u16(0);
    let slots: Vec<usize> = (0..3).map(|_| sub.slot()).collect();
    let mut out = SubTable { fixed: sub.finish(math)?, devices: Vec::new() };
    for (slot, bytes) in slots.iter().zip([new_constants.as_deref(), Some(&new_glyph_info[..]), Some(&new_variants[..])]) {
        if let Some(b) = bytes { out.place(*slot, b)?; }
    }
    Some(out.fixed)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2,000 vertical glyphs sharing one construction of 100 variants.
    fn math() -> Vec<u8> {
        const N: u16 = 2_000;
        const V: u16 = 100;
        let construction = 20 + 2 * N;
        let coverage = construction + 4 + 4 * V;
        let mut t = [1u16, 0, 0, 0, 10].map(u16::to_be_bytes).concat();
        t.extend([0u16, coverage - 10, 0, N, 0].map(u16::to_be_bytes).concat());
        (0..N).for_each(|_| t.extend((construction - 10).to_be_bytes()));
        t.extend([0u16, V].map(u16::to_be_bytes).concat());
        (0..V).for_each(|v| t.extend([N + 1 + v, 100].map(u16::to_be_bytes).concat()));
        t.extend([1u16, N].map(u16::to_be_bytes).concat());
        t.extend((1..=N).flat_map(u16::to_be_bytes));
        t
    }

    fn everything() -> (GlyphSet, Vec<u16>) {
        let mut active = GlyphSet::new();
        (0..2_200).for_each(|g| { active.insert(g); });
        (active, (0..2_200).collect())
    }

    // A run of (0, 100) records reads as a construction of 100 variants at every 4-byte step, so 200
    // glyphs naming offsets 4 bytes apart rebuild 80 KB from 2 KB: past what 16-bit offsets reach.
    #[test]
    fn variants_past_16_bits_are_refused_not_left_without_coverage() {
        const N: usize = 200;
        const V: usize = 100;
        let region = 10 + 2 * N;
        let coverage = region + 4 * (N + V);
        let mut t = [1u16, 0, 0, 0, 10].map(u16::to_be_bytes).concat();
        t.extend([0, coverage, 0, N, 0].map(|v| (v as u16).to_be_bytes()).concat());
        (0..N).for_each(|i| t.extend(((region + 4 * i) as u16).to_be_bytes()));
        (0..N + V).for_each(|_| t.extend([0u16, 100].map(u16::to_be_bytes).concat()));
        t.extend([1u16, N as u16].map(u16::to_be_bytes).concat());
        t.extend((1..=N as u16).flat_map(u16::to_be_bytes));
        let (active, gid_map) = everything();

        if let Some(out) = subset_math(&t, &active, &gid_map) {
            let variants = usize::from(read_u16_be(&out, 8).expect("a variants offset"));
            let declared = read_u16_be(&out, variants + 6).unwrap_or(0);
            let covered = read_u16_be(&out, variants + 2).unwrap_or(0);
            assert!(declared == 0 || covered != 0, "{declared} glyphs declared with no coverage");
        }
    }

    #[test]
    fn a_shared_construction_is_closed_over_once() {
        let (active, _) = everything();
        let found = math_closure(&math(), &active);
        assert!(found.len() <= 100, "100 variants shared by 2,000 glyphs found {}", found.len());
    }

    #[test]
    fn a_shared_construction_is_written_once() {
        let t = math();
        let (active, gid_map) = everything();
        let out = subset_math(&t, &active, &gid_map).expect("a subset MATH");
        assert!(out.len() <= t.len(), "a {}-byte MATH subset to {} bytes", t.len(), out.len());
    }

    // math() with constants appended: each i16 and MathValueRecord value its own index.
    fn with_constants(mut t: Vec<u8>) -> (Vec<u8>, usize) {
        let at = t.len();
        write_u16_be(&mut t, 4, at as u16);
        (0..4u16).for_each(|k| t.extend(k.to_be_bytes()));
        (4..55u16).for_each(|k| t.extend([k, 0].map(u16::to_be_bytes).concat()));
        t.extend(55u16.to_be_bytes());
        (t, at)
    }

    #[test]
    fn a_subset_without_glyph_info_or_variants_keeps_the_constants() {
        let (t, at) = with_constants(math());
        let mut active = GlyphSet::new();
        active.insert(0);
        let out = subset_math(&t, &active, &[0]).expect("MATH is kept for its constants");
        let constants = usize::from(read_u16_be(&out, 4).expect("a constants offset"));
        assert_eq!(out.get(constants..constants + MATH_CONSTANTS_LEN), t.get(at..at + MATH_CONSTANTS_LEN));
    }

    #[test]
    fn constants_cut_short_are_left_out_not_zeroed() {
        let (mut t, _) = with_constants(math());
        t.truncate(t.len() - 2);
        let (active, gid_map) = everything();
        let out = subset_math(&t, &active, &gid_map).expect("a subset MATH");
        assert_eq!(read_u16_be(&out, 4), Some(0));
    }

    // 20 glyphs naming constructions 4 bytes apart in a run of (0, 65,535) records, then one naming
    // a construction of glyph 7: 1.3M records, past the 590K budget, so glyph 7 is never reached.
    #[test]
    fn the_closure_stops_reading_constructions_at_the_budget() {
        const N: u16 = 20;
        let coverage = 10 + 2 * (N + 1);
        let single = coverage + 4 + 2 * (N + 1);
        let region = single + 8;
        let mut t = [1u16, 0, 0, 0, 10, 0, coverage, 0, N + 1, 0].map(u16::to_be_bytes).concat();
        (0..N).for_each(|i| t.extend((region + 4 * i).to_be_bytes()));
        t.extend(single.to_be_bytes());
        t.extend([1u16, N + 1].map(u16::to_be_bytes).concat());
        t.extend((1..=N + 1).flat_map(u16::to_be_bytes));
        t.extend([0u16, 1, 7, 0].map(u16::to_be_bytes).concat());
        (0..u32::from(N) + 65_535).for_each(|_| t.extend([0u16, 0xFFFF].map(u16::to_be_bytes).concat()));
        let (active, _) = everything();
        assert!(!math_closure(&t, &active).contains(&7));
    }

    // 2,000 glyphs whose four corners all name one MathKern of 38 bytes: rebuilt per corner, that
    // is 304 KB, past what MathKernInfo's 16-bit offsets reach.
    #[test]
    fn a_math_kern_shared_by_many_corners_is_written_once() {
        const N: u16 = 2_000;
        let kern_info = 18u16;
        let kern = 4 + 8 * N;
        let coverage = kern + 38;
        let mut t = [1u16, 0, 0, 10, 0, 0, 0, 0, kern_info - 10, coverage, N].map(u16::to_be_bytes).concat();
        (0..4 * N).for_each(|_| t.extend(kern.to_be_bytes()));
        t.extend(4u16.to_be_bytes());
        (0..9u16).for_each(|k| t.extend([k, 0].map(u16::to_be_bytes).concat()));
        t.extend([1u16, N].map(u16::to_be_bytes).concat());
        t.extend((1..=N).flat_map(u16::to_be_bytes));
        let (active, gid_map) = everything();
        let out = subset_math(&t, &active, &gid_map).expect("a subset MATH");
        let glyph_info = usize::from(read_u16_be(&out, 6).expect("a glyph info offset"));
        assert_ne!(read_u16_be(&out, glyph_info + 6), Some(0), "MathKernInfo was refused");
        assert!(out.len() <= t.len(), "a {}-byte MATH subset to {} bytes", t.len(), out.len());
    }
}
