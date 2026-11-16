// The glyphs GSUB reaches from a set, as a subset closes them: the lookups its features name, in turn until a pass
// adds nothing, a context's nested lookups only over the glyphs its rule can hold at their place.
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use core::cell::Cell;

use crate::daecore::daeshaper::ot::apply::MAX_NESTING_LEVEL;
use crate::daecore::daeshaper::ot::{long, nth_table_width, resolve_extension, table_at_width, tables_of, LayoutTable};
use crate::daecore::daetype::decoder::{read_u16_be, read_uint};
use crate::daecore::daetype::format::{class_def, coverage};
use super::super::{subset_budget, GlyphSet};

// The work a closure may spend for each unit of the subset budget its GSUB's size gives: 17 times the most a font of
// the corpus spends, NotoNastaliq's whole font at 7.5. A unit costs at most about 100 instructions.
const WORK: usize = 1 << 7;

// The lookups every language system's features name, required ones and feature variations' too, so a subset keeps
// every feature. Each script, system and Feature is read once, a record spending one unit of the budget and a
// Feature's lookup index one of the subset budget; None past either.
pub(crate) fn feature_lookups(gsub: &[u8]) -> Option<Vec<u16>> {
    let Some(t) = LayoutTable::parse(gsub) else { return Some(Vec::new()) };
    let mut left = budget(gsub);
    let (mut scripts, mut systems, mut features) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for script in 0..t.script_count() {
        left = left.checked_sub(1)?;
        if !t.script(script).is_some_and(|at| scripts.insert(at)) {
            continue;
        }
        for lang in core::iter::once(None).chain((0..t.lang_sys_count(script)).map(Some)) {
            left = left.checked_sub(1)?;
            if !t.lang_sys(script, lang).is_some_and(|system| systems.insert(system)) {
                continue;
            }
            features.extend(t.required_feature(script, lang));
            for feature in t.feature_indices(script, lang) {
                left = left.checked_sub(1)?;
                features.insert(feature);
            }
        }
    }
    let (mut read, mut reads, mut lookups) = (BTreeSet::new(), subset_budget(gsub.len()), BTreeSet::new());
    for &feature in features.iter().filter(|&&f| t.feature_at(f).is_some_and(|at| read.insert(at))) {
        for lookup in t.feature_lookups(feature, None) {
            reads = reads.checked_sub(1)?;
            lookups.insert(lookup);
        }
    }
    lookups.extend(t.variant_lookups(&features)?);
    Some(lookups.into_iter().collect())
}

// The work a closure over `gsub` may spend.
pub(crate) fn budget(gsub: &[u8]) -> usize {
    subset_budget(gsub.len()).saturating_mul(WORK)
}

// `glyphs` with every glyph GSUB's `lookups` reach from them, below `num_glyphs`, the lookups run until a pass adds
// nothing: false where the work runs past `budget`, which bounds a table that adds a glyph a pass.
pub(crate) fn close(gsub: &[u8], lookups: &[u16], glyphs: &mut GlyphSet, num_glyphs: u32, budget: usize) -> bool {
    let Some(table) = LayoutTable::parse(gsub) else { return true };
    glyphs.fit(num_glyphs);
    let mut c = Closure {
        table,
        glyphs: core::mem::replace(glyphs, GlyphSet::empty()),
        output: GlyphSet::empty(),
        active: Vec::new(),
        done: BTreeMap::new(),
        nesting_left: MAX_NESTING_LEVEL,
        num_glyphs,
        work: Work::new(budget),
        classes: ClassCache::default(),
    };
    loop {
        let before = c.glyphs.len();
        for &lookup in lookups {
            c.lookup(lookup);
        }
        if c.work.over() || c.glyphs.len() == before {
            break;
        }
    }
    *glyphs = c.glyphs;
    !c.work.over()
}

struct Closure<'a> {
    table: LayoutTable<'a>,
    glyphs: GlyphSet,
    // What the lookup visited now adds, joined to `glyphs` when it's done.
    output: GlyphSet,
    // The glyphs each context the visit is inside can hold where its nested lookup applies.
    active: Vec<GlyphSet>,
    // Each lookup's glyph count when last visited, and the glyphs it was visited over since.
    done: BTreeMap<u16, (usize, GlyphSet)>,
    // Nested lookups as deep as shaping applies them, so the closure reaches what shaping can.
    nesting_left: usize,
    num_glyphs: u32,
    // The work left: each visit, subtable, rule and nested lookup spends what it reads, a set its words.
    work: Work,
    // The class-based context closing now: its classes' answers, kept for its subtable so each is worked out once.
    classes: ClassCache,
}

// The work a closure has left, spent through a shared borrow so a walk over the closure's own sets can stop on it.
struct Work {
    left: Cell<usize>,
    over: Cell<bool>,
}

impl Work {
    fn new(budget: usize) -> Work {
        Work { left: Cell::new(budget), over: Cell::new(false) }
    }

    // Spends `n` of what's left: false past it, and nothing is left from then on.
    fn spend(&self, n: usize) -> bool {
        let left = self.left.get().checked_sub(n);
        self.left.set(left.unwrap_or(0));
        self.over.set(self.over.get() || left.is_none());
        left.is_some()
    }

    fn over(&self) -> bool {
        self.over.get()
    }
}

// Whether the set holds a class, by its ClassDef and class, and a class's glyphs, by whether they're the first
// place's too, as the first place reads the parent's set and the others the whole set.
#[derive(Default)]
struct ClassCache {
    holds: BTreeMap<(usize, u32), bool>,
    glyphs: BTreeMap<(usize, u32, bool), GlyphSet>,
}

impl<'a> Closure<'a> {
    fn parent(&self) -> &GlyphSet {
        self.active.last().unwrap_or(&self.glyphs)
    }

    // A top-level lookup, its output joined to the set after it.
    fn lookup(&mut self, index: u16) {
        if !self.should_visit(index) {
            return;
        }
        self.dispatch(index);
        let num_glyphs = self.num_glyphs;
        self.output.iter().filter(|&g| g < num_glyphs).for_each(|g| {
            self.glyphs.insert(g);
        });
        self.output.clear();
        self.active.clear();
    }

    // A nested lookup over the top of `active`. One that may change the run's length leaves the rest of the rule's
    // places to the whole set, through `covered`.
    fn recurse(&mut self, index: u16, covered: &mut GlyphSet, seq: u32, end: u32) {
        if self.nesting_left == 0 {
            return;
        }
        self.nesting_left -= 1;
        if self.should_visit(index) {
            if self.non_1to1(index) && self.work.spend(end.saturating_sub(seq) as usize + 1) {
                (seq..=end).for_each(|i| {
                    covered.insert(i);
                });
            }
            self.dispatch(index);
        }
        self.nesting_left += 1;
    }

    // False past the budget: a walk is charged before it runs, and a range coverage can name billions of glyphs, so a
    // walk whose charge failed must not run.
    fn spend(&self, n: usize) -> bool {
        self.work.spend(n)
    }

    fn should_visit(&mut self, index: u16) -> bool {
        self.spend(1 + self.parent().words());
        if self.work.over() {
            return false;
        }
        let count = self.glyphs.len();
        let parent = self.active.last().unwrap_or(&self.glyphs);
        let (seen, covered) = self.done.entry(index).or_insert_with(|| (usize::MAX, GlyphSet::empty()));
        if *seen != count {
            *seen = count;
            covered.clear();
        }
        if parent.is_subset(covered) {
            return false;
        }
        covered.union(parent);
        true
    }

    // Each subtable of lookup `index` by its type, through an Extension's.
    fn subtables(&self, index: u16) -> impl Iterator<Item = (u16, &'a [u8])> + use<'a> {
        self.table.lookup(index).into_iter().flat_map(|lookup| {
            (0..lookup.subtable_count).filter_map(move |i| resolve_extension(lookup.subtable(i), lookup.kind, 7))
        })
    }

    // Whether the lookup holds a subtable that can change a run's length: a MultipleSubst, LigatureSubst or context
    // of a known format.
    fn non_1to1(&self, index: u16) -> bool {
        self.subtables(index).any(|(kind, data)| {
            let format = read_u16_be(data, 0).unwrap_or(0);
            match kind {
                2 | 4 => (1..=2).contains(&format),
                5 => (1..=6).contains(&format),
                6 => (1..=5).contains(&format),
                _ => false,
            }
        })
    }

    fn dispatch(&mut self, index: u16) {
        for (kind, data) in self.subtables(index) {
            if !self.spend(1 + self.parent().words()) {
                return;
            }
            match (kind, read_u16_be(data, 0).unwrap_or(0)) {
                (1, 1) => self.single_delta::<2>(data),
                (1, 2) => self.single_list::<2>(data),
                (1, 3) => self.single_delta::<3>(data),
                (1, 4) => self.single_list::<3>(data),
                (2 | 3, 1) => self.sequences::<2>(data),
                (2 | 3, 2) => self.sequences::<3>(data),
                (4, 1) => self.ligatures::<2>(data),
                (4, 2) => self.ligatures::<3>(data),
                (5, 1) => self.glyph_rules::<2, 2>(data, false),
                (5, 2) => self.class_rules::<2, 2>(data, false),
                (5, 3) => self.coverages(data, 2),
                (5, 4) => self.glyph_rules::<3, 2>(data, false),
                (5, 5) => self.class_rules::<3, 3>(data, false),
                (5, 6) => self.coverages(data, 3),
                (6, 1) => self.glyph_rules::<2, 2>(data, true),
                (6, 2) => self.class_rules::<2, 2>(data, true),
                (6, 3) => self.chain_coverages(data),
                (6, 4) => self.glyph_rules::<3, 3>(data, true),
                (6, 5) => self.class_rules::<3, 3>(data, true),
                (8, 1) => self.reverse::<2>(data),
                (8, 2) => self.reverse::<3>(data),
                _ => {}
            }
        }
    }

    // SingleSubst 1 or 3, a delta wrapping at the glyph width.
    fn single_delta<const W: usize>(&mut self, data: &[u8]) {
        let l = long(W);
        let cov = table_at_width(data, 2, l);
        let mask = if W == 3 { 0xFF_FFFF } else { 0xFFFF };
        let delta = read_uint(data, 2 + l, W).unwrap_or(0);
        let mut from = GlyphSet::empty();
        intersect(cov, self.parent(), &mut from, &self.work);
        if !self.spend(entries(cov)) {
            return;
        }
        from.iter().for_each(|g| {
            self.output.insert(g.wrapping_add(delta) & mask);
        });
    }

    // SingleSubst 2 or 4: over a set small beside the list, each glyph looked up; else the list walked.
    fn single_list<const W: usize>(&mut self, data: &[u8]) {
        let l = long(W);
        let cov = table_at_width(data, 2, l);
        let count = read_uint(data, 2 + l, W).unwrap_or(0);
        let substitute = |i: u32| read_uint(data, 2 + l + W + i as usize * W, W);
        let parent = self.active.last().unwrap_or(&self.glyphs);
        let mut found = Vec::new();
        let walk = count as usize > parent.len().saturating_mul(4);
        let cost = if walk { parent.len() } else { population(cov) as usize };
        if !self.spend(cost) {
            return;
        }
        if walk {
            found.extend(parent.iter().filter_map(|g| coverage::coverage_index(cov, g)).filter(|&i| i < count));
        } else {
            each_covered(cov, |i, g| {
                if i < count && parent.contains(&g) {
                    found.push(i);
                }
            });
        }
        found.into_iter().filter_map(substitute).for_each(|g| {
            self.output.insert(g);
        });
    }

    // MultipleSubst or AlternateSubst: each covered glyph's whole list.
    fn sequences<const W: usize>(&mut self, data: &'a [u8]) {
        for list in self.covered_sets::<W>(data) {
            let count = read_u16_be(list, 0).unwrap_or(0);
            if !self.spend(usize::from(count)) {
                return;
            }
            (0..usize::from(count)).map_while(|i| read_uint(list, 2 + i * W, W)).for_each(|g| {
                self.output.insert(g);
            });
        }
    }

    // LigatureSubst: each ligature of a covered glyph whose other components the set holds.
    fn ligatures<const W: usize>(&mut self, data: &'a [u8]) {
        for set in self.covered_sets::<W>(data) {
            for lig in tables_of::<W>(set, 0) {
                let count = read_u16_be(lig, W).unwrap_or(0);
                if !self.spend(1 + usize::from(count)) {
                    return;
                }
                let rest = (0..usize::from(count.saturating_sub(1))).map(|i| read_uint(lig, W + 2 + i * W, W));
                if rest.into_iter().all(|g| g.is_some_and(|g| self.glyphs.contains(&g))) {
                    self.output.insert(read_uint(lig, 0, W).unwrap_or(0));
                }
            }
        }
    }

    // The list or set each covered glyph of the parent's names, in the coverage's order.
    fn covered_sets<const W: usize>(&mut self, data: &'a [u8]) -> Vec<&'a [u8]> {
        let l = long(W);
        if !self.spend(population(table_at_width(data, 2, l)) as usize) {
            return Vec::new();
        }
        let parent = self.parent();
        let mut sets = Vec::new();
        each_covered(table_at_width(data, 2, l), |i, g| {
            if parent.contains(&g) {
                sets.push(nth_table_width(data, 2 + l, W, W, i));
            }
        });
        sets
    }

    // ReverseChainSingleSubst, where its context can match.
    fn reverse<const W: usize>(&mut self, data: &[u8]) {
        let l = long(W);
        let cov = table_at_width(data, 2, l);
        let back = usize::from(read_u16_be(data, 2 + l).unwrap_or(0));
        let ahead_at = 4 + l + back * W;
        let ahead = usize::from(read_u16_be(data, ahead_at).unwrap_or(0));
        let at = ahead_at + 2 + ahead * W;
        let context = (0..back).map(|i| 4 + l + i * W).chain((0..ahead).map(|i| ahead_at + 2 + i * W));
        let read = context.clone().map(|at| entries(table_at_width(data, at, W))).sum::<usize>();
        if !self.spend(read + population(cov) as usize) {
            return;
        }
        let held = |cov| intersects(cov, &self.glyphs, &self.work);
        if !held(cov) || !context.into_iter().all(|at| held(table_at_width(data, at, W))) {
            return;
        }
        let count = read_uint(data, at, W).unwrap_or(0);
        let parent = self.parent();
        let mut found = Vec::new();
        each_covered(cov, |i, g| {
            if i < count && parent.contains(&g) {
                found.extend(read_uint(data, at + W + i as usize * W, W));
            }
        });
        found.into_iter().for_each(|g| {
            self.output.insert(g);
        });
    }

    // Context or ChainContext 1, or 4: a rule set of glyph rules for each covered glyph, rules at `RW`-byte offsets.
    fn glyph_rules<const W: usize, const RW: usize>(&mut self, data: &'a [u8], chain: bool) {
        let l = long(W);
        let cov = table_at_width(data, 2, l);
        if !self.spend(population(cov) as usize) {
            return;
        }
        let parent = self.parent();
        let mut active = GlyphSet::empty();
        intersect(cov, parent, &mut active, &self.work);
        let mut sets = Vec::new();
        each_covered(cov, |i, g| {
            if parent.contains(&g) {
                sets.push((g, nth_table_width(data, 2 + l, W, W, i)));
            }
        });
        self.active.push(active);
        let m = Match::Glyph(W);
        for (g, set) in sets {
            self.rule_set::<RW>(set, g, [m; 3], chain, Place::Glyph);
        }
        self.active.pop();
    }

    // Context or ChainContext 2, or 5: a rule set of class rules for each class the covered glyphs take.
    fn class_rules<const W: usize, const RW: usize>(&mut self, data: &'a [u8], chain: bool) {
        let l = long(W);
        let cov = table_at_width(data, 2, l);
        if !intersects(cov, &self.glyphs, &self.work) {
            return;
        }
        let mut active = GlyphSet::empty();
        intersect(cov, self.parent(), &mut active, &self.work);
        let defs = if chain { 3 } else { 1 };
        let classes: Vec<&'a [u8]> = (0..defs).map(|i| table_at_width(data, 2 + l * (i + 1), l)).collect();
        let input = classes[defs / 2];
        // ChainContext 5 counts its rule sets in 16 bits.
        let count_at = 2 + l * (defs + 1);
        let count_width = if chain { 2 } else { W };
        let count = read_uint(data, count_at, count_width).unwrap_or(0);
        if !self.spend(entries(cov) + (count as usize).saturating_mul(1 + class_entries(input))) {
            return;
        }
        let sets: Vec<(u32, &'a [u8])> = (0..count)
            .filter(|&class| class_intersects(input, &active, class, &self.work))
            .map(|class| (class, nth_table_width(data, count_at, count_width, W, class)))
            .collect();
        self.active.push(active);
        let m = [classes[0], input, classes[defs - 1]].map(Match::Class);
        let outer = core::mem::take(&mut self.classes);
        for (class, set) in sets {
            self.rule_set::<RW>(set, class, m, chain, Place::Class(input));
        }
        self.classes = outer;
        self.active.pop();
    }

    // Context 3 or 6: one rule of coverages at `width`-byte offsets.
    fn coverages(&mut self, data: &'a [u8], width: usize) {
        let first = table_at_width(data, 6, width);
        if !self.spend(entries(first)) || !intersects(first, &self.glyphs, &self.work) {
            return;
        }
        let mut active = GlyphSet::empty();
        intersect(first, self.parent(), &mut active, &self.work);
        self.active.push(active);
        let count = read_u16_be(data, 2).unwrap_or(0);
        let input = Seq { data: data.get(6 + width..).unwrap_or_default(), width, len: count.saturating_sub(1) };
        let records = Records::at(data, 6 + usize::from(count) * width, read_u16_be(data, 4).unwrap_or(0));
        let m = Match::Coverage(data);
        if self.all_intersect(input, m) {
            self.recurse_records(u32::from(count), input, records, 0, m, Place::Coverage);
        }
        self.active.pop();
    }

    // ChainContext 3: one rule of coverages at 16-bit offsets.
    fn chain_coverages(&mut self, data: &'a [u8]) {
        let seq = |at: usize| Seq::counted(data, at, 2);
        let back = seq(2);
        let input_at = 4 + usize::from(back.len) * 2;
        let input = seq(input_at);
        let ahead_at = input_at + 2 + usize::from(input.len) * 2;
        let ahead = seq(ahead_at);
        let records_at = ahead_at + 2 + usize::from(ahead.len) * 2;
        let first = table_at_width(data, input_at + 2, 2);
        if !self.spend(entries(first)) || !intersects(first, &self.glyphs, &self.work) {
            return;
        }
        let mut active = GlyphSet::empty();
        intersect(first, self.parent(), &mut active, &self.work);
        self.active.push(active);
        let records = Records::at(data, records_at + 2, read_u16_be(data, records_at).unwrap_or(0));
        let rest = Seq { data: input.data.get(2..).unwrap_or_default(), width: 2, len: input.len.saturating_sub(1) };
        let m = Match::Coverage(data);
        if self.all_intersect(back, m) && self.all_intersect(rest, m) && self.all_intersect(ahead, m) {
            self.recurse_records(u32::from(input.len), rest, records, 0, m, Place::Coverage);
        }
        self.active.pop();
    }

    // Each rule of a set at `RW`-byte offsets, while the work lasts, with `value` its first glyph or class.
    fn rule_set<const RW: usize>(&mut self, set: &'a [u8], value: u32, m: [Match<'a>; 3], chain: bool, first: Place) {
        if self.work.over() {
            return;
        }
        for rule in tables_of::<RW>(set, 0) {
            self.spend(1);
            if self.work.over() {
                return;
            }
            let width = match m[1] {
                Match::Glyph(w) => w,
                _ => 2,
            };
            if chain {
                let (back, input, ahead, count, records) = chain_rule(rule, width);
                let matches = [(back, m[0]), (input, m[1]), (ahead, m[2])];
                if matches.into_iter().all(|(places, m)| self.all_intersect(places, m)) {
                    self.recurse_records(u32::from(count), input, records, value, m[1], first);
                }
            } else {
                let (input, count, records) = context_rule(rule, width);
                if self.all_intersect(input, m[1]) {
                    self.recurse_records(u32::from(count), input, records, value, m[1], first);
                }
            }
        }
    }

    fn all_intersect(&mut self, values: Seq, m: Match) -> bool {
        (0..values.len).all(|i| values.get(i).is_some_and(|v| self.intersects(m, v)))
    }

    // Whether the set holds what `value` matches.
    fn intersects(&mut self, m: Match, value: u32) -> bool {
        let Match::Class(def) = m else {
            if !self.spend(m.cost(value)) {
                return false;
            }
            return match m {
                Match::Coverage(base) => intersects(at_offset(base, value), &self.glyphs, &self.work),
                _ => self.glyphs.contains(&value),
            };
        };
        let key = (def.as_ptr() as usize, value);
        if let Some(&held) = self.classes.holds.get(&key) {
            self.spend(1);
            return held;
        }
        if !self.spend(m.cost(value)) {
            return false;
        }
        let held = class_intersects(def, &self.glyphs, value, &self.work);
        self.classes.holds.insert(key, held);
        held
    }

    // The glyphs `value` matches at a later place, of the whole set.
    fn intersected(&mut self, m: Match, value: u32, out: &mut GlyphSet) {
        match m {
            Match::Glyph(_) => {
                out.insert(value);
            }
            Match::Class(def) => self.class_glyphs(def, value, false, out),
            Match::Coverage(base) => intersect(at_offset(base, value), &self.glyphs, out, &self.work),
        }
    }

    // A class's glyphs, of the parent's set at the first place and of the whole set at any other.
    fn class_glyphs(&mut self, def: &[u8], class: u32, first: bool, out: &mut GlyphSet) {
        let key = (def.as_ptr() as usize, class, first);
        if !self.classes.glyphs.contains_key(&key) {
            let mut found = GlyphSet::empty();
            class_glyphs(def, if first { self.parent() } else { &self.glyphs }, class, &mut found, &self.work);
            self.classes.glyphs.insert(key, found);
        }
        if let Some(found) = self.classes.glyphs.get(&key) {
            out.union(found);
        }
    }

    // `context_closure_recurse_lookups`: each record's lookup over the glyphs its place can hold, the first place's
    // from the parent's, others' from the set; over the whole set at a place already covered or after a length change.
    // Each record read is spent, as rules can share one of 65,535 records past their places.
    fn recurse_records(&mut self, count: u32, input: Seq, records: Records, value: u32, m: Match, first: Place) {
        let mut covered = GlyphSet::empty();
        for i in 0..records.count {
            if !self.spend(1) {
                break;
            }
            let Some(r) = records.get(i) else { break };
            let seq = u32::from(r.0);
            if seq >= count {
                continue;
            }
            let active = if covered.contains(&seq) {
                self.glyphs.clone()
            } else {
                let mut place = GlyphSet::empty();
                match (seq, first) {
                    (0, Place::Glyph) => {
                        place.insert(value);
                    }
                    (0, Place::Class(def)) => self.class_glyphs(def, value, true, &mut place),
                    (0, Place::Coverage) => place.union(self.parent()),
                    _ => {
                        if let Some(v) = input.get(seq as u16 - 1) {
                            self.intersected(m, v, &mut place);
                        }
                    }
                }
                place
            };
            covered.insert(seq);
            self.spend(self.glyphs.words());
            self.active.push(active);
            let end = count + u32::from(matches!(first, Place::Coverage));
            self.recurse(r.1, &mut covered, seq, end);
            self.active.pop();
        }
    }
}

// What a rule's first place matches: the covered glyph, its class in the input ClassDef, or the first coverage.
#[derive(Clone, Copy)]
enum Place<'a> {
    Glyph,
    Class(&'a [u8]),
    Coverage,
}

// How a rule's values match a glyph: as glyphs of a width, as classes of a ClassDef, or as offsets to coverages
// from the subtable.
#[derive(Clone, Copy)]
enum Match<'a> {
    Glyph(usize),
    Class(&'a [u8]),
    Coverage(&'a [u8]),
}

impl Match<'_> {
    // What matching one value reads: a glyph, or a ClassDef's or a coverage's entries.
    fn cost(&self, value: u32) -> usize {
        1 + match *self {
            Match::Glyph(_) => 0,
            Match::Class(def) => class_entries(def),
            Match::Coverage(base) => entries(at_offset(base, value)),
        }
    }
}

// `len` values of `width` bytes.
#[derive(Clone, Copy)]
struct Seq<'a> {
    data: &'a [u8],
    width: usize,
    len: u16,
}

impl Seq<'_> {
    fn get(&self, i: u16) -> Option<u32> {
        (i < self.len).then(|| read_uint(self.data, usize::from(i) * self.width, self.width)).flatten()
    }
}

impl<'a> Seq<'a> {
    // Values counted in 16 bits at `at`.
    fn counted(data: &'a [u8], at: usize, width: usize) -> Self {
        Seq { data: data.get(at + 2..).unwrap_or_default(), width, len: read_u16_be(data, at).unwrap_or(0) }
    }
}

// A rule's (sequence index, lookup) records.
#[derive(Clone, Copy)]
struct Records<'a> {
    data: &'a [u8],
    count: u16,
}

impl<'a> Records<'a> {
    fn at(data: &'a [u8], at: usize, count: u16) -> Self {
        Records { data: data.get(at..).unwrap_or_default(), count }
    }

    fn get(&self, i: u16) -> Option<(u16, u16)> {
        let at = usize::from(i) * 4;
        (i < self.count).then(|| Some((read_u16_be(self.data, at)?, read_u16_be(self.data, at + 2)?))).flatten()
    }
}

// A context rule of values of `width` bytes: those after its first place, its input count and its records. A null
// rule holds none.
fn context_rule(rule: &[u8], width: usize) -> (Seq<'_>, u16, Records<'_>) {
    let count = read_u16_be(rule, 0).unwrap_or(0);
    let input = Seq { data: rule.get(4..).unwrap_or_default(), width, len: count.saturating_sub(1) };
    let records = Records::at(rule, 4 + usize::from(input.len) * width, read_u16_be(rule, 2).unwrap_or(0));
    (input, count, records)
}

// A chain rule's backtrack, input after its first place and lookahead, its input count and its records.
fn chain_rule(rule: &[u8], width: usize) -> (Seq<'_>, Seq<'_>, Seq<'_>, u16, Records<'_>) {
    let back = Seq::counted(rule, 0, width);
    let at = 2 + usize::from(back.len) * width;
    let count = read_u16_be(rule, at).unwrap_or(0);
    let input = Seq { data: rule.get(at + 2..).unwrap_or_default(), width, len: count.saturating_sub(1) };
    let ahead = Seq::counted(rule, at + 2 + usize::from(input.len) * width, width);
    let at = at + 4 + usize::from(input.len) * width + usize::from(ahead.len) * width;
    let records = Records::at(rule, at + 2, read_u16_be(rule, at).unwrap_or(0));
    (back, input, ahead, count, records)
}

// The table at `offset` from `base`, empty for a null one.
fn at_offset(base: &[u8], offset: u32) -> &[u8] {
    if offset == 0 { &[] } else { base.get(offset as usize..).unwrap_or_default() }
}

// The first glyph of `set` past `g`, from the first for u32::MAX.
fn next(set: &GlyphSet, g: u32) -> Option<u32> {
    set.within(g.wrapping_add(1), u32::MAX).next()
}

// The last glyph a gap walk has passed, u32::MAX for none, once it passes a range ending at `last`.
fn past(g: u32, last: u32) -> u32 {
    if g == u32::MAX { last } else { g.max(last) }
}

fn bit_storage(n: u32) -> usize {
    (u32::BITS - n.leading_zeros()) as usize
}

// A coverage's format, 1 or 2, and the width of its glyphs and counts.
fn coverage_shape(cov: &[u8]) -> Option<(u16, usize)> {
    match read_u16_be(cov, 0)? {
        f @ (1 | 2) => Some((f, 2)),
        f @ (3 | 4) => Some((f - 2, 3)),
        _ => None,
    }
}

// Range `i` of a coverage of ranges of `w`-byte values: (first, last, first index).
fn coverage_range(cov: &[u8], w: usize, i: u32) -> Option<(u32, u32, u32)> {
    let at = 2 + w + i as usize * 3 * w;
    Some((read_uint(cov, at, w)?, read_uint(cov, at + w, w)?, read_uint(cov, at + 2 * w, w)?))
}

// A coverage's glyphs with their coverage indices: format 1's by place in its list, format 2's each range's from
// its own startCoverageIndex. A range running backward covers nothing.
fn each_covered(cov: &[u8], mut f: impl FnMut(u32, u32)) {
    let Some((format, w)) = coverage_shape(cov) else { return };
    let count = read_uint(cov, 2, w).unwrap_or(0);
    if format == 1 {
        (0..count).map_while(|i| Some((i, read_uint(cov, 2 + w + i as usize * w, w)?))).for_each(|(i, g)| f(i, g));
        return;
    }
    for (first, last, index) in (0..count).map_while(|i| coverage_range(cov, w, i)) {
        (first..=last).for_each(|g| f(index.wrapping_add(g - first), g));
    }
}

// The glyphs `set` holds that a coverage covers: those of each glyph or range it lists. This and the walkers below
// spend each range's reading of the set, again for a range that overlaps another, and stop where the work runs out.
fn intersect(cov: &[u8], set: &GlyphSet, out: &mut GlyphSet, work: &Work) {
    let Some((format, w)) = coverage_shape(cov) else { return };
    let count = read_uint(cov, 2, w).unwrap_or(0);
    if format == 1 {
        (0..count).map_while(|i| read_uint(cov, 2 + w + i as usize * w, w)).filter(|g| set.contains(g)).for_each(|g| {
            out.insert(g);
        });
        return;
    }
    for (first, last, _) in (0..count).map_while(|i| coverage_range(cov, w, i)) {
        within(set, first, last, work).for_each(|g| {
            out.insert(g);
        });
    }
}

// Whether a coverage covers a glyph `set` holds: by looking each up where the set is small beside the coverage.
fn intersects(cov: &[u8], set: &GlyphSet, work: &Work) -> bool {
    let Some((format, w)) = coverage_shape(cov) else { return false };
    let count = read_uint(cov, 2, w).unwrap_or(0);
    if count as usize > set.len().saturating_mul(bit_storage(count)) {
        return work.spend(set.words()) && set.iter().any(|g| coverage::coverage_index(cov, g).is_some());
    }
    if format == 1 {
        return (0..count).map_while(|i| read_uint(cov, 2 + w + i as usize * w, w)).any(|g| set.contains(&g));
    }
    let mut ranges = (0..count).map_while(|i| coverage_range(cov, w, i));
    ranges.any(|(first, last, _)| within(set, first, last, work).next().is_some())
}

// The glyphs `set` holds from `first` to `last`, spending the words read and each glyph met.
fn within<'s>(set: &'s GlyphSet, first: u32, last: u32, work: &'s Work) -> impl Iterator<Item = u32> + 's {
    let read = work.spend(set.words_within(first, last));
    set.within(first, last).take_while(move |_| read && work.spend(1))
}

// A coverage's glyphs or ranges, as many as reading it walks.
fn entries(cov: &[u8]) -> usize {
    coverage_shape(cov).and_then(|(_, w)| read_uint(cov, 2, w)).unwrap_or(0) as usize
}

// How many glyphs a coverage covers, a backward range none, at most u32::MAX.
fn population(cov: &[u8]) -> u32 {
    let Some((format, w)) = coverage_shape(cov) else { return 0 };
    let count = read_uint(cov, 2, w).unwrap_or(0);
    if format == 1 {
        return count;
    }
    let sum: u64 = (0..count)
        .map_while(|i| coverage_range(cov, w, i))
        .map(|(first, last, _)| if last < first { 0 } else { u64::from(last - first) + 1 })
        .sum();
    u32::try_from(sum).unwrap_or(u32::MAX)
}

// A ClassDef's format, 1 or 2, and the width of its glyphs and counts.
fn class_shape(def: &[u8]) -> Option<(u16, usize)> {
    match read_u16_be(def, 0)? {
        f @ (1 | 2) => Some((f, 2)),
        f @ (3 | 4) => Some((f - 2, 3)),
        _ => None,
    }
}

// A ClassDef's classes or ranges, as many as reading it walks.
fn class_entries(def: &[u8]) -> usize {
    let count = |(format, w)| read_uint(def, if format == 1 { 2 + w } else { 2 }, w);
    class_shape(def).and_then(count).unwrap_or(0) as usize
}

// Format 1's first glyph and its classes, each of `w` bytes, as many as the table holds.
fn class_array(def: &[u8], w: usize) -> (u32, impl ExactSizeIterator<Item = u32> + '_) {
    let start = read_uint(def, 2, w).unwrap_or(0);
    let held = def.len().saturating_sub(2 + 2 * w) / w;
    let count = (read_uint(def, 2 + w, w).unwrap_or(0) as usize).min(held);
    (start, (0..count).map(move |i| read_uint(def, 2 + 2 * w + i * w, w).unwrap_or(0)))
}

// Format 2's ranges, as many as the table holds.
fn class_range_count(def: &[u8], w: usize) -> usize {
    let held = def.len().saturating_sub(2 + w) / (2 * w + 2);
    (read_uint(def, 2, w).unwrap_or(0) as usize).min(held)
}

// Format 2's range `i` below its count: first, last and class, glyphs of `w` bytes and classes of 2.
fn class_range(def: &[u8], w: usize, i: usize) -> (u32, u32, u32) {
    let at = 2 + w + i * (2 * w + 2);
    let read = |at, w| read_uint(def, at, w).unwrap_or(0);
    (read(at, w), read(at + w, w), read(at + 2 * w, 2))
}

fn class_ranges(def: &[u8], w: usize) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
    (0..class_range_count(def, w)).map(move |i| class_range(def, w, i))
}

// Whether a glyph `set` holds takes `class`. Class 0 is every glyph a ClassDef lists as 0 or leaves out, the latter
// found in the gaps before, between and after its ranges, each gap walk reading on from the glyph it last found.
fn class_intersects(def: &[u8], set: &GlyphSet, class: u32, work: &Work) -> bool {
    let Some((format, w)) = class_shape(def) else { return false };
    if format == 1 {
        let (start, classes) = class_array(def, w);
        if class == 0 {
            let Some(g) = next(set, u32::MAX) else { return false };
            if g < start || next(set, start.wrapping_add(classes.len() as u32).wrapping_sub(1)).is_some() {
                return true;
            }
        }
        return classes.zip(start..).any(|(c, g)| c == class && set.contains(&g));
    }
    if class == 0 {
        let (mut g, mut n) = (u32::MAX, next(set, u32::MAX));
        for (first, last, _) in class_ranges(def, w) {
            if n.is_some_and(|n| n < first) {
                return true;
            }
            g = past(g, last);
            if n.is_some_and(|n| n <= g) {
                n = next(set, g);
            }
        }
        if n.is_some() {
            return true;
        }
    }
    class_ranges(def, w).any(|(first, last, c)| c == class && within(set, first, last, work).next().is_some())
}

// The glyphs of `set` that take `class`, into `out`; class 0's from the gaps and those listed as 0.
fn class_glyphs(def: &[u8], set: &GlyphSet, class: u32, out: &mut GlyphSet, work: &Work) {
    let Some((format, w)) = class_shape(def) else { return };
    let mut add = |g: u32| {
        out.insert(g);
    };
    if format == 1 {
        let (start, classes) = class_array(def, w);
        if class == 0 {
            set.iter().take_while(|&g| g < start).for_each(&mut add);
            let mut g = start.wrapping_add(classes.len() as u32).wrapping_sub(1);
            while let Some(n) = next(set, g) {
                add(n);
                g = n;
            }
        }
        classes.zip(start..).filter(|&(c, g)| c == class && set.contains(&g)).for_each(|(_, g)| add(g));
        return;
    }
    if class == 0 {
        let (mut g, mut n) = (u32::MAX, next(set, u32::MAX));
        for (first, last, _) in class_ranges(def, w) {
            while let Some(m) = n.filter(|&m| m < first) {
                add(m);
                n = next(set, m);
            }
            g = past(g, last);
            if n.is_some_and(|m| m <= g) {
                n = next(set, g);
            }
        }
        while let Some(m) = n {
            add(m);
            n = next(set, m);
        }
    }
    let count = read_uint(def, 2, w).unwrap_or(0);
    if count as usize > set.len().saturating_mul(bit_storage(count)) {
        if work.spend(set.words()) {
            set.iter().filter(|&g| class_def::class_of(def, g) == class).for_each(add);
        }
        return;
    }
    for (first, last, c) in class_ranges(def, w) {
        if c == class {
            within(set, first, last, work).for_each(&mut add);
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/otl/closure.rs"]
mod tests;
