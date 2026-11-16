// layout.h: lines laid out, and one line justified by JSTF.

use alloc::vec::Vec;
use core::ffi::{c_char, c_void};

use crate::ffi::font::LocationHandle;
use crate::ffi::handle::{
    Options, Status, borrow, cleared, deliver, defaults_into, fill, free, ident_of, into_buffer, invalid, options_of,
    or_return, past_end, put, slice_of, text_of,
};
use crate::ffi::shape::deliver_run;
use crate::ffi::unicode::{SegmentFn, segment_with};
use crate::layout::{
    Adjustment, Align, BreakStrategy, JstfModLists, JustifyOptions, LayoutOptions, TextLayout, TextOrientation,
    WritingMode,
};
use crate::shape::{Offset, ShapedRun};
use crate::unicode::WordSegmenter;
use crate::{Font, Tag};

// options

pub const WRITING_HORIZONTAL: i32 = 0;
pub const WRITING_VERTICAL_RL: i32 = 1;
pub const WRITING_VERTICAL_LR: i32 = 2;

fn writing_mode(code: i32) -> Result<WritingMode, Status> {
    match code {
        WRITING_HORIZONTAL => Ok(WritingMode::Horizontal),
        WRITING_VERTICAL_RL => Ok(WritingMode::VerticalRl),
        WRITING_VERTICAL_LR => Ok(WritingMode::VerticalLr),
        _ => Err(invalid(format_args!("writing mode {code} is none of DAEGUN_WRITING_*"))),
    }
}

pub const ALIGN_START: i32 = 0;
pub const ALIGN_END: i32 = 1;
pub const ALIGN_CENTER: i32 = 2;
pub const ALIGN_JUSTIFY: i32 = 3;

fn align(code: i32) -> Result<Align, Status> {
    match code {
        ALIGN_START => Ok(Align::Start),
        ALIGN_END => Ok(Align::End),
        ALIGN_CENTER => Ok(Align::Center),
        ALIGN_JUSTIFY => Ok(Align::Justify),
        _ => Err(invalid(format_args!("align {code} is none of DAEGUN_ALIGN_*"))),
    }
}

pub const TEXT_ORIENTATION_MIXED: i32 = 0;
pub const TEXT_ORIENTATION_UPRIGHT: i32 = 1;
pub const TEXT_ORIENTATION_SIDEWAYS: i32 = 2;

fn text_orientation(code: i32) -> Result<TextOrientation, Status> {
    match code {
        TEXT_ORIENTATION_MIXED => Ok(TextOrientation::Mixed),
        TEXT_ORIENTATION_UPRIGHT => Ok(TextOrientation::Upright),
        TEXT_ORIENTATION_SIDEWAYS => Ok(TextOrientation::Sideways),
        _ => Err(invalid(format_args!("text orientation {code} is none of DAEGUN_TEXT_ORIENTATION_*"))),
    }
}

pub const BREAK_GREEDY: i32 = 0;
pub const BREAK_OPTIMAL: i32 = 1;

fn strategy(code: i32) -> Result<BreakStrategy, Status> {
    match code {
        BREAK_GREEDY => Ok(BreakStrategy::Greedy),
        BREAK_OPTIMAL => Ok(BreakStrategy::Optimal),
        _ => Err(invalid(format_args!("strategy {code} is none of DAEGUN_BREAK_*"))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_writing_mode_is_vertical(mode: i32, out: *mut bool) -> Status {
    let mode = or_return!(writing_mode(mode));
    unsafe { put(out, mode.is_vertical()) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LayoutOptionsC {
    size: usize,
    max_inline_size: f64,
    line_height: f64,
    tab_size: f64,
    language: *const c_char,
    segmenter: Option<SegmentFn>,
    segmenter_user: *mut c_void,
    max_lines: usize,
    has_line_height: u8,
    has_max_lines: u8,
    align: i32,
    writing_mode: i32,
    text_orientation: i32,
    base_direction: i32,
    strategy: i32,
    features: *const crate::ffi::shape::FeatureC,
    features_len: usize,
}

// The header's sizes, which a 64-bit target holds both sides to.
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(size_of::<LayoutOptionsC>() == 104 && core::mem::offset_of!(LayoutOptionsC, align) == 68);
    assert!(size_of::<LayoutInfoC>() == 40 && size_of::<LayoutLineC>() == 64 && size_of::<LayoutRunC>() == 48);
    assert!(size_of::<JustifyOptionsC>() == 32 && size_of::<JustificationC>() == 24);
    assert!(core::mem::offset_of!(JustificationC, adjustment) == 16);
};

impl Options for LayoutOptionsC {
    unsafe fn set_defaults(p: *mut Self) {
        unsafe {
            (&raw mut (*p).max_inline_size).write(f64::INFINITY);
            (&raw mut (*p).tab_size).write(8.0);
            (&raw mut (*p).base_direction).write(-1);
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_layout_options_default(out: *mut LayoutOptionsC, size: usize) -> Status {
    unsafe { defaults_into(out, size) }
}

// Rust's options from C's, with the segmenter `segment_with` wrapped and the features converted from them.
fn layout_options<'a>(
    opts: &'a LayoutOptionsC,
    segmenter: Option<WordSegmenter<'a>>,
    features: &'a [crate::shape::Feature],
) -> Result<LayoutOptions<'a>, Status> {
    Ok(LayoutOptions {
        max_inline_size: opts.max_inline_size,
        align: align(opts.align)?,
        writing_mode: writing_mode(opts.writing_mode)?,
        text_orientation: text_orientation(opts.text_orientation)?,
        base_direction: crate::ffi::base_direction(opts.base_direction)?,
        language: unsafe { ident_of(opts.language) }?,
        line_height: (opts.has_line_height != 0).then_some(opts.line_height),
        strategy: strategy(opts.strategy)?,
        max_lines: (opts.has_max_lines != 0).then_some(opts.max_lines),
        tab_size: opts.tab_size,
        segmenter,
        features,
    })
}

// layout

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_layout(
    font: *const Font,
    text: *const c_char,
    text_len: usize,
    location: *const LocationHandle,
    opts: *const LayoutOptionsC,
    out: *mut *mut TextLayout,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let text = or_return!(unsafe { text_of(text, text_len) });
    let opts = or_return!(unsafe { options_of(opts) });
    let segment = opts.segmenter.map(|f| segment_with(f, opts.segmenter_user));
    let Some(given) = (unsafe { slice_of(opts.features, opts.features_len) }) else { return Status::Null };
    let features: Vec<_> = given.iter().map(crate::ffi::shape::feature_of).collect();
    let options = or_return!(layout_options(&opts, segment.as_ref().map(|s| WordSegmenter(s)), &features));
    unsafe { deliver(out, font.layout(text, crate::ffi::at(location), &options)) }
}

free!(daegun_layout_free, TextLayout);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LayoutInfoC {
    line_count: usize,
    inline_size: f64,
    block_size: f64,
    truncated: usize,
    has_truncated: bool,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_layout_info_of(layout: *const TextLayout, out: *mut LayoutInfoC) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    let info = LayoutInfoC {
        line_count: layout.lines.len(),
        inline_size: layout.inline_size,
        block_size: layout.block_size,
        truncated: layout.truncated.unwrap_or(0),
        has_truncated: layout.truncated.is_some(),
    };
    unsafe { put(out, info) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LayoutLineC {
    run_count: usize,
    start: usize,
    end: usize,
    baseline: f64,
    inline_size: f64,
    ascent: f64,
    descent: f64,
    hard_break: bool,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_layout_line_at(
    layout: *const TextLayout,
    line: usize,
    out: *mut LayoutLineC,
) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(l) = layout.lines.get(line) else { return past_end("line", line, layout.lines.len()) };
    let line = LayoutLineC {
        run_count: l.runs.len(),
        start: l.bytes.start,
        end: l.bytes.end,
        baseline: l.baseline,
        inline_size: l.inline_size,
        ascent: l.ascent,
        descent: l.descent,
        hard_break: l.hard_break,
    };
    unsafe { put(out, line) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct LayoutRunC {
    run: *const ShapedRun,
    offset: Offset,
    start: usize,
    end: usize,
    level: u8,
    upright: bool,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_layout_run_at(
    layout: *const TextLayout,
    line: usize,
    run: usize,
    out: *mut LayoutRunC,
) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(l) = layout.lines.get(line) else { return past_end("line", line, layout.lines.len()) };
    let Some(r) = l.runs.get(run) else { return past_end("run", run, l.runs.len()) };
    let run = LayoutRunC {
        run: &raw const r.run,
        offset: r.offset,
        start: r.bytes.start,
        end: r.bytes.end,
        level: r.level,
        upright: r.upright,
    };
    unsafe { put(out, run) }
}

// justification

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JustifyOptionsC {
    size: usize,
    script: u32,
    language_system: u32,
    target_width: f64,
    tolerance: f64,
}

// Every default is zero.
impl Options for JustifyOptionsC {
    unsafe fn set_defaults(_: *mut Self) {}
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_justify_options_default(out: *mut JustifyOptionsC, size: usize) -> Status {
    unsafe { defaults_into(out, size) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct JustificationC {
    width: f64,
    level: usize,
    adjustment: i32,
    has_level: bool,
    best_effort: bool,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_justify(
    font: *const Font,
    text: *const c_char,
    text_len: usize,
    location: *const LocationHandle,
    orientation: i32,
    opts: *const JustifyOptionsC,
    out_run: *mut *mut ShapedRun,
    out_info: *mut JustificationC,
) -> Status {
    let Some(out_run) = (unsafe { cleared(out_run) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let text = or_return!(unsafe { text_of(text, text_len) });
    let orientation = or_return!(crate::ffi::orientation(orientation));
    let opts = or_return!(unsafe { options_of(opts) });
    let options = JustifyOptions {
        script: Tag(opts.script),
        language_system: crate::ffi::opt_tag(opts.language_system),
        target_width: opts.target_width,
        tolerance: opts.tolerance,
    };
    let justified = font.justify(text, unsafe { crate::ffi::at(location) }, orientation, &options);
    let info = JustificationC {
        width: justified.width,
        level: justified.level.unwrap_or(0),
        adjustment: match justified.adjustment {
            Adjustment::Shrink => ADJUST_SHRINK,
            Adjustment::Extend => ADJUST_EXTEND,
        },
        has_level: justified.level.is_some(),
        best_effort: justified.best_effort,
    };
    unsafe { fill(out_info, info) };
    unsafe { deliver_run(out_run, justified.run) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_justification_glyphs(
    font: *const Font,
    script: u32,
    out: *mut u32,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    match font.justification_glyphs(Tag(script)) {
        Some(glyphs) => unsafe { into_buffer(glyphs.into_iter().map(|g| g.0), out, out_len, out_count) },
        None => Status::Absent,
    }
}

pub struct JstfPriorities(Vec<JstfModLists>);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_justification_priorities(
    font: *const Font,
    script: u32,
    language_system: u32,
    out: *mut *mut JstfPriorities,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    match font.justification_priorities(Tag(script), crate::ffi::opt_tag(language_system)) {
        Some(levels) => unsafe { deliver(out, JstfPriorities(levels)) },
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_jstf_priorities_count(priorities: *const JstfPriorities, out: *mut usize) -> Status {
    let Some(priorities) = (unsafe { borrow(priorities) }) else { return Status::Null };
    unsafe { put(out, priorities.0.len()) }
}

free!(daegun_jstf_priorities_free, JstfPriorities);

pub const JSTF_SHRINKAGE_ENABLE_GSUB: i32 = 0;
pub const JSTF_SHRINKAGE_DISABLE_GSUB: i32 = 1;
pub const JSTF_SHRINKAGE_ENABLE_GPOS: i32 = 2;
pub const JSTF_SHRINKAGE_DISABLE_GPOS: i32 = 3;
pub const JSTF_EXTENSION_ENABLE_GSUB: i32 = 4;
pub const JSTF_EXTENSION_DISABLE_GSUB: i32 = 5;
pub const JSTF_EXTENSION_ENABLE_GPOS: i32 = 6;
pub const JSTF_EXTENSION_DISABLE_GPOS: i32 = 7;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_jstf_priorities_lookups(
    priorities: *const JstfPriorities,
    level: usize,
    which: i32,
    out: *mut *const u16,
    out_count: *mut usize,
) -> Status {
    let Some(priorities) = (unsafe { borrow(priorities) }) else { return Status::Null };
    if out.is_null() || out_count.is_null() {
        return Status::Null;
    }
    let Some(l) = priorities.0.get(level) else { return past_end("level", level, priorities.0.len()) };
    let list = match which {
        JSTF_SHRINKAGE_ENABLE_GSUB => &l.shrinkage_enable_gsub,
        JSTF_SHRINKAGE_DISABLE_GSUB => &l.shrinkage_disable_gsub,
        JSTF_SHRINKAGE_ENABLE_GPOS => &l.shrinkage_enable_gpos,
        JSTF_SHRINKAGE_DISABLE_GPOS => &l.shrinkage_disable_gpos,
        JSTF_EXTENSION_ENABLE_GSUB => &l.extension_enable_gsub,
        JSTF_EXTENSION_DISABLE_GSUB => &l.extension_disable_gsub,
        JSTF_EXTENSION_ENABLE_GPOS => &l.extension_enable_gpos,
        JSTF_EXTENSION_DISABLE_GPOS => &l.extension_disable_gpos,
        _ => return invalid(format_args!("lookup list {which} is none of DAEGUN_JSTF_*")),
    };
    let Some(list) = list else { return Status::Absent };
    unsafe {
        *out = list.as_ptr();
        *out_count = list.len();
    }
    Status::Ok
}

pub const ADJUST_SHRINK: i32 = 0;
pub const ADJUST_EXTEND: i32 = 1;

fn adjustment_of(code: i32) -> Result<Adjustment, Status> {
    match code {
        ADJUST_SHRINK => Ok(Adjustment::Shrink),
        ADJUST_EXTEND => Ok(Adjustment::Extend),
        _ => Err(invalid(format_args!("adjustment {code} is none of DAEGUN_ADJUST_*"))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_shape_justified(
    font: *const Font,
    text: *const c_char,
    text_len: usize,
    location: *const LocationHandle,
    orientation: i32,
    priorities: *const JstfPriorities,
    level: usize,
    adjustment: i32,
    out: *mut *mut ShapedRun,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let (Some(font), Some(priorities)) = (unsafe { (borrow(font), borrow(priorities)) }) else {
        return Status::Null;
    };
    let text = or_return!(unsafe { text_of(text, text_len) });
    let orientation = or_return!(crate::ffi::orientation(orientation));
    let Some(level) = priorities.0.get(level) else { return past_end("level", level, priorities.0.len()) };
    let adjustment = or_return!(adjustment_of(adjustment));
    let run = font.shape_justified(text, unsafe { crate::ffi::at(location) }, orientation, level, adjustment);
    unsafe { deliver_run(out, run) }
}

#[cfg(test)]
#[path = "tests/layout.rs"]
mod tests;
