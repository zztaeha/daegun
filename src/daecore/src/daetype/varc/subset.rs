// VARC's closure, the glyphs its records reach, and its subset, the table rewritten for the glyphs kept.
use alloc::collections::btree_map::Entry;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec;
use alloc::vec::Vec;
use super::measure::{decompile, Table, AXIS_LIST, AXIS_VAR, CONDITION, GID, TRANSFORM_VAR};
use super::measure::{AXIS_VALUES_HAVE_VARIATION, GID_IS_24BIT, HAVE_AXES, HAVE_CONDITION, NO_VARIATION};
use super::measure::TRANSFORM_HAS_VARIATION;
use crate::daecore::daetype::subsetter::{subset_budget, GlyphSet};

// Every glyph the records of `closed`'s covered glyphs name, and theirs in turn, added to `closed` below `num_glyphs`;
// conditions are ignored. A record ends at a component that doesn't read, failing the subset; false past the budget.
pub(crate) fn closure(varc: &[u8], closed: &mut GlyphSet, num_glyphs: u32) -> bool {
    let t = Table::new(varc);
    let mut seen: BTreeSet<u32> = closed.iter().collect();
    let mut pending: Vec<u32> = seen.iter().copied().collect();
    let mut once = RecordsRead::within(varc);
    while let Some(gid) = pending.pop() {
        let Some(index) = t.coverage(gid) else { continue };
        let mut record = t.entry(t.records, index);
        match once.first(record) {
            Some(true) => {}
            Some(false) => continue,
            None => return false,
        }
        while let Some(c) = (!record.is_empty()).then(|| decompile(&t, record)).flatten() {
            if seen.insert(c.gid) {
                pending.push(c.gid);
            }
            record = &record[c.size..];
        }
    }
    for gid in seen.into_iter().filter(|&g| g < num_glyphs) {
        closed.insert(gid);
    }
    true
}

// The records read, by where their bytes lie, as many glyphs' records may share or overlap them: each is read once,
// and all of them within the subset budget.
struct RecordsRead {
    read: BTreeSet<(*const u8, usize)>,
    left: usize,
}

impl RecordsRead {
    fn within(varc: &[u8]) -> RecordsRead {
        RecordsRead { read: BTreeSet::new(), left: subset_budget(varc.len()) }
    }

    // Whether `record` is read for the first time; None once the records read pass the budget.
    fn first(&mut self, record: &[u8]) -> Option<bool> {
        if !self.read.insert((record.as_ptr(), record.len())) {
            return Some(false);
        }
        self.left = self.left.checked_sub(record.len())?;
        Some(true)
    }
}

// What a subset makes of VARC: the table, none where it keeps no covered glyph, or a refusal where it can't write
// one: a component or index that doesn't read or map, an axis named where all are `pinned`, an offset past its width.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Subset {
    Written(Vec<u8>),
    Unkept,
    Refused,
}

// VARC subset to the covered glyphs of `varced` (the closure up to VARC's own stage), each glyph at `new_gid`'s id;
// a component's glyph keeps its own id where the subset `retain`s them. Every axis is `pinned` in an instance.
pub(crate) fn subset(
    varc: &[u8], varced: &GlyphSet, new_gid: impl Fn(u32) -> Option<u32>, retain: bool, pinned: bool,
) -> Subset {
    let t = Table::new(varc);
    let mut kept = Vec::new();
    for (old, index) in covered(&t) {
        if varced.contains(&old) {
            let Some(new) = new_gid(old) else { return Subset::Refused };
            kept.push((index, new));
        }
    }
    if kept.is_empty() {
        return Subset::Unkept;
    }
    let plan = Plan { t, kept, new_gid: &new_gid, retain, pinned };
    plan.write().map_or(Subset::Refused, Subset::Written)
}

// The glyphs a record follows for, in Coverage's order.
pub(crate) fn covered_glyphs(varc: &[u8]) -> Vec<u32> {
    covered(&Table::new(varc)).into_iter().map(|(gid, _)| gid).collect()
}

// The glyphs Coverage lists with a record, in its order, each with its coverage index: a list's by position, a range's
// from its start index on, as the spec numbers them. A range running backward lists nothing, the others kept.
fn covered(t: &Table) -> Vec<(u32, u32)> {
    let Some(at) = t.coverage else { return Vec::new() };
    // Each record takes a byte of the table at least, which bounds what a malformed Coverage can list.
    let records = t.records.map_or(0, |at| t.u32(at)).min(t.t.len() as u32);
    let format = t.u16(at);
    let w = if format > 2 { 3 } else { 2 };
    let read = |at: usize| (0..w).fold(0, |v, b| v << 8 | u32::from(t.t.get(at + b).copied().unwrap_or(0)));
    let count = read(at + 2) as usize;
    let mut out = Vec::new();
    match format {
        1 | 3 => out.extend((0..count.min(records as usize)).map(|i| (read(at + 2 + w + i * w), i as u32))),
        2 | 4 => {
            for i in 0..count {
                let range = at + 2 + w + i * 3 * w;
                let (first, last, index) = (read(range), read(range + w), read(range + 2 * w));
                let room = records as usize - out.len();
                out.extend((first..=last).zip(index..records).take(room));
            }
        }
        _ => {}
    }
    out
}

// Records rebuilt, by where the source's bytes lie.
type Rebuilt = BTreeMap<(*const u8, usize), Vec<u8>>;

struct Plan<'a, F> {
    t: Table<'a>,
    kept: Vec<(u32, u32)>,
    new_gid: &'a F,
    retain: bool,
    pinned: bool,
}

// The indices the kept records use, each mapped to its rank: conditions, axis lists, and variation indices by store
// row, each row's outer the rank of its data set among those keeping any. `nulls` where a component names a null one.
struct Maps {
    nulls: bool,
    conditions: BTreeMap<u32, u32>,
    lists: BTreeMap<u32, u32>,
    rows: BTreeMap<u32, BTreeSet<u32>>,
    vars: BTreeMap<u32, u32>,
}

impl<F: Fn(u32) -> Option<u32>> Plan<'_, F> {
    fn record(&self, index: u32) -> &[u8] {
        self.t.entry(self.t.records, index)
    }

    // Each component of each kept record, in order, a record many glyphs share read once; None past the budget.
    fn components(&self, mut each: impl FnMut(&[u8], &super::measure::Component) -> Option<()>) -> Option<()> {
        let mut once = RecordsRead::within(self.t.t);
        for &(index, _) in &self.kept {
            let mut r = self.record(index);
            if !once.first(r)? {
                continue;
            }
            while !r.is_empty() {
                let c = decompile(&self.t, r)?;
                each(r, &c)?;
                r = &r[c.size..];
            }
        }
        Some(())
    }

    fn maps(&self) -> Option<Maps> {
        let t = &self.t;
        let count = |at: Option<usize>| at.map_or(0, |at| t.u32(at));
        let (mut conditions, mut lists, mut vars) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        let mut nulls = false;
        self.components(|_, c| {
            if c.flags & HAVE_CONDITION != 0 {
                if self.real_condition(c.condition).is_some() {
                    conditions.insert(c.condition);
                } else {
                    nulls = true;
                }
            }
            if c.flags & HAVE_AXES != 0 {
                (c.axis_list < count(t.axis_indices)).then(|| lists.insert(c.axis_list))?;
            }
            for (flag, var) in [(AXIS_VALUES_HAVE_VARIATION, c.axis_var), (TRANSFORM_HAS_VARIATION, c.transform_var)] {
                if c.flags & flag != 0 && var != NO_VARIATION {
                    vars.insert(var);
                }
            }
            Some(())
        })?;
        self.condition_vars(conditions.iter().filter_map(|&i| t.condition(i)).collect(), &mut vars)?;
        let mut rows: BTreeMap<u32, BTreeSet<u32>> = BTreeMap::new();
        for &var in &vars {
            let (outer, inner) = (var >> 16, var & 0xFFFF);
            (inner < self.rows_in(outer)?).then(|| rows.entry(outer).or_default().insert(inner))?;
        }
        let ranked = |set: &BTreeSet<u32>| set.iter().enumerate().map(|(i, &old)| (old, i as u32)).collect();
        let mut var_map = BTreeMap::new();
        for (new_outer, (&outer, inners)) in rows.iter().enumerate() {
            for (new_inner, &inner) in inners.iter().enumerate() {
                var_map.insert(outer << 16 | inner, (new_outer as u32) << 16 | new_inner as u32);
            }
        }
        Some(Maps { nulls, conditions: ranked(&conditions), lists: ranked(&lists), rows, vars: var_map })
    }

    // Where condition `index` starts; None for a null one (no offset, an index past the list, format 0), which holds.
    fn real_condition(&self, index: u32) -> Option<usize> {
        self.t.condition(index).filter(|&at| self.t.u16(at) != 0)
    }

    // How many rows data set `outer` holds; None past the store's data sets.
    fn rows_in(&self, outer: u32) -> Option<u32> {
        let store = self.t.store?;
        (outer < u32::from(self.t.u16(store + 6))).then_some(())?;
        Some(self.data_set(outer).and_then(|set| self.delta_sets(set)).map_or(0, |at| self.t.u32(at)))
    }

    fn data_set(&self, outer: u32) -> Option<usize> {
        let store = self.t.store?;
        self.offset(store, store + 8 + 4 * outer as usize)
    }

    fn delta_sets(&self, set: usize) -> Option<usize> {
        self.offset(set, set + 3 + 2 * usize::from(self.t.u16(set + 1)))
    }

    // The 32-bit offset at `at` from `base`, None where it's null.
    fn offset(&self, base: usize, at: usize) -> Option<usize> {
        let off = self.t.u32(at);
        (off != 0).then(|| self.t.at(base, off))
    }

    // A 24-bit child offset of the condition at `at`.
    fn child(&self, at: usize, field: usize) -> Option<usize> {
        let off = (0..3).fold(0, |v, b| v << 8 | usize::from(self.t.t.get(at + field + b).copied().unwrap_or(0)));
        (off != 0).then_some(at + off)
    }

    // The variation indices the conditions at `pending` and those inside them name; None for an unknown format.
    // Conditions may share children, so each is read once.
    fn condition_vars(&self, mut pending: Vec<usize>, vars: &mut BTreeSet<u32>) -> Option<()> {
        let mut seen = BTreeSet::new();
        while let Some(at) = pending.pop() {
            if !seen.insert(at) {
                continue;
            }
            match self.t.u16(at) {
                0 | 1 => {}
                2 => {
                    let var = self.t.u32(at + 4);
                    if var != NO_VARIATION {
                        vars.insert(var);
                    }
                }
                3 | 4 => {
                    let count = usize::from(self.t.t.get(at + 2).copied().unwrap_or(0));
                    pending.extend((0..count).filter_map(|i| self.child(at, 3 + 3 * i)));
                }
                5 => pending.extend(self.child(at, 2)),
                _ => return None,
            }
        }
        Some(())
    }

    fn write(&self) -> Option<Vec<u8>> {
        let maps = self.maps()?;
        let rebuilt = self.rebuilt(&maps)?;
        let records = self.records(&rebuilt);
        let lists = self.lists(&maps)?;
        let mut p = Packer::within(subset_budget(self.t.t.len()));
        let gids: Vec<u32> = self.kept.iter().map(|&(_, new)| new).collect();
        let coverage = p.pack(Object::leaf(coverage(&gids)?))?;
        let conditions = match maps.conditions.is_empty() {
            true => None,
            false => Some(self.condition_list(&maps, &mut p)?),
        };
        let lists = match maps.lists.is_empty() {
            true => None,
            false => Some(p.index(&lists)?),
        };
        let store = match maps.vars.is_empty() {
            true => None,
            false => Some(self.store(&maps, &mut p)?),
        };
        let records = p.index(&records)?;
        let mut root = Object::leaf(self.t.t.get(..4)?.to_vec());
        root.bytes.resize(24, 0);
        for (i, to) in [Some(coverage), store, conditions, lists, Some(records)].into_iter().enumerate() {
            root.links.extend(to.map(|to| (4 + 4 * i, 4, to)));
        }
        p.finish(root)
    }

    // The kept records rebuilt where ids or indices move, a record many glyphs share once: flags at their shortest,
    // new glyph ids (24 bits past 65,535), indices mapped. Empty where all stay; None where one doesn't read or map.
    fn rebuilt(&self, maps: &Maps) -> Option<Rebuilt> {
        let mut out = Rebuilt::new();
        let same = maps.conditions.iter().chain(&maps.lists).chain(&maps.vars).all(|(old, new)| old == new);
        if self.retain && same && !maps.nulls {
            return Some(out);
        }
        for &(index, _) in &self.kept {
            let mut r = self.record(index);
            let Entry::Vacant(slot) = out.entry((r.as_ptr(), r.len())) else { continue };
            let mut written = Vec::new();
            while !r.is_empty() {
                let c = decompile(&self.t, r)?;
                self.component(&r[..c.size], &c, maps, &mut written)?;
                r = &r[c.size..];
            }
            slot.insert(written);
        }
        Some(out)
    }

    // Each kept record as written, in the order of the glyphs kept: as rebuilt, else as the source holds it.
    fn records<'r>(&'r self, rebuilt: &'r Rebuilt) -> Vec<&'r [u8]> {
        let record = |r: &'r [u8]| rebuilt.get(&(r.as_ptr(), r.len())).map_or(r, Vec::as_slice);
        self.kept.iter().map(|&(index, _)| record(self.record(index))).collect()
    }

    fn component(&self, r: &[u8], c: &super::measure::Component, maps: &Maps, out: &mut Vec<u8>) -> Option<()> {
        let gid = if self.retain { Some(c.gid) } else { (self.new_gid)(c.gid) };
        let gid = gid.filter(|&g| g <= 0xFF_FFFF)?;
        let mut flags = if gid > 0xFFFF { c.flags | GID_IS_24BIT } else { c.flags & !GID_IS_24BIT };
        // A null condition holds, so the component is written without one.
        if !maps.conditions.contains_key(&c.condition) {
            flags &= !HAVE_CONDITION;
        }
        varint(out, flags);
        let bytes = gid.to_be_bytes();
        out.extend_from_slice(&bytes[if flags & GID_IS_24BIT != 0 { 1 } else { 2 }..]);
        let mut cursor = c.spans[GID].0 + c.spans[GID].1;
        let var = |v: u32| if v == NO_VARIATION { Some(v) } else { maps.vars.get(&v).copied() };
        let fields = [
            (HAVE_CONDITION, CONDITION, maps.conditions.get(&c.condition).copied()),
            (HAVE_AXES, AXIS_LIST, maps.lists.get(&c.axis_list).copied()),
            (AXIS_VALUES_HAVE_VARIATION, AXIS_VAR, var(c.axis_var)),
            (TRANSFORM_HAS_VARIATION, TRANSFORM_VAR, var(c.transform_var)),
        ];
        for (flag, span, mapped) in fields {
            if c.flags & flag != 0 {
                let (at, len) = c.spans[span];
                out.extend_from_slice(r.get(cursor..at)?);
                if flags & flag != 0 {
                    varint(out, mapped?);
                }
                cursor = at + len;
            }
        }
        out.extend_from_slice(r.get(cursor..)?);
        Some(())
    }

    // The kept axis lists in new order; in an instance only empty ones, as every axis a list names is pinned.
    fn lists(&self, maps: &Maps) -> Option<Vec<&[u8]>> {
        let entry = |old: u32| self.t.entry(self.t.axis_indices, old);
        let lists: Vec<&[u8]> = maps.lists.keys().map(|&old| entry(old)).collect();
        (!self.pinned || lists.iter().all(|l| l.is_empty())).then_some(lists)
    }

    fn condition_list(&self, maps: &Maps, p: &mut Packer) -> Option<usize> {
        let mut list = Object::leaf((maps.conditions.len() as u32).to_be_bytes().to_vec());
        let mut written = BTreeMap::new();
        for (i, &old) in maps.conditions.keys().enumerate() {
            let to = self.condition(self.real_condition(old), maps, p, &mut written)??;
            list.bytes.extend([0; 4]);
            list.links.push((4 + 4 * i, 4, to));
        }
        p.pack(list)
    }

    // A condition as a subset writes it; Some(None) for a null one, which holds as a null offset does, so it's no
    // object. Conditions may share children, so each is written once, its object kept in `written` by its offset; a
    // group or negation is packed after its children, its offsets linked to theirs.
    fn condition(
        &self, at: Option<usize>, maps: &Maps, p: &mut Packer, written: &mut BTreeMap<usize, usize>,
    ) -> Option<Option<usize>> {
        let t = &self.t;
        // Groups and negations being written, each with its object and its next child, and the condition to start.
        let mut open: Vec<(usize, Object, usize)> = Vec::new();
        let mut start = Some(at);
        loop {
            let to = match start.take() {
                Some(at) => match at.filter(|&at| t.u16(at) != 0) {
                    None => None,
                    Some(at) => match written.get(&at) {
                        Some(&object) => Some(object),
                        None if matches!(t.u16(at), 3..=5) => {
                            open.push((at, self.condition_object(at, maps)?, 0));
                            continue;
                        }
                        None => Some(Self::packed(at, self.condition_object(at, maps)?, p, written)?),
                    },
                },
                None => {
                    let (at, o, next) = open.last_mut()?;
                    let count = if t.u16(*at) == 5 { 1 } else { usize::from(o.bytes[2]) };
                    if *next < count {
                        start = Some(self.child(*at, child_field(t.u16(*at), *next)));
                        *next += 1;
                        continue;
                    }
                    let (at, o, _) = open.pop()?;
                    Some(Self::packed(at, o, p, written)?)
                }
            };
            let Some((at, o, next)) = open.last_mut() else { return Some(to) };
            o.links.extend(to.map(|to| (child_field(t.u16(*at), *next - 1), 3, to)));
        }
    }

    // Condition `at`'s object, a variation index mapped, an axis refused in an instance, and a group's or negation's
    // offsets 0 until linked; None for one a subset can't write.
    fn condition_object(&self, at: usize, maps: &Maps) -> Option<Object> {
        let t = &self.t;
        let mut o = Object::leaf(t.t.get(at..at + 2)?.to_vec());
        match t.u16(at) {
            1 if !self.pinned => o.bytes = t.t.get(at..at + 8)?.to_vec(),
            2 => {
                let var = t.u32(at + 4);
                let var = if var == NO_VARIATION { var } else { *maps.vars.get(&var)? };
                o.bytes.extend_from_slice(t.t.get(at + 2..at + 4)?);
                o.bytes.extend(var.to_be_bytes());
            }
            3 | 4 => {
                let count = *t.t.get(at + 2)?;
                o.bytes.push(count);
                o.bytes.resize(3 + 3 * usize::from(count), 0);
            }
            5 => o.bytes.extend([0; 3]),
            _ => return None,
        }
        Some(o)
    }

    fn packed(at: usize, o: Object, p: &mut Packer, written: &mut BTreeMap<usize, usize>) -> Option<usize> {
        let object = p.pack(o)?;
        written.insert(at, object);
        Some(object)
    }

    // The store holding the kept rows: every region a kept data set names, renumbered, each data set's rows in new
    // order. A region axis is refused in an instance, and a null one anywhere.
    fn store(&self, maps: &Maps, p: &mut Packer) -> Option<usize> {
        let (t, store) = (&self.t, self.t.store?);
        let list = self.offset(store, store + 2);
        let regions = list.map_or(0, |at| t.u32(at));
        let indices = |set: Option<usize>| {
            let (at, n) = set.map_or((0, 0), |at| (at, usize::from(t.u16(at + 1))));
            (0..n).map(move |i| u32::from(t.u16(at + 3 + 2 * i)))
        };
        let sets: Vec<Option<usize>> = maps.rows.keys().map(|&outer| self.data_set(outer)).collect();
        let used: BTreeSet<u32> = sets.iter().flat_map(|&set| indices(set)).filter(|&r| r < regions).collect();
        let region_map: BTreeMap<u32, u32> = used.iter().enumerate().map(|(i, &old)| (old, i as u32)).collect();

        let mut region_list = Object::leaf((used.len() as u32).to_be_bytes().to_vec());
        for (i, &old) in used.iter().enumerate() {
            let region = list.and_then(|list| self.offset(list, list + 4 + 4 * old as usize));
            let axes = region.map_or(0, |at| usize::from(t.u16(at)));
            let mut o = Object::leaf((axes as u16).to_be_bytes().to_vec());
            for a in 0..axes {
                let region = region?;
                let axis = self.offset(region, region + 2 + 4 * a).filter(|_| !self.pinned)?;
                let to = p.pack(Object::leaf(t.t.get(axis..axis + 8)?.to_vec()))?;
                o.bytes.extend([0; 4]);
                o.links.push((2 + 4 * a, 4, to));
            }
            let to = p.pack(o)?;
            region_list.bytes.extend([0; 4]);
            region_list.links.push((4 + 4 * i, 4, to));
        }
        let region_list = p.pack(region_list)?;

        let mut out = Object::leaf(vec![0, 1, 0, 0, 0, 0]);
        out.links.push((2, 4, region_list));
        out.bytes.extend((sets.len() as u16).to_be_bytes());
        for (i, (set, inners)) in sets.iter().zip(maps.rows.values()).enumerate() {
            let mut o = Object::leaf(vec![1]);
            let names: Vec<u32> = indices(*set).collect();
            o.bytes.extend((names.len() as u16).to_be_bytes());
            for r in names {
                o.bytes.extend((*region_map.get(&r)? as u16).to_be_bytes());
            }
            let deltas = set.and_then(|set| self.delta_sets(set));
            let rows: Vec<&[u8]> = inners.iter().map(|&inner| t.entry(deltas, inner)).collect();
            let to = p.index(&rows)?;
            o.links.push((o.bytes.len(), 4, to));
            o.bytes.extend([0; 4]);
            let to = p.pack(o)?;
            out.links.push((8 + 4 * i, 4, to));
            out.bytes.extend([0; 4]);
        }
        p.pack(out)
    }
}

// Where a group's child `i`'s 24-bit offset lies, or a negation's (format 5) one.
fn child_field(format: u16, i: usize) -> usize {
    if format == 5 { 2 } else { 3 + 3 * i }
}

// An object of the serializer: its bytes and its offsets to others, each as where it lies, its width and the
// object it names.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Object {
    bytes: Vec<u8>,
    links: Vec<(usize, usize, usize)>,
}

impl Object {
    fn leaf(bytes: Vec<u8>) -> Object {
        Object { bytes, links: Vec::new() }
    }
}

// The serializer as VARC needs it: each object packed once finished, its children before it, one the same as an
// earlier one (bytes and offsets) taken as that one; laid out root first, then last packed first, so offsets run on.
struct Packer {
    objects: Vec<Object>,
    found: BTreeMap<Object, usize>,
    // What may still be packed, as records may share or overlap their bytes; a pack past it is None.
    left: usize,
}

impl Packer {
    fn within(budget: usize) -> Packer {
        Packer { objects: Vec::new(), found: BTreeMap::new(), left: budget }
    }

    fn pack(&mut self, o: Object) -> Option<usize> {
        if let Some(&i) = self.found.get(&o) {
            return Some(i);
        }
        self.left = self.left.checked_sub(o.bytes.len())?;
        self.objects.push(o.clone());
        self.found.insert(o, self.objects.len() - 1);
        Some(self.objects.len() - 1)
    }

    // An INDEX of `entries`, measured against what's left before it's built.
    fn index(&mut self, entries: &[&[u8]]) -> Option<usize> {
        (entries.iter().fold(0usize, |n, e| n.saturating_add(e.len())) <= self.left).then_some(())?;
        self.pack(Object::leaf(index(entries)))
    }

    // The table, None where an offset doesn't fit its width.
    fn finish(self, root: Object) -> Option<Vec<u8>> {
        let mut at = vec![0; self.objects.len()];
        let mut end = root.bytes.len();
        for (i, o) in self.objects.iter().enumerate().rev() {
            at[i] = end;
            end += o.bytes.len();
        }
        let mut out = Vec::with_capacity(end);
        let packed = self.objects.iter().enumerate().rev().map(|(i, o)| (at[i], o));
        for (start, o) in core::iter::once((0, &root)).chain(packed) {
            let mut bytes = o.bytes.clone();
            for &(field, w, to) in &o.links {
                let offset = at[to] - start;
                (offset as u64 >> (8 * w) == 0).then_some(())?;
                bytes[field..field + w].copy_from_slice(&(offset as u32).to_be_bytes()[4 - w..]);
            }
            out.extend(bytes);
        }
        Some(out)
    }
}

// Coverage as a subset writes it, the smaller of a list and ranges, a tie to the list, which needs ascending glyphs;
// ranges sorted by their first glyph, each 24-bit format past glyph 65,535. None past 24 bits.
fn coverage(gids: &[u32]) -> Option<Vec<u8>> {
    let max = gids.iter().copied().max().unwrap_or(0);
    (max <= 0xFF_FFFF).then_some(())?;
    let (w, wide) = if max > 0xFFFF { (3, 2) } else { (2, 0) };
    let starts = |g: &[u32]| (0..g.len()).filter(|&i| i == 0 || g[i - 1].wrapping_add(1) != g[i]).count();
    let ranges = starts(gids);
    let sorted = gids.windows(2).all(|p| p[0] <= p[1]);
    let put = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_be_bytes()[4 - w..]);
    let mut out = Vec::new();
    if sorted && gids.len() <= ranges * 3 {
        out.extend((1 + wide as u16).to_be_bytes());
        put(&mut out, gids.len() as u32);
        gids.iter().for_each(|&g| put(&mut out, g));
        return Some(out);
    }
    let mut runs: Vec<(u32, u32, u32)> = Vec::new();
    for (i, &g) in gids.iter().enumerate() {
        match runs.last_mut() {
            Some(run) if run.1.wrapping_add(1) == g => run.1 = g,
            _ => runs.push((g, g, i as u32)),
        }
    }
    runs.sort_by_key(|run| run.0);
    out.extend((2 + wide as u16).to_be_bytes());
    put(&mut out, runs.len() as u32);
    runs.iter().for_each(|&(first, last, index)| [first, last, index].iter().for_each(|&v| put(&mut out, v)));
    Some(out)
}

// A CFF2-style INDEX of `entries`, its offsets from 1 in as few bytes as hold the last; a count of 0 alone.
fn index(entries: &[&[u8]]) -> Vec<u8> {
    let mut out = (entries.len() as u32).to_be_bytes().to_vec();
    if entries.is_empty() {
        return out;
    }
    let size: usize = entries.iter().map(|e| e.len()).sum();
    let w = (usize::BITS - (size + 1).leading_zeros()).div_ceil(8) as usize;
    out.push(w as u8);
    let mut offset = 1usize;
    for len in core::iter::once(0).chain(entries.iter().map(|e| e.len())) {
        offset += len;
        out.extend_from_slice(&(offset as u64).to_be_bytes()[8 - w..]);
    }
    entries.iter().for_each(|e| out.extend_from_slice(e));
    out
}

// A variable-length number at its shortest.
fn varint(out: &mut Vec<u8>, v: u32) {
    let len = match v {
        0..0x80 => 1,
        0x80..0x4000 => 2,
        0x4000..0x20_0000 => 3,
        0x20_0000..0x1000_0000 => 4,
        _ => 5,
    };
    let at = out.len();
    out.extend_from_slice(&u64::from(v).to_be_bytes()[8 - len..]);
    out[at] |= (((1u32 << (len - 1)) - 1) << (9 - len)) as u8;
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/varc/subset.rs"]
mod tests;
