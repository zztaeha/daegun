use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::super::super::decoder::read_u16_be;
use super::super::super::format::aat::{
    class, end_of, initial_glyphs, start_classes, Entry, Lookup, StateTable, StxHeader,
};
use super::super::super::format::sanitize::state_reach;
use super::super::remap_gid16;
use super::lookup::build_class_lookup;
use crate::daecore::daeshaper::ot::morx::actionable as morx_acts;

const HEADER_LEN: usize = 16;

// Ids past the font's last glyph are placeholders morx passes between subtables (Zapfino's 32000
// to 32003) or the deleted 0xFFFF: they keep their ids, still past the subset's, and count as present.
pub(crate) fn remap_aat(active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, g: u16) -> Option<u16> {
    if u32::from(g) >= num_glyphs { Some(g) } else { remap_gid16(active, gid_map, g) }
}

pub(crate) fn present(active: &GlyphSet, num_glyphs: u32, g: u16) -> bool {
    u32::from(g) >= num_glyphs || active.contains(&g)
}

pub(crate) fn assemble(parts: &StateParts, header_len: usize, trailing: &[&[u8]]) -> Option<Vec<u8>> {
    let class_table = build_class_lookup(&parts.classes, parts.wide_unit)?;
    let class_off = header_len;
    let state_off = class_off + class_table.len();
    let entry_off = state_off + parts.state.len();

    let mut out = alloc::vec![0u8; header_len];
    for (i, v) in [parts.n_classes, class_off as u32, state_off as u32, entry_off as u32].iter().enumerate() {
        out.get_mut(i * 4..i * 4 + 4)?.copy_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&class_table);
    out.extend_from_slice(parts.state);
    out.extend_from_slice(parts.entry);
    for t in trailing { out.extend_from_slice(t); }
    Some(out)
}

pub(crate) struct StateParts<'a> {
    pub(crate) n_classes: u32,
    pub(crate) classes: Vec<(u16, u16)>,
    pub(crate) wide_unit: Option<usize>,
    pub(crate) state: &'a [u8],
    pub(crate) entry: &'a [u8],
}

// A state table's parts as a subset copies them, each to the next part its header, `extra_starts` or a morx subtable's
// end names, else the table's end; the state array and entry table run as far as the machine reaches from state 0.
struct Layout {
    header: StxHeader,
    starts: Vec<usize>,
    reach: [usize; 2],
}

impl Layout {
    fn read(data: &[u8], extra_starts: &[usize], extra_words: usize) -> Option<Self> {
        let header = StxHeader::read(data)?;
        let entry_size = 4 + 2 * extra_words;
        let (rows, entries) = state_reach(data, entry_size as u32).unwrap_or_default();
        let starts = header.starts().into_iter().chain(extra_starts.iter().copied()).collect();
        Some(Layout { header, starts, reach: [rows * 2 * header.n_classes, entries * entry_size] })
    }

    fn part<'a>(&self, data: &'a [u8], from: usize, reach: usize) -> Option<&'a [u8]> {
        data.get(from..end_of(&self.starts, from, data.len()).max(from + reach))
    }

    fn states<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        self.part(data, self.header.state, self.reach[0])
    }

    fn entries<'a>(&self, data: &'a [u8]) -> Option<&'a [u8]> {
        self.part(data, self.header.entry, self.reach[1])
    }
}

// Spends `n` of the `work` a writer's subtables share. Short of it, or once it's spent, None, and it stays spent so no
// later subtable is read only to be refused.
pub(crate) fn charge(work: &mut usize, n: usize) -> Option<()> {
    let rest = work.checked_sub(n).filter(|_| *work > 0);
    *work = rest.unwrap_or(0);
    rest.map(drop)
}

// Spends `n` of what a closure pass has left, which is None from then on once it runs out.
pub(crate) fn spend(left: &mut Option<usize>, n: usize) -> Option<()> {
    *left = left.and_then(|left| left.checked_sub(n));
    left.map(drop)
}

// `machine` holds how many extra words an entry has and which entries act, which the start set is read by. Subtables
// can share a machine and its class lookup, so each pays `work` for its arrays and the lookup's reads.
pub(crate) fn state_parts<'a>(
    data: &'a [u8], extra_starts: &[usize], machine: (usize, fn(&Entry) -> bool), active: &GlyphSet, gid_map: &[u32],
    num_glyphs: u32, work: &mut usize,
) -> Option<StateParts<'a>> {
    let (extra_words, actionable) = machine;
    let layout = Layout::read(data, extra_starts, extra_words)?;
    let (state, entry) = (layout.states(data)?, layout.entries(data)?);
    charge(work, state.len() + entry.len())?;
    if state.is_empty() || entry.is_empty() { return None; }
    let lookup = layout.header.classes(data, num_glyphs).ok()?;
    charge(work, lookup.reads())?;
    let table = StateTable::parse(data, extra_words, num_glyphs).ok().filter(|_| lookup.wide_unit().is_none());
    let classes = table.map_or_else(|| lookup.pairs(), |table| machine_classes(&lookup, &table, actionable));
    let remap = |(g, k)| Some((remap_aat(active, gid_map, num_glyphs, g)?, k));
    let classes = classes.into_iter().filter_map(remap).collect();
    Some(StateParts { n_classes: layout.header.n_classes as u32, classes, wide_unit: lookup.wide_unit(), state, entry })
}

// The class lookup's glyphs listed so that each reads its class as in the font, by search or 1 where that misses, and
// starts the machine where any of its units did: by a second unit in a starting class where its own class doesn't.
fn machine_classes(lookup: &Lookup, table: &StateTable, actionable: fn(&Entry) -> bool) -> Vec<(u16, u16)> {
    let filter = start_classes(table, actionable);
    let takes = |k: u16| filter.is_none_or(|c| c.takes(k.into()));
    let starter = (0..512).find(|&k| takes(k));
    let starts = initial_glyphs(table, actionable);
    let mut out = Vec::new();
    for g in lookup.glyph_set().glyphs().filter_map(|g| u16::try_from(g).ok()).filter(|&g| g != 0xFFFF) {
        let found = lookup.typed_value(g.into());
        let class = found.unwrap_or(class::OUT_OF_BOUNDS);
        let held = starts.contains(g.into());
        if found.is_some() || held {
            out.push((g, class));
        }
        if let Some(starter) = starter.filter(|_| held && !takes(class)) {
            out.push((g, starter));
        }
    }
    out
}

// A machine's state array and entry table, as the subset copies them.
pub(crate) fn machine_parts<'a>(
    data: &'a [u8], extra_starts: &[usize], extra_words: usize,
) -> Option<(&'a [u8], &'a [u8])> {
    let layout = Layout::read(data, extra_starts, extra_words)?;
    Some((layout.states(data)?, layout.entries(data)?))
}

// The entries a run can reach, with the entry table: from the two start states, through the classes active glyphs fall
// in and the four the shaper makes itself (end of text, out of bounds, deleted glyph, end of line). The arrays and the
// class lookup's reads spend `left`; they bound the walk, which reads each state's cells at most once.
pub(crate) fn reachable_entries<'a>(
    data: &'a [u8], extra_starts: &[usize], extra_words: usize, active: &GlyphSet, num_glyphs: u32,
    left: &mut Option<usize>,
) -> Option<(BTreeSet<u16>, &'a [u8])> {
    let layout = Layout::read(data, extra_starts, extra_words)?;
    let StxHeader { n_classes, state: state_off, .. } = layout.header;
    let (states, entry) = (layout.states(data)?, layout.entries(data)?);
    spend(left, states.len() + entry.len())?;
    let n_states = states.len() / (2 * n_classes);

    let mut classes: BTreeSet<u16> = (0..4).collect();
    let lookup = layout.header.classes(data, num_glyphs).ok()?;
    spend(left, lookup.reads())?;
    classes.extend(lookup.typed_entries().into_iter().filter(|&(g, _)| present(active, num_glyphs, g)).map(|(_, c)| c));
    classes.retain(|&c| usize::from(c) < n_classes);

    let (mut seen, mut reached) = (alloc::vec![false; n_states], BTreeSet::new());
    let mut queue: Vec<usize> = (0..n_states.min(2)).collect();
    queue.iter().for_each(|&s| seen[s] = true);
    while let Some(s) = queue.pop() {
        for &c in &classes {
            let Some(e) = read_u16_be(data, state_off + 2 * (s * n_classes + usize::from(c))) else { continue };
            reached.insert(e);
            let Some(next) = read_u16_be(entry, usize::from(e) * (4 + 2 * extra_words)).map(usize::from) else {
                continue;
            };
            if next < n_states && !seen[next] {
                seen[next] = true;
                queue.push(next);
            }
        }
    }
    Some((reached, entry))
}

pub(crate) fn entries_of(entry: &[u8], extra_words: usize) -> Vec<(u16, u16, u16)> {
    let stride = 4 + 2 * extra_words;
    (0..entry.len() / stride)
        .filter_map(|i| {
            let at = i * stride;
            Some((
                read_u16_be(entry, at + 2)?,
                if extra_words >= 1 { read_u16_be(entry, at + 4)? } else { 0 },
                if extra_words >= 2 { read_u16_be(entry, at + 6)? } else { 0 },
            ))
        })
        .collect()
}

pub(crate) fn subset_state_table(
    data: &[u8], extra_starts: &[usize], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, work: &mut usize,
) -> Option<Vec<u8>> {
    let parts = state_parts(data, extra_starts, (0, morx_acts::rearrangement), active, gid_map, num_glyphs, work)?;
    assemble(&parts, HEADER_LEN, &[])
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/aat/state.rs"]
mod tests;
