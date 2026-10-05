use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use alloc::vec;
use super::super::decoder::{read_u16_be, read_u32_be, write_u16_be, write_u32_be, records_fit};

struct Bitmap<'a> {
    gid: u16,
    data: &'a [u8],
}

enum Storage {
    Variable,
    Fixed { image_size: u32, metrics: Vec<u8> },
}

struct SubTable<'a> {
    image_format: u16,
    storage: Storage,
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

// One strike's index records as (subtable offset, first glyph, last glyph), none if they overrun.
fn index_records(cblc: &[u8], st: usize) -> impl Iterator<Item = (usize, u16, u16)> + '_ {
    let (array, n) = match (read_u32_be(cblc, st), read_u32_be(cblc, st + 8)) {
        (Some(a), Some(n)) if records_fit(a as usize, n as usize, 8, cblc.len()) => (a as usize, n as usize),
        _ => (0, 0),
    };
    (0..n).filter_map(move |j| {
        let rec = array + j * 8;
        let (first, last, add) = (read_u16_be(cblc, rec)?, read_u16_be(cblc, rec + 2)?, read_u32_be(cblc, rec + 4)?);
        Some((array.checked_add(add as usize)?, first, last))
    })
}

// A composite image (EBDT image formats 8 and 9) names its components by glyph id: each id with
// where it sits in the image.
fn components(image_format: u16, data: &[u8]) -> impl Iterator<Item = (usize, u16)> + '_ {
    let records = match image_format { 8 => 8, 9 => 10, _ => 0 };
    let count = if records == 0 { 0 } else { read_u16_be(data, records - 2).map_or(0, usize::from) };
    (0..count).map_while(move |k| Some((records + k * 4, read_u16_be(data, records + k * 4)?)))
}

fn read_subtable<'a>(
    cblc: &[u8],
    cbdt: &'a [u8],
    ist: usize,
    first: u16,
    last: u16,
    active: &GlyphSet,
    steps: &mut Steps,
) -> Option<SubTable<'a>> {
    let index_format = read_u16_be(cblc, ist)?;
    let image_format = read_u16_be(cblc, ist + 2)?;
    let data_off = read_u32_be(cblc, ist + 4)? as usize;
    if last < first { return None; }
    let span = (last - first) as usize + 1;

    let mut glyphs = Vec::new();
    let mut take = |gid: u16, s: usize, e: usize| -> Option<()> {
        if e <= s { return Some(()); }
        let data = cbdt.get(data_off.checked_add(s)?..data_off.checked_add(e)?)?;
        glyphs.push(Bitmap { gid, data });
        Some(())
    };

    let storage = match index_format {
        1 | 3 => {
            let width = if index_format == 1 { 4 } else { 2 };
            if !records_fit(ist + 8, span + 1, width, cblc.len()) { return None; }
            steps.take(span)?;
            let at = |i: usize| -> Option<usize> {
                if index_format == 1 { read_u32_be(cblc, ist + 8 + i * 4).map(|v| v as usize) }
                else { read_u16_be(cblc, ist + 8 + i * 2).map(|v| v as usize) }
            };
            for i in 0..span {
                let gid = first + i as u16;
                if !active.contains(&gid) { continue; }
                take(gid, at(i)?, at(i + 1)?)?;
            }
            Storage::Variable
        }
        2 => {
            let image_size = read_u32_be(cblc, ist + 8)? as usize;
            let metrics = cblc.get(ist + 12..ist + 20)?.to_vec();
            steps.take(span)?;
            for i in 0..span {
                let gid = first + i as u16;
                if !active.contains(&gid) { continue; }
                let s = i.checked_mul(image_size)?;
                take(gid, s, s.checked_add(image_size)?)?;
            }
            Storage::Fixed { image_size: image_size as u32, metrics }
        }
        4 => {
            let n = read_u32_be(cblc, ist + 8)? as usize;
            if !records_fit(ist + 12, n + 1, 4, cblc.len()) { return None; }
            steps.take(n)?;
            for i in 0..n {
                let gid = read_u16_be(cblc, ist + 12 + i * 4)?;
                if !active.contains(&gid) { continue; }
                let s = read_u16_be(cblc, ist + 12 + i * 4 + 2)? as usize;
                let e = read_u16_be(cblc, ist + 12 + (i + 1) * 4 + 2)? as usize;
                take(gid, s, e)?;
            }
            Storage::Variable
        }
        5 => {
            let image_size = read_u32_be(cblc, ist + 8)? as usize;
            let metrics = cblc.get(ist + 12..ist + 20)?.to_vec();
            let n = read_u32_be(cblc, ist + 20)? as usize;
            if !records_fit(ist + 24, n, 2, cblc.len()) { return None; }
            steps.take(n)?;
            for i in 0..n {
                let gid = read_u16_be(cblc, ist + 24 + i * 2)?;
                if !active.contains(&gid) { continue; }
                let s = i.checked_mul(image_size)?;
                take(gid, s, s.checked_add(image_size)?)?;
            }
            Storage::Fixed { image_size: image_size as u32, metrics }
        }
        _ => return None,
    };
    Some(SubTable { image_format, storage, glyphs })
}

// Copies a run of consecutive new glyphs' images into `cbdt`, refusing once it would pass `room`.
// Composite images get their component ids renumbered.
fn build_subtable(
    sub: &SubTable, run: &[(u16, &Bitmap)], gid_map: &[u16], cbdt: &mut Vec<u8>, room: usize,
) -> Option<Built> {
    let data_off = u32::try_from(cbdt.len()).ok()?;
    let mut copy = |b: &Bitmap| -> Option<()> {
        if cbdt.len().saturating_add(b.data.len()) > room { return None; }
        let at = cbdt.len();
        cbdt.extend_from_slice(b.data);
        for (k, c) in components(sub.image_format, b.data) {
            write_u16_be(cbdt, at + k, *gid_map.get(usize::from(c))?);
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
            bytes.extend_from_slice(metrics);
            push_u32(&mut bytes, run.len() as u32);
            for &(new_gid, b) in run {
                push_u16(&mut bytes, new_gid);
                copy(b)?;
            }
            if run.len() % 2 == 1 { bytes.extend_from_slice(&[0, 0]); }
        }
    }
    Some(Built { bytes, first: run.first()?.0, last: run.last()?.0 })
}

struct Strike {
    record: Vec<u8>,
    blob: Vec<u8>,
    subtables: usize,
    lo: u16,
    hi: u16,
}

pub fn subset_bitmap_strikes(
    cblc: &[u8],
    cbdt: &[u8],
    active: &GlyphSet,
    gid_map: &[u16],
) -> Option<(Vec<u8>, Vec<u8>)> {
    let num_sizes = read_u32_be(cblc, 4)? as usize;
    if !records_fit(8, num_sizes, 48, cblc.len()) { return None; }

    let mut new_cbdt: Vec<u8> = cbdt.get(0..4)?.to_vec();
    let mut strikes: Vec<Strike> = Vec::new();
    // Records may name one subtable again and again, so each is read and built once, empty or not.
    let mut built_from: BTreeMap<(usize, u16, u16), Vec<Built>> = BTreeMap::new();
    let source = cblc.len().saturating_add(cbdt.len());
    let (budget, mut steps) = (super::subset_budget(source), Steps::new(source));
    let mut held = 0usize;

    for i in 0..num_sizes {
        let st = 8 + i * 48;
        let mut built: Vec<Built> = Vec::new();
        let mut grown = 0usize;
        for (ist, first, last) in index_records(cblc, st) {
            let from = built.len();
            if let Some(again) = built_from.get(&(ist, first, last)) {
                built.extend(again.iter().cloned());
            } else {
                match read_subtable(cblc, cbdt, ist, first, last, active, &mut steps) {
                    Some(sub) => {
                        // The closure keeps every component, so one missing here is past the font's end; the composite goes.
                        let mut sorted: Vec<(u16, &Bitmap)> = sub.glyphs.iter()
                            .filter(|b| components(sub.image_format, b.data).all(|(_, c)| active.contains(&c)))
                            .filter_map(|b| Some((*gid_map.get(usize::from(b.gid))?, b)))
                            .collect();
                        sorted.sort_unstable_by_key(|&(new_gid, _)| new_gid);
                        let room = budget.saturating_sub(held.saturating_add(grown));
                        for run in sorted.chunk_by(|a, b| a.0.checked_add(1) == Some(b.0)) {
                            built.push(build_subtable(&sub, run, gid_map, &mut new_cbdt, room)?);
                        }
                    }
                    None if steps.spent => return None,
                    None => {}
                }
                built_from.insert((ist, first, last), built[from..].to_vec());
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
            record: cblc.get(st..st + 48)?.to_vec(),
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
    for (i, Strike { record, blob, subtables, lo, hi }) in strikes.iter().enumerate() {
        let at = 8 + i * 48;
        new_cblc[at..at + 48].copy_from_slice(record);
        let blob_at = new_cblc.len() as u32;
        write_u32_be(&mut new_cblc, at, blob_at);
        write_u32_be(&mut new_cblc, at + 4, blob.len() as u32);
        write_u32_be(&mut new_cblc, at + 8, *subtables as u32);
        write_u16_be(&mut new_cblc, at + 40, *lo);
        write_u16_be(&mut new_cblc, at + 42, *hi);
        new_cblc.extend_from_slice(blob);
    }

    Some((new_cblc, new_cbdt))
}

// The sbix graphic types whose data is a glyph id: 'dupe' draws that glyph's image, and Apple's
// 'flip' draws it mirrored.
fn names_a_glyph(record: &[u8]) -> Option<u16> {
    matches!(record.get(4..8), Some(b"dupe" | b"flip")).then(|| read_u16_be(record, 8)).flatten()
}

// The strikes whose glyph offsets fit, each listed once however often it is named.
fn sbix_strikes(sbix: &[u8], num_glyphs: usize) -> Option<Vec<usize>> {
    let num_strikes = read_u32_be(sbix, 4)? as usize;
    if !records_fit(8, num_strikes, 4, sbix.len()) { return None; }
    let mut seen = BTreeSet::new();
    Some((0..num_strikes)
        .filter_map(|i| read_u32_be(sbix, 8 + i * 4).map(|v| v as usize))
        .filter(|&strike| records_fit(strike + 4, num_glyphs + 1, 4, sbix.len()) && seen.insert(strike))
        .collect())
}

fn sbix_record(sbix: &[u8], strike: usize, num_glyphs: usize, gid: u16) -> Option<&[u8]> {
    let g = usize::from(gid);
    if g >= num_glyphs { return None; }
    let (s, e) = (read_u32_be(sbix, strike + 4 + g * 4)? as usize, read_u32_be(sbix, strike + 8 + g * 4)? as usize);
    if e <= s { return None; }
    sbix.get(strike.checked_add(s)?..strike.checked_add(e)?)
}

pub fn subset_sbix(
    sbix: &[u8],
    num_glyphs: usize,
    active_sorted: &[u16],
    gid_map: &[u16],
) -> Option<Vec<u8>> {
    let active: GlyphSet = active_sorted.iter().copied().collect();
    let budget = super::subset_budget(sbix.len());
    let data_at = 4 + (active_sorted.len() + 1) * 4;

    // Each strike's walk is charged whether it keeps a glyph or not, and each record before it is
    // copied, so strikes and records that overlap stop at the budget.
    let mut strikes: Vec<Vec<u8>> = Vec::new();
    let mut held = 0usize;
    for strike in sbix_strikes(sbix, num_glyphs)? {
        held = held.saturating_add(data_at);
        if held > budget { return None; }
        let mut out = sbix.get(strike..strike + 4)?.to_vec();
        let mut offsets: Vec<u32> = Vec::with_capacity(active_sorted.len() + 1);
        let mut data: Vec<u8> = Vec::new();
        let mut kept = 0usize;

        for &orig in active_sorted {
            offsets.push(u32::try_from(data_at + data.len()).ok()?);
            let Some(record) = sbix_record(sbix, strike, num_glyphs, orig) else { continue };
            held = held.saturating_add(record.len());
            if held > budget { return None; }
            match names_a_glyph(record) {
                Some(target) => {
                    let Some(&new_target) = gid_map.get(usize::from(target)).filter(|_| active.contains(&target))
                    else { continue };
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
    for strike in sbix_strikes(sbix, num_glyphs)? {
        held = held.saturating_add(walk);
        if held > budget { return None; }
        found.extend(active.iter().filter_map(|g| names_a_glyph(sbix_record(sbix, strike, num_glyphs, g)?)));
    }
    Some(())
}

fn composite_components(cblc: &[u8], cbdt: &[u8], active: &GlyphSet, found: &mut Vec<u16>) -> Option<()> {
    let num_sizes = read_u32_be(cblc, 4)? as usize;
    if !records_fit(8, num_sizes, 48, cblc.len()) { return None; }
    let mut steps = Steps::new(cblc.len().saturating_add(cbdt.len()));
    let mut seen = BTreeSet::new();
    for i in 0..num_sizes {
        for (ist, first, last) in index_records(cblc, 8 + i * 48) {
            if !matches!(read_u16_be(cblc, ist + 2), Some(8 | 9)) || !seen.insert((ist, first, last)) { continue; }
            let Some(sub) = read_subtable(cblc, cbdt, ist, first, last, active, &mut steps) else {
                if steps.spent { return None; }
                continue;
            };
            for b in &sub.glyphs {
                found.extend(components(sub.image_format, b.data).map(|(_, c)| c));
            }
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // One strike whose 2,000 index records all name one subtable of one 1,000-byte image.
    #[test]
    fn an_image_named_many_times_is_copied_once() {
        const N: usize = 2_000;
        let mut cblc = [0x0003_0000u32, 1, 56, 0, N as u32, 0].map(u32::to_be_bytes).concat();
        cblc.resize(56, 0);
        for _ in 0..N {
            cblc.extend([1u16, 1].map(u16::to_be_bytes).concat());
            cblc.extend(((N * 8) as u32).to_be_bytes());
        }
        cblc.extend([1u16, 17].map(u16::to_be_bytes).concat());
        cblc.extend([4u32, 0, 1_000].map(u32::to_be_bytes).concat());
        let mut cbdt = 0x0003_0000u32.to_be_bytes().to_vec();
        cbdt.resize(4 + 1_000, 7);
        let mut active = GlyphSet::new();
        active.insert(1);

        let (_, data) = subset_bitmap_strikes(&cblc, &cbdt, &active, &[0, 1]).expect("a subset strike");
        assert!(data.len() <= cbdt.len(), "one image named {N} times took {} bytes", data.len());
    }

    // 2,000 strike offsets naming one strike of one 1,000-byte image.
    #[test]
    fn a_strike_named_many_times_is_kept_once() {
        const N: usize = 2_000;
        let strike = 8 + 4 * N;
        let mut sbix = [1u16, 1].map(u16::to_be_bytes).concat();
        sbix.extend((N as u32).to_be_bytes());
        (0..N).for_each(|_| sbix.extend((strike as u32).to_be_bytes()));
        sbix.extend([72u16, 72].map(u16::to_be_bytes).concat());
        sbix.extend([16u32, 16, 16 + 8 + 1_000].map(u32::to_be_bytes).concat());
        sbix.extend([0u16, 0].map(u16::to_be_bytes).concat());
        sbix.extend(b"png ");
        sbix.extend([7u8; 1_000]);

        let out = subset_sbix(&sbix, 2, &[0, 1], &[0, 1]).expect("a subset sbix");
        assert!(out.len() <= sbix.len(), "one strike named {N} times took {} bytes", out.len());
    }

    // Images are copied once, but 2,000 records of one 5,000-glyph subtable still name 40 MB of index.
    #[test]
    fn a_subtable_named_past_the_budget_is_refused() {
        const N: usize = 2_000;
        const GLYPHS: u16 = 5_000;
        let mut cblc = [0x0003_0000u32, 1, 56, 0, N as u32, 0].map(u32::to_be_bytes).concat();
        cblc.resize(56, 0);
        for _ in 0..N {
            cblc.extend([1u16, GLYPHS].map(u16::to_be_bytes).concat());
            cblc.extend(((N * 8) as u32).to_be_bytes());
        }
        cblc.extend([1u16, 17].map(u16::to_be_bytes).concat());
        cblc.extend(4u32.to_be_bytes());
        cblc.extend((0..=u32::from(GLYPHS)).flat_map(u32::to_be_bytes));
        let mut cbdt = 0x0003_0000u32.to_be_bytes().to_vec();
        cbdt.resize(4 + usize::from(GLYPHS), 7);
        let mut active = GlyphSet::new();
        (1..=GLYPHS).for_each(|g| { active.insert(g); });
        let gid_map: Vec<u16> = (0..=GLYPHS).collect();

        assert!(subset_bitmap_strikes(&cblc, &cbdt, &active, &gid_map).is_none(), "40 MB of index was built");
    }

    // A CBLC of `strikes` strikes all naming one array of these records (first, last, the
    // subtable's bytes). Bodies are laid end to end, so a record with none names the next one.
    fn cblc(strikes: usize, records: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
        let array = 8 + strikes * 48;
        let mut out = [0x0003_0000u32, strikes as u32].map(u32::to_be_bytes).concat();
        for _ in 0..strikes {
            out.extend([array as u32, 0, records.len() as u32].map(u32::to_be_bytes).concat());
            out.extend([0; 28]);
            out.extend([0, 0, 0xFF, 0xFF, 16, 16, 32, 1]);
        }
        let mut at = records.len() * 8;
        for (first, last, body) in records {
            out.extend([first.to_be_bytes(), last.to_be_bytes()].concat());
            out.extend((at as u32).to_be_bytes());
            at += body.len();
        }
        records.iter().for_each(|r| out.extend(&r.2));
        out
    }

    fn format2_empty() -> Vec<u8> {
        [&[0, 2, 0, 17, 0, 0, 0, 4][..], &[0; 12]].concat()
    }

    fn format1(image_format: u16, len: u32) -> Vec<u8> {
        [&[0, 1][..], &image_format.to_be_bytes(), &[0, 0, 0, 4], &0u32.to_be_bytes(), &len.to_be_bytes()].concat()
    }

    // A real font walks each glyph of a strike once. These 200 records name one empty subtable of
    // about 65,535 glyphs, a different last glyph each, so the cache cannot help and the walk is refused.
    #[test]
    fn index_walks_past_their_steps_are_refused() {
        let mut records: Vec<(u16, u16, Vec<u8>)> = (1..200).map(|j| (1, 65_535 - j, Vec::new())).collect();
        records.push((1, 65_535, format2_empty()));
        records.push((1, 1, format1(17, 4)));
        let cbdt = [0, 3, 0, 0, 1, 2, 3, 4];
        let mut active = GlyphSet::new();
        active.insert(1);
        assert!(subset_bitmap_strikes(&cblc(1, &records), &cbdt, &active, &[0, 1]).is_none());
    }

    // 50 strikes of 50 records naming one empty 65,535-glyph subtable: walked once, it fits the steps.
    #[test]
    fn an_empty_subtable_named_many_times_is_walked_once() {
        let mut records: Vec<(u16, u16, Vec<u8>)> = (0..49).map(|_| (1, 65_535, Vec::new())).collect();
        records.push((1, 65_535, format2_empty()));
        records.push((1, 1, format1(17, 4)));
        let table = cblc(50, &records);
        let mut active = GlyphSet::new();
        active.insert(1);
        let (_, data) = subset_bitmap_strikes(&table, &[0, 3, 0, 0, 1, 2, 3, 4], &active, &[0, 1]).expect("a subset");
        assert_eq!(data, [0, 3, 0, 0, 1, 2, 3, 4]);
    }

    // An EBDT composite (image format 8) naming glyph 9, past this three-glyph font, has nothing to
    // draw and is dropped; the rest of the strike stays.
    #[test]
    fn a_composite_naming_a_missing_glyph_is_dropped() {
        let simple = [2u8, 3, 0, 2, 3, 0b1110_0000, 0b1010_0000];
        let composite = [&[2u8, 3, 0, 2, 3, 0, 0, 1][..], &9u16.to_be_bytes(), &[0, 0]].concat();
        let cbdt = [&[0, 2, 0, 0][..], &simple, &composite].concat();
        let second = [&[0, 1, 0, 8][..], &(4 + simple.len() as u32).to_be_bytes(), &0u32.to_be_bytes(), &(composite.len() as u32).to_be_bytes()].concat();
        let table = cblc(1, &[(1, 1, format1(1, simple.len() as u32)), (2, 2, second)]);
        let mut active = GlyphSet::new();
        [0, 1, 2].iter().for_each(|&g| { active.insert(g); });
        let (loc, data) = subset_bitmap_strikes(&table, &cbdt, &active, &[0, 1, 2]).expect("the simple glyph stays");
        assert_eq!(read_u32_be(&loc, 16), Some(1), "the composite kept its index subtable");
        assert_eq!(data, [&[0, 2, 0, 0][..], &simple].concat());
    }

    // Strikes overlapping one window of zero offsets keep nothing, but each walks every glyph. Charged,
    // 100 of them pass the budget before the one real strike is reached.
    #[test]
    fn strikes_that_keep_nothing_still_spend_the_budget() {
        const GLYPHS: usize = 1_000;
        const SHIFTED: usize = 100;
        let strikes = 8 + (SHIFTED + 1) * 4;
        let window = 4 + 4 * (GLYPHS + 1) + SHIFTED;
        let real = strikes + window;
        let mut sbix = [1u16, 1].map(u16::to_be_bytes).concat();
        sbix.extend(((SHIFTED + 1) as u32).to_be_bytes());
        (0..SHIFTED).for_each(|i| sbix.extend(((strikes + i) as u32).to_be_bytes()));
        sbix.extend((real as u32).to_be_bytes());
        sbix.resize(real, 0);
        let head = 4 + 4 * (GLYPHS + 1);
        sbix.extend([72u16, 72].map(u16::to_be_bytes).concat());
        (0..=GLYPHS).for_each(|g| sbix.extend(((head + if g < 2 { 0 } else { 12 }) as u32).to_be_bytes()));
        sbix.extend([0, 0, 0, 0]);
        sbix.extend(b"png ");
        sbix.extend([7; 4]);
        let active: Vec<u16> = (0..GLYPHS as u16).collect();
        assert!(subset_sbix(&sbix, GLYPHS, &active, &active).is_none());
    }

    // Glyph 5 is past this two-glyph font. Read anyway, its offsets would land on glyph 1's record.
    #[test]
    fn a_glyph_past_the_font_reads_no_record() {
        let record = [&[0u8, 0, 0, 0][..], b"png ", &[7; 4]].concat();
        let mut sbix = [&[0u8, 1, 0, 1][..], &1u32.to_be_bytes(), &12u32.to_be_bytes(), &[0, 72, 0, 72]].concat();
        [32u32, 32, 32 + 12, 0, 0, 32, 32 + 12].iter().for_each(|o| sbix.extend(o.to_be_bytes()));
        sbix.resize(12 + 32, 0);
        sbix.extend(&record);
        let out = subset_sbix(&sbix, 2, &[0, 1, 5], &[0, 1, 0, 0, 0, 2]).expect("glyph 1 is kept");
        let offset = |g: usize| read_u32_be(&out, 12 + 4 + g * 4);
        assert_eq!(offset(2), offset(3), "glyph 5 took a record");
        assert_eq!(out.len(), 12 + 4 + 4 * 4 + record.len());
    }

    // One strike: per glyph, its whole record or none.
    fn sbix_of(glyphs: &[Option<Vec<u8>>]) -> Vec<u8> {
        let head = 4 + 4 * (glyphs.len() + 1);
        let mut out = [&[0u8, 1, 0, 1][..], &1u32.to_be_bytes(), &12u32.to_be_bytes(), &[0, 16, 0, 72]].concat();
        let mut data = Vec::new();
        for g in glyphs {
            out.extend(((head + data.len()) as u32).to_be_bytes());
            data.extend(g.iter().flatten());
        }
        out.extend(((head + data.len()) as u32).to_be_bytes());
        [out, data].concat()
    }

    // Glyph 4 is a 'dupe' of 3 and glyph 5 a 'flip' of 2, and dropping glyph 1 shifts every id down.
    #[test]
    fn dupes_and_flips_name_their_targets_new_ids() {
        let record = |kind: &[u8; 4], data: &[u8]| Some([&[0u8, 0, 0, 0][..], kind, data].concat());
        let sbix = sbix_of(&[
            None,
            record(b"png ", &[1; 4]),
            record(b"png ", &[2; 4]),
            record(b"png ", &[3; 4]),
            record(b"dupe", &3u16.to_be_bytes()),
            record(b"flip", &2u16.to_be_bytes()),
        ]);
        let out = subset_sbix(&sbix, 6, &[0, 2, 3, 4, 5], &[0, 0, 1, 2, 3, 4]).expect("a subset");
        let named = |g: usize| {
            let at = 12 + read_u32_be(&out, 16 + g * 4).expect("an offset") as usize;
            (&out[at + 4..at + 8], read_u16_be(&out, at + 8))
        };
        assert_eq!(named(3), (&b"dupe"[..], Some(2)));
        assert_eq!(named(4), (&b"flip"[..], Some(1)));
    }

    // Glyph 5 is a composite (EBDT image format 8) of glyphs 3 and 1, which become 2 and 1.
    #[test]
    fn a_composite_names_its_components_by_their_new_ids() {
        let simple = [2u8, 3, 0, 2, 3, 0b1110_0000, 0b1010_0000];
        let composite = [&[2u8, 3, 0, 2, 3, 0, 0, 2][..], &3u16.to_be_bytes(), &[0, 0], &1u16.to_be_bytes(), &[1, 0]].concat();
        let cbdt = [&[0, 2, 0, 0][..], &simple, &simple, &composite].concat();
        let index = |n: usize, len: usize, format: u16| {
            let from = (4 + n * simple.len()) as u32;
            [&[0u8, 1][..], &format.to_be_bytes(), &from.to_be_bytes(), &0u32.to_be_bytes(), &(len as u32).to_be_bytes()].concat()
        };
        let table = cblc(1, &[(1, 1, index(0, simple.len(), 1)), (3, 3, index(1, simple.len(), 1)), (5, 5, index(2, composite.len(), 8))]);
        let mut active = GlyphSet::new();
        [0, 1, 3, 5].iter().for_each(|&g| { active.insert(g); });
        let (_, data) = subset_bitmap_strikes(&table, &cbdt, &active, &[0, 1, 0, 2, 0, 3]).expect("a subset");
        let image = &data[data.len() - composite.len()..];
        assert_eq!((read_u16_be(image, 8), read_u16_be(image, 12)), (Some(2), Some(1)));
    }
}
