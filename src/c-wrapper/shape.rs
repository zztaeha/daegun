// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use alloc::vec::Vec;
use core::ffi::c_char;

use crate::{ClusterLevel, Font, Ignorables, ShapeOptions};

use crate::ffi::handle::{OwnedStr, Status, Str, borrow, deliver, release, slice_of, str_of};
use crate::ffi::list::{Axis, axes_of};

pub struct Run {
    glyphs: Vec<u16>,
    advances: Vec<f64>,
    offsets: Vec<f64>,
    clusters: Vec<u32>,
    unsafe_to_break: Vec<u8>,
    unsafe_to_concat: Vec<u8>,
    safe_to_insert_tatweel: Vec<u8>,
    complete: bool,
    has_broken_syllable: bool,
    shaper: OwnedStr,
}

impl Run {
    pub(crate) fn of(r: &crate::ShapedRun) -> Run {
        let crate::ShapedRun {
            glyphs,
            advances,
            offsets,
            unsafe_to_break,
            unsafe_to_concat,
            safe_to_insert_tatweel,
            clusters,
            complete,
            has_broken_syllable,
            shaper,
        } = r;
        let bytes = |flags: &[bool]| flags.iter().map(|&b| u8::from(b)).collect();
        Run {
            glyphs: glyphs.clone(),
            advances: advances.clone(),
            offsets: offsets.iter().flat_map(|&(x, y)| [x, y]).collect(),
            clusters: clusters.clone(),
            unsafe_to_break: bytes(unsafe_to_break),
            unsafe_to_concat: bytes(unsafe_to_concat),
            safe_to_insert_tatweel: bytes(safe_to_insert_tatweel),
            complete: *complete,
            has_broken_syllable: *has_broken_syllable,
            shaper: OwnedStr::new(shaper),
        }
    }
}

macro_rules! run_view {
    ($fn_name:ident, $field:ident, $elem:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(run: *const Run, out_count: *mut usize) -> *const $elem {
            let Some(r) = (unsafe { borrow(run) }) else { return core::ptr::null() };
            if out_count.is_null() {
                return core::ptr::null();
            }
            unsafe { *out_count = r.$field.len() };
            r.$field.as_ptr()
        }
    };
}

run_view!(daegun_run_glyphs, glyphs, u16);
run_view!(daegun_run_advances, advances, f64);
run_view!(daegun_run_offsets, offsets, f64);
run_view!(daegun_run_clusters, clusters, u32);
run_view!(daegun_run_unsafe_to_break, unsafe_to_break, u8);
run_view!(daegun_run_unsafe_to_concat, unsafe_to_concat, u8);
run_view!(daegun_run_safe_to_insert_tatweel, safe_to_insert_tatweel, u8);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_run_complete(run: *const Run, out: *mut bool) -> Status {
    let Some(r) = (unsafe { borrow(run) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = r.complete };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_run_has_broken_syllable(
    run: *const Run,
    out: *mut bool,
) -> Status {
    let Some(r) = (unsafe { borrow(run) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = r.has_broken_syllable };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_run_shaper(run: *const Run, out: *mut Str) -> Status {
    let Some(r) = (unsafe { borrow(run) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = r.shaper.as_str() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_run_free(run: *mut Run) {
    unsafe { release(run) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Feature {
    pub tag: *const c_char,
    pub value: u32,
}
const _: () = assert!(size_of::<Feature>() == 16);

pub(crate) unsafe fn features_of<'a>(features: *const Feature, len: usize) -> Option<Vec<(&'a str, u32)>> {
    let slice = unsafe { slice_of(features, len) }?;
    Some(slice.iter().filter_map(|f| unsafe { str_of(f.tag) }.map(|t| (t, f.value))).collect())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_shape(
    font: *const Font,
    text: *const c_char,
    axes: *const Axis,
    axes_len: usize,
    vertical: bool,
    out: *mut *mut Run,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(text) = (unsafe { str_of(text) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let Some(r) = font.shape(text, &location, vertical) else { return Status::Absent };
    unsafe { deliver(out, Run::of(&r)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_shape_with_language(
    font: *const Font,
    text: *const c_char,
    axes: *const Axis,
    axes_len: usize,
    vertical: bool,
    language: *const c_char,
    out: *mut *mut Run,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(text) = (unsafe { str_of(text) }) else { return Status::Null };
    let Some(language) = (unsafe { str_of(language) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let Some(r) = font.shape_with_language(text, &location, vertical, language) else {
        return Status::Absent;
    };
    unsafe { deliver(out, Run::of(&r)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_shape_with_features(
    font: *const Font,
    text: *const c_char,
    axes: *const Axis,
    axes_len: usize,
    vertical: bool,
    script: *const c_char,
    features: *const Feature,
    features_len: usize,
    out: *mut *mut Run,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(text) = (unsafe { str_of(text) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let script = unsafe { str_of(script) };
    let Some(feats) = (unsafe { features_of(features, features_len) }) else { return Status::Null };
    let Some(r) = font.shape_with_features(text, &location, vertical, script, &feats) else {
        return Status::Absent;
    };
    unsafe { deliver(out, Run::of(&r)) }
}

pub const CLUSTER_MONOTONE_GRAPHEMES: i32 = 0;
pub const CLUSTER_MONOTONE_CHARACTERS: i32 = 1;
pub const CLUSTER_CHARACTERS: i32 = 2;
pub const CLUSTER_GRAPHEMES: i32 = 3;

// The point size a run is tracked at when its options give none, in whole points as C reads it.
pub const DEFAULT_POINT_SIZE: i32 = 12;
const _: () = assert!(DEFAULT_POINT_SIZE as f64 == crate::DEFAULT_POINT_SIZE);

pub const IGNORABLES_HIDE: i32 = 0;
pub const IGNORABLES_REMOVE: i32 = 1;
pub const IGNORABLES_PRESERVE: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ShapeOptionsC {
    pub cluster_level: i32,
    pub ignorables: i32,
    pub before: *const c_char,
    pub after: *const c_char,
    pub beginning_of_text: bool,
    pub has_point_size: bool,
    pub point_size: f64,
    pub features: *const Feature,
    pub features_len: usize,
    pub script: *const c_char,
    pub language: *const c_char,
    pub report_unsafe_to_concat: bool,
    pub report_tatweel_positions: bool,
    pub suppress_dotted_circle: bool,
    pub has_invisible_glyph: bool,
    pub invisible_glyph: u16,
    pub has_seed_script: bool,
    pub seed_script: u16,
}
const _: () = assert!(size_of::<ShapeOptionsC>() == 88);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shape_options_default(out: *mut ShapeOptionsC) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe {
        *out = ShapeOptionsC {
            cluster_level: CLUSTER_MONOTONE_GRAPHEMES,
            ignorables: IGNORABLES_HIDE,
            before: core::ptr::null(),
            after: core::ptr::null(),
            beginning_of_text: false,
            has_point_size: false,
            point_size: 0.0,
            features: core::ptr::null(),
            features_len: 0,
            script: core::ptr::null(),
            language: core::ptr::null(),
            report_unsafe_to_concat: false,
            report_tatweel_positions: false,
            suppress_dotted_circle: false,
            has_invisible_glyph: false,
            invisible_glyph: 0,
            has_seed_script: false,
            seed_script: 0,
        }
    };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_shape_with_options(
    font: *const Font,
    text: *const c_char,
    axes: *const Axis,
    axes_len: usize,
    vertical: bool,
    opts: *const ShapeOptionsC,
    out: *mut *mut Run,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(text) = (unsafe { str_of(text) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };

    let feats;
    let built = match unsafe { borrow(opts) } {
        None => ShapeOptions::default(),
        Some(o) => {
            let Some(f) = (unsafe { features_of(o.features, o.features_len) }) else { return Status::Null };
            feats = f;
            unsafe { options_of(o, &feats) }
        }
    };

    let Some(r) = font.shape_with_options(text, &location, vertical, &built) else {
        return Status::Absent;
    };
    unsafe { deliver(out, Run::of(&r)) }
}

// `features` is the caller's, read from `o.features` with `features_of`, as it outlives this call.
pub(crate) unsafe fn options_of<'a>(o: &ShapeOptionsC, features: &'a [(&'a str, u32)]) -> ShapeOptions<'a> {
    ShapeOptions {
        cluster_level: cluster_of(o.cluster_level),
        before: unsafe { str_of(o.before) }.unwrap_or(""),
        after: unsafe { str_of(o.after) }.unwrap_or(""),
        beginning_of_text: o.beginning_of_text,
        point_size: o.has_point_size.then_some(o.point_size),
        features,
        script: unsafe { str_of(o.script) },
        language: unsafe { str_of(o.language) },
        report_unsafe_to_concat: o.report_unsafe_to_concat,
        report_tatweel_positions: o.report_tatweel_positions,
        ignorables: ignorables_of(o.ignorables),
        suppress_dotted_circle: o.suppress_dotted_circle,
        invisible_glyph: o.has_invisible_glyph.then_some(o.invisible_glyph),
        seed_script: o.has_seed_script.then_some(crate::Script(o.seed_script)),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_measure_width(
    font: *const Font,
    text: *const c_char,
    axes: *const Axis,
    axes_len: usize,
    font_size: f64,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(text) = (unsafe { str_of(text) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    unsafe { *out = font.measure_width(text, &location, font_size) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_justification_extenders(
    font: *const Font,
    script_tag: *const c_char,
    out: *mut *mut crate::ffi::list::U16List,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(tag) = (unsafe { str_of(script_tag) }) else { return Status::Null };
    unsafe { deliver(out, crate::ffi::list::U16List(font.justification_extenders(tag))) }
}

pub(crate) fn cluster_of(code: i32) -> ClusterLevel {
    match code {
        CLUSTER_MONOTONE_CHARACTERS => ClusterLevel::MonotoneCharacters,
        CLUSTER_CHARACTERS => ClusterLevel::Characters,
        CLUSTER_GRAPHEMES => ClusterLevel::Graphemes,
        _ => ClusterLevel::MonotoneGraphemes,
    }
}

pub(crate) fn ignorables_of(code: i32) -> Ignorables {
    match code {
        IGNORABLES_REMOVE => Ignorables::Remove,
        IGNORABLES_PRESERVE => Ignorables::Preserve,
        _ => Ignorables::Hide,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cluster_level_is_graphemes(level: i32, out: *mut i32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = i32::from(cluster_of(level).is_graphemes()) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cluster_level_is_monotone(level: i32, out: *mut i32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = i32::from(cluster_of(level).is_monotone()) };
    Status::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    // "(" alone in an Arabic font, seeded Arabic as a layout seeds punctuation between Arabic words:
    // C shapes it as Rust's seeded run is, the Arabic shaper and the mirrored parenthesis.
    #[test]
    fn a_seed_script_reaches_the_shaper() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/scheherazade-new/ScheherazadeNew-Regular.ttf")).unwrap();
        let font = Font::from_bytes(&bytes).unwrap();
        let arabic = crate::script_runs("\u{0633}\u{0644}\u{0627}\u{0645}")[0].script;
        let seeded = ShapeOptions { seed_script: Some(arabic), ..Default::default() };
        let want = font.shape_with_options("(", &[], false, &seeded).unwrap();
        let plain = font.shape_with_options("(", &[], false, &ShapeOptions::default()).unwrap();
        assert_ne!(want.glyphs, plain.glyphs, "seeding changes nothing here to test");

        let mut handle = core::ptr::null_mut();
        assert_eq!(unsafe { crate::ffi::daegun_font_open(bytes.as_ptr(), bytes.len(), &mut handle) }, Status::Ok);
        let mut opts = core::mem::MaybeUninit::<ShapeOptionsC>::uninit();
        assert_eq!(unsafe { daegun_shape_options_default(opts.as_mut_ptr()) }, Status::Ok);
        let mut opts = unsafe { opts.assume_init() };
        assert!(!opts.has_seed_script, "seeded by default");
        opts.has_seed_script = true;
        opts.seed_script = arabic.0;
        let mut run = core::ptr::null_mut();
        let text = c"(".as_ptr();
        let st = unsafe { daegun_font_shape_with_options(handle, text, core::ptr::null(), 0, false, &opts, &mut run) };
        assert_eq!(st, Status::Ok);
        let (mut n, mut shaper) = (0, Str { data: core::ptr::null(), len: 0 });
        let glyphs = unsafe { core::slice::from_raw_parts(daegun_run_glyphs(run, &mut n), n) };
        assert_eq!(glyphs, &want.glyphs[..]);
        assert_eq!(unsafe { daegun_run_shaper(run, &mut shaper) }, Status::Ok);
        let name = unsafe { core::slice::from_raw_parts(shaper.data.cast::<u8>(), shaper.len) };
        assert_eq!(name, want.shaper.as_bytes());
        unsafe { daegun_run_free(run) };
        unsafe { crate::ffi::daegun_font_free(handle) };
    }
}
