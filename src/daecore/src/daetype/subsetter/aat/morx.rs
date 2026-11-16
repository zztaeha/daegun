use crate::daecore::daeshaper::ot::morx::actionable;
use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::collections::btree_map::Entry;
use core::cmp::Ordering;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use super::super::super::decoder::{read_u16_be, read_u32_be};
use super::super::super::format::aat::{end_of, morx_chains, Lookup, MorxSubtable, StxHeader};
use super::lookup::{build_aat_lookup, build_class_lookup};
use super::state::{
    assemble, charge, entries_of, machine_parts, present, reachable_entries, remap_aat, spend, state_parts,
    subset_state_table,
};

const NO_INDEX: u16 = 0xFFFF;
const DELETED_GLYPH: u16 = 0xFFFF;
const HEADER_WITH_ONE_OFFSET: usize = 20;
const LIGATURE_HEADER: usize = 28;
const SET_COMPONENT: u16 = 0x8000;
const DONT_ADVANCE: u16 = 0x4000;
const PERFORM_ACTION: u16 = 0x2000;
const LIG_LAST: u32 = 0x8000_0000;
const LIG_STORE: u32 = 0x4000_0000;
const LIG_OFFSET: u32 = 0x3FFF_FFFF;
const LIG_SIGN: u32 = 0x2000_0000;
const CURRENT_INSERT_COUNT: u16 = 0x03E0;
const MARKED_INSERT_COUNT: u16 = 0x001F;

// The pairs whose glyphs survive. A substitution to the deleted glyph still deletes.
fn remap_pairs(pairs: Vec<(u16, u16)>, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32) -> Vec<(u16, u16)> {
    let remap = |g| remap_aat(active, gid_map, num_glyphs, g);
    let typed = pairs.into_iter().filter_map(|(g, to)| Some((remap(g)?, to)));
    typed.filter_map(|(from, to)| Some((from, remap(to)?))).collect()
}

fn subset_non_contextual(
    body: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, work: &mut usize,
) -> Option<Vec<u8>> {
    let lookup = Lookup::parse(body, num_glyphs).ok()?;
    charge(work, lookup.reads())?;
    build_aat_lookup(&remap_pairs(lookup.typed_entries(), active, gid_map, num_glyphs))
}

fn subset_contextual(
    body: &[u8], own: usize, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, work: &mut usize,
) -> Option<Vec<u8>> {
    let subst_off = read_u32_be(body, 16)? as usize;
    let parts = state_parts(body, &[subst_off, own], (2, actionable::contextual), active, gid_map, num_glyphs, work)?;

    let n_lookups = entries_of(parts.entry, 2).iter()
        .flat_map(|&(_, w1, w2)| [w1, w2])
        .filter(|&w| w != NO_INDEX)
        .map(|w| w as usize + 1)
        .max()
        .unwrap_or(0);
    charge(work, n_lookups)?;

    // Indices may share a lookup, rebuilt once and named by each. One that cannot be read names an
    // empty lookup: these offsets have no null value, so 0 would name the first lookup.
    let substitutions = body.get(subst_off..)?;
    let table_len = 4 * n_lookups;
    let mut offsets: Vec<u32> = Vec::with_capacity(n_lookups);
    let mut blob: Vec<u8> = Vec::new();
    let mut placed: BTreeMap<Option<u32>, u32> = BTreeMap::new();
    for i in 0..n_lookups {
        let raw = read_u32_be(substitutions, 4 * i);
        let lookup = raw.and_then(|o| Lookup::parse(substitutions.get(o as usize..)?, num_glyphs).ok());
        let off = raw.filter(|_| lookup.is_some());
        if let Some(&at) = placed.get(&off) {
            offsets.push(at);
            continue;
        }
        // Unlike the closure this expands lookups no reachable entry names, so their reads spend the shared `work`.
        if let Some(lookup) = &lookup { charge(work, lookup.reads())?; }
        let pairs = lookup.map(|lookup| lookup.typed_entries()).unwrap_or_default();
        let pairs = remap_pairs(pairs, active, gid_map, num_glyphs);
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

// An insertion subtable whose list runs past the table, inserting nothing: its glyphs go after the last chain, and
// each such list (an entry's offset in the body, and whether it's the current one) points past the new end.
struct Tail {
    glyphs: Vec<u8>,
    past: Vec<(usize, bool)>,
}

fn subset_insertion(
    body: &[u8], own: usize, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, work: &mut usize,
) -> Option<(Vec<u8>, Option<Tail>)> {
    let action_off = read_u32_be(body, 16)? as usize;
    let parts = state_parts(body, &[action_off, own], (2, actionable::insertion), active, gid_map, num_glyphs, work)?;

    // Each entry's lists, as where they end and whether each is the current one.
    let lists = |&(flags, w1, w2): &(u16, u16, u16)| {
        [(w1, (flags & CURRENT_INSERT_COUNT) >> 5, true), (w2, flags & MARKED_INSERT_COUNT, false)]
            .into_iter()
            .filter(|&(w, count, _)| w != NO_INDEX && count > 0)
            .map(|(w, count, current)| (usize::from(w) + usize::from(count), current))
    };
    let actions = body.get(action_off..)?;
    let entries = entries_of(parts.entry, 2);
    let n_glyphs_in_table = entries.iter().flat_map(lists).map(|(end, _)| end).filter(|&end| 2 * end <= actions.len());
    let n_glyphs_in_table = n_glyphs_in_table.max().unwrap_or(0);
    charge(work, n_glyphs_in_table)?;
    let mut table: Vec<u8> = Vec::with_capacity(2 * n_glyphs_in_table);
    for i in 0..n_glyphs_in_table {
        let g = read_u16_be(actions, 2 * i).unwrap_or(DELETED_GLYPH);
        let new = remap_aat(active, gid_map, num_glyphs, g).unwrap_or(DELETED_GLYPH);
        table.extend_from_slice(&new.to_be_bytes());
    }

    let past: Vec<(usize, bool)> = entries.iter().enumerate()
        .flat_map(|(e, entry)| lists(entry).filter(|&(end, _)| 2 * end > actions.len()).map(move |(_, c)| (e, c)))
        .collect();
    if !past.is_empty() {
        let out = assemble(&parts, HEADER_WITH_ONE_OFFSET, &[])?;
        let entry_at = read_u32_be(&out, 12)? as usize;
        let past = past.into_iter().map(|(e, current)| (entry_at + 8 * e, current)).collect();
        return Some((out, Some(Tail { glyphs: table, past })));
    }
    let mut out = assemble(&parts, HEADER_WITH_ONE_OFFSET, &[&table])?;
    let action_at = (out.len() - table.len()) as u32;
    out.get_mut(16..20)?.copy_from_slice(&action_at.to_be_bytes());
    Some((out, None))
}

// Each tail's glyphs after the last chain, its lists pointed a glyph past the end. A list a 16-bit index can't point
// that far takes a count of 0, inserting nothing as the font's does but spending no budget.
fn place_tails(out: &mut Vec<u8>, tails: &[(usize, Tail)]) -> Option<()> {
    let mut starts = Vec::with_capacity(tails.len());
    for (_, tail) in tails {
        starts.push(out.len());
        out.extend_from_slice(&tail.glyphs);
    }
    let end = out.len();
    for ((body, tail), start) in tails.iter().zip(starts) {
        out.get_mut(body + 16..body + 20)?.copy_from_slice(&u32::try_from(start - body).ok()?.to_be_bytes());
        let index = u16::try_from((end - start) / 2 + 1).ok().filter(|&i| i != NO_INDEX);
        for &(entry, current) in &tail.past {
            let at = body + entry;
            if let Some(index) = index {
                let field = at + if current { 4 } else { 6 };
                out.get_mut(field..field + 2)?.copy_from_slice(&index.to_be_bytes());
            } else {
                let count = if current { CURRENT_INSERT_COUNT } else { MARKED_INSERT_COUNT };
                let flags = read_u16_be(out, at + 2)? & !count;
                out.get_mut(at + 2..at + 4)?.copy_from_slice(&flags.to_be_bytes());
            }
        }
    }
    Some(())
}

// A ligature subtable's arrays, so the closure and the subset read the same lengths: each runs to the next of `starts`,
// the header's parts and the subtable's own end, and one past that end to the next part or the table's end.
struct LigatureLayout {
    starts: Vec<usize>,
    action: usize,
    component: usize,
    ligature: usize,
    n_actions: usize,
    n_components: usize,
    n_ligatures: usize,
}

fn ligature_layout(body: &[u8], own: usize) -> Option<LigatureLayout> {
    let mut starts: Vec<usize> = (1..7).map(|i| read_u32_be(body, i * 4).map(|v| v as usize)).collect::<Option<_>>()?;
    let (action, component, ligature) = (starts[3], starts[4], starts[5]);
    starts.push(own);
    let len = |from: usize| end_of(&starts, from, body.len()).saturating_sub(from);
    let (n_actions, n_components, n_ligatures) = (len(action) / 4, len(component) / 2, len(ligature) / 2);
    Some(LigatureLayout { starts, action, component, ligature, n_actions, n_components, n_ligatures })
}

fn action_offset(word: u32) -> i64 {
    let offset = i64::from(word & LIG_OFFSET);
    if word & LIG_SIGN != 0 { offset - i64::from(LIG_OFFSET) - 1 } else { offset }
}

// The glyphs a ligature subtable can push, with their classes: those its class table names, and where a state pushes
// class 1, every other kept glyph, which reads it. Beside them, what finding them read: its machine and class lookup.
fn pushed_glyphs(
    body: &[u8], layout: &LigatureLayout, active: &GlyphSet, num_glyphs: u32,
) -> Option<(Vec<(u16, u16)>, usize)> {
    let header = StxHeader::read(body)?;
    let lookup = header.classes(body, num_glyphs).ok()?;
    let mut listed = lookup.typed_entries();
    let (states, entry) = machine_parts(body, &layout.starts[3..], 1)?;
    let n_states = states.len() / (2 * header.n_classes);
    let pushes_class_1 = header.n_classes > 1 && (0..n_states).any(|s| {
        read_u16_be(body, header.state + 2 * (s * header.n_classes + 1))
            .and_then(|e| read_u16_be(entry, usize::from(e) * 6 + 2))
            .is_some_and(|flags| flags & SET_COMPONENT != 0)
    });
    if pushes_class_1 {
        let named: BTreeSet<u16> = listed.iter().map(|&(g, _)| g).collect();
        let unlisted = active.iter().filter_map(|g| u16::try_from(g).ok()).filter(|g| !named.contains(g));
        listed.extend(unlisted.map(|g| (g, 1)));
    }
    Some((listed, states.len() + entry.len() + lookup.reads()))
}

// The slots a morx's ligature components may try for their places, or 64 for each unit of its subset budget where
// that's more: five times the most a subset of Devanagari Sangam MN tries, 51 million.
const PLACING: usize = 1 << 28;

// Where an action's components land: its own offset when those slots are free or hold the same values, as in an
// identity subset, else the lowest where they do. `popped` is in glyph order; each glyph and slot tried spends `work`.
fn place_components(
    components: &mut Vec<Option<u16>>, body: &[u8], layout: &LigatureLayout, popped: &[(u16, u16)], offset: i64,
    work: &mut usize,
) -> Option<i64> {
    let before = |end: i64| popped.partition_point(|&(g, _)| i64::from(g) + offset < end);
    let within = popped.get(before(0)..before(i64::try_from(layout.n_components).ok()?)).unwrap_or_default();
    *work = work.checked_sub(within.len())?;
    let reaching: Vec<(u16, u16)> = within.iter()
        .filter_map(|&(g, ng)| {
            let at = usize::try_from(i64::from(g) + offset).ok()?;
            Some((ng, read_u16_be(body, layout.component.checked_add(at * 2)?)?))
        })
        .collect();
    let Some(lo) = reaching.iter().map(|&(ng, _)| ng).min() else { return Some(offset) };
    let free = |base: i64, (ng, v): (u16, u16)| {
        let free = |at: usize| components.get(at).is_none_or(|c| c.is_none_or(|c| c == v));
        usize::try_from(i64::from(ng) + base).is_ok_and(free)
    };
    let mut base = offset;
    if !reaching.iter().all(|&r| free(base, r)) {
        // From the lowest base each glyph's slot is tried in turn, first the one that last clashed, as it most likely
        // clashes again. Past the table's end every slot is free, so the search ends there at the latest.
        let (mut clash, n) = (0, reaching.len());
        base = -i64::from(lo);
        'search: loop {
            for i in (clash..n).chain(0..clash) {
                *work = work.checked_sub(1)?;
                if reaching.get(i).is_some_and(|&r| !free(base, r)) {
                    (clash, base) = (i, base + 1);
                    continue 'search;
                }
            }
            break;
        }
    }
    for (ng, v) in reaching {
        let at = usize::try_from(i64::from(ng) + base).ok()?;
        if at >= components.len() { components.resize(at + 1, None); }
        components[at] = Some(v);
    }
    Some(base)
}

// `popped` is the subtable's stacking over `active` where the closure kept it.
fn subset_ligature(
    body: &[u8], own: usize, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, popped: Option<&Popped>,
    work: &mut Work,
) -> Option<Vec<u8>> {
    let layout = ligature_layout(body, own)?;
    let starts = [layout.action, layout.component, layout.ligature, own];
    let parts = state_parts(body, &starts, (1, actionable::ligature), active, gid_map, num_glyphs, &mut work.reading)?;
    if layout.n_actions == 0 || layout.n_components == 0 { return None; }
    charge(&mut work.reading, layout.n_actions + layout.n_ligatures)?;
    let walked;
    let popped = match popped {
        Some(popped) => popped,
        None => {
            let (listed, read) = pushed_glyphs(body, &layout, active, num_glyphs)?;
            charge(&mut work.reading, listed.len() + read)?;
            walked = stacking(body, &layout, &listed, active, num_glyphs, &mut work.placing)?.popped;
            &walked
        }
    };
    let remapped = |glyphs: &[u16]| -> Vec<(u16, u16)> {
        glyphs.iter().filter_map(|&g| remap_aat(active, gid_map, num_glyphs, g).map(|ng| (g, ng))).collect()
    };
    let (reaching, any): (BTreeMap<i64, Vec<(u16, u16)>>, _) = match popped {
        Popped::Each(each) => (each.iter().map(|(&o, glyphs)| (o, remapped(glyphs))).collect(), Vec::new()),
        Popped::Any(glyphs) => (BTreeMap::new(), remapped(glyphs)),
    };

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
                let reached = match popped {
                    Popped::Each(_) => reaching.get(&offset).map_or(&[][..], Vec::as_slice),
                    Popped::Any(_) => &any,
                };
                let o = place_components(&mut components, body, &layout, reached, offset, &mut work.placing)?;
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
        .map(|i| {
            read_u16_be(body, layout.ligature.checked_add(i * 2)?)
                .map(|g| remap_aat(active, gid_map, num_glyphs, g).unwrap_or(0))
        })
        .collect::<Option<_>>()?;

    let class_table = build_class_lookup(&parts.classes, parts.wide_unit)?;
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

// What a morx's writer may spend across its subtables. Reading their machines and lookups, which many can share, may
// spend 64 for each unit of its subset budget; placing components and walking, at least PLACING.
struct Work {
    reading: usize,
    placing: usize,
}

#[allow(clippy::too_many_arguments, reason = "a ligature subtable reads two more than the others")]
fn subset_subtable(
    body: &[u8], own: usize, coverage: u32, active: &GlyphSet, gid_map: &[u32], num_glyphs: u32,
    popped: Option<&Popped>, work: &mut Work,
) -> Option<(Vec<u8>, Option<Tail>)> {
    // Each reads on through the table, so once reading is spent every later one is left out before it reads.
    let reading = &mut work.reading;
    if *reading == 0 { return None; }
    let rebuilt = match coverage & 0xFF {
        0 => subset_state_table(body, &[own], active, gid_map, num_glyphs, reading),
        1 => subset_contextual(body, own, active, gid_map, num_glyphs, reading),
        4 => subset_non_contextual(body, active, gid_map, num_glyphs, reading),
        2 => subset_ligature(body, own, active, gid_map, num_glyphs, popped, work),
        5 => return subset_insertion(body, own, active, gid_map, num_glyphs, reading),
        _ => None,
    };
    rebuilt.map(|body| (body, None))
}

#[cfg(test)]
pub fn morx_closure(morx: &[u8], active: &GlyphSet, num_glyphs: u32) -> Vec<u16> {
    let mut left = usize::MAX;
    Stackings::new(morx).reach(active, num_glyphs, &mut left).unwrap_or_default()
}

// By action offset, the glyphs its words can pop, as words sharing one read one range of components; or, where the
// walk gave up, the glyphs any word can pop, held once. Each in glyph order.
#[derive(PartialEq)]
enum Popped {
    Each(BTreeMap<i64, Vec<u16>>),
    Any(Vec<u16>),
}

// A morx's ligature stackings over the glyph set it last read, by chain and subtable. The closure reads each pass's
// set through one, and its last pass reads the set the subset keeps, so the subset reads them rather than walking.
pub(crate) struct Stackings<'a> {
    morx: &'a [u8],
    active: GlyphSet,
    popped: BTreeMap<(usize, usize), Popped>,
    // Each ligature subtable's pushed glyphs present at its last walk: all a walk reads of the set.
    kept: BTreeMap<(usize, usize), Vec<(u16, u16)>>,
    // What its walks may still spend, across subtables and passes, as many subtables can share one machine.
    walks: usize,
}

// What a closure's ligature walks may spend together for each unit of its morx's subset budget: nine times the most
// the corpus spends, Devanagari Sangam MN's 221.
const WALKS: usize = 1 << 11;

impl<'a> Stackings<'a> {
    pub(crate) fn new(morx: &'a [u8]) -> Self {
        let walks = super::super::subset_budget(morx.len()).saturating_mul(WALKS);
        Stackings { morx, active: GlyphSet::empty(), popped: BTreeMap::new(), kept: BTreeMap::new(), walks }
    }

    // The glyphs `active` (every set read before) reaches through morx, less some those sets reached: a noncontextual
    // subtable reads only new glyphs, an unchanged ligature subtable isn't rewalked. A closure keeps each pass's finds.
    // Each lookup a pass expands and each machine it reads spend `budget`; None past it.
    pub(crate) fn reach(&mut self, active: &GlyphSet, num_glyphs: u32, budget: &mut usize) -> Option<Vec<u16>> {
        let (mut found, mut left) = (GlyphSet::empty(), Some(*budget));
        let mut new = GlyphSet::empty();
        active.iter().filter(|g| !self.active.contains(g)).for_each(|g| _ = new.insert(g));
        for (c, chain) in morx_chains(self.morx).enumerate() {
            for (t, MorxSubtable { coverage, body, own, .. }) in chain.subtables(self.morx).enumerate() {
                let _ = match coverage & 0xFF {
                    1 => contextual_targets(body, own, active, num_glyphs, &mut left, &mut found),
                    2 => self.walk(body, own, (c, t), active, num_glyphs, &mut left, &mut found),
                    4 => Lookup::parse(body, num_glyphs)
                        .ok()
                        .and_then(|lookup| substituted(&lookup, &new, num_glyphs, &mut left, &mut found)),
                    5 => insertion_targets(body, own, active, num_glyphs, &mut left, &mut found),
                    _ => None,
                };
                *budget = left?;
            }
        }
        self.active = active.clone();
        Some(found.iter().filter_map(|g| u16::try_from(g).ok()).filter(|&g| g != DELETED_GLYPH).collect())
    }

    #[allow(clippy::too_many_arguments, reason = "a walk spends the pass's budget as well as adding what it finds")]
    fn walk(
        &mut self, body: &[u8], own: usize, at: (usize, usize), active: &GlyphSet, num_glyphs: u32,
        left: &mut Option<usize>, found: &mut GlyphSet,
    ) -> Option<()> {
        let layout = ligature_layout(body, own)?;
        let (pushed, read) = pushed_glyphs(body, &layout, active, num_glyphs)?;
        // Many subtables may share one machine and its arrays, so each pass pays for reading them again.
        spend(left, pushed.len() + read + layout.n_actions + layout.n_ligatures)?;
        let kept: Vec<(u16, u16)> = pushed.into_iter().filter(|&(g, _)| present(active, num_glyphs, g)).collect();
        if self.kept.get(&at) == Some(&kept) {
            return Some(());
        }
        let Stacking { formed, popped } = stacking(body, &layout, &kept, active, num_glyphs, &mut self.walks)?;
        found.extend(formed.into_iter().map(u32::from));
        self.popped.insert(at, popped);
        self.kept.insert(at, kept);
        Some(())
    }
}

// Only an active glyph's substitute in a lookup a reachable entry names.
fn contextual_targets(
    body: &[u8], own: usize, active: &GlyphSet, num_glyphs: u32, left: &mut Option<usize>, found: &mut GlyphSet,
) -> Option<()> {
    let subst_off = read_u32_be(body, 16)? as usize;
    let table = body.get(subst_off..)?;
    let (reached, entry) = reachable_entries(body, &[subst_off, own], 2, active, num_glyphs, left)?;
    let indices: BTreeSet<u16> = reached.into_iter()
        .flat_map(|e| [read_u16_be(entry, usize::from(e) * 8 + 4), read_u16_be(entry, usize::from(e) * 8 + 6)])
        .flatten()
        .filter(|&w| w != NO_INDEX)
        .collect();
    let offsets: BTreeSet<u32> = indices.into_iter().filter_map(|i| read_u32_be(table, 4 * usize::from(i))).collect();
    for off in offsets {
        let Some(lookup) = table.get(off as usize..).and_then(|d| Lookup::parse(d, num_glyphs).ok()) else { continue };
        substituted(&lookup, active, num_glyphs, left, found)?;
    }
    Some(())
}

// The glyphs a substitution lookup turns present ones into, read as the shaper reads it, its reads spending `left`.
fn substituted(
    lookup: &Lookup, active: &GlyphSet, num_glyphs: u32, left: &mut Option<usize>, found: &mut GlyphSet,
) -> Option<()> {
    spend(left, lookup.reads())?;
    let pairs = lookup.typed_entries();
    found.extend(pairs.into_iter().filter(|&(from, _)| present(active, num_glyphs, from)).map(|(_, to)| to.into()));
    Some(())
}

// The glyphs a reachable entry inserts.
fn insertion_targets(
    body: &[u8], own: usize, active: &GlyphSet, num_glyphs: u32, left: &mut Option<usize>, found: &mut GlyphSet,
) -> Option<()> {
    let action_off = read_u32_be(body, 16)? as usize;
    let (reached, entry) = reachable_entries(body, &[action_off, own], 2, active, num_glyphs, left)?;
    for e in reached {
        let at = usize::from(e) * 8;
        let Some(flags) = read_u16_be(entry, at + 2) else { continue };
        for (index, count) in [
            (read_u16_be(entry, at + 4), (flags & CURRENT_INSERT_COUNT) >> 5),
            (read_u16_be(entry, at + 6), flags & MARKED_INSERT_COUNT),
        ] {
            let Some(index) = index.filter(|&i| i != NO_INDEX) else { continue };
            // A list past the table inserts nothing.
            if 2 * (usize::from(index) + usize::from(count)) > body.len().saturating_sub(action_off) { continue; }
            for k in 0..usize::from(count) {
                let at = (usize::from(index) + k).checked_mul(2).and_then(|o| action_off.checked_add(o));
                if let Some(g) = at.and_then(|at| read_u16_be(body, at)) { found.insert(g.into()); }
            }
        }
    }
    Some(())
}

// What a ligature subtable's actions can store, and every glyph each action word can pop.
struct Stacking {
    formed: BTreeSet<u16>,
    popped: Popped,
}

// The stack the machine can leave at a node, as words: the class sets pushed since the last stored ligature, newest
// first, then the ligatures beneath, the last bit for none. Stacks at a node are as deep, so they join word by word.
type Stack = Vec<u64>;

// The shaper's ring of the last components pushed: an action popping more than it holds is taken whole, every
// ligature formed and any glyph popped.
const STACK_DEPTH: usize = crate::daecore::daeshaper::ot::apply::SEQUENCE_LIMIT;

// What the stacking walk may spend for each unit of its subtable's subset budget; past it the walk counts every
// ligature at every word, never refused. Apple Color Emoji stops here with the same closures at under half the cost.
const STACKING: usize = 64;

// A word p deep after p pushes pops the ligature beneath them, and a deeper one any formed ligature or kept glyph;
// passes repeat until none new forms. Past the work bound or the stack's depth, every one counts at every word. What
// the walk spends comes off `room`, which may hold it below its own budget.
fn stacking(
    body: &[u8], layout: &LigatureLayout, listed: &[(u16, u16)], active: &GlyphSet, num_glyphs: u32,
    room: &mut usize,
) -> Option<Stacking> {
    let header = StxHeader::read(body)?;
    let n_classes = header.n_classes;
    let (states, entry) = machine_parts(body, &layout.starts[3..], 1)?;
    // No entry names a state past these, however far a hostile state array runs, and the walk holds slots for each.
    let named = (0..entry.len() / 6).filter_map(|e| read_u16_be(entry, e * 6)).max().map_or(0, |n| usize::from(n) + 1);
    let n_states = (states.len() / (2 * n_classes)).min(named.max(2));
    // A class past the table's reads as out of bounds, and a glyph a ligature deleted as class 2.
    let kept: Vec<(u16, u16)> = listed.iter().copied()
        .filter(|&(g, _)| present(active, num_glyphs, g))
        .chain([(DELETED_GLYPH, 2)])
        .map(|(g, c)| (g, if usize::from(c) < n_classes { c } else { 1 }))
        .collect();
    let mut classes: Vec<u16> =
        (0..4).chain(kept.iter().map(|&(_, c)| c)).filter(|&c| usize::from(c) < n_classes).collect();
    classes.sort_unstable();
    classes.dedup();
    let mut members: Vec<Vec<u16>> = alloc::vec![Vec::new(); classes.len()];
    for &(g, c) in &kept {
        if let Ok(i) = classes.binary_search(&c) { members[i].push(g); }
    }

    let n = layout.n_ligatures;
    let glyph = |i: usize| if i < n { read_u16_be(body, layout.ligature + i * 2) } else { None };
    let component = |g: u16, offset: i64| {
        let at = usize::try_from(i64::from(g) + offset).ok().filter(|&a| a < layout.n_components)?;
        read_u16_be(body, layout.component + at * 2).map(usize::from)
    };
    // An action's words, through the one marked last.
    let words = |first: usize| {
        (first..layout.n_actions)
            .map_while(|k| read_u32_be(body, layout.action + k * 4))
            .scan(true, |more, w| core::mem::replace(more, w & LIG_LAST == 0).then_some(w))
    };
    // How many words each action runs to its last, from one pass back, as entries may each start one and share it.
    let mut runs = alloc::vec![0u32; layout.n_actions + 1];
    for k in (0..layout.n_actions).rev() {
        runs[k] = match read_u32_be(body, layout.action + k * 4) {
            Some(w) if w & LIG_LAST != 0 => 1,
            Some(_) => runs[k + 1] + 1,
            None => 0,
        };
    }
    let deepest = (0..entry.len() / 6)
        .filter_map(|e| (read_u16_be(entry, e * 6 + 2)? & PERFORM_ACTION != 0).then(|| read_u16_be(entry, e * 6 + 4))?)
        .map(|action| runs.get(usize::from(action)).map_or(0, |&n| n as usize))
        .max()
        .unwrap_or(0);
    let every = || Stacking {
        formed: (0..n).filter_map(glyph).collect(),
        popped: Popped::Any(sorted((0..n).filter_map(glyph).chain(kept.iter().map(|&(g, _)| g)))),
    };
    if deepest > STACK_DEPTH {
        return Some(every());
    }

    let given = super::super::subset_budget(body.len()).saturating_mul(STACKING).min(*room);
    let mut work = given;
    // By offset, each class's component values and whether one of its glyphs reads none.
    let mut values: BTreeMap<i64, Vec<(Vec<usize>, bool)>> = BTreeMap::new();
    let class_words = classes.len().div_ceil(64);
    let mut every_class = bits(classes.len());
    (0..classes.len()).for_each(|i| set(&mut every_class, i));
    let no_class = bits(classes.len());
    // By action offset, the ligatures and the classes its words can pop.
    let mut popped: BTreeMap<i64, (Vec<u64>, Vec<u64>)> = BTreeMap::new();
    let (mut older, mut formed) = (bits(n + 1), bits(n + 1));
    let mut empty = bits(n + 1);
    set(&mut empty, n);
    let depths = deepest + 1;
    // A node is a state, the pushes since the last stored ligature and whether the stack's top is the current glyph.
    let at_node = |state: usize, p: usize, here: bool| (state * depths + p) * 2 + usize::from(here);
    // By action and pushes, what it does from each stack, held as the walk's stacks are.
    let mut acted: BTreeMap<(u16, usize), BTreeMap<Stack, Acted>> = BTreeMap::new();
    // The nodes and the stacks they hold draw on their own budget, as large as `work`.
    let nodes = n_states.saturating_mul(depths * 2);
    let Some(left) = work.checked_sub(nodes.saturating_mul(size_of::<Option<Stack>>() + 1)) else {
        return Some(every());
    };
    let (stacks, queued) = (alloc::vec![None; nodes], alloc::vec![false; nodes]);
    let mut walk = Walk { stacks, queued, queue: Vec::new(), left };
    for s in 0..n_states.min(2) {
        walk.reach(at_node(s, 0, false), &empty);
    }
    let (mut stack, mut pushed): (Stack, Stack) = (Vec::new(), Vec::new());
    let stacked = 'walk: loop {
        while let Some(node) = walk.queue.pop() {
            if walk.left == 0 {
                break 'walk every();
            }
            walk.queued[node] = false;
            let (state, above, here) = (node / 2 / depths, node / 2 % depths, node % 2 == 1);
            let Some(held) = &walk.stacks[node] else { continue };
            stack.clone_from(held);
            for (ci, &c) in classes.iter().enumerate() {
                let cell = header.state + 2 * (state * n_classes + usize::from(c));
                let Some(e) = read_u16_be(body, cell) else { continue };
                let at = usize::from(e) * 6;
                let (Some(next), Some(flags), Some(action)) =
                    (read_u16_be(entry, at), read_u16_be(entry, at + 2), read_u16_be(entry, at + 4))
                else {
                    continue;
                };
                let next = usize::from(next);
                if next >= n_states { continue; }
                // A push onto the glyph the top already holds, back at it after DontAdvance, stacks nothing.
                let (p, on, now) = if flags & SET_COMPONENT == 0 {
                    (above, here, &stack)
                } else if here {
                    (above, true, &stack)
                } else {
                    let p = (above + 1).min(deepest);
                    pushed.clear();
                    if p > 0 {
                        pushed.resize(class_words, 0);
                        set(&mut pushed, ci);
                        pushed.extend_from_slice(&stack[..(p - 1) * class_words]);
                    }
                    pushed.extend_from_slice(&stack[above * class_words..]);
                    (p, true, &pushed)
                };
                // The next glyph comes unless DontAdvance holds, and does once its budget runs out.
                let to = |walk: &mut Walk, p: usize, on: bool, stack: &[u64]| {
                    walk.reach(at_node(next, p, false), stack);
                    if on && flags & DONT_ADVANCE != 0 { walk.reach(at_node(next, p, true), stack); }
                };
                if flags & PERFORM_ACTION == 0 {
                    to(&mut walk, p, on, now);
                    continue;
                }
                let memo = acted.entry((action, p)).or_default();
                if !memo.contains_key(now) {
                    let mut done = Acted { stored: bits(n + 1), keeps: false, deeper: false, clears: false };
                    let mut sums: BTreeSet<usize> = [0].into();
                    let (mut failing, mut storing) = (false, false);
                    for (i, w) in words(usize::from(action)).enumerate() {
                        let offset = action_offset(w);
                        if let Entry::Vacant(slot) = values.entry(offset) {
                            let Some(left) = work.checked_sub(kept.len()) else { break 'walk every() };
                            work = left;
                            let per_class = members.iter().map(|glyphs| {
                                let reads: Vec<Option<usize>> = glyphs.iter().map(|&g| component(g, offset)).collect();
                                let mut vs: Vec<usize> = reads.iter().flatten().copied().collect();
                                vs.sort_unstable();
                                vs.dedup();
                                (vs, reads.iter().any(Option::is_none))
                            });
                            slot.insert(per_class.collect());
                        }
                        // A pushed glyph of a class this depth can hold; past the pushes the ligature beneath them,
                        // and deeper whatever lay under it. Either can be missing, which empties the stack.
                        let (from, under) = match i.cmp(&p) {
                            Ordering::Less => (&now[i * class_words..(i + 1) * class_words], None),
                            Ordering::Equal => (&no_class[..], Some(&now[p * class_words..])),
                            Ordering::Greater => (&every_class[..], Some(&older[..])),
                        };
                        done.deeper |= i > p;
                        done.clears |= i > p || under.is_some_and(|b| has(b, n));
                        let (mut vs, mut short) = (Vec::new(), false);
                        for ci in ones(from) {
                            let (classed, misses) = &values[&offset][ci];
                            vs.extend_from_slice(classed);
                            short |= *misses;
                        }
                        let read = popped.entry(offset).or_insert_with(|| {
                            let read = (bits(n + 1), bits(classes.len()));
                            walk.hold(size_of_val(read.0.as_slice()) + size_of_val(read.1.as_slice()));
                            read
                        });
                        or(&mut read.1, from);
                        if let Some(under) = under {
                            for t in ones(under).filter(|&t| t < n) {
                                match glyph(t).and_then(|g| component(g, offset)) {
                                    Some(v) => vs.push(v),
                                    None => short = true,
                                }
                            }
                            or(&mut read.0, under);
                        }
                        vs.sort_unstable();
                        vs.dedup();
                        let Some(left) = work.checked_sub(sums.len().saturating_mul(vs.len()).max(1)) else {
                            break 'walk every();
                        };
                        work = left;
                        let mut next_sums = BTreeSet::new();
                        for &j in &sums {
                            for &v in &vs {
                                if j + v < n { next_sums.insert(j + v); } else { short = true; }
                            }
                        }
                        sums = next_sums;
                        failing |= short && !storing;
                        if w & (LIG_LAST | LIG_STORE) != 0 {
                            storing = true;
                            sums.iter().for_each(|&j| set(&mut done.stored, j));
                        }
                    }
                    done.keeps = failing || !storing;
                    walk.hold(size_of_val(now.as_slice()) + size_of_val(done.stored.as_slice()));
                    memo.insert(now.clone(), done);
                }
                let Acted { stored, keeps, clears, .. } = &memo[now];
                or(&mut formed, stored);
                // The ligature is the current glyph only if the first word stored it last.
                let first = words(usize::from(action)).next().is_some_and(|w| w & (LIG_LAST | LIG_STORE) != 0);
                // Stored ligatures under no pushes are a stack themselves.
                if stored.iter().any(|&w| w != 0) {
                    to(&mut walk, 0, on && first, stored);
                }
                if *keeps { to(&mut walk, p, on, now); }
                if *clears { walk.reach(at_node(next, 0, false), &empty); }
            }
        }
        if walk.left == 0 {
            break 'walk every();
        }
        // What read `older` reads it again once it grows; the rest stands.
        let reread = acted.values().flat_map(BTreeMap::values).any(|a| a.deeper);
        if !or(&mut older, &formed) || !reread {
            // Each offset's classes expand to their glyphs once, held to the subset.
            let size = |(ligatures, classed): &(Vec<u64>, Vec<u64>)| {
                ones(ligatures).count() + ones(classed).map(|ci| members[ci].len()).sum::<usize>()
            };
            if !walk.hold(size_of::<u16>().saturating_mul(popped.values().map(size).sum())) {
                break 'walk every();
            }
            let glyphs = |(ligatures, classed): (Vec<u64>, Vec<u64>)| {
                let pushed = ones(&classed).flat_map(|ci| members[ci].iter().copied()).collect::<Vec<_>>();
                sorted(ones(&ligatures).filter_map(glyph).chain(pushed))
            };
            break 'walk Stacking {
                formed: ones(&older).filter_map(glyph).collect(),
                popped: Popped::Each(popped.into_iter().map(|(k, read)| (k, glyphs(read))).collect()),
            };
        }
        acted.values_mut().for_each(|memo| memo.retain(|_, a| !a.deeper));
        for node in 0..walk.stacks.len() {
            if walk.stacks[node].is_some() && !walk.queued[node] {
                walk.queued[node] = true;
                walk.queue.push(node);
            }
        }
    };
    // `work` and the walk's budget each start at `given`, and what the walk holds counts the nodes' slots.
    *room -= (given - work + (given - walk.left)).min(*room);
    Some(stacked)
}

// What an action does from one stack: the ligatures it stores, whether it can leave the stack as it was, read
// `older` or find the stack empty.
struct Acted {
    stored: Vec<u64>,
    keeps: bool,
    deeper: bool,
    clears: bool,
}

// The nodes' stacks, and the nodes whose stacks grew since they were walked.
struct Walk {
    stacks: Vec<Option<Stack>>,
    queued: Vec<bool>,
    queue: Vec<usize>,
    // What it may still store or merge: each node holds a stack as wide as the ligatures, so a table of many states
    // would hold far more than it takes. What a node, the memo or the popped sets store costs its bytes, held to the
    // walk's end, and a stack merged its words, each an OR. Spent, it's 0, and the walk gives up.
    left: usize,
}

impl Walk {
    fn reach(&mut self, node: usize, stack: &[u64]) {
        let cost = if self.stacks[node].is_some() { stack.len() } else { size_of_val(stack) };
        if !self.hold(cost) {
            return;
        }
        let grew = match &mut self.stacks[node] {
            Some(s) => or(s, stack),
            s @ None => {
                *s = Some(stack.to_vec());
                true
            }
        };
        if grew && !self.queued[node] {
            self.queued[node] = true;
            self.queue.push(node);
        }
    }

    // Whether `cost` was left to spend.
    fn hold(&mut self, cost: usize) -> bool {
        self.left = self.left.checked_sub(cost).filter(|&left| left > 0).unwrap_or(0);
        self.left > 0
    }
}

fn sorted(glyphs: impl Iterator<Item = u16>) -> Vec<u16> {
    let mut glyphs: Vec<u16> = glyphs.collect();
    glyphs.sort_unstable();
    glyphs.dedup();
    glyphs
}

// Sets of ligature or class indices, as words.
fn bits(n: usize) -> Vec<u64> {
    alloc::vec![0; n.div_ceil(64)]
}

fn set(bits: &mut [u64], i: usize) {
    bits[i / 64] |= 1 << (i % 64);
}

fn has(bits: &[u64], i: usize) -> bool {
    bits.get(i / 64).is_some_and(|w| w >> (i % 64) & 1 != 0)
}

// Whether `a` grew.
fn or(a: &mut [u64], b: &[u64]) -> bool {
    let mut grew = false;
    for (a, &b) in a.iter_mut().zip(b) {
        grew |= b & !*a != 0;
        *a |= b;
    }
    grew
}

fn ones(bits: &[u64]) -> impl Iterator<Item = usize> + '_ {
    bits.iter().enumerate().flat_map(|(i, &w)| {
        let mut rest = w;
        core::iter::from_fn(move || {
            let b = (rest != 0).then(|| rest.trailing_zeros())?;
            rest &= rest - 1;
            Some(i * 64 + b as usize)
        })
    })
}

#[cfg(test)]
pub fn subset_morx(morx: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32) -> Option<Vec<u8>> {
    subset_morx_with(morx, active, gid_map, num_glyphs, None)
}

// `walked` is the closure's stackings, read where they were over this morx and `active`.
pub(crate) fn subset_morx_with(
    morx: &[u8], active: &GlyphSet, gid_map: &[u32], num_glyphs: u32, walked: Option<&Stackings>,
) -> Option<Vec<u8>> {
    let walked = walked.filter(|w| core::ptr::eq(w.morx, morx) && w.active == *active);
    let version = read_u16_be(morx, 0)?;
    let budget = super::super::subset_budget(morx.len());
    let mut work = Work { reading: budget.saturating_mul(64), placing: budget.saturating_mul(64).max(PLACING) };

    // A subtable that cannot be rebuilt is left out and its chain keeps the rest.
    let mut out = morx.get(..8)?.to_vec();
    let mut n_chains = 0u32;
    // Tails go after the last chain, so what they hold counts toward the budget as each is made.
    let (mut tails, mut held) = (Vec::new(), 0);
    for (c, chain) in morx_chains(morx).enumerate() {
        let mut rebuilt = chain.bytes[..chain.prefix].to_vec();
        let mut kept = 0u32;
        let mut chain_tails = Vec::new();
        for (t, MorxSubtable { coverage, flags, body, own }) in chain.subtables(morx).enumerate() {
            let popped = walked.and_then(|w| w.popped.get(&(c, t)));
            let subset = subset_subtable(body, own, coverage, active, gid_map, num_glyphs, popped, &mut work);
            let Some((mut new_body, tail)) = subset else { continue };
            // The morx chapter has a chain's length a multiple of 4 and version 3's coverage array right after its last
            // subtable; padding each subtable within its own length keeps both.
            new_body.resize(new_body.len().next_multiple_of(4), 0);
            held += tail.as_ref().map_or(0, |tail| tail.glyphs.len());
            chain_tails.extend(tail.map(|tail| (rebuilt.len() + 12, tail)));
            rebuilt.extend_from_slice(&u32::try_from(12 + new_body.len()).ok()?.to_be_bytes());
            rebuilt.extend_from_slice(&coverage.to_be_bytes());
            rebuilt.extend_from_slice(&flags.to_be_bytes());
            rebuilt.extend_from_slice(&new_body);
            kept += 1;
            if out.len() + rebuilt.len() + held > budget { return None; }
        }
        rebuilt.get_mut(12..16)?.copy_from_slice(&kept.to_be_bytes());
        // Version 3 follows a chain's subtables with a coverage offset for each; 0 means none.
        if version >= 3 { rebuilt.resize(rebuilt.len() + 4 * kept as usize, 0); }
        let length = u32::try_from(rebuilt.len()).ok()?;
        rebuilt.get_mut(4..8)?.copy_from_slice(&length.to_be_bytes());
        tails.extend(chain_tails.into_iter().map(|(body, tail)| (out.len() + body, tail)));
        out.extend_from_slice(&rebuilt);
        n_chains += 1;
    }
    // A morx of no chains stays: any morx the sanitizer passes counts as data, which keeps GSUB from running.
    out.get_mut(4..8)?.copy_from_slice(&n_chains.to_be_bytes());
    place_tails(&mut out, &tails)?;
    (out.len() <= budget).then_some(out)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/aat/morx.rs"]
mod tests;
