use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, read_uint, records_fit};
use super::super::jstf::ScriptList;
use super::remap_gid;

// The whole table is refused past its bound, or when an offset cannot reach a child, rather than written with that
// child left unlinked.
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
    placed: BTreeMap<usize, usize>,
}

impl Blob {
    fn new() -> Self { Blob { bytes: Vec::new(), placed: BTreeMap::new() } }
    fn u16(&mut self, v: u16) { self.bytes.extend_from_slice(&v.to_be_bytes()); }
    fn tag(&mut self, t: &[u8]) { self.bytes.extend_from_slice(t); }
    fn slot(&mut self, width: usize) -> usize { let at = self.bytes.len(); self.bytes.resize(at + width, 0); at }
}

// The children a parent keeps, as (slot, source), and each one's bytes by source.
type Children = (Vec<(usize, usize)>, BTreeMap<usize, Vec<u8>>);

// Each part is rebuilt once per source offset; parts may overlap, so records read and children copied into parents are
// charged against four times the subset budget. Scripts read the source's form and write `wide`'s.
struct Rebuild<'a> {
    jstf: &'a [u8],
    list: ScriptList,
    wide: bool,
    active: &'a GlyphSet,
    gid_map: &'a [u32],
    built: BTreeMap<(Part, usize), Option<Vec<u8>>>,
    work: usize,
}

impl Rebuild<'_> {
    fn spend(&mut self, bytes: usize) -> Result<(), Refused> {
        self.work = self.work.checked_sub(bytes).ok_or(Refused)?;
        Ok(())
    }

    fn place(
        &mut self, blob: &mut Blob, slot: usize, width: usize, source: usize, child: &[u8],
    ) -> Result<(), Refused> {
        let at = match blob.placed.get(&source) {
            Some(&at) => at,
            None => {
                let at = blob.bytes.len();
                self.spend(child.len())?;
                blob.bytes.extend_from_slice(child);
                blob.placed.insert(source, at);
                at
            }
        };
        let at = u32::try_from(at).ok().filter(|&at| u64::from(at) >> (8 * width) == 0).ok_or(Refused)?;
        blob.bytes[slot..slot + width].copy_from_slice(&at.to_be_bytes()[4 - width..]);
        Ok(())
    }

    // A script's extender glyphs the subset keeps, under their new ids.
    fn extenders(&mut self, script: usize) -> Result<Vec<u32>, Refused> {
        let (jstf, o, w) = (self.jstf, self.list.offset_width(), self.list.glyph_width());
        let at = read_uint(jstf, script, o).filter(|&rel| rel != 0).and_then(|rel| script.checked_add(rel as usize));
        let Some(at) = at else { return Ok(Vec::new()) };
        let Some(count) = read_u16_be(jstf, at).map(usize::from) else { return Ok(Vec::new()) };
        if !records_fit(at + 2, count, w, jstf.len()) { return Ok(Vec::new()); }
        self.spend(count * w)?;
        Ok((0..count)
            .filter_map(|i| read_uint(jstf, at + 2 + i * w, w))
            .filter_map(|g| remap_gid(self.active, self.gid_map, g))
            .collect())
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
        let slots: Vec<usize> = (0..10).map(|_| blob.slot(2)).collect();
        let named: Vec<(usize, usize)> = slots.iter().enumerate()
            .filter(|(i, _)| !MAX_SLOTS.contains(i))
            .filter_map(|(i, &slot)| {
                let rel = read_u16_be(self.jstf, off + i * 2).filter(|&r| r != 0)?;
                Some((slot, off + usize::from(rel)))
            })
            .collect();
        let (kept, lists) = self.children(Part::List, named)?;
        if kept.is_empty() { return Ok(None); }
        for (slot, source) in kept { self.place(&mut blob, slot, 2, source, &lists[&source])?; }
        Ok(Some(blob.bytes))
    }

    fn lang_sys(&mut self, off: usize) -> Result<Option<Vec<u8>>, Refused> {
        let Some(count) = read_u16_be(self.jstf, off).map(usize::from) else { return Ok(None) };
        if !records_fit(off + 2, count, 2, self.jstf.len()) { return Ok(None); }
        self.spend(2 * count)?;
        let named: Vec<(usize, usize)> = (0..count)
            .filter_map(|i| read_u16_be(self.jstf, off + 2 + i * 2).filter(|&r| r != 0))
            .map(|rel| (0, off + usize::from(rel)))
            .collect();
        let (kept, priorities) = self.children(Part::Priority, named)?;
        if kept.is_empty() { return Ok(None); }

        let mut blob = Blob::new();
        blob.u16(kept.len() as u16);
        let slots: Vec<usize> = kept.iter().map(|_| blob.slot(2)).collect();
        for (slot, (_, source)) in slots.into_iter().zip(kept) {
            self.place(&mut blob, slot, 2, source, &priorities[&source])?;
        }
        Ok(Some(blob.bytes))
    }

    fn script(&mut self, off: usize) -> Result<Option<Vec<u8>>, Refused> {
        let (jstf, o) = (self.jstf, self.list.offset_width());
        let extenders = self.extenders(off)?;

        let default = read_uint(jstf, off + o, o).filter(|&r| r != 0).and_then(|rel| off.checked_add(rel as usize));
        let default = match default {
            Some(source) => self.part(Part::LangSys, source)?.map(|bytes| (source, bytes)),
            None => None,
        };

        let Some(count) = read_u16_be(jstf, off + 2 * o).map(usize::from) else { return Ok(None) };
        let records: Vec<(&[u8], usize)> = if records_fit(off + 2 * o + 2, count, 6, jstf.len()) {
            self.spend(6 * count)?;
            (0..count)
                .filter_map(|i| {
                    let rec = off + 2 * o + 2 + i * 6;
                    Some((jstf.get(rec..rec + 4)?, off + usize::from(read_u16_be(jstf, rec + 4).filter(|&r| r != 0)?)))
                })
                .collect()
        } else {
            Vec::new()
        };
        let (_, lang_systems) = self.children(Part::LangSys, records.iter().map(|&(_, s)| (0, s)).collect())?;
        let named: Vec<(&[u8], usize)> = records.into_iter().filter(|(_, s)| lang_systems.contains_key(s)).collect();

        if extenders.is_empty() && default.is_none() && named.is_empty() { return Ok(None); }

        let (o, w) = if self.wide { (4, 3) } else { (2, 2) };
        let mut blob = Blob::new();
        let ext_slot = blob.slot(o);
        let def_slot = blob.slot(o);
        blob.u16(named.len() as u16);
        let named_slots: Vec<usize> = named.iter().map(|(tag, _)| { blob.tag(tag); blob.slot(2) }).collect();

        if !extenders.is_empty() {
            let mut ext = Blob::new();
            ext.u16(extenders.len() as u16);
            for g in &extenders { ext.bytes.extend_from_slice(&g.to_be_bytes()[4 - w..]); }
            self.place(&mut blob, ext_slot, o, usize::MAX, &ext.bytes)?;
        }
        if let Some((source, bytes)) = &default { self.place(&mut blob, def_slot, o, *source, bytes)?; }
        for (slot, (_, source)) in named_slots.into_iter().zip(named) {
            self.place(&mut blob, slot, 2, source, &lang_systems[&source])?;
        }
        Ok(Some(blob.bytes))
    }
}

// Version 1.0, or 1.1 with an empty scriptList and every script in scriptList2 where a kept extender's new id passes
// 16 bits, as scriptList2 is read alone once it names any script.
pub fn subset_jstf(jstf: &[u8], active: &GlyphSet, gid_map: &[u32]) -> Option<Vec<u8>> {
    let list = ScriptList::read(jstf)?;
    if !records_fit(list.records, list.count, list.stride(), jstf.len()) { return None; }
    let records: Vec<(&[u8], usize)> = (0..list.count).filter_map(|i| list.record(jstf, i)).collect();
    let budget = super::subset_budget(jstf.len());
    let mut rebuild =
        Rebuild { jstf, list, wide: false, active, gid_map, built: BTreeMap::new(), work: budget.saturating_mul(4) };
    let named: BTreeSet<usize> = records.iter().map(|&(_, s)| s).collect();
    for s in named {
        rebuild.wide |= rebuild.extenders(s).ok()?.iter().any(|&g| g > 0xFFFF);
    }

    let (_, scripts) = rebuild.children(Part::Script, records.iter().map(|&(_, s)| (0, s)).collect()).ok()?;
    let kept: Vec<(&[u8], usize)> = records.into_iter().filter(|(_, s)| scripts.contains_key(s)).collect();
    if kept.is_empty() { return None; }

    let o = if rebuild.wide { 4 } else { 2 };
    let mut blob = Blob::new();
    blob.u16(1);
    blob.u16(u16::from(rebuild.wide));
    if rebuild.wide { blob.u16(0); }
    blob.u16(kept.len() as u16);
    let slots: Vec<usize> = kept.iter().map(|(tag, _)| { blob.tag(tag); blob.slot(o) }).collect();
    for (slot, (_, source)) in slots.into_iter().zip(kept) {
        rebuild.place(&mut blob, slot, o, source, &scripts[&source]).ok()?;
    }
    (blob.bytes.len() <= budget).then_some(blob.bytes)
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/jstf.rs"]
mod tests;
