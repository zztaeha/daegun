use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::vec::Vec;
use super::generic::schema::Schema;
use super::generic::{Graph, Link, Parsed, Work};
use crate::daecore::daetype::decoder::{fitting_count, read_u16_be, read_u32_be, write_u16_be, write_u32_be};

const EXTENSION_RECORD_LEN: usize = 8;

pub(crate) type SubtableSubsetter<'a> =
    &'a dyn for<'b> Fn(u16, Option<&Schema>, &'b [u8], usize, &mut Work) -> Option<Parsed<'b>>;

// A subtable as read, shared by every lookup naming it.
type Read<'b> = Rc<Parsed<'b>>;

// Subtables read in this pass by where they start and their type, kept once named again, so one
// that many lookups name is parsed at most twice, and the work that costs is spent across the table.
struct Pass<'b> {
    work: Work,
    offsets: OffsetBudget,
    read: BTreeMap<(usize, u16), Option<Read<'b>>>,
}

type SchemaForType = fn(u16) -> Option<&'static Schema>;

fn resolve_effective_type(buf: &[u8], ext_type: u16, lookup_type: u16, sub_off: usize) -> Option<(u16, usize)> {
    if lookup_type != ext_type { return Some((lookup_type, sub_off)); }
    let ext_format = read_u16_be(buf, sub_off)?;
    if ext_format != 1 { return None; }
    let real_type = read_u16_be(buf, sub_off + 2)?;
    if real_type == ext_type { return None; }
    let real_off = read_u32_be(buf, sub_off + 4)? as usize;
    Some((real_type, sub_off.checked_add(real_off)?))
}

// Where each lookup a GSUB or GPOS names starts, in LookupList order, None for a null lookup, which holds nothing.
// None for a list cut short; a null list names no lookups.
pub(crate) fn lookups(buf: &[u8]) -> Option<impl ExactSizeIterator<Item = Option<usize>> + '_> {
    let ([_, _, list], width) = super::layout_lists(buf)?;
    let count = if list == 0 { 0 } else { usize::from(read_u16_be(buf, list)?) };
    let offsets = buf.get(list + 2..list + 2 + width * count)?.chunks_exact(width);
    Some(offsets.map(move |rel| {
        let rel = rel.iter().fold(0, |rel, &b| rel << 8 | usize::from(b));
        (rel != 0).then(|| list.saturating_add(rel).min(buf.len()))
    }))
}

// The subtable offsets a table's lookups may name between them, one per byte of the table. Lookups laid
// out apart name at most one per two bytes; lookups crafted to overlap could name 65,535 each.
pub(crate) struct OffsetBudget(usize);

impl OffsetBudget {
    pub(crate) fn for_table(len: usize) -> OffsetBudget {
        OffsetBudget(len)
    }
}

// A lookup's subtables, each named once, an Extension resolved or, malformed, dropped; a count past the
// table names the offsets it holds. None once the lookups walked name more offsets than `budget` allows.
pub(crate) fn subtables<'a>(
    buf: &'a [u8], lookup: usize, ext_type: u16, budget: &mut OffsetBudget,
) -> Option<impl Iterator<Item = (u16, usize)> + 'a> {
    let lookup_type = read_u16_be(buf, lookup).unwrap_or(0);
    let count = fitting_count(buf, lookup + 4, 2);
    budget.0 = budget.0.checked_sub(count)?;
    let mut seen = alloc::collections::BTreeSet::new();
    Some((0..count)
        .map_while(move |i| read_u16_be(buf, lookup + 6 + 2 * i))
        .filter_map(move |rel| resolve_effective_type(buf, ext_type, lookup_type, lookup + usize::from(rel)))
        .filter(move |&(_, off)| seen.insert(off)))
}

#[derive(Clone)]
struct RebuiltLookup<T> {
    real_type: u16,
    flag: u16,
    mark_filtering_set: Option<u16>,
    subtables: Vec<T>,
}

impl<T> RebuiltLookup<T> {
    fn hollow(lookup_type: u16) -> Self {
        let subtables = Vec::new();
        RebuiltLookup { real_type: lookup_type, flag: 0, mark_filtering_set: None, subtables }
    }

    // Its header's fields, for subtables of another form.
    fn with_none<U>(&self) -> RebuiltLookup<U> {
        let RebuiltLookup { real_type, flag, mark_filtering_set, .. } = *self;
        RebuiltLookup { real_type, flag, mark_filtering_set, subtables: Vec::new() }
    }

    fn is_hollow(&self) -> bool { self.subtables.is_empty() }

    fn trailing(&self) -> usize { if self.flag & 0x0010 != 0 { 2 } else { 0 } }

    fn header_len(&self) -> usize { 6 + self.subtables.len() * 2 + self.trailing() }

    fn write_header_into(&self, out: &mut [u8], at: usize, declared_type: u16) {
        write_u16_be(out, at, declared_type);
        write_u16_be(out, at + 2, self.flag);
        write_u16_be(out, at + 4, self.subtables.len() as u16);
        if self.trailing() > 0 {
            write_u16_be(out, at + 6 + self.subtables.len() * 2, self.mark_filtering_set.unwrap_or(0));
        }
    }
}

impl RebuiltLookup<Vec<u8>> {
    fn inline_len(&self) -> usize {
        if self.is_hollow() { return 6; }
        self.header_len() + self.subtables.iter().map(|t| t.len()).sum::<usize>()
    }

    fn promoted_header_len(&self) -> usize {
        if self.is_hollow() { return 6; }
        self.header_len() + self.subtables.len() * EXTENSION_RECORD_LEN
    }

    fn payload_len(&self) -> usize { self.subtables.iter().map(|t| t.len()).sum() }
}

fn rebuild_one_lookup<'b>(
    buf: &'b [u8],
    lookup_start: usize,
    ext_type: u16,
    mark_sets: &[Option<u16>],
    subset_subtable: SubtableSubsetter,
    schema_for_type: SchemaForType,
    pass: &mut Pass<'b>,
) -> Option<RebuiltLookup<Read<'b>>> {
    let Some(lookup_type) = read_u16_be(buf, lookup_start) else { return Some(RebuiltLookup::hollow(1)) };
    let Some(orig_flag) = read_u16_be(buf, lookup_start + 2) else { return Some(RebuiltLookup::hollow(lookup_type)) };
    let Some(sub_count) = read_u16_be(buf, lookup_start + 4) else { return Some(RebuiltLookup::hollow(lookup_type)) };

    // A mark filtering set takes its index in the subset's GDEF. One the font lacks filters by none, so its flag goes
    // rather than name a set the subset's GDEF doesn't hold; one left empty ignores every mark, as it did.
    let set = (orig_flag & 0x0010 != 0)
        .then(|| read_u16_be(buf, lookup_start + 6 + sub_count as usize * 2))
        .flatten()
        .and_then(|set| mark_sets.get(usize::from(set)));
    let mark_filtering_set = set.copied().flatten();
    let flag = match set {
        Some(Some(_)) => orig_flag,
        Some(None) => orig_flag & !0x0010 | 0x0008,
        None => orig_flag & !0x0010,
    };

    // A subtable named twice in one lookup applies where its first naming does, so later ones go.
    let mut effective_type: Option<u16> = None;
    let mut rebuilt_subtables: Vec<Read<'b>> = Vec::new();
    for (this_type, this_off) in subtables(buf, lookup_start, ext_type, &mut pass.offsets)? {
        if *effective_type.get_or_insert(this_type) != this_type { continue; }
        let schema = schema_for_type(this_type);
        let read = |pass: &mut Pass<'b>| subset_subtable(this_type, schema, buf, this_off, &mut pass.work).map(Rc::new);
        let rebuilt = match pass.read.get(&(this_off, this_type)) {
            Some(done) => done.clone(),
            None if pass.work.seen(this_off) => {
                let done = read(pass);
                pass.read.insert((this_off, this_type), done.clone());
                done
            }
            None => read(pass),
        };
        rebuilt_subtables.extend(rebuilt);
    }
    let (Some(real_type), false) = (effective_type, rebuilt_subtables.is_empty()) else {
        return Some(RebuiltLookup::hollow(lookup_type));
    };

    Some(RebuiltLookup { real_type, flag, mark_filtering_set, subtables: rebuilt_subtables })
}

// The table's lookups as read, with the work the reading spent.
fn read_lookups<'b>(
    buf: &'b [u8],
    ext_type: u16,
    mark_sets: &[Option<u16>],
    subset_subtable: SubtableSubsetter,
    schema_for_type: SchemaForType,
) -> Option<(Vec<RebuiltLookup<Read<'b>>>, Work)> {
    let starts = lookups(buf)?;
    let mut out: Vec<RebuiltLookup<Read<'b>>> = Vec::with_capacity(starts.len());
    let mut first_of = BTreeMap::new();
    let mut pass =
        Pass { work: Work::for_table(buf.len()), offsets: OffsetBudget::for_table(buf.len()), read: BTreeMap::new() };
    for (i, start) in starts.enumerate() {
        let lookup = match start.map(|start| (start, first_of.get(&start))) {
            None => RebuiltLookup::hollow(1),
            Some((_, Some(&j))) => RebuiltLookup::clone(&out[j]),
            Some((start, None)) => {
                first_of.insert(start, i);
                rebuild_one_lookup(buf, start, ext_type, mark_sets, subset_subtable, schema_for_type, &mut pass)?
            }
        };
        out.push(lookup);
    }
    Some((out, pass.work))
}

// Each lookup's number in the subset: those a feature names that hold a subtable, then every lookup their records
// name in turn, in the source's order, and u16::MAX for the rest, which go. With `held`, each keeps its own. A Feature,
// subtable or child many name is read once, each lookup index spending the subset budget; None past it.
fn numbering(buf: &[u8], lookups: &[RebuiltLookup<Read<'_>>], held: bool) -> Option<Vec<u16>> {
    if held {
        return (0..lookups.len()).map(|i| u16::try_from(i).ok()).collect();
    }
    let live = live(lookups);
    let ([_, feature_off, _], _) = super::layout_lists(buf)?;
    let features = if feature_off == 0 { 0 } else { usize::from(read_u16_be(buf, feature_off)?) };
    let (mut kept, mut walk) = (alloc::vec![false; lookups.len()], Vec::new());
    let (mut read, mut left) = (alloc::collections::BTreeSet::new(), super::super::subset_budget(buf.len()));
    for i in 0..features {
        let rel = usize::from(read_u16_be(buf, feature_off + 2 + i * 6 + 4)?);
        if rel == 0 || !read.insert(rel) {
            continue;
        }
        let feature = feature_off + rel;
        let count = usize::from(read_u16_be(buf, feature + 2)?);
        left = left.checked_sub(count)?;
        for k in 0..count {
            let lookup = usize::from(read_u16_be(buf, feature + 4 + 2 * k)?);
            if live.get(lookup) == Some(&true) && !kept[lookup] {
                kept[lookup] = true;
                walk.push(lookup);
            }
        }
    }
    let (mut named, mut seen) = (Vec::new(), alloc::collections::BTreeSet::new());
    while let Some(lookup) = walk.pop() {
        for read in &lookups[lookup].subtables {
            if let Parsed::Value(value) = &**read && seen.insert(core::ptr::from_ref(value)) {
                super::generic::named_lookups(value, &mut seen, &mut named);
            }
        }
        left = left.checked_sub(named.len())?;
        for nested in named.drain(..).map(usize::from) {
            if nested < kept.len() && !kept[nested] {
                kept[nested] = true;
                walk.push(nested);
            }
        }
    }
    let mut next = 0;
    Some(kept.iter().map(|&k| if k { next += 1; next - 1 } else { u16::MAX }).collect())
}

// The lookups `numbers` keeps, each subtable written once however many lookups name it, one writing nothing left out.
// None once the headers and what `held` counts of their subtables pass `budget`, which shared copies would.
fn written<'b, T: Clone>(
    lookups: &[RebuiltLookup<Read<'b>>], numbers: &[u16], budget: usize, held: impl Fn(&T) -> usize,
    mut write: impl FnMut(&Parsed<'b>) -> Option<T>,
) -> Option<Vec<RebuiltLookup<T>>> {
    let mut done: BTreeMap<*const Parsed<'b>, Option<T>> = BTreeMap::new();
    let mut spent = 0usize;
    let mut out = Vec::new();
    for (lookup, _) in lookups.iter().zip(numbers).filter(|&(_, &n)| n != u16::MAX) {
        let subtables = lookup.subtables.iter().filter_map(|read| {
            done.entry(Rc::as_ptr(read)).or_insert_with(|| write(read)).clone()
        });
        let subtables: Vec<T> = subtables.collect();
        let rebuilt = match subtables.is_empty() {
            true => RebuiltLookup::hollow(lookup.real_type),
            false => RebuiltLookup { subtables, ..lookup.with_none() },
        };
        spent = spent.saturating_add(rebuilt.header_len()).saturating_add(rebuilt.subtables.iter().map(&held).sum());
        if spent > budget {
            return None;
        }
        out.push(rebuilt);
    }
    Some(out)
}

// Whether each lookup holds a subtable still.
fn live<T>(lookups: &[RebuiltLookup<T>]) -> Vec<bool> {
    lookups.iter().map(|l| !l.is_hollow()).collect()
}

// The LookupList through `graph`, each subtable, lookup and part of them written once across the table: inline where
// `fits` takes it, else each subtable behind an Extension record. None where an offset can't reach.
fn shared_list(
    lookups: &[RebuiltLookup<usize>], mut graph: Graph, ext_type: u16, long: bool, fits: impl Fn(usize) -> bool,
) -> Option<Vec<u8>> {
    first_packed(lookups, &mut graph, |graph, promoted| {
        let list = list_root(lookups, graph, ext_type, long, promoted).and_then(|root| graph.pack(root))?;
        (long || promoted.contains(&true) || fits(list.len())).then_some(list)
    })
}

// What `pack` writes with no lookup behind Extension records, else, where that can't reach, with every one: the
// simplest plan that reaches whatever can be.
fn first_packed(
    lookups: &[RebuiltLookup<usize>], graph: &mut Graph, mut pack: impl FnMut(&mut Graph, &[bool]) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    pack(graph, &alloc::vec![false; lookups.len()]).or_else(|| pack(graph, &alloc::vec![true; lookups.len()]))
}

fn list_root(
    lookups: &[RebuiltLookup<usize>], graph: &mut Graph, ext_type: u16, long: bool, promoted: &[bool],
) -> Option<usize> {
    let width = if long { 4 } else { 2 };
    let mut list = alloc::vec![0u8; 2 + lookups.len() * width];
    write_u16_be(&mut list, 0, u16::try_from(lookups.len()).ok()?);
    let links = lookups.iter().zip(promoted).enumerate().map(|(i, (lookup, &promoted))| {
        Link { at: 2 + i * width, width, to: lookup_object(lookup, graph, ext_type, promoted) }
    });
    let links = links.collect();
    Some(graph.add(list, links))
}

// A lookup's header naming its subtables, or with `promoted`, an Extension record each that names it.
fn lookup_object(lookup: &RebuiltLookup<usize>, graph: &mut Graph, ext_type: u16, promoted: bool) -> usize {
    let mut header = alloc::vec![0u8; if lookup.is_hollow() { 6 } else { lookup.header_len() }];
    if lookup.is_hollow() {
        write_u16_be(&mut header, 0, lookup.real_type);
        return graph.add(header, Vec::new());
    }
    lookup.write_header_into(&mut header, 0, if promoted { ext_type } else { lookup.real_type });
    let mut links = Vec::with_capacity(lookup.subtables.len());
    for (i, &subtable) in lookup.subtables.iter().enumerate() {
        let mut to = subtable;
        if promoted {
            let mut record = alloc::vec![0u8; EXTENSION_RECORD_LEN];
            write_u16_be(&mut record, 0, 1);
            write_u16_be(&mut record, 2, lookup.real_type);
            to = graph.add(record, alloc::vec![Link { at: 4, width: 4, to }]);
        }
        links.push(Link { at: 6 + 2 * i, width: 2, to });
    }
    graph.add(header, links)
}

fn assemble_inline(lookups: &[RebuiltLookup<Vec<u8>>]) -> Option<Vec<u8>> {
    let header_len = 2 + lookups.len() * 2;
    let mut out = vec![0u8; header_len];
    write_u16_be(&mut out, 0, u16::try_from(lookups.len()).ok()?);

    let mut pos = header_len;
    for (i, lk) in lookups.iter().enumerate() {
        write_u16_be(&mut out, 2 + i * 2, u16::try_from(pos).ok()?);
        pos = pos.checked_add(lk.inline_len())?;
    }

    for lk in lookups {
        let at = out.len();
        out.resize(at + lk.header_len(), 0);
        if lk.is_hollow() {
            write_u16_be(&mut out, at, lk.real_type);
            continue;
        }
        lk.write_header_into(&mut out, at, lk.real_type);
        let mut sub_pos = lk.header_len();
        for (i, st) in lk.subtables.iter().enumerate() {
            write_u16_be(&mut out, at + 6 + i * 2, u16::try_from(sub_pos).ok()?);
            sub_pos = sub_pos.checked_add(st.len())?;
        }
        for st in &lk.subtables { out.extend_from_slice(st); }
    }
    Some(out)
}

fn assemble_promoted(lookups: &[RebuiltLookup<Vec<u8>>], ext_type: u16) -> Option<Vec<u8>> {
    let list_header = 2 + lookups.len() * 2;

    let mut lookup_at = Vec::with_capacity(lookups.len());
    let mut pos = list_header;
    for lk in lookups {
        lookup_at.push(pos);
        pos = pos.checked_add(lk.promoted_header_len())?;
    }
    let payload_base = pos;

    let mut out = vec![0u8; payload_base];
    write_u16_be(&mut out, 0, u16::try_from(lookups.len()).ok()?);
    for (i, &at) in lookup_at.iter().enumerate() {
        write_u16_be(&mut out, 2 + i * 2, u16::try_from(at).ok()?);
    }

    let mut payload_pos = payload_base;
    for (lk, &at) in lookups.iter().zip(&lookup_at) {
        if lk.is_hollow() {
            write_u16_be(&mut out, at, lk.real_type);
            continue;
        }
        lk.write_header_into(&mut out, at, ext_type);
        let records_at = at + lk.header_len();
        for (i, st) in lk.subtables.iter().enumerate() {
            let record = records_at + i * EXTENSION_RECORD_LEN;
            write_u16_be(&mut out, at + 6 + i * 2, u16::try_from(record - at).ok()?);
            write_u16_be(&mut out, record, 1);
            write_u16_be(&mut out, record + 2, lk.real_type);
            write_u32_be(&mut out, record + 4, u32::try_from(payload_pos - record).ok()?);
            payload_pos = payload_pos.checked_add(st.len())?;
        }
    }

    out.reserve(lookups.iter().map(RebuiltLookup::payload_len).sum());
    for lk in lookups {
        for st in &lk.subtables { out.extend_from_slice(st); }
    }
    Some(out)
}

// From 1.2 the LookupList names lookups at 32-bit offsets, so only a lookup whose own subtables lie past 16-bit reach
// becomes an Extension. Its offsets count from itself, so it's laid out as a list of one, less that list's 4-byte head.
fn assemble_long(lookups: &[RebuiltLookup<Vec<u8>>], ext_type: u16) -> Option<Vec<u8>> {
    let mut out = vec![0u8; 2 + lookups.len() * 4];
    write_u16_be(&mut out, 0, u16::try_from(lookups.len()).ok()?);
    for (i, lk) in lookups.iter().enumerate() {
        let at = u32::try_from(out.len()).ok()?;
        write_u32_be(&mut out, 2 + i * 4, at);
        let one = core::slice::from_ref(lk);
        out.extend_from_slice(assemble_inline(one).or_else(|| assemble_promoted(one, ext_type))?.get(4..)?);
    }
    Some(out)
}

// The subset table, one graph with its header, ScriptList and FeatureList. Without FeatureVariations or a JSTF (`held`)
// naming them by index, features and lookups renumber afresh. Past the graph, `compose` writes the lists apart.
pub(crate) fn subset_lookup_table(
    buf: &[u8],
    ext_type: u16,
    mark_sets: &[Option<u16>],
    subset_subtable: SubtableSubsetter,
    schema_for_type: SchemaForType,
    held: bool,
) -> Option<Vec<u8>> {
    let long = super::layout_header_len(buf)? == 26;
    let variations = super::feature_variations(buf);
    let lists_end = match variations {
        Some(_) => super::layout_live_prefix_len(buf).unwrap_or(buf.len()),
        None => buf.len(),
    };
    let fits = |len: usize| lists_end + len <= 0xFFFF;
    let budget = crate::daecore::daetype::subsetter::subset_budget(buf.len());

    let (read, mut work) = read_lookups(buf, ext_type, mark_sets, subset_subtable, schema_for_type)?;
    let numbers = numbering(buf, &read, held || variations != Some(None))?;
    let renumbered = numbers.iter().enumerate().any(|(i, &n)| usize::from(n) != i);
    work.lookups = renumbered.then(|| numbers.clone());
    let lists = (&live(&read)[..], &numbers[..], renumbered);

    let mut graph = Graph::within(budget);
    let shared = written(&read, &numbers, budget, |_| 0, |parsed| match parsed {
        Parsed::Value(value) => super::generic::emit_value(value, &mut graph, &mut work),
        Parsed::Bytes(bytes) => Some(graph.add(bytes.clone(), Vec::new())),
    });
    if let Some(lookups) = shared.filter(|_| graph.held() <= budget) {
        if let Some(fv) = &variations {
            let whole = first_packed(&lookups, &mut graph, |graph, promoted| {
                whole_table(buf, &lookups, lists, fv.clone(), graph, ext_type, promoted)
            });
            if whole.is_some() {
                return whole;
            }
        }
        if let Some(list) = shared_list(&lookups, graph, ext_type, long, fits) {
            return compose(buf, list, lists, variations, lists_end);
        }
    }
    let lookups = written(&read, &numbers, budget, Vec::len, |parsed| match parsed {
        Parsed::Value(value) => super::generic::build_value(value, &mut work),
        Parsed::Bytes(bytes) => Some(bytes.clone()),
    })?;
    let list = match long {
        true => assemble_long(&lookups, ext_type)?,
        false => match assemble_inline(&lookups).filter(|list| fits(list.len())) {
            Some(inline) => inline,
            None => assemble_promoted(&lookups, ext_type)?,
        },
    };
    compose(buf, list, lists, variations, lists_end)
}

// Each lookup as read, whether it holds a subtable, its number in the subset, and whether any number moved.
type Lists<'a> = (&'a [bool], &'a [u16], bool);

// The whole table through `graph`: header, lists and lookups. 1.1 without FeatureVariations is written as 1.0, since
// that offset is all 1.1 adds.
fn whole_table(
    buf: &[u8], lookups: &[RebuiltLookup<usize>], lists: Lists, variations: Option<core::ops::Range<usize>>,
    graph: &mut Graph, ext_type: u16, promoted: &[bool],
) -> Option<Vec<u8>> {
    let long = super::layout_header_len(buf)? == 26;
    let (features, scripts, _) = read_lists(buf, lists, variations.is_none())?;
    let roots = [
        script_list(&scripts, graph)?,
        feature_list(&features, graph)?,
        list_root(lookups, graph, ext_type, long, promoted)?,
    ];
    let mut header = alloc::vec![0u8; if long { 26 } else if variations.is_some() { 14 } else { 10 }];
    write_u16_be(&mut header, 0, 1);
    if header.len() > 10 {
        header[2..4].copy_from_slice(buf.get(2..4)?);
    }
    let (at, width) = if long { ([14, 18, 22], 4) } else { ([4, 6, 8], 2) };
    let mut links: Vec<Link> = at.into_iter().zip(roots).map(|(at, to)| Link { at, width, to }).collect();
    if let Some(fv) = variations {
        links.push(Link { at: 10, width: 4, to: graph.add(buf.get(fv)?.to_vec(), Vec::new()) });
    }
    let root = graph.add(header, links);
    graph.pack(root)
}

// The lists ahead of `list`, the LookupList: as the source has them, or rebuilt where a feature goes or a lookup's
// number moves, then FeatureVariations moved whole if the lists' bytes don't hold it.
fn compose(
    buf: &[u8], list: Vec<u8>, lists: Lists, variations: Option<Option<core::ops::Range<usize>>>, lists_end: usize,
) -> Option<Vec<u8>> {
    let long = super::layout_header_len(buf)? == 26;
    if !long && lists_end > 0xFFFF { return None; }
    let rebuilt = match variations {
        Some(None) => rebuild_prefix(buf, lists).filter(|p| p.len() <= lists_end || lists.2),
        _ => None,
    };
    let (mut out, moved) = match (rebuilt, variations.flatten()) {
        (Some(prefix), _) => (prefix, None),
        // The source's lists name lookups by their old numbers.
        (None, _) if lists.2 => return None,
        (None, Some(fv)) if fv.end <= lists_end => (buf.get(..lists_end)?.to_vec(), None),
        (None, fv) => (buf.get(..lists_end)?.to_vec(), fv),
    };
    if long {
        // A 1.2 table's lists are written through their 32-bit offsets alone.
        let ([script, feature, _], _) = super::layout_lists(&out)?;
        let lists = [script, feature, out.len()].map(u32::try_from);
        for (at, list) in [14, 18, 22].into_iter().zip(lists) {
            write_u32_be(&mut out, at, list.ok()?);
        }
        out[4..10].fill(0);
    } else {
        let lookup_at = u16::try_from(out.len()).ok()?;
        write_u16_be(&mut out, 8, lookup_at);
    }
    out.extend_from_slice(&list);
    if let Some(fv) = moved {
        let at = u32::try_from(out.len()).ok()?;
        write_u32_be(&mut out, 10, at);
        out.extend_from_slice(buf.get(fv)?);
    }
    Some(out)
}

#[derive(Clone)]
struct LangSys {
    tag: u32,
    features: Vec<u16>,
    required: Option<u16>,
}

#[derive(Clone)]
struct Script {
    tag: u32,
    default_features: Option<Vec<u16>>,
    default_required: Option<u16>,
    langs: Vec<LangSys>,
}

// A feature's tag, lookup indices and parameters' bytes.
type Feature = (u32, Vec<u16>, Option<Vec<u8>>);

// Features the shaper asks after by presence alone, kept with no lookup: a GPOS kern feature turns
// the legacy kern table off, and Indic base finding reads pref's mask, which stays without lookups.
const STEERING: [[u8; 4]; 2] = [*b"kern", *b"pref"];

// Whether a feature with no live lookup stays: one that steers shaping, or `size` with its parameters, which are all
// it holds; another's parameters, such as a stylistic set's name, go with it.
fn stays_empty(tag: u32, params: &Option<Vec<u8>>) -> bool {
    STEERING.contains(&tag.to_be_bytes()) || (&tag.to_be_bytes() == b"size" && params.is_some())
}

// The features and scripts, each lookup by its number in the subset. With `drop`, features with no live lookup go, but
// those that stay empty, one the same as an earlier one merges into it, and the answer says if any went. A Script or
// LangSys many records name is read once, and None once the bytes read and copied pass the subset budget.
fn read_lists(buf: &[u8], (live, numbers, _): Lists, drop: bool) -> Option<(Vec<Feature>, Vec<Script>, bool)> {
    let ([script_off, feature_off, _], _) = super::layout_lists(buf)?;
    let count = |off: usize| if off == 0 { Some(0) } else { read_u16_be(buf, off).map(usize::from) };
    let left = core::cell::Cell::new(crate::daecore::daetype::subsetter::subset_budget(buf.len()));
    let spend = |n: usize| left.get().checked_sub(n).map(|rest| left.set(rest));

    let feature_count = count(feature_off)?;
    let mut kept: Vec<Feature> = Vec::new();
    let mut alike: BTreeMap<Feature, u16> = BTreeMap::new();
    let mut remap: Vec<Option<u16>> = alloc::vec![None; feature_count];
    for (i, slot) in remap.iter_mut().enumerate() {
        let rec = feature_off + 2 + i * 6;
        let tag = read_u32_be(buf, rec)?;
        // A null feature, script or language system is an empty one.
        let (f, params_rel, n_idx) = match read_u16_be(buf, rec + 4)? {
            0 => (feature_off, 0, 0),
            rel => {
                let f = feature_off + usize::from(rel);
                (f, usize::from(read_u16_be(buf, f)?), usize::from(read_u16_be(buf, f + 2)?))
            }
        };

        spend(4 + 2 * n_idx)?;
        let mut indices: Vec<u16> = Vec::new();
        for k in 0..n_idx {
            let idx = read_u16_be(buf, f + 4 + k * 2)?;
            if !drop || live.get(idx as usize).copied().unwrap_or(false) {
                indices.push(numbers.get(usize::from(idx)).copied().unwrap_or(idx));
            }
        }
        let params = if params_rel == 0 {
            None
        } else {
            let at = f + params_rel;
            let end = super::feature_params_end(buf, rec, at)?;
            spend(end.saturating_sub(at))?;
            Some(buf.get(at..end)?.to_vec())
        };
        if drop && indices.is_empty() && !stays_empty(tag, &params) { continue; }
        let feature = (tag, indices, params);
        *slot = Some(match alike.get(&feature) {
            Some(&same) => same,
            None => {
                let new = u16::try_from(kept.len()).ok()?;
                if drop { alike.insert(feature.clone(), new); }
                kept.push(feature);
                new
            }
        });
    }
    let dropped = kept.len() != feature_count;

    let script_count = count(script_off)?;
    let mut scripts: Vec<Script> = Vec::with_capacity(script_count);
    let mut seen = alloc::vec![false; kept.len()];
    let mut systems: BTreeMap<usize, (Vec<u16>, Option<u16>)> = BTreeMap::new();
    let mut read_langsys = |at: usize| -> Option<(Vec<u16>, Option<u16>)> {
        if let Some(read) = systems.get(&at) {
            spend(6 + 2 * read.0.len())?;
            return Some(read.clone());
        }
        let required = read_u16_be(buf, at + 2)?;
        let n = read_u16_be(buf, at + 4)? as usize;
        spend(6 + 2 * n)?;
        let mut out = Vec::new();
        for k in 0..n {
            let old = read_u16_be(buf, at + 6 + k * 2)? as usize;
            if let Some(&Some(new)) = remap.get(old) && !core::mem::replace(&mut seen[usize::from(new)], true) {
                out.push(new);
            }
        }
        out.iter().for_each(|&new| seen[usize::from(new)] = false);
        let new_required = match required {
            0xFFFF => None,
            r => remap.get(r as usize).copied().flatten(),
        };
        systems.insert(at, (out.clone(), new_required));
        Some((out, new_required))
    };
    // Each Script's first record, by where the Script starts.
    let mut first: BTreeMap<usize, usize> = BTreeMap::new();
    for i in 0..script_count {
        let rec = script_off + 2 + i * 6;
        let tag = read_u32_be(buf, rec)?;
        let s_rel = read_u16_be(buf, rec + 4)? as usize;
        if s_rel == 0 {
            scripts.push(Script { tag, default_features: None, default_required: None, langs: Vec::new() });
            continue;
        }
        let s = script_off + s_rel;
        if let Some(&j) = first.get(&s) {
            let read = scripts.get(j)?;
            let lists = read.default_features.iter().chain(read.langs.iter().map(|l| &l.features));
            spend(4 + 6 * read.langs.len() + lists.map(|f| 6 + 2 * f.len()).sum::<usize>())?;
            let copy = Script { tag, ..read.clone() };
            scripts.push(copy);
            continue;
        }
        first.insert(s, scripts.len());
        let default_rel = read_u16_be(buf, s)? as usize;
        let lang_count = read_u16_be(buf, s + 2)? as usize;
        spend(4 + 6 * lang_count)?;

        let (default_features, default_required) = if default_rel == 0 {
            (None, None)
        } else {
            let (f, r) = read_langsys(s + default_rel)?;
            (Some(f), r)
        };
        let mut langs = Vec::new();
        for j in 0..lang_count {
            let lrec = s + 4 + j * 6;
            let ltag = read_u32_be(buf, lrec)?;
            let (f, r) = match read_u16_be(buf, lrec + 4)? {
                0 => (Vec::new(), None),
                rel => read_langsys(s + usize::from(rel))?,
            };
            // A system the same as its script's default goes, as the shaper reads the default for a language it
            // lacks. No FeatureVariations names one, so it goes with them too.
            if default_features.as_ref() == Some(&f) && default_required == r {
                continue;
            }
            langs.push(LangSys { tag: ltag, features: f, required: r });
        }
        scripts.push(Script { tag, default_features, default_required, langs });
    }
    Some((kept, scripts, dropped))
}

// Each script's records naming its language systems, each an object of its own, so systems alike are one.
fn script_list(scripts: &[Script], graph: &mut Graph) -> Option<usize> {
    let mut list = alloc::vec![0u8; 2 + scripts.len() * 6];
    write_u16_be(&mut list, 0, u16::try_from(scripts.len()).ok()?);
    let mut links = Vec::with_capacity(scripts.len());
    for (i, Script { tag, default_features, default_required, langs }) in scripts.iter().enumerate() {
        list[2 + i * 6..6 + i * 6].copy_from_slice(&tag.to_be_bytes());
        let mut table = alloc::vec![0u8; 4 + langs.len() * 6];
        write_u16_be(&mut table, 2, u16::try_from(langs.len()).ok()?);
        let mut own = Vec::with_capacity(langs.len() + 1);
        if let Some(features) = default_features {
            own.push(Link { at: 0, width: 2, to: lang_sys(features, *default_required, graph)? });
        }
        for (j, LangSys { tag, features, required }) in langs.iter().enumerate() {
            table[4 + j * 6..8 + j * 6].copy_from_slice(&tag.to_be_bytes());
            own.push(Link { at: 8 + j * 6, width: 2, to: lang_sys(features, *required, graph)? });
        }
        links.push(Link { at: 6 + i * 6, width: 2, to: graph.add(table, own) });
    }
    Some(graph.add(list, links))
}

fn lang_sys(features: &[u16], required: Option<u16>, graph: &mut Graph) -> Option<usize> {
    let head = [0, required.unwrap_or(0xFFFF), u16::try_from(features.len()).ok()?];
    let mut bytes = Vec::with_capacity(6 + 2 * features.len());
    bytes.extend(head.iter().chain(features).flat_map(|v| v.to_be_bytes()));
    Some(graph.add(bytes, Vec::new()))
}

// Each feature's record naming its table, and each table its parameters, as objects of their own.
fn feature_list(features: &[Feature], graph: &mut Graph) -> Option<usize> {
    let mut list = alloc::vec![0u8; 2 + features.len() * 6];
    write_u16_be(&mut list, 0, u16::try_from(features.len()).ok()?);
    let mut links = Vec::with_capacity(features.len());
    for (i, (tag, indices, params)) in features.iter().enumerate() {
        list[2 + i * 6..6 + i * 6].copy_from_slice(&tag.to_be_bytes());
        let head = [0, u16::try_from(indices.len()).ok()?];
        let mut table = Vec::with_capacity(4 + 2 * indices.len());
        table.extend(head.iter().chain(indices).flat_map(|v| v.to_be_bytes()));
        let own = match params {
            Some(params) => alloc::vec![Link { at: 0, width: 2, to: graph.add(params.clone(), Vec::new()) }],
            None => Vec::new(),
        };
        links.push(Link { at: 6 + i * 6, width: 2, to: graph.add(table, own) });
    }
    Some(graph.add(list, links))
}

// A header, ScriptList and FeatureList for a table with no FeatureVariations, as `read_lists` keeps them; None when
// every feature and lookup keeps its number. 1.1 is written as 1.0, having no FeatureVariations to point to.
fn rebuild_prefix(buf: &[u8], lists: Lists) -> Option<Vec<u8>> {
    let long = super::layout_header_len(buf)? == 26;
    let (kept, scripts, dropped) = read_lists(buf, lists, true)?;
    if !dropped && !lists.2 { return None; }
    let mut script_list: Vec<u8> = Vec::new();
    script_list.extend_from_slice(&u16::try_from(scripts.len()).ok()?.to_be_bytes());
    let mut script_bodies: Vec<u8> = Vec::new();
    let script_records_len = 2 + scripts.len() * 6;
    for Script { tag, default_features, default_required, langs } in &scripts {
        let body_at = script_records_len + script_bodies.len();
        script_list.extend_from_slice(&tag.to_be_bytes());
        script_list.extend_from_slice(&u16::try_from(body_at).ok()?.to_be_bytes());

        let header_len = 4 + langs.len() * 6;
        let mut body: Vec<u8> = Vec::new();
        let mut tail: Vec<u8> = Vec::new();
        let push_langsys = |tail: &mut Vec<u8>, features: &[u16], required: Option<u16>| -> usize {
            let at = header_len + tail.len();
            tail.extend_from_slice(&0u16.to_be_bytes());
            tail.extend_from_slice(&required.unwrap_or(0xFFFF).to_be_bytes());
            tail.extend_from_slice(&(features.len() as u16).to_be_bytes());
            for f in features { tail.extend_from_slice(&f.to_be_bytes()); }
            at
        };
        let default_at = default_features.as_ref()
            .map(|f| push_langsys(&mut tail, f, *default_required));
        body.extend_from_slice(&u16::try_from(default_at.unwrap_or(0)).ok()?.to_be_bytes());
        body.extend_from_slice(&u16::try_from(langs.len()).ok()?.to_be_bytes());
        for LangSys { tag: ltag, features, required } in langs {
            let at = push_langsys(&mut tail, features, *required);
            body.extend_from_slice(&ltag.to_be_bytes());
            body.extend_from_slice(&u16::try_from(at).ok()?.to_be_bytes());
        }
        body.extend_from_slice(&tail);
        script_bodies.extend_from_slice(&body);
    }
    script_list.extend_from_slice(&script_bodies);

    let mut feature_list: Vec<u8> = Vec::new();
    feature_list.extend_from_slice(&u16::try_from(kept.len()).ok()?.to_be_bytes());
    let feature_records_len = 2 + kept.len() * 6;
    let mut feature_bodies: Vec<u8> = Vec::new();
    for (tag, indices, params) in &kept {
        let body_at = feature_records_len + feature_bodies.len();
        feature_list.extend_from_slice(&tag.to_be_bytes());
        feature_list.extend_from_slice(&u16::try_from(body_at).ok()?.to_be_bytes());

        let own_len = 4 + indices.len() * 2;
        feature_bodies.extend_from_slice(&u16::try_from(params.as_ref().map_or(0, |_| own_len)).ok()?.to_be_bytes());
        feature_bodies.extend_from_slice(&u16::try_from(indices.len()).ok()?.to_be_bytes());
        for idx in indices { feature_bodies.extend_from_slice(&idx.to_be_bytes()); }
        if let Some(p) = params { feature_bodies.extend_from_slice(p); }
    }
    feature_list.extend_from_slice(&feature_bodies);

    let mut out: Vec<u8> = alloc::vec![0u8; if long { 26 } else { 10 }];
    write_u16_be(&mut out, 0, 1);
    let script_at = out.len();
    let feature_at = script_at + script_list.len();
    if long {
        out[2..4].copy_from_slice(buf.get(2..4)?);
        write_u32_be(&mut out, 14, u32::try_from(script_at).ok()?);
        write_u32_be(&mut out, 18, u32::try_from(feature_at).ok()?);
    } else {
        write_u16_be(&mut out, 4, u16::try_from(script_at).ok()?);
        write_u16_be(&mut out, 6, u16::try_from(feature_at).ok()?);
    }
    out.extend_from_slice(&script_list);
    out.extend_from_slice(&feature_list);
    Some(out)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/subsetter/otl/lookup_list.rs"]
mod tests;
