// tables.h: subsetting with its glyph closure, BASE, and the tags GSUB and GPOS offer.

use alloc::vec::Vec;
use core::ffi::c_char;

use crate::ffi::font::LocationHandle;
use crate::ffi::handle::{
    Status, apart, borrow, cleared, deliver, deliver_result, failed, field_view, free, into_buffer, or_return,
    put_some, slice_of, span, text_of,
};
use crate::ffi::opt_tag;
use crate::tables::Subset;
use crate::{Font, GlyphId, Tag};

// A caller's glyph ids as Rust's: GlyphId is a transparent u32.
pub(crate) fn glyphs_of(gids: &[u32]) -> &[GlyphId] {
    unsafe { core::slice::from_raw_parts(gids.as_ptr().cast::<GlyphId>(), gids.len()) }
}

// Tags into the caller's buffer.
unsafe fn tags_into(tags: Vec<Tag>, out: *mut u32, out_len: usize, out_count: *mut usize) -> Status {
    unsafe { into_buffer(tags.into_iter().map(|t| t.0), out, out_len, out_count) }
}

// subsetting

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_subset(
    font: *const Font,
    gids: *const u32,
    gids_len: usize,
    location: *const LocationHandle,
    out: *mut *mut Subset,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(gids) = (unsafe { slice_of(gids, gids_len) }) else { return Status::Null };
    unsafe { deliver_result(out, font.subset(glyphs_of(gids), crate::ffi::at(location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_subset_text(
    font: *const Font,
    text: *const c_char,
    text_len: usize,
    location: *const LocationHandle,
    out: *mut *mut Subset,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let text = or_return!(unsafe { text_of(text, text_len) });
    unsafe { deliver_result(out, font.subset_text(text, crate::ffi::at(location))) }
}

field_view!(daegun_subset_bytes, Subset, font, u8);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subset_renumbered(subset: *const Subset, old: u32, out: *mut u32) -> Status {
    let Some(subset) = (unsafe { borrow(subset) }) else { return Status::Null };
    unsafe { put_some(out, subset.renumbered(GlyphId(old)).map(|g| g.0)) }
}

free!(daegun_subset_free, Subset);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_closure(
    font: *const Font,
    gids: *const u32,
    gids_len: usize,
    location: *const LocationHandle,
    out: *mut u32,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    let Some(gids) = (unsafe { slice_of(gids, gids_len) }) else { return Status::Null };
    or_return!(apart(&[span(gids.as_ptr(), gids_len), span(out, out_len), span(out_count, 1)]));
    let closure = or_return!(font.glyph_closure(glyphs_of(gids), unsafe { crate::ffi::at(location) }).map_err(failed));
    unsafe { into_buffer(closure.into_iter().map(|g| g.0), out, out_len, out_count) }
}

// BASE

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BaselineC {
    tag: u32,
    position: f64,
}

// Rust's map of baselines laid out as C reads them.
pub struct BaseScriptC {
    default_baseline: Option<Tag>,
    baselines: Vec<BaselineC>,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ExtentsC {
    min: f64,
    max: f64,
    has_min: bool,
    has_max: bool,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<BaselineC>() == 16 && size_of::<ExtentsC>() == 24);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_base_script(
    font: *const Font,
    script: u32,
    location: *const LocationHandle,
    orientation: i32,
    out: *mut *mut BaseScriptC,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let orientation = or_return!(crate::ffi::orientation(orientation));
    let Some(found) = font.base_script(Tag(script), unsafe { crate::ffi::at(location) }, orientation) else {
        return Status::Absent;
    };
    let baselines = found.baselines.iter().map(|(t, &position)| BaselineC { tag: t.0, position }).collect();
    unsafe { deliver(out, BaseScriptC { default_baseline: found.default_baseline, baselines }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_base_script_default_baseline(script: *const BaseScriptC, out: *mut u32) -> Status {
    let Some(script) = (unsafe { borrow(script) }) else { return Status::Null };
    unsafe { put_some(out, script.default_baseline.map(|t| t.0)) }
}

field_view!(daegun_base_script_baselines, BaseScriptC, baselines, BaselineC);
free!(daegun_base_script_free, BaseScriptC);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_base_extents(
    font: *const Font,
    script: u32,
    location: *const LocationHandle,
    orientation: i32,
    language_system: u32,
    feature: u32,
    out: *mut ExtentsC,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let orientation = or_return!(crate::ffi::orientation(orientation));
    let at = unsafe { crate::ffi::at(location) };
    let extents = font.base_extents(Tag(script), at, orientation, opt_tag(language_system), opt_tag(feature));
    let extents = extents.map(|e| ExtentsC {
        min: e.min.unwrap_or(0.0),
        max: e.max.unwrap_or(0.0),
        has_min: e.min.is_some(),
        has_max: e.max.is_some(),
    });
    unsafe { put_some(out, extents) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_has_glyph_free_base(font: *const Font, out: *mut bool) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { crate::ffi::handle::put(out, font.has_glyph_free_base()) }
}

// layout tags

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_script_tags(
    font: *const Font,
    out: *mut u32,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { tags_into(font.script_tags(), out, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_language_system_tags(
    font: *const Font,
    script: u32,
    out: *mut u32,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { tags_into(font.language_system_tags(Tag(script)), out, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_feature_tags(
    font: *const Font,
    script: u32,
    language_system: u32,
    out: *mut u32,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { tags_into(font.feature_tags(opt_tag(script), opt_tag(language_system)), out, out_len, out_count) }
}

#[cfg(test)]
#[path = "tests/tables.rs"]
mod tests;
