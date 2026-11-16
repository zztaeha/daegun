// Context lookups (GSUB 5 and 6, GPOS 7 and 8): a rule matches glyphs, classes or coverages about the current glyph,
// then applies lookups at its input. Formats 1 to 3, and 24-bit forms with 3-byte glyphs and 32-bit first offsets.
use super::{Engine, Peek, Side, SEQUENCE_LIMIT};
use crate::daecore::daekit::base::sync::Shared;
use crate::daecore::daeshaper::ot::{long, nth_table_width, spend_work, table_at_width, tables_of, tables_width};
use crate::daecore::daeshaper::ot::{ClassDef, Coverage, COVERAGE_ABSENT};
use crate::daecore::daetype::decoder::{read_u16_be, read_uint};
use crate::daecore::daetype::format::index::SparseIndex;

// `len` values of `W` bytes: glyphs, or 16-bit classes.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ValueList<'a, const W: usize> {
    bytes: &'a [u8],
    pub(crate) len: u16,
}

impl<'a, const W: usize> ValueList<'a, W> {
    pub(crate) fn at(data: &'a [u8], at: usize, len: u16) -> Self {
        ValueList { bytes: data.get(at..).unwrap_or_default(), len }
    }

    pub(crate) fn get(&self, i: usize) -> Option<u32> {
        (i < usize::from(self.len)).then(|| read_uint(self.bytes, i * W, W)).flatten()
    }
}

// A rule's sequence lookup records: an input glyph and the lookup to apply there, 4 bytes each.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Records<'a> {
    bytes: &'a [u8],
    count: u16,
}

impl Records<'_> {
    fn get(&self, i: u16) -> Option<(u16, u16)> {
        let at = usize::from(i) * 4;
        let field = |k: usize| read_u16_be(self.bytes, at + k);
        if i < self.count { Some((field(0)?, field(2)?)) } else { None }
    }

    fn lookups(&self) -> impl Iterator<Item = u16> + '_ {
        (0..self.count).map_while(|i| self.get(i)).map(|(_, lookup)| lookup)
    }
}

// Reads a rule's fields in order.
struct Fields<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Fields<'a> {
    fn word(&mut self) -> u16 {
        self.at += 2;
        read_u16_be(self.bytes, self.at - 2).unwrap_or(0)
    }

    fn values<const W: usize>(&mut self, len: u16) -> ValueList<'a, W> {
        let list = ValueList::at(self.bytes, self.at, len);
        self.at += usize::from(len) * W;
        list
    }

    fn records(&mut self, count: u16) -> Records<'a> {
        Records { bytes: self.bytes.get(self.at..).unwrap_or_default(), count }
    }
}

// A rule as a match reads it: backtrack values nearest first, input length and values past its first glyph, lookahead
// values, records. A plain rule has no backtrack or lookahead; a null rule has no input.
#[derive(Debug)]
pub(crate) struct Rule<'a, const W: usize> {
    pub(crate) back: ValueList<'a, W>,
    pub(crate) count: u16,
    pub(crate) input: ValueList<'a, W>,
    pub(crate) ahead: ValueList<'a, W>,
    pub(crate) records: Records<'a>,
}

impl<'a, const W: usize> Rule<'a, W> {
    pub(crate) fn read(bytes: &'a [u8], chained: bool) -> Self {
        let mut f = Fields { bytes, at: 0 };
        let none = ValueList { bytes: &[], len: 0 };
        if !chained {
            let (count, records) = (f.word(), f.word());
            let input = f.values(count.saturating_sub(1));
            return Rule { back: none, count, input, ahead: none, records: f.records(records) };
        }
        let back_len = f.word();
        let back = f.values(back_len);
        let count = f.word();
        let input = f.values(count.saturating_sub(1));
        let ahead_len = f.word();
        let ahead = f.values(ahead_len);
        let records = f.word();
        Rule { back, count, input, ahead, records: f.records(records) }
    }
}

// The value a rule compares with the glyph its match reads first past the current one, and which part reads it: the
// input's second glyph, or for an input of one, the first glyph after it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FirstRead {
    Input(u32),
    Ahead(u32),
}

impl FirstRead {
    // One word for the plan to keep: a 16-bit value, bit 16 for the lookahead's, bit 17 alone for none.
    pub(crate) fn pack(first: Option<FirstRead>) -> u32 {
        match first {
            Some(FirstRead::Input(value)) => value & 0xFFFF,
            Some(FirstRead::Ahead(value)) => 1 << 16 | value & 0xFFFF,
            None => 1 << 17,
        }
    }

    #[inline(always)]
    fn unpack(word: u32) -> Option<FirstRead> {
        match word >> 16 {
            0 => Some(FirstRead::Input(word)),
            1 => Some(FirstRead::Ahead(word & 0xFFFF)),
            _ => None,
        }
    }
}

// None where the match decides without reading either (an input of one and nothing after, or past the sequence
// limit) or the value doesn't read.
pub(crate) fn first_read<const W: usize>(rule: &[u8], chained: bool) -> Option<FirstRead> {
    let count_at = if chained { 2 + W * usize::from(read_u16_be(rule, 0)?) } else { 0 };
    let count = usize::from(read_u16_be(rule, count_at)?);
    let input_at = count_at + if chained { 2 } else { 4 };
    if (2..=SEQUENCE_LIMIT).contains(&count) {
        return read_uint(rule, input_at, W).map(FirstRead::Input);
    }
    // Before an input of one, nothing: its lookahead count follows the input count at once.
    let ahead_len = if chained && count < 2 { read_u16_be(rule, count_at + 2)? } else { 0 };
    (ahead_len > 0).then(|| read_uint(rule, count_at + 4, W).map(FirstRead::Ahead)).flatten()
}

// The `len` coverages at offsets of `W` bytes from `at`, each answered by the plan's index for it where there is one,
// before its offset is read.
#[derive(Clone, Copy)]
pub(crate) struct Coverages<'a, const W: usize> {
    data: &'a [u8],
    at: usize,
    pub(crate) len: u16,
    indexes: &'a [Option<Shared<SparseIndex>>],
}

impl<'a, const W: usize> Coverages<'a, W> {
    pub(crate) fn new(data: &'a [u8], at: usize, len: u16, indexes: &'a [Option<Shared<SparseIndex>>]) -> Self {
        Coverages { data, at, len, indexes }
    }

    pub(crate) fn covers(&self, i: usize, glyph: u32) -> bool {
        if i >= usize::from(self.len) {
            return false;
        }
        if let Some(Some(index)) = self.indexes.get(i) {
            return index.lookup(glyph) != COVERAGE_ABSENT;
        }
        Coverage::new(table_at_width(self.data, self.at + i * W, W)).contains(glyph)
    }
}

// Which of a chained rule's lists a value belongs to.
#[derive(Clone, Copy)]
enum List {
    Back,
    Input,
    Ahead,
}

// How a rule's values stand for glyphs: as the glyphs themselves (format 1), or as their classes in each list's class
// definition (format 2).
trait Values: Copy {
    fn of(&self, list: List, glyph: u32) -> u32;
}

#[derive(Clone, Copy)]
struct AsGlyphs;

impl Values for AsGlyphs {
    #[inline(always)]
    fn of(&self, _: List, glyph: u32) -> u32 {
        glyph
    }
}

#[derive(Clone, Copy)]
struct AsClasses<'a>([ClassDef<'a>; 3]);

impl Values for AsClasses<'_> {
    #[inline(always)]
    fn of(&self, list: List, glyph: u32) -> u32 {
        self.0[list as usize].indexed_class_of(glyph)
    }
}

// What a rule asks of the glyphs at and about the current one, however its format names them.
trait Pattern {
    fn count(&self) -> u16;
    fn records(&self) -> Records<'_>;
    // The `k`th glyph of the input after the first, of the backtrack (nearest first) and of the lookahead.
    fn input(&self, k: usize, glyph: u32) -> bool;
    fn back(&self) -> (u16, impl Fn(u32, usize) -> bool);
    fn ahead(&self) -> (u16, impl Fn(u32, usize) -> bool);
}

impl<const W: usize, V: Values> Pattern for (Rule<'_, W>, V) {
    fn count(&self) -> u16 {
        self.0.count
    }
    fn records(&self) -> Records<'_> {
        self.0.records
    }
    fn input(&self, k: usize, glyph: u32) -> bool {
        self.0.input.get(k).is_some_and(|v| self.1.of(List::Input, glyph) == v)
    }
    fn back(&self) -> (u16, impl Fn(u32, usize) -> bool) {
        (self.0.back.len, |g, k| self.0.back.get(k).is_some_and(|v| self.1.of(List::Back, g) == v))
    }
    fn ahead(&self) -> (u16, impl Fn(u32, usize) -> bool) {
        (self.0.ahead.len, |g, k| self.0.ahead.get(k).is_some_and(|v| self.1.of(List::Ahead, g) == v))
    }
}

// A format 3 rule: a coverage for each glyph.
struct Covered<'a, const W: usize> {
    back: Coverages<'a, W>,
    input: Coverages<'a, W>,
    ahead: Coverages<'a, W>,
    records: Records<'a>,
}

impl<const W: usize> Pattern for Covered<'_, W> {
    fn count(&self) -> u16 {
        self.input.len
    }
    fn records(&self) -> Records<'_> {
        self.records
    }
    fn input(&self, k: usize, glyph: u32) -> bool {
        self.input.covers(k + 1, glyph)
    }
    fn back(&self) -> (u16, impl Fn(u32, usize) -> bool) {
        (self.back.len, |g, k| self.back.covers(k, g))
    }
    fn ahead(&self) -> (u16, impl Fn(u32, usize) -> bool) {
        (self.ahead.len, |g, k| self.ahead.covers(k, g))
    }
}

// Sequence context: formats 1 to 3, or with `W` 3 their 24-bit forms 4 to 6.
pub(crate) fn sequence<const W: usize>(e: &mut Engine, data: &[u8]) -> bool {
    match read_u16_be(data, 0).map(|format| format.wrapping_sub(if W == 3 { 3 } else { 0 })) {
        Some(format @ (1 | 2)) => rule_sets::<W>(e, data, format, false),
        Some(3) => {
            let (Some(count), Some(records)) = (read_u16_be(data, 2), read_u16_be(data, 4)) else { return false };
            let input = Coverages::<W>::new(data, 6, count, e.rule_indexes());
            if !input.covers(0, e.run.current().id) {
                return false;
            }
            let none = Coverages::new(&[], 0, 0, &[]);
            let bytes = data.get(6 + usize::from(count) * W..).unwrap_or_default();
            let records = Records { bytes, count: records };
            try_rule(e, &Covered { back: none, input, ahead: none, records }, false)
        }
        _ => false,
    }
}

// Chained sequence context: formats 1 to 3, or with `W` 3 the 24-bit forms of 1 and 2, 4 and 5.
pub(crate) fn chained<const W: usize>(e: &mut Engine, data: &[u8]) -> bool {
    match read_u16_be(data, 0).map(|format| format.wrapping_sub(if W == 3 { 3 } else { 0 })) {
        Some(format @ (1 | 2)) => rule_sets::<W>(e, data, format, true),
        Some(3) if W == 2 => chained_coverages(e, data),
        _ => false,
    }
}

// A chain format 3's backtrack, input and lookahead lengths and record count; None where they don't all read and fit
// within the subtable, which then doesn't apply.
pub(crate) fn chain_lists(data: &[u8]) -> Option<[u16; 4]> {
    let (mut lists, mut at) = ([0; 4], 2);
    for (i, len) in lists.iter_mut().enumerate() {
        *len = read_u16_be(data, at)?;
        at += 2 + if i < 3 { usize::from(*len) * 2 } else { 0 };
    }
    (at <= data.len()).then_some(lists)
}

// Backtrack, input and lookahead coverage lists, each counted, then the records; the plan indexes them in that order.
fn chained_coverages(e: &mut Engine, data: &[u8]) -> bool {
    let Some([back, input, ahead, count]) = e.chain_lists(data) else { return false };
    let (mut indexes, mut at) = (e.rule_indexes(), 2);
    let mut list = |len: u16| {
        let list = Coverages::<2>::new(data, at + 2, len, indexes);
        indexes = indexes.get(usize::from(len)..).unwrap_or_default();
        at += 2 + usize::from(len) * 2;
        list
    };
    let (back, input, ahead) = (list(back), list(input), list(ahead));
    let records = Records { bytes: data.get(at + 2..).unwrap_or_default(), count };
    if !input.covers(0, e.run.current().id) {
        return false;
    }
    // A one-glyph input's lookahead reads first what follows it. Where its first coverage refuses that, the rule fails
    // unparsed, marked as its match would mark it: most rules tried end here.
    let start = e.run.cursor;
    if input.len == 1 && ahead.len > 0 {
        let refused = Met::of(e.peek_once(Side::Context), |g| g).refuses_unless(|g| ahead.covers(0, g));
        if let Some(end) = refused {
            e.unsafe_to_concat(start..end);
            return false;
        }
    }
    // A longer input reads its second glyph first, which fails the rule alike.
    if input.len > 1
        && let Some(end) = second_input_refused(e, input)
    {
        e.unsafe_to_concat(start..end);
        return false;
    }
    try_rule(e, &Covered { back, input, ahead, records }, true)
}

// Apart, as inline it lengthened the one-glyph inputs' path, which most format 3 rules take.
#[inline(never)]
fn second_input_refused(e: &mut Engine, input: Coverages<2>) -> Option<usize> {
    Met::of(e.peek_once(Side::Input), |g| g).refuses_unless(|g| input.covers(1, g))
}

// Formats 1 and 2: the rules of the set the current glyph's coverage index or class picks. Out of line, as inlined
// they'd lengthen the format 3 path most fonts take.
#[inline(never)]
fn rule_sets<const W: usize>(e: &mut Engine, data: &[u8], format: u16, chained: bool) -> bool {
    let glyph = e.run.current().id;
    let l = long(W);
    let Some(covered) = e.coverage(data, 2, l).index_of(glyph) else { return false };
    match (format, chained) {
        // A 24-bit plain set reaches its glyph rules at 16-bit offsets, a chained one at 24-bit offsets.
        (1, false) => try_set::<2, W>(e, nth_table_width(data, 2 + l, W, W, covered), covered, false, AsGlyphs),
        (1, true) => try_set::<W, W>(e, nth_table_width(data, 2 + l, W, W, covered), covered, true, AsGlyphs),
        (_, false) => {
            let classes = e.rule_classes(data, 2 + l, l, 0);
            let class = classes.class_of(glyph);
            let set = nth_table_width(data, 2 + 2 * l, W, W, class);
            try_set::<W, 2>(e, set, class, false, AsClasses([classes; 3]))
        }
        (_, true) => {
            let lists = [0, 1, 2].map(|i| e.rule_classes(data, 2 + l * (i + 1), l, i));
            let class = lists[1].class_of(glyph);
            // Even the 24-bit form counts these sets in 16 bits.
            let set = nth_table_width(data, 2 + 4 * l, 2, W, class);
            try_set::<W, 2>(e, set, class, true, AsClasses(lists))
        }
    }
}

// What a rule's first read past the current glyph meets: a value and the end past it, nothing (the read stops before
// `end`), or a glyph a value may take or pass, which tells nothing alone.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Met {
    Value(u32, usize),
    Stop(usize),
    Unsure,
}

impl Met {
    fn of(peek: Peek, value: impl Fn(u32) -> u32) -> Met {
        match peek {
            Peek::Glyph(g, end) => Met::Value(value(g), end),
            Peek::Stop(end) => Met::Stop(end),
            Peek::Optional(..) => Met::Unsure,
        }
    }

    // Whether a rule wanting `value` here fails at once, and the end of what the failing read reads.
    fn refuses(self, value: u32) -> Option<usize> {
        self.refuses_unless(|v| v == value)
    }

    // `refuses`, for a rule taking any value `takes` does.
    fn refuses_unless(self, takes: impl Fn(u32) -> bool) -> Option<usize> {
        match self {
            Met::Value(v, end) if !takes(v) => Some(end),
            Met::Stop(end) => Some(end),
            _ => None,
        }
    }

    fn end(self) -> Option<usize> {
        match self {
            Met::Value(_, end) | Met::Stop(end) => Some(end),
            Met::Unsure => None,
        }
    }

    // Whether a digest half rules out every rule reading this first: the value's bit is clear (each value sets bit
    // `value % 31`), bit 31 is clear where the read stops, or, where the glyph tells nothing, no rule reads here.
    fn ruled_out_by(self, half: u32) -> bool {
        match self {
            Met::Value(v, _) => half & 1 << (v % 31) == 0,
            Met::Stop(_) => half & 1 << 31 == 0,
            Met::Unsure => half == 0,
        }
    }
}

// The set's rules in turn until one applies. A rule whose first value isn't what's there fails unparsed, marked as its
// match would mark it, and a set whose digest rules out every rule fails at once.
fn try_set<const RW: usize, const VW: usize>(
    e: &mut Engine,
    set: &[u8],
    set_index: u32,
    chained: bool,
    values: impl Values,
) -> bool {
    let start = e.run.cursor;
    let input = Met::of(e.peek_once(Side::Input), |g| values.of(List::Input, g));
    let mut ahead = None;
    let mut read_ahead = |e: &mut Engine| *ahead.get_or_insert_with(|| {
        Met::of(e.peek_once(Side::Context), |g| values.of(List::Ahead, g))
    });

    let (digest, firsts) = e.rule_set(set_index);
    let (input_half, ahead_half) = (digest as u32, (digest >> 32) as u32);
    if input.ruled_out_by(input_half) && (ahead_half == 0 || read_ahead(e).ruled_out_by(ahead_half)) {
        let input_end = if input_half != 0 { input.end() } else { None };
        let ahead_end = if ahead_half != 0 { read_ahead(e).end() } else { None };
        let end = input_end.max(ahead_end).unwrap_or(start);
        e.unsafe_to_concat(start..end);
        return false;
    }
    // A set may name one rule over and over, so each tried costs one of the run's budget. They're counted here and
    // taken before a rule is matched and at the end, so the run fails at the rule it would fail at one by one.
    let mut left = e.run.ops_left();
    let mut owed = 0usize;
    for i in 0..usize::from(read_u16_be(set, 0).unwrap_or(0)) {
        owed += 1;
        if owed as i64 > left {
            e.run.spend(owed);
            return false;
        }
        let rule = || table_at_width(set, 2 + i * RW, RW);
        // The plan's copy where it kept one, so a rule refused here costs no read of the font.
        let first = match firsts.get(i) {
            Some(&word) => FirstRead::unpack(word),
            None => first_read::<VW>(rule(), chained),
        };
        let refused = match first {
            Some(FirstRead::Input(value)) => input.refuses(value),
            Some(FirstRead::Ahead(value)) => read_ahead(e).refuses(value),
            None => None,
        };
        if let Some(end) = refused {
            e.unsafe_to_concat(start..end);
            continue;
        }
        e.run.spend(core::mem::take(&mut owed));
        if try_rule(e, &(Rule::<VW>::read(rule(), chained), values), chained) {
            return true;
        }
        left = e.run.ops_left();
    }
    e.run.spend(owed);
    false
}

// Matches a rule at the cursor, its input first, then what follows and what precedes it, and applies its records. A
// failure marks what it read unsafe to concatenate, as more text there could complete the match.
fn try_rule(e: &mut Engine, rule: &impl Pattern, chained: bool) -> bool {
    let start = e.run.cursor;
    let input = match e.match_input(rule.count(), |g, k| rule.input(k, g)) {
        Ok(input) => input,
        Err(end) => {
            e.unsafe_to_concat(start..end);
            return false;
        }
    };
    if !chained {
        e.run.unsafe_to_break(start..input.end);
        apply_records(e, usize::from(rule.count()), input.end, rule.records());
        return true;
    }
    let (ahead_len, ahead) = rule.ahead();
    let end = match e.match_lookahead(input.end, ahead_len, ahead) {
        Ok(end) => end,
        Err(end) => {
            e.unsafe_to_concat(start..end);
            return false;
        }
    };
    let (back_len, back) = rule.back();
    let from = match e.match_backtrack(back_len, back) {
        Ok(from) => from,
        Err(from) => {
            e.unsafe_to_concat_across(from, end);
            return false;
        }
    };
    e.run.unsafe_to_break_across(from, end);
    apply_records(e, usize::from(rule.count()), input.end, rule.records());
    true
}

// An input's glyphs by their places in the run as a rewrite has it: the written glyphs, then the unread ones.
struct Sequence {
    places: [u32; SEQUENCE_LIMIT],
    len: usize,
    end: usize,
}

impl Sequence {
    // The input just matched, ending at unread glyph `end`.
    fn matched(e: &Engine, len: usize, end: usize) -> Sequence {
        let to_live = |unread: usize| unread + e.run.behind() - e.run.cursor;
        let mut places = e.matched();
        places[..len].iter_mut().for_each(|p| *p = to_live(*p as usize) as u32);
        Sequence { places, len, end: to_live(end) }
    }

    // The run grew by `grown` (shrank where negative) as a lookup applied at input glyph `i`. False where the input
    // would pass the sequence limit.
    fn moved(&mut self, i: usize, grown: isize) -> bool {
        let at = self.places[i] as usize;
        // A lookup that took glyphs past the input's end leaves the end at its glyph, and only the input's taken.
        let end = self.end.checked_add_signed(grown).filter(|&end| end >= at);
        let grown = end.map_or(at as isize - self.end as isize, |_| grown);
        self.end = end.unwrap_or(at);
        let next = i + 1;
        match grown.unsigned_abs() {
            n if grown > 0 => {
                if self.len + n > SEQUENCE_LIMIT {
                    return false;
                }
                self.places.copy_within(next..self.len, next + n);
                self.len += n;
                // The glyphs it put in come right after its own and join the input.
                for (k, p) in self.places[next..next + n].iter_mut().enumerate() {
                    *p = (at + 1 + k) as u32;
                }
                self.places[next + n..self.len].iter_mut().for_each(|p| *p += n as u32);
            }
            n => {
                let gone = n.min(self.len - next);
                self.places.copy_within(next + gone..self.len, next);
                self.len -= gone;
                self.places[next..self.len].iter_mut().for_each(|p| *p -= n as u32);
            }
        }
        true
    }
}

// Each record's lookup at its input glyph in order, each seeing the run as those before left it. A lookup at one glyph
// adds glyphs only right after it and removes only later ones, so the places past it shift by the difference.
fn apply_records(e: &mut Engine, len: usize, end: usize, records: Records) {
    let mut input = Sequence::matched(e, len, end);
    for (glyph, lookup) in (0..records.count).map_while(|i| records.get(i)) {
        let glyph = usize::from(glyph);
        if e.run.failed {
            break;
        }
        if glyph >= input.len {
            continue;
        }
        let (at, before) = (input.places[glyph] as usize, e.run.live_len());
        if at >= before {
            continue;
        }
        if !e.run.seek(at) || e.run.ops_left() <= 0 {
            break;
        }
        if !e.nested(lookup) {
            continue;
        }
        let grown = e.run.live_len() as isize - before as isize;
        if grown != 0 && !input.moved(glyph, grown) {
            break;
        }
    }
    e.run.seek(input.end);
}

// What a walk over a subtable's records may still read, and where it hands the lookups it finds.
struct Visit<'w, F> {
    work: &'w mut usize,
    visit: F,
}

impl<F: FnMut(u16)> Visit<'_, F> {
    fn pay(&mut self, n: usize) -> bool {
        spend_work(self.work, n)
    }

    // A rule's records cost one, and one each that fits.
    fn records(&mut self, records: Records) -> bool {
        let paid = self.pay(1 + usize::from(records.count).min(records.bytes.len() / 4));
        if paid {
            records.lookups().for_each(&mut self.visit);
        }
        paid
    }

    // Each rule of each set, the sets' offsets counted at `count_at` in `count` bytes, each of `width`; the rules at
    // `RW`-byte offsets, their values `VW` bytes each. A set costs one.
    fn sets<const RW: usize, const VW: usize>(&mut self, data: &[u8], chained: bool, (at, count, width): Spot) {
        for set in tables_width(data, at, count, width) {
            if !self.pay(1) {
                return;
            }
            for rule in tables_of::<RW>(set, 0) {
                if !self.records(Rule::<VW>::read(rule, chained).records) {
                    return;
                }
            }
        }
    }
}

// Where a subtable counts its rule sets, in how many bytes, and the width of their offsets.
type Spot = (usize, usize, usize);

// Every lookup a context subtable's records name, as often as they name it, for the plan to index; until `work`
// runs out. The 24-bit forms keep 16-bit records.
pub(crate) fn record_lookups(subtable: &[u8], chained: bool, work: &mut usize, visit: impl FnMut(u16)) {
    let mut v = Visit { work, visit };
    let word = |at: usize| read_u16_be(subtable, at).unwrap_or(0);
    let records_at = |at: usize, count: u16| Records { bytes: subtable.get(at..).unwrap_or_default(), count };
    match (read_u16_be(subtable, 0).unwrap_or(0), chained) {
        (format @ (3 | 6), false) => {
            let width = if format == 6 { 3 } else { 2 };
            v.records(records_at(6 + usize::from(word(2)) * width, word(4)));
        }
        (3, true) => {
            let lists_end = (0..3).fold(2, |at, _| at + 2 + usize::from(word(at)) * 2);
            v.records(records_at(lists_end + 2, word(lists_end)));
        }
        (1, _) => v.sets::<2, 2>(subtable, chained, (4, 2, 2)),
        (2, false) => v.sets::<2, 2>(subtable, chained, (6, 2, 2)),
        (2, true) => v.sets::<2, 2>(subtable, chained, (10, 2, 2)),
        (4, false) => v.sets::<2, 3>(subtable, chained, (6, 3, 3)),
        (4, true) => v.sets::<3, 3>(subtable, chained, (6, 3, 3)),
        (5, false) => v.sets::<3, 2>(subtable, chained, (10, 3, 3)),
        (5, true) => v.sets::<3, 2>(subtable, chained, (18, 2, 3)),
        _ => {}
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daeshaper/ot/apply/context.rs"]
mod tests;
