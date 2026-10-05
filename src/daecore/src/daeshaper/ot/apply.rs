use crate::daecore::daeshaper::buffer::{glyph_props, Buffer, GlyphInfo, Mask};
use crate::daecore::daeshaper::face::Face;
use super::map::TableIndex;
use super::{lookup_flags, ClassDef, Coverage, LayoutTable};
use crate::daecore::daeshaper::unicode::GeneralCategory;
use crate::daecore::daetype::decoder::{read_u16_be, window};

pub(crate) const MAX_NESTING_LEVEL: usize = 64;
pub(crate) const MAX_CONTEXT_LENGTH: usize = 64;

pub(crate) type MatchPositions = [u32; MAX_CONTEXT_LENGTH];

pub(crate) fn new_match_positions() -> MatchPositions {
    [0; MAX_CONTEXT_LENGTH]
}

#[derive(Clone, Copy, Default)]
struct MatcherFlags {
    ignore_zwnj: bool,
    ignore_zwj: bool,
    ignore_hidden: bool,
    mask: Mask,
}

pub(crate) type LookupDispatch = for<'a> fn(&mut ApplyContext<'a>, u16) -> bool;

pub(crate) struct ApplyContext<'a> {
    pub(crate) table_index: TableIndex,
    pub(crate) face: &'a Face<'a>,
    pub(crate) buffer: &'a mut Buffer,
    pub(crate) gsub: Option<&'a LayoutTable<'a>>,
    pub(crate) gpos: Option<&'a LayoutTable<'a>>,
    dispatch: LookupDispatch,

    lookup_mask: Mask,
    pub(crate) per_syllable: bool,
    pub(crate) lookup_index: u16,
    pub(crate) lookup_props: u32,
    pub(crate) nesting_level_left: usize,
    auto_zwnj: bool,
    auto_zwj: bool,
    pub(crate) random: bool,
    random_state: u32,
    pub(crate) last_base: i32,
    pub(crate) last_base_until: u32,
    pub(crate) glyph_digest: Option<super::digest::Digest>,
    pub(crate) subtable_indexes: Option<crate::daecore::sync::Shared<alloc::vec::Vec<super::SubtableIndex>>>,
    pub(crate) subtable: usize,
    pub(crate) lookup: Option<(u16, super::Lookup<'a>, u32)>,
    matcher: [MatcherFlags; 2],
}

impl<'a> ApplyContext<'a> {
    pub fn new(
        table_index: TableIndex,
        face: &'a Face<'a>,
        buffer: &'a mut Buffer,
        dispatch: LookupDispatch,
    ) -> Self {
        let mut ctx = ApplyContext {
            table_index,
            face,
            buffer,
            gsub: None,
            gpos: None,
            dispatch,
            lookup_mask: 1,
            per_syllable: false,
            lookup_index: u16::MAX,
            lookup_props: 0,
            nesting_level_left: MAX_NESTING_LEVEL,
            auto_zwnj: true,
            auto_zwj: true,
            random: false,
            random_state: 1,
            last_base: -1,
            last_base_until: 0,
            glyph_digest: None,
            subtable_indexes: None,
            subtable: 0,
            lookup: None,
            matcher: Default::default(),
        };
        ctx.refresh_matcher();
        ctx
    }

    fn indexes(&self) -> Option<&super::SubtableIndex> {
        self.subtable_indexes.as_ref()?.get(self.subtable)
    }

    pub(crate) fn subtable_may_have(&self, i: usize, glyph: u16) -> bool {
        self.subtable_indexes
            .as_ref()
            .and_then(|v| v.get(i))
            .is_none_or(|e| e.digest.may_have(glyph))
    }

    pub(crate) fn coverage_resolved<'s>(
        &'s self,
        subtable: &'s [u8],
        off: u16,
        indexed: bool,
    ) -> Option<Coverage<'s>> {
        let data = subtable.get(off as usize..)?;
        if indexed {
            Coverage::with_index(data, self.indexes().and_then(|i| i.coverage.as_ref()))
        } else {
            Coverage::new(data)
        }
    }

    pub(crate) fn coverage<'s>(&'s self, subtable: &'s [u8], at: usize) -> Option<Coverage<'s>> {
        let off = read_u16_be(subtable, at)? as usize;
        let data = subtable.get(off..)?;
        match at {
            2 => Coverage::with_index(data, self.indexes().and_then(|i| i.coverage.as_ref())),
            _ => Coverage::new(data),
        }
    }

    pub(crate) fn class_def<'s>(&'s self, subtable: &'s [u8], at: usize) -> Option<ClassDef<'s>> {
        let off = read_u16_be(subtable, at).filter(|&o| o != 0)? as usize;
        let data = subtable.get(off..)?;
        let index = match at {
            8 => self.indexes().and_then(|i| i.class1.as_ref()),
            10 => self.indexes().and_then(|i| i.class2.as_ref()),
            _ => None,
        };
        ClassDef::with_index(data, index)
    }

    pub(crate) fn lookup_mask(&self) -> Mask {
        self.lookup_mask
    }

    fn refresh_matcher(&mut self) {
        let gpos = self.table_index == TableIndex::Gpos;
        self.matcher = [
            MatcherFlags {
                ignore_zwnj: gpos,
                ignore_zwj: self.auto_zwj,
                ignore_hidden: gpos,
                mask: self.lookup_mask,
            },
            MatcherFlags {
                ignore_zwnj: gpos || self.auto_zwnj,
                ignore_zwj: true,
                ignore_hidden: gpos,
                mask: Mask::MAX,
            },
        ];
    }

    pub(crate) fn set_joiners(&mut self, auto_zwnj: bool, auto_zwj: bool) {
        if self.auto_zwnj != auto_zwnj || self.auto_zwj != auto_zwj {
            self.auto_zwnj = auto_zwnj;
            self.auto_zwj = auto_zwj;
            self.refresh_matcher();
        }
    }

    pub(crate) fn set_lookup_mask(&mut self, mask: Mask) {
        if self.lookup_mask != mask {
            self.lookup_mask = mask;
            self.refresh_matcher();
        }
        self.last_base = -1;
        self.last_base_until = 0;
    }

    pub(crate) fn random_number(&mut self) -> u32 {
        self.random_state = self.random_state.wrapping_mul(48271) % 2147483647;
        self.random_state
    }

    pub(crate) fn table(&self) -> Option<&'a LayoutTable<'a>> {
        match self.table_index {
            TableIndex::Gsub => self.gsub,
            TableIndex::Gpos => self.gpos,
        }
    }

    pub(crate) fn apply_lookup_index(&mut self, index: u16) -> bool {
        (self.dispatch)(self, index)
    }

    // Clears `subtable_indexes` and `glyph_digest` before descending: a nested lookup has its own
    // subtables with their own coverages, so leaving the caller's in place answers its queries from
    // the wrong tables entirely.
    pub(crate) fn recurse(&mut self, index: u16) -> bool {
        if self.nesting_level_left == 0 {
            self.buffer.shaping_failed = true;
            return false;
        }
        self.buffer.max_ops -= 1;
        if self.buffer.max_ops < 0 {
            self.buffer.shaping_failed = true;
            return false;
        }

        self.nesting_level_left -= 1;
        let saved_props = self.lookup_props;
        let saved_index = self.lookup_index;
        let saved_indexes = self.subtable_indexes.take();
        let saved_digest = self.glyph_digest.take();
        let saved_lookup = self.lookup.take();
        let saved_subtable = self.subtable;
        self.lookup_index = index;

        let applied = self.apply_lookup_index(index);

        self.lookup_props = saved_props;
        self.lookup_index = saved_index;
        self.subtable_indexes = saved_indexes;
        self.glyph_digest = saved_digest;
        self.lookup = saved_lookup;
        self.subtable = saved_subtable;
        self.nesting_level_left += 1;
        applied
    }

    pub(crate) fn check_glyph_property(&self, info: &GlyphInfo, match_props: u32) -> bool {
        let props = info.glyph_props;
        let flags = match_props as u16;

        if props & flags & lookup_flags::IGNORE_FLAGS != 0 {
            return false;
        }

        if props & glyph_props::MARK != 0 {
            if flags & lookup_flags::USE_MARK_FILTERING_SET != 0 {
                return self.face.is_mark_glyph(info.id as u16, (match_props >> 16) as u16);
            }
            if flags & lookup_flags::MARK_ATTACHMENT_TYPE_MASK != 0 {
                return (flags & lookup_flags::MARK_ATTACHMENT_TYPE_MASK)
                    == (props & lookup_flags::MARK_ATTACHMENT_TYPE_MASK);
            }
        }

        true
    }

    fn set_glyph_class(&mut self, glyph: u16, class_guess: u16, ligature: bool, component: bool) {
        let has_glyph_classes = self.face.has_glyph_classes();
        let face_props = self.face.glyph_props(glyph);
        let cur = self.buffer.cur_mut(0);
        let mut props = cur.glyph_props;

        props |= glyph_props::SUBSTITUTED;

        if ligature {
            props |= glyph_props::LIGATED;
            // Uniscribe honors only the last of ligate/expand/ligate, so re-ligating forgives an
            // intervening multiplication. Matching it is what keeps output identical on Windows.
            props &= !glyph_props::MULTIPLIED;
        }

        if component {
            props |= glyph_props::MULTIPLIED;
        }

        if has_glyph_classes {
            cur.glyph_props = (props & glyph_props::PRESERVE) | face_props;
        } else if class_guess != 0 {
            cur.glyph_props = (props & glyph_props::PRESERVE) | class_guess;
        } else {
            cur.glyph_props = props;
        }
    }

    pub(crate) fn replace_glyph(&mut self, glyph: u16) {
        self.set_glyph_class(glyph, 0, false, false);
        self.buffer.replace_glyph(u32::from(glyph));
    }

    pub(crate) fn replace_glyph_inplace(&mut self, glyph: u16) {
        self.set_glyph_class(glyph, 0, false, false);
        self.buffer.cur_mut(0).id = u32::from(glyph);
    }

    pub(crate) fn replace_glyph_with_ligature(&mut self, glyph: u16, class_guess: u16) {
        self.set_glyph_class(glyph, class_guess, true, false);
        self.buffer.replace_glyph(u32::from(glyph));
    }

    pub(crate) fn output_glyph_for_component(&mut self, glyph: u16, class_guess: u16) {
        self.set_glyph_class(glyph, class_guess, false, true);
        self.buffer.output_glyph(u32::from(glyph));
    }
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub(crate) enum Match {
    Yes,
    No,
    Skip,
}

#[derive(PartialEq, Eq, Copy, Clone)]
enum MayMatch {
    No,
    Yes,
    Maybe,
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub(crate) enum MaySkip {
    No,
    Yes,
    Maybe,
}

pub(crate) type NoMatch = fn(u16, u16) -> bool;

pub(crate) struct SkippingIterator<'i, 'a, F: ?Sized = NoMatch> {
    ctx: &'i ApplyContext<'a>,
    lookup_props: u32,
    ignore_zwnj: bool,
    ignore_zwj: bool,
    ignore_hidden: bool,
    mask: Mask,
    syllable: u8,
    matching: Option<&'i F>,
    buf_len: usize,
    glyph_data: u16,
    buf_idx: usize,
}

impl<'i, 'a> SkippingIterator<'i, 'a, NoMatch> {
    pub fn new(
        ctx: &'i ApplyContext<'a>,
        start_buf_index: usize,
        context_match: bool,
    ) -> Self {
        Self::build(ctx, start_buf_index, context_match, None)
    }
}

impl<'i, 'a, F: ?Sized + Fn(u16, u16) -> bool> SkippingIterator<'i, 'a, F> {
    pub(crate) fn with_matching(
        ctx: &'i ApplyContext<'a>,
        start_buf_index: usize,
        context_match: bool,
        func: &'i F,
    ) -> Self {
        Self::build(ctx, start_buf_index, context_match, Some(func))
    }

    fn build(
        ctx: &'i ApplyContext<'a>,
        start_buf_index: usize,
        context_match: bool,
        matching: Option<&'i F>,
    ) -> Self {
        SkippingIterator {
            ctx,
            lookup_props: ctx.lookup_props,
            ignore_zwnj: ctx.matcher[context_match as usize].ignore_zwnj,
            ignore_zwj: ctx.matcher[context_match as usize].ignore_zwj,
            ignore_hidden: ctx.matcher[context_match as usize].ignore_hidden,
            mask: ctx.matcher[context_match as usize].mask,
            // The syllable confined to is the one the *cursor* is in, not the one this walk starts
            // from. Deriving it from the start silently drops confinement for backtrack and
            // lookahead, which begin elsewhere – a chained rule then reaches into the next syllable
            // to satisfy its lookahead and substitutes on the strength of it.
            syllable: if ctx.per_syllable { ctx.buffer.cur(0).syllable } else { 0 },
            matching,
            buf_len: ctx.buffer.len,
            glyph_data: 0,
            buf_idx: start_buf_index,
        }
    }

    pub(crate) fn set_glyph_data(&mut self, glyph_data: u16) {
        self.glyph_data = glyph_data;
    }

    pub(crate) fn set_lookup_props(&mut self, lookup_props: u32) {
        self.lookup_props = lookup_props;
    }

    pub(crate) fn index(&self) -> usize {
        self.buf_idx
    }

    pub(crate) fn next(&mut self, unsafe_to: Option<&mut usize>) -> bool {
        let stop = self.buf_len as isize - 1;

        while (self.buf_idx as isize) < stop {
            self.buf_idx += 1;
            match self.match_(&self.ctx.buffer.info[self.buf_idx]) {
                Match::Yes => {
                    self.glyph_data += 1;
                    return true;
                }
                Match::No => {
                    if let Some(unsafe_to) = unsafe_to {
                        *unsafe_to = self.buf_idx + 1;
                    }
                    return false;
                }
                Match::Skip => continue,
            }
        }

        if let Some(unsafe_to) = unsafe_to {
            *unsafe_to = self.buf_idx + 1;
        }
        false
    }

    pub(crate) fn prev(&mut self, unsafe_from: Option<&mut usize>) -> bool {
        while self.buf_idx > 0 {
            self.buf_idx -= 1;
            match self.match_(&self.ctx.buffer.out_info()[self.buf_idx]) {
                Match::Yes => {
                    self.glyph_data += 1;
                    return true;
                }
                Match::No => {
                    if let Some(unsafe_from) = unsafe_from {
                        *unsafe_from = self.buf_idx.max(1) - 1;
                    }
                    return false;
                }
                Match::Skip => continue,
            }
        }

        if let Some(unsafe_from) = unsafe_from {
            *unsafe_from = 0;
        }
        false
    }

    pub(crate) fn match_(&self, info: &GlyphInfo) -> Match {
        let skip = self.may_skip(info);
        if skip == MaySkip::Yes {
            return Match::Skip;
        }

        let matched = self.may_match(info);
        if matched == MayMatch::Yes || (matched == MayMatch::Maybe && skip == MaySkip::No) {
            return Match::Yes;
        }

        if skip == MaySkip::No {
            return Match::No;
        }

        Match::Skip
    }

    fn may_match(&self, info: &GlyphInfo) -> MayMatch {
        if (info.mask & self.mask) == 0 || (self.syllable != 0 && self.syllable != info.syllable) {
            return MayMatch::No;
        }

        match self.matching {
            Some(func) if func(info.id as u16, self.glyph_data) => MayMatch::Yes,
            Some(_) => MayMatch::No,
            None => MayMatch::Maybe,
        }
    }

    pub(crate) fn may_skip(&self, info: &GlyphInfo) -> MaySkip {
        if !self.ctx.check_glyph_property(info, self.lookup_props) {
            return MaySkip::Yes;
        }

        if info.is_default_ignorable()
            && (self.ignore_zwnj || !info.is_zwnj())
            && (self.ignore_zwj || !info.is_zwj())
            && (self.ignore_hidden || !info.is_hidden())
        {
            return MaySkip::Maybe;
        }

        MaySkip::No
    }
}

pub(crate) fn match_input<F: ?Sized + Fn(u16, u16) -> bool>(
    ctx: &ApplyContext,
    input_len: u16,
    match_func: &F,
    end_position: &mut usize,
    match_positions: &mut MatchPositions,
    total_component_count: Option<&mut u8>,
) -> bool {
    #[derive(PartialEq)]
    enum LigBase {
        NotChecked,
        MayNotSkip,
        MaySkip,
    }

    let count = usize::from(input_len) + 1;
    if count > MAX_CONTEXT_LENGTH {
        return false;
    }

    let mut iter = SkippingIterator::with_matching(ctx, ctx.buffer.idx, false, match_func);
    iter.set_glyph_data(0);

    let first = ctx.buffer.cur(0);
    let first_lig_id = first.lig_id();
    let first_lig_comp = first.lig_comp();
    let mut total_count = 0u8;
    let mut ligbase = LigBase::NotChecked;

    for position in &mut match_positions[1..count] {
        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            *end_position = unsafe_to;
            return false;
        }

        *position = iter.index() as u32;

        let this = &ctx.buffer.info[iter.index()];
        let this_lig_id = this.lig_id();
        let this_lig_comp = this.lig_comp();

        if first_lig_id != 0 && first_lig_comp != 0 {
            if first_lig_id != this_lig_id || first_lig_comp != this_lig_comp {
                if ligbase == LigBase::NotChecked {
                    let out = ctx.buffer.out_info();
                    let mut j = ctx.buffer.out_len;
                    let mut found = false;
                    while j > 0 && out[j - 1].lig_id() == first_lig_id {
                        if out[j - 1].lig_comp() == 0 {
                            j -= 1;
                            found = true;
                            break;
                        }
                        j -= 1;
                    }

                    ligbase = if found && iter.may_skip(&out[j]) == MaySkip::Yes {
                        LigBase::MaySkip
                    } else {
                        LigBase::MayNotSkip
                    };
                }

                if ligbase == LigBase::MayNotSkip {
                    return false;
                }
            }
        } else if this_lig_id != 0 && this_lig_comp != 0 && this_lig_id != first_lig_id {
            return false;
        }

        total_count = total_count.saturating_add(this.lig_num_comps());
    }

    *end_position = iter.index() + 1;

    if let Some(out) = total_component_count {
        total_count = total_count.saturating_add(first.lig_num_comps());
        *out = total_count;
    }

    match_positions[0] = ctx.buffer.idx as u32;
    true
}

pub(crate) fn match_backtrack<F: ?Sized + Fn(u16, u16) -> bool>(
    ctx: &ApplyContext,
    backtrack_len: u16,
    match_func: &F,
    match_start: &mut usize,
) -> bool {
    let mut iter =
        SkippingIterator::with_matching(ctx, ctx.buffer.backtrack_len(), true, match_func);
    iter.set_glyph_data(0);

    for _ in 0..backtrack_len {
        let mut unsafe_from = 0;
        if !iter.prev(Some(&mut unsafe_from)) {
            *match_start = unsafe_from;
            return false;
        }
    }

    *match_start = iter.index();
    true
}

pub(crate) fn match_lookahead<F: ?Sized + Fn(u16, u16) -> bool>(
    ctx: &ApplyContext,
    lookahead_len: u16,
    match_func: &F,
    start_index: usize,
    end_index: &mut usize,
) -> bool {
    debug_assert!(start_index >= 1);
    let mut iter = SkippingIterator::with_matching(ctx, start_index.max(1) - 1, true, match_func);
    iter.set_glyph_data(0);

    for _ in 0..lookahead_len {
        let mut unsafe_to = 0;
        if !iter.next(Some(&mut unsafe_to)) {
            *end_index = unsafe_to;
            return false;
        }
    }

    *end_index = iter.index() + 1;
    true
}

#[derive(Clone, Copy)]
struct SequenceLookupRecord {
    sequence_index: u16,
    lookup_list_index: u16,
}

#[derive(Clone, Copy)]
struct LookupRecords<'a> {
    data: &'a [u8],
    count: u16,
}

impl LookupRecords<'_> {
    fn get(&self, i: u16) -> Option<SequenceLookupRecord> {
        if i >= self.count {
            return None;
        }
        let r = window::<4>(self.data, i as usize * 4)?;
        Some(SequenceLookupRecord {
            sequence_index: u16::from_be_bytes([r[0], r[1]]),
            lookup_list_index: u16::from_be_bytes([r[2], r[3]]),
        })
    }
}

#[derive(Clone, Copy)]
pub(crate) struct U16Array<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) len: u16,
}

impl U16Array<'_> {
    pub(crate) fn get(&self, i: u16) -> Option<u16> {
        if i >= self.len {
            return None;
        }
        read_u16_be(self.data, i as usize * 2)
    }
}

fn replay_edits(match_positions: &mut MatchPositions, count: usize, edits: &[(usize, isize)]) -> usize {
    let mut live = count;
    for &(at, delta) in edits {
        let at = at as u32;
        if delta < 0 {
            for _ in 0..(-delta) {
                let mut w = 0;
                for r in 0..live {
                    let p = match_positions[r];
                    if p == at {
                        continue;
                    }
                    match_positions[w] = if p > at { p - 1 } else { p };
                    w += 1;
                }
                live = w;
            }
        } else {
            for p in match_positions.iter_mut().take(live).filter(|p| **p >= at) {
                *p = (*p as isize + delta) as u32;
            }
        }
    }
    live
}

fn apply_lookup(
    ctx: &mut ApplyContext,
    input_len: usize,
    match_positions: &mut MatchPositions,
    match_end: usize,
    lookups: LookupRecords,
) {
    let mut count = input_len + 1;

    let mut end: isize = {
        let backtrack_len = ctx.buffer.backtrack_len();
        let delta = backtrack_len as isize - ctx.buffer.idx as isize;
        for position in &mut match_positions[..count] {
            *position = (*position as isize + delta) as u32;
        }
        backtrack_len as isize + match_end as isize - ctx.buffer.idx as isize
    };

    for i in 0..lookups.count {
        if !ctx.buffer.successful {
            break;
        }
        let Some(record) = lookups.get(i) else { break };

        let idx = usize::from(record.sequence_index);
        if idx >= count {
            continue;
        }

        let orig_len = ctx.buffer.backtrack_len() + ctx.buffer.lookahead_len();

        if match_positions[idx] as usize >= orig_len {
            continue;
        }

        if !ctx.buffer.move_to(match_positions[idx] as usize) {
            break;
        }

        if ctx.buffer.max_ops <= 0 {
            break;
        }

        let mark = ctx.buffer.edit_journal.len();
        let was_recording = ctx.buffer.recording_edits;
        ctx.buffer.recording_edits = true;
        let recursed = ctx.recurse(record.lookup_list_index);
        ctx.buffer.recording_edits = was_recording;
        if !recursed {
            ctx.buffer.edit_journal.truncate(mark);
            continue;
        }

        let new_len = ctx.buffer.backtrack_len() + ctx.buffer.lookahead_len();
        let delta = new_len as isize - orig_len as isize;

        if delta == 0 && ctx.buffer.edit_journal.len() == mark {
            continue;
        }

        if delta > 0 {
            if delta as usize + count > MAX_CONTEXT_LENGTH {
                ctx.buffer.edit_journal.truncate(mark);
                break;
            }
            let next = idx + 1;
            match_positions.copy_within(next..count, next + delta as usize);
            count += delta as usize;
            for j in next..next + delta as usize {
                match_positions[j] = match_positions[j - 1] + 1;
            }
            for p in match_positions[next + delta as usize..count].iter_mut() {
                *p = (*p as isize + delta) as u32;
            }
        } else {
            count = replay_edits(match_positions, count, &ctx.buffer.edit_journal[mark..]);
        }
        ctx.buffer.edit_journal.truncate(mark);

        end += delta;
        if idx < count && end < match_positions[idx] as isize {
            end = match_positions[idx] as isize;
        }
    }

    debug_assert!(end >= 0);
    ctx.buffer.move_to(end.max(0) as usize);
}

fn match_glyph(glyph: u16, value: u16) -> bool {
    glyph == value
}

pub(crate) fn apply_context(ctx: &mut ApplyContext, subtable: &[u8]) -> bool {
    let Some(format) = read_u16_be(subtable, 0) else { return false };
    let glyph = ctx.buffer.cur(0).id as u16;

    match format {
        1 => {
            let Some(set) = coverage_indexed_set(subtable, glyph) else { return false };
            apply_rule_set(ctx, set, &match_glyph)
        }
        2 => {
            let Some(coverage) = offset_coverage(subtable, 2) else { return false };
            if !coverage.contains(glyph) {
                return false;
            }
            let Some(classes) = offset_class_def(subtable, 4) else { return false };
            let Some(set) = indexed_set(subtable, classes.class_of(glyph), 6, 8) else {
                return false;
            };
            apply_rule_set(ctx, set, &|g, value| classes.class_of(g) == value)
        }
        3 => {
            let Some(h) = window::<4>(subtable, 2) else { return false };
            let input_count = u16::from_be_bytes([h[0], h[1]]);
            let lookup_count = u16::from_be_bytes([h[2], h[3]]);
            if input_count == 0 {
                return false;
            }

            let input = CoverageList { data: subtable, at: 6, len: input_count };
            if !input.contains(0, glyph) {
                return false;
            }
            let Some(records) = subtable.get(6 + input_count as usize * 2..) else {
                return false;
            };
            let records = LookupRecords { data: records, count: lookup_count };

            let mut match_end = 0;
            let mut positions = new_match_positions();
            let f = |g: u16, index: u16| input.contains(index + 1, g);

            if match_input(ctx, input_count - 1, &f, &mut match_end, &mut positions, None) {
                let idx = ctx.buffer.idx;
                ctx.buffer.unsafe_to_break(idx, match_end);
                apply_lookup(ctx, usize::from(input_count) - 1, &mut positions, match_end, records);
                true
            } else {
                let idx = ctx.buffer.idx;
                ctx.buffer.unsafe_to_concat(idx, match_end);
                false
            }
        }
        _ => false,
    }
}

pub(crate) fn apply_chain_context(ctx: &mut ApplyContext, subtable: &[u8]) -> bool {
    let Some(format) = read_u16_be(subtable, 0) else { return false };
    let glyph = ctx.buffer.cur(0).id as u16;

    match format {
        1 => {
            let Some(set) = coverage_indexed_set(subtable, glyph) else { return false };
            apply_chain_rule_set(ctx, set, &match_glyph, &match_glyph, &match_glyph)
        }
        2 => {
            let Some(coverage) = offset_coverage(subtable, 2) else { return false };
            if !coverage.contains(glyph) {
                return false;
            }
            let Some(backtrack) = offset_class_def(subtable, 4) else { return false };
            let Some(input) = offset_class_def(subtable, 6) else { return false };
            let Some(lookahead) = offset_class_def(subtable, 8) else { return false };
            let Some(set) = indexed_set(subtable, input.class_of(glyph), 10, 12) else {
                return false;
            };
            apply_chain_rule_set(
                ctx,
                set,
                &|g, v| backtrack.class_of(g) == v,
                &|g, v| input.class_of(g) == v,
                &|g, v| lookahead.class_of(g) == v,
            )
        }
        3 => apply_chain_context_format3(ctx, subtable),
        _ => false,
    }
}

fn apply_chain_context_format3(ctx: &mut ApplyContext, subtable: &[u8]) -> bool {
    let Some(back_count) = read_u16_be(subtable, 2) else { return false };
    let back = CoverageList { data: subtable, at: 4, len: back_count };

    let at = 4 + back_count as usize * 2;
    let Some(input_count) = read_u16_be(subtable, at) else { return false };
    if input_count == 0 {
        return false;
    }
    let input = CoverageList { data: subtable, at: at + 2, len: input_count };

    let at = at + 2 + input_count as usize * 2;
    let Some(ahead_count) = read_u16_be(subtable, at) else { return false };
    let ahead = CoverageList { data: subtable, at: at + 2, len: ahead_count };

    let at = at + 2 + ahead_count as usize * 2;
    let Some(lookup_count) = read_u16_be(subtable, at) else { return false };
    let Some(records) = subtable.get(at + 2..) else { return false };
    let records = LookupRecords { data: records, count: lookup_count };

    let Some(first) = input.get(0) else { return false };
    if !first.contains(ctx.buffer.cur(0).id as u16) {
        return false;
    }

    chain_match_and_apply(
        ctx,
        input_count - 1,
        &|g, i| input.contains(i + 1, g),
        back_count,
        &|g, i| back.contains(i, g),
        ahead_count,
        &|g, i| ahead.contains(i, g),
        records,
    )
}

#[derive(Clone, Copy)]
pub(crate) struct CoverageList<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) at: usize,
    pub(crate) len: u16,
}

impl<'a> CoverageList<'a> {
    pub(crate) fn get(&self, i: u16) -> Option<Coverage<'a>> {
        if i >= self.len {
            return None;
        }
        let off = read_u16_be(self.data, self.at + i as usize * 2)? as usize;
        Coverage::new(self.data.get(off..)?)
    }

    pub(crate) fn contains(&self, i: u16, glyph: u16) -> bool {
        self.get(i).is_some_and(|c| c.contains(glyph))
    }
}

pub(crate) fn offset_coverage_at(subtable: &[u8], off: u16) -> Option<Coverage<'_>> {
    Coverage::new(subtable.get(off as usize..)?)
}

pub(crate) fn offset_coverage(subtable: &[u8], at: usize) -> Option<Coverage<'_>> {
    let off = read_u16_be(subtable, at)? as usize;
    Coverage::new(subtable.get(off..)?)
}

pub(crate) fn offset_class_def(subtable: &[u8], at: usize) -> Option<ClassDef<'_>> {
    let off = read_u16_be(subtable, at)? as usize;
    if off == 0 {
        return Some(ClassDef::all_class_zero());
    }
    ClassDef::new(subtable.get(off..)?)
}

fn coverage_indexed_set(subtable: &[u8], glyph: u16) -> Option<&[u8]> {
    let index = offset_coverage(subtable, 2)?.index_of(glyph)?;
    indexed_set(subtable, index, 4, 6)
}

fn indexed_set(subtable: &[u8], index: u16, count_at: usize, offsets_at: usize) -> Option<&[u8]> {
    if index >= read_u16_be(subtable, count_at)? {
        return None;
    }
    let off = read_u16_be(subtable, offsets_at + index as usize * 2)? as usize;
    if off == 0 {
        return None;
    }
    subtable.get(off..)
}

fn apply_rule_set<F: ?Sized + Fn(u16, u16) -> bool>(ctx: &mut ApplyContext, set: &[u8], match_func: &F) -> bool {
    let Some(count) = read_u16_be(set, 0) else { return false };
    for i in 0..count {
        let Some(off) = read_u16_be(set, 2 + i as usize * 2) else { break };
        let Some(rule) = set.get(off as usize..) else { continue };
        if apply_rule(ctx, rule, match_func) {
            return true;
        }
    }
    false
}

fn apply_rule<F: ?Sized + Fn(u16, u16) -> bool>(ctx: &mut ApplyContext, rule: &[u8], match_func: &F) -> bool {
    let Some(h) = window::<4>(rule, 0) else { return false };
    let glyph_count = u16::from_be_bytes([h[0], h[1]]);
    let lookup_count = u16::from_be_bytes([h[2], h[3]]);
    if glyph_count == 0 {
        return false;
    }

    let input_len = glyph_count - 1;
    let Some(values) = rule.get(4..) else { return false };
    let input = U16Array { data: values, len: input_len };
    let Some(records) = rule.get(4 + input_len as usize * 2..) else { return false };
    let records = LookupRecords { data: records, count: lookup_count };

    let f = |glyph: u16, index: u16| input.get(index).is_some_and(|v| match_func(glyph, v));

    let mut match_end = 0;
    let mut positions = new_match_positions();

    if match_input(ctx, input_len, &f, &mut match_end, &mut positions, None) {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_break(idx, match_end);
        apply_lookup(ctx, usize::from(input_len), &mut positions, match_end, records);
        true
    } else {
        false
    }
}

fn apply_chain_rule_set<B: ?Sized + Fn(u16, u16) -> bool, I: ?Sized + Fn(u16, u16) -> bool, A: ?Sized + Fn(u16, u16) -> bool>(
    ctx: &mut ApplyContext,
    set: &[u8],
    back_func: &B,
    input_func: &I,
    ahead_func: &A,
) -> bool {
    let Some(count) = read_u16_be(set, 0) else { return false };

    for i in 0..count {
        let Some(off) = read_u16_be(set, 2 + i as usize * 2) else { break };
        let Some(rule) = set.get(off as usize..) else { continue };
        if apply_chain_rule(ctx, rule, back_func, input_func, ahead_func) {
            return true;
        }
    }
    false
}

fn apply_chain_rule<B: ?Sized + Fn(u16, u16) -> bool, I: ?Sized + Fn(u16, u16) -> bool, A: ?Sized + Fn(u16, u16) -> bool>(
    ctx: &mut ApplyContext,
    rule: &[u8],
    back_func: &B,
    input_func: &I,
    ahead_func: &A,
) -> bool {
    let Some(back_count) = read_u16_be(rule, 0) else { return false };
    let Some(back_values) = rule.get(2..) else { return false };
    let back = U16Array { data: back_values, len: back_count };

    let at = 2 + back_count as usize * 2;
    let Some(glyph_count) = read_u16_be(rule, at) else { return false };
    if glyph_count == 0 {
        return false;
    }
    let input_len = glyph_count - 1;
    let Some(input_values) = rule.get(at + 2..) else { return false };
    let input = U16Array { data: input_values, len: input_len };

    let at = at + 2 + input_len as usize * 2;
    let Some(ahead_count) = read_u16_be(rule, at) else { return false };
    let Some(ahead_values) = rule.get(at + 2..) else { return false };
    let ahead = U16Array { data: ahead_values, len: ahead_count };

    let at = at + 2 + ahead_count as usize * 2;
    let Some(lookup_count) = read_u16_be(rule, at) else { return false };
    let Some(records) = rule.get(at + 2..) else { return false };
    let records = LookupRecords { data: records, count: lookup_count };

    chain_match_and_apply(
        ctx,
        input_len,
        &|g, i| input.get(i).is_some_and(|v| input_func(g, v)),
        back_count,
        &|g, i| back.get(i).is_some_and(|v| back_func(g, v)),
        ahead_count,
        &|g, i| ahead.get(i).is_some_and(|v| ahead_func(g, v)),
        records,
    )
}

#[allow(clippy::too_many_arguments, reason = "one call site each; bundling would only add a struct")]
fn chain_match_and_apply<B: ?Sized + Fn(u16, u16) -> bool, I: ?Sized + Fn(u16, u16) -> bool, A: ?Sized + Fn(u16, u16) -> bool>(
    ctx: &mut ApplyContext,
    input_len: u16,
    input_func: &I,
    back_len: u16,
    back_func: &B,
    ahead_len: u16,
    ahead_func: &A,
    records: LookupRecords,
) -> bool {
    let mut end_index = ctx.buffer.idx;
    let mut match_end = 0;
    let mut positions = new_match_positions();

    let input_matches =
        match_input(ctx, input_len, input_func, &mut match_end, &mut positions, None);
    if input_matches {
        end_index = match_end;
    }

    if !(input_matches
        && match_lookahead(ctx, ahead_len, ahead_func, match_end, &mut end_index))
    {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat(idx, end_index);
        return false;
    }

    let mut start_index = ctx.buffer.out_len;
    if !match_backtrack(ctx, back_len, back_func, &mut start_index) {
        ctx.buffer.unsafe_to_concat_from_outbuffer(start_index, end_index);
        return false;
    }

    ctx.buffer.unsafe_to_break_from_outbuffer(start_index, end_index);
    apply_lookup(ctx, usize::from(input_len), &mut positions, match_end, records);
    true
}

pub(crate) fn ligate_input(
    ctx: &mut ApplyContext,
    count: usize,
    match_positions: &MatchPositions,
    match_end: usize,
    total_component_count: u8,
    lig_glyph: u16,
) {
    let idx = ctx.buffer.idx;
    ctx.buffer.merge_clusters(idx, match_end);

    let mut is_base_ligature = ctx.buffer.info[match_positions[0] as usize].is_base_glyph();
    let mut is_mark_ligature = ctx.buffer.info[match_positions[0] as usize].is_mark();
    for &pos in &match_positions[1..count] {
        if !ctx.buffer.info[pos as usize].is_mark() {
            is_base_ligature = false;
            is_mark_ligature = false;
        }
    }

    let is_ligature = !is_base_ligature && !is_mark_ligature;
    let class = if is_ligature { glyph_props::LIGATURE } else { 0 };
    let lig_id = if is_ligature { ctx.buffer.allocate_lig_id() } else { 0 };

    let first = ctx.buffer.cur_mut(0);
    let mut last_lig_id = first.lig_id();
    let mut last_num_comps = first.lig_num_comps();
    let mut comps_so_far = last_num_comps;

    if is_ligature {
        first.set_lig_props_for_ligature(lig_id, total_component_count);
        if first.general_category() == GeneralCategory::NonspacingMark as u16 {
            first.set_general_category(GeneralCategory::OtherLetter as u16);
        }
    }

    ctx.replace_glyph_with_ligature(lig_glyph, class);

    for &pos in &match_positions[1..count] {
        let pos = pos as usize;
        while ctx.buffer.idx < pos && ctx.buffer.successful {
            if is_ligature {
                let cur = ctx.buffer.cur_mut(0);
                let this_comp = if cur.lig_comp() == 0 { last_num_comps } else { cur.lig_comp() };
                debug_assert!(comps_so_far >= last_num_comps);
                let new_comp =
                    comps_so_far.saturating_sub(last_num_comps) + this_comp.min(last_num_comps);
                cur.set_lig_props_for_mark(lig_id, new_comp);
            }
            ctx.buffer.next_glyph();
        }

        let cur = ctx.buffer.cur(0);
        last_lig_id = cur.lig_id();
        last_num_comps = cur.lig_num_comps_in_ligation();
        comps_so_far += last_num_comps;

        ctx.buffer.idx += 1;
    }

    if !is_mark_ligature && last_lig_id != 0 {
        for i in ctx.buffer.idx..ctx.buffer.len {
            let info = &mut ctx.buffer.info[i];
            if last_lig_id != info.lig_id() {
                break;
            }
            let this_comp = info.lig_comp();
            if this_comp == 0 {
                break;
            }
            debug_assert!(comps_so_far >= last_num_comps);
            let new_comp =
                comps_so_far.saturating_sub(last_num_comps) + this_comp.min(last_num_comps);
            info.set_lig_props_for_mark(lig_id, new_comp);
        }
    }
}

pub(crate) fn apply_string(
    ctx: &mut ApplyContext,
    index: u16,
    in_place: bool,
    is_reverse: bool,
) -> bool {
    if ctx.buffer.is_empty() || ctx.lookup_mask() == 0 {
        return false;
    }

    let Some(resolved) = ctx.table().and_then(|t| t.lookup(index)) else {
        return false;
    };
    ctx.lookup_props = resolved.props();
    ctx.lookup = Some((index, resolved, ctx.lookup_props));

    if is_reverse {
        debug_assert!(!ctx.buffer.have_output);
        ctx.buffer.idx = ctx.buffer.len - 1;
        apply_backward(ctx, index)
    } else {
        if !in_place {
            ctx.buffer.clear_output();
        }
        ctx.buffer.idx = 0;
        let applied = apply_forward(ctx, index);
        if !in_place {
            ctx.buffer.sync();
        }
        applied
    }
}

pub(crate) fn apply_forward(ctx: &mut ApplyContext, index: u16) -> bool {
    let mut ret = false;
    while ctx.buffer.idx < ctx.buffer.len && ctx.buffer.successful {
        let start = ctx.buffer.idx;
        let len = ctx.buffer.len;

        let budget = ctx.buffer.max_ops.max(0) as usize;
        let window = (len - start).min(budget);
        let mask = ctx.lookup_mask();
        let props = ctx.lookup_props;
        let digest = ctx.glyph_digest.as_ref();

        let hit = ctx.buffer.info[start..start + window].iter().position(|cur| {
            digest.is_none_or(|d| d.may_have(cur.id as u16))
                && (cur.mask & mask) != 0
                && ctx.check_glyph_property(cur, props)
        });

        let (j, exhausted) = match hit {
            Some(k) => {
                ctx.buffer.max_ops -= (k + 1) as i32;
                (start + k, false)
            }
            None if window < len - start => {
                ctx.buffer.max_ops -= window as i32 + 1;
                ctx.buffer.shaping_failed = true;
                (start + window, true)
            }
            None => {
                ctx.buffer.max_ops -= window as i32;
                (len, false)
            }
        };

        if j > start {
            ctx.buffer.next_glyphs(j - start);
        }
        if exhausted || ctx.buffer.idx >= ctx.buffer.len {
            break;
        }

        let before = (ctx.buffer.idx, ctx.buffer.out_len);

        if ctx.apply_lookup_index(index) {
            ret = true;
            if (ctx.buffer.idx, ctx.buffer.out_len) == before {
                ctx.buffer.next_glyph();
            }
        } else {
            ctx.buffer.next_glyph();
        }
    }
    ret
}

pub(crate) fn apply_backward(ctx: &mut ApplyContext, index: u16) -> bool {
    let mut ret = false;
    loop {
        ctx.buffer.max_ops -= 1;
        if ctx.buffer.max_ops < 0 {
            ctx.buffer.shaping_failed = true;
            break;
        }

        let cur = ctx.buffer.cur(0);
        let eligible = (cur.mask & ctx.lookup_mask()) != 0
            && ctx.glyph_digest.is_none_or(|d| d.may_have(cur.id as u16))
            && ctx.check_glyph_property(cur, ctx.lookup_props);

        ret |= eligible && ctx.apply_lookup_index(index);

        if ctx.buffer.idx == 0 {
            break;
        }
        ctx.buffer.idx -= 1;
    }
    ret
}
