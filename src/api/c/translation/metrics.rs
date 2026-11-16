// metrics.h: the line, OS/2, post and typographic metrics, PDF flags, tracking and STAT.

use alloc::vec::Vec;

use crate::Font;
use crate::ffi::font::{LocationHandle, Names};
use crate::daecore::daetype::stat::ValueKind;
use crate::ffi::base::{OnNul, StrList};
use crate::ffi::handle::{Status, Str, borrow, cleared, deliver, free, or_return, past_end, put, put_some};
use crate::metrics::{LineMetrics, Os2};

unsafe fn at_location<T>(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut T,
    read: impl FnOnce(&Font, &crate::Location) -> T,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { put(out, read(font, crate::ffi::at(location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_ascender(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut f64,
) -> Status {
    unsafe { at_location(font, location, out, Font::ascender) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_descender(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut f64,
) -> Status {
    unsafe { at_location(font, location, out, Font::descender) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_cap_height(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut f64,
) -> Status {
    unsafe { at_location(font, location, out, Font::cap_height) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_italic_angle(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut f64,
) -> Status {
    unsafe { at_location(font, location, out, Font::italic_angle) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_bounds(font: *const Font, out: *mut crate::Bounds) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put_some(out, font.bounds()) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LineMetricsC {
    ascent: f64,
    descent: f64,
    line_gap: f64,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_line_metrics(
    font: *const Font,
    location: *const LocationHandle,
    orientation: i32,
    out: *mut LineMetricsC,
) -> Status {
    let orientation = or_return!(crate::ffi::orientation(orientation));
    unsafe {
        at_location(font, location, out, |f, at| {
            let LineMetrics { ascent, descent, line_gap } = f.line_metrics(at, orientation);
            LineMetricsC { ascent, descent, line_gap }
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_line_metrics_line_height(metrics: *const LineMetricsC, out: *mut f64) -> Status {
    let Some(&LineMetricsC { ascent, descent, line_gap }) = (unsafe { borrow(metrics) }) else { return Status::Null };
    unsafe { put(out, LineMetrics { ascent, descent, line_gap }.line_height()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_pdf_flags(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut u32,
) -> Status {
    unsafe { at_location(font, location, out, |f, at| f.pdf_flags(at).bits()) }
}

// OS/2

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Os2C {
    version: u16,
    weight_class: u16,
    width_class: u16,
    fs_type: u16,
    family_class: u16,
    selection: u16,
    has_weight_class: u8,
    has_width_class: u8,
    has_fs_type: u8,
    has_family_class: u8,
    has_selection: u8,
    has_win_metrics: u8,
    has_typo_metrics: u8,
    win_ascent: f64,
    win_descent: f64,
    typo_ascender: f64,
    typo_descender: f64,
    typo_line_gap: f64,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Os2C>() == 64 && core::mem::offset_of!(Os2C, win_ascent) == 24);

impl Os2C {
    fn of(os2: &Os2) -> Os2C {
        let Os2 { version, weight_class, width_class, fs_type, family_class, selection, win_metrics, typo_metrics } =
            *os2;
        let win = win_metrics.map_or((0.0, 0.0), |w| (w.ascent, w.descent));
        let typo = typo_metrics.map_or((0.0, 0.0, 0.0), |t| (t.ascent, t.descent, t.line_gap));
        Os2C {
            version,
            weight_class: weight_class.unwrap_or(0),
            width_class: width_class.unwrap_or(0),
            fs_type: fs_type.unwrap_or(0),
            has_weight_class: weight_class.is_some().into(),
            has_width_class: width_class.is_some().into(),
            has_fs_type: fs_type.is_some().into(),
            has_family_class: family_class.is_some().into(),
            has_selection: selection.is_some().into(),
            has_win_metrics: win_metrics.is_some().into(),
            has_typo_metrics: typo_metrics.is_some().into(),
            family_class: family_class.unwrap_or(0),
            selection: selection.unwrap_or(0),
            win_ascent: win.0,
            win_descent: win.1,
            typo_ascender: typo.0,
            typo_descender: typo.1,
            typo_line_gap: typo.2,
        }
    }

    // Back to Rust's, a flag read as set for any byte but 0, as C's bool may come from anywhere.
    fn rust(&self) -> Os2 {
        fn some<T>(has: u8, v: T) -> Option<T> {
            (has != 0).then_some(v)
        }
        Os2 {
            version: self.version,
            weight_class: some(self.has_weight_class, self.weight_class),
            width_class: some(self.has_width_class, self.width_class),
            fs_type: some(self.has_fs_type, self.fs_type),
            family_class: some(self.has_family_class, self.family_class),
            selection: some(self.has_selection, self.selection),
            win_metrics: some(
                self.has_win_metrics,
                crate::metrics::WinMetrics { ascent: self.win_ascent, descent: self.win_descent },
            ),
            typo_metrics: some(
                self.has_typo_metrics,
                LineMetrics { ascent: self.typo_ascender, descent: self.typo_descender, line_gap: self.typo_line_gap },
            ),
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_os2(font: *const Font, location: *const LocationHandle, out: *mut Os2C) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put_some(out, font.os2(crate::ffi::at(location)).as_ref().map(Os2C::of)) }
}

unsafe fn os2_says(os2: *const Os2C, out: *mut bool, says: fn(&Os2) -> bool) -> Status {
    let Some(os2) = (unsafe { borrow(os2) }) else { return Status::Null };
    unsafe { put(out, says(&os2.rust())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_os2_is_italic(os2: *const Os2C, out: *mut bool) -> Status {
    unsafe { os2_says(os2, out, Os2::is_italic) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_os2_is_bold(os2: *const Os2C, out: *mut bool) -> Status {
    unsafe { os2_says(os2, out, Os2::is_bold) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_os2_is_regular(os2: *const Os2C, out: *mut bool) -> Status {
    unsafe { os2_says(os2, out, Os2::is_regular) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_os2_is_oblique(os2: *const Os2C, out: *mut bool) -> Status {
    unsafe { os2_says(os2, out, Os2::is_oblique) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_os2_uses_typo_metrics(os2: *const Os2C, out: *mut bool) -> Status {
    unsafe { os2_says(os2, out, Os2::uses_typo_metrics) }
}

// typographic metrics

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TypographicMetricsC {
    has_x_height: bool,
    has_underline: bool,
    has_strikeout: bool,
    has_subscript: bool,
    has_superscript: bool,
    x_height: f64,
    underline: [f64; 2],
    strikeout: [f64; 2],
    subscript: [f64; 4],
    superscript: [f64; 4],
}

const _: () = assert!(size_of::<TypographicMetricsC>() == 112);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_typographic_metrics(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut TypographicMetricsC,
) -> Status {
    unsafe {
        at_location(font, location, out, |f, at| {
            let m = f.typographic_metrics(at);
            let line = |l: Option<crate::metrics::DecorationLine>| l.map_or([0.0; 2], |l| [l.position, l.thickness]);
            let script = |s: Option<crate::metrics::ScriptMetrics>| {
                s.map_or([0.0; 4], |s| [s.x_size, s.y_size, s.x_offset, s.y_offset])
            };
            TypographicMetricsC {
                has_x_height: m.x_height.is_some(),
                has_underline: m.underline.is_some(),
                has_strikeout: m.strikeout.is_some(),
                has_subscript: m.subscript.is_some(),
                has_superscript: m.superscript.is_some(),
                x_height: m.x_height.unwrap_or(0.0),
                underline: line(m.underline),
                strikeout: line(m.strikeout),
                subscript: script(m.subscript),
                superscript: script(m.superscript),
            }
        })
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_tracking(
    font: *const Font,
    point_size: f64,
    orientation: i32,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let orientation = or_return!(crate::ffi::orientation(orientation));
    unsafe { put(out, font.tracking(point_size, orientation)) }
}

// STAT

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StatAxisC {
    tag: u32,
    ordering: u16,
    has_name: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StatValueC {
    kind: i32,
    axis_index: u16,
    elidable: bool,
    has_name: bool,
    older_sibling: bool,
    value: f64,
    min: f64,
    max: f64,
    linked_value: f64,
    combo_start: u32,
    combo_count: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AxisValueC {
    axis_index: u16,
    value: f64,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<StatValueC>() == 56 && size_of::<AxisValueC>() == 16);

pub struct Stat {
    axes: Vec<StatAxisC>,
    values: Vec<StatValueC>,
    combos: Vec<AxisValueC>,
    // The name table's every name, decoded once, and the one each axis then each value names, where its flag says it
    // has one.
    names: StrList,
    slots: Vec<usize>,
    elided_fallback_name: Option<usize>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_stat(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut *mut Stat,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let (mut names, mut slots, mut combos) = (Names::new(), Vec::new(), Vec::new());
    let (names_out, slots_out) = (&mut names, &mut slots);
    // Rust's budget, so a font whose every value names one long name loses the same names: it counts bytes copied
    // there, and is kept here though nothing is copied.
    let lookup = move |tables: &_, stat_len: usize| {
        // Moved in, so the lookup it makes can hold them past this call.
        let (names, slots) = (names_out, slots_out);
        crate::daecore::daetype::decoder::each_name_string(tables, names);
        let names: &Names = names;
        let held = names.strings.bytes_held();
        let mut budget = stat_len.saturating_mul(16).saturating_add(held);
        move |id: u16| {
            let slot = names.ids.binary_search(&id).ok().filter(|&slot| {
                let len = names.strings.get(slot).map_or(0, |s| s.len);
                budget.checked_sub(len).map(|left| budget = left).is_some()
            });
            slots.push(slot.unwrap_or(0));
            slot
        }
    };
    let axis = |tag: crate::Tag, name: Option<usize>, ordering| {
        StatAxisC { tag: tag.0, ordering, has_name: name.is_some() }
    };
    let value = |kind, name: Option<usize>, elidable, older_sibling| {
        stat_value(kind, name.is_some(), elidable, older_sibling, &mut combos)
    };
    let at = unsafe { crate::ffi::at(location) };
    let Some((axes, values, elided_fallback_name)) = font.stat_with(at, lookup, axis, value) else {
        return Status::Absent;
    };
    let names = names.strings.finish(OnNul::Replace);
    unsafe { deliver(out, Stat { axes, values, combos, names, slots, elided_fallback_name }) }
}

pub const STAT_SINGLE: i32 = 0;
pub const STAT_RANGE: i32 = 1;
pub const STAT_LINKED: i32 = 2;
pub const STAT_COMBO: i32 = 3;

fn stat_value(
    kind: ValueKind,
    has_name: bool,
    elidable: bool,
    older_sibling: bool,
    combos: &mut Vec<AxisValueC>,
) -> StatValueC {
    let row = |kind, axis_index| StatValueC {
        kind,
        axis_index,
        elidable,
        has_name,
        older_sibling,
        value: 0.0,
        min: 0.0,
        max: 0.0,
        linked_value: 0.0,
        combo_start: 0,
        combo_count: 0,
    };
    match kind {
        ValueKind::Single { axis, value } => StatValueC { value, ..row(STAT_SINGLE, axis) },
        ValueKind::Range { axis, nominal, min, max } => {
            StatValueC { value: nominal, min, max, ..row(STAT_RANGE, axis) }
        }
        ValueKind::Linked { axis, value, linked } => {
            StatValueC { value, linked_value: linked, ..row(STAT_LINKED, axis) }
        }
        ValueKind::Combo(values) => {
            let start = combos.len() as u32;
            combos.extend(values.iter().map(|&(axis_index, value)| AxisValueC { axis_index, value }));
            StatValueC { combo_start: start, combo_count: values.len() as u32, ..row(STAT_COMBO, 0) }
        }
    }
}

crate::ffi::handle::field_view!(daegun_stat_axes, Stat, axes, StatAxisC);
crate::ffi::handle::field_view!(daegun_stat_combo_values, Stat, combos, AxisValueC);
free!(daegun_stat_free, Stat);

// The name the axis or value at `first + index` of `slots` names, where `index` is under `len` and `named` says it has
// one.
unsafe fn named(stat: &Stat, first: usize, index: usize, len: usize, named: bool, what: &str, out: *mut Str) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    if index >= len {
        return past_end(what, index, len);
    }
    let name = named.then(|| stat.slots.get(first + index).and_then(|&slot| stat.names.get(slot))).flatten();
    unsafe { put_some(out, name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_axis_name(stat: *const Stat, index: usize, out: *mut Str) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    let has_name = stat.axes.get(index).is_some_and(|a| a.has_name);
    unsafe { named(stat, 0, index, stat.axes.len(), has_name, "STAT axis", out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_value_count(stat: *const Stat, out: *mut usize) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    unsafe { put(out, stat.values.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_elided_fallback_name(stat: *const Stat, out: *mut Str) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    unsafe { put_some(out, stat.elided_fallback_name.and_then(|slot| stat.names.get(slot))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_value_at(stat: *const Stat, index: usize, out: *mut StatValueC) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    match stat.values.get(index) {
        Some(&value) => unsafe { put(out, value) },
        None => past_end("STAT value", index, stat.values.len()),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_stat_value_name(stat: *const Stat, index: usize, out: *mut Str) -> Status {
    let Some(stat) = (unsafe { borrow(stat) }) else { return Status::Null };
    let has_name = stat.values.get(index).is_some_and(|v| v.has_name);
    unsafe { named(stat, stat.axes.len(), index, stat.values.len(), has_name, "STAT value", out) }
}

#[cfg(test)]
#[path = "tests/metrics.rs"]
mod tests;
