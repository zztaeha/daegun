use super::apply::{ApplyContext, SkippingIterator};
use crate::daecore::daeshaper::buffer::{scratch_flags, Buffer, Mask};
use crate::daecore::daeshaper::face::Face;
use super::gpos::attach_type;
use super::map::TableIndex;
use super::lookup_flags;
use crate::daecore::daetype::decoder::{read_i16_be, read_u16_be, read_u32_be, window};

mod coverage {
    pub(super) const HORIZONTAL: u16 = 0x0001;
    pub(super) const MINIMUM: u16 = 0x0002;
    pub(super) const CROSS_STREAM: u16 = 0x0004;
}

#[derive(Clone, Copy)]
struct Subtable<'a> {
    data: &'a [u8],
    body: usize,
    format: u8,
    horizontal: bool,
    cross_stream: bool,
}

fn subtables(table: &[u8]) -> Option<(usize, u32, bool)> {
    let version = read_u16_be(table, 0)?;
    if version == 0 {
        Some((4, u32::from(read_u16_be(table, 2)?), false))
    } else if version == 1 && read_u16_be(table, 2) == Some(0) {
        Some((8, read_u32_be(table, 4)?, true))
    } else {
        None
    }
}

// The last OpenType subtable runs to the table's end, as HarfBuzz reads it: one of more than 10,920
// pairs overflows its 16-bit length, and fonts write it wrapped.
fn parse_subtable(table: &[u8], at: usize, apple: bool, last: bool) -> Option<(Subtable<'_>, usize)> {
    if apple {
        let h = window::<6>(table, at)?;
        let length = u32::from_be_bytes([h[0], h[1], h[2], h[3]]) as usize;
        let cov = u16::from_be_bytes([h[4], h[5]]);
        let format = (cov & 0x00FF) as u8;
        let flags = cov >> 8;
        let sub = Subtable {
            data: table.get(at..at + length.max(8))?,
            body: 8,
            format,
            horizontal: flags & 0x80 == 0,
            cross_stream: flags & 0x40 != 0,
        };
        Some((sub, at + length))
    } else {
        let h = window::<4>(table, at + 2)?;
        let length = u16::from_be_bytes([h[0], h[1]]) as usize;
        let cov = u16::from_be_bytes([h[2], h[3]]);
        if cov & coverage::MINIMUM != 0 {
            return Some((
                Subtable { data: &[], body: 0, format: 0xFF, horizontal: false, cross_stream: false },
                at + length.max(6),
            ));
        }
        let end = if last { table.len() } else { at + length.max(6) };
        let sub = Subtable {
            data: table.get(at..end)?,
            body: 6,
            format: (cov >> 8) as u8,
            horizontal: cov & coverage::HORIZONTAL != 0,
            cross_stream: cov & coverage::CROSS_STREAM != 0,
        };
        Some((sub, at + length))
    }
}

impl Subtable<'_> {
    fn kerning(&self, left: u16, right: u16) -> i32 {
        match self.format {
            0 => self.format0(left, right),
            2 => self.format2(left, right),
            _ => 0,
        }
    }

    fn format0(&self, left: u16, right: u16) -> i32 {
        let Some(count) = read_u16_be(self.data, self.body) else { return 0 };
        let want = (u32::from(left) << 16) | u32::from(right);

        let pairs = self.body + 8;
        let (mut lo, mut hi) = (0u16, count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let at = pairs + mid as usize * 6;
            let Some(pair) = read_u32_be(self.data, at) else { return 0 };
            match pair.cmp(&want) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => {
                    return i32::from(read_i16_be(self.data, at + 4).unwrap_or(0))
                }
            }
        }
        0
    }

    fn format2(&self, left: u16, right: u16) -> i32 {
        let Some(left_off) = read_u16_be(self.data, self.body + 2).map(usize::from) else {
            return 0;
        };
        let Some(right_off) = read_u16_be(self.data, self.body + 4).map(usize::from) else {
            return 0;
        };

        let Some(array_off) = read_u16_be(self.data, self.body + 6).map(usize::from) else {
            return 0;
        };

        let class = |off: usize, glyph: u16| -> usize {
            let Some(h) = window::<4>(self.data, off) else { return 0 };
            let first = u16::from_be_bytes([h[0], h[1]]);
            let count = u16::from_be_bytes([h[2], h[3]]);
            if glyph < first || glyph >= first.saturating_add(count) {
                return 0;
            }
            let at = off + 4 + usize::from(glyph - first) * 2;
            read_u16_be(self.data, at).map_or(0, usize::from)
        };

        let left_class = class(left_off, left);
        if left_class < array_off {
            return 0;
        }

        let at = left_class + class(right_off, right);
        i32::from(read_i16_be(self.data, at).unwrap_or(0))
    }
}

pub(crate) fn has_cross_stream(face: &Face) -> bool {
    let Some(table) = face.table("kern") else { return false };
    let Some((mut at, count, apple)) = subtables(table) else { return false };

    for i in 0..count {
        let Some((sub, next)) = parse_subtable(table, at, apple, i + 1 == count) else { return false };
        if next <= at {
            return false;
        }
        at = next;
        if sub.format != 0xFF && sub.cross_stream {
            return true;
        }
    }
    false
}

pub(crate) fn has_machine_kerning(face: &Face) -> bool {
    let Some(table) = face.table("kern") else { return false };
    let Some((mut at, count, apple)) = subtables(table) else { return false };

    for i in 0..count {
        let Some((sub, next)) = parse_subtable(table, at, apple, i + 1 == count) else { return false };
        if next <= at {
            return false;
        }
        at = next;
        if sub.format == 1 {
            return true;
        }
    }
    false
}

pub(crate) fn apply(face: &Face, buffer: &mut Buffer, kern_mask: Mask, requested: bool) {
    let Some(table) = face.table("kern") else { return };
    let Some((mut at, count, apple)) = subtables(table) else { return };

    let mut seen_cross_stream = false;

    for i in 0..count {
        let Some((sub, next)) = parse_subtable(table, at, apple, i + 1 == count) else { break };
        if next <= at {
            break;
        }
        at = next;

        if sub.format == 0xFF || buffer.direction.is_horizontal() != sub.horizontal {
            continue;
        }
        if !requested && !sub.cross_stream {
            continue;
        }

        if !seen_cross_stream && sub.cross_stream {
            seen_cross_stream = true;
            chain_for_cross_stream(buffer);
        }

        let reverse = buffer.direction.is_backward();
        if reverse {
            buffer.reverse();
        }
        kern_pairs(face, buffer, kern_mask, &sub);
        if reverse {
            buffer.reverse();
        }
    }
}

fn kern_pairs(face: &Face, buffer: &mut Buffer, kern_mask: Mask, sub: &Subtable) {
    let horizontal = buffer.direction.is_horizontal();
    let cross_stream = sub.cross_stream;

    let mut ctx = ApplyContext::new(TableIndex::Gpos, face, buffer, |_, _| false);
    ctx.set_lookup_mask(kern_mask);
    ctx.lookup_props = u32::from(lookup_flags::IGNORE_MARKS);

    let mut i = 0;
    while i < ctx.buffer.len {
        if ctx.buffer.info[i].mask & kern_mask == 0 {
            i += 1;
            continue;
        }

        let j = {
            let mut iter = SkippingIterator::new(&ctx, i, false);
            if !iter.next(None) {
                i += 1;
                continue;
            }
            iter.index()
        };

        let kern = sub.kerning(ctx.buffer.info[i].id as u16, ctx.buffer.info[j].id as u16);
        if kern != 0 {
            apply_pair(ctx.buffer, i, j, kern, horizontal, cross_stream);
            ctx.buffer.unsafe_to_break(i, j + 1);
        }

        i = j;
    }
}

pub(super) fn apply_pair(
    buffer: &mut Buffer,
    i: usize,
    j: usize,
    kern: i32,
    horizontal: bool,
    cross_stream: bool,
) {
    if cross_stream {
        if horizontal {
            buffer.pos[j].y_offset = kern;
        } else {
            buffer.pos[j].x_offset = kern;
        }
        buffer.scratch_flags |= scratch_flags::HAS_GPOS_ATTACHMENT;
        return;
    }

    let first = kern >> 1;
    let second = kern - first;
    if horizontal {
        buffer.pos[i].x_advance = buffer.pos[i].x_advance.saturating_add(first);
        buffer.pos[j].x_advance = buffer.pos[j].x_advance.saturating_add(second);
        buffer.pos[j].x_offset = buffer.pos[j].x_offset.saturating_add(second);
    } else {
        buffer.pos[i].y_advance = buffer.pos[i].y_advance.saturating_add(first);
        buffer.pos[j].y_advance = buffer.pos[j].y_advance.saturating_add(second);
        buffer.pos[j].y_offset = buffer.pos[j].y_offset.saturating_add(second);
    }
}

pub(super) fn chain_for_cross_stream(buffer: &mut Buffer) {
    let back = buffer.direction.is_backward();
    for pos in &mut buffer.pos[..buffer.len] {
        pos.attach_type = attach_type::CURSIVE;
        pos.attach_chain = if back { 1 } else { -1 };
    }
}
