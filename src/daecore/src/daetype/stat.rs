use alloc::string::String;
use alloc::vec::Vec;
use super::decoder::{each_name_string, read_fixed, read_u16_be, read_u32_be, records_fit, window, NameText};
use crate::daecore::daetype::Tables;
use crate::daecore::{Error, Tag};

/// One of STAT's design axes.
#[non_exhaustive]
#[derive(Debug, PartialEq, Clone)]
pub struct StatAxis {
    /// The axis's tag, as fvar's would be.
    pub tag: Tag,
    /// Its name, none where the name table has no record for it.
    pub name: Option<String>,
    /// Where the axis falls when names are put together, lowest first.
    pub ordering: u16,
}

/// A value STAT names on its axes, in user coordinates.
#[non_exhaustive]
#[derive(Debug, PartialEq, Clone)]
pub enum StatAxisValue {
    /// One value on one axis.
    Single {
        /// Which of STAT's axes, by index.
        axis_index: u16,
        /// Its name, none where the name table has no record for it.
        name: Option<String>,
        /// The value.
        value: f64,
        /// Whether the name is left out when names are put together.
        elidable: bool,
        /// Whether it describes other fonts of the family rather than this one.
        older_sibling: bool,
    },
    /// A range on one axis, with the value it is named for.
    Range {
        /// Which of STAT's axes, by index.
        axis_index: u16,
        /// Its name, none where the name table has no record for it.
        name: Option<String>,
        /// The value the name is for.
        nominal: f64,
        /// The lowest value in the range.
        min: f64,
        /// The highest value in the range.
        max: f64,
        /// Whether the name is left out when names are put together.
        elidable: bool,
        /// Whether it describes other fonts of the family rather than this one.
        older_sibling: bool,
    },
    /// One value on one axis, with the value linked to it, as bold is to regular.
    Linked {
        /// Which of STAT's axes, by index.
        axis_index: u16,
        /// Its name, none where the name table has no record for it.
        name: Option<String>,
        /// The value.
        value: f64,
        /// The value linked to it.
        linked_value: f64,
        /// Whether the name is left out when names are put together.
        elidable: bool,
        /// Whether it describes other fonts of the family rather than this one.
        older_sibling: bool,
    },
    /// Values on several axes, named together.
    Combo {
        /// Its name, none where the name table has no record for it.
        name: Option<String>,
        /// Each value with the index of its axis.
        values: Vec<(u16, f64)>,
        /// Whether the name is left out when names are put together.
        elidable: bool,
        /// Whether it describes other fonts of the family rather than this one.
        older_sibling: bool,
    },
}

pub(crate) const OLDER_SIBLING_FONT_ATTRIBUTE: u16 = 0x0001;
const ELIDABLE_AXIS_VALUE_NAME: u16 = 0x0002;

// A STAT table as the public API and instancing both read it.
pub(crate) struct Stat<'a> {
    pub data: &'a [u8],
    axis_size: usize,
    axis_count: usize,
    axes_at: usize,
    value_count: usize,
    values_at: usize,
}

// An AxisValue table: where it lies, its flags and name, and what it describes.
pub(crate) struct AxisValue {
    pub at: usize,
    pub len: usize,
    pub flags: u16,
    pub name_id: u16,
    pub kind: ValueKind,
}

pub(crate) enum ValueKind {
    Single { axis: u16, value: f64 },
    Range { axis: u16, nominal: f64, min: f64, max: f64 },
    Linked { axis: u16, value: f64, linked: f64 },
    Combo(Vec<(u16, f64)>),
}

impl<'a> Stat<'a> {
    // The header must lie in the table. The design axes are absent where their count leaves it or designAxisSize is
    // narrower than an AxisRecord, and the value offsets where theirs does.
    pub fn read(data: &'a [u8]) -> Result<Self, Error> {
        let field = |at: usize| read_u16_be(data, at).map(usize::from);
        let wide = |at: usize| read_u32_be(data, at).map(|v| v as usize);
        let (Some(axis_size), Some(axis_count), Some(axes_at), Some(value_count), Some(values_at)) =
            (field(4), field(6), wide(8), field(12), wide(14))
        else {
            return Err(Error::malformed("STAT: header truncated"));
        };
        let fits = |at, count, size| records_fit(at, count, size, data.len());
        let axis_count = if axis_size >= 8 && fits(axes_at, axis_count, axis_size) { axis_count } else { 0 };
        let value_count = if fits(values_at, value_count, 2) { value_count } else { 0 };
        Ok(Stat { data, axis_size, axis_count, axes_at, value_count, values_at })
    }

    // The design axis records, as they lie.
    pub fn design_axes(&self) -> &'a [u8] {
        self.data.get(self.axes_at..self.axes_at + self.axis_count * self.axis_size).unwrap_or_default()
    }

    // Each design axis: its tag, name ID and ordering.
    pub fn axes(&self) -> impl Iterator<Item = ([u8; 4], u16, u16)> + '_ {
        (0..self.axis_count).filter_map(|i| {
            let rec = self.axes_at + i * self.axis_size;
            Some((*window::<4>(self.data, rec)?, read_u16_be(self.data, rec + 4)?, read_u16_be(self.data, rec + 6)?))
        })
    }

    // Each AxisValue table that reads. One naming an axis the table lacks is left out, as is a
    // combination naming one; combinations hold one record per six bytes of the table between them.
    pub fn values(&self) -> impl Iterator<Item = AxisValue> + '_ {
        let mut combo_left = self.data.len() / 6;
        (0..self.value_count).filter_map(move |i| {
            let at = self.values_at + usize::from(read_u16_be(self.data, self.values_at + 2 * i)?);
            self.value(at, &mut combo_left)
        })
    }

    fn value(&self, at: usize, combo_left: &mut usize) -> Option<AxisValue> {
        let data = self.data;
        let fixed = |k: usize| read_fixed(data, at + k);
        let on_an_axis = |rec: usize| read_u16_be(data, rec).filter(|&axis| usize::from(axis) < self.axis_count);
        let (kind, len) = match read_u16_be(data, at)? {
            1 => (ValueKind::Single { axis: on_an_axis(at + 2)?, value: fixed(8)? }, 12),
            2 => (
                ValueKind::Range { axis: on_an_axis(at + 2)?, nominal: fixed(8)?, min: fixed(12)?, max: fixed(16)? },
                20,
            ),
            3 => (ValueKind::Linked { axis: on_an_axis(at + 2)?, value: fixed(8)?, linked: fixed(12)? }, 16),
            4 => {
                let count = usize::from(read_u16_be(data, at + 2)?);
                if !records_fit(at + 8, count, 6, data.len()) {
                    return None;
                }
                *combo_left = combo_left.checked_sub(count)?;
                let records = (0..count).map(|j| Some((on_an_axis(at + 8 + 6 * j)?, fixed(10 + 6 * j)?)));
                (ValueKind::Combo(records.collect::<Option<_>>()?), 8 + 6 * count)
            }
            _ => return None,
        };
        Some(AxisValue { at, len, flags: read_u16_be(data, at + 4)?, name_id: read_u16_be(data, at + 6)?, kind })
    }

    // STAT 1.1's name for a font whose every value is elided.
    pub fn elided_fallback(&self) -> Option<u16> {
        (read_u16_be(self.data, 2)? >= 1).then(|| read_u16_be(self.data, 18)).flatten()
    }
}

// Whether this STAT is read at all: major version 1, as the spec defines it, with the header its minor version gives,
// 18 bytes at 1.0 and 20 from 1.1. Axis records or values past the table are absent where they're read.
pub(crate) fn has_data(stat: &[u8]) -> bool {
    let header = if read_u16_be(stat, 2).is_some_and(|minor| minor >= 1) { 20 } else { 18 };
    stat.len() >= header && read_u16_be(stat, 0) == Some(1)
}

// The axes, the values and the elided fallback name, each name Rust's own copy or the place C keeps it.
pub type StatInfo<A = StatAxis, V = StatAxisValue, N = String> = (Vec<A>, Vec<V>, Option<N>);

pub fn parse_stat(
    table_map: &Tables,
) -> Result<StatInfo, Error> {
    // Each axis and value gets its name's own copy, so a name that every value names cannot multiply
    // past the bytes the two tables could hold between them.
    let names = |table_map: &Tables, stat_len: usize| {
        let mut names = NameText::default();
        each_name_string(table_map, &mut names);
        let mut name_budget = stat_len.saturating_mul(16).saturating_add(names.bytes());
        move |id: u16| -> Option<String> {
            let name = names.get(id)?;
            name_budget = name_budget.checked_sub(name.len())?;
            Some(name.into())
        }
    };
    let axis = |tag, name, ordering| StatAxis { tag, name, ordering };
    let value = |kind, name, elidable, older_sibling| match kind {
        ValueKind::Single { axis, value } => {
            StatAxisValue::Single { axis_index: axis, name, value, elidable, older_sibling }
        }
        ValueKind::Range { axis, nominal, min, max } => {
            StatAxisValue::Range { axis_index: axis, name, nominal, min, max, elidable, older_sibling }
        }
        ValueKind::Linked { axis, value, linked } => StatAxisValue::Linked {
            axis_index: axis,
            name,
            value,
            linked_value: linked,
            elidable,
            older_sibling,
        },
        ValueKind::Combo(values) => StatAxisValue::Combo { name, values, elidable, older_sibling },
    };
    parse_stat_with(table_map, names, axis, value)
}

// STAT's axes, values and elided fallback name in order, each axis and value made by its closure and each name taken
// by the lookup `names` makes once the table reads: Rust's own copy of it, or the place C keeps it.
pub(crate) fn parse_stat_with<N, F: FnMut(u16) -> Option<N>, A, V>(
    table_map: &Tables,
    names: impl FnOnce(&Tables, usize) -> F,
    mut axis: impl FnMut(Tag, Option<N>, u16) -> A,
    mut value: impl FnMut(ValueKind, Option<N>, bool, bool) -> V,
) -> Result<StatInfo<A, V, N>, Error> {
    let stat = Stat::read(table_map.get("STAT").ok_or(Error::missing("missing STAT"))?)?;
    let mut name_of = names(table_map, stat.data.len());
    let axes = stat.axes()
        .map(|(tag, name_id, ordering)| axis(Tag::new(&tag), name_of(name_id), ordering))
        .collect();
    let values = stat.values()
        .map(|v| {
            let (name, elidable, older_sibling) = (
                name_of(v.name_id),
                v.flags & ELIDABLE_AXIS_VALUE_NAME != 0,
                v.flags & OLDER_SIBLING_FONT_ATTRIBUTE != 0,
            );
            value(v.kind, name, elidable, older_sibling)
        })
        .collect();
    let elided_fallback_name = stat.elided_fallback().and_then(&mut name_of);
    Ok((axes, values, elided_fallback_name))
}

#[cfg(test)]
#[path = "../../tests/unit/daetype/stat.rs"]
mod tests;
