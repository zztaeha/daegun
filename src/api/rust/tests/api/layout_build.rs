use std::ops::Range;
use std::sync::Arc;

use daegun::shape::{Direction, ShapeOptions, ShapedRun};
use daegun::{Font, Location};

use super::aat_subset::u16s;

// A table's fields: words as they are, or an offset of 16 or 32 bits to a subtable laid out after the fields, null
// for None.
pub(super) enum Field {
    Words(Vec<u8>),
    To(Option<Vec<u8>>),
    To32(Option<Vec<u8>>),
}

pub(super) const NULL: Field = Field::To(None);

pub(super) fn w(words: &[u16]) -> Field {
    Field::Words(u16s(words))
}

pub(super) fn tag(tag: &[u8; 4]) -> Field {
    Field::Words(tag.to_vec())
}

pub(super) fn to(subtable: Vec<u8>) -> Field {
    Field::To(Some(subtable))
}

pub(super) fn maybe(subtable: Option<Vec<u8>>) -> Field {
    Field::To(subtable)
}

// The fields in order, then the subtables their offsets name, each offset counted from the table's start.
pub(super) fn table(fields: Vec<Field>) -> Vec<u8> {
    let width = |f: &Field| match f {
        Field::Words(words) => words.len(),
        Field::To(_) => 2,
        Field::To32(_) => 4,
    };
    let head: usize = fields.iter().map(width).sum();
    let (mut out, mut tail) = (Vec::new(), Vec::new());
    for field in fields {
        let (subtable, size) = match field {
            Field::Words(words) => {
                out.extend(words);
                continue;
            }
            Field::To(subtable) => (subtable, 2),
            Field::To32(subtable) => (subtable, 4),
        };
        let at = subtable.map_or(0, |s| {
            tail.extend(&s);
            head + tail.len() - s.len()
        });
        out.extend(&(at as u32).to_be_bytes()[4 - size..]);
    }
    [out, tail].concat()
}

// A list of offsets after its count.
pub(super) fn list(subtables: Vec<Field>) -> Vec<u8> {
    table([w(&[subtables.len() as u16])].into_iter().chain(subtables).collect())
}

// A GSUB or GPOS whose one script's default language system runs `features` in order, each its tag and lookups.
pub(super) fn layout(script: &[u8; 4], features: &[(&[u8; 4], &[u16])], lookups: Vec<Vec<u8>>) -> Vec<u8> {
    let indices: Vec<u16> = (0..features.len() as u16).collect();
    let lang_sys = table(vec![w(&[0, 0xFFFF, indices.len() as u16]), w(&indices)]);
    let scripts = table(vec![w(&[1]), tag(script), to(table(vec![to(lang_sys), w(&[0])]))]);
    let feature = |lookups: &[u16]| table(vec![w(&[0, lookups.len() as u16]), w(lookups)]);
    let records = features.iter().flat_map(|(t, l)| [tag(t), to(feature(l))]);
    let features = table([w(&[features.len() as u16])].into_iter().chain(records).collect());
    table(vec![w(&[1, 0]), to(scripts), to(features), to(list(lookups.into_iter().map(to).collect()))])
}

pub(super) fn lookup(kind: u16, flags: u16, subtables: Vec<Field>) -> Vec<u8> {
    table([w(&[kind, flags, subtables.len() as u16])].into_iter().chain(subtables).collect())
}

// A lookup of `kind` and `flags` whose `count` subtable offsets all name `subtable`.
pub(super) fn repeated(kind: u16, flags: u16, count: u16, subtable: Vec<u8>) -> Vec<u8> {
    [u16s(&[kind, flags, count]), u16s(&vec![6 + 2 * count; usize::from(count)]), subtable].concat()
}

// A GSUB or GPOS of `count` lookups naming `shared`, then each of `own`, whose default script runs `features` in
// order, each its tag and the range of those lookups it runs.
pub(super) fn aliased(features: &[(&[u8; 4], Range<u16>)], count: u16, shared: Vec<u8>, own: Vec<Vec<u8>>) -> Vec<u8> {
    let all = count + own.len() as u16;
    let indices: Vec<u16> = (0..features.len() as u16).collect();
    let lang_sys = table(vec![w(&[0, 0xFFFF, indices.len() as u16]), w(&indices)]);
    let scripts = table(vec![w(&[1]), tag(b"DFLT"), to(table(vec![to(lang_sys), w(&[0])]))]);
    let feature = |lookups: Range<u16>| table(vec![w(&[0, lookups.len() as u16]), w(&lookups.collect::<Vec<_>>())]);
    let records = features.iter().flat_map(|(t, l)| [tag(t), to(feature(l.clone()))]);
    let features = table([w(&[features.len() as u16])].into_iter().chain(records).collect());
    let mut offsets = vec![2 + 2 * all; usize::from(count)];
    let mut at = 2 + 2 * all + shared.len() as u16;
    for l in &own {
        offsets.push(at);
        at += l.len() as u16;
    }
    let lookups = [u16s(&[all]), u16s(&offsets), shared, own.concat()].concat();
    table(vec![w(&[1, 0]), to(scripts), to(features), to(lookups)])
}

pub(super) fn coverage(glyphs: &[u16]) -> Vec<u8> {
    [u16s(&[1, glyphs.len() as u16]), u16s(glyphs)].concat()
}

// A format 2 class definition of (first, last, class) ranges in order.
pub(super) fn classes(ranges: &[(u16, u16, u16)]) -> Vec<u8> {
    let words: Vec<u16> = ranges.iter().flat_map(|r| [r.0, r.1, r.2]).collect();
    [u16s(&[2, ranges.len() as u16]), u16s(&words)].concat()
}

pub(super) fn anchor(x: i16, y: i16) -> Vec<u8> {
    u16s(&[1, x as u16, y as u16])
}

pub(super) fn single(from: u16, by: u16) -> Vec<u8> {
    table(vec![w(&[2]), to(coverage(&[from])), w(&[1, by])])
}

// `text` shaped with unsafe-to-concat reported.
pub(super) fn run(f: &Font, text: &str, vertical: bool) -> Arc<ShapedRun> {
    let options = ShapeOptions::default().with_report_unsafe_to_concat(true);
    f.shape(
        text,
        &Location::default(),
        &if vertical { options.with_direction(Direction::TopToBottom) } else { options },
    )
}
