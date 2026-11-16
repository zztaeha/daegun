use alloc::vec::Vec;
use crate::daecore::daetype::format::aat::{morx_chains, Entry, Lookup, StateTable};
use crate::daecore::daetype::format::index::SparseIndex;
use crate::daecore::daetype::format::sanitize::AatTable;
use crate::daecore::daeshaper::run::{Fact, Run, Glyph};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daekit::base::sync::Shared;
use crate::daecore::daetype::decoder::{read_u16_be, read_u32_be, window};
use super::aat::{run_machine, SubtableSets, Switches, DONT_ADVANCE, NO_ACTION};
use super::morx_flags::RangeFlags;
use super::apply::SEQUENCE_LIMIT;
use crate::daecore::daeshaper::run::text;

// A deleted component is set to this, a *class* later subtables can still match on, and swept once the whole table has
// run, as the format needs deletion deferred.
pub(crate) const DELETED_GLYPH: u32 = 0xFFFF;

mod coverage {
    pub(super) const VERTICAL: u32 = 0x8000_0000;
    pub(super) const DESCENDING: u32 = 0x4000_0000;
    pub(super) const ALL_DIRECTIONS: u32 = 0x2000_0000;
    pub(super) const LOGICAL: u32 = 0x1000_0000;
    pub(super) const TYPE: u32 = 0x0000_00FF;
}

mod entry_flags {
    pub(super) const REARRANGE_VERB: u16 = 0x000F;
    pub(super) const MARK_LAST: u16 = 0x2000;
    pub(super) const MARK_FIRST: u16 = 0x8000;

    pub(super) const SET_MARK: u16 = 0x8000;

    pub(super) const SET_COMPONENT: u16 = 0x8000;
    pub(super) const PERFORM_ACTION: u16 = 0x2000;

    pub(super) const CURRENT_INSERT_BEFORE: u16 = 0x0800;
    pub(super) const MARKED_INSERT_BEFORE: u16 = 0x0400;
    pub(super) const CURRENT_INSERT_COUNT: u16 = 0x03E0;
    pub(super) const MARKED_INSERT_COUNT: u16 = 0x001F;
}

mod lig_action {
    pub(super) const LAST: u32 = 0x8000_0000;
    pub(super) const STORE: u32 = 0x4000_0000;
    pub(super) const OFFSET: u32 = 0x3FFF_FFFF;
    pub(super) const SIGN: u32 = 0x2000_0000;
}

// Each subtable's sets by its place among every chain's subtables, for those `chain_flags` turn on, as the font cache
// keeps them: the plan holds them, so a shape reads them without asking the face.
pub(crate) fn plan_sets(
    face: &Face,
    table: &[u8],
    chain_flags: &[Vec<RangeFlags>],
) -> Vec<Option<Shared<SubtableSets>>> {
    let num_glyphs = face.glyph_count();
    // A slot for each subtable that fits, not each a chain's header counts, which may run to 2^32.
    let count = morx_chains(table).map(|chain| chain.subtables(table).count()).sum();
    let mut sets = Vec::new();
    for (chain, ranges) in morx_chains(table).zip(chain_flags) {
        for sub in chain.subtables(table) {
            let (index, on) = (sets.len(), ranges.iter().any(|r| r.flags & sub.flags != 0));
            let build = || start_set(face, sub.body, sub.coverage, num_glyphs);
            sets.push(on.then(|| face.subtable_sets(AatTable::Morx, count, index, build)));
        }
    }
    sets.shrink_to_fit();
    sets
}

// `table` is one the sanitizer passes, and `chain_flags` and `sets` what the plan holds of it. In a font with glyph
// classes, an edit gives the glyphs it puts in their own. Out of line, as inlined it slowed shapes that never reach it.
#[inline(never)]
pub(crate) fn apply(
    face: &Face,
    table: &[u8],
    buffer: &mut Run,
    num_glyphs: u32,
    chain_flags: &[Vec<RangeFlags>],
    sets: &[Option<Shared<SubtableSets>>],
) {
    if buffer.failed {
        return;
    }
    buffer.unsafe_to_concat_all();
    let mut cx = Context::new(buffer, num_glyphs, face.gives_classes().then_some(face));
    let mut index = 0;
    'chains: for (chain, ranges) in morx_chains(table).zip(chain_flags) {
        for sub in chain.subtables(table) {
            if let Some(Some(sets)) = sets.get(index) {
                let on = Switches { ranges, flags: sub.flags };
                apply_subtable(sub.body, sub.coverage, buffer, num_glyphs, sets, &mut cx, on);
                if buffer.failed {
                    break 'chains;
                }
            }
            index += 1;
        }
    }
    cx.finish(buffer);
}

// What a subtable reads once a face: a machine's initial glyphs and class index, or the glyphs a non-contextual
// lookup lists.
fn start_set(face: &Face, body: &[u8], cov: u32, num_glyphs: u32) -> SubtableSets {
    let machine = |extra: usize, actionable: fn(&Entry) -> bool| {
        StateTable::parse(body, extra, num_glyphs).ok().map(|t| SubtableSets::machine(face, &t, actionable))
    };
    let sets = match cov & coverage::TYPE {
        0 => machine(0, actionable::rearrangement),
        1 => machine(2, actionable::contextual),
        2 => machine(1, actionable::ligature),
        4 => Lookup::parse(body, num_glyphs).ok().map(|l| SubtableSets { first: l.glyph_set(), ..<_>::default() }),
        5 => machine(2, actionable::insertion),
        _ => None,
    };
    sets.unwrap_or_default()
}

pub(crate) fn remove_deleted_glyphs(buffer: &mut Run) {
    if buffer.facts.has(Fact::Deleted) {
        buffer.delete_where(|g: &Glyph| g.char_is(text::DELETED));
    }
}

// What morx's subtables share as they run: a glyph turned into the deletion sentinel is marked deleted, in a font with
// glyph classes a glyph put in takes its own GDEF properties, and a subtable runs only when a glyph can start it.
struct Context<'a> {
    classes: Option<&'a Face<'a>>,
    seen: Option<Seen>,
}

// Every glyph the run held when morx started and every one an edit put in since, kept from four glyphs up, where it
// beats a scan. A glyph gone since only lets a subtable run that changes nothing.
#[derive(Default)]
pub(crate) struct Seen {
    bits: Vec<u64>,
    glyphs: Vec<u16>,
}

impl Seen {
    fn add(&mut self, glyph: u32) {
        let g = glyph as u16;
        let (word, bit) = (usize::from(g / 64), g % 64);
        if word >= self.bits.len() {
            self.bits.resize(word + 1, 0);
        }
        if self.bits[word] >> bit & 1 == 0 {
            self.bits[word] |= 1 << bit;
            self.glyphs.push(g);
        }
    }

    // Emptied by the glyphs it holds, its room kept.
    pub(crate) fn clear(&mut self) {
        for &g in &self.glyphs {
            self.bits[usize::from(g / 64)] = 0;
        }
        self.glyphs.clear();
    }
}

impl<'a> Context<'a> {
    // Takes the buffer's set, wide enough for the font's glyphs, until `finish` gives it back.
    fn new(buffer: &mut Run, num_glyphs: u32, classes: Option<&'a Face<'a>>) -> Self {
        let seen = (buffer.len >= 4).then(|| {
            let mut seen = core::mem::take(&mut buffer.morx_seen);
            let words = num_glyphs as usize / 64 + 1;
            if seen.bits.len() < words {
                seen.bits.resize(words, 0);
            }
            seen.glyphs.reserve(buffer.len);
            buffer.glyphs[..buffer.len].iter().for_each(|info| seen.add(info.id));
            seen
        });
        Context { classes, seen }
    }

    fn finish(self, buffer: &mut Run) {
        if let Some(mut seen) = self.seen {
            seen.clear();
            buffer.morx_seen = seen;
        }
    }

    // Whether a glyph the set holds can start the subtable, or, with no set, a glyph in the buffer.
    fn reaches(&self, buffer: &Run, starts: impl Fn(u32) -> bool) -> bool {
        match &self.seen {
            Some(seen) => seen.glyphs.iter().any(|&g| starts(u32::from(g))),
            None => buffer.glyphs[..buffer.len].iter().any(|info| starts(info.id)),
        }
    }

    // As `replace_glyph_inplace`.
    fn replace_at(&mut self, buffer: &mut Run, i: usize, glyph: u32) {
        if glyph == DELETED_GLYPH {
            buffer.facts.note(Fact::Deleted);
            buffer.glyphs[i].mark_deleted();
        }
        if let Some(seen) = &mut self.seen {
            seen.add(glyph);
        }
        if let Some(face) = self.classes {
            (buffer.glyphs[i].class, buffer.glyphs[i].attach_class) = face.classes(glyph);
        }
        buffer.glyphs[i].id = glyph;
    }

    // As `Run::delete_current`: the current glyph goes out as the deletion sentinel and keeps its GDEF properties.
    fn delete(&mut self, buffer: &mut Run) {
        buffer.facts.note(Fact::Deleted);
        if let Some(seen) = &mut self.seen {
            seen.add(DELETED_GLYPH);
        }
        let i = buffer.cursor;
        buffer.glyphs[i].mark_deleted();
        buffer.replace(DELETED_GLYPH);
    }

    // As `Run::replace`: the current glyph, put out.
    fn replace(&mut self, buffer: &mut Run, glyph: u32) {
        self.replace_at(buffer, buffer.cursor, glyph);
        buffer.replace(glyph);
    }

    // Puts out one glyph, marking the glyph it copies, the current one, so a glyph put in before it hands it the
    // deletion mark or its properties.
    fn output(&mut self, buffer: &mut Run, glyph: u32) {
        if let Some(seen) = &mut self.seen {
            seen.add(glyph);
        }
        let at_end = buffer.cursor >= buffer.len;
        if at_end {
            buffer.insert(glyph);
            if buffer.written == 0 {
                return;
            }
        }
        let deleted = glyph == DELETED_GLYPH;
        if deleted {
            buffer.facts.note(Fact::Deleted);
        }
        let props = self.classes.map(|face| face.classes(glyph));
        let last = buffer.written.saturating_sub(1);
        let info = if at_end { &mut buffer.out_mut()[last] } else { buffer.current_mut() };
        if deleted {
            info.mark_deleted();
        } else if let Some(props) = props {
            (info.class, info.attach_class) = props;
        }
        if !at_end {
            buffer.insert(glyph);
        }
    }
}


fn apply_subtable(
    body: &[u8],
    cov: u32,
    buffer: &mut Run,
    num_glyphs: u32,
    sets: &SubtableSets,
    cx: &mut Context,
    on: Switches,
) {
    let vertical = buffer.direction.is_vertical();

    if cov & coverage::ALL_DIRECTIONS == 0 && vertical != (cov & coverage::VERTICAL != 0) {
        return;
    }
    if !cx.reaches(buffer, |g| sets.first.contains(g)) {
        return;
    }

    let backward = buffer.direction.is_backward();
    let descending = cov & coverage::DESCENDING != 0;
    let reverse =
        if cov & coverage::LOGICAL != 0 { descending } else { descending != backward };

    if reverse {
        buffer.reverse();
    }
    let classes = sets.classes.as_ref();
    match cov & coverage::TYPE {
        0 => rearrangement(body, buffer, num_glyphs, classes, on),
        1 => contextual(body, buffer, num_glyphs, classes, cx, on),
        2 => ligature(body, buffer, num_glyphs, classes, cx, on),
        4 => non_contextual(body, buffer, num_glyphs, cx, on),
        5 => insertion(body, buffer, num_glyphs, classes, cx, on),
        _ => {}
    }
    if reverse {
        buffer.reverse();
    }
}

// Whether an entry acts, for each subtable type, which the subsetter reads start sets by too.
pub(crate) mod actionable {
    use super::{entry_flags, Entry, NO_ACTION};

    pub(crate) fn rearrangement(e: &Entry) -> bool {
        e.flags & entry_flags::REARRANGE_VERB != 0
    }
    pub(crate) fn contextual(e: &Entry) -> bool {
        e.word1 != NO_ACTION || e.word2 != NO_ACTION
    }
    pub(crate) fn ligature(e: &Entry) -> bool {
        e.flags & entry_flags::PERFORM_ACTION != 0
    }
    pub(crate) fn insertion(e: &Entry) -> bool {
        e.flags & (entry_flags::CURRENT_INSERT_COUNT | entry_flags::MARKED_INSERT_COUNT) != 0
            && (e.word1 != NO_ACTION || e.word2 != NO_ACTION)
    }
}

// Each verb's glyphs taken from the front and the back, and whether each end is turned.
const VERBS: [(usize, usize, bool, bool); 16] = [
    (0, 0, false, false),
    (1, 0, false, false),
    (0, 1, false, false),
    (1, 1, false, false),
    (2, 0, false, false),
    (2, 0, true, false),
    (0, 2, false, false),
    (0, 2, false, true),
    (1, 2, false, false),
    (1, 2, false, true),
    (2, 1, false, false),
    (2, 1, true, false),
    (2, 2, false, false),
    (2, 2, true, false),
    (2, 2, false, true),
    (2, 2, true, true),
];

fn rearrangement(body: &[u8], buffer: &mut Run, num_glyphs: u32, classes: Option<&SparseIndex>, on: Switches) {
    let Ok(table) = StateTable::parse(body, 0, num_glyphs) else { return };
    let table = table.with_classes(classes);

    let mut span_start = 0usize;
    let mut span_end = 0usize;

    run_machine(&table, buffer, false, actionable::rearrangement, Some(on), |buffer, entry| {
        let i = buffer.cursor;
        if entry.flags & entry_flags::MARK_FIRST != 0 {
            span_start = i;
        }
        if entry.flags & entry_flags::MARK_LAST != 0 {
            span_end = (i + 1).min(buffer.len);
        }

        let verb = usize::from(entry.flags & entry_flags::REARRANGE_VERB);
        if verb == 0 || span_start >= span_end {
            return;
        }
        // A span of any length rearranges; merging its clusters charges it to the budget.
        let (l, r, _, _) = VERBS[verb];
        if span_end - span_start < l + r {
            return;
        }

        buffer.join_clusters(span_start..(i + 1).min(buffer.len));
        buffer.join_clusters(span_start..span_end);
        rearrange(&mut buffer.glyphs[span_start..span_end], VERBS[verb]);
    });
}

// The first `l` and last `r` glyphs swap ends, each turned where flagged: the front rotated past the rest, then the
// back past the middle.
fn rearrange<T>(span: &mut [T], (l, r, rev_l, rev_r): (usize, usize, bool, bool)) {
    let n = span.len();
    if rev_l {
        span[..l].reverse();
    }
    if rev_r {
        span[n - r..].reverse();
    }
    span.rotate_left(l);
    span[..n - l].rotate_right(r);
}

fn contextual(
    body: &[u8],
    buffer: &mut Run,
    num_glyphs: u32,
    classes: Option<&SparseIndex>,
    cx: &mut Context,
    on: Switches,
) {
    let Ok(table) = StateTable::parse(body, 2, num_glyphs) else { return };
    let table = table.with_classes(classes);
    let Some(subst_off) = read_u32_be(body, 16) else { return };
    let Some(substitutions) = body.get(subst_off as usize..) else { return };

    let mut memo: [Option<(u16, Lookup<'_>)>; 2] = [None, None];

    let mut mark = 0usize;
    let mut mark_set = false;

    run_machine(&table, buffer, false, actionable::contextual, Some(on), |buffer, entry| {
        let i = buffer.cursor;
        if i >= buffer.len && !mark_set {
            return;
        }

        if entry.word1 != NO_ACTION && mark < buffer.len {
            let Some(l) = resolve(substitutions, num_glyphs, &mut memo, entry.word1) else { return };
            // Replaced or not, the glyph at the mark decided it, and a piece starting after the mark reads another.
            buffer.unsafe_to_break(mark..(i + 1).min(buffer.len));
            if let Some(g) = l.typed_value(buffer.glyphs[mark].id) {
                cx.replace_at(buffer, mark, u32::from(g));
            }
        }
        if entry.word2 != NO_ACTION && buffer.len > 0 {
            let at = i.min(buffer.len - 1);
            let Some(l) = resolve(substitutions, num_glyphs, &mut memo, entry.word2) else { return };
            if let Some(g) = l.typed_value(buffer.glyphs[at].id) {
                cx.replace_at(buffer, at, u32::from(g));
            }
        }
        if entry.flags & entry_flags::SET_MARK != 0 {
            mark = i;
            mark_set = true;
        }
    });
}

fn resolve<'a>(
    substitutions: &'a [u8],
    num_glyphs: u32,
    memo: &mut [Option<(u16, Lookup<'a>)>; 2],
    index: u16,
) -> Option<Lookup<'a>> {
    if let Some((at, l)) = memo[0]
        && at == index {
            return Some(l);
        }
    if let Some((at, l)) = memo[1]
        && at == index {
            memo.swap(0, 1);
            return Some(l);
        }
    let off = read_u32_be(substitutions, 4 * usize::from(index))? as usize;
    let l = Lookup::parse(substitutions.get(off..)?, num_glyphs).ok()?;
    memo[1] = memo[0];
    memo[0] = Some((index, l));
    Some(l)
}

fn ligature(
    body: &[u8],
    buffer: &mut Run,
    num_glyphs: u32,
    classes: Option<&SparseIndex>,
    cx: &mut Context,
    on: Switches,
) {
    let Ok(table) = StateTable::parse(body, 1, num_glyphs) else { return };
    let table = table.with_classes(classes);
    let Some(h) = window::<12>(body, 16) else { return };
    let w = |i: usize| u32::from_be_bytes([h[i], h[i + 1], h[i + 2], h[i + 3]]);
    let (action_off, component_off, ligature_off) = (w(0), w(4), w(8));

    let actions = body.get(action_off as usize..).unwrap_or_default();
    let components = body.get(component_off as usize..).unwrap_or_default();
    let ligatures = body.get(ligature_off as usize..).unwrap_or_default();

    let at = |index: u32, width: usize| usize::try_from(index).ok().and_then(|i| i.checked_mul(width));

    // Out-buffer positions of the components, a ring: past 64, the oldest are overwritten.
    let mut positions = [0usize; SEQUENCE_LIMIT];
    let mut depth = 0usize;

    run_machine(&table, buffer, true, actionable::ligature, Some(on), |buffer, entry| {
        if entry.flags & entry_flags::SET_COMPONENT != 0 {
            // DontAdvance can come back to a glyph; it is stacked once.
            if depth > 0 && positions[(depth - 1) % SEQUENCE_LIMIT] == buffer.written {
                depth -= 1;
            }
            positions[depth % SEQUENCE_LIMIT] = buffer.written;
            depth += 1;
        }

        if entry.flags & entry_flags::PERFORM_ACTION == 0 || depth == 0 || buffer.cursor >= buffer.len {
            return;
        }

        let end = buffer.written;
        let mut cursor = depth;
        // The ring holds the last 64 components, so a walk past them would act on ones it has already passed.
        let floor = depth.saturating_sub(SEQUENCE_LIMIT);
        let mut action_idx = u32::from(entry.word1);
        let mut ligature_idx = 0u32;
        loop {
            if cursor == floor {
                depth = 0;
                break;
            }
            cursor -= 1;
            if !buffer.seek(positions[cursor % SEQUENCE_LIMIT]) {
                return;
            }

            let Some(action) = at(action_idx, 4).and_then(|a| read_u32_be(actions, a)) else { break };
            let mut offset = action & lig_action::OFFSET;
            if offset & lig_action::SIGN != 0 {
                offset |= !lig_action::OFFSET;
            }
            let component_idx = buffer.current().id.wrapping_add(offset);
            let Some(component) = at(component_idx, 2).and_then(|a| read_u16_be(components, a)) else { break };
            ligature_idx = ligature_idx.wrapping_add(u32::from(component));

            if action & (lig_action::LAST | lig_action::STORE) != 0 {
                let Some(lig) = at(ligature_idx, 2).and_then(|a| read_u16_be(ligatures, a)) else { break };
                cx.replace(buffer, u32::from(lig));

                // The components stacked after this one go, and the ligature stays stacked for the next
                // action to build on.
                let lig_end = positions[(depth - 1) % SEQUENCE_LIMIT] + 1;
                while depth - 1 > cursor {
                    depth -= 1;
                    if !buffer.seek(positions[depth % SEQUENCE_LIMIT]) {
                        return;
                    }
                    cx.delete(buffer);
                }
                if !buffer.seek(lig_end) {
                    return;
                }
                buffer.join_out_clusters(positions[cursor % SEQUENCE_LIMIT]..buffer.written);
            }

            action_idx = action_idx.wrapping_add(1);
            if action & lig_action::LAST != 0 {
                break;
            }
        }
        buffer.seek(end);
    });
}

fn non_contextual(body: &[u8], buffer: &mut Run, num_glyphs: u32, cx: &mut Context, on: Switches) {
    let Ok(lookup) = Lookup::parse(body, num_glyphs) else { return };
    for i in 0..buffer.len {
        if !on.on_at(buffer.glyphs[i].cluster) {
            continue;
        }
        if let Some(g) = lookup.typed_value(buffer.glyphs[i].id) {
            cx.replace_at(buffer, i, u32::from(g));
        }
    }
}

fn insertion(
    body: &[u8],
    buffer: &mut Run,
    num_glyphs: u32,
    classes: Option<&SparseIndex>,
    cx: &mut Context,
    on: Switches,
) {
    let Ok(table) = StateTable::parse(body, 2, num_glyphs) else { return };
    let table = table.with_classes(classes);
    let Some(action_off) = read_u32_be(body, 16) else { return };
    let actions = body.get(action_off as usize..).unwrap_or_default();
    // A list is checked whole: one that runs past the table inserts nothing.
    let list = |index: u16, count: u16| {
        let at = 2 * usize::from(index);
        actions.get(at..at + 2 * usize::from(count)).unwrap_or_default()
    };

    // Each insertion spends its count first, and with the budget gone the transition ends there.
    let spend = |buffer: &mut Run, count: u16| buffer.take_ops(u32::from(count)) > 0;

    let mut mark = 0usize;
    run_machine(&table, buffer, true, actionable::insertion, Some(on), |buffer, entry| {
        let mark_loc = buffer.written;

        if entry.word2 != NO_ACTION {
            let count = entry.flags & entry_flags::MARKED_INSERT_COUNT;
            if !spend(buffer, count) {
                return;
            }
            let glyphs = list(entry.word2, count);
            let end = buffer.written;
            if !buffer.seek(mark) {
                return;
            }
            insert(buffer, glyphs, entry.flags & entry_flags::MARKED_INSERT_BEFORE != 0, cx);
            if !buffer.seek(end + glyphs.len() / 2) {
                return;
            }
            buffer.unsafe_to_break_across(mark, (buffer.cursor + 1).min(buffer.len));
        }

        if entry.flags & entry_flags::SET_MARK != 0 {
            mark = mark_loc;
        }

        if entry.word1 != NO_ACTION {
            let count = (entry.flags & entry_flags::CURRENT_INSERT_COUNT) >> 5;
            if !spend(buffer, count) {
                return;
            }
            let glyphs = list(entry.word1, count);
            let end = buffer.written;
            insert(buffer, glyphs, entry.flags & entry_flags::CURRENT_INSERT_BEFORE != 0, cx);
            let advance = if entry.flags & DONT_ADVANCE != 0 { 0 } else { glyphs.len() / 2 };
            buffer.seek(end + advance);
        }
    });
}

// Puts `glyphs` out before the current glyph, or after it, where the current glyph is put out first and
// then skipped.
fn insert(buffer: &mut Run, glyphs: &[u8], before: bool, cx: &mut Context) {
    let after = buffer.cursor < buffer.len && !before;
    if after {
        let info = buffer.glyphs[buffer.cursor];
        buffer.insert_glyph(info);
    }
    for g in glyphs.as_chunks::<2>().0 {
        cx.output(buffer, u32::from(u16::from_be_bytes(*g)));
    }
    if after {
        buffer.pass_over();
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/ot/morx.rs"]
mod tests;
