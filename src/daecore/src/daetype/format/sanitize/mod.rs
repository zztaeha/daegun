use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::super::decoder::{read_u16_be, read_u32_be};
use super::aat::morx_chains;

// Bounds checks over one table, for the walks that test a structure before reading it.
pub(crate) struct Sanitizer<'a> {
    data: &'a [u8],
}

impl<'a> Sanitizer<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Sanitizer { data }
    }

    // Whether the `size` bytes at `at` lie in the table.
    pub(crate) fn fits(&self, at: usize, size: usize) -> bool {
        at.checked_add(size).is_some_and(|end| end <= self.data.len())
    }

    // `n` bytes at `at`, inside the table.
    pub(crate) fn range(&self, at: usize, n: u32) -> bool {
        usize::try_from(n).is_ok_and(|n| self.fits(at, n))
    }

    // `count` items of `size` bytes, none where the product overflows 32 bits.
    pub(crate) fn array(&self, at: usize, count: u32, size: u32) -> bool {
        count.checked_mul(size).is_some_and(|n| self.range(at, n))
    }
}

// Whether `data` is long enough for a header of `size` bytes and states a version `known` takes, read as a 16-bit
// word at its start. A table whose header doesn't read, or of a version its spec doesn't define, is skipped whole;
// anything inside it that leaves the table is absent where its reader meets it, and the rest is kept.
fn header(data: &[u8], size: usize, known: impl Fn(u16) -> bool) -> bool {
    data.len() >= size && read_u16_be(data, 0).is_some_and(known)
}

// The AAT tables shaping reads once their headers pass.
#[derive(Clone, Copy)]
pub(crate) enum AatTable {
    Morx,
    Kerx,
    Kern,
    Ankr,
}

impl AatTable {
    pub(crate) const ALL: [AatTable; 4] = [AatTable::Morx, AatTable::Kerx, AatTable::Kern, AatTable::Ankr];

    pub(crate) fn tag(self) -> &'static str {
        match self {
            AatTable::Morx => "morx",
            AatTable::Kerx => "kerx",
            AatTable::Kern => "kern",
            AatTable::Ankr => "ankr",
        }
    }
}

// Whether shaping uses the font's `which`, by the versions Apple's manual defines: morx 2 and 3 whose walk fits,
// kerx 2 to 4, ankr 0, and `kern` as OpenType's version 0 or Apple's 1.0, holding at least one subtable.
pub(crate) fn shapes_with<'a>(which: AatTable, table: impl Fn(&str) -> Option<&'a [u8]>) -> bool {
    let Some(data) = table(which.tag()) else { return false };
    match which {
        AatTable::Morx => header(data, 8, |v| matches!(v, 2 | 3)) && morx_walk_fits(data),
        AatTable::Kerx => header(data, 8, |v| matches!(v, 2..=4)),
        AatTable::Kern => match read_u16_be(data, 0) {
            Some(0) => read_u16_be(data, 2).is_some_and(|n| n > 0),
            Some(1) => read_u16_be(data, 2) == Some(0) && read_u32_be(data, 4).is_some_and(|n| n > 0),
            _ => false,
        },
        AatTable::Ankr => header(data, 12, |v| v == 0),
    }
}

// Whether morx's chains yield at most twice the subtables its bytes hold. Each is read on past its end, so each can
// read the rest of the table, which no real font's does; the walk is counted only that far.
fn morx_walk_fits(data: &[u8]) -> bool {
    let most = data.len() / 12 * 2;
    morx_chains(data).flat_map(|chain| chain.subtables(data)).take(most + 1).count() <= most
}

// Whether a font's COLR, or its CPAL, is read by the readers that go by these verdicts: versions 0 and 1, each with
// its whole header, as the OpenType spec defines them.
pub(crate) fn colr_passes(data: &[u8]) -> bool {
    header(data, if read_u16_be(data, 0) == Some(1) { 34 } else { 14 }, |v| v <= 1)
}

pub(crate) fn cpal_passes(data: &[u8]) -> bool {
    header(data, 12, |v| v <= 1)
}

// Whether measuring reads a font's VARC: major version 1 with its header's five offsets.
pub(crate) fn varc_passes(data: &[u8]) -> bool {
    header(data, 24, |v| v == 1)
}

// The OpenType layout tables read once their headers pass.
#[derive(Clone, Copy)]
pub(crate) enum OtlTable {
    Gsub,
    Gpos,
    Gdef,
}

impl OtlTable {
    pub(crate) const ALL: [OtlTable; 3] = [OtlTable::Gsub, OtlTable::Gpos, OtlTable::Gdef];

    pub(crate) fn tag(self) -> &'static str {
        match self {
            OtlTable::Gsub => "GSUB",
            OtlTable::Gpos => "GPOS",
            OtlTable::Gdef => "GDEF",
        }
    }

    pub(crate) fn of(tag: &str) -> Option<OtlTable> {
        OtlTable::ALL.into_iter().find(|which| which.tag() == tag)
    }
}

// Whether `data` is read as the font's `which`: major version 1, as the OpenType spec defines GSUB, GPOS and GDEF,
// with version 1.0's header. A later minor's fields that leave the table are absent.
pub(crate) fn otl_passes(which: OtlTable, data: &[u8]) -> bool {
    header(data, if matches!(which, OtlTable::Gdef) { 12 } else { 10 }, |major| major == 1)
}

// The check GPOS makes before it reads an anchor: one of format 1, 2 or 3 whose own fields lie in `data`, which runs
// from the offset's base to the table's end. Its devices are absent where they leave.
pub(crate) fn anchor_fits(data: &[u8], off: u32) -> bool {
    let size = match read_u16_be(data, off as usize) {
        Some(1) => 6,
        Some(2) => 8,
        Some(3) => 10,
        _ => return false,
    };
    Sanitizer::new(data).fits(off as usize, size)
}

// How many rows and entries the extended state table at the start of `data` reaches from state 0, its entries
// `entry_size` bytes. A row or entry past the table is absent, and so is what only it would reach.
pub(crate) fn state_reach(data: &[u8], entry_size: u32) -> Option<(usize, usize)> {
    let field = |i: usize| read_u32_be(data, 4 * i).and_then(|v| usize::try_from(v).ok());
    let (n_classes, states, entries) = (field(0)?, field(2)?, field(3)?);
    let row = n_classes.checked_mul(2).filter(|_| n_classes >= 4)?;
    let fitting = |at: usize, size: usize| data.len().checked_sub(at).map_or(0, |left| left / size);
    let (rows_in, entries_in) = (fitting(states, row), fitting(entries, entry_size as usize));
    let (mut rows, mut reached) = (1usize.min(rows_in), 0usize);
    let (mut rows_read, mut entries_read) = (0, 0);
    while rows_read < rows || entries_read < reached {
        for cell in rows_read * n_classes..rows * n_classes {
            let index = read_u16_be(data, states + 2 * cell).map_or(0, usize::from);
            reached = reached.max(index + 1).min(entries_in);
        }
        rows_read = rows;
        for entry in entries_read..reached {
            let new_state = read_u16_be(data, entries + entry * entry_size as usize).map_or(0, usize::from);
            rows = rows.max(new_state + 1).min(rows_in);
        }
        entries_read = reached;
    }
    Some((rows, reached))
}

// Whether shaping reads a font's feat, or its ltag, which name the features and languages morx chains take: feat at
// major version 1 and ltag at version 1, as Apple's manual defines them, each with its header.
pub(crate) fn feat_passes(data: &[u8]) -> bool {
    header(data, 12, |major| major == 1)
}

pub(crate) fn ltag_passes(data: &[u8]) -> bool {
    data.len() >= 12 && read_u32_be(data, 0) == Some(1)
}

// Each verdict a subset takes of its font, once: which layout and color tables the sanitizer rejects, and whether
// shaping uses each AAT table.
#[derive(Clone, Copy)]
pub(crate) struct Reads {
    rejected: [bool; 3],
    colr: bool,
    cpal: bool,
    varc: bool,
    shaped: [bool; 4],
}

impl Reads {
    pub(crate) fn of<'a>(table: impl Fn(&str) -> Option<&'a [u8]> + Copy) -> Reads {
        let mut reads = Reads::layout(table);
        reads.shaped = AatTable::ALL.map(|which| shapes_with(which, reads.reading(table)));
        reads
    }

    // The verdicts an instance takes of the font these are the tables of, its glyphs counted as maxp counts them.
    pub(crate) fn of_tables(table_map: &crate::daecore::daetype::Tables) -> Reads {
        Reads::of(|tag| table_map.get(tag).map(|t| &t[..]))
    }

    fn layout<'a>(table: impl Fn(&str) -> Option<&'a [u8]>) -> Reads {
        Reads {
            rejected: OtlTable::ALL.map(|which| table(which.tag()).is_some_and(|data| !otl_passes(which, data))),
            colr: table("COLR").is_some_and(|data| !colr_passes(data)),
            cpal: table("CPAL").is_some_and(|data| !cpal_passes(data)),
            varc: table("VARC").is_some_and(|data| !varc_passes(data)),
            shaped: [false; 4],
        }
    }

    // The font's tables as their readers take them.
    pub(crate) fn reading<'a>(self, table: impl Fn(&str) -> Option<&'a [u8]>) -> impl Fn(&str) -> Option<&'a [u8]> {
        move |tag| {
            let read = OtlTable::of(tag).is_none_or(|which| !self.rejected[which as usize]);
            let rejected = match tag {
                "COLR" => self.colr,
                "CPAL" => self.cpal,
                "VARC" => self.varc,
                _ => false,
            };
            table(tag).filter(|_| read && !rejected)
        }
    }

    pub(crate) fn shaped(&self, which: AatTable) -> bool {
        self.shaped[which as usize]
    }
}

// A subset's or an instance's AAT tables by its font's verdicts: one shaping doesn't use in the font goes, as the new
// font would otherwise shape with it.
pub(crate) fn read_as_judged<V: AsRef<[u8]> + From<Vec<u8>>>(reads: &Reads, out: &mut BTreeMap<String, V>) {
    for which in AatTable::ALL {
        if !reads.shaped(which) {
            out.remove(which.tag());
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/format/sanitize/mod.rs"]
mod tests;
