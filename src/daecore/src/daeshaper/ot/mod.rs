pub(crate) mod aat;
pub(crate) mod apply;
pub(crate) mod gsub;
pub(crate) mod gpos;
pub(crate) mod digest;
pub mod tag;
pub(crate) mod kern;
pub(crate) mod kerx;
pub(crate) mod morx;
pub(crate) mod morx_flags;

use crate::daecore::daetype::decoder::{
    midpoint_search, read_u16_be, read_u24_be, read_u32_be, read_uint, resolve_offset16, window,
};
use crate::daecore::daetype::format::feature_variations::FeatureVariations;
use crate::daecore::daekit::cache::shaping::IndexEntries;
use crate::daecore::daetype::format::index::{Entries, GlyphBits, SparseIndex};
use crate::daecore::daekit::base::sync::Shared;
use self::tag::Tag;

pub(crate) const COVERAGE_ABSENT: u16 = u16::MAX;

// The table the Offset16 at `at` names, empty where the offset is null, so it reads as zeros: a coverage of no glyph,
// class 0, an anchor at the origin, a rule of no input, an empty list.
pub(crate) fn table_at(data: &[u8], at: usize) -> &[u8] {
    table_at_width(data, at, 2)
}

// `table_at` for an offset of `width` bytes.
#[inline(always)]
pub(crate) fn table_at_width(data: &[u8], at: usize, width: usize) -> &[u8] {
    match read_uint(data, at, width) {
        None | Some(0) => &[],
        Some(off) => data.get(off as usize..).unwrap_or_default(),
    }
}

// The table the `index`th of the offsets of `width` bytes counted in `count` bytes at `count_at` names, empty past
// the count.
#[inline(always)]
pub(crate) fn nth_table_width(data: &[u8], count_at: usize, count: usize, width: usize, index: u32) -> &[u8] {
    match read_uint(data, count_at, count) {
        Some(n) if index < n => table_at_width(data, count_at + count + index as usize * width, width),
        _ => &[],
    }
}

// Each table the Offset16s counted at `count_at` name.
pub(crate) fn tables(data: &[u8], count_at: usize) -> impl Iterator<Item = &[u8]> {
    tables_of::<2>(data, count_at)
}

// `tables` for offsets of `W` bytes, still counted in 16 bits.
#[inline(always)]
pub(crate) fn tables_of<const W: usize>(data: &[u8], count_at: usize) -> impl Iterator<Item = &[u8]> {
    let count = read_u16_be(data, count_at).unwrap_or(0);
    (0..count).map(move |i| table_at_width(data, count_at + 2 + usize::from(i) * W, W))
}

// `tables` for a count of `count` bytes and offsets of `width`.
pub(crate) fn tables_width(data: &[u8], count_at: usize, count: usize, width: usize) -> impl Iterator<Item = &[u8]> {
    let n = read_uint(data, count_at, count).unwrap_or(0);
    (0..n as usize).map(move |i| table_at_width(data, count_at + count + i * width, width))
}

// The offset width of a subtable's first tables, for one of `W`-byte glyphs: a 24-bit format, built for fonts past
// 65,535 glyphs, reaches them at 32-bit offsets.
pub(crate) const fn long(width: usize) -> usize {
    if width == 3 { 4 } else { 2 }
}

// Set on a subtable's type where its format is a 24-bit one, so shaping reads it apart from the 16-bit formats.
pub(crate) const MEDIUM: u16 = 0x100;

// `kind`, with `MEDIUM` set where `data`'s format is the 24-bit twin of one: format 2 of a type with one 16-bit
// format, 3 and 4 of SingleSubst, SinglePos and PairPos, 4 to 6 of a context and 4 and 5 of a chain.
pub(crate) fn medium_kind(kind: u16, data: &[u8], kinds: &LookupKinds) -> u16 {
    let formats = match kind {
        _ if kind == kinds.context => 4..=6,
        _ if kind == kinds.chain_context => 4..=5,
        _ if kind == 1 || Some(kind) == kinds.pair_pos => 3..=4,
        _ if kind == kinds.extension || !(1..=8).contains(&kind) => return kind,
        _ => 2..=2,
    };
    if read_u16_be(data, 0).is_some_and(|format| formats.contains(&format)) { kind | MEDIUM } else { kind }
}

#[derive(Clone, Copy)]
pub(crate) struct Coverage<'a> {
    data: &'a [u8],
    index: Option<&'a crate::daecore::daetype::format::index::SparseIndex>,
}

impl<'a> Coverage<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Coverage { data, index: None }
    }

    pub(crate) fn at(data: &'a [u8], at: usize) -> Self {
        Coverage::new(table_at(data, at))
    }

    pub(crate) fn with_index(
        data: &'a [u8],
        index: Option<&'a crate::daecore::daetype::format::index::SparseIndex>,
    ) -> Self {
        Coverage { data, index }
    }

    // Every glyph and its coverage index, for an index: None where the glyphs don't ascend, a range runs backward, or
    // an index overflows or reads as absent. A range's indexes ascend, so its last is the one to check.
    pub(crate) fn each_entry(&self, e: &mut Entries) -> Option<()> {
        let d = self.data;
        let format = read_u16_be(d, 0)?;
        let count = usize::from(read_u16_be(d, 2)?);
        let mut next = 0u32;
        match format {
            1 => {
                for i in 0..count {
                    let g = u32::from(read_u16_be(d, 4 + i * 2)?);
                    if g < next {
                        return None;
                    }
                    next = g + 1;
                    e.put(g, i as u32);
                }
            }
            2 => {
                for i in 0..count {
                    let r = window::<6>(d, 4 + i * 6)?;
                    let (start, end, first) = (
                        u16::from_be_bytes([r[0], r[1]]),
                        u16::from_be_bytes([r[2], r[3]]),
                        u16::from_be_bytes([r[4], r[5]]),
                    );
                    if start > end || u32::from(start) < next {
                        return None;
                    }
                    first.checked_add(end - start).filter(|&last| last != COVERAGE_ABSENT)?;
                    next = u32::from(end) + 1;
                    e.put_run(start.into(), end.into(), first.into(), 1);
                }
            }
            3 | 4 => return medium_entries(d, format == 4, e),
            _ => return None,
        }
        Some(())
    }

    pub(crate) fn index_of(&self, glyph: u32) -> Option<u32> {
        if let Some(index) = self.index {
            let found = index.lookup(glyph);
            return (found != COVERAGE_ABSENT).then_some(found.into());
        }
        crate::daecore::daetype::format::coverage::coverage_index(self.data, glyph)
    }

    pub(crate) fn contains(&self, glyph: u32) -> bool {
        self.index_of(glyph).is_some()
    }

    // A null coverage, or one of another format, adds nothing.
    pub(crate) fn add_to_digest(&self, digest: &mut digest::Digest) {
        let d = self.data;
        let count = usize::from(read_u16_be(d, 2).unwrap_or(0));
        match read_u16_be(d, 0) {
            Some(1) => (0..count).map_while(|i| read_u16_be(d, 4 + i * 2)).for_each(|g| digest.add(g.into())),
            Some(2) => (0..count)
                .map_while(|i| Some((read_u16_be(d, 4 + i * 6)?, read_u16_be(d, 6 + i * 6)?)))
                .for_each(|(first, last)| digest.add_range(first.into(), last.into())),
            Some(3) => (0..read_u24_be(d, 2).unwrap_or(0) as usize)
                .map_while(|i| read_u24_be(d, 5 + i * 3))
                .for_each(|g| digest.add(g)),
            Some(4) => (0..read_u24_be(d, 2).unwrap_or(0) as usize)
                .map_while(|i| Some((read_u24_be(d, 5 + i * 9)?, read_u24_be(d, 8 + i * 9)?)))
                .for_each(|(first, last)| digest.add_range(first, last)),
            _ => {}
        }
    }
}

// `Coverage::each_entry` for formats 3 and 4, formats 1 and 2 in 24 bits, out of line. An index past 16 bits refuses
// the index, as do the conditions the 16-bit formats refuse it on.
#[inline(never)]
fn medium_entries(d: &[u8], ranges: bool, e: &mut Entries) -> Option<()> {
    let size = if ranges { 9 } else { 3 };
    let mut next = 0u32;
    for i in 0..read_u24_be(d, 2)? as usize {
        let field = |at: usize| read_u24_be(d, 5 + i * size + at);
        let (start, end, first) =
            if ranges { (field(0)?, field(3)?, field(6)?) } else { (field(0)?, field(0)?, i as u32) };
        if start > end || start < next || first + (end - start) >= u32::from(COVERAGE_ABSENT) {
            return None;
        }
        next = end + 1;
        e.put_run(start, end, first, 1);
    }
    Some(())
}

// The lookup types a table's subtable indexes build for.
#[derive(Clone, Copy)]
pub(crate) struct LookupKinds {
    // The highest lookup type the table defines.
    pub(crate) last: u16,
    pub(crate) extension: u16,
    pub(crate) context: u16,
    pub(crate) chain_context: u16,
    pub(crate) pair_pos: Option<u16>,
    pub(crate) ligature: Option<u16>,
    pub(crate) reverse: Option<u16>,
    pub(crate) marks: &'static [u16],
}

// What shaping reads of one lookup, built once a face and shared by its plans: its subtables' indexes, the union of
// their digests, whether it runs in reverse, and the lookups its context records name, each once.
#[derive(Default)]
pub(crate) struct LookupIndex {
    pub(crate) subtables: alloc::vec::Vec<SubtableIndex>,
    pub(crate) digest: digest::Digest,
    pub(crate) reverse: bool,
    pub(crate) nested: alloc::boxed::Box<[u16]>,
}

impl LookupIndex {
    pub(crate) fn heap_bytes(&self) -> usize {
        let tables = self.subtables.iter().map(SubtableIndex::heap_bytes).sum::<usize>();
        crate::daecore::daekit::base::sync::heap_of(&self.subtables) + tables + core::mem::size_of_val(&*self.nested)
    }
}

// A subtable shaping doesn't apply keeps the empty digest, so it gates every glyph out.
#[derive(Default)]
pub(crate) struct SubtableIndex {
    pub(crate) coverage: Option<SparseIndex>,
    pub(crate) class1: Option<SparseIndex>,
    // A pair's second glyph: its class in format 2; in format 1, 1 where any pair set names it, so a miss skips the
    // search.
    pub(crate) second: Option<SparseIndex>,
    // The class definitions or coverages a subtable reads past its first, in `rule_tables`' order.
    pub(crate) rules: alloc::boxed::Box<[Option<Shared<SparseIndex>>]>,
    pub(crate) digest: digest::Digest,
    // The subtable's type and start in its table, its Extension followed, so a glyph doesn't resolve it again;
    // Some(None) where the Extension is skipped, None where it wasn't resolved.
    pub(crate) resolved: Option<Option<(u16, usize)>>,
    pub(crate) sets: Option<SetIndex>,
}

// What the plan reads of a subtable past its coverage, so shaping doesn't: a ligature subtable's second glyphs, a
// context subtable's rule sets, or a chain format 3's list lengths and record count (`apply::chain_lists`).
pub(crate) enum SetIndex {
    Ligatures(gsub::LigatureIndex),
    RuleSets(RuleSets),
    Lists([u16; 4]),
}

// A format 1 or 2 context subtable's rule sets as a match first reads them: 64 bits a set of its rules' first read
// values, and each rule's first read itself, so a rule that fails on it is refused without reading the font.
pub(crate) struct RuleSets {
    digests: alloc::boxed::Box<[u64]>,
    // Set `i`'s rules' first reads, packed, are `firsts[starts[i]..starts[i + 1]]`. Both are empty where the face's
    // index bytes ran out, and a set's reads stop at the last rule its bytes hold an offset for.
    starts: alloc::boxed::Box<[u32]>,
    firsts: alloc::boxed::Box<[u32]>,
}

impl RuleSets {
    // Every bit set for a set it doesn't hold.
    pub(crate) fn digest(&self, set: u32) -> u64 {
        self.digests.get(set as usize).copied().unwrap_or(u64::MAX)
    }

    pub(crate) fn firsts(&self, set: u32) -> &[u32] {
        let bound = |i: usize| self.starts.get(i).map(|&at| at as usize);
        let set = set as usize;
        bound(set).zip(bound(set + 1)).and_then(|(from, to)| self.firsts.get(from..to)).unwrap_or_default()
    }

    fn heap_bytes(&self) -> usize {
        core::mem::size_of_val(&*self.digests) + core::mem::size_of_val(&*self.starts)
            + core::mem::size_of_val(&*self.firsts)
    }
}

impl SubtableIndex {
    pub(crate) fn heap_bytes(&self) -> usize {
        let SubtableIndex { coverage, class1, second, rules, digest: _, resolved: _, sets } = self;
        [coverage, class1, second].into_iter().flatten().map(|i| i.bytes()).sum::<usize>()
            + core::mem::size_of_val(&**rules)
            + match sets {
                Some(SetIndex::Ligatures(ligatures)) => ligatures.bytes(),
                Some(SetIndex::RuleSets(sets)) => sets.heap_bytes(),
                Some(SetIndex::Lists(_)) | None => 0,
            }
    }
}

// Whether a lookup runs in reverse, by its first subtable's type; None for one that applies nothing: of a type its
// table doesn't define, or an Extension lookup whose subtables mix types, refused as a reverse chain among others
// would edit with no rewrite open. It reads every subtable, so a caller pays for them.
pub(crate) fn reverse_of(lookup: &Lookup, kinds: &LookupKinds) -> Option<bool> {
    if !(1..=kinds.last).contains(&lookup.kind) {
        return None;
    }
    let kind_of = |i: u16| resolve_extension(lookup.subtable(i), lookup.kind, kinds.extension).map(|(kind, _)| kind);
    let first = (0..lookup.subtable_count).find_map(kind_of);
    let mixed = (0..lookup.subtable_count).any(|i| kind_of(i).is_some_and(|k| Some(k) != first));
    (lookup.kind != kinds.extension || !mixed).then(|| kinds.reverse.is_some_and(|r| first == Some(r)))
}

// A lookup that applies, `reverse` as `reverse_of` tells, whose subtables' entries the face's allowance can't hold:
// every subtable read where shaping meets it.
pub(crate) fn unindexed(reverse: bool) -> LookupIndex {
    LookupIndex { digest: digest::Digest::full(), reverse, ..LookupIndex::default() }
}

// A lookup that applies, `reverse` as `reverse_of` tells. `shared` gives the index of a class definition (true) or
// coverage (false) every subtable naming it shares.
pub(crate) fn lookup_index(
    lookup: &Lookup,
    kinds: &LookupKinds,
    reverse: bool,
    mut build: impl FnMut(&IndexEntries, u16) -> Option<SparseIndex>,
    mut shared: impl FnMut(&[u8], bool) -> Option<Shared<SparseIndex>>,
    mut reserve: impl FnMut(usize) -> bool,
    work: &mut usize,
) -> LookupIndex {
    let LookupKinds { extension, context, chain_context, pair_pos, ligature, .. } = *kinds;
    let mut out = alloc::vec::Vec::with_capacity(lookup.subtable_count as usize);
    let mut nested = Nested::default();
    for i in 0..lookup.subtable_count {
        let mut entry = SubtableIndex::default();
        let resolved = resolve_extension(lookup.subtable(i), lookup.kind, extension)
            .map(|(kind, data)| (medium_kind(kind, data, kinds), data));
        entry.resolved = resolved.map_or(Some(None), |(kind, data)| lookup.start_of(data).map(|at| Some((kind, at))));
        if !spend_work(work, 1) {
            entry.digest = digest::Digest::full();
        } else if let Some((kind, data)) = resolved
            && kind & MEDIUM != 0
        {
            medium_index(&mut entry, kind & !MEDIUM, data, kinds, &mut build, &mut nested, work);
        } else if let Some((kind, data)) = resolved
            && let Some(at) = first_coverage_at(kind, data, pair_pos, context, chain_context)
        {
            if !spend_work(work, stated_entries(table_at(data, at), false)) {
                entry.digest = digest::Digest::full();
                out.push(entry);
                continue;
            }
            let coverage = Coverage::at(data, at);
            coverage.add_to_digest(&mut entry.digest);
            if at == 2 {
                entry.coverage = build(&|e| coverage.each_entry(e), COVERAGE_ABSENT);
            }

            if ligature == Some(kind) && read_u16_be(data, 0) == Some(1) {
                entry.sets = gsub::LigatureIndex::build(data, &mut reserve, work).map(SetIndex::Ligatures);
            }
            if pair_pos == Some(kind) && read_u16_be(data, 0) == Some(1) {
                entry.second = pair_seconds(data, work).and_then(|seconds| {
                    build(&|e| { seconds.runs().for_each(|(a, b)| e.put_run(a, b, 1, 0)); Some(()) }, 0)
                });
            }
            if pair_pos == Some(kind) && read_u16_be(data, 0) == Some(2) {
                let mut class_at = |at: usize| {
                    spend_work(work, stated_entries(table_at(data, at), true))
                        .then(|| build(&|e| ClassDef::at(data, at).each_entry(e), 0))
                        .flatten()
                };
                entry.class1 = class_at(8);
                entry.second = class_at(10);
            }
            // The plan keeps a slot for each table a count claims, so the slots cost work and are drawn from the
            // allowance as indexes are; without them shaping reads the tables.
            let (at, class) = rule_tables(kind, data, kinds);
            if spend_work(work, at.len()) && reserve(at.len() * size_of::<Option<Shared<SparseIndex>>>()) {
                let mut rule_table = |at: usize| {
                    let table = table_at(data, at);
                    spend_work(work, stated_entries(table, class)).then(|| shared(table, class)).flatten()
                };
                entry.rules = at.into_iter().map(&mut rule_table).collect();
            }
            if kind == context || kind == chain_context {
                entry.sets = rule_sets(data, kind == chain_context, &mut reserve, work).map(SetIndex::RuleSets);
                apply::record_lookups(data, kind == chain_context, work, |index| nested.push(index));
            }
            if kind == chain_context && read_u16_be(data, 0) == Some(3) {
                entry.sets = apply::chain_lists(data).map(SetIndex::Lists);
            }
        }
        out.push(entry);
    }
    let mut digest = digest::Digest::default();
    out.iter().for_each(|entry| digest.union(&entry.digest));
    LookupIndex { subtables: out, digest, reverse, nested: nested.kept(reserve) }
}

// The lookups a lookup's context records name, each once. Records may name one over and over, so the list is settled
// whenever it holds twice as many as there are lookups.
#[derive(Default)]
struct Nested(alloc::vec::Vec<u16>);

impl Nested {
    const LOOKUPS: usize = 1 << 16;

    fn push(&mut self, index: u16) {
        if self.0.len() == 2 * Self::LOOKUPS {
            self.settle();
        }
        self.0.push(index);
    }

    fn settle(&mut self) {
        self.0.sort_unstable();
        self.0.dedup();
    }

    // Drawn from the face's allowance; past it none are kept, and shaping tells each apart as it meets it.
    fn kept(mut self, mut reserve: impl FnMut(usize) -> bool) -> alloc::boxed::Box<[u16]> {
        self.settle();
        if !reserve(self.0.len() * size_of::<u16>()) {
            return alloc::boxed::Box::default();
        }
        self.0.into_boxed_slice()
    }
}

// A 24-bit subtable's digest, its first coverage's index where that lies at the start, and the lookups its context
// records name. Its rules, sets and pairs keep no index: past 65,535 glyphs, the 16-bit readers matter more.
fn medium_index(
    entry: &mut SubtableIndex,
    kind: u16,
    data: &[u8],
    kinds: &LookupKinds,
    build: &mut impl FnMut(&IndexEntries, u16) -> Option<SparseIndex>,
    nested: &mut Nested,
    work: &mut usize,
) {
    let contextual = kind == kinds.context || kind == kinds.chain_context;
    let (at, width) = if contextual && read_u16_be(data, 0) == Some(6) { (6, 3) } else { (2, 4) };
    let coverage = Coverage::new(table_at_width(data, at, width));
    if !spend_work(work, stated_entries(coverage.data, false)) {
        entry.digest = digest::Digest::full();
        return;
    }
    coverage.add_to_digest(&mut entry.digest);
    if at == 2 {
        entry.coverage = build(&|e| coverage.each_entry(e), COVERAGE_ABSENT);
    }
    if contextual {
        apply::record_lookups(data, kind == kinds.chain_context, work, |index| nested.push(index));
    }
}

// What indexing a plan's lookups may read of one table, 16 entries a byte and 2^20 more: offsets sharing tables
// multiply a walk's reads, and past this a subtable's index passes every glyph and shaping reads the subtable itself.
pub(crate) fn index_work(table_len: usize) -> usize {
    table_len.saturating_mul(16).saturating_add(1 << 20)
}

// Takes `n` from `work`: false, leaving none, once it can't cover them.
pub(crate) fn spend_work(work: &mut usize, n: usize) -> bool {
    let left = work.checked_sub(n);
    *work = left.unwrap_or(0);
    left.is_some()
}

// The entries a coverage (false) or class definition (true) at `data`'s start states, at most one a byte of it: what
// indexing it reads.
fn stated_entries(data: &[u8], class: bool) -> usize {
    let count = match (read_u16_be(data, 0), class) {
        (Some(1), true) => read_u16_be(data, 4).map(u32::from),
        (Some(1 | 2), _) => read_u16_be(data, 2).map(u32::from),
        (Some(3), true) => read_u24_be(data, 5),
        (Some(3 | 4), _) => read_u24_be(data, 2),
        _ => None,
    };
    count.map_or(0, |n| n as usize).min(data.len())
}

// Per rule set of a format 1 or 2 context subtable, its rules' first read values hashed as input and lookahead halves:
// neither taking the glyph each reads next fails every rule. Empty sets, or rules reading nothing first, set all.
fn rule_sets(
    data: &[u8],
    chain: bool,
    reserve: &mut impl FnMut(usize) -> bool,
    work: &mut usize,
) -> Option<RuleSets> {
    let count_at = rule_set_count_at(data, chain)?;
    let sets = usize::from(read_u16_be(data, count_at)?);
    // Offsets past a set's bytes name no rule.
    let held = |set: &[u8]| usize::from(read_u16_be(set, 0).unwrap_or(0)).min(set.len() / 2);
    let mut all_held = 0;
    // All the work is paid before the allowance is drawn from, so running out strands nothing.
    for set in tables(data, count_at) {
        if !spend_work(work, 1 + held(set)) {
            return None;
        }
        all_held += held(set);
    }
    if sets == 0 || !reserve(sets * core::mem::size_of::<u64>()) {
        return None;
    }
    let bits = |first: Option<apply::FirstRead>| match first {
        Some(apply::FirstRead::Input(value)) => 1 << (value % 31),
        Some(apply::FirstRead::Ahead(value)) => 1 << (32 + value % 31),
        None => u64::MAX,
    };
    let mut digests = alloc::vec::Vec::with_capacity(sets);
    // Each rule's first read is kept too, while the face's index bytes last; no allowance covers a size past usize.
    let bytes = all_held.checked_add(sets + 1).and_then(|n| n.checked_mul(size_of::<u32>()));
    let kept = u32::try_from(all_held).is_ok() && bytes.is_some_and(reserve);
    let (mut starts, mut firsts) = (alloc::vec::Vec::new(), alloc::vec::Vec::new());
    if kept {
        (starts, firsts) = (alloc::vec::Vec::with_capacity(sets + 1), alloc::vec::Vec::with_capacity(all_held));
        starts.push(0);
    }
    for set in tables(data, count_at) {
        let (rules, held) = (usize::from(read_u16_be(set, 0).unwrap_or(0)), held(set));
        // A rule past its set's bytes reads nothing first.
        let mut digest = if rules == 0 || rules > held { u64::MAX } else { 0 };
        for rule in tables(set, 0).take(held) {
            let first = apply::first_read::<2>(rule, chain);
            digest |= bits(first);
            if kept {
                firsts.push(apply::FirstRead::pack(first));
            }
        }
        digests.push(digest);
        if kept {
            starts.push(firsts.len() as u32);
        }
    }
    let (digests, starts, firsts) = (digests.into(), starts.into(), firsts.into());
    Some(RuleSets { digests, starts, firsts })
}

// Where a context subtable of format 1 or 2 counts its rule sets.
pub(crate) fn rule_set_count_at(data: &[u8], chain: bool) -> Option<usize> {
    match read_u16_be(data, 0)? {
        1 => Some(4),
        2 if chain => Some(10),
        2 => Some(6),
        _ => None,
    }
}

// Every second glyph a PairPos format 1's sets name. A set's records stop at the first one past the table, as the
// search's reads do. None once `work` runs out.
fn pair_seconds(data: &[u8], work: &mut usize) -> Option<GlyphBits> {
    let record_len = |at: usize| value_record_len(read_u16_be(data, at).unwrap_or(0));
    let stride = 2 + record_len(4) + record_len(6);
    let mut seconds = GlyphBits::new();
    for set in tables(data, 8) {
        let count = usize::from(read_u16_be(set, 0).unwrap_or(0));
        if !spend_work(work, 1 + count.min(set.len() / stride)) {
            return None;
        }
        (0..count).map_while(|i| read_u16_be(set, 2 + i * stride)).for_each(|g| seconds.insert(g.into()));
    }
    Some(seconds)
}

// Where a subtable's first coverage lies, for the kinds and formats shaping applies; any other subtable adds nothing
// to its lookup's digest.
fn first_coverage_at(kind: u16, data: &[u8], pair_pos: Option<u16>, context: u16, chain_context: u16) -> Option<usize> {
    let contextual = kind == context || kind == chain_context;
    match read_u16_be(data, 0)? {
        _ if !(1..=8).contains(&kind) => None,
        1 => Some(2),
        2 if contextual || kind == 1 || Some(kind) == pair_pos => Some(2),
        3 if kind == context => Some(6),
        3 if kind == chain_context => read_u16_be(data, 2).map(|n| 4 + usize::from(n) * 2 + 2),
        _ => None,
    }
}

// The class definitions (true) or coverages (false) a subtable reads past its first coverage: a context's classes or
// format 3 coverages, a chain's lists in turn, a reverse chain's context coverages, a mark attachment's second.
fn rule_tables(kind: u16, data: &[u8], kinds: &LookupKinds) -> (alloc::vec::Vec<usize>, bool) {
    let count = |at: usize| usize::from(read_u16_be(data, at).unwrap_or(0));
    let lists = |mut count_at: usize, n: usize| {
        let mut ats = alloc::vec::Vec::new();
        for _ in 0..n {
            let len = count(count_at);
            ats.extend((0..len).map(|i| count_at + 2 + i * 2));
            count_at += 2 + len * 2;
        }
        ats
    };
    let chain = kind == kinds.chain_context;
    let contextual = chain || kind == kinds.context;
    match read_u16_be(data, 0) {
        Some(1) if Some(kind) == kinds.reverse => (lists(4, 2), false),
        Some(1) if kinds.marks.contains(&kind) => (alloc::vec![4], false),
        Some(2) if contextual => ((0..if chain { 3 } else { 1 }).map(|i| 4 + i * 2).collect(), true),
        Some(3) if chain => (lists(2, 3), false),
        Some(3) if contextual => ((0..count(2)).map(|i| 6 + i * 2).collect(), false),
        _ => (alloc::vec::Vec::new(), false),
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ClassDef<'a> {
    data: &'a [u8],
    index: Option<&'a crate::daecore::daetype::format::index::SparseIndex>,
}

impl<'a> ClassDef<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        ClassDef { data, index: None }
    }

    pub(crate) fn at(data: &'a [u8], at: usize) -> Self {
        ClassDef::new(table_at(data, at))
    }

    pub(crate) fn with_index(
        data: &'a [u8],
        index: Option<&'a crate::daecore::daetype::format::index::SparseIndex>,
    ) -> Self {
        ClassDef { data, index }
    }

    pub(crate) fn indexed<'b>(
        &self,
        index: Option<&'b crate::daecore::daetype::format::index::SparseIndex>,
    ) -> ClassDef<'b>
    where
        'a: 'b,
    {
        ClassDef { data: self.data, index }
    }

    // Every glyph with a class, and its class, for an index.
    pub(crate) fn each_entry(&self, e: &mut Entries) -> Option<()> {
        crate::daecore::daetype::format::class_def::each_run(self.data, |start, end, class| {
            e.put_run(start, end, class, 0);
        })
    }

    // `class_of` inlined where an index answers, for the rules that read a class per glyph per rule.
    #[inline]
    pub(crate) fn indexed_class_of(&self, glyph: u32) -> u32 {
        match self.index {
            Some(index) => index.lookup(glyph).into(),
            None => self.class_of(glyph),
        }
    }

    // Kept out of line: inlined twice into `Face::classes`, it stops that inlining into the per-glyph
    // shaping loops, at up to 0.85% more instructions a shape.
    #[inline(never)]
    pub(crate) fn class_of(&self, glyph: u32) -> u32 {
        if let Some(index) = self.index {
            return index.lookup(glyph).into();
        }
        crate::daecore::daetype::format::class_def::class_of(self.data, glyph)
    }
}

pub(crate) struct Gdef<'a> {
    pub(crate) glyph_classes: Option<ClassDef<'a>>,
    pub(crate) mark_attach_classes: Option<ClassDef<'a>>,
    mark_glyph_sets: &'a [u8],
}

impl<'a> Gdef<'a> {
    // A major version other than 1 holds nothing, as none is defined. Mark glyph sets come in 1.2, and from 1.4 each
    // table at a 32-bit offset answers where it isn't null, else its 16-bit one.
    pub fn new(data: &'a [u8]) -> Option<Self> {
        if data.len() < 12 || read_u16_be(data, 0)? != 1 { return None; }
        let minor = read_u16_be(data, 2)?;
        let table = |at: usize, long_at: usize| {
            match (minor >= 4).then(|| read_u32_be(data, long_at)).flatten() {
                Some(off @ 1..) => Some(data.get(off as usize..).unwrap_or_default()),
                _ => (read_u16_be(data, at)? != 0).then(|| table_at(data, at)),
            }
        };
        Some(Gdef {
            glyph_classes: table(4, 18).map(ClassDef::new),
            mark_attach_classes: table(10, 30).map(ClassDef::new),
            mark_glyph_sets: if minor >= 2 { table(12, 34).unwrap_or_default() } else { &[] },
        })
    }

    pub(crate) fn has_glyph_classes(&self) -> bool {
        self.glyph_classes.is_some()
    }

    // Only format 1 holds sets.
    pub(crate) fn mark_glyph_set_count(&self) -> u16 {
        match read_u16_be(self.mark_glyph_sets, 0) {
            Some(1) => read_u16_be(self.mark_glyph_sets, 2).unwrap_or(0),
            _ => 0,
        }
    }

    // Mark glyph set `set_index`'s coverage, None past the count or at a null offset.
    pub(crate) fn mark_set(&self, set_index: u16) -> Option<Coverage<'a>> {
        if set_index >= self.mark_glyph_set_count() {
            return None;
        }
        match read_u32_be(self.mark_glyph_sets, 4 + usize::from(set_index) * 4)? {
            0 => None,
            off => Some(Coverage::new(self.mark_glyph_sets.get(off as usize..).unwrap_or_default())),
        }
    }

    pub(crate) fn is_mark_glyph(&self, glyph: u32, set_index: u16) -> bool {
        self.mark_set(set_index).is_some_and(|c| c.contains(glyph))
    }
}

// A glyph's class and mark attachment class from GDEF's: a class past 3, 65,535 or more included, is none, and a
// mark's attachment class keeps its low 8 bits.
pub(crate) fn gdef_class(class: u32, mark_attach: u32) -> (u8, u8) {
    use super::run::class;
    match class {
        1 => (class::BASE, 0),
        2 => (class::LIGATURE, 0),
        3 => (class::MARK, mark_attach as u8),
        _ => (0, 0),
    }
}

pub(crate) mod lookup_flags {
    pub(crate) const RIGHT_TO_LEFT: u16 = 0x0001;
    pub(crate) const IGNORE_BASE_GLYPHS: u16 = 0x0002;
    pub(crate) const IGNORE_LIGATURES: u16 = 0x0004;
    pub(crate) const IGNORE_MARKS: u16 = 0x0008;
    pub(crate) const IGNORE_FLAGS: u16 = IGNORE_BASE_GLYPHS | IGNORE_LIGATURES | IGNORE_MARKS;
    pub(crate) const USE_MARK_FILTERING_SET: u16 = 0x0010;
    pub(crate) const MARK_ATTACHMENT_TYPE_MASK: u16 = 0xFF00;
}

#[derive(Clone, Copy)]
pub(crate) struct Lookup<'a> {
    table: &'a [u8],
    at: usize,
    pub(crate) kind: u16,
    pub(crate) flags: u16,
    pub(crate) subtable_count: u16,
}

impl<'a> Lookup<'a> {
    // Subtable `i`, empty past the count.
    pub(crate) fn subtable(&self, i: u16) -> &'a [u8] {
        if i >= self.subtable_count {
            return &[];
        }
        table_at(&self.table[self.at..], 6 + usize::from(i) * 2)
    }

    // Where `data`, a slice running to the table's end as a resolved subtable does, starts: None for any other.
    pub(crate) fn start_of(&self, data: &[u8]) -> Option<usize> {
        let start = self.table.len().checked_sub(data.len())?;
        (data.is_empty() || core::ptr::eq(self.table[start..].as_ptr(), data.as_ptr())).then_some(start)
    }

    // The table from `start` on, empty past its end.
    pub(crate) fn tail(&self, start: usize) -> &'a [u8] {
        self.table.get(start..).unwrap_or_default()
    }

    pub(crate) fn mark_filtering_set(&self) -> Option<u16> {
        if self.flags & lookup_flags::USE_MARK_FILTERING_SET == 0 {
            return None;
        }
        read_u16_be(self.table, self.at + 6 + self.subtable_count as usize * 2)
    }
}

pub mod offered {
    use super::tag::Tag;
    use super::LayoutTable;
    use alloc::vec::Vec;

    pub fn script_tags(data: &[u8]) -> Vec<[u8; 4]> {
        let Some(t) = LayoutTable::parse(data) else { return Vec::new() };
        t.script_list.map_or_else(Vec::new, |list| t.record_tags(list))
    }

    pub fn language_tags(data: &[u8], script: &[u8; 4]) -> Vec<[u8; 4]> {
        let Some(t) = LayoutTable::parse(data) else { return Vec::new() };
        let script = t.find_script(Tag::new(script)).and_then(|i| t.script(i));
        script.map_or_else(Vec::new, |s| t.record_tags(s + 2))
    }

    // The features the language system shaping would take lists, a feature index past the list naming none.
    pub fn feature_tags(
        data: &[u8],
        script: Option<&[u8; 4]>,
        language: Option<&[u8; 4]>,
    ) -> Vec<[u8; 4]> {
        let Some(t) = LayoutTable::parse(data) else { return Vec::new() };
        let si = match script {
            Some(tag) => t.find_script(Tag::new(tag)),
            None => t.select_script(&[]).map(|(_, i, _)| i),
        };
        let Some(si) = si else { return Vec::new() };
        let li = t.select_langsys(si, &language.map(Tag::new).into_iter().collect::<Vec<_>>());
        let mut out: Vec<[u8; 4]> =
            t.required_feature(si, li).into_iter().chain(t.feature_indices(si, li))
                .filter_map(|f| t.feature_tag(f))
                .map(Tag::to_bytes)
                .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LayoutTable<'a> {
    data: &'a [u8],
    script_list: Option<usize>,
    feature_list: Option<usize>,
    lookup_list: Option<usize>,
    // 4 where the lookup list is 1.2's, which names its lookups at 32-bit offsets.
    lookup_width: usize,
    feature_variations: Option<usize>,
}

impl<'a> LayoutTable<'a> {
    // A table of version 0 holds no data, and one of a major version other than 1 no lists. A null list is none.
    // From 1.2 each list at a 32-bit offset answers where it isn't null, else its 16-bit one.
    pub(crate) fn parse(data: &'a [u8]) -> Option<Self> {
        let version = read_u32_be(data, 0).filter(|&v| v != 0)?;
        let read = version >> 16 == 1;
        let long = |at: usize| (read && version >= 0x0001_0002).then(|| read_u32_be(data, at)).flatten();
        let short = |at: usize| read_u16_be(data, at).filter(|_| read).map(u32::from);
        let list = |at: usize, long_at: usize| {
            let off = long(long_at).filter(|&off| off != 0).or_else(|| short(at))?;
            (off != 0).then_some(off as usize)
        };
        let feature_variations = (read && version >= 0x0001_0001).then(|| read_u32_be(data, 10)).flatten();
        Some(LayoutTable {
            data,
            script_list: list(4, 14),
            feature_list: list(6, 18),
            lookup_list: list(8, 22),
            lookup_width: if long(22).is_some_and(|off| off != 0) { 4 } else { 2 },
            feature_variations: feature_variations.filter(|&off| off != 0).map(|off| off as usize),
        })
    }

    pub(crate) fn bytes(&self) -> &'a [u8] {
        self.data
    }

    // Where its ScriptList, FeatureList and LookupList start, 0 for none, and the width of the lookups' offsets.
    pub(crate) fn lists(&self) -> ([usize; 3], usize) {
        let lists = [self.script_list, self.feature_list, self.lookup_list].map(|list| list.unwrap_or(0));
        (lists, self.lookup_width)
    }

    pub(crate) fn lookup_count(&self) -> u16 {
        self.lookup_list.and_then(|list| read_u16_be(self.data, list)).unwrap_or(0)
    }

    // None for a null lookup as well, which holds no subtables.
    #[inline]
    pub(crate) fn lookup(&self, index: u16) -> Option<Lookup<'a>> {
        if index >= self.lookup_count() {
            return None;
        }
        let list = self.lookup_list?;
        let at = list + 2 + usize::from(index) * self.lookup_width;
        let off = match self.lookup_width {
            2 => read_u16_be(self.data, at).map(u32::from),
            _ => read_u32_be(self.data, at),
        };
        let off = off.filter(|&off| off != 0)?;
        let at = list.checked_add(off as usize)?;
        let h = window::<6>(self.data, at)?;
        Some(Lookup {
            table: self.data,
            at,
            kind: u16::from_be_bytes([h[0], h[1]]),
            flags: u16::from_be_bytes([h[2], h[3]]),
            subtable_count: u16::from_be_bytes([h[4], h[5]]),
        })
    }

    // Record `index` of the (tag, offset) records counted at `at`: its tag, and the table its offset from `base`
    // names, None where the offset is null. None past the count.
    fn record(&self, at: usize, base: usize, index: u16) -> Option<(Tag, Option<usize>)> {
        if index >= read_u16_be(self.data, at)? {
            return None;
        }
        let record = at + 2 + usize::from(index) * 6;
        Some((Tag::new(window::<4>(self.data, record)?), resolve_offset16(self.data, record + 4, base)))
    }

    // The record tagged `tag` among those counted at `at`, by bisection, as the records sort by tag.
    fn find_record(&self, at: usize, tag: Tag) -> Option<u16> {
        let count = read_u16_be(self.data, at)?;
        let tag = tag.to_bytes();
        let found = midpoint_search(usize::from(count), |i| Some(tag.cmp(window::<4>(self.data, at + 2 + i * 6)?)))?;
        u16::try_from(found).ok()
    }

    fn record_tags(&self, at: usize) -> alloc::vec::Vec<[u8; 4]> {
        let count = read_u16_be(self.data, at).unwrap_or(0);
        (0..usize::from(count)).filter_map(|i| window::<4>(self.data, at + 2 + i * 6).copied()).collect()
    }

    fn find_script(&self, tag: Tag) -> Option<u16> {
        self.find_record(self.script_list?, tag)
    }

    pub(crate) fn select_script(&self, tags: &[Tag]) -> Option<(bool, u16, Tag)> {
        if let Some((i, &tag)) = tags.iter().find_map(|tag| Some((self.find_script(*tag)?, tag))) {
            return Some((true, i, tag));
        }
        [Tag::DEFAULT_SCRIPT, Tag::DEFAULT_LANGUAGE, Tag::new(b"latn")]
            .into_iter()
            .find_map(|tag| Some((false, self.find_script(tag)?, tag)))
    }

    pub(crate) fn script_count(&self) -> u16 {
        self.script_list.and_then(|list| read_u16_be(self.data, list)).unwrap_or(0)
    }

    // Script `index`'s table, None where its offset is null.
    pub(crate) fn script(&self, index: u16) -> Option<usize> {
        let list = self.script_list?;
        self.record(list, list, index)?.1
    }

    // The language system of the first of `tags` the script has, then of one tagged dflt; None for the default.
    pub(crate) fn select_langsys(&self, script: u16, tags: &[Tag]) -> Option<u16> {
        let s = self.script(script)?;
        tags.iter().chain([&Tag::DEFAULT_LANGUAGE]).find_map(|&tag| self.find_record(s + 2, tag))
    }

    // How many language systems script `script` tags.
    pub(crate) fn lang_sys_count(&self, script: u16) -> u16 {
        self.script(script).and_then(|s| read_u16_be(self.data, s + 2)).unwrap_or(0)
    }

    // Language system `lang` of script `script`, or its default for None: None where either offset is null.
    pub(crate) fn lang_sys(&self, script: u16, lang: Option<u16>) -> Option<usize> {
        let s = self.script(script)?;
        match lang {
            Some(i) => self.record(s + 2, s, i)?.1,
            None => resolve_offset16(self.data, s, s),
        }
    }

    pub(crate) fn required_feature(&self, script: u16, lang: Option<u16>) -> Option<u16> {
        let ls = self.lang_sys(script, lang)?;
        match read_u16_be(self.data, ls + 2)? {
            0xFFFF => None,
            i => Some(i),
        }
    }

    pub(crate) fn feature_indices(&self, script: u16, lang: Option<u16>)
        -> impl Iterator<Item = u16> + '_
    {
        let ls = self.lang_sys(script, lang);
        let count = ls.and_then(|ls| read_u16_be(self.data, ls + 4)).unwrap_or(0);
        (0..count).filter_map(move |i| {
            read_u16_be(self.data, ls.unwrap_or(0) + 6 + i as usize * 2)
        })
    }

    // None past the list's count, where the feature is the empty one.
    pub(crate) fn feature_tag(&self, feature: u16) -> Option<Tag> {
        let list = self.feature_list?;
        Some(self.record(list, list, feature)?.0)
    }

    pub(crate) fn find_feature(&self, script: u16, lang: Option<u16>, tag: Tag) -> Option<u16> {
        self.feature_indices(script, lang)
            .find(|&i| self.feature_tag(i) == Some(tag))
    }

    pub(crate) fn find_feature_globally(&self, tag: Tag) -> Option<u16> {
        let count = read_u16_be(self.data, self.feature_list?)?;
        (0..count).find(|&i| self.feature_tag(i) == Some(tag))
    }

    // Where a feature's table starts, None for the empty one.
    pub(crate) fn feature_at(&self, feature: u16) -> Option<usize> {
        self.feature_list.and_then(|list| self.record(list, list, feature)?.1)
    }

    // A feature's lookups, or those of the alternate a variation substitutes for it, of which None is the empty one.
    pub(crate) fn feature_lookups(&self, feature: u16, substitute: Option<Option<usize>>)
        -> impl Iterator<Item = u16> + '_
    {
        let at = match substitute {
            Some(alternate) => alternate,
            None => self.feature_at(feature),
        };
        let count = at.and_then(|at| read_u16_be(self.data, at + 2)).unwrap_or(0);
        (0..count).filter_map(move |i| read_u16_be(self.data, at.unwrap_or(0) + 4 + i as usize * 2))
    }

    // The record that holds at `coords`, in F2Dot14, its value conditions read from GDEF's `store`.
    pub(crate) fn find_variation_index(
        &self,
        coords: &[i32],
        store: Option<&crate::daecore::daetype::format::ivs::ItemVariationStore>,
    ) -> Option<u32> {
        FeatureVariations::at(self.data, self.feature_variations?).find(coords, store)
    }

    pub(crate) fn variation_substitute(&self, variation: u32, feature: u16) -> Option<Option<usize>> {
        FeatureVariations::at(self.data, self.feature_variations?).alternate(variation, feature)
    }

    // The conditions of the lookup variations that hold at `coords`, as `find_variation_index` reads them.
    pub(crate) fn lookup_conditions(
        &self,
        coords: &[i32],
        store: Option<&crate::daecore::daetype::format::ivs::ItemVariationStore>,
    ) -> alloc::vec::Vec<u32> {
        self.feature_variations
            .map_or_else(Default::default, |at| FeatureVariations::at(self.data, at).lookup_conditions(coords, store))
    }

    // The lookups a lookup variation record gives `feature` where `conditions` hold, the feature's own read as
    // `feature_lookups` reads them; None where no record names it, or past the reads `left`.
    pub(crate) fn varied_lookups(
        &self, feature: u16, conditions: &[u32], substitute: Option<Option<usize>>, left: &mut usize,
    ) -> Option<alloc::collections::BTreeSet<u16>> {
        FeatureVariations::at(self.data, self.feature_variations?)
            .varied_lookups(feature, conditions, self.feature_lookups(feature, substitute), left)
    }

    // The reads one walk of FeatureVariations may make, which a plan's lookup variations share.
    pub(crate) fn variation_reads(&self) -> usize {
        self.feature_variations.map_or(0, |at| FeatureVariations::at(self.data, at).budget())
    }

    // The lookups `features`' variations give them beside their own, wherever they hold; None past the reads the
    // walk may make.
    pub(crate) fn variant_lookups(
        &self, features: &alloc::collections::BTreeSet<u16>,
    ) -> Option<alloc::collections::BTreeSet<u16>> {
        let variations = |at| FeatureVariations::at(self.data, at).variant_lookups(features);
        self.feature_variations.map_or_else(|| Some(Default::default()), variations)
    }
}

// An Extension's subtable and the type it names, the subtable empty where its offset is null.
pub(crate) fn resolve_extension(
    subtable: &[u8],
    lookup_kind: u16,
    extension_kind: u16,
) -> Option<(u16, &[u8])> {
    if lookup_kind != extension_kind {
        return Some((lookup_kind, subtable));
    }
    if read_u16_be(subtable, 0)? != 1 {
        return None;
    }
    let real = read_u16_be(subtable, 2)?;
    if real == extension_kind {
        return None;
    }
    let off = read_u32_be(subtable, 4)? as usize;
    Some((real, if off == 0 { &[] } else { subtable.get(off..).unwrap_or_default() }))
}

pub(crate) fn value_record_len(format: u16) -> usize {
    (format & 0x00FF).count_ones() as usize * 2
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/ot/mod_index_tests.rs"]
mod index_tests;

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/ot/mod_nested_tests.rs"]
mod nested_tests;

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/ot/mod_gdef_tests.rs"]
mod gdef_tests;
