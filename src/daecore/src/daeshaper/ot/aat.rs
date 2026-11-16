// Apple's state machines (TrueType Reference Manual, "AAT state tables") as morx, kerx and kern run them: a glyph's
// class and the state name an entry, which gives the next state, whether to advance, and the table's own action.
use super::morx_flags::RangeFlags;
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::run::Run;
use crate::daecore::daetype::format::aat::{
    class, initial_glyphs, start_classes, state, Entry, GlyphRanges, Machine, StateTable,
};
use crate::daecore::daetype::format::index::{Entries, SparseIndex};

// An entry's DontAdvance flag: the machine reads the same glyph again, in its new state.
pub(super) const DONT_ADVANCE: u16 = 0x4000;
// An action or substitution index that names none.
pub(super) const NO_ACTION: u16 = 0xFFFF;

const START: i32 = state::START_OF_TEXT as i32;

// What is collected of a subtable once a face: the glyphs that can start it, for the pair formats of kerx and kern the
// glyphs a pair can end on, and a machine's class table indexed where it is searched.
#[derive(Default)]
pub(crate) struct SubtableSets {
    pub(super) first: GlyphRanges,
    pub(super) second: GlyphRanges,
    pub(super) classes: Option<SparseIndex>,
}

impl SubtableSets {
    // A machine's start glyphs and class index, the index drawn from the face's allowance.
    pub(super) fn machine(face: &Face, table: &StateTable, acts: fn(&Entry) -> bool) -> Self {
        let classes = table.searched_classes().and_then(|listed| {
            let fill = |e: &mut Entries| {
                listed.iter().for_each(|&(glyph, class)| e.put(glyph.into(), class.into()));
                Some(())
            };
            face.build_index(&fill, class::OUT_OF_BOUNDS)
        });
        SubtableSets { first: start_glyphs(table, acts), classes, ..SubtableSets::default() }
    }

    pub(crate) fn heap_bytes(&self) -> usize {
        self.first.heap_bytes() + self.second.heap_bytes() + self.classes.as_ref().map_or(0, SparseIndex::bytes)
    }
}

// The glyphs that can start a machine, so a run holding none of them passes it by. Where the end of text or a glyph
// the class table doesn't list leaves the start state or acts, any text starts it.
pub(super) fn start_glyphs(table: &impl Machine, acts: fn(&Entry) -> bool) -> GlyphRanges {
    let any_text = [class::END_OF_TEXT, class::OUT_OF_BOUNDS].map(u32::from);
    match start_classes(table, acts) {
        Some(taken) if !any_text.iter().any(|&k| taken.takes(k)) => initial_glyphs(table, acts),
        _ => GlyphRanges::new(alloc::vec![(0, u32::MAX)]),
    }
}

// A morx subtable's feature flags, and its chain's flags over each range of clusters: a glyph is on where the range
// holding its cluster shares a flag with the subtable.
#[derive(Clone, Copy)]
pub(super) struct Switches<'a> {
    pub(super) ranges: &'a [RangeFlags],
    pub(super) flags: u32,
}

impl Switches<'_> {
    pub(super) fn on_at(&self, cluster: u32) -> bool {
        let k = self.ranges.partition_point(|r| r.last < cluster);
        self.ranges.get(k).is_some_and(|r| r.flags & self.flags != 0)
    }
}

// The machine's state, and the entry it read last with the state and class that named it.
struct Walk<'t, T> {
    table: &'t T,
    state: i32,
    last: Option<((i32, u16), Entry)>,
}

impl<T: Machine> Walk<'_, T> {
    // The class of glyph `id`, or of the end of text for none; a class past the table's count reads as out of bounds.
    fn class(&self, id: Option<u32>) -> u16 {
        let Some(id) = id else { return class::END_OF_TEXT };
        let k = self.table.class(id);
        if usize::from(k) < self.table.n_classes() { k } else { class::OUT_OF_BOUNDS }
    }

    // Most steps read the entry the step before read.
    fn entry(&mut self, class: u16) -> Option<Entry> {
        let key = (self.state, class);
        match self.last {
            Some((k, entry)) if k == key => Some(entry),
            _ => {
                let entry = self.table.entry_at(key.0, key.1)?;
                self.last = Some((key, entry));
                Some(entry)
            }
        }
    }
}

// A glyph `switches` turns off is passed over and resets the machine. A step that doesn't advance costs one of the
// run's budget, and once that is spent every step advances, so a font can't loop forever.
pub(super) fn run_machine(
    table: &impl Machine,
    run: &mut Run,
    rewrite: bool,
    acts: fn(&Entry) -> bool,
    switches: Option<Switches>,
    mut act: impl FnMut(&mut Run, &Entry),
) {
    if rewrite {
        run.start_rewrite();
    }
    run.cursor = 0;
    let mut walk = Walk { table, state: START, last: None };
    let mut on = switches.is_none_or(|s| s.on_at(0));
    let mut stopped = false;
    while !run.failed {
        let glyph = run.glyphs[..run.len].get(run.cursor).map(|g| (g.id, g.cluster));
        if let (Some(s), Some((_, cluster))) = (switches, glyph) {
            on = s.on_at(cluster);
        }
        match (on, glyph) {
            (false, None) => break,
            (false, Some(_)) => {
                // Switched off, the machine stops as at a text's end but without that end's action; a piece ending here
                // would act.
                if run.behind() > 0 && !ends_quietly(table, walk.state, acts) {
                    run.unsafe_to_break_across(run.behind() - 1, run.cursor + 1);
                }
                walk.state = START;
                run.keep();
                continue;
            }
            _ => {}
        }
        let class = walk.class(glyph.map(|(id, _)| id));
        let Some(entry) = walk.entry(class) else {
            stopped = glyph.is_some();
            break;
        };
        if glyph.is_some() && run.behind() > 0 && !break_is_safe(table, walk.state, class, &entry, acts) {
            run.unsafe_to_break_across(run.behind() - 1, run.cursor + 1);
        }
        act(run, &entry);
        walk.state = table.next_state(&entry);
        if run.cursor >= run.len {
            break;
        }
        if entry.flags & DONT_ADVANCE == 0 || run.take_ops(1) < 0 {
            run.keep();
        }
    }
    if rewrite {
        run.finish_rewrite();
    }
    // A state past the table stops the machine with glyphs unread, which a piece starting among them would read.
    if stopped {
        run.unsafe_to_break_all();
    }
}

// Text broken before the current glyph shapes the same where this step does nothing, a machine started afresh here
// would go on alike, and an end of text after the glyph before would have done nothing.
fn break_is_safe(table: &impl Machine, state: i32, class: u16, entry: &Entry, acts: fn(&Entry) -> bool) -> bool {
    let stays = |e: &Entry| e.flags & DONT_ADVANCE != 0;
    let next = table.next_state(entry);
    let fresh_alike = || {
        table.entry_at(START, class).is_some_and(|fresh| {
            !acts(&fresh) && table.next_state(&fresh) == next && stays(&fresh) == stays(entry)
        })
    };
    let fresh = state == START || (stays(entry) && next == START) || fresh_alike();
    !acts(entry) && fresh && ends_quietly(table, state, acts)
}

// Whether the text ending in `state` would do nothing.
fn ends_quietly(table: &impl Machine, state: i32, acts: fn(&Entry) -> bool) -> bool {
    table.entry_at(state, class::END_OF_TEXT).is_none_or(|end| !acts(&end))
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/ot/aat.rs"]
mod tests;
