use crate::daecore::daetype::subsetter::{remap_gid16, GlyphSet};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use alloc::vec;
use super::super::decoder::{read_u16_be, read_u32_be, write_u16_be, write_u32_be};
use super::super::format::{eblc, sbix};

struct Bitmap<'a> {
    gid: u16,
    data: &'a [u8],
}

enum Storage<'a> {
    Variable,
    Fixed { image_size: u32, metrics: &'a [u8; 8] },
}

struct SubTable<'a> {
    image_format: u16,
    storage: Storage<'a>,
    glyphs: Vec<Bitmap<'a>>,
}

#[derive(Clone)]
struct Built {
    bytes: Vec<u8>,
    first: u16,
    last: u16,
}

// Glyph steps for walking index subtables; once spent, the table is refused rather than skipped.
// A sound font walks each glyph of a strike once at the cost of index or image bytes, so it fits.
struct Steps {
    left: usize,
    spent: bool,
}

impl Steps {
    fn new(source: usize) -> Self {
        Steps { left: super::subset_budget(source), spent: false }
    }

    fn take(&mut self, n: usize) -> Option<()> {
        match self.left.checked_sub(n) {
            Some(left) => { self.left = left; Some(()) }
            None => { self.spent = true; None }
        }
    }
}

fn push_u16(v: &mut Vec<u8>, x: u16) { v.extend_from_slice(&x.to_be_bytes()); }
fn push_u32(v: &mut Vec<u8>, x: u32) { v.extend_from_slice(&x.to_be_bytes()); }

// Whether a glyph lies in one of the ranges a record decides, which `Claims::claim` gives in order.
fn decides(decided: &[(u16, u16)], gid: u16) -> bool {
    let i = decided.partition_point(|&(_, end)| end < gid);
    decided.get(i).is_some_and(|&(start, _)| start <= gid)
}

// The glyphs `keep` names that an index subtable holds images for, its walk charged to `steps`.
fn read_subtable<'a>(
    index: &eblc::IndexSubtable<'a>, keep: impl Fn(u16) -> bool, steps: &mut Steps,
) -> Option<SubTable<'a>> {
    steps.take(index.entry_count())?;
    let storage = match index.fixed() {
        Some((image_size, metrics)) => Storage::Fixed { image_size, metrics },
        None => Storage::Variable,
    };
    let glyphs = index.entries().filter(|&(gid, _)| keep(gid)).map(|(gid, data)| Bitmap { gid, data }).collect();
    Some(SubTable { image_format: index.image_format, storage, glyphs })
}

// Copies a run of consecutive new glyphs' images into `cbdt`, refusing past `room`, in index format 1, or 5 for one
// size and metrics: the simplest formats that hold any run. Composites get their ids renumbered.
fn build_subtable(
    sub: &SubTable, run: &[(u16, &Bitmap)], gid_map: &[u32], cbdt: &mut Vec<u8>, room: usize,
) -> Option<Built> {
    let data_off = u32::try_from(cbdt.len()).ok()?;
    let mut copy = |b: &Bitmap| -> Option<()> {
        if cbdt.len().saturating_add(b.data.len()) > room { return None; }
        let at = cbdt.len();
        cbdt.extend_from_slice(b.data);
        for (k, c, ..) in eblc::components(sub.image_format, b.data)? {
            write_u16_be(cbdt, at + k, u16::try_from(*gid_map.get(usize::from(c))?).ok()?);
        }
        Some(())
    };
    let mut bytes = Vec::new();
    match &sub.storage {
        Storage::Variable => {
            push_u16(&mut bytes, 1);
            push_u16(&mut bytes, sub.image_format);
            push_u32(&mut bytes, data_off);
            let mut running = 0u32;
            for &(_, b) in run {
                push_u32(&mut bytes, running);
                running = running.checked_add(u32::try_from(b.data.len()).ok()?)?;
                copy(b)?;
            }
            push_u32(&mut bytes, running);
        }
        Storage::Fixed { image_size, metrics } => {
            push_u16(&mut bytes, 5);
            push_u16(&mut bytes, sub.image_format);
            push_u32(&mut bytes, data_off);
            push_u32(&mut bytes, *image_size);
            bytes.extend_from_slice(&metrics[..]);
            push_u32(&mut bytes, run.len() as u32);
            for &(new_gid, b) in run {
                push_u16(&mut bytes, new_gid);
                copy(b)?;
            }
            // The EBLC chapter has every subtable 32-bit aligned, so an odd glyph array pads.
            if run.len() % 2 == 1 { bytes.extend_from_slice(&[0, 0]); }
        }
    }
    Some(Built { bytes, first: run.first()?.0, last: run.last()?.0 })
}

// Where the blocks of images written lie, by their format and their glyphs' lengths, each with a hash of its bytes
// once another block of that shape comes: most blocks are alone in their shape and never hashed.
type Written = BTreeMap<(u16, Vec<usize>), Vec<(usize, Option<u64>)>>;

fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xCBF2_9CE4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01B3))
}

// A run's block of images, just written, given up for one written before in the same format, split the same between
// its glyphs and holding the same bytes, its subtable naming that one instead: the smaller table.
fn share(subtable: &mut Built, sub: &SubTable, run: &[(u16, &Bitmap)], cbdt: &mut Vec<u8>, written: &mut Written) {
    let Some(at) = read_u32_be(&subtable.bytes, 4).map(|at| at as usize).filter(|&at| at < cbdt.len()) else { return };
    let len = cbdt.len() - at;
    let lengths = run.iter().map(|(_, b)| b.data.len()).collect();
    let same = written.entry((sub.image_format, lengths)).or_default();
    if same.is_empty() {
        same.push((at, None));
        return;
    }
    let mine = fnv(&cbdt[at..]);
    for (before, seen) in same.iter_mut() {
        let block = cbdt.get(*before..*before + len).unwrap_or_default();
        if *seen.get_or_insert_with(|| fnv(block)) == mine && block == &cbdt[at..] {
            let before = *before;
            cbdt.truncate(at);
            write_u32_be(&mut subtable.bytes, 4, before as u32);
            return;
        }
    }
    same.push((at, Some(mine)));
}

struct Strike {
    record: Vec<u8>,
    blob: Vec<u8>,
    subtables: usize,
    lo: u16,
    hi: u16,
}

// The strikes of `cblc` and their images in `cbdt` for the glyphs kept, CBLC and EBLC alike: a subtable per run of
// consecutive glyphs, like image blocks shared; None where no strike keeps a glyph.
pub fn subset_bitmap_strikes(
    cblc: &[u8], cbdt: &[u8], active: &GlyphSet, gid_map: &[u32],
) -> Option<(Vec<u8>, Vec<u8>)> {
    let sizes = eblc::bitmap_sizes(cblc)?;
    let mut new_cbdt: Vec<u8> = cbdt.get(0..4)?.to_vec();
    let mut strikes: Vec<Strike> = Vec::new();
    // Strikes may share subtables, so one deciding its whole range is read and built once, empty or not.
    let mut built_from: BTreeMap<(usize, u16, u16), Vec<Built>> = BTreeMap::new();
    let source = cblc.len().saturating_add(cbdt.len());
    let (budget, mut steps) = (super::subset_budget(source), Steps::new(source));
    let (mut held, mut copied) = (0usize, 0usize);
    let mut written = BTreeMap::new();

    for size in sizes {
        let mut built: Vec<Built> = Vec::new();
        let mut grown = 0usize;
        let mut claims = eblc::Claims::default();
        for (first, last, ist) in eblc::index_records(cblc, size).iter().flat_map(eblc::IndexRecords::iter) {
            let decided = claims.claim(first, last);
            if decided.is_empty() { continue; }
            let whole = decided == [(first, last)];
            let from = built.len();
            if let Some(again) = built_from.get(&(ist, first, last)).filter(|_| whole) {
                built.extend(again.iter().cloned());
            } else {
                let keep = |gid: u16| active.contains(&gid) && decides(&decided, gid);
                let index = eblc::IndexSubtable::new(cblc, cbdt, ist, first, last);
                match index.and_then(|index| read_subtable(&index, keep, &mut steps)) {
                    Some(sub) => {
                        // The closure keeps every component, so one missing here is past the font's end, and a
                        // composite whose list the image cuts short draws nothing: either goes.
                        let drawable = |b: &&Bitmap| {
                            eblc::components(sub.image_format, b.data)
                                .is_some_and(|mut parts| parts.all(|(_, c, ..)| active.contains(&c)))
                        };
                        let mut sorted: Vec<(u16, &Bitmap)> = sub.glyphs.iter()
                            .filter(drawable)
                            .filter_map(|b| Some((u16::try_from(*gid_map.get(b.gid as usize)?).ok()?, b)))
                            .collect();
                        sorted.sort_unstable_by_key(|&(new_gid, _)| new_gid);
                        let room = budget.saturating_sub(held.saturating_add(grown));
                        for run in sorted.chunk_by(|a, b| a.0.checked_add(1) == Some(b.0)) {
                            let before = new_cbdt.len();
                            let mut subtable = build_subtable(&sub, run, gid_map, &mut new_cbdt, room)?;
                            // A shared block is still copied once, so overlapping images stop at the budget.
                            copied = copied.saturating_add(new_cbdt.len() - before);
                            if copied > budget { return None; }
                            share(&mut subtable, &sub, run, &mut new_cbdt, &mut written);
                            built.push(subtable);
                        }
                    }
                    None if steps.spent => return None,
                    None => {}
                }
                if whole { built_from.insert((ist, first, last), built[from..].to_vec()); }
            }
            grown = grown.saturating_add(built[from..].iter().map(|b| b.bytes.len() + 8).sum::<usize>());
            if held.saturating_add(grown).saturating_add(new_cbdt.len()) > budget { return None; }
        }
        held = held.saturating_add(grown);

        if built.is_empty() { continue; }
        let lo = built.iter().map(|b| b.first).min()?;
        let hi = built.iter().map(|b| b.last).max()?;

        let mut blob = vec![0u8; built.len() * 8];
        for (j, b) in built.iter().enumerate() {
            let at = blob.len() as u32;
            write_u16_be(&mut blob, j * 8, b.first);
            write_u16_be(&mut blob, j * 8 + 2, b.last);
            write_u32_be(&mut blob, j * 8 + 4, at);
            blob.extend_from_slice(&b.bytes);
        }
        strikes.push(Strike {
            record: size.to_vec(),
            blob,
            subtables: built.len(),
            lo,
            hi,
        });
    }

    if strikes.is_empty() { return None; }

    let mut new_cblc = vec![0u8; 8 + strikes.len() * 48];
    new_cblc[0..4].copy_from_slice(cblc.get(0..4)?);
    write_u32_be(&mut new_cblc, 4, strikes.len() as u32);
    let mut placed = Vec::with_capacity(strikes.len());
    for (i, Strike { record, blob, subtables, lo, hi }) in strikes.iter().enumerate() {
        let at = 8 + i * 48;
        new_cblc[at..at + 48].copy_from_slice(record);
        // A strike whose subtables come out as another's names that one's.
        let blob_at = match strikes[..i].iter().position(|earlier| earlier.blob == *blob) {
            Some(earlier) => placed[earlier],
            None => {
                new_cblc.extend_from_slice(blob);
                (new_cblc.len() - blob.len()) as u32
            }
        };
        placed.push(blob_at);
        write_u32_be(&mut new_cblc, at, blob_at);
        write_u32_be(&mut new_cblc, at + 4, blob.len() as u32);
        write_u32_be(&mut new_cblc, at + 8, *subtables as u32);
        write_u16_be(&mut new_cblc, at + 40, *lo);
        write_u16_be(&mut new_cblc, at + 42, *hi);
    }

    Some((new_cblc, new_cbdt))
}

// A table's sbix strikes, each once however often it is named.
fn sbix_strikes(sbix: &[u8]) -> Option<impl Iterator<Item = usize> + '_> {
    let mut seen = BTreeSet::new();
    Some(sbix::strikes(sbix)?.filter(move |&strike| seen.insert(strike)))
}

// The sbix graphic types whose data is a glyph id: 'dupe' draws that glyph's image, and Apple's
// 'flip' draws it mirrored.
fn names_a_glyph(record: &[u8]) -> Option<u16> {
    matches!(record.get(4..8), Some(b"dupe" | b"flip")).then(|| read_u16_be(record, 8)).flatten()
}

pub fn subset_sbix(
    sbix: &[u8],
    num_glyphs: usize,
    active_sorted: &[u32],
    gid_map: &[u32],
) -> Option<Vec<u8>> {
    let active: GlyphSet = active_sorted.iter().copied().collect();
    let budget = super::subset_budget(sbix.len());
    let data_at = 4 + (active_sorted.len() + 1) * 4;

    // Each strike's walk is charged whether it keeps a glyph or not, and each record before it is
    // copied, so strikes and records that overlap stop at the budget.
    let mut strikes: Vec<Vec<u8>> = Vec::new();
    let mut held = 0usize;
    for strike in sbix_strikes(sbix)? {
        held = held.saturating_add(data_at);
        if held > budget { return None; }
        let mut out = sbix.get(strike..strike + 4)?.to_vec();
        let mut offsets: Vec<u32> = Vec::with_capacity(active_sorted.len() + 1);
        let mut data: Vec<u8> = Vec::new();
        let mut kept = 0usize;

        for &orig in active_sorted {
            offsets.push(u32::try_from(data_at + data.len()).ok()?);
            let Some(record) = sbix::record(sbix, strike, num_glyphs, orig) else { continue };
            held = held.saturating_add(record.len());
            if held > budget { return None; }
            match names_a_glyph(record) {
                Some(target) => {
                    let Some(new_target) = remap_gid16(&active, gid_map, target) else { continue };
                    data.extend_from_slice(record.get(0..8)?);
                    data.extend_from_slice(&new_target.to_be_bytes());
                }
                None => data.extend_from_slice(record),
            }
            kept += 1;
        }
        offsets.push(u32::try_from(data_at + data.len()).ok()?);
        if kept == 0 { continue; }

        for o in &offsets { out.extend_from_slice(&o.to_be_bytes()); }
        out.extend_from_slice(&data);
        strikes.push(out);
    }
    if strikes.is_empty() { return None; }

    let mut out = sbix.get(0..4)?.to_vec();
    push_u32(&mut out, strikes.len() as u32);
    let mut at = 8 + strikes.len() * 4;
    for st in &strikes {
        push_u32(&mut out, u32::try_from(at).ok()?);
        at += st.len();
    }
    for st in &strikes { out.extend_from_slice(st); }
    Some(out)
}

// Glyphs a kept bitmap draws on: sbix 'dupe' and 'flip' targets and EBDT composite components. A
// walk the subset would refuse stops here too, since that table is dropped anyway.
pub fn bitmap_closure<'a>(
    table: impl Fn(&str) -> Option<&'a [u8]>, num_glyphs: usize, active: &GlyphSet,
) -> Vec<u16> {
    let mut found = Vec::new();
    if let Some(sbix) = table("sbix") {
        sbix_targets(sbix, num_glyphs, active, &mut found);
    }
    for (data, loc) in [("CBDT", "CBLC"), ("EBDT", "EBLC"), ("bdat", "bloc")] {
        if let (Some(data), Some(loc)) = (table(data), table(loc)) {
            composite_components(loc, data, active, &mut found);
        }
    }
    found
}

fn sbix_targets(sbix: &[u8], num_glyphs: usize, active: &GlyphSet, found: &mut Vec<u16>) -> Option<()> {
    let walk = 4 + (active.len() + 1) * 4;
    let (budget, mut held) = (super::subset_budget(sbix.len()), 0usize);
    for strike in sbix_strikes(sbix)? {
        held = held.saturating_add(walk);
        if held > budget { return None; }
        found.extend(active.iter().filter_map(|g| names_a_glyph(sbix::record(sbix, strike, num_glyphs, g)?)));
    }
    Some(())
}

fn composite_components(cblc: &[u8], cbdt: &[u8], active: &GlyphSet, found: &mut Vec<u16>) -> Option<()> {
    let sizes = eblc::bitmap_sizes(cblc)?;
    let mut steps = Steps::new(cblc.len().saturating_add(cbdt.len()));
    // A subtable read for its whole range holds every glyph a later reading of it could.
    let mut read_whole = BTreeSet::new();
    for size in sizes {
        let mut claims = eblc::Claims::default();
        for (first, last, ist) in eblc::index_records(cblc, size).iter().flat_map(eblc::IndexRecords::iter) {
            let decided = claims.claim(first, last);
            if decided.is_empty() || read_whole.contains(&(ist, first, last)) {
                continue;
            }
            let index = eblc::IndexSubtable::new(cblc, cbdt, ist, first, last);
            let Some(index) = index.filter(|index| matches!(index.image_format, 8 | 9)) else { continue };
            if decided == [(first, last)] {
                read_whole.insert((ist, first, last));
            }
            let keep = |gid: u16| active.contains(&gid) && decides(&decided, gid);
            let Some(sub) = read_subtable(&index, keep, &mut steps) else {
                if steps.spent { return None; }
                continue;
            };
            for b in &sub.glyphs {
                found.extend(eblc::components(sub.image_format, b.data).into_iter().flatten().map(|(_, c, ..)| c));
            }
        }
    }
    Some(())
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/bitmap.rs"]
mod tests;
