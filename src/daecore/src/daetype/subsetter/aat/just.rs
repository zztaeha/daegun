use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be};
use super::super::super::format::aat::Lookup;
use super::super::otl::remap_gid;
use super::lookup::{build_aat_lookup, build_aat_offset_lookup};

const DIRECTION_HEADER: usize = 6;
const ACTION_HEADER: usize = 8;
mod action {
    pub(super) const DECOMPOSITION: u16 = 0;
    pub(super) const ADD_GLYPH: u16 = 1;
    pub(super) const CONDITIONAL_ADD_GLYPH: u16 = 2;
    pub(super) const STRETCH: u16 = 3;
    pub(super) const DUCTILE: u16 = 4;
    pub(super) const REPEATED_ADD_GLYPH: u16 = 5;
}

const NO_GLYPH: u16 = 0xFFFF;

fn subset_actions(data: &[u8], at: usize, active: &GlyphSet, gid_map: &[u16]) -> Option<Vec<u8>> {
    let count = read_u32_be(data, at)?;
    let mut kept: Vec<Vec<u8>> = Vec::new();

    let mut cursor = at + 4;
    for _ in 0..count {
        let action_type = read_u16_be(data, cursor + 2)?;
        let length = read_u32_be(data, cursor + 4)? as usize;
        if length < ACTION_HEADER { return None; }
        let record = data.get(cursor..cursor.checked_add(length)?)?;
        cursor += length;

        let remap_at = |out: &mut Vec<u8>, offset: usize| -> Option<()> {
            let g = read_u16_be(record, ACTION_HEADER + offset)?;
            let new = remap_gid(active, gid_map, g)?;
            out.get_mut(ACTION_HEADER + offset..ACTION_HEADER + offset + 2)?
                .copy_from_slice(&new.to_be_bytes());
            Some(())
        };

        let mut out = record.to_vec();
        let survived = match action_type {
            action::DECOMPOSITION => {
                let n = read_u16_be(record, ACTION_HEADER + 10)? as usize;
                let glyphs_at = ACTION_HEADER + 12;
                if glyphs_at + n * 2 > record.len() { return None; }
                (0..n).try_for_each(|i| remap_at(&mut out, 12 + i * 2)).is_some()
            }
            action::ADD_GLYPH => remap_at(&mut out, 0).is_some(),
            action::CONDITIONAL_ADD_GLYPH => {
                let add = read_u16_be(record, ACTION_HEADER + 4)?;
                let new_add = if add == NO_GLYPH { NO_GLYPH } else {
                    remap_gid(active, gid_map, add).unwrap_or(NO_GLYPH)
                };
                out.get_mut(ACTION_HEADER + 4..ACTION_HEADER + 6)?.copy_from_slice(&new_add.to_be_bytes());
                remap_at(&mut out, 6).is_some()
            }
            action::REPEATED_ADD_GLYPH => remap_at(&mut out, 2).is_some(),
            action::STRETCH | action::DUCTILE => true,
            _ => false,
        };
        if survived { kept.push(out); }
    }
    if kept.is_empty() { return None; }

    let mut out = (kept.len() as u32).to_be_bytes().to_vec();
    for k in kept { out.extend_from_slice(&k); }
    Some(out)
}

// The kept glyphs a lookup names, with their values; an empty list when it names none.
fn remap_keys(data: &[u8], at: usize, active: &GlyphSet, gid_map: &[u16], num_glyphs: u16)
    -> Option<Vec<(u16, u16)>>
{
    let lookup = Lookup::parse(data.get(at..)?, num_glyphs)?;
    Some(lookup.entries().into_iter()
        .filter_map(|(g, v)| remap_gid(active, gid_map, g).map(|ng| (ng, v)))
        .collect())
}

// The category table is mort-style: an 8-byte header and a classic state table. Its class array is
// rebuilt in place, which only shrinks it, so the state array and entries keep their offsets.
fn subset_category_table(just: &[u8], off: usize, active: &GlyphSet, gid_map: &[u16]) -> Option<Vec<u8>> {
    const STATE_TABLE: usize = 8;
    let length = usize::from(read_u16_be(just, off)?);
    let mut table = just.get(off..off.checked_add(length)?)?.to_vec();
    let class_at = STATE_TABLE + usize::from(read_u16_be(&table, STATE_TABLE + 2)?);
    let first = read_u16_be(&table, class_at)?;
    let n = usize::from(read_u16_be(&table, class_at + 2)?);
    let kept: Vec<(u16, u8)> = table.get(class_at + 4..class_at + 4 + n)?.iter().enumerate()
        .filter_map(|(i, &class)| {
            let g = first.checked_add(u16::try_from(i).ok()?)?;
            Some((remap_gid(active, gid_map, g)?, class))
        })
        .collect();

    let new_first = kept.first().map_or(0, |&(g, _)| g);
    let new_n = kept.last().map_or(0, |&(g, _)| usize::from(g - new_first) + 1);
    let mut classes = alloc::vec![1u8; new_n];
    kept.into_iter().for_each(|(g, class)| classes[usize::from(g - new_first)] = class);
    table.get_mut(class_at..class_at + 4)?
        .copy_from_slice(&[new_first.to_be_bytes(), u16::try_from(new_n).ok()?.to_be_bytes()].concat());
    table.get_mut(class_at + 4..class_at + 4 + new_n)?.copy_from_slice(&classes);
    Some(table)
}

// Glyph to action list. A list many glyphs share is rebuilt and written once, and a value of 0 means
// the glyph has no action.
fn subset_postcompensation(
    just: &[u8], off: usize, active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let mut lists: BTreeMap<u16, Option<Vec<u8>>> = BTreeMap::new();
    let mut kept: Vec<(u16, u16)> = Vec::new();
    for (g, value) in remap_keys(just, off, active, gid_map, num_glyphs)? {
        if value == 0 { continue; }
        let list = lists.entry(value)
            .or_insert_with(|| subset_actions(just, off.checked_add(usize::from(value))?, active, gid_map));
        if list.is_some() { kept.push((g, value)); }
    }
    if kept.is_empty() { return None; }

    let lookup_len = build_aat_offset_lookup(&kept)?.len();
    let mut actions: Vec<u8> = Vec::new();
    let mut placed: BTreeMap<u16, u16> = BTreeMap::new();
    for &(_, value) in &kept {
        if placed.contains_key(&value) { continue; }
        placed.insert(value, u16::try_from(lookup_len + actions.len()).ok()?);
        actions.extend_from_slice(lists.get(&value)?.as_ref()?);
    }
    let entries: Vec<(u16, u16)> = kept.iter().map(|&(g, value)| (g, placed[&value])).collect();
    let mut table = build_aat_offset_lookup(&entries)?;
    table.extend_from_slice(&actions);
    Some(table)
}

struct Direction {
    head: Vec<u8>,
    class_table: Option<Vec<u8>>,
    wdc: Vec<u8>,
    pc: Option<Vec<u8>>,
}

fn subset_direction(
    just: &[u8], at: usize, active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Direction> {
    let class_off = read_u16_be(just, at)? as usize;
    let wdc_off = read_u16_be(just, at + 2)? as usize;
    let pc_off = read_u16_be(just, at + 4)? as usize;

    let mut head = alloc::vec![0u8; DIRECTION_HEADER];
    head.extend_from_slice(&build_aat_lookup(&remap_keys(just, at + DIRECTION_HEADER, active, gid_map, num_glyphs)?)?);

    let class_table = (class_off != 0).then(|| subset_category_table(just, class_off, active, gid_map)).flatten();
    let wdc_end = [class_off, pc_off, just.len()].into_iter()
        .filter(|&o| o > wdc_off).min().unwrap_or(just.len());
    let wdc = just.get(wdc_off..wdc_end)?.to_vec();
    let pc = (pc_off != 0).then(|| subset_postcompensation(just, pc_off, active, gid_map, num_glyphs)).flatten();

    Some(Direction { head, class_table, wdc, pc })
}

pub fn subset_just(
    just: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let version = read_u32_be(just, 0)?;
    let format = read_u16_be(just, 4)?;

    let mut built: [Option<Direction>; 2] = [None, None];
    for (i, slot) in [6usize, 8].into_iter().enumerate() {
        let off = read_u16_be(just, slot)? as usize;
        if off == 0 { continue; }
        built[i] = subset_direction(just, off, active, gid_map, num_glyphs);
    }
    if built.iter().all(Option::is_none) { return None; }

    // Every offset is 16 bits from the table's start: a layout past that is refused, not wrapped. A
    // part both directions share, such as one width delta table, is written once.
    let mut dir_offsets = [0u16; 2];
    let mut at = 10usize;
    for (i, d) in built.iter().enumerate() {
        let Some(d) = d else { continue };
        dir_offsets[i] = u16::try_from(at).ok()?;
        at += d.head.len();
    }
    let mut placed: BTreeMap<&[u8], u16> = BTreeMap::new();
    let mut parts: Vec<&[u8]> = Vec::new();
    let mut table_offsets = [[0u16; 3]; 2];
    for (i, d) in built.iter().enumerate() {
        let Some(d) = d else { continue };
        for (k, part) in [d.class_table.as_deref(), Some(d.wdc.as_slice()), d.pc.as_deref()].into_iter().enumerate() {
            let Some(part) = part else { continue };
            table_offsets[i][k] = match placed.get(part) {
                Some(&o) => o,
                None => {
                    let o = u16::try_from(at).ok()?;
                    placed.insert(part, o);
                    parts.push(part);
                    at += part.len();
                    o
                }
            };
        }
    }
    if at > super::super::subset_budget(just.len()) { return None; }

    let mut out = Vec::with_capacity(at);
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&dir_offsets[0].to_be_bytes());
    out.extend_from_slice(&dir_offsets[1].to_be_bytes());
    for (i, d) in built.iter().enumerate() {
        let Some(d) = d else { continue };
        let mut head = d.head.clone();
        for (k, v) in table_offsets[i].iter().enumerate() {
            head.get_mut(k * 2..k * 2 + 2)?.copy_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&head);
    }
    parts.iter().for_each(|p| out.extend_from_slice(p));
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A postcompensation lookup that names glyph 1 twice, as format 6 allows.
    #[test]
    fn a_glyph_named_twice_by_postcompensation_is_kept_once() {
        let words: [u16; 34] = [
            1, 0, 0, 10, 0,
            0, 32, 36,
            6, 4, 1, 4, 0, 0, 1, 0,
            0, 0,
            6, 4, 2, 8, 1, 0, 1, 20, 1, 20,
            0, 1, 0, 3, 0, 8,
        ];
        let just: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
        let mut active = GlyphSet::new();
        active.insert(0);
        active.insert(1);
        let out = subset_just(&just, &active, &[0, 1], 2).expect("a subset just");
        let pc = usize::from(read_u16_be(&out, 14).expect("a pc offset"));
        let lookup = Lookup::parse(out.get(pc..).expect("the pc lookup"), 2).expect("the pc lookup parses");
        let (_, at) = lookup.entries()[0];
        assert_eq!(read_u32_be(&out, pc + usize::from(at)), Some(1), "glyph 1's actions are not where its offset says");
    }

    fn u16s(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    // A just table of (category table, width deltas, postcompensation, lookup) per direction, laid out
    // after the headers. Parts with the same bytes are written once, so directions can share one.
    type Dir = (Option<Vec<u8>>, Vec<u8>, Option<Vec<u8>>, Vec<u8>);

    fn just_of(dirs: [Option<Dir>; 2]) -> Vec<u8> {
        let mut out = u16s(&[1, 0, 0, 0, 0]);
        let mut at = 10 + dirs.iter().flatten().map(|d| 6 + d.3.len()).sum::<usize>();
        let mut parts: Vec<Vec<u8>> = Vec::new();
        let mut heads = Vec::new();
        for (i, d) in dirs.iter().enumerate() {
            let Some((class, wdc, pc, lookup)) = d else { continue };
            out[6 + 2 * i..8 + 2 * i].copy_from_slice(&((10 + heads.len()) as u16).to_be_bytes());
            let mut head = Vec::new();
            for part in [class.as_ref(), Some(wdc), pc.as_ref()] {
                let Some(part) = part else { head.extend([0, 0]); continue };
                let found = parts.iter().scan(10 + dirs.iter().flatten().map(|d| 6 + d.3.len()).sum::<usize>(), |o, p| {
                    let here = *o;
                    *o += p.len();
                    Some((here, p))
                }).find(|(_, p)| *p == part).map(|(o, _)| o);
                let o = found.unwrap_or_else(|| { let o = at; at += part.len(); parts.push(part.clone()); o });
                head.extend((o as u16).to_be_bytes());
            }
            head.extend(lookup);
            heads.extend(head);
        }
        [out, heads, parts.concat()].concat()
    }

    fn active(glyphs: &[u16]) -> GlyphSet {
        let mut set = GlyphSet::new();
        glyphs.iter().for_each(|&g| { set.insert(g); });
        set
    }

    // The category table is a mort-style subtable: rebuilt in place, its class array follows the
    // kept glyphs. Read as an extended state table, it would be dropped from every subset.
    #[test]
    fn the_category_table_is_kept_with_its_classes_renumbered() {
        let states = [0u8; 12];
        let st = [u16s(&[6, 8, 17, 29]), u16s(&[1, 5]), alloc::vec![4, 5, 4, 5, 4], states.to_vec(), u16s(&[0, 0, 0, 0])].concat();
        let category = [u16s(&[(8 + st.len()) as u16, 0, 0, 1]), st].concat();
        let lookup = build_aat_lookup(&[(2, 0)]).expect("a lookup");
        let just = just_of([Some((Some(category), u16s(&[0, 0]), None, lookup)), None]);
        let out = subset_just(&just, &active(&[0, 2, 4]), &[0, 0, 1, 0, 2], 6).expect("a subset just");
        let at = usize::from(read_u16_be(&out, usize::from(read_u16_be(&out, 6).expect("a direction"))).expect("a category offset"));
        assert_ne!(at, 0, "the category table was dropped");
        assert_eq!(out[at + 16..at + 22], [0, 1, 0, 2, 5, 5]);
    }

    // Two directions each with its own 40,000-byte width delta table cannot both sit within 16-bit
    // offsets: the table is refused, not written with an offset wrapped past 64 KB. One they share fits.
    #[test]
    fn offsets_past_64_kb_refuse_the_table_but_a_shared_part_fits() {
        let lookup = build_aat_lookup(&[(1, 0)]).expect("a lookup");
        let (a, b) = (alloc::vec![1u8; 40_000], alloc::vec![2u8; 40_000]);
        let everything = active(&[0, 1]);
        let apart = just_of([Some((None, a.clone(), None, lookup.clone())), Some((None, b, None, lookup.clone()))]);
        assert!(subset_just(&apart, &everything, &[0, 1], 2).is_none(), "an offset past 64 KB was written");

        let shared = just_of([Some((None, a.clone(), None, lookup.clone())), Some((None, a, None, lookup))]);
        let out = subset_just(&shared, &everything, &[0, 1], 2).expect("a shared table fits");
        let wdc = |dir: usize| read_u16_be(&out, usize::from(read_u16_be(&out, dir).expect("a direction")) + 2);
        assert_eq!(wdc(6), wdc(8));
        assert!(out.len() < 41_000, "the shared table was written twice");
    }

    // 1,100 glyphs whose postcompensation names one action list: copied per glyph it would pass 64 KB
    // and drop the whole postcompensation table.
    #[test]
    fn an_action_list_many_glyphs_share_is_written_once() {
        let n = 1_100u16;
        let list = [0u32.to_be_bytes().to_vec(), 1u32.to_be_bytes().to_vec(), u16s(&[0, 3]), 56u32.to_be_bytes().to_vec(), alloc::vec![0; 48]].concat();
        let lookup_len = 12 + 4 * (usize::from(n) + 1);
        let pc_lookup = {
            let mut t = u16s(&[6, 4, n, 4096, 10, 4 * n - 4096]);
            (1..=n).for_each(|g| t.extend(u16s(&[g, lookup_len as u16])));
            t.extend(u16s(&[0xFFFF, 0]));
            t
        };
        let list = list[4..].to_vec();
        let pc = [pc_lookup, list].concat();
        let lookup = build_aat_lookup(&[(1, 0)]).expect("a lookup");
        let just = just_of([Some((None, u16s(&[0, 0]), Some(pc), lookup)), None]);
        let glyphs: Vec<u16> = (0..=n).collect();
        let out = subset_just(&just, &active(&glyphs), &glyphs, n + 1).expect("a subset just");
        let head = usize::from(read_u16_be(&out, 6).expect("a direction"));
        let pc = usize::from(read_u16_be(&out, head + 4).expect("a pc offset"));
        assert_ne!(pc, 0, "the postcompensation table was dropped");
        let lookup = Lookup::parse(&out[pc..], n + 1).expect("the pc lookup");
        let at: alloc::collections::BTreeSet<u16> = lookup.entries().into_iter().map(|(_, v)| v).collect();
        assert_eq!(at.len(), 1, "the list was written more than once");
        assert_eq!(read_u32_be(&out, pc + usize::from(*at.first().expect("one"))), Some(1));
    }

    // A postcompensation value of 0 means no action. Glyphs 1 and 3 have one, and read as an offset it
    // parses the lookup's own bytes into a stretch action they never had.
    #[test]
    fn a_postcompensation_value_of_0_is_no_action() {
        let list = [1u32.to_be_bytes().to_vec(), u16s(&[0, 3]), 8u32.to_be_bytes().to_vec()].concat();
        let pc = [u16s(&[0, 1, 0, 3, 0, 8, 14]), list].concat();
        let lookup = build_aat_lookup(&[(1, 0)]).expect("a lookup");
        let just = just_of([Some((None, u16s(&[0, 0]), Some(pc), lookup)), None]);
        let glyphs: Vec<u16> = (0..6).collect();
        let out = subset_just(&just, &active(&glyphs), &glyphs, 6).expect("a subset just");
        let head = usize::from(read_u16_be(&out, 6).expect("a direction"));
        let pc = usize::from(read_u16_be(&out, head + 4).expect("a pc offset"));
        let lookup = Lookup::parse(&out[pc..], 6).expect("the pc lookup");
        assert_eq!(lookup.entries().into_iter().map(|(g, _)| g).collect::<Vec<_>>(), [5]);
    }
}
