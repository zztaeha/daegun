use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::{read_u16_be, read_u32_be, write_u16_be, write_u32_be, read_i16_be, write_i16_be};
use super::generic::{self, schemas};

pub fn subset_gdef(gdef: &[u8], active: &GlyphSet, gid_map: &[u16]) -> Option<Vec<u8>> {
    if gdef.len() < 12 { return None; }
    let major = read_u16_be(gdef, 0)?;
    let minor = read_u16_be(gdef, 2)?;
    let glyph_class_off = read_u16_be(gdef, 4)? as usize;
    let attach_list_off = read_u16_be(gdef, 6)? as usize;
    let lig_caret_list_off = read_u16_be(gdef, 8)? as usize;
    let mark_attach_off = read_u16_be(gdef, 10)? as usize;
    let mark_glyph_sets_off = if minor >= 2 && gdef.len() >= 14 { read_u16_be(gdef, 12)? as usize } else { 0 };
    let item_var_store = if minor >= 3 && gdef.len() >= 18 {
        match read_u32_be(gdef, 14)? as usize {
            0 => None,
            off => gdef.get(off..).map(<[u8]>::to_vec),
        }
    } else {
        None
    };

    let mut work = generic::Work::for_table(gdef.len());
    let mut part = |off: usize, schema: generic::schema::Schema| {
        if off == 0 { return None; }
        generic::subset_subtable(gdef, off, &schema, active, gid_map, generic::Devices::Keep, &mut work)
    };
    let glyph_class = part(glyph_class_off, schemas::gdef::glyph_class_def_schema());
    let attach_list = part(attach_list_off, schemas::gdef::attach_list_schema());
    let lig_caret_list = part(lig_caret_list_off, schemas::gdef::lig_caret_list_schema());
    let mark_attach = part(mark_attach_off, schemas::gdef::mark_attach_class_def_schema());
    // MarkGlyphSets that cannot be read are left out alone, as HarfBuzz's sanitizer drops them; the
    // lookups naming them drop their mark filtering sets to match.
    let mark_glyph_sets = part(mark_glyph_sets_off, schemas::gdef::mark_glyph_sets_schema());

    if glyph_class.is_none() && attach_list.is_none() && lig_caret_list.is_none()
        && mark_attach.is_none() && mark_glyph_sets.is_none() && item_var_store.is_none()
    {
        return None;
    }

    let has_mgs = mark_glyph_sets.is_some();
    let has_ivs = item_var_store.is_some();
    let header_len = if has_ivs { 18 } else if has_mgs { 14 } else { 12 };
    let mut out = vec![0u8; header_len];
    write_u16_be(&mut out, 0, major);
    write_u16_be(&mut out, 2, if has_ivs { 3 } else if has_mgs { 2 } else { 0 });
    // Offsets are 16-bit, to each part's start. Where the usual order pushes one past 64 KB, the largest
    // part goes last, so only the others have to fit before it.
    let mut parts = [(4, glyph_class), (6, attach_list), (8, lig_caret_list), (10, mark_attach), (12, mark_glyph_sets)];
    let mut tail = match place(&mut out, &parts) {
        Some(tail) => tail,
        None => {
            parts.sort_by_key(|(_, part)| part.as_ref().map_or(0, Vec::len));
            place(&mut out, &parts)?
        }
    };
    if let Some(ivs) = &item_var_store {
        write_u32_be(&mut out, 14, u32::try_from(header_len + tail.len()).ok()?);
        tail.extend_from_slice(ivs);
    }
    out.extend(tail);
    Some(out)
}

fn place(header: &mut [u8], parts: &[(usize, Option<Vec<u8>>)]) -> Option<Vec<u8>> {
    let mut tail = Vec::new();
    for (slot, part) in parts {
        let Some(part) = part else { continue };
        write_u16_be(header, *slot, u16::try_from(header.len() + tail.len()).ok()?);
        tail.extend_from_slice(part);
    }
    Some(tail)
}

pub fn glyph_class(gdef: &[u8], glyph: u16) -> u16 {
    class_at(gdef, 4, glyph)
}

pub fn mark_attach_class(gdef: &[u8], glyph: u16) -> u16 {
    class_at(gdef, 10, glyph)
}

fn class_at(gdef: &[u8], header_offset: usize, glyph: u16) -> u16 {
    let Some(off) = read_u16_be(gdef, header_offset).map(usize::from).filter(|&o| o != 0) else {
        return 0;
    };
    let Some(format) = read_u16_be(gdef, off) else { return 0 };
    match format {
        1 => {
            let (Some(start), Some(count)) =
                (read_u16_be(gdef, off + 2), read_u16_be(gdef, off + 4)) else { return 0 };
            if glyph < start || glyph - start >= count { return 0; }
            read_u16_be(gdef, off + 6 + usize::from(glyph - start) * 2).unwrap_or(0)
        }
        2 => {
            let Some(count) = read_u16_be(gdef, off + 2) else { return 0 };
            for i in 0..usize::from(count) {
                let rec = off + 4 + i * 6;
                let (Some(lo), Some(hi), Some(class)) = (
                    read_u16_be(gdef, rec),
                    read_u16_be(gdef, rec + 2),
                    read_u16_be(gdef, rec + 4),
                ) else { return 0 };
                if glyph < lo { return 0; }
                if glyph <= hi { return class; }
            }
            0
        }
        _ => 0,
    }
}

// How many mark glyph sets a GDEF holds, 0 when it has no MarkGlyphSetsDef or one that cannot be read.
pub fn mark_glyph_set_count(gdef: &[u8]) -> u16 {
    let sets = match (read_u16_be(gdef, 2), read_u16_be(gdef, 12)) {
        (Some(2..), Some(off)) if off != 0 => usize::from(off),
        _ => return 0,
    };
    match (read_u16_be(gdef, sets), read_u16_be(gdef, sets + 2)) {
        (Some(1), Some(count)) => count,
        _ => 0,
    }
}

pub(crate) enum CaretValue {
    Coordinate(i16),
    Point(u16),
}

pub(crate) fn parse_caret_value(buf: &[u8], off: usize) -> Option<CaretValue> {
    match read_u16_be(buf, off)? {
        1 => Some(CaretValue::Coordinate(read_i16_be(buf, off + 2)?)),
        2 => Some(CaretValue::Point(read_u16_be(buf, off + 2)?)),
        3 => Some(CaretValue::Coordinate(read_i16_be(buf, off + 2)?)),
        _ => None,
    }
}

pub(crate) fn build_caret_value(cv: &CaretValue) -> Vec<u8> {
    let mut out = vec![0u8; 4];
    match cv {
        CaretValue::Coordinate(c) => { write_u16_be(&mut out, 0, 1); write_i16_be(&mut out, 2, *c); }
        CaretValue::Point(p) => { write_u16_be(&mut out, 0, 2); write_u16_be(&mut out, 2, *p); }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // GDEF 1.2 whose MarkGlyphSets have an undefined format: the class definition stays without them,
    // rather than the whole GDEF going.
    #[test]
    fn unreadable_mark_glyph_sets_leave_the_rest() {
        let gdef = [1u16, 2, 14, 0, 0, 0, 22, 1, 5, 1, 3, 2, 0].map(u16::to_be_bytes).concat();
        let mut active = GlyphSet::new();
        (0..8).for_each(|g| { active.insert(g); });
        let out = subset_gdef(&gdef, &active, &(0..8).collect::<Vec<u16>>()).expect("a subset GDEF");
        assert_eq!(glyph_class(&out, 5), 3);
        assert_eq!(mark_glyph_set_count(&out), 0);
    }

    // A source GDEF that keeps its 80 KB class definition last, so its own offsets fit 16 bits. Rebuilt
    // class definition first, the attach list behind it would sit past 64 KB.
    #[test]
    fn a_large_part_does_not_push_another_past_16_bit_offsets() {
        const N: u16 = 40_000;
        let attach = [10u16, 1, 6, 1, 0, 1, 1, 5].map(u16::to_be_bytes).concat();
        let mut gdef = [1u16, 0, 12 + attach.len() as u16, 12, 0, 0].map(u16::to_be_bytes).concat();
        gdef.extend(&attach);
        gdef.extend([1u16, 0, N].map(u16::to_be_bytes).concat());
        gdef.extend((0..N).flat_map(|g| (1 + g % 2).to_be_bytes()));
        let mut active = GlyphSet::new();
        (0..N).for_each(|g| { active.insert(g); });
        let gid_map: Vec<u16> = (0..N).collect();

        let out = subset_gdef(&gdef, &active, &gid_map).expect("a subset GDEF");
        let schema = schemas::gdef::attach_list_schema();
        let mut work = generic::Work::for_table(gdef.len());
        let want = generic::subset_subtable(&gdef, 12, &schema, &active, &gid_map, generic::Devices::Keep, &mut work);
        let want = want.expect("the attach list subsets");
        let attach_at = usize::from(read_u16_be(&out, 6).expect("an attach list offset"));
        assert_eq!(out.get(attach_at..attach_at + want.len()), Some(&want[..]), "the attach list offset is wrong");
        assert_eq!(glyph_class(&out, 7), 2, "the class definition moved");
    }
}
