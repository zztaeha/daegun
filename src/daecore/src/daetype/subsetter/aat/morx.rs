use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be};
use super::super::super::format::aat::Lookup;
use super::lookup::build_aat_lookup;
use super::state::{assemble, entries_of, entry_table, present, reachable_entries, remap_aat, state_parts, subset_state_table};

const NO_INDEX: u16 = 0xFFFF;
const DELETED_GLYPH: u16 = 0xFFFF;
const HEADER_WITH_ONE_OFFSET: usize = 20;
const LIGATURE_HEADER: usize = 28;
const PERFORM_ACTION: u16 = 0x2000;
const LIG_LAST: u32 = 0x8000_0000;
const LIG_STORE: u32 = 0x4000_0000;
const LIG_OFFSET: u32 = 0x3FFF_FFFF;
const LIG_SIGN: u32 = 0x2000_0000;
const CURRENT_INSERT_COUNT: u16 = 0x03E0;
const MARKED_INSERT_COUNT: u16 = 0x001F;

// The pairs whose glyphs survive. A substitution to the deleted glyph still deletes.
fn remap_pairs(lookup: &Lookup, active: &GlyphSet, gid_map: &[u16], num_glyphs: u16) -> Vec<(u16, u16)> {
    lookup.entries().into_iter()
        .filter_map(|(from, to)| {
            Some((remap_aat(active, gid_map, num_glyphs, from)?, remap_aat(active, gid_map, num_glyphs, to)?))
        })
        .collect()
}

fn subset_non_contextual(
    body: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    build_aat_lookup(&remap_pairs(&Lookup::parse(body, num_glyphs)?, active, gid_map, num_glyphs))
}

fn subset_contextual(
    body: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let subst_off = read_u32_be(body, 16)? as usize;
    let parts = state_parts(body, &[subst_off], active, gid_map, num_glyphs)?;

    let n_lookups = entries_of(parts.entry, 2).iter()
        .flat_map(|&(_, w1, w2)| [w1, w2])
        .filter(|&w| w != NO_INDEX)
        .map(|w| w as usize + 1)
        .max()
        .unwrap_or(0);

    // Indices may share a lookup, rebuilt once and named by each. One that cannot be read names an
    // empty lookup: these offsets have no null value, so 0 would name the first lookup.
    let substitutions = body.get(subst_off..)?;
    let table_len = 4 * n_lookups;
    let mut offsets: Vec<u32> = Vec::with_capacity(n_lookups);
    let mut blob: Vec<u8> = Vec::new();
    let mut placed: BTreeMap<Option<u32>, u32> = BTreeMap::new();
    for i in 0..n_lookups {
        let off = read_u32_be(substitutions, 4 * i)
            .filter(|&o| substitutions.get(o as usize..).and_then(|d| Lookup::parse(d, num_glyphs)).is_some());
        if let Some(&at) = placed.get(&off) {
            offsets.push(at);
            continue;
        }
        let pairs = off
            .and_then(|o| Lookup::parse(substitutions.get(o as usize..)?, num_glyphs))
            .map(|lookup| remap_pairs(&lookup, active, gid_map, num_glyphs))
            .unwrap_or_default();
        let at = u32::try_from(table_len + blob.len()).ok()?;
        blob.extend_from_slice(&build_aat_lookup(&pairs)?);
        if blob.len() > super::super::subset_budget(body.len()) { return None; }
        placed.insert(off, at);
        offsets.push(at);
    }

    let mut table: Vec<u8> = Vec::with_capacity(table_len + blob.len());
    for o in &offsets { table.extend_from_slice(&o.to_be_bytes()); }
    table.extend_from_slice(&blob);

    let mut out = assemble(&parts, HEADER_WITH_ONE_OFFSET, &[&table])?;
    let subst_at = (out.len() - table.len()) as u32;
    out.get_mut(16..20)?.copy_from_slice(&subst_at.to_be_bytes());
    Some(out)
}

fn subset_insertion(
    body: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let action_off = read_u32_be(body, 16)? as usize;
    let parts = state_parts(body, &[action_off], active, gid_map, num_glyphs)?;

    let n_glyphs_in_table = entries_of(parts.entry, 2).iter()
        .flat_map(|&(flags, w1, w2)| [
            (w1, (flags & CURRENT_INSERT_COUNT) >> 5),
            (w2, flags & MARKED_INSERT_COUNT),
        ])
        .filter(|&(w, _)| w != NO_INDEX)
        .map(|(w, count)| w as usize + count as usize)
        .max()
        .unwrap_or(0);

    let actions = body.get(action_off..)?;
    let mut table: Vec<u8> = Vec::with_capacity(2 * n_glyphs_in_table);
    for i in 0..n_glyphs_in_table {
        let g = read_u16_be(actions, 2 * i)?;
        let new = remap_aat(active, gid_map, num_glyphs, g).unwrap_or(DELETED_GLYPH);
        table.extend_from_slice(&new.to_be_bytes());
    }

    let mut out = assemble(&parts, HEADER_WITH_ONE_OFFSET, &[&table])?;
    let action_at = (out.len() - table.len()) as u32;
    out.get_mut(16..20)?.copy_from_slice(&action_at.to_be_bytes());
    Some(out)
}

// A ligature subtable's action, component and ligature arrays, each running to the next part the
// header names, so the closure and the subset read the same lengths.
struct LigatureLayout {
    action: usize,
    component: usize,
    ligature: usize,
    n_actions: usize,
    n_components: usize,
    n_ligatures: usize,
}

fn ligature_layout(body: &[u8]) -> Option<LigatureLayout> {
    let starts: Vec<usize> = (1..7).map(|i| read_u32_be(body, i * 4).map(|v| v as usize)).collect::<Option<_>>()?;
    let end_of = |from: usize| starts.iter().copied().chain([body.len()]).filter(|&o| o > from).min().unwrap_or(body.len());
    let (action, component, ligature) = (starts[3], starts[4], starts[5]);
    Some(LigatureLayout {
        action,
        component,
        ligature,
        n_actions: end_of(action).saturating_sub(action) / 4,
        n_components: end_of(component).saturating_sub(component) / 2,
        n_ligatures: end_of(ligature).saturating_sub(ligature) / 2,
    })
}

fn action_offset(word: u32) -> i64 {
    let offset = i64::from(word & LIG_OFFSET);
    if word & LIG_SIGN != 0 { offset - i64::from(LIG_OFFSET) - 1 } else { offset }
}

// The glyphs the class table names.
fn class_glyphs(body: &[u8], num_glyphs: u16) -> Option<Vec<u16>> {
    let class_off = read_u32_be(body, 4)? as usize;
    Some(Lookup::parse(body.get(class_off..)?, num_glyphs)?.entries().into_iter().map(|(g, _)| g).collect())
}

// Where an action's components land: at the action's own offset when those slots are free or hold
// the same values, as they always do in an identity subset, otherwise in a new range at the end.
fn place_components(
    components: &mut Vec<Option<u16>>, body: &[u8], layout: &LigatureLayout, classed: &[(u16, u16)], offset: i64,
) -> Option<i64> {
    let reaching: Vec<(u16, u16)> = classed.iter()
        .filter_map(|&(g, ng)| {
            let at = usize::try_from(i64::from(g) + offset).ok().filter(|&a| a < layout.n_components)?;
            Some((ng, read_u16_be(body, layout.component.checked_add(at * 2)?)?))
        })
        .collect();
    let Some(lo) = reaching.iter().map(|&(ng, _)| ng).min() else { return Some(offset) };
    let fits = reaching.iter().all(|&(ng, v)| {
        usize::try_from(i64::from(ng) + offset).is_ok_and(|at| components.get(at).is_none_or(|c| c.is_none_or(|c| c == v)))
    });
    let base = if fits { offset } else { components.len() as i64 - i64::from(lo) };
    for (ng, v) in reaching {
        let at = usize::try_from(i64::from(ng) + base).ok()?;
        if at >= components.len() { components.resize(at + 1, None); }
        components[at] = Some(v);
    }
    Some(base)
}

fn subset_ligature(
    body: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let layout = ligature_layout(body)?;
    let parts = state_parts(body, &[layout.action, layout.component, layout.ligature], active, gid_map, num_glyphs)?;
    if layout.n_actions == 0 || layout.n_components == 0 { return None; }
    let classed: Vec<(u16, u16)> = class_glyphs(body, num_glyphs)?.into_iter()
        .filter_map(|g| remap_aat(active, gid_map, num_glyphs, g).map(|ng| (g, ng)))
        .collect();

    // The glyphs an action reaches depend only on its offset, so actions sharing one share a range.
    let budget = super::super::subset_budget(body.len());
    let mut components: Vec<Option<u16>> = Vec::new();
    let mut placed: BTreeMap<i64, i64> = BTreeMap::new();
    let mut actions: Vec<u32> = Vec::with_capacity(layout.n_actions);
    for i in 0..layout.n_actions {
        let word = read_u32_be(body, layout.action.checked_add(i * 4)?)?;
        let offset = action_offset(word);
        let new_offset = match placed.get(&offset) {
            Some(&o) => o,
            None => {
                let o = place_components(&mut components, body, &layout, &classed, offset)?;
                if components.len() * 2 > budget { return None; }
                placed.insert(offset, o);
                o
            }
        };
        if !(-(1 << 29)..(1 << 29)).contains(&new_offset) { return None; }
        actions.push((word & (LIG_LAST | LIG_STORE)) | (new_offset as u32 & LIG_OFFSET));
    }

    // A ligature the closure left out is one no kept glyphs can form; .notdef keeps a mistake visible.
    let ligatures: Vec<u16> = (0..layout.n_ligatures)
        .map(|i| read_u16_be(body, layout.ligature.checked_add(i * 2)?).map(|g| remap_aat(active, gid_map, num_glyphs, g).unwrap_or(0)))
        .collect::<Option<_>>()?;

    let class_table = build_aat_lookup(&parts.classes)?;
    let new_class = LIGATURE_HEADER;
    let new_state = new_class + class_table.len();
    let new_entry = new_state + parts.state.len();
    let new_action = new_entry + parts.entry.len();
    let new_component = new_action + actions.len() * 4;
    let new_ligature = new_component + components.len() * 2;

    let mut out = alloc::vec![0u8; LIGATURE_HEADER];
    for (i, v) in [parts.n_classes as usize, new_class, new_state, new_entry, new_action, new_component, new_ligature]
        .into_iter().enumerate()
    {
        out.get_mut(i * 4..i * 4 + 4)?.copy_from_slice(&u32::try_from(v).ok()?.to_be_bytes());
    }
    out.extend_from_slice(&class_table);
    out.extend_from_slice(parts.state);
    out.extend_from_slice(parts.entry);
    for a in &actions { out.extend_from_slice(&a.to_be_bytes()); }
    for c in &components { out.extend_from_slice(&c.unwrap_or(0).to_be_bytes()); }
    for l in &ligatures { out.extend_from_slice(&l.to_be_bytes()); }
    Some(out)
}

fn subset_subtable(
    body: &[u8], coverage: u32, active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    match coverage & 0xFF {
        0 => subset_state_table(body, active, gid_map, num_glyphs),
        1 => subset_contextual(body, active, gid_map, num_glyphs),
        4 => subset_non_contextual(body, active, gid_map, num_glyphs),
        2 => subset_ligature(body, active, gid_map, num_glyphs),
        5 => subset_insertion(body, active, gid_map, num_glyphs),
        _ => None,
    }
}

// A chain's slice, where its subtables start and how many it names.
fn chain_at(morx: &[u8], at: usize) -> Option<(&[u8], usize, u32)> {
    let length = read_u32_be(morx, at.checked_add(4)?)? as usize;
    let n_features = read_u32_be(morx, at + 8)? as usize;
    let n_subtables = read_u32_be(morx, at + 12)?;
    let chain = morx.get(at..at.checked_add(length)?)?;
    let prefix = n_features.checked_mul(12)?.checked_add(16)?;
    (prefix <= chain.len()).then_some((chain, prefix, n_subtables))
}

// Like the shaper, a chain or subtable that cannot be read ends the walk, and what came before stands.
pub(crate) fn chains(morx: &[u8]) -> impl Iterator<Item = (&[u8], usize, u32)> {
    let mut at = 8usize;
    (0..read_u32_be(morx, 4).unwrap_or(0)).map_while(move |_| {
        let chain = chain_at(morx, at)?;
        at += chain.0.len();
        Some(chain)
    })
}

// Each subtable as its 12-byte header and its body.
fn subtables(chain: &[u8], mut at: usize, n: u32) -> impl Iterator<Item = (&[u8], &[u8])> {
    (0..n).map_while(move |_| {
        let length = read_u32_be(chain, at)? as usize;
        if length < 12 { return None; }
        let sub = chain.get(at..at.checked_add(length)?)?;
        at += length;
        Some(sub.split_at(12))
    })
}

pub fn morx_closure(morx: &[u8], active: &GlyphSet, num_glyphs: u16) -> Vec<u16> {
    let mut found = Vec::new();
    for (chain, prefix, n) in chains(morx) {
        for (header, body) in subtables(chain, prefix, n) {
            let Some(coverage) = read_u32_be(header, 4) else { continue };
            let _ = match coverage & 0xFF {
                1 => contextual_targets(body, active, num_glyphs, &mut found),
                2 => ligature_targets(body, active, num_glyphs, &mut found),
                4 => Lookup::parse(body, num_glyphs).map(|lookup| {
                    found.extend(lookup.entries().into_iter().filter(|&(from, _)| present(active, num_glyphs, from)).map(|(_, to)| to));
                }),
                5 => insertion_targets(body, active, num_glyphs, &mut found),
                _ => None,
            };
        }
    }
    found.retain(|g| *g != DELETED_GLYPH);
    found
}

// Only an active glyph's substitute in a lookup a reachable entry names.
fn contextual_targets(body: &[u8], active: &GlyphSet, num_glyphs: u16, found: &mut Vec<u16>) -> Option<()> {
    let subst_off = read_u32_be(body, 16)? as usize;
    let entry = entry_table(body, &[subst_off])?;
    let table = body.get(subst_off..)?;
    let indices: BTreeSet<u16> = reachable_entries(body, &[subst_off], 2, active, num_glyphs)?.into_iter()
        .flat_map(|e| [read_u16_be(entry, usize::from(e) * 8 + 4), read_u16_be(entry, usize::from(e) * 8 + 6)])
        .flatten()
        .filter(|&w| w != NO_INDEX)
        .collect();
    let offsets: BTreeSet<u32> = indices.into_iter().filter_map(|i| read_u32_be(table, 4 * usize::from(i))).collect();
    for off in offsets {
        let Some(lookup) = table.get(off as usize..).and_then(|d| Lookup::parse(d, num_glyphs)) else { continue };
        found.extend(lookup.entries().into_iter().filter(|&(from, _)| present(active, num_glyphs, from)).map(|(_, to)| to));
    }
    Some(())
}

// The glyphs a reachable entry inserts.
fn insertion_targets(body: &[u8], active: &GlyphSet, num_glyphs: u16, found: &mut Vec<u16>) -> Option<()> {
    let action_off = read_u32_be(body, 16)? as usize;
    let entry = entry_table(body, &[action_off])?;
    for e in reachable_entries(body, &[action_off], 2, active, num_glyphs)? {
        let at = usize::from(e) * 8;
        let Some(flags) = read_u16_be(entry, at + 2) else { continue };
        for (index, count) in [
            (read_u16_be(entry, at + 4), (flags & CURRENT_INSERT_COUNT) >> 5),
            (read_u16_be(entry, at + 6), flags & MARKED_INSERT_COUNT),
        ] {
            let Some(index) = index.filter(|&i| i != NO_INDEX) else { continue };
            for k in 0..usize::from(count) {
                let at = (usize::from(index) + k).checked_mul(2).and_then(|o| action_off.checked_add(o));
                if let Some(g) = at.and_then(|at| read_u16_be(body, at)) { found.push(g); }
            }
        }
    }
    Some(())
}

// The ligatures kept glyphs can form: each action adds the component its popped glyph reaches, so a
// chain reaches sums of those. Past the work bound every ligature is kept, which only keeps too much.
fn ligature_targets(body: &[u8], active: &GlyphSet, num_glyphs: u16, found: &mut Vec<u16>) -> Option<()> {
    let layout = ligature_layout(body)?;
    let extras = [layout.action, layout.component, layout.ligature];
    let entry = entry_table(body, &extras)?;
    let classed: Vec<u16> = class_glyphs(body, num_glyphs)?.into_iter().filter(|&g| present(active, num_glyphs, g)).collect();
    let starts: BTreeSet<usize> = reachable_entries(body, &extras, 1, active, num_glyphs)?.into_iter()
        .filter_map(|e| {
            let at = usize::from(e) * 6;
            (read_u16_be(entry, at + 2)? & PERFORM_ACTION != 0).then(|| read_u16_be(entry, at + 4).map(usize::from))?
        })
        .collect();

    let n = layout.n_ligatures;
    let mut work = super::super::subset_budget(body.len()).saturating_mul(64);
    let mut values: BTreeMap<i64, Vec<usize>> = BTreeMap::new();
    let mut reached = alloc::vec![false; n];
    'chains: for start in starts {
        let mut sums = alloc::vec![false; n];
        if let Some(s) = sums.first_mut() { *s = true; }
        for k in start..layout.n_actions {
            let Some(word) = read_u32_be(body, layout.action + k * 4) else { break };
            let offset = action_offset(word);
            if let alloc::collections::btree_map::Entry::Vacant(slot) = values.entry(offset) {
                let Some(left) = work.checked_sub(classed.len()) else { reached.fill(true); break 'chains };
                work = left;
                let mut vs: Vec<usize> = classed.iter()
                    .filter_map(|&g| usize::try_from(i64::from(g) + offset).ok().filter(|&a| a < layout.n_components))
                    .filter_map(|at| read_u16_be(body, layout.component + at * 2).map(usize::from))
                    .collect();
                vs.sort_unstable();
                vs.dedup();
                slot.insert(vs);
            }
            let vs = &values[&offset];
            let Some(left) = work.checked_sub(n.saturating_mul(vs.len())) else { reached.fill(true); break 'chains };
            work = left;
            let mut next = alloc::vec![false; n];
            for i in (0..n).filter(|&i| sums[i]) {
                for &v in vs.iter().take_while(|&&v| i + v < n) { next[i + v] = true; }
            }
            sums = next;
            if word & (LIG_LAST | LIG_STORE) != 0 {
                reached.iter_mut().zip(&sums).for_each(|(r, &s)| *r |= s);
            }
            if word & LIG_LAST != 0 { break; }
        }
    }
    for (i, _) in reached.iter().enumerate().filter(|&(_, &r)| r) {
        if let Some(g) = read_u16_be(body, layout.ligature + i * 2) { found.push(g); }
    }
    Some(())
}

pub fn subset_morx(
    morx: &[u8], active: &GlyphSet, gid_map: &[u16], num_glyphs: u16,
) -> Option<Vec<u8>> {
    let version = read_u16_be(morx, 0)?;
    let budget = super::super::subset_budget(morx.len());

    // A subtable that cannot be rebuilt is left out and its chain keeps the rest.
    let mut out = morx.get(..8)?.to_vec();
    let mut n_chains = 0u32;
    for (chain, prefix, n) in chains(morx) {
        let mut rebuilt = chain.get(..prefix)?.to_vec();
        let mut kept = 0u32;
        for (header, body) in subtables(chain, prefix, n) {
            let Some(mut new_body) = subset_subtable(body, read_u32_be(header, 4)?, active, gid_map, num_glyphs)
            else { continue };
            new_body.resize(new_body.len().next_multiple_of(4), 0);
            rebuilt.extend_from_slice(&u32::try_from(12 + new_body.len()).ok()?.to_be_bytes());
            rebuilt.extend_from_slice(header.get(4..12)?);
            rebuilt.extend_from_slice(&new_body);
            kept += 1;
            if out.len() + rebuilt.len() > budget { return None; }
        }
        rebuilt.get_mut(12..16)?.copy_from_slice(&kept.to_be_bytes());
        // Version 3 follows a chain's subtables with a coverage offset for each; 0 means none.
        if version >= 3 { rebuilt.resize(rebuilt.len() + 4 * kept as usize, 0); }
        let length = u32::try_from(rebuilt.len()).ok()?;
        rebuilt.get_mut(4..8)?.copy_from_slice(&length.to_be_bytes());
        out.extend_from_slice(&rebuilt);
        n_chains += 1;
    }
    if n_chains == 0 { return None; }
    out.get_mut(4..8)?.copy_from_slice(&n_chains.to_be_bytes());
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // One contextual subtable whose single entry names index 999, so 1,000 substitution offsets, all of
    // them naming one lookup of glyphs 1 to 1,000.
    fn morx() -> Vec<u8> {
        const N: u32 = 1_000;
        let mut body = [3u32, 20, 20, 20, 28].map(u32::to_be_bytes).concat();
        body.extend([0u16, 0, 999, NO_INDEX].map(u16::to_be_bytes).concat());
        (0..N).for_each(|_| body.extend((4 * N).to_be_bytes()));
        body.extend([8u16, 1, N as u16].map(u16::to_be_bytes).concat());
        body.extend((1..=N as u16).flat_map(|g| (g + 1_000).to_be_bytes()));
        let mut t = [2u16, 0].map(u16::to_be_bytes).concat();
        t.extend(1u32.to_be_bytes());
        t.extend([1u32, (16 + 12 + body.len()) as u32, 0, 1].map(u32::to_be_bytes).concat());
        t.extend([(12 + body.len()) as u32, 1, 1].map(u32::to_be_bytes).concat());
        t.extend(body);
        t
    }

    #[test]
    fn a_shared_contextual_lookup_is_closed_over_once() {
        let mut active = GlyphSet::new();
        active.insert(1);
        let found = morx_closure(&morx(), &active, 3_000);
        assert_eq!(found, [1_001], "glyph 1's one substitute, named 1,000 times, found {} times", found.len());
    }

    #[test]
    fn a_shared_contextual_lookup_is_written_once() {
        let t = morx();
        let mut active = GlyphSet::new();
        (0..=2_000).for_each(|g| { active.insert(g); });
        let gid_map: Vec<u16> = (0..=2_000).collect();
        let out = subset_morx(&t, &active, &gid_map, 3_000);
        let len = out.as_ref().map_or(0, Vec::len);
        assert!(out.is_some() && len <= 2 * t.len(), "a {}-byte morx subset to {len} bytes", t.len());
    }

    fn u16s(words: &[u16]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    // A version 2 morx of one chain declaring `declared` subtables and holding these (type, body).
    fn chain_of(declared: u32, subtables: &[(u32, Vec<u8>)]) -> Vec<u8> {
        let body: Vec<u8> = subtables.iter()
            .flat_map(|(kind, b)| [[(12 + b.len()) as u32, *kind, 1].map(u32::to_be_bytes).concat(), b.clone()].concat())
            .collect();
        [u16s(&[2, 0]), [1u32, 1, (16 + body.len()) as u32, 0, declared].map(u32::to_be_bytes).concat(), body].concat()
    }

    // Format 2: one segment of glyphs `first..=last` with one value.
    fn segment(first: u16, last: u16, value: u16) -> Vec<u8> {
        u16s(&[2, 6, 2, 12, 1, 0, last, first, value, 0xFFFF, 0xFFFF, 0])
    }

    fn everything(n: u16) -> (GlyphSet, Vec<u16>) {
        let mut active = GlyphSet::new();
        (0..n).for_each(|g| { active.insert(g); });
        (active, (0..n).collect())
    }

    // Noncontextual subtables each mapping glyphs 1 to 65,533 in one 18-byte segment. Rebuilt at four
    // bytes a glyph, 100 of them would come to 26 MB; each stays one segment.
    #[test]
    fn a_segment_stays_a_segment() {
        let t = chain_of(20, &(0..20).map(|_| (4, segment(1, 65_533, 7))).collect::<Vec<_>>());
        let (active, gid_map) = everything(u16::MAX);
        let out = subset_morx(&t, &active, &gid_map, u16::MAX).expect("a subset morx");
        assert!(out.len() <= t.len() + 400, "{} bytes of morx became {}", t.len(), out.len());
    }

    // A chain naming one subtable more than it holds keeps the ones it holds, as the shaper runs them.
    #[test]
    fn a_missing_last_subtable_keeps_the_ones_before() {
        let t = chain_of(2, &[(4, segment(1, 3, 2))]);
        let (active, gid_map) = everything(4);
        let out = subset_morx(&t, &active, &gid_map, 4).expect("the readable subtable stays");
        assert_eq!(read_u32_be(&out, 20), Some(1));
    }

    // 2,000 ligature actions at two offsets over 2,000 glyphs, each action its own range, would be 8 MB:
    // actions share their offset's range, which an identity subset places over the source's.
    #[test]
    fn actions_sharing_an_offset_share_their_components() {
        const N: u16 = 2_000;
        let row: &[u16] = &[0, 0, 0, 0, 0];
        let classes = segment(1, N, 4);
        let class_at = 28;
        let state_at = class_at + classes.len();
        let entry_at = state_at + 2 * row.len() * 2;
        let action_at = entry_at + 6;
        let component_at = action_at + 4 * usize::from(N);
        let ligature_at = component_at + 2 * usize::from(N);
        let mut body = [5u32, class_at as u32, state_at as u32, entry_at as u32, action_at as u32, component_at as u32, ligature_at as u32]
            .map(u32::to_be_bytes).concat();
        body.extend(classes);
        body.extend(u16s(row));
        body.extend(u16s(row));
        body.extend(u16s(&[0, 0, 0]));
        (0..N).for_each(|i| body.extend((0xBFFF_FFFFu32 - u32::from(i % 2)).to_be_bytes()));
        (0..N).for_each(|g| body.extend(g.to_be_bytes()));
        (0..N).for_each(|g| body.extend((g + 1).to_be_bytes()));
        let t = chain_of(1, &[(2, body)]);
        let (mut active, mut gid_map) = everything(N + 1);
        let out = subset_morx(&t, &active, &gid_map, N + 1).expect("a subset morx");
        assert_eq!(read_u32_be(&out, 20), Some(1), "the ligature subtable was dropped");
        assert!(out.len() <= t.len() + 64, "{} bytes of morx became {}", t.len(), out.len());

        // Dropping glyph 1,000 moves the second offset's values onto the first's, so it takes one new
        // range, not one per action.
        active = GlyphSet::new();
        (0..=N).filter(|&g| g != 1_000).for_each(|g| { active.insert(g); });
        gid_map.iter_mut().skip(1_000).for_each(|g| *g = g.saturating_sub(1));
        let out = subset_morx(&t, &active, &gid_map, N + 1).expect("a subset morx");
        assert_eq!(read_u32_be(&out, 20), Some(1), "the ligature subtable was dropped");
        assert!(out.len() <= t.len() + 4 * usize::from(N) + 64, "{} bytes of morx became {}", t.len(), out.len());
    }
}
