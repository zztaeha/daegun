use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, write_u16_be, records_fit};
use super::otl::remap_gid;

// The whole table is refused past its bound, or when a 16-bit offset cannot reach a child, rather
// than written with that child left unlinked.
struct Refused;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    List,
    Priority,
    LangSys,
    Script,
}

// A parent's bytes. A child it names from several slots is written once.
struct Blob {
    bytes: Vec<u8>,
    placed: BTreeMap<usize, u16>,
}

impl Blob {
    fn new() -> Self { Blob { bytes: Vec::new(), placed: BTreeMap::new() } }
    fn u16(&mut self, v: u16) { self.bytes.extend_from_slice(&v.to_be_bytes()); }
    fn tag(&mut self, t: &[u8]) { self.bytes.extend_from_slice(t); }
    fn slot(&mut self) -> usize { let at = self.bytes.len(); self.u16(0); at }
}

// The children a parent keeps, as (slot, source), and each one's bytes by source.
type Children = (Vec<(usize, usize)>, BTreeMap<usize, Vec<u8>>);

// Each part is rebuilt once per source offset, however many records name it. Copying children into
// parents is charged against four times the subset budget, one for each level a byte rises through.
struct Rebuild<'a> {
    jstf: &'a [u8],
    active: &'a GlyphSet,
    gid_map: &'a [u16],
    built: BTreeMap<(Part, usize), Option<Vec<u8>>>,
    work: usize,
}

impl Rebuild<'_> {
    fn place(&mut self, blob: &mut Blob, slot: usize, source: usize, child: &[u8]) -> Result<(), Refused> {
        let at = match blob.placed.get(&source) {
            Some(&at) => at,
            None => {
                let at = u16::try_from(blob.bytes.len()).map_err(|_| Refused)?;
                self.work = self.work.checked_sub(child.len()).ok_or(Refused)?;
                blob.bytes.extend_from_slice(child);
                blob.placed.insert(source, at);
                at
            }
        };
        write_u16_be(&mut blob.bytes, slot, at);
        Ok(())
    }

    fn part(&mut self, part: Part, off: usize) -> Result<Option<Vec<u8>>, Refused> {
        if let Some(built) = self.built.get(&(part, off)) { return Ok(built.clone()); }
        let built = match part {
            Part::List => self.mod_list(off),
            Part::Priority => self.priority(off)?,
            Part::LangSys => self.lang_sys(off)?,
            Part::Script => self.script(off)?,
        };
        self.built.insert((part, off), built.clone());
        Ok(built)
    }

    // The children a parent names, each fetched once: (slot, source), in the parent's order, for
    // those that survive.
    fn children(&mut self, part: Part, named: Vec<(usize, usize)>) -> Result<Children, Refused> {
        let mut built: BTreeMap<usize, Option<Vec<u8>>> = BTreeMap::new();
        for &(_, source) in &named {
            if let alloc::collections::btree_map::Entry::Vacant(slot) = built.entry(source) {
                slot.insert(self.part(part, source)?);
            }
        }
        let kept = named.into_iter().filter(|(_, source)| built[source].is_some()).collect();
        Ok((kept, built.into_iter().filter_map(|(s, b)| Some((s, b?))).collect()))
    }

    fn mod_list(&self, off: usize) -> Option<Vec<u8>> {
        let count = read_u16_be(self.jstf, off)? as usize;
        if !records_fit(off + 2, count, 2, self.jstf.len()) { return None; }
        self.jstf.get(off..off + 2 + count * 2).map(<[u8]>::to_vec)
    }

    fn priority(&mut self, off: usize) -> Result<Option<Vec<u8>>, Refused> {
        // Slots 4 and 9 name JstfMax tables, whose lookups the subset does not rebuild.
        const MAX_SLOTS: [usize; 2] = [4, 9];
        let mut blob = Blob::new();
        let slots: Vec<usize> = (0..10).map(|_| blob.slot()).collect();
        let named: Vec<(usize, usize)> = slots.iter().enumerate()
            .filter(|(i, _)| !MAX_SLOTS.contains(i))
            .filter_map(|(i, &slot)| {
                let rel = read_u16_be(self.jstf, off + i * 2).filter(|&r| r != 0)?;
                Some((slot, off + usize::from(rel)))
            })
            .collect();
        let (kept, lists) = self.children(Part::List, named)?;
        if kept.is_empty() { return Ok(None); }
        for (slot, source) in kept { self.place(&mut blob, slot, source, &lists[&source])?; }
        Ok(Some(blob.bytes))
    }

    fn lang_sys(&mut self, off: usize) -> Result<Option<Vec<u8>>, Refused> {
        let Some(count) = read_u16_be(self.jstf, off).map(usize::from) else { return Ok(None) };
        if !records_fit(off + 2, count, 2, self.jstf.len()) { return Ok(None); }
        let named: Vec<(usize, usize)> = (0..count)
            .filter_map(|i| read_u16_be(self.jstf, off + 2 + i * 2).filter(|&r| r != 0))
            .map(|rel| (0, off + usize::from(rel)))
            .collect();
        let (kept, priorities) = self.children(Part::Priority, named)?;
        if kept.is_empty() { return Ok(None); }

        let mut blob = Blob::new();
        blob.u16(kept.len() as u16);
        let slots: Vec<usize> = kept.iter().map(|_| blob.slot()).collect();
        for (slot, (_, source)) in slots.into_iter().zip(kept) {
            self.place(&mut blob, slot, source, &priorities[&source])?;
        }
        Ok(Some(blob.bytes))
    }

    fn script(&mut self, off: usize) -> Result<Option<Vec<u8>>, Refused> {
        let jstf = self.jstf;
        let extenders: Vec<u16> = read_u16_be(jstf, off)
            .filter(|&r| r != 0)
            .map(|rel| off + rel as usize)
            .and_then(|at| {
                let count = read_u16_be(jstf, at)? as usize;
                if !records_fit(at + 2, count, 2, jstf.len()) { return None; }
                Some((0..count)
                    .filter_map(|i| read_u16_be(jstf, at + 2 + i * 2))
                    .filter_map(|g| remap_gid(self.active, self.gid_map, g))
                    .collect())
            })
            .unwrap_or_default();

        let default = read_u16_be(jstf, off + 2).filter(|&r| r != 0).map(|rel| off + usize::from(rel));
        let default = match default {
            Some(source) => self.part(Part::LangSys, source)?.map(|bytes| (source, bytes)),
            None => None,
        };

        let Some(count) = read_u16_be(jstf, off + 4).map(usize::from) else { return Ok(None) };
        let records: Vec<(&[u8], usize)> = if records_fit(off + 6, count, 6, jstf.len()) {
            (0..count)
                .filter_map(|i| {
                    let rec = off + 6 + i * 6;
                    Some((jstf.get(rec..rec + 4)?, off + usize::from(read_u16_be(jstf, rec + 4).filter(|&r| r != 0)?)))
                })
                .collect()
        } else {
            Vec::new()
        };
        let (_, lang_systems) = self.children(Part::LangSys, records.iter().map(|&(_, s)| (0, s)).collect())?;
        let named: Vec<(&[u8], usize)> = records.into_iter().filter(|(_, s)| lang_systems.contains_key(s)).collect();

        if extenders.is_empty() && default.is_none() && named.is_empty() { return Ok(None); }

        let mut blob = Blob::new();
        let ext_slot = blob.slot();
        let def_slot = blob.slot();
        blob.u16(named.len() as u16);
        let named_slots: Vec<usize> = named.iter().map(|(tag, _)| { blob.tag(tag); blob.slot() }).collect();

        if !extenders.is_empty() {
            let mut ext = Blob::new();
            ext.u16(extenders.len() as u16);
            for g in &extenders { ext.u16(*g); }
            self.place(&mut blob, ext_slot, usize::MAX, &ext.bytes)?;
        }
        if let Some((source, bytes)) = &default { self.place(&mut blob, def_slot, *source, bytes)?; }
        for (slot, (_, source)) in named_slots.into_iter().zip(named) {
            self.place(&mut blob, slot, source, &lang_systems[&source])?;
        }
        Ok(Some(blob.bytes))
    }
}

pub fn subset_jstf(jstf: &[u8], active: &GlyphSet, gid_map: &[u16]) -> Option<Vec<u8>> {
    let count = read_u16_be(jstf, 4)? as usize;
    if !records_fit(6, count, 6, jstf.len()) { return None; }
    let budget = super::subset_budget(jstf.len());
    let mut rebuild = Rebuild { jstf, active, gid_map, built: BTreeMap::new(), work: budget.saturating_mul(4) };

    let records: Vec<(&[u8], usize)> = (0..count)
        .filter_map(|i| {
            let rec = 6 + i * 6;
            Some((jstf.get(rec..rec + 4)?, usize::from(read_u16_be(jstf, rec + 4).filter(|&r| r != 0)?)))
        })
        .collect();
    let (_, scripts) = rebuild.children(Part::Script, records.iter().map(|&(_, s)| (0, s)).collect()).ok()?;
    let kept: Vec<(&[u8], usize)> = records.into_iter().filter(|(_, s)| scripts.contains_key(s)).collect();
    if kept.is_empty() { return None; }

    let mut blob = Blob::new();
    blob.u16(1);
    blob.u16(0);
    blob.u16(kept.len() as u16);
    let slots: Vec<usize> = kept.iter().map(|(tag, _)| { blob.tag(tag); blob.slot() }).collect();
    for (slot, (_, source)) in slots.into_iter().zip(kept) {
        rebuild.place(&mut blob, slot, source, &scripts[&source]).ok()?;
    }
    (blob.bytes.len() <= budget).then_some(blob.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u16s(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    // A priority naming one mod list from its eight lookup slots, the list `rel` bytes on.
    fn priority(rel: u16) -> Vec<u8> {
        u16s(&[rel, rel, rel, rel, 0, rel, rel, rel, rel, 0])
    }

    fn list(n: u16) -> Vec<u8> {
        [u16s(&[n]), u16s(&(0..n).collect::<Vec<_>>())].concat()
    }

    fn everything() -> GlyphSet {
        let mut set = GlyphSet::new();
        set.insert(0);
        set
    }

    // One language system of 50 priorities named 51 times, each priority naming one 500-lookup list
    // eight times: 1,442 bytes that would come to 20.5 MB. Each part is written once in each parent.
    fn shared() -> Vec<u8> {
        let script_len = 6 + 50 * 6;
        let mut script = u16s(&[0, script_len as u16, 50]);
        (0..50).for_each(|_| script.extend([&b"dflt"[..], &(script_len as u16).to_be_bytes()].concat()));
        let lang_sys = [u16s(&[50]), u16s(&[102; 50])].concat();
        [u16s(&[1, 0, 1]), b"latn".to_vec(), u16s(&[12]), script, lang_sys, priority(20), list(500)].concat()
    }

    #[test]
    fn a_part_named_many_times_is_written_once() {
        let jstf = shared();
        assert_eq!(jstf.len(), 1_442);
        let out = subset_jstf(&jstf, &everything(), &[0]).expect("a subset JSTF");
        assert!(out.len() <= jstf.len(), "{} bytes of JSTF became {}", jstf.len(), out.len());
    }

    // Four language systems of three priorities, each priority naming one 2,000-lookup list eight
    // times. Copied per naming that would pass 64 KB and leave the language systems NULL.
    #[test]
    fn every_language_system_stays_linked() {
        let script_at = 12usize;
        let lang_at = |k: usize| script_at + 24 + 8 * k;
        let prio_at = |j: usize| lang_at(4) + 20 * j;
        let list_at = prio_at(3);
        let rel = |from: usize, to: usize| (to - from) as u16;
        let mut script = u16s(&[0, rel(script_at, lang_at(0)), 3]);
        for k in 1..4 {
            script.extend([&b"LNG "[..], &rel(script_at, lang_at(k)).to_be_bytes()].concat());
        }
        let lang_systems: Vec<u8> = (0..4).flat_map(|k| u16s(&[3, rel(lang_at(k), prio_at(0)), rel(lang_at(k), prio_at(1)), rel(lang_at(k), prio_at(2))])).collect();
        let priorities: Vec<u8> = (0..3).flat_map(|j| priority(rel(prio_at(j), list_at))).collect();
        let jstf = [u16s(&[1, 0, 1]), b"latn".to_vec(), u16s(&[script_at as u16]), script, lang_systems, priorities, list(2_000)].concat();

        let out = subset_jstf(&jstf, &everything(), &[0]).expect("a subset JSTF");
        let script = usize::from(read_u16_be(&out, 10).expect("a script"));
        let mut lang_systems = alloc::vec![read_u16_be(&out, script + 2)];
        lang_systems.extend((0..3).map(|k| read_u16_be(&out, script + 6 + 6 * k + 4)));
        for ls in lang_systems {
            let ls = script + usize::from(ls.filter(|&o| o != 0).expect("a language system left NULL"));
            for p in 0..3 {
                let prio = ls + usize::from(read_u16_be(&out, ls + 2 + 2 * p).filter(|&o| o != 0).expect("a priority left NULL"));
                let list = prio + usize::from(read_u16_be(&out, prio).expect("a list"));
                assert_eq!(read_u16_be(&out, list), Some(2_000));
            }
        }
    }

    // Three language systems share one priority and its 40,000-byte list. In the subset each takes a
    // copy, the third lands past the script's 16-bit offsets, and the table is refused, not left NULL.
    #[test]
    fn a_script_past_64_kb_is_refused() {
        const N: usize = 3;
        let lang_at = |k: usize| 12 + 6 + 6 * N + 4 * k;
        let prio_at = lang_at(N);
        let mut script = u16s(&[0, 0, N as u16]);
        (0..N).for_each(|k| script.extend([&b"LNG "[..], &((lang_at(k) - 12) as u16).to_be_bytes()].concat()));
        let lang_systems: Vec<u8> = (0..N).flat_map(|k| u16s(&[1, (prio_at - lang_at(k)) as u16])).collect();
        let jstf = [u16s(&[1, 0, 1]), b"latn".to_vec(), u16s(&[12]), script, lang_systems, priority(20), list(20_000)].concat();
        assert!(subset_jstf(&jstf, &everything(), &[0]).is_none(), "a language system was left unlinked");
    }
}
