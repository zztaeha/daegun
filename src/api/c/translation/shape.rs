// shape.h: runs, their options, bidi runs and carets.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ffi::c_char;

use crate::ffi::font::LocationHandle;
use crate::ffi::handle::{
    Options, Status, Str, borrow, cleared, deliver, field_view, fill, free, ident_of, into_buffer, invalid,
    not_utf8, options_of, or_return, past_end, put, slice_of, text_of, defaults_into, apart, span,
};
use crate::shape::{BidiRun, ClusterLevel, Direction, Feature, Ignorables, Offset, ShapeOptions, ShapedRun};
use crate::{Font, GlyphId, Tag};

// runs

// A run handed out owned is an Arc's, so a cached run reaches C without a copy. One lent by a set of bidi runs is an
// Arc's too, and one a layout lends a plain reference; the header forbids freeing either.
pub(crate) unsafe fn deliver_run(out: *mut *mut ShapedRun, run: Arc<ShapedRun>) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = Arc::into_raw(run).cast_mut() };
    Status::Ok
}

// A glyph id is a transparent u32 and a flag a bool, one byte of 0 or 1, as the header's types are.
field_view!(daegun_shaped_run_glyphs, ShapedRun, glyphs, GlyphId);
field_view!(daegun_shaped_run_advances, ShapedRun, advances, f64);
field_view!(daegun_shaped_run_offsets, ShapedRun, offsets, Offset);
field_view!(daegun_shaped_run_clusters, ShapedRun, clusters, u32);
field_view!(daegun_shaped_run_unsafe_to_break, ShapedRun, unsafe_to_break, bool);
field_view!(daegun_shaped_run_unsafe_to_concat, ShapedRun, unsafe_to_concat, bool);
field_view!(daegun_shaped_run_safe_to_insert_tatweel, ShapedRun, safe_to_insert_tatweel, bool);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shaped_run_width(run: *const ShapedRun, out: *mut f64) -> Status {
    let Some(run) = (unsafe { borrow(run) }) else { return Status::Null };
    unsafe { put(out, run.width()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shaped_run_complete(run: *const ShapedRun, out: *mut bool) -> Status {
    let Some(run) = (unsafe { borrow(run) }) else { return Status::Null };
    unsafe { put(out, run.complete) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shaped_run_has_broken_syllable(run: *const ShapedRun, out: *mut bool) -> Status {
    let Some(run) = (unsafe { borrow(run) }) else { return Status::Null };
    unsafe { put(out, run.has_broken_syllable) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shaped_run_shaper(run: *const ShapedRun, out: *mut i32) -> Status {
    let Some(run) = (unsafe { borrow(run) }) else { return Status::Null };
    unsafe { put(out, run.shaper as i32) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shaper_name(shaper: i32, out: *mut Str) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let shapers = &crate::daecore::daeshaper::script::ALL;
    let Some(s) = usize::try_from(shaper).ok().and_then(|i| shapers.get(i)) else {
        return invalid(format_args!("shaper {shaper} is none of DAEGUN_SHAPER_*"));
    };
    unsafe { put(out, Str::of_static(s.kind().c_name())) }
}

// Its arrays copied: a run a layout lends lies in the layout, held by no Arc, so no hold can be taken on it, and
// keeping each layout's runs in Arcs cost every layout an allocation a run where this costs only a copy.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shaped_run_copy(run: *const ShapedRun, out: *mut *mut ShapedRun) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(run) = (unsafe { borrow(run) }) else { return Status::Null };
    unsafe { deliver_run(out, Arc::new(run.clone())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shaped_run_free(run: *mut ShapedRun) {
    if !run.is_null() {
        drop(unsafe { Arc::from_raw(run) });
    }
}

// options

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FeatureC {
    tag: u32,
    value: u32,
    start: u32,
    end: u32,
}

const _: () = assert!(size_of::<FeatureC>() == 16);

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShapeOptionsC {
    size: usize,
    direction: i32,
    script: u32,
    language: *const c_char,
    features: *const FeatureC,
    features_len: usize,
    before: *const c_char,
    before_len: usize,
    after: *const c_char,
    after_len: usize,
    cluster_level: i32,
    ignorables: i32,
    invisible_glyph: u32,
    not_found_glyph: u32,
    not_found_variation_selector_glyph: u32,
    seed_script: u32,
    beginning_of_text: u8,
    has_point_size: u8,
    report_unsafe_to_concat: u8,
    report_tatweel_positions: u8,
    suppress_dotted_circle: u8,
    has_invisible_glyph: u8,
    has_not_found_variation_selector_glyph: u8,
    point_size: f64,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<ShapeOptionsC>() == 112 && core::mem::offset_of!(ShapeOptionsC, point_size) == 104);

// Every default is zero.
impl Options for ShapeOptionsC {
    unsafe fn set_defaults(_: *mut Self) {}
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shape_options_default(out: *mut ShapeOptionsC, size: usize) -> Status {
    unsafe { defaults_into(out, size) }
}

pub const DIRECTION_DEFAULT: i32 = 0;
pub const DIRECTION_LTR: i32 = 1;
pub const DIRECTION_RTL: i32 = 2;
pub const DIRECTION_TTB: i32 = 3;
pub const DIRECTION_BTT: i32 = 4;

fn direction(code: i32) -> Result<Option<Direction>, Status> {
    match code {
        DIRECTION_DEFAULT => Ok(None),
        DIRECTION_LTR => Ok(Some(Direction::LeftToRight)),
        DIRECTION_RTL => Ok(Some(Direction::RightToLeft)),
        DIRECTION_TTB => Ok(Some(Direction::TopToBottom)),
        DIRECTION_BTT => Ok(Some(Direction::BottomToTop)),
        _ => Err(invalid(format_args!("direction {code} is none of DAEGUN_DIRECTION_*"))),
    }
}

pub const CLUSTER_MONOTONE_GRAPHEMES: i32 = 0;
pub const CLUSTER_MONOTONE_CHARACTERS: i32 = 1;
pub const CLUSTER_CHARACTERS: i32 = 2;
pub const CLUSTER_GRAPHEMES: i32 = 3;

fn cluster_level(code: i32) -> Result<ClusterLevel, Status> {
    match code {
        CLUSTER_MONOTONE_GRAPHEMES => Ok(ClusterLevel::MonotoneGraphemes),
        CLUSTER_MONOTONE_CHARACTERS => Ok(ClusterLevel::MonotoneCharacters),
        CLUSTER_CHARACTERS => Ok(ClusterLevel::Characters),
        CLUSTER_GRAPHEMES => Ok(ClusterLevel::Graphemes),
        _ => Err(invalid(format_args!("cluster level {code} is none of DAEGUN_CLUSTER_*"))),
    }
}

pub const IGNORABLES_HIDE: i32 = 0;
pub const IGNORABLES_REMOVE: i32 = 1;
pub const IGNORABLES_PRESERVE: i32 = 2;

fn ignorables(code: i32) -> Result<Ignorables, Status> {
    match code {
        IGNORABLES_HIDE => Ok(Ignorables::Hide),
        IGNORABLES_REMOVE => Ok(Ignorables::Remove),
        IGNORABLES_PRESERVE => Ok(Ignorables::Preserve),
        _ => Err(invalid(format_args!("ignorables {code} is none of DAEGUN_IGNORABLES_*"))),
    }
}

// As many features as a call converts on the stack; more go to the heap.
const STACK_FEATURES: usize = 16;

// An end of 0 or UINT32_MAX runs to the text's end, so a feature zeroed but for its tag and value covers it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_from_string(
    text: *const c_char,
    text_len: usize,
    out: *mut FeatureC,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let text = or_return!(unsafe { text_of(text, text_len) });
    let feature: Feature = match text.parse() {
        Ok(feature) => feature,
        Err(e) => return invalid(e),
    };
    let wide = |at: usize| u32::try_from(at).unwrap_or(u32::MAX);
    // C reads an end of 0 as the text's end, so an empty range ending at 0 is written as 1..1, still empty.
    let (start, end) = match feature.bytes.end {
        0 => (1, 1),
        end => (wide(feature.bytes.start), wide(end)),
    };
    unsafe { put(out, FeatureC { tag: feature.tag.0, value: feature.value, start, end }) }
}

pub(crate) fn feature_of(f: &FeatureC) -> Feature {
    let end = if f.end == 0 || f.end == u32::MAX { usize::MAX } else { f.end as usize };
    Feature { bytes: core::range::Range { start: f.start as usize, end }, ..Feature::new(Tag(f.tag), f.value) }
}

// A call given C's options as Rust's: NULL is Rust's defaults as they stand, which converting C's would build again.
// Out of line, as in line its frame was a shaped run's caller's to pay, 5 ns of `daegun_font_shape`.
#[inline(never)]
unsafe fn with_options<R>(opts: *const ShapeOptionsC, with: impl FnOnce(&ShapeOptions) -> R) -> Result<R, Status> {
    if opts.is_null() {
        return Ok(with(&ShapeOptions::DEFAULT));
    }
    let opts = unsafe { options_of(opts) }?;
    let given = unsafe { slice_of(opts.features, opts.features_len) }.ok_or(Status::Null)?;
    if given.len() > STACK_FEATURES {
        let features: Vec<Feature> = given.iter().map(feature_of).collect();
        return Ok(with(&shape_options(&opts, &features)?));
    }
    // Only the slots given are written, so a call with none pays nothing for the room.
    let mut stack = [const { core::mem::MaybeUninit::<Feature>::uninit() }; STACK_FEATURES];
    stack.iter_mut().zip(given).for_each(|(slot, f)| {
        slot.write(feature_of(f));
    });
    let features = unsafe { core::slice::from_raw_parts(stack.as_ptr().cast::<Feature>(), given.len()) };
    Ok(with(&shape_options(&opts, features)?))
}

// Rust's options from C's, borrowing the features converted from them.
fn shape_options<'a>(opts: &'a ShapeOptionsC, features: &'a [Feature]) -> Result<ShapeOptions<'a>, Status> {
    let glyph = |has: u8, gid: u32| (has != 0).then_some(GlyphId(gid));
    Ok(ShapeOptions {
        direction: direction(opts.direction)?,
        script: crate::ffi::opt_tag(opts.script),
        language: unsafe { ident_of(opts.language) }?,
        features,
        cluster_level: cluster_level(opts.cluster_level)?,
        before: unsafe { text_of(opts.before, opts.before_len) }?,
        after: unsafe { text_of(opts.after, opts.after_len) }?,
        beginning_of_text: opts.beginning_of_text != 0,
        point_size: (opts.has_point_size != 0).then_some(opts.point_size),
        report_unsafe_to_concat: opts.report_unsafe_to_concat != 0,
        report_tatweel_positions: opts.report_tatweel_positions != 0,
        ignorables: ignorables(opts.ignorables)?,
        suppress_dotted_circle: opts.suppress_dotted_circle != 0,
        invisible_glyph: glyph(opts.has_invisible_glyph, opts.invisible_glyph),
        seed_script: crate::ffi::opt_tag(opts.seed_script).map(crate::ffi::unicode::script_of).transpose()?,
        not_found_glyph: GlyphId(opts.not_found_glyph),
        not_found_variation_selector_glyph: glyph(
            opts.has_not_found_variation_selector_glyph,
            opts.not_found_variation_selector_glyph,
        ),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cluster_level_is_graphemes(level: i32, out: *mut bool) -> Status {
    let level = or_return!(cluster_level(level));
    unsafe { put(out, level.is_graphemes()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cluster_level_is_monotone(level: i32, out: *mut bool) -> Status {
    let level = or_return!(cluster_level(level));
    unsafe { put(out, level.is_monotone()) }
}

// shaping

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_shape(
    font: *const Font,
    text: *const c_char,
    text_len: usize,
    location: *const LocationHandle,
    opts: *const ShapeOptionsC,
    out: *mut *mut ShapedRun,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(text) = (unsafe { slice_of(text.cast::<u8>(), text_len) }) else { return Status::Null };
    let at = unsafe { crate::ffi::at(location) };
    // NULL shapes with the defaults in place and no closure: a run's caller waited on the stores of either, 40 ns.
    let shaped = match opts.is_null() {
        true => Ok(font.shape_bytes(text, at, &ShapeOptions::DEFAULT)),
        false => unsafe { with_options(opts, |o| font.shape_bytes(text, at, o)) },
    };
    // The text is read as UTF-8 only where the cache misses, after the options, so its fault is still answered first;
    // None is only text that isn't.
    let run = match shaped {
        Ok(Some(run)) => run,
        Ok(None) => return core::str::from_utf8(text).map_or_else(not_utf8, |_| Status::Ok),
        Err(status) => return core::str::from_utf8(text).map_or_else(not_utf8, |_| status),
    };
    unsafe { deliver_run(out, run) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_shape_into(
    font: *const Font,
    text: *const c_char,
    text_len: usize,
    location: *const LocationHandle,
    opts: *const ShapeOptionsC,
    run: *mut *mut ShapedRun,
) -> Status {
    if run.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let text = or_return!(unsafe { text_of(text, text_len) });
    let at = unsafe { crate::ffi::at(location) };
    // The caller's run, its count taken back, or a new one: copied first only where the shape cache shares it.
    let shaped = unsafe {
        with_options(opts, |o| {
            let held = *run;
            let mut owned = if held.is_null() { Arc::default() } else { Arc::from_raw(held.cast_const()) };
            font.shape_into(text, at, o, Arc::make_mut(&mut owned));
            *run = Arc::into_raw(owned).cast_mut();
        })
    };
    or_return!(shaped);
    Status::Ok
}

pub struct BidiRuns(Vec<BidiRun>);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_shape_bidi(
    font: *const Font,
    text: *const c_char,
    text_len: usize,
    location: *const LocationHandle,
    base: i32,
    opts: *const ShapeOptionsC,
    out: *mut *mut BidiRuns,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let text = or_return!(unsafe { text_of(text, text_len) });
    let base = or_return!(crate::ffi::base_direction(base));
    let at = unsafe { crate::ffi::at(location) };
    let runs = or_return!(unsafe { with_options(opts, |o| font.shape_bidi(text, at, base, o)) });
    unsafe { deliver(out, BidiRuns(runs)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_bidi_runs_count(runs: *const BidiRuns, out: *mut usize) -> Status {
    let Some(runs) = (unsafe { borrow(runs) }) else { return Status::Null };
    unsafe { put(out, runs.0.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_bidi_runs_at(
    runs: *const BidiRuns,
    index: usize,
    out_run: *mut *const ShapedRun,
    out_level: *mut u8,
    out_start: *mut usize,
    out_end: *mut usize,
) -> Status {
    let Some(runs) = (unsafe { borrow(runs) }) else { return Status::Null };
    let Some(run) = runs.0.get(index) else { return past_end("bidi run", index, runs.0.len()) };
    unsafe {
        fill(out_run, Arc::as_ptr(&run.run));
        fill(out_level, run.level);
        fill(out_start, run.bytes.start);
        fill(out_end, run.bytes.end);
    }
    Status::Ok
}

free!(daegun_bidi_runs_free, BidiRuns);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_caret_positions(
    font: *const Font,
    text: *const c_char,
    text_len: usize,
    location: *const LocationHandle,
    orientation: i32,
    out: *mut f64,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    let text = or_return!(unsafe { text_of(text, text_len) });
    or_return!(apart(&[span(text.as_ptr(), text.len()), span(out, out_len), span(out_count, 1)]));
    let orientation = or_return!(crate::ffi::orientation(orientation));
    let carets = font.caret_positions(text, unsafe { crate::ffi::at(location) }, orientation);
    unsafe { into_buffer(carets.into_iter(), out, out_len, out_count) }
}

#[cfg(test)]
#[path = "tests/shape.rs"]
mod tests;
