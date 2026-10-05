use alloc::vec::Vec;

use super::apply::{
    apply_chain_context, apply_context, offset_coverage_at, ApplyContext, Match,
    SkippingIterator,
};
use crate::daecore::daeshaper::buffer::{scratch_flags, Buffer, Direction, GlyphPosition};
use crate::daecore::daeshaper::face::Face;
use super::{lookup_flags, resolve_extension, value_record_len, Lookup};
use crate::daecore::daetype::decoder::read_u16_be;

pub(crate) const SINGLE: u16 = 1;
pub(crate) const PAIR: u16 = 2;
pub(crate) const CURSIVE: u16 = 3;
pub(crate) const MARK_BASE: u16 = 4;
pub(crate) const MARK_LIG: u16 = 5;
pub(crate) const MARK_MARK: u16 = 6;
pub(crate) const CONTEXT: u16 = 7;
pub(crate) const CHAIN_CONTEXT: u16 = 8;
pub(crate) const EXTENSION: u16 = 9;

pub(crate) const IN_PLACE: bool = true;

pub(crate) mod attach_type {
    pub(crate) const MARK: u8 = 1;
    pub(crate) const CURSIVE: u8 = 2;
}

enum Second {
    NotResolved,
    Found(usize, u16),
    Missing(usize),
}

pub(crate) fn apply(ctx: &mut ApplyContext, index: u16) -> bool {
    let Some(table) = ctx.gpos else { return false };
    let lookup = match ctx.lookup {
        Some((at, l, props)) if at == index => {
            ctx.lookup_props = props;
            l
        }
        _ => match table.lookup(index) {
            Some(l) => {
                ctx.lookup_props = l.props();
                l
            }
            None => return false,
        },
    };

    let already_cleared = lookup.subtable_count == 1 && ctx.glyph_digest.is_some();
    let cur = (!already_cleared).then(|| ctx.buffer.cur(0).id as u16);

    if lookup.kind == PAIR || lookup.kind == EXTENSION {
        let mut second = Second::NotResolved;
        for i in 0..lookup.subtable_count {
            if cur.is_some_and(|g| !ctx.subtable_may_have(i as usize, g)) {
                continue;
            }
            let Some(data) = lookup.subtable(i) else { continue };
            let Some((kind, data)) = resolve_extension(data, lookup.kind, EXTENSION) else {
                continue;
            };
            ctx.subtable = i as usize;
            let applied = if kind == PAIR {
                pair(ctx, data, &mut second)
            } else {
                apply_subtable(ctx, kind, data)
            };
            if applied {
                return true;
            }
        }
        return false;
    }

    for i in 0..lookup.subtable_count {
        if cur.is_some_and(|g| !ctx.subtable_may_have(i as usize, g)) {
            continue;
        }
        let Some(data) = lookup.subtable(i) else { continue };
        let Some((kind, data)) = resolve_extension(data, lookup.kind, EXTENSION) else { continue };
        ctx.subtable = i as usize;
        if apply_subtable(ctx, kind, data) {
            return true;
        }
    }
    false
}

fn resolve_second(ctx: &ApplyContext, second: &mut Second) -> Result<(usize, u16), usize> {
    if let Second::NotResolved = second {
        let mut iter = SkippingIterator::new(ctx, ctx.buffer.idx, false);
        let mut unsafe_to = 0;
        *second = if iter.next(Some(&mut unsafe_to)) {
            let at = iter.index();
            Second::Found(at, ctx.buffer.info[at].id as u16)
        } else {
            Second::Missing(unsafe_to)
        };
    }
    match *second {
        Second::Found(at, glyph) => Ok((at, glyph)),
        Second::Missing(unsafe_to) => Err(unsafe_to),
        Second::NotResolved => unreachable!("just resolved above"),
    }
}

pub(crate) fn is_reverse(_: &Lookup) -> bool {
    false
}

fn apply_subtable(ctx: &mut ApplyContext, kind: u16, data: &[u8]) -> bool {
    match kind {
        SINGLE => single(ctx, data),
        CURSIVE => cursive(ctx, data),
        MARK_BASE => mark_base(ctx, data),
        MARK_LIG => mark_ligature(ctx, data),
        MARK_MARK => mark_mark(ctx, data),
        CONTEXT => apply_context(ctx, data),
        CHAIN_CONTEXT => apply_chain_context(ctx, data),
        _ => false,
    }
}

mod value_format {
    pub(super) const X_PLACEMENT: u16 = 0x0001;
    pub(super) const Y_PLACEMENT: u16 = 0x0002;
    pub(super) const X_ADVANCE: u16 = 0x0004;
    pub(super) const Y_ADVANCE: u16 = 0x0008;
    pub(super) const X_PLACEMENT_DEVICE: u16 = 0x0010;
    pub(super) const Y_PLACEMENT_DEVICE: u16 = 0x0020;
    pub(super) const X_ADVANCE_DEVICE: u16 = 0x0040;
    pub(super) const Y_ADVANCE_DEVICE: u16 = 0x0080;
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct ValueRecord {
    x_placement: i16,
    y_placement: i16,
    x_advance: i16,
    y_advance: i16,
    x_placement_device: u16,
    y_placement_device: u16,
    x_advance_device: u16,
    y_advance_device: u16,
}

impl ValueRecord {
    fn parse(data: &[u8], at: usize, format: u16) -> ValueRecord {
        let mut v = ValueRecord::default();

        let rec = data.get(at..).map_or(&[][..], |r| &r[..value_record_len(format).min(r.len())]);
        let (fields, _) = rec.as_chunks::<2>();
        let mut fields = fields.iter().copied().map(u16::from_be_bytes);
        let mut take = || fields.next().unwrap_or(0);

        if format & value_format::X_PLACEMENT != 0 {
            v.x_placement = take() as i16;
        }
        if format & value_format::Y_PLACEMENT != 0 {
            v.y_placement = take() as i16;
        }
        if format & value_format::X_ADVANCE != 0 {
            v.x_advance = take() as i16;
        }
        if format & value_format::Y_ADVANCE != 0 {
            v.y_advance = take() as i16;
        }
        if format & value_format::X_PLACEMENT_DEVICE != 0 {
            v.x_placement_device = take();
        }
        if format & value_format::Y_PLACEMENT_DEVICE != 0 {
            v.y_placement_device = take();
        }
        if format & value_format::X_ADVANCE_DEVICE != 0 {
            v.x_advance_device = take();
        }
        if format & value_format::Y_ADVANCE_DEVICE != 0 {
            v.y_advance_device = take();
        }

        v
    }

    fn apply(&self, ctx: &mut ApplyContext, subtable: &[u8], idx: usize) -> bool {
        let horizontal = ctx.buffer.direction.is_horizontal();
        let deltas = ctx.face.uses_device_tables().then(|| {
            [
                device_delta(ctx.face, subtable, self.x_placement_device),
                device_delta(ctx.face, subtable, self.y_placement_device),
                device_delta(ctx.face, subtable, self.x_advance_device),
                device_delta(ctx.face, subtable, self.y_advance_device),
            ]
        });

        let pos = &mut ctx.buffer.pos[idx];
        let mut worked = false;

        if self.x_placement != 0 {
            pos.x_offset += i32::from(self.x_placement);
            worked = true;
        }
        if self.y_placement != 0 {
            pos.y_offset += i32::from(self.y_placement);
            worked = true;
        }
        if self.x_advance != 0 && horizontal {
            pos.x_advance += i32::from(self.x_advance);
            worked = true;
        }
        if self.y_advance != 0 && !horizontal {
            pos.y_advance -= i32::from(self.y_advance);
            worked = true;
        }

        let Some(deltas) = deltas else { return worked };
        if self.x_placement_device != 0 {
            pos.x_offset += deltas[0];
            worked = true;
        }
        if self.y_placement_device != 0 {
            pos.y_offset += deltas[1];
            worked = true;
        }
        if self.x_advance_device != 0 && horizontal {
            pos.x_advance += deltas[2];
            worked = true;
        }
        if self.y_advance_device != 0 && !horizontal {
            pos.y_advance -= deltas[3];
            worked = true;
        }

        worked
    }
}

fn device_delta(face: &Face, owner: &[u8], off: u16) -> i32 {
    if off == 0 {
        return 0;
    }
    let Some(d) = owner.get(off as usize..) else { return 0 };
    if read_u16_be(d, 4) != Some(0x8000) {
        return 0;
    }
    let (Some(outer), Some(inner)) = (read_u16_be(d, 0), read_u16_be(d, 2)) else { return 0 };
    face.variation_delta(outer, inner)
}

fn anchor(face: &Face, owner: &[u8], off: u16) -> Option<(i32, i32)> {
    if off == 0 {
        return None;
    }
    let d = owner.get(off as usize..)?;
    let head: &[u8; 6] = d.get(..6)?.try_into().ok()?;
    let format = u16::from_be_bytes([head[0], head[1]]);
    let mut x = i32::from(i16::from_be_bytes([head[2], head[3]]));
    let mut y = i32::from(i16::from_be_bytes([head[4], head[5]]));

    if format == 3 && face.uses_device_tables() {
        x += device_delta(face, d, read_u16_be(d, 6).unwrap_or(0));
        y += device_delta(face, d, read_u16_be(d, 8).unwrap_or(0));
    }

    Some((x, y))
}

#[derive(Clone, Copy)]
struct AnchorMatrix<'a> {
    data: &'a [u8],
    rows: u16,
    cols: u16,
}

impl<'a> AnchorMatrix<'a> {
    fn new(data: &'a [u8], cols: u16) -> Option<Self> {
        Some(AnchorMatrix { data, rows: read_u16_be(data, 0)?, cols })
    }

    fn get(&self, face: &Face, row: u16, col: u16) -> Option<(i32, i32)> {
        if row >= self.rows || col >= self.cols {
            return None;
        }
        let at = (row as usize)
            .checked_mul(self.cols as usize)
            .and_then(|i| i.checked_add(col as usize))
            .and_then(|i| i.checked_mul(2))
            .and_then(|i| i.checked_add(2))?;
        anchor(face, self.data, read_u16_be(self.data, at)?)
    }
}

fn mark_record(marks: &[u8], index: u16) -> Option<(u16, u16)> {
    if index >= read_u16_be(marks, 0)? {
        return None;
    }
    let at = 2 + index as usize * 4;
    let rec: &[u8; 4] = marks.get(at..at + 4)?.try_into().ok()?;
    Some((u16::from_be_bytes([rec[0], rec[1]]), u16::from_be_bytes([rec[2], rec[3]])))
}

fn offset_slice_at(data: &[u8], off: u16) -> Option<&[u8]> {
    if off == 0 {
        return None;
    }
    data.get(off as usize..)
}

struct MarkHead {
    mark_cov: u16,
    other_cov: u16,
    class_count: u16,
    mark_array: u16,
    other_array: u16,
}

fn mark_head(data: &[u8]) -> Option<MarkHead> {
    let h = crate::daecore::daetype::decoder::window::<12>(data, 0)?;
    let at = |i: usize| u16::from_be_bytes([h[i], h[i + 1]]);
    if at(0) != 1 {
        return None;
    }
    Some(MarkHead {
        mark_cov: at(2),
        other_cov: at(4),
        class_count: at(6),
        mark_array: at(8),
        other_array: at(10),
    })
}

fn offset_slice(data: &[u8], at: usize) -> Option<&[u8]> {
    let off = read_u16_be(data, at)? as usize;
    if off == 0 {
        return None;
    }
    data.get(off..)
}

fn cross_offset(pos: &[GlyphPosition], at: usize, direction: Direction) -> i32 {
    let horizontal = direction.is_horizontal();
    let axis = |p: &GlyphPosition| if horizontal { p.y_offset } else { p.x_offset };

    let mut at = at;
    let mut offset = axis(&pos[at]);
    while pos[at].attach_type & attach_type::CURSIVE != 0 {
        let chain = pos[at].attach_chain;
        if chain == 0 {
            break;
        }
        let parent = at as isize + chain as isize;
        if parent < 0 || parent as usize >= pos.len() {
            break;
        }
        at = parent as usize;
        offset = offset.saturating_add(axis(&pos[at]));
    }
    offset
}

fn attach_mark(
    ctx: &mut ApplyContext,
    marks: &[u8],
    anchors: AnchorMatrix,
    mark_index: u16,
    glyph_index: u16,
    to: usize,
) -> bool {
    let Some((mark_class, mark_anchor)) = mark_record(marks, mark_index) else { return false };
    let Some((mark_x, mark_y)) = anchor(ctx.face, marks, mark_anchor) else { return false };
    let Some((base_x, base_y)) = anchors.get(ctx.face, glyph_index, mark_class) else {
        return false;
    };

    let idx = ctx.buffer.idx;
    ctx.buffer.unsafe_to_break(to, idx + 1);

    let direction = ctx.buffer.direction;
    let base_offset = cross_offset(&ctx.buffer.pos, to, direction);

    let pos = ctx.buffer.cur_pos_mut();
    pos.x_offset = base_x - mark_x;
    pos.y_offset = base_y - mark_y;
    if direction.is_horizontal() {
        pos.y_offset = pos.y_offset.saturating_add(base_offset);
    } else {
        pos.x_offset = pos.x_offset.saturating_add(base_offset);
    }
    pos.attach_type = attach_type::MARK;
    // A mark records what it attached to rather than a final offset, and
    // `position_finish_offsets` resolves the chains at the end – a base can itself move after a
    // mark has already attached to it.
    pos.attach_chain = (to as isize - idx as isize) as i16;

    ctx.buffer.scratch_flags |= scratch_flags::HAS_GPOS_ATTACHMENT;
    ctx.buffer.idx += 1;
    true
}

fn single(ctx: &mut ApplyContext, data: &[u8]) -> bool {
    let glyph = ctx.buffer.cur(0).id as u16;
    let Some(format) = read_u16_be(data, 0) else { return false };
    let Some(coverage) = ctx.coverage(data, 2) else { return false };
    let Some(value_format) = read_u16_be(data, 4) else { return false };

    let record = match format {
        1 => {
            if !coverage.contains(glyph) {
                return false;
            }
            ValueRecord::parse(data, 6, value_format)
        }
        2 => {
            let Some(index) = coverage.index_of(glyph) else { return false };
            let Some(count) = read_u16_be(data, 6) else { return false };
            if index >= count {
                return false;
            }
            ValueRecord::parse(data, 8 + index as usize * value_record_len(value_format), value_format)
        }
        _ => return false,
    };

    let idx = ctx.buffer.idx;
    record.apply(ctx, data, idx);
    ctx.buffer.idx += 1;
    true
}

fn pair(ctx: &mut ApplyContext, data: &[u8], second_cache: &mut Second) -> bool {
    let first = ctx.buffer.cur(0).id as u16;
    let Some(head) = data.get(..8).and_then(|h| <&[u8; 8]>::try_from(h).ok()) else { return false };
    let format = u16::from_be_bytes([head[0], head[1]]);
    let Some(coverage) = ctx.coverage(data, 2) else { return false };
    let Some(first_index) = coverage.index_of(first) else { return false };
    let format1 = u16::from_be_bytes([head[4], head[5]]);
    let format2 = u16::from_be_bytes([head[6], head[7]]);

    let (second_pos, second) = match resolve_second(&*ctx, second_cache) {
        Ok(v) => v,
        Err(unsafe_to) => {
            let idx = ctx.buffer.idx;
            ctx.buffer.unsafe_to_concat(idx, unsafe_to);
            return false;
        }
    };

    // A format 1 record's devices are offsets from its PairSet; a format 2 record's from the subtable.
    let (records, parent) = match format {
        1 => {
            let Some(set) = offset_slice(data, 10 + first_index as usize * 2) else {
                return false;
            };
            let Some(r) = pair_set_lookup(set, second, format1, format2) else { return false };
            (r, set)
        }
        2 => {
            let (Some(class1), Some(class2)) =
                (ctx.class_def(data, 8), ctx.class_def(data, 10))
            else {
                return false;
            };
            let (Some(count1), Some(count2)) = (read_u16_be(data, 12), read_u16_be(data, 14)) else {
                return false;
            };
            let c1 = class1.class_of(first);
            let c2 = class2.class_of(second);
            match class_matrix_lookup(data, c1, c2, count1, count2, format1, format2) {
                Some(r) => (r, data),
                None => {
                    let idx = ctx.buffer.idx;
                    ctx.buffer.unsafe_to_concat(idx, second_pos + 1);
                    return false;
                }
            }
        }
        _ => return false,
    };

    let (r1, r2) = records;
    let has1 = value_record_len(format1) != 0;
    let has2 = value_record_len(format2) != 0;

    let idx = ctx.buffer.idx;
    let moved1 = has1 && r1.apply(ctx, parent, idx);
    let moved2 = has2 && r2.apply(ctx, parent, second_pos);

    if moved1 || moved2 {
        ctx.buffer.unsafe_to_break(idx, second_pos + 1);
    } else {
        ctx.buffer.unsafe_to_concat(idx, second_pos + 1);
    }

    let mut next = second_pos;
    if has2 {
        next += 1;
        ctx.buffer.unsafe_to_break(idx, next + 1);
    }
    ctx.buffer.idx = next;
    true
}

fn pair_set_lookup(
    set: &[u8],
    second: u16,
    format1: u16,
    format2: u16,
) -> Option<(ValueRecord, ValueRecord)> {
    let count = read_u16_be(set, 0)?;
    let stride = 2 + value_record_len(format1) + value_record_len(format2);

    let (mut lo, mut hi) = (0u16, count);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let at = 2 + mid as usize * stride;
        let glyph = read_u16_be(set, at)?;
        match glyph.cmp(&second) {
            core::cmp::Ordering::Less => lo = mid + 1,
            core::cmp::Ordering::Greater => hi = mid,
            core::cmp::Ordering::Equal => {
                return Some((
                    ValueRecord::parse(set, at + 2, format1),
                    ValueRecord::parse(set, at + 2 + value_record_len(format1), format2),
                ));
            }
        }
    }
    None
}

fn class_matrix_lookup(
    data: &[u8],
    class1: u16,
    class2: u16,
    count1: u16,
    count2: u16,
    format1: u16,
    format2: u16,
) -> Option<(ValueRecord, ValueRecord)> {
    if class1 >= count1 || class2 >= count2 {
        return None;
    }
    let len1 = value_record_len(format1);
    let stride = len1 + value_record_len(format2);
    let at = (class1 as usize)
        .checked_mul(count2 as usize)
        .and_then(|i| i.checked_add(class2 as usize))
        .and_then(|i| i.checked_mul(stride))
        .and_then(|i| i.checked_add(16))?;
    if at.checked_add(stride).is_none_or(|end| end > data.len()) {
        return None;
    }
    Some((ValueRecord::parse(data, at, format1), ValueRecord::parse(data, at + len1, format2)))
}

fn cursive(ctx: &mut ApplyContext, data: &[u8]) -> bool {
    if read_u16_be(data, 0) != Some(1) {
        return false;
    }
    let this = ctx.buffer.cur(0).id as u16;
    let Some(coverage) = ctx.coverage(data, 2) else { return false };
    let Some(index_this) = coverage.index_of(this) else { return false };
    let Some(count) = read_u16_be(data, 4) else { return false };
    if index_this >= count {
        return false;
    }
    let entry_off = read_u16_be(data, 6 + index_this as usize * 4).unwrap_or(0);
    let Some(entry_this) = anchor(ctx.face, data, entry_off) else { return false };

    let mut iter = SkippingIterator::new(&*ctx, ctx.buffer.idx, false);
    let mut unsafe_from = 0;
    if !iter.prev(Some(&mut unsafe_from)) {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat_from_outbuffer(unsafe_from, idx + 1);
        return false;
    }

    let i = iter.index();
    let prev = ctx.buffer.info[i].id as u16;
    let Some(index_prev) = coverage.index_of(prev) else { return false };
    if index_prev >= count {
        return false;
    }
    let exit_off = read_u16_be(data, 6 + index_prev as usize * 4 + 2).unwrap_or(0);
    let Some(exit_prev) = anchor(ctx.face, data, exit_off) else {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat_from_outbuffer(i, idx + 1);
        return false;
    };

    let (exit_x, exit_y) = exit_prev;
    let (entry_x, entry_y) = entry_this;
    let direction = ctx.buffer.direction;
    let j = ctx.buffer.idx;
    ctx.buffer.unsafe_to_break(i, j + 1);

    let pos = &mut ctx.buffer.pos;
    match direction {
        Direction::LeftToRight => {
            pos[i].x_advance = exit_x + pos[i].x_offset;
            let d = entry_x + pos[j].x_offset;
            pos[j].x_advance -= d;
            pos[j].x_offset -= d;
        }
        Direction::RightToLeft => {
            let d = exit_x + pos[i].x_offset;
            pos[i].x_advance -= d;
            pos[i].x_offset -= d;
            pos[j].x_advance = entry_x + pos[j].x_offset;
        }
        Direction::TopToBottom => {
            pos[i].y_advance = exit_y + pos[i].y_offset;
            let d = entry_y + pos[j].y_offset;
            pos[j].y_advance -= d;
            pos[j].y_offset -= d;
        }
        Direction::BottomToTop => {
            let d = exit_y + pos[i].y_offset;
            pos[i].y_advance -= d;
            pos[i].y_offset -= d;
            pos[j].y_advance = entry_y;
        }
    }

    let mut child = i;
    let mut parent = j;
    let mut x_offset = entry_x - exit_x;
    let mut y_offset = entry_y - exit_y;

    if ctx.lookup_props as u16 & lookup_flags::RIGHT_TO_LEFT == 0 {
        core::mem::swap(&mut child, &mut parent);
        x_offset = -x_offset;
        y_offset = -y_offset;
    }

    reverse_cursive_minor_offset(&mut ctx.buffer.pos, child, direction, parent);

    let pos = &mut ctx.buffer.pos;
    pos[child].attach_type = attach_type::CURSIVE;
    pos[child].attach_chain = (parent as isize - child as isize) as i16;

    if direction.is_horizontal() {
        pos[child].y_offset = y_offset;
    } else {
        pos[child].x_offset = x_offset;
    }

    if pos[parent].attach_chain == -pos[child].attach_chain {
        pos[parent].attach_chain = 0;
        if direction.is_horizontal() {
            pos[parent].y_offset = 0;
        } else {
            pos[parent].x_offset = 0;
        }
    }

    ctx.buffer.scratch_flags |= scratch_flags::HAS_GPOS_ATTACHMENT;
    ctx.buffer.idx += 1;
    true
}

fn reverse_cursive_minor_offset(
    pos: &mut [GlyphPosition],
    start: usize,
    direction: Direction,
    new_parent: usize,
) {
    let mut path: Vec<(usize, usize, i16, u8)> = Vec::new();
    let mut i = start;

    loop {
        let chain = pos[i].attach_chain;
        let kind = pos[i].attach_type;
        if chain == 0 || kind & attach_type::CURSIVE == 0 {
            break;
        }
        pos[i].attach_chain = 0;

        let j = (i as isize + chain as isize) as usize;
        if j == new_parent || j >= pos.len() {
            break;
        }
        path.push((i, j, chain, kind));
        i = j;
    }

    while let Some((i, j, chain, kind)) = path.pop() {
        if direction.is_horizontal() {
            pos[j].y_offset = -pos[i].y_offset;
        } else {
            pos[j].x_offset = -pos[i].x_offset;
        }
        pos[j].attach_chain = -chain;
        pos[j].attach_type = kind;
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Attaching {
    Base,
    Ligature,
}

fn find_base(ctx: &mut ApplyContext, kind: Attaching, coverage: &super::Coverage) -> i32 {
    if ctx.last_base_until > ctx.buffer.idx as u32 {
        ctx.last_base_until = 0;
        ctx.last_base = -1;
    }

    let mut found = ctx.last_base;
    {
        let mut iter = SkippingIterator::new(&*ctx, 0, false);
        iter.set_lookup_props(u32::from(lookup_flags::IGNORE_MARKS));

        let mut j = ctx.buffer.idx;
        while j > ctx.last_base_until as usize {
            let mut m = iter.match_(&ctx.buffer.info[j - 1]);
            if m == Match::Yes {
                let acceptable = match kind {
                    Attaching::Base => accept(ctx.buffer, j - 1),
                    Attaching::Ligature => accept_ligature(ctx.buffer, j - 1),
                };
                if !acceptable && !coverage.contains(ctx.buffer.info[j - 1].id as u16) {
                    m = Match::Skip;
                }
            }
            if m == Match::Yes {
                found = j as i32 - 1;
                break;
            }
            j -= 1;
        }
    }

    ctx.last_base = found;
    ctx.last_base_until = ctx.buffer.idx as u32;
    found
}

fn accept(buffer: &Buffer, idx: usize) -> bool {
    let info = &buffer.info;
    !info[idx].multiplied()
        || info[idx].lig_comp() == 0
        || idx == 0
        || info[idx - 1].is_mark()
        || !info[idx - 1].multiplied()
        || info[idx].lig_id() != info[idx - 1].lig_id()
        || info[idx].lig_comp() != info[idx - 1].lig_comp() + 1
}

fn accept_ligature(buffer: &Buffer, idx: usize) -> bool {
    !buffer.info[idx].multiplied() || buffer.info[idx].lig_comp() == 0
}

fn mark_base(ctx: &mut ApplyContext, data: &[u8]) -> bool {
    let Some(head) = mark_head(data) else { return false };
    let mark_glyph = ctx.buffer.cur(0).id as u16;
    let Some(mark_cov) = ctx.coverage_resolved(data, head.mark_cov, true) else { return false };
    let Some(mark_index) = mark_cov.index_of(mark_glyph) else { return false };
    let Some(base_cov) = offset_coverage_at(data, head.other_cov) else { return false };
    let class_count = head.class_count;
    let Some(marks) = offset_slice_at(data, head.mark_array) else { return false };
    let Some(bases) = offset_slice_at(data, head.other_array) else { return false };
    let Some(anchors) = AnchorMatrix::new(bases, class_count) else { return false };

    let base = find_base(ctx, Attaching::Base, &base_cov);
    if base == -1 {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat_from_outbuffer(0, idx + 1);
        return false;
    }

    let at = base as usize;
    let Some(base_index) = base_cov.index_of(ctx.buffer.info[at].id as u16) else {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat_from_outbuffer(at, idx + 1);
        return false;
    };

    attach_mark(ctx, marks, anchors, mark_index, base_index, at)
}

fn mark_ligature(ctx: &mut ApplyContext, data: &[u8]) -> bool {
    let Some(head) = mark_head(data) else { return false };
    let mark_glyph = ctx.buffer.cur(0).id as u16;
    let Some(mark_cov) = ctx.coverage_resolved(data, head.mark_cov, true) else { return false };
    let Some(mark_index) = mark_cov.index_of(mark_glyph) else { return false };
    let Some(lig_cov) = offset_coverage_at(data, head.other_cov) else { return false };
    let class_count = head.class_count;
    let Some(marks) = offset_slice_at(data, head.mark_array) else { return false };
    let Some(ligatures) = offset_slice_at(data, head.other_array) else { return false };

    let base = find_base(ctx, Attaching::Ligature, &lig_cov);
    if base == -1 {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat_from_outbuffer(0, idx + 1);
        return false;
    }

    let at = base as usize;
    let Some(lig_index) = lig_cov.index_of(ctx.buffer.info[at].id as u16) else {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat_from_outbuffer(at, idx + 1);
        return false;
    };

    if lig_index >= read_u16_be(ligatures, 0).unwrap_or(0) {
        return false;
    }
    let Some(attach) = offset_slice(ligatures, 2 + lig_index as usize * 2) else { return false };
    let Some(anchors) = AnchorMatrix::new(attach, class_count) else { return false };

    let comp_count = anchors.rows;
    if comp_count == 0 {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat_from_outbuffer(at, idx + 1);
        return false;
    }

    let lig_id = ctx.buffer.info[at].lig_id();
    let mark_id = ctx.buffer.cur(0).lig_id();
    let mark_comp = u16::from(ctx.buffer.cur(0).lig_comp());
    let matches = lig_id != 0 && lig_id == mark_id && mark_comp > 0;
    let comp_index = if matches { mark_comp.min(comp_count) } else { comp_count } - 1;

    attach_mark(ctx, marks, anchors, mark_index, comp_index, at)
}

fn mark_mark(ctx: &mut ApplyContext, data: &[u8]) -> bool {
    let Some(head) = mark_head(data) else { return false };
    let mark1_glyph = ctx.buffer.cur(0).id as u16;
    let Some(mark1_cov) = ctx.coverage_resolved(data, head.mark_cov, true) else { return false };
    let Some(mark1_index) = mark1_cov.index_of(mark1_glyph) else { return false };
    let Some(mark2_cov) = offset_coverage_at(data, head.other_cov) else { return false };
    let class_count = head.class_count;
    let Some(marks) = offset_slice_at(data, head.mark_array) else { return false };
    let Some(mark2s) = offset_slice_at(data, head.other_array) else { return false };
    let Some(anchors) = AnchorMatrix::new(mark2s, class_count) else { return false };

    let prev = {
        let mut iter = SkippingIterator::new(&*ctx, ctx.buffer.idx, false);
        iter.set_lookup_props(ctx.lookup_props & !u32::from(lookup_flags::IGNORE_FLAGS));
        let mut unsafe_from = 0;
        if !iter.prev(Some(&mut unsafe_from)) {
            let idx = ctx.buffer.idx;
            ctx.buffer.unsafe_to_concat_from_outbuffer(unsafe_from, idx + 1);
            return false;
        }
        iter.index()
    };

    if !ctx.buffer.info[prev].is_mark() {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat_from_outbuffer(prev, idx + 1);
        return false;
    }

    let id1 = ctx.buffer.cur(0).lig_id();
    let id2 = ctx.buffer.info[prev].lig_id();
    let comp1 = ctx.buffer.cur(0).lig_comp();
    let comp2 = ctx.buffer.info[prev].lig_comp();

    let matches = if id1 == id2 {
        id1 == 0 || comp1 == comp2
    } else {
        (id1 > 0 && comp1 == 0) || (id2 > 0 && comp2 == 0)
    };

    if !matches {
        let idx = ctx.buffer.idx;
        ctx.buffer.unsafe_to_concat_from_outbuffer(prev, idx + 1);
        return false;
    }

    let Some(mark2_index) = mark2_cov.index_of(ctx.buffer.info[prev].id as u16) else {
        return false;
    };

    attach_mark(ctx, marks, anchors, mark1_index, mark2_index, prev)
}

pub(crate) fn position_start(buffer: &mut Buffer) {
    let len = buffer.len;
    for pos in &mut buffer.pos[..len] {
        pos.attach_chain = 0;
        pos.attach_type = 0;
    }
}

pub(crate) fn position_finish_offsets(buffer: &mut Buffer) {
    if buffer.scratch_flags & scratch_flags::HAS_GPOS_ATTACHMENT == 0 {
        return;
    }

    let len = buffer.len;
    let direction = buffer.direction;
    let mut path: Vec<(usize, usize)> = Vec::new();

    let mut cum: Vec<(i64, i64)> = Vec::with_capacity(len + 1);
    let (mut sx, mut sy) = (0i64, 0i64);
    cum.push((0, 0));
    for p in &buffer.pos[..len] {
        sx += p.x_advance as i64;
        sy += p.y_advance as i64;
        cum.push((sx, sy));
    }
    let span = Advances { cum: &cum };

    for i in 0..len {
        propagate_attachment_offsets(&mut buffer.pos, len, i, direction, &mut path, span);
    }
}

#[derive(Clone, Copy)]
struct Advances<'a> {
    cum: &'a [(i64, i64)],
}

impl Advances<'_> {
    fn between(&self, a: usize, b: usize) -> (i64, i64) {
        let (xb, yb) = self.cum[b];
        let (xa, ya) = self.cum[a];
        (xb - xa, yb - ya)
    }
}

fn offset_by(offset: &mut i32, delta: i64) {
    *offset = (*offset as i64).saturating_add(delta).clamp(i32::MIN as i64, i32::MAX as i64) as i32;
}

fn propagate_attachment_offsets(
    pos: &mut [GlyphPosition],
    len: usize,
    start: usize,
    direction: Direction,
    path: &mut Vec<(usize, usize)>,
    span: Advances,
) {
    path.clear();
    let mut i = start;

    loop {
        let chain = pos[i].attach_chain;
        if chain == 0 {
            break;
        }
        pos[i].attach_chain = 0;

        let j = (i as isize + chain as isize) as usize;
        if j >= len {
            break;
        }
        path.push((i, j));
        i = j;
    }

    while let Some((i, j)) = path.pop() {
        match pos[i].attach_type {
            attach_type::MARK => {
                if direction.is_horizontal() {
                    let px = pos[j].x_offset;
                    offset_by(&mut pos[i].x_offset, i64::from(px));
                } else {
                    let py = pos[j].y_offset;
                    offset_by(&mut pos[i].y_offset, i64::from(py));
                }

                debug_assert!(j < i);
                let (dx, dy) = if direction.is_backward() {
                    span.between(j + 1, i + 1)
                } else {
                    let (x, y) = span.between(j, i);
                    (-x, -y)
                };
                offset_by(&mut pos[i].x_offset, dx);
                offset_by(&mut pos[i].y_offset, dy);
            }
            attach_type::CURSIVE => {
                if direction.is_horizontal() {
                    let py = pos[j].y_offset;
                    offset_by(&mut pos[i].y_offset, i64::from(py));
                } else {
                    let px = pos[j].x_offset;
                    offset_by(&mut pos[i].x_offset, i64::from(px));
                }
            }
            _ => {}
        }
    }
}
