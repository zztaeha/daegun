use super::aat::SubtableSets;
use super::kerx::{self, Kind, Subtable};
use crate::daecore::daeshaper::run::{Run, Mask};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daetype::decoder::{read_i16_be, read_u16_be};
use crate::daecore::daetype::format::aat::GlyphRanges;
use crate::daecore::daetype::format::sanitize::AatTable;
use crate::daecore::daekit::base::sync::Shared;
use alloc::vec::Vec;

// `kern` and its form, told apart by its first 16 bits: 0 for OpenType's header, 1 for Apple's. Another passes
// the sanitizer and applies nothing.
fn table<'a>(face: &Face<'a>) -> Option<(&'a [u8], Kind)> {
    let table = face.aat(AatTable::Kern)?;
    let kind = match read_u16_be(table, 0)? {
        0 => Kind::KernOt,
        1 => Kind::KernAat,
        _ => return None,
    };
    Some((table, kind))
}

// The plan's machine and cross-stream probes count every subtable, whatever its direction or variation.
fn subtables<'a>(face: &Face<'a>) -> impl Iterator<Item = Subtable<'a>> {
    table(face).into_iter().flat_map(|(table, kind)| kerx::subtables(table, kind))
}

// Whether the table holds data once the sanitizer passes it: a version or subtable count that isn't zero.
pub(crate) fn has_data(face: &Face) -> bool {
    face.aat(AatTable::Kern).is_some()
}

pub(crate) fn has_machine_kerning(face: &Face) -> bool {
    subtables(face).any(|s| s.format == 1)
}

pub(crate) fn has_cross_stream(face: &Face) -> bool {
    subtables(face).any(|s| s.cross_stream())
}

// The table's subtables' sets for a plan whose `kern` runs.
pub(crate) fn plan_sets(face: &Face, horizontal: bool) -> Vec<Option<Shared<SubtableSets>>> {
    table(face).map_or_else(Vec::new, |(table, kind)| kerx::plan_sets(face, table, kind, horizontal))
}

pub(crate) fn apply(face: &Face, buffer: &mut Run, kern_mask: Mask, planned: &[Option<Shared<SubtableSets>>]) {
    if let Some((table, kind)) = table(face) {
        kerx::walk(face, buffer, table, kind, kern_mask, planned);
    }
}

// Format 3's counts: glyphs, values, left classes and right classes.
fn format3_counts(sub: &Subtable) -> Option<(usize, usize, usize, usize)> {
    let at = sub.body();
    let byte = |i: usize| sub.data.get(at + i).map(|&b| usize::from(b));
    Some((usize::from(read_u16_be(sub.data, at)?), byte(2)?, byte(3)?, byte(4)?))
}

// Every glyph of the font on both sides, unless the table holds no glyphs.
pub(super) fn format3_sets(sub: &Subtable, num_glyphs: u32) -> SubtableSets {
    let held = format3_counts(sub).is_some_and(|(glyphs, ..)| glyphs != 0) && num_glyphs > 0;
    let all = || GlyphRanges::new(held.then(|| (0, num_glyphs - 1)).into_iter().collect());
    SubtableSets { first: all(), second: all(), classes: None }
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/ot/kern_testing.rs"]
pub(crate) mod testing;

// Apple's format 3: a byte class for each side picks a byte index into the values. A read past an array gives 0.
pub(super) fn format3(sub: &Subtable, left: u32, right: u32) -> i32 {
    let Some((glyphs, values, lefts, rights)) = format3_counts(sub) else { return 0 };
    let values_at = sub.body() + 6;
    let lefts_at = values_at + values * 2;
    let rights_at = lefts_at + glyphs;
    let index_at = rights_at + glyphs;
    let byte = |at: usize| sub.data.get(at).map_or(0, |&b| usize::from(b));
    let class = |at: usize, glyph: u32| if (glyph as usize) < glyphs { byte(at + glyph as usize) } else { 0 };

    let (l, r) = (class(lefts_at, left), class(rights_at, right));
    if l >= lefts || r >= rights {
        return 0;
    }
    let index = byte(index_at + l * rights + r);
    if index >= values {
        return 0;
    }
    read_i16_be(sub.data, values_at + index * 2).map_or(0, i32::from)
}
