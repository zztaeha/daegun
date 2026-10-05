use alloc::string::String;
use alloc::vec::Vec;
use alloc::collections::BTreeMap;

use super::super::decoder::{read_u16_be, read_u32_be, write_i16_be, write_u16_be};
use super::super::format::ivs::{
    compute_ivs_delta_f64, parse_item_variation_store, precompute_region_scalars, ItemVariationStore,
};
use super::super::format::round::ot_round;
use crate::daecore::daetype::TableBytes;

mod kind {
    pub(super) const SINGLE: u16 = 1;
    pub(super) const PAIR: u16 = 2;
    pub(super) const CURSIVE: u16 = 3;
    pub(super) const MARK_BASE: u16 = 4;
    pub(super) const MARK_LIG: u16 = 5;
    pub(super) const MARK_MARK: u16 = 6;
    pub(super) const EXTENSION: u16 = 9;
}

const VALUE_FIELDS: [u16; 4] = [0x0001, 0x0002, 0x0004, 0x0008];
const DEVICE_FIELDS: [u16; 4] = [0x0010, 0x0020, 0x0040, 0x0080];

const DELTA_FORMAT_VARIATION_INDEX: u16 = 0x8000;

fn value_record_len(format: u16) -> usize {
    (format & 0x00FF).count_ones() as usize * 2
}

struct Ctx {
    store: ItemVariationStore,
    scalars: Vec<f64>,
    patched: Vec<u64>,
    // Value records and anchors visited, each naming of a shared one counted, so the work is linear
    // in the table's size whatever counts it states.
    visits_left: usize,
    // A record names a VariationIndex with a delta for a value it does not hold, so the table is
    // written out afresh with that value added, which patching in place cannot do.
    widen: bool,
}

impl Ctx {
    fn delta(&self, outer: u16, inner: u16) -> i32 {
        ot_round(compute_ivs_delta_f64(&self.store, outer as usize, inner as usize, &self.scalars))
    }

    fn visit(&mut self) -> bool {
        match self.visits_left.checked_sub(1) {
            Some(left) => { self.visits_left = left; true }
            None => false,
        }
    }
}

pub(crate) fn apply_gpos_var(
    table_map: &BTreeMap<String, TableBytes>,
    location: &[f64],
) -> Option<Vec<u8>> {
    let gpos = table_map.get("GPOS")?;
    let gdef = table_map.get("GDEF")?;

    if read_u16_be(gdef, 0)? != 1 || read_u16_be(gdef, 2)? < 3 {
        return None;
    }
    let ivs_off = read_u32_be(gdef, 14)? as usize;
    if ivs_off == 0 {
        return None;
    }
    let store = parse_item_variation_store(gdef, ivs_off).ok()?;
    let scalars = precompute_region_scalars(&store, location);

    let mut out = gpos.to_vec();
    let visits_left = out.len().saturating_mul(4);
    let mut ctx = Ctx { store, scalars, patched: alloc::vec![0u64; out.len().div_ceil(64)], visits_left, widen: false };

    let lookup_list = read_u16_be(&out, 8)? as usize;
    let lookup_count = read_u16_be(&out, lookup_list)?;
    for i in 0..lookup_count {
        let rel = read_u16_be(&out, lookup_list + 2 + i as usize * 2)? as usize;
        let lookup = lookup_list + rel;
        let Some(lookup_kind) = read_u16_be(&out, lookup) else { continue };
        let Some(subtable_count) = read_u16_be(&out, lookup + 4) else { continue };

        for j in 0..subtable_count {
            let Some(sub_rel) = read_u16_be(&out, lookup + 6 + j as usize * 2) else { break };
            if !ctx.visit() {
                return None;
            }
            if let Some((real_kind, at)) = resolve_extension(&out, lookup_kind, lookup + sub_rel as usize) {
                patch_subtable(&mut out, &mut ctx, real_kind, at);
            }
            if ctx.visits_left == 0 {
                return None;
            }
        }
    }

    if ctx.widen {
        let n_glyphs = read_u16_be(table_map.get("maxp")?, 4)?;
        let vary = |outer, inner| ctx.delta(outer, inner);
        let mark_sets = crate::daecore::daetype::subsetter::mark_glyph_set_count(gdef);
        return crate::daecore::daetype::subsetter::instance_gpos(gpos, n_glyphs, mark_sets, &vary);
    }
    Some(out)
}

fn resolve_extension(buf: &[u8], lookup_kind: u16, at: usize) -> Option<(u16, usize)> {
    if lookup_kind != kind::EXTENSION {
        return Some((lookup_kind, at));
    }
    if read_u16_be(buf, at)? != 1 {
        return None;
    }
    let real = read_u16_be(buf, at + 2)?;
    if real == kind::EXTENSION {
        return None;
    }
    let off = read_u32_be(buf, at + 4)? as usize;
    Some((real, at + off))
}

fn patch_subtable(buf: &mut [u8], ctx: &mut Ctx, real_kind: u16, at: usize) {
    match real_kind {
        kind::SINGLE => patch_single(buf, ctx, at),
        kind::PAIR => patch_pair(buf, ctx, at),
        kind::CURSIVE => patch_cursive(buf, ctx, at),
        kind::MARK_BASE | kind::MARK_MARK => patch_mark_attach(buf, ctx, at),
        kind::MARK_LIG => patch_mark_lig(buf, ctx, at),
        _ => {}
    }
}

fn patch_single(buf: &mut [u8], ctx: &mut Ctx, at: usize) {
    let Some(format) = read_u16_be(buf, at) else { return };
    let Some(value_format) = read_u16_be(buf, at + 4) else { return };
    if value_format & ANY_DEVICE_FIELD == 0 { return; }
    match format {
        1 => patch_value_record(buf, ctx, at, at + 6, value_format),
        2 => {
            let Some(count) = read_u16_be(buf, at + 6) else { return };
            let len = value_record_len(value_format);
            for i in 0..count as usize {
                let rec = at + 8 + i * len;
                if rec >= buf.len() || !ctx.visit() { return; }
                patch_value_record(buf, ctx, at, rec, value_format);
            }
        }
        _ => {}
    }
}

fn patch_pair(buf: &mut [u8], ctx: &mut Ctx, at: usize) {
    let Some(format) = read_u16_be(buf, at) else { return };
    let (Some(format1), Some(format2)) = (read_u16_be(buf, at + 4), read_u16_be(buf, at + 6)) else {
        return;
    };
    if (format1 | format2) & ANY_DEVICE_FIELD == 0 { return; }
    let (len1, len2) = (value_record_len(format1), value_record_len(format2));

    match format {
        1 => {
            let Some(set_count) = read_u16_be(buf, at + 8) else { return };
            for i in 0..set_count as usize {
                let Some(set_rel) = read_u16_be(buf, at + 10 + i * 2) else { continue };
                let set = at + set_rel as usize;
                let Some(pair_count) = read_u16_be(buf, set) else { continue };
                for p in 0..pair_count as usize {
                    let rec = set + 2 + p * (2 + len1 + len2);
                    // A PairSet's devices are offsets from the PairSet, not from the subtable.
                    if rec >= buf.len() || !ctx.visit() { break; }
                    patch_value_record(buf, ctx, set, rec + 2, format1);
                    patch_value_record(buf, ctx, set, rec + 2 + len1, format2);
                }
            }
        }
        2 => {
            let (Some(count1), Some(count2)) = (read_u16_be(buf, at + 12), read_u16_be(buf, at + 14))
            else {
                return;
            };
            let stride = len1 + len2;
            let cells = usize::from(count1) * usize::from(count2);
            for i in 0..cells {
                let Some(cell) = i.checked_mul(stride).and_then(|o| o.checked_add(at + 16)) else { return };
                if cell >= buf.len() || !ctx.visit() { return; }
                patch_value_record(buf, ctx, at, cell, format1);
                patch_value_record(buf, ctx, at, cell + len1, format2);
            }
        }
        _ => {}
    }
}

fn patch_cursive(buf: &mut [u8], ctx: &mut Ctx, at: usize) {
    let Some(count) = read_u16_be(buf, at + 4) else { return };
    for i in 0..count as usize {
        if !ctx.visit() { return; }
        for slot in 0..2 {
            let Some(rel) = read_u16_be(buf, at + 6 + i * 4 + slot * 2) else { return };
            if rel != 0 {
                patch_anchor(buf, ctx, at + rel as usize);
            }
        }
    }
}

fn patch_mark_attach(buf: &mut [u8], ctx: &mut Ctx, at: usize) {
    let Some(class_count) = read_u16_be(buf, at + 6) else { return };
    if let Some(marks_rel) = read_u16_be(buf, at + 8) {
        patch_mark_array(buf, ctx, at + marks_rel as usize);
    }
    if let Some(bases_rel) = read_u16_be(buf, at + 10) {
        patch_anchor_matrix(buf, ctx, at + bases_rel as usize, class_count);
    }
}

fn patch_mark_lig(buf: &mut [u8], ctx: &mut Ctx, at: usize) {
    let Some(class_count) = read_u16_be(buf, at + 6) else { return };
    if let Some(marks_rel) = read_u16_be(buf, at + 8) {
        patch_mark_array(buf, ctx, at + marks_rel as usize);
    }
    let Some(ligs_rel) = read_u16_be(buf, at + 10) else { return };
    let ligs = at + ligs_rel as usize;
    let Some(lig_count) = read_u16_be(buf, ligs) else { return };
    for i in 0..lig_count as usize {
        if !ctx.visit() { return; }
        let Some(attach_rel) = read_u16_be(buf, ligs + 2 + i * 2) else { return };
        if attach_rel != 0 {
            patch_anchor_matrix(buf, ctx, ligs + attach_rel as usize, class_count);
        }
    }
}

fn patch_mark_array(buf: &mut [u8], ctx: &mut Ctx, at: usize) {
    let Some(count) = read_u16_be(buf, at) else { return };
    for i in 0..count as usize {
        if !ctx.visit() { return; }
        let Some(rel) = read_u16_be(buf, at + 2 + i * 4 + 2) else { return };
        if rel != 0 {
            patch_anchor(buf, ctx, at + rel as usize);
        }
    }
}

fn patch_anchor_matrix(buf: &mut [u8], ctx: &mut Ctx, at: usize, cols: u16) {
    let Some(rows) = read_u16_be(buf, at) else { return };
    for i in 0..(rows as usize).saturating_mul(cols as usize) {
        if !ctx.visit() { return; }
        let Some(rel) = read_u16_be(buf, at + 2 + i * 2) else { return };
        if rel != 0 {
            patch_anchor(buf, ctx, at + rel as usize);
        }
    }
}

fn patch_anchor(buf: &mut [u8], ctx: &mut Ctx, at: usize) {
    if read_u16_be(buf, at) != Some(3) {
        return;
    }
    for (slot, coord) in [(6usize, 2usize), (8, 4)] {
        let Some(dev_rel) = read_u16_be(buf, at + slot) else { continue };
        if dev_rel == 0 {
            continue;
        }
        // A device table of sizes is not variation data and stays; only a VariationIndex goes.
        if let Some(delta) = variation_delta(buf, ctx, at + dev_rel as usize) {
            bump_i16(buf, ctx, at + coord, delta);
            write_u16_be(buf, at + slot, 0);
        }
    }
}

const ANY_DEVICE_FIELD: u16 = 0x00F0;

fn patch_value_record(buf: &mut [u8], ctx: &mut Ctx, parent: usize, at: usize, format: u16) {
    let value_bytes = (format & 0x000F).count_ones() as usize * 2;

    for (i, device_bit) in DEVICE_FIELDS.iter().enumerate() {
        if format & device_bit == 0 {
            continue;
        }
        let slot = (format & (device_bit - 1) & 0x00F0).count_ones() as usize;
        let dev_at = at + value_bytes + slot * 2;
        let Some(dev_rel) = read_u16_be(buf, dev_at) else { continue };

        if dev_rel != 0 && let Some(delta) = variation_delta(buf, ctx, parent + dev_rel as usize) {
            if format & VALUE_FIELDS[i] != 0 {
                let value_slot = (format & (VALUE_FIELDS[i] - 1) & 0x000F).count_ones() as usize;
                bump_i16(buf, ctx, at + value_slot * 2, delta);
            } else if delta != 0 {
                ctx.widen = true;
            }
            write_u16_be(buf, dev_at, 0);
        }
    }
}

fn variation_delta(buf: &[u8], ctx: &Ctx, at: usize) -> Option<i32> {
    if read_u16_be(buf, at + 4)? != DELTA_FORMAT_VARIATION_INDEX {
        return None;
    }
    let outer = read_u16_be(buf, at)?;
    let inner = read_u16_be(buf, at + 2)?;
    Some(ctx.delta(outer, inner))
}

fn bump_i16(buf: &mut [u8], ctx: &mut Ctx, at: usize, delta: i32) {
    if delta == 0 {
        return;
    }
    let Some(word) = ctx.patched.get_mut(at >> 6) else { return };
    let bit = 1u64 << (at & 63);
    if *word & bit != 0 {
        return;
    }
    *word |= bit;
    let Some(current) = super::super::decoder::read_i16_be(buf, at) else { return };
    let next = i32::from(current).saturating_add(delta).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    write_i16_be(buf, at, next);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(w: &[u16]) -> Vec<u8> {
        w.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    // A SinglePos whose x advance device has no x advance, a VariationIndex of -40 at the peak, in a
    // lookup filtering marks by GDEF's one mark glyph set when `filter`: the lookup instanced there.
    fn instanced_lookup(filter: bool) -> (Vec<u8>, usize) {
        let mut gpos = words(&[1, 0, 10, 12, 14, 0, 0, 1, 4]);
        gpos.extend(if filter { words(&[1, 0x0010, 1, 10, 0]) } else { words(&[1, 0, 1, 8]) });
        gpos.extend(words(&[1, 8, 0x0040, 14, 1, 1, 5, 0, 0, 0x8000]));
        let mut gdef = words(&[1, 3, 0, 0, 0, 0, 18, 0, 32, 1, 1, 0, 8, 1, 1, 7]);
        gdef.extend(crate::daecore::daetype::colr_v1::testing::store(1, -40));
        let map: BTreeMap<String, TableBytes> = [("GPOS", gpos), ("GDEF", gdef), ("maxp", words(&[0, 0x5000, 10]))]
            .into_iter()
            .map(|(t, b)| (String::from(t), TableBytes::from(b)))
            .collect();
        let out = apply_gpos_var(&map, &[1.0]).expect("an instanced GPOS");
        let lookups = usize::from(read_u16_be(&out, 8).expect("a lookup list"));
        let lookup = lookups + usize::from(read_u16_be(&out, lookups + 2).expect("a lookup"));
        (out, lookup)
    }

    // The missing value is 0, so the instance kerns by -40 as daegun's shaper and fontTools do, not
    // dropping the delta.
    #[test]
    fn a_device_without_its_value_still_moves_the_glyph() {
        let (out, lookup) = instanced_lookup(false);
        let sub = lookup + usize::from(read_u16_be(&out, lookup + 6).expect("a subtable"));
        let format = read_u16_be(&out, sub + 4).expect("a value format");
        assert!(format & 0x0004 != 0, "no x advance in format {format:#06x}");
        assert_eq!(super::super::super::decoder::read_i16_be(&out, sub + 6), Some(-40));
    }

    // Rebuilt to hold that value, the lookup keeps filtering by the mark glyph set GDEF still holds.
    #[test]
    fn a_rebuilt_lookup_keeps_its_mark_filtering_set() {
        let (out, lookup) = instanced_lookup(true);
        assert_eq!(read_u16_be(&out, lookup + 2), Some(0x0010));
        assert_eq!(read_u16_be(&out, lookup + 8), Some(0));
    }
}
