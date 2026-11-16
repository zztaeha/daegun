use super::apply::{chained, sequence, Coverages, Engine, Peek, Rule, Side, ValueList, SEQUENCE_LIMIT};
use crate::daecore::daeshaper::plan::RANDOM_PICK;
use crate::daecore::daeshaper::run::{class, Run};
use super::{
    long, LookupKinds, nth_table_width, resolve_extension, table_at_width, tables, tables_of, ClassDef, Coverage,
    LayoutTable, Lookup, MEDIUM,
};
use alloc::vec::Vec;
use crate::daecore::daetype::decoder::{read_u16_be, read_uint};

pub(crate) const SINGLE: u16 = 1;
pub(crate) const MULTIPLE: u16 = 2;
pub(crate) const ALTERNATE: u16 = 3;
pub(crate) const LIGATURE: u16 = 4;
pub(crate) const CONTEXT: u16 = 5;
pub(crate) const CHAIN_CONTEXT: u16 = 6;
pub(crate) const EXTENSION: u16 = 7;
pub(crate) const REVERSE_CHAIN_SINGLE: u16 = 8;

pub(crate) const KINDS: LookupKinds = LookupKinds {
    last: REVERSE_CHAIN_SINGLE,
    extension: EXTENSION,
    context: CONTEXT,
    chain_context: CHAIN_CONTEXT,
    pair_pos: None,
    ligature: Some(LIGATURE),
    reverse: Some(REVERSE_CHAIN_SINGLE),
    marks: &[],
};

// `lookup` at the current glyph.
pub(crate) fn at_glyph<'a>(e: &mut Engine<'a>, lookup: &Lookup<'a>) -> bool {
    e.each_subtable(lookup, &KINDS, apply_subtable)
}

// Each reader takes the width of its format's glyphs, `W`: 2, or 3 for the 24-bit twins `MEDIUM` marks.
fn apply_subtable(e: &mut Engine, kind: u16, data: &[u8]) -> bool {
    match kind {
        SINGLE => single::<2>(e, data),
        MULTIPLE => multiple::<2>(e, data),
        ALTERNATE => alternate::<2>(e, data),
        LIGATURE => ligature::<2>(e, data),
        CONTEXT => sequence::<2>(e, data),
        CHAIN_CONTEXT => chained::<2>(e, data),
        REVERSE_CHAIN_SINGLE => reverse_chain_single::<2>(e, data),
        _ if kind & MEDIUM != 0 => apply_medium(e, kind & !MEDIUM, data),
        _ => false,
    }
}

#[inline(never)]
fn apply_medium(e: &mut Engine, kind: u16, data: &[u8]) -> bool {
    match kind {
        SINGLE => single::<3>(e, data),
        MULTIPLE => multiple::<3>(e, data),
        ALTERNATE => alternate::<3>(e, data),
        LIGATURE => ligature::<3>(e, data),
        CONTEXT => sequence::<3>(e, data),
        CHAIN_CONTEXT => chained::<3>(e, data),
        REVERSE_CHAIN_SINGLE => reverse_chain_single::<3>(e, data),
        _ => false,
    }
}

// The format of a type with one 16-bit format, or of its 24-bit twin.
const fn only_format(width: usize) -> u16 {
    if width == 3 { 2 } else { 1 }
}

// Formats 1 and 2, or 3 and 4. A delta wraps at the glyph width.
fn single<const W: usize>(e: &mut Engine, data: &[u8]) -> bool {
    let glyph = e.run.current().id;
    let l = long(W);
    let Some(index) = e.coverage(data, 2, l).index_of(glyph) else { return false };
    let subst = match read_u16_be(data, 0).map(|format| format.wrapping_sub(if W == 3 { 2 } else { 0 })) {
        Some(1) => glyph.wrapping_add(read_uint(data, 2 + l, W).unwrap_or(0)) & if W == 3 { 0xFF_FFFF } else { 0xFFFF },
        Some(2) => match read_uint(data, 2 + l, W) {
            Some(count) if index < count => {
                let Some(g) = read_uint(data, 2 + l + W + index as usize * W, W) else { return false };
                g
            }
            _ => return false,
        },
        _ => return false,
    };

    e.substitute(subst);
    true
}

// A covered glyph's list of glyphs: its MultipleSubst sequence or AlternateSubst set, empty where it's missing.
fn covered_glyphs<'a, const W: usize>(e: &Engine, data: &'a [u8]) -> Option<ValueList<'a, W>> {
    if read_u16_be(data, 0) != Some(only_format(W)) {
        return None;
    }
    let l = long(W);
    let index = e.coverage(data, 2, l).index_of(e.run.current().id)?;
    let list = nth_table_width(data, 2 + l, W, W, index);
    Some(ValueList::at(list, 2, read_u16_be(list, 0).unwrap_or(0)))
}

fn multiple<const W: usize>(e: &mut Engine, data: &[u8]) -> bool {
    let Some(substitutes) = covered_glyphs::<W>(e, data) else { return false };

    match substitutes.len {
        0 => {
            e.run.delete_current();
            true
        }
        1 => match substitutes.get(0) {
            Some(g) => {
                e.substitute(g);
                true
            }
            None => false,
        },
        count => {
            // A sequence cut short by the table's end matches nothing, so it's refused before a piece is written.
            if substitutes.get((count - 1).into()).is_none() {
                return false;
            }
            let cur = e.run.current();
            let class = if cur.glyph_is(class::LIGATURE) { class::BASE } else { 0 };
            let lig_id = cur.ligature_id();

            for i in 0..count {
                let Some(g) = substitutes.get(i.into()) else { return false };
                if lig_id == 0 {
                    e.run.current_mut().make_component(i);
                }
                e.put_piece(g, class);
            }

            e.run.pass_over();
            true
        }
    }
}

fn alternate<const W: usize>(e: &mut Engine, data: &[u8]) -> bool {
    let Some(alternates) = covered_glyphs::<W>(e, data).filter(|a| a.len != 0) else { return false };

    let glyph_mask = e.run.current().mask;
    let shift = e.mask().trailing_zeros();
    let mut alt_index = (e.mask() & glyph_mask) >> shift;

    if alt_index == RANDOM_PICK && e.random {
        e.run.unsafe_to_break_all();
        alt_index = e.next_random() % u32::from(alternates.len) + 1;
    }

    let Ok(alt_index) = u16::try_from(alt_index) else { return false };
    let Some(index) = alt_index.checked_sub(1) else { return false };
    let Some(g) = alternates.get(index.into()) else { return false };

    e.substitute(g);
    true
}

// Each ligature in turn: a failure marks what it read unsafe to concat before the next applies, one whose second glyph
// isn't the next read failing on it. It holds its glyph, component count, then the components after the first.
fn ligature<const W: usize>(e: &mut Engine, data: &[u8]) -> bool {
    if read_u16_be(data, 0) != Some(only_format(W)) {
        return false;
    }
    let l = long(W);
    let Some(index) = e.coverage(data, 2, l).index_of(e.run.current().id) else { return false };
    let set = nth_table_width(data, 2 + l, W, W, index);
    // Peeked at each subtable, each read paid: a lookup rarely holds two sets, and keeping it cost Latin about 1%.
    let next = match e.peek(e.run.cursor, Side::Input) {
        Peek::Glyph(glyph, end) => Some((glyph, end)),
        Peek::Optional(ignorable, at) => match e.peek(at, Side::Input) {
            Peek::Glyph(after, end) => return ligature_across::<W>(e, set, index, (ignorable, after, end)),
            _ => None,
        },
        Peek::Stop(end) => return ligature_at_end::<W>(e, set, end),
    };
    if let Some((glyph, end)) = next
        && read_u16_be(set, 0).is_some_and(|count| count > LigatureIndex::SHORT)
        && index_rules_out(e, index, glyph)
    {
        let idx = e.run.cursor;
        e.unsafe_to_concat(idx..end);
        return false;
    }
    let (glyph, end) = next.unzip();
    each_ligature::<W>(e, set, end.unwrap_or(0), |lig| {
        glyph.is_some()
            && (2..=SEQUENCE_LIMIT).contains(&usize::from(read_u16_be(lig, W).unwrap_or(0)))
            && read_uint(lig, W + 2, W) != glyph
    })
}

// The set's ligatures in turn until one forms; one `refuses` fails as its match would, marked to `end`. A set may name
// one ligature over and over, so each tried costs the run one, paid as the set ends: past what's left, none is tried.
fn each_ligature<const W: usize>(e: &mut Engine, set: &[u8], end: usize, refuses: impl Fn(&[u8]) -> bool) -> bool {
    let idx = e.run.cursor;
    let count = usize::from(read_u16_be(set, 0).unwrap_or(0));
    let room = count.min(e.run.ops_room());
    let formed = (0..room).position(|i| {
        let lig = table_at_width(set, 2 + i * W, W);
        if refuses(lig) {
            e.unsafe_to_concat(idx..end);
            return false;
        }
        apply_ligature::<W>(e, lig)
    });
    e.run.spend(formed.map_or(count, |i| i + 1));
    formed.is_some()
}

// A set whose next glyph is an ignorable: a ligature naming it matches through it, so its next glyph meets `after`;
// any other skips it, its first glyph meeting `after`. Each fails there as its full match would.
#[inline(never)]
fn ligature_across<const W: usize>(
    e: &mut Engine, set: &[u8], index: u32, (ignorable, after, end): (u32, u32, usize),
) -> bool {
    let idx = e.run.cursor;
    let long = read_u16_be(set, 0).is_some_and(|count| count > LigatureIndex::SHORT);
    if long && let Some(super::SetIndex::Ligatures(ligatures)) = e.subtable_index().and_then(|i| i.sets.as_ref())
        && ligatures.rules_out_across(index, ignorable, after)
    {
        e.unsafe_to_concat(idx..end);
        return false;
    }
    each_ligature::<W>(e, set, end, |lig| {
        let count = usize::from(read_u16_be(lig, W).unwrap_or(0));
        let component = |i: usize| read_uint(lig, W + 2 + (i - 1) * W, W);
        (2..=SEQUENCE_LIMIT).contains(&count)
            && if component(1) == Some(ignorable) {
                count > 2 && component(2) != Some(after)
            } else {
                component(1) != Some(after)
            }
    })
}

// A set where no glyph follows to be read: a ligature of two components or more fails there as its match would, marked
// with the walk's end. Apart, as inside the common loop it cost every shape; not cold, as every run's end reaches it.
#[inline(never)]
fn ligature_at_end<const W: usize>(e: &mut Engine, set: &[u8], end: usize) -> bool {
    let longer = |lig: &[u8]| (2..=SEQUENCE_LIMIT).contains(&usize::from(read_u16_be(lig, W).unwrap_or(0)));
    each_ligature::<W>(e, set, end, longer)
}

// Out of line: inlined, it slowed shapes that never reach it, Devanagari's by 1,300 cycles on equal instructions.
#[inline(never)]
fn index_rules_out(e: &Engine, set: u32, second: u32) -> bool {
    match e.subtable_index().and_then(|i| i.sets.as_ref()) {
        Some(super::SetIndex::Ligatures(ligatures)) => ligatures.rules_out(set, second),
        _ => false,
    }
}

// Each set's ligatures by their second glyph and third, where every ligature in it has 2 to 64 glyphs: a set none
// of whose ligatures can take the next glyph, or the glyphs about an ignorable, is passed without reading it.
pub(crate) struct LigatureIndex {
    // Each set's run of seconds, sorted, or None where it is short or a ligature of another length must be tried.
    sets: Vec<Option<(u32, u32)>>,
    // Every set's seconds, then the same runs as sorted (second, third) pairs, a third of 0 (two glyphs, or unreadable)
    // only keeping a set. One allocation, so the index stays the size every subtable's record holds.
    glyphs: Vec<u16>,
}

impl LigatureIndex {
    // A set this short is read faster in turn than through its index.
    const SHORT: u16 = 4;

    // None without a long set, or where the font's allowance or `work` can't cover it, which bounds what sets sharing
    // their offsets can make it hold.
    pub(crate) fn build(
        data: &[u8], reserve: &mut impl FnMut(usize) -> bool, work: &mut usize,
    ) -> Option<LigatureIndex> {
        let long = |set: &[u8]| read_u16_be(set, 0).filter(|&count| count > Self::SHORT);
        let sets = usize::from(read_u16_be(data, 4).unwrap_or(0));
        if !super::spend_work(work, sets) {
            return None;
        }
        let ligatures: usize = tables(data, 4).filter_map(long).map(usize::from).sum();
        if !super::spend_work(work, ligatures) {
            return None;
        }
        // Sets sharing one can state more bytes than a 32-bit usize counts, which no allowance covers.
        let bytes = ligatures.checked_mul(2 + 4).and_then(|b| b.checked_add(sets * size_of::<Option<(u32, u32)>>()));
        if ligatures == 0 || !bytes.is_some_and(reserve) {
            return None;
        }
        let mut index = LigatureIndex { sets: Vec::with_capacity(sets), glyphs: Vec::new() };
        let mut pairs: Vec<[u16; 2]> = Vec::with_capacity(ligatures);
        for set in tables(data, 4) {
            if long(set).is_none() {
                index.sets.push(None);
                continue;
            }
            let start = pairs.len();
            let mut open = false;
            for lig in tables(set, 0) {
                match (usize::from(read_u16_be(lig, 2).unwrap_or(0)), read_u16_be(lig, 4)) {
                    (count @ 2..=SEQUENCE_LIMIT, Some(second)) => {
                        pairs.push([second, if count > 2 { read_u16_be(lig, 6).unwrap_or(0) } else { 0 }]);
                    }
                    (2..=SEQUENCE_LIMIT, None) => {}
                    _ => open = true,
                }
            }
            if open {
                pairs.truncate(start);
                index.sets.push(None);
            } else {
                pairs[start..].sort_unstable();
                index.sets.push(Some((start as u32, pairs.len() as u32)));
            }
        }
        index.glyphs.reserve_exact(pairs.len() * 3);
        index.glyphs.extend(pairs.iter().map(|&[second, _]| second));
        index.glyphs.extend(pairs.iter().flatten());
        Some(index)
    }

    pub(crate) fn bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        heap_of(&self.sets) + heap_of(&self.glyphs)
    }

    fn rules_out(&self, set: u32, second: u32) -> bool {
        let Some(&Some((start, end))) = self.sets.get(set as usize) else { return false };
        let second = u16::try_from(second).ok();
        self.glyphs
            .get(start as usize..end as usize)
            .is_some_and(|s| second.is_none_or(|g| s.binary_search(&g).is_err()))
    }

    // An ignorable a ligature names is matched, so its third glyph meets `after`; one it doesn't is skipped.
    fn rules_out_across(&self, set: u32, ignorable: u32, after: u32) -> bool {
        let Some(&Some((start, end))) = self.sets.get(set as usize) else { return false };
        let (start, end, ligatures) = (start as usize, end as usize, self.glyphs.len() / 3);
        let (Some(seconds), Some(pairs)) =
            (self.glyphs.get(start..end), self.glyphs.get(ligatures + start * 2..ligatures + end * 2))
        else {
            return false;
        };
        let pairs = pairs.as_chunks::<2>().0;
        let (named, after16) = (u16::try_from(ignorable).ok(), u16::try_from(after).ok());
        !(after != ignorable && after16.is_some_and(|a| seconds.binary_search(&a).is_ok()))
            && named.zip(after16).is_none_or(|(i, a)| pairs.binary_search(&[i, a]).is_err())
            && named.is_none_or(|i| pairs.binary_search(&[i, 0]).is_err())
    }
}

// A null ligature has no components and forms nothing.
fn apply_ligature<const W: usize>(e: &mut Engine, lig: &[u8]) -> bool {
    let lig_glyph = read_uint(lig, 0, W).unwrap_or(0);
    let count = read_u16_be(lig, W).unwrap_or(0);
    match count {
        0 => return false,
        1 => {
            e.substitute(lig_glyph);
            return true;
        }
        _ => {}
    }

    let components = ValueList::<W>::at(lig, W + 2, count - 1);
    match e.match_input(count, |g, k| components.get(k) == Some(g)) {
        Ok(input) => {
            e.ligate(usize::from(count), input.end, input.components, lig_glyph);
            true
        }
        Err(end) => {
            let idx = e.run.cursor;
            e.unsafe_to_concat(idx..end);
            false
        }
    }
}

// The coverage, the backtrack's and lookahead's coverages, each list counted in 16 bits, then the substitutes.
// A failed backtrack read up to the current glyph, a failed lookahead from the backtrack's start.
fn reverse_chain_single<const W: usize>(e: &mut Engine, data: &[u8]) -> bool {
    if read_u16_be(data, 0) != Some(only_format(W)) {
        return false;
    }
    let l = long(W);
    let Some(index) = e.coverage(data, 2, l).index_of(e.run.current().id) else { return false };

    if e.is_nested() {
        return false;
    }

    let indexes = e.rule_indexes();
    let Some(back_count) = read_u16_be(data, 2 + l) else { return false };
    let back = Coverages::<W>::new(data, 4 + l, back_count, indexes);
    let indexes = indexes.get(usize::from(back_count)..).unwrap_or_default();

    let at = 4 + l + back_count as usize * W;
    let Some(ahead_count) = read_u16_be(data, at) else { return false };
    let ahead = Coverages::<W>::new(data, at + 2, ahead_count, indexes);

    let at = at + 2 + ahead_count as usize * W;
    let Some(subst_count) = read_uint(data, at, W) else { return false };
    let subst = (index < subst_count).then(|| read_uint(data, at + W + index as usize * W, W)).flatten();
    let Some(subst) = subst else { return false };

    let after = e.run.cursor + 1;
    let (from, end) = match e.match_backtrack(back_count, |g, k| back.covers(k, g)) {
        Err(from) => (from, after),
        Ok(from) => match e.match_lookahead(after, ahead_count, |g, k| ahead.covers(k, g)) {
            Ok(end) => {
                e.run.unsafe_to_break_across(from, end);
                e.substitute_here(subst);
                return true;
            }
            Err(end) => (from, end),
        },
    };
    e.unsafe_to_concat_across(from, end);
    false
}

// Whether lookup `index` substitutes `glyphs`; with `zero_context`, a rule reading glyphs around them doesn't. `run`
// pays for each subtable and set entry, as offsets may name one over and over; past its edits, nothing substitutes.
pub(crate) fn would_apply(gsub: &LayoutTable, index: u16, glyphs: &[u32], zero_context: bool, run: &mut Run) -> bool {
    let ctx = &mut WouldApplyContext { glyphs, zero_context, run };
    if ctx.glyphs.is_empty() {
        return false;
    }
    let Some(lookup) = gsub.lookup(index) else { return false };

    ctx.run.spend(lookup.subtable_count.into())
        && (0..lookup.subtable_count).any(|i| {
            resolve_extension(lookup.subtable(i), lookup.kind, EXTENSION)
                .is_some_and(|(kind, data)| would_apply_subtable(ctx, kind, data))
        })
}

struct WouldApplyContext<'a> {
    glyphs: &'a [u32],
    zero_context: bool,
    run: &'a mut Run,
}

impl WouldApplyContext<'_> {
    // Whether the glyphs are an input of `count`, each after the first giving the value `input` holds for it.
    fn matches<const W: usize>(&self, count: u16, input: ValueList<W>, value_of: impl Fn(u32) -> u32) -> bool {
        usize::from(count) == self.glyphs.len()
            && self.glyphs[1..].iter().zip(0..).all(|(&g, i)| input.get(i) == Some(value_of(g)))
    }

    // Whether the glyphs are an input of `count`, each in its coverage.
    fn covered<const W: usize>(&self, count: u16, input: Coverages<W>) -> bool {
        usize::from(count) == self.glyphs.len() && self.glyphs.iter().zip(0..).all(|(&g, i)| input.covers(i, g))
    }

    // A rule with a backtrack or lookahead doesn't apply where only the glyphs are known.
    fn without_context(&self, backtrack: u16, lookahead: u16) -> bool {
        !self.zero_context || (backtrack == 0 && lookahead == 0)
    }

    // The set's entries paid for before any is read: false once the run can't pay.
    fn pays_for(&mut self, set: &[u8]) -> bool {
        self.run.spend(read_u16_be(set, 0).unwrap_or(0).into())
    }
}

fn would_apply_subtable(ctx: &mut WouldApplyContext, kind: u16, data: &[u8]) -> bool {
    let first = ctx.glyphs[0];
    let format = read_u16_be(data, 0);
    let covers = |width: usize| ctx.glyphs.len() == 1 && Coverage::new(table_at_width(data, 2, width)).contains(first);
    match kind {
        SINGLE if matches!(format, Some(1 | 2)) => covers(2),
        SINGLE if matches!(format, Some(3 | 4)) => covers(4),
        MULTIPLE | ALTERNATE | REVERSE_CHAIN_SINGLE if format == Some(1) => covers(2),
        MULTIPLE | ALTERNATE | REVERSE_CHAIN_SINGLE if format == Some(2) => covers(4),
        LIGATURE if format == Some(1) => would_ligate::<2>(ctx, data),
        LIGATURE if format == Some(2) => would_ligate::<3>(ctx, data),
        CONTEXT => would_apply_context(ctx, data),
        CHAIN_CONTEXT => would_apply_chain_context(ctx, data),
        _ => false,
    }
}

fn would_ligate<const W: usize>(ctx: &mut WouldApplyContext, data: &[u8]) -> bool {
    let l = long(W);
    let Some(index) = Coverage::new(table_at_width(data, 2, l)).index_of(ctx.glyphs[0]) else { return false };
    let set = nth_table_width(data, 2 + l, W, W, index);
    ctx.pays_for(set)
        && tables_of::<W>(set, 0).any(|lig| {
            let count = read_u16_be(lig, W).unwrap_or(0);
            ctx.matches(count, ValueList::<W>::at(lig, W + 2, count.saturating_sub(1)), |g| g)
        })
}

// The rule sets a context subtable of rule sets holds for the first glyph, `sets` counting them and the width of
// their offsets: by its coverage index, or by its class in `classes` alone.
fn first_rule_set<'a>(
    data: &'a [u8], (count_at, count, width): (usize, usize, usize), first: u32, classes: Option<ClassDef>,
) -> &'a [u8] {
    // The first glyph must be in the subtable's coverage, which a class format's classes don't stand in for.
    let covered = Coverage::new(table_at_width(data, 2, long(width))).index_of(first);
    let index = match classes {
        Some(classes) => covered.map(|_| classes.class_of(first)),
        None => covered,
    };
    index.map_or(&[][..], |index| nth_table_width(data, count_at, count, width, index))
}

// Whether a rule of a set whose rules lie at offsets of `RW` bytes, holding values of `VW`, takes the glyphs.
fn set_matches<const RW: usize, const VW: usize>(
    ctx: &mut WouldApplyContext, set: &[u8], chain: bool, value_of: impl Fn(u32) -> u32,
) -> bool {
    ctx.pays_for(set)
        && tables_of::<RW>(set, 0).any(|rule| {
            if chain {
                let rule = Rule::<VW>::read(rule, true);
                ctx.without_context(rule.back.len, rule.ahead.len) && ctx.matches(rule.count, rule.input, &value_of)
            } else {
                let rule = Rule::<VW>::read(rule, false);
                ctx.matches(rule.count, rule.input, &value_of)
            }
        })
}

// Formats 1 to 3 and their 24-bit twins, 4 to 6, laid out as `apply_context` reads them.
fn would_apply_context(ctx: &mut WouldApplyContext, data: &[u8]) -> bool {
    let first = ctx.glyphs[0];
    match read_u16_be(data, 0) {
        Some(1) => set_matches::<2, 2>(ctx, first_rule_set(data, (4, 2, 2), first, None), false, |g| g),
        Some(4) => set_matches::<2, 3>(ctx, first_rule_set(data, (6, 3, 3), first, None), false, |g| g),
        Some(format @ (2 | 5)) => {
            let (l, w) = if format == 2 { (2, 2) } else { (4, 3) };
            let classes = ClassDef::new(table_at_width(data, 2 + l, l));
            let set = first_rule_set(data, (2 + 2 * l, w, w), first, Some(classes));
            if format == 2 {
                set_matches::<2, 2>(ctx, set, false, |g| classes.class_of(g))
            } else {
                set_matches::<3, 2>(ctx, set, false, |g| classes.class_of(g))
            }
        }
        Some(3) => {
            let count = read_u16_be(data, 2).unwrap_or(0);
            ctx.covered(count, Coverages::<2>::new(data, 6, count, &[]))
        }
        Some(6) => {
            let count = read_u16_be(data, 2).unwrap_or(0);
            ctx.covered(count, Coverages::<3>::new(data, 6, count, &[]))
        }
        _ => false,
    }
}

// Formats 1 to 3 and the 24-bit twins of 1 and 2, laid out as `apply_chain_context` reads them.
fn would_apply_chain_context(ctx: &mut WouldApplyContext, data: &[u8]) -> bool {
    let first = ctx.glyphs[0];
    match read_u16_be(data, 0) {
        Some(1) => set_matches::<2, 2>(ctx, first_rule_set(data, (4, 2, 2), first, None), true, |g| g),
        Some(4) => set_matches::<3, 3>(ctx, first_rule_set(data, (6, 3, 3), first, None), true, |g| g),
        Some(format @ (2 | 5)) => {
            let (l, count) = if format == 2 { (2, 2) } else { (4, 2) };
            let classes = ClassDef::new(table_at_width(data, 2 + 2 * l, l));
            let width = if format == 2 { 2 } else { 3 };
            let set = first_rule_set(data, (2 + 4 * l, count, width), first, Some(classes));
            if format == 2 {
                set_matches::<2, 2>(ctx, set, true, |g| classes.class_of(g))
            } else {
                set_matches::<3, 2>(ctx, set, true, |g| classes.class_of(g))
            }
        }
        Some(3) => {
            let word = |at: usize| read_u16_be(data, at).unwrap_or(0);
            let input_at = 4 + usize::from(word(2)) * 2;
            let count = word(input_at);
            let lookahead_at = input_at + 2 + usize::from(count) * 2;
            ctx.without_context(word(2), word(lookahead_at))
                && ctx.covered(count, Coverages::<2>::new(data, input_at + 2, count, &[]))
        }
        _ => false,
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/ot/gsub.rs"]
mod tests;
