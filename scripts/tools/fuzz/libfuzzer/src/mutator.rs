//! Mutations that keep a font a font: inside one table with its directory rebuilt around it, whole tables moved,
//! dropped or swapped between fonts, the choices and text on their own, and raw bytes still, so the directory's own
//! reading stays fuzzed.

use daegun_fuzz_harness::{sections, CHOICES};
use libfuzzer_sys::fuzzer_mutate;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { self.next() as usize % n }
    }
}

// What a font file starts with: such an input is all font, as the harness reads it.
fn plain_font(data: &[u8]) -> bool {
    [b"\0\x01\0\0", b"OTTO", b"true", b"ttcf"].iter().any(|m| data.starts_with(*m))
}

fn u16_at(d: &[u8], at: usize) -> Option<usize> {
    Some(usize::from(u16::from_be_bytes(d.get(at..at + 2)?.try_into().ok()?)))
}

fn u32_at(d: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?) as usize)
}

/// A font's tables in directory order, each its tag and bytes; None for a collection, a directory past the data, or
/// tables of more than `room` bytes in all, which no rebuild fits: records sharing one span are each a copy of it.
fn tables(font: &[u8], room: usize) -> Option<(u32, Vec<([u8; 4], Vec<u8>)>)> {
    if font.starts_with(b"ttcf") {
        return None;
    }
    let version = u32_at(font, 0)? as u32;
    let n = u16_at(font, 4)?;
    let (mut out, mut total) = (Vec::with_capacity(n), 0usize);
    for i in 0..n {
        let record = 12 + 16 * i;
        let tag: [u8; 4] = font.get(record..record + 4)?.try_into().ok()?;
        let (offset, len) = (u32_at(font, record + 8)?, u32_at(font, record + 12)?);
        let bytes = font.get(offset..offset.checked_add(len)?).unwrap_or_default();
        total += bytes.len();
        if total > room {
            return None;
        }
        out.push((tag, bytes.to_vec()));
    }
    Some((version, out))
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, c| {
        let mut word = [0; 4];
        word[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// A font written from its tables in the order given, each on a four-byte boundary with its checksum.
fn build(version: u32, tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let n = tables.len();
    let power = if n == 0 { 0 } else { 1usize << n.ilog2() };
    let mut out = Vec::new();
    out.extend_from_slice(&version.to_be_bytes());
    for v in [n, power * 16, power.max(1).ilog2() as usize, n * 16 - power * 16] {
        out.extend_from_slice(&(v as u16).to_be_bytes());
    }
    let mut at = 12 + 16 * n;
    let mut records = Vec::new();
    for (tag, bytes) in tables {
        records.extend_from_slice(tag);
        records.extend_from_slice(&checksum(bytes).to_be_bytes());
        records.extend_from_slice(&(at as u32).to_be_bytes());
        records.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        at += bytes.len().next_multiple_of(4);
    }
    out.extend_from_slice(&records);
    for (_, bytes) in tables {
        out.extend_from_slice(bytes);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

/// `bytes` mutated by libFuzzer within `room` more, or as they are where neither leaves a byte, which libFuzzer
/// asserts against.
fn mutated(mut bytes: Vec<u8>, room: usize) -> Vec<u8> {
    let len = bytes.len();
    if len + room == 0 {
        return bytes;
    }
    bytes.resize(len + room, 0);
    let new = fuzzer_mutate(&mut bytes, len, len + room);
    bytes.truncate(new);
    bytes
}

/// An input put back together: choices, text and font in the harness's layout, or the font alone where it was.
fn join(head: Option<(&[u8], &[u8])>, font: &[u8]) -> Vec<u8> {
    let Some((choices, text)) = head else { return font.to_vec() };
    let mut out = Vec::with_capacity(CHOICES + 2 + text.len() + font.len());
    out.extend_from_slice(choices);
    out.resize(CHOICES, 0);
    out.extend_from_slice(&(text.len().min(u16::MAX.into()) as u16).to_be_bytes());
    out.extend_from_slice(&text[..text.len().min(u16::MAX.into())]);
    out.extend_from_slice(font);
    out
}

pub fn mutate(data: &mut [u8], size: usize, max_size: usize, seed: u32) -> usize {
    // Shrinking a crash asks for less room than the input holds: cut to it, then libFuzzer's own mutation.
    if size > max_size {
        return fuzzer_mutate(data, max_size, max_size);
    }
    let mut rng = Rng(seed | 1);
    let roll = rng.below(20);
    if roll < 5 {
        return fuzzer_mutate(data, size, max_size);
    }
    let input = data[..size].to_vec();
    let plain = plain_font(&input);
    let [c, t, f] = sections(&input);
    let (head, font) = if plain { (None, &input[..]) } else { (Some((&input[c], &input[t])), &input[f]) };
    let room = max_size.saturating_sub(size);
    let out = match (roll, head, tables(font, max_size)) {
        // The choices or the text alone.
        (5..=7, Some((choices, text)), _) => {
            if rng.below(2) == 0 {
                let mut fixed = choices.to_vec();
                fixed.resize(CHOICES, 0);
                let new = fuzzer_mutate(&mut fixed, CHOICES, CHOICES);
                fixed.truncate(new);
                join(Some((&fixed, text)), font)
            } else {
                join(Some((choices, &mutated(text.to_vec(), room.min(256)))), font)
            }
        }
        // Inside one table, its directory rebuilt to fit.
        (5..=15, _, Some((version, mut tables))) if !tables.is_empty() => {
            let k = rng.below(tables.len());
            let bytes = core::mem::take(&mut tables[k].1);
            tables[k].1 = mutated(bytes, room.min(4096));
            join(head, &build(version, &tables))
        }
        // Whole tables: dropped, repeated, reordered, cut, or two records naming one span.
        (_, _, Some((version, mut tables))) if !tables.is_empty() => {
            let (a, b) = (rng.below(tables.len()), rng.below(tables.len()));
            match rng.below(5) {
                0 if tables.len() > 1 => drop(tables.remove(a)),
                1 => tables.push(tables[a].clone()),
                2 => tables.swap(a, b),
                3 => {
                    let cut = rng.below(tables[a].1.len() + 1);
                    tables[a].1.truncate(cut);
                }
                _ => {
                    let mut font = build(version, &tables);
                    let (ra, rb) = (12 + 16 * a, 12 + 16 * b);
                    let span: [u8; 8] = font[rb + 8..rb + 16].try_into().unwrap_or_default();
                    font[ra + 8..ra + 16].copy_from_slice(&span);
                    return put(data, size, max_size, &join(head, &font));
                }
            }
            join(head, &build(version, &tables))
        }
        // A collection, or what isn't a font: in place, so its offsets still hold.
        _ => return fuzzer_mutate(data, size, max_size),
    };
    put(data, size, max_size, &out)
}

// `out` written over the input where it fits, else a raw mutation in its place.
fn put(data: &mut [u8], size: usize, max_size: usize, out: &[u8]) -> usize {
    if out.len() > max_size || out.is_empty() {
        return fuzzer_mutate(data, size, max_size);
    }
    data[..out.len()].copy_from_slice(out);
    out.len()
}

/// `a`'s font with one table taken from `b`'s, in place of its own of that tag or added, so tables written for
/// different fonts meet.
pub fn cross(a: &[u8], b: &[u8], out: &mut [u8], seed: u32) -> usize {
    let mut rng = Rng(seed | 1);
    let split = |d: &[u8]| -> (Option<(Vec<u8>, Vec<u8>)>, Vec<u8>) {
        if plain_font(d) {
            return (None, d.to_vec());
        }
        let [c, t, f] = sections(d);
        (Some((d[c].to_vec(), d[t].to_vec())), d[f].to_vec())
    };
    let ((head, font_a), (_, font_b)) = (split(a), split(b));
    let (Some((version, mut tables)), Some((_, theirs))) = (tables(&font_a, out.len()), tables(&font_b, out.len()))
    else {
        let n = a.len().min(out.len());
        out[..n].copy_from_slice(&a[..n]);
        return n;
    };
    if let Some(taken) = theirs.get(rng.below(theirs.len())).cloned() {
        match tables.iter_mut().find(|t| t.0 == taken.0) {
            Some(own) => *own = taken,
            None => tables.push(taken),
        }
    }
    let joined = join(head.as_ref().map(|(c, t)| (&c[..], &t[..])), &build(version, &tables));
    let n = joined.len().min(out.len());
    out[..n].copy_from_slice(&joined[..n]);
    n
}
