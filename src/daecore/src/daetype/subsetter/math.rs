use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;
use super::super::decoder::{offset16_in, read_u16_be, read_i16_be, write_u16_be, resolve_offset16, Placed};
use super::super::math_table::{construction, kern_heights, Covered, CONSTANTS_LEN};
use super::otl::{parse_coverage, build_coverage, device_table};
use super::{remap_gid, remap_gid16};

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
        let mut devices = Placed::new(self.fixed.len());
        for &(slot, src) in &self.devices {
            let Some(dev) = device_table(math, src) else { continue };
            write_u16_be(&mut self.fixed, slot, devices.place(dev)?);
        }
        self.fixed.extend(devices.bytes);
        Some(self.fixed)
    }
}

// Glyphs may share a construction, which is read once. Constructions at overlapping offsets read the
// same records again, so the records read stop at the subset budget.
pub fn math_closure(math: &[u8], active: &GlyphSet) -> Vec<u32> {
    let mut found = GlyphSet::new();
    let Some(variants) = offset16_in(math, 0, 8) else { return Vec::new() };
    let mut read = alloc::collections::BTreeSet::new();
    let mut budget = super::subset_budget(math.len());

    'directions: for vertical in [true, false] {
        let Some(direction) = Covered::variants(math, variants, vertical) else { continue };
        let Ok(covered) = parse_coverage(math, direction.coverage) else { continue };
        for (i, &gid) in covered.iter().enumerate().take(direction.count(math)) {
            if !active.contains(&gid) { continue; }
            let Some(at) = resolve_offset16(math, direction.records + i * 2, variants) else { continue };
            if read.insert(at) && collect_construction_glyphs(math, at, &mut found, &mut budget).is_none() {
                break 'directions;
            }
        }
    }
    found.iter().collect()
}

fn collect_construction_glyphs(math: &[u8], at: usize, out: &mut GlyphSet, budget: &mut usize) -> Option<()> {
    let Some(found) = construction(math, at) else { return Some(()) };
    let parts = found.assembly.map(|a| (a.parts, a.part_count, 10));
    for (records, count, stride) in [(found.variants, found.variant_count, 4)].into_iter().chain(parts) {
        *budget = budget.checked_sub(count)?;
        for g in (0..count).filter_map(|i| read_u16_be(math, records + i * stride)) {
            out.insert(g.into());
        }
    }
    Some(())
}

fn subset_parallel_values(
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u32],
) -> Option<Vec<u8>> {
    let values = Covered::at(math, off, 4)?;
    let covered = parse_coverage(math, values.coverage).ok()?;
    let kept: Vec<(u32, usize)> = covered.iter().enumerate().take(values.count(math))
        .filter(|&(_, g)| active.contains(g))
        .filter_map(|(i, g)| remap_gid(active, gid_map, *g).map(|ng| (ng, i)))
        .collect();
    if kept.is_empty() { return None; }

    let mut sub = SubTable::new();
    let cov_slot = sub.slot();
    sub.u16(kept.len() as u16);
    for &(_, i) in &kept { sub.value(math, values.records + i * 4, off); }
    let mut out = SubTable { fixed: sub.finish(math)?, devices: Vec::new() };
    let gids: Vec<u32> = kept.iter().map(|&(g, _)| g).collect();
    out.place(cov_slot, &build_coverage(&gids))?;
    Some(out.fixed)
}

fn subset_math_kern(math: &[u8], off: usize) -> Option<Vec<u8>> {
    let heights = kern_heights(math, off)?;
    let mut sub = SubTable::new();
    sub.u16(heights as u16);
    for i in 0..2 * heights + 1 { sub.value(math, off + 2 + i * 4, off); }
    sub.finish(math)
}

fn subset_kern_info(
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u32],
) -> Option<Vec<u8>> {
    let records = Covered::at(math, off, 8)?;
    let covered = parse_coverage(math, records.coverage).ok()?;
    let kept: Vec<(u32, usize)> = covered.iter().enumerate().take(records.count(math))
        .filter(|&(_, g)| active.contains(g))
        .filter_map(|(i, g)| remap_gid(active, gid_map, *g).map(|ng| (ng, i)))
        .collect();
    if kept.is_empty() { return None; }

    let mut sub = SubTable::new();
    let cov_slot = sub.slot();
    sub.u16(kept.len() as u16);
    let mut corner_slots: Vec<(usize, Option<usize>)> = Vec::new();
    for &(_, i) in &kept {
        for c in 0..4 {
            corner_slots.push((sub.slot(), offset16_in(math, off, 4 + i * 8 + c * 2)));
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
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u32],
) -> Option<Vec<u8>> {
    let found = construction(math, off)?;
    let variants: Vec<(u16, u16)> = (0..found.variant_count)
        .filter_map(|i| {
            let rec = found.variants + i * 4;
            Some((remap_gid16(active, gid_map, read_u16_be(math, rec)?)?, read_u16_be(math, rec + 2).unwrap_or(0)))
        })
        .collect();

    let assembly = found.assembly.and_then(|a| {
        let mut mapped: Vec<(u16, [u16; 4])> = Vec::with_capacity(a.part_count);
        for i in 0..a.part_count {
            let rec = a.parts + i * 10;
            let ng = remap_gid16(active, gid_map, read_u16_be(math, rec)?)?;
            let mut rest = [0u16; 4];
            for (k, slot) in rest.iter_mut().enumerate() { *slot = read_u16_be(math, rec + 2 + k * 2)?; }
            mapped.push((ng, rest));
        }
        let mut sub = SubTable::new();
        sub.value(math, a.at, a.at);
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
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u32],
) -> Option<Vec<u8>> {
    // Glyphs sharing a construction share its rebuilt bytes too, placed once. Nothing past 16 bits can be placed, so
    // rebuilding stops there; overlapping constructions read records again, so those read stop at the subset budget.
    let mut budget = super::subset_budget(math.len());
    let mut built: [Vec<(u32, usize)>; 2] = [Vec::new(), Vec::new()];
    let mut constructions: Vec<Vec<u8>> = Vec::new();
    let mut total = 0usize;
    let mut index_of: BTreeMap<usize, Option<usize>> = BTreeMap::new();
    for (slot, vertical) in [(0usize, true), (1, false)] {
        let Some(direction) = Covered::variants(math, off, vertical) else { continue };
        let Ok(covered) = parse_coverage(math, direction.coverage) else { continue };
        for (i, &gid) in covered.iter().enumerate().take(direction.count(math)) {
            if !active.contains(&gid) { continue; }
            let at = resolve_offset16(math, direction.records + i * 2, off);
            let (Some(ng), Some(at)) = (remap_gid(active, gid_map, gid), at) else { continue };
            let index = match index_of.get(&at) {
                Some(&index) => index,
                None => {
                    let found = construction(math, at);
                    let records = found.map_or(0, |c| c.variant_count + c.assembly.map_or(0, |a| a.part_count));
                    budget = budget.checked_sub(records)?;
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
                Some(at) => { write_u16_be(&mut out.fixed, slot, at); }
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
    math: &[u8], off: usize, active: &GlyphSet, gid_map: &[u32],
) -> Option<Vec<u8>> {
    let italics = offset16_in(math, off, 0).and_then(|at| subset_parallel_values(math, at, active, gid_map));
    let top_accent = offset16_in(math, off, 2).and_then(|at| subset_parallel_values(math, at, active, gid_map));
    let extended = offset16_in(math, off, 4).and_then(|at| {
        let covered = parse_coverage(math, at).ok()?;
        let kept: Vec<u32> = covered.iter().filter_map(|&g| remap_gid(active, gid_map, g)).collect();
        (!kept.is_empty()).then(|| build_coverage(&kept))
    });
    let kern = offset16_in(math, off, 6).and_then(|at| subset_kern_info(math, at, active, gid_map));

    let mut sub = SubTable::new();
    let slots: Vec<usize> = (0..4).map(|_| sub.slot()).collect();
    let mut out = SubTable { fixed: sub.finish(math)?, devices: Vec::new() };
    for (slot, bytes) in slots.iter().zip([&italics, &top_accent, &extended, &kern]) {
        if let Some(b) = bytes { out.place(*slot, b)?; }
    }
    Some(out.fixed)
}

// Constants cut short are left out, not written as zeros.
fn subset_constants(math: &[u8], at: usize) -> Option<Vec<u8>> {
    math.get(at..at + CONSTANTS_LEN)?;
    let mut sub = SubTable::new();
    for k in 0..2 { sub.i16(read_i16_be(math, at + k * 2)?); }
    for k in 0..2 { sub.u16(read_u16_be(math, at + 4 + k * 2)?); }
    for k in 0..51 { sub.value(math, at + 8 + k * 4, at); }
    sub.u16(read_u16_be(math, at + CONSTANTS_LEN - 2)?);
    sub.finish(math)
}

const EMPTY_GLYPH_INFO: [u8; 8] = [0; 8];

// MathVariants with no construction: the source's minConnectorOverlap, no coverage, no glyphs.
fn empty_variants(math: &[u8], variants: Option<usize>) -> Vec<u8> {
    let overlap = variants.and_then(|at| read_u16_be(math, at)).unwrap_or(0);
    [overlap.to_be_bytes(), [0; 2], [0; 2], [0; 2], [0; 2]].concat()
}

// The constants are font-wide, so MATH stays whenever the source has them. Glyph info and variants
// no kept glyph has are written empty, as the header's offsets may not be NULL.
pub fn subset_math(math: &[u8], active: &GlyphSet, gid_map: &[u32]) -> Option<Vec<u8>> {
    let [constants, glyph_info, variants] = [4, 6, 8].map(|field| offset16_in(math, 0, field));
    let new_constants = constants.and_then(|at| subset_constants(math, at));
    let new_glyph_info = glyph_info
        .and_then(|at| subset_glyph_info(math, at, active, gid_map))
        .unwrap_or_else(|| EMPTY_GLYPH_INFO.to_vec());
    let empty = empty_variants(math, variants);
    let new_variants = variants
        .and_then(|at| subset_variants(math, at, active, gid_map))
        .unwrap_or_else(|| empty.clone());
    if new_constants.is_none() && new_glyph_info == EMPTY_GLYPH_INFO && new_variants == empty { return None; }

    let mut sub = SubTable::new();
    sub.u16(1);
    sub.u16(0);
    let slots: Vec<usize> = (0..3).map(|_| sub.slot()).collect();
    let mut out = SubTable { fixed: sub.finish(math)?, devices: Vec::new() };
    let tables = [new_constants.as_deref(), Some(&new_glyph_info[..]), Some(&new_variants[..])];
    for (slot, bytes) in slots.iter().zip(tables) {
        if let Some(b) = bytes { out.place(*slot, b)?; }
    }
    Some(out.fixed)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/math.rs"]
mod tests;
