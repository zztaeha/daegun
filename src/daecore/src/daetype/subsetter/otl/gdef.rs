use crate::daecore::daetype::subsetter::{remap_gid, GlyphSet};
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::{read_u16_be, read_u32_be, write_u16_be, write_u32_be, read_i16_be, write_i16_be};
use crate::daecore::daetype::format::class_def;
use crate::daecore::daetype::format::ivs::ItemVariationStore;
use super::generic::{self, schemas};

// A GDEF header's offsets, 0 for none, as the shaper reads them: major 1's by the minor version, a field past the
// table's end left out, from 1.4 each non-null 32-bit offset in place of its 16-bit one. Another major holds nothing.
pub(crate) struct GdefHeader {
    pub(crate) glyph_class: usize,
    pub(crate) attach_list: usize,
    pub(crate) lig_caret_list: usize,
    pub(crate) mark_attach: usize,
    pub(crate) mark_glyph_sets: usize,
    pub(crate) var_store: usize,
    // Which of the five parts, in header order, 1.4's 32-bit offsets name.
    pub(crate) long: [bool; 5],
}

impl GdefHeader {
    pub(crate) fn read(gdef: &[u8]) -> Option<GdefHeader> {
        if gdef.len() < 12 || read_u16_be(gdef, 0)? != 1 { return None; }
        let minor = read_u16_be(gdef, 2)?;
        let long = |at: usize| (minor >= 4).then(|| read_u32_be(gdef, at)).flatten().filter(|&off| off != 0);
        let offset = |at: usize, long_at: usize| {
            long(long_at).map_or_else(|| read_u16_be(gdef, at).map_or(0, usize::from), |off| off as usize)
        };
        Some(GdefHeader {
            glyph_class: offset(4, 18),
            attach_list: offset(6, 22),
            lig_caret_list: offset(8, 26),
            mark_attach: offset(10, 30),
            mark_glyph_sets: if minor >= 2 { offset(12, 34) } else { 0 },
            var_store: if minor >= 3 { read_u32_be(gdef, 14).map_or(0, |v| v as usize) } else { 0 },
            long: [18, 22, 26, 30, 34].map(|at| long(at).is_some()),
        })
    }

    // Whether the ligature carets are 1.4's, which count their ligatures and reach them in 24 bits.
    pub(crate) fn lig_carets_24(&self) -> bool {
        self.long[2]
    }
}

// The store a GDEF from 1.3 varies its values by.
pub(crate) fn var_store(gdef: &[u8]) -> Option<ItemVariationStore> {
    match GdefHeader::read(gdef)?.var_store {
        0 => None,
        at => ItemVariationStore::parse(gdef, at).ok(),
    }
}

pub fn subset_gdef(gdef: &[u8], active: &GlyphSet, gid_map: &[u32]) -> Option<Vec<u8>> {
    let header = GdefHeader::read(gdef)?;
    let item_var_store = match header.var_store {
        0 => None,
        at => gdef.get(at..).map(<[u8]>::to_vec),
    };

    let mut work = generic::Work::for_table(gdef.len());
    let mut part = |off: usize, schema: &generic::schema::Schema| {
        if off == 0 { return None; }
        let kept = generic::Kept::Subset(active, gid_map);
        generic::subset_subtable(gdef, off, schema, kept, generic::Devices::Keep, &mut work)
    };
    // A font with glyph classes keeps them, if empty: without them a subset's marks would come from Unicode.
    let has_classes = header.glyph_class != 0 && gdef.len().saturating_sub(header.glyph_class) >= 4;
    let glyph_class = part(header.glyph_class, &schemas::gdef::CLASS_DEF)
        .or_else(|| has_classes.then(|| [0, 2, 0, 0].to_vec()));
    let attach_list = part(header.attach_list, &schemas::gdef::ATTACH_LIST);
    let lig_carets = match header.lig_carets_24() {
        true => &schemas::gdef::LIG_CARET_LIST_24,
        false => &schemas::gdef::LIG_CARET_LIST,
    };
    let lig_caret_list = part(header.lig_caret_list, lig_carets);
    let mark_attach = part(header.mark_attach, &schemas::gdef::CLASS_DEF);
    let (_, sets) = mark_sets(gdef, active, gid_map);
    let mark_glyph_sets = (!sets.is_empty()).then(|| {
        let mut out = [1, sets.len() as u16].map(u16::to_be_bytes).concat();
        let starts = sets.iter().scan(4 + 4 * sets.len(), |at, set| Some(core::mem::replace(at, *at + set.len())));
        starts.for_each(|at| out.extend_from_slice(&(at as u32).to_be_bytes()));
        out.extend(sets.concat());
        out
    });

    if glyph_class.is_none() && attach_list.is_none() && lig_caret_list.is_none()
        && mark_attach.is_none() && mark_glyph_sets.is_none() && item_var_store.is_none()
    {
        return None;
    }

    // 1.4 stays where the source names a part at a 32-bit offset, the one version that holds such offsets. Otherwise
    // the version is the least that holds what's kept, and a 1.4 header cut to 1.3 leaves no 20 zero bytes behind.
    let has_mgs = mark_glyph_sets.is_some();
    let has_ivs = item_var_store.is_some();
    let (header_len, minor) = match (header.long.contains(&true), has_ivs, has_mgs) {
        (true, ..) => (38, 4),
        (_, true, _) => (18, 3),
        (_, _, true) => (14, 2),
        _ => (12, 0),
    };
    let mut out = vec![0u8; header_len];
    write_u16_be(&mut out, 0, 1);
    write_u16_be(&mut out, 2, minor);
    // Offsets are to each part's start. Where the usual order pushes a 16-bit one past 64 KB, the largest part goes
    // last, so only the others have to fit before it, and the 32-bit ones go after them all.
    let parts = [glyph_class, attach_list, lig_caret_list, mark_attach, mark_glyph_sets];
    let mut parts: Vec<_> = (0..5).zip(header.long).zip(parts)
        .map(|((k, long), part)| if long { (18 + 4 * k, 4, part) } else { (4 + 2 * k, 2, part) })
        .collect();
    let mut tail = match place(&mut out, &parts) {
        Some(tail) => tail,
        None => {
            parts.sort_by_key(|(_, width, part)| (*width, part.as_ref().map_or(0, Vec::len)));
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

// Each part after the header, its offset written in the slot of its width.
fn place(header: &mut [u8], parts: &[(usize, usize, Option<Vec<u8>>)]) -> Option<Vec<u8>> {
    let mut tail = Vec::new();
    for (slot, width, part) in parts {
        let Some(part) = part else { continue };
        let at = header.len() + tail.len();
        match width {
            2 => { write_u16_be(header, *slot, u16::try_from(at).ok()?); }
            _ => { write_u32_be(header, *slot, u32::try_from(at).ok()?); }
        }
        tail.extend_from_slice(part);
    }
    Some(tail)
}

pub fn glyph_class(gdef: &[u8], glyph: u32) -> u32 {
    class_at(gdef, GdefHeader::read(gdef).map_or(0, |h| h.glyph_class), glyph)
}

pub fn mark_attach_class(gdef: &[u8], glyph: u32) -> u32 {
    class_at(gdef, GdefHeader::read(gdef).map_or(0, |h| h.mark_attach), glyph)
}

fn class_at(gdef: &[u8], at: usize, glyph: u32) -> u32 {
    let class_def = (at != 0).then(|| gdef.get(at..)).flatten();
    class_def.map_or(0, |data| class_def::class_of(data, glyph))
}

// Each mark glyph set's new index, and the coverages of those kept.
type MarkSets = (Vec<Option<u16>>, Vec<Vec<u8>>);

// A GDEF's mark glyph sets as a subset keeps them: each set's new index, those covering a kept glyph numbered in
// order, and their coverages. None where one can't be read, as the sanitizer drops them, so lookups filter by none.
pub(crate) fn mark_sets(gdef: &[u8], active: &GlyphSet, gid_map: &[u32]) -> MarkSets {
    kept_sets(gdef, active, gid_map).unwrap_or_default()
}

// A coverage many sets name is read once. Its glyphs spend the table's work and each set kept its bytes from the
// subset budget; past either, no set can be read.
fn kept_sets(gdef: &[u8], active: &GlyphSet, gid_map: &[u32]) -> Option<MarkSets> {
    let at = GdefHeader::read(gdef).map_or(0, |h| h.mark_glyph_sets);
    let (mut work, mut held) = (generic::Work::for_table(gdef.len()), super::super::subset_budget(gdef.len()));
    let mut built: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    let (mut map, mut sets) = (Vec::new(), Vec::new());
    for i in 0..usize::from(mark_glyph_set_count(gdef)) {
        let off = read_u32_be(gdef, at + 4 + 4 * i)?;
        if off != 0 && !built.contains_key(&off) {
            let coverage = super::Coverage::read(gdef, at + off as usize).ok()?;
            work.spend(coverage.len).ok()?;
            let mut glyphs = Vec::new();
            coverage.runs(|first, last| glyphs.extend((first..=last).filter_map(|g| remap_gid(active, gid_map, g))));
            built.insert(off, if glyphs.is_empty() { Vec::new() } else { super::build_coverage(&glyphs) });
        }
        let set = built.get(&off).filter(|set| !set.is_empty());
        map.push(set.map(|_| sets.len() as u16));
        if let Some(set) = set {
            held = held.checked_sub(set.len())?;
            sets.push(set.clone());
        }
    }
    Some((map, sets))
}

// A GDEF of `count` mark glyph sets, each covering every glyph, and nothing else: what an instance keeps of a GDEF
// its font doesn't read, its lookups still naming them. Each filters by none, as the font's missing sets do.
pub(crate) fn sets_of_every_glyph(count: u16, glyphs: usize) -> Option<Vec<u8>> {
    if count == 0 || glyphs == 0 { return None; }
    let coverage = 4 + 4 * u32::from(count);
    let mut out = [1u16, 2, 0, 0, 0, 0, 14, 1, count].map(u16::to_be_bytes).concat();
    (0..count).for_each(|_| out.extend_from_slice(&coverage.to_be_bytes()));
    let last = (glyphs - 1).min(0xFFFF) as u16;
    out.extend([2u16, 1, 0, last, 0].map(u16::to_be_bytes).concat());
    Some(out)
}

// How many mark glyph sets a GDEF holds, 0 when it has no MarkGlyphSetsDef or one that cannot be read.
pub fn mark_glyph_set_count(gdef: &[u8]) -> u16 {
    let sets = match GdefHeader::read(gdef) {
        Some(header) if header.mark_glyph_sets != 0 => header.mark_glyph_sets,
        _ => return 0,
    };
    match (read_u16_be(gdef, sets), read_u16_be(gdef, sets + 2)) {
        (Some(1), Some(count)) => count,
        _ => 0,
    }
}

#[derive(Clone, Copy)]
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
#[path = "../../../../tests/unit/daetype/subsetter/otl/gdef.rs"]
mod tests;
