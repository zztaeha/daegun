// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

#![allow(unsafe_code)]

mod handle;
mod atlas;
mod color;
mod contours;
mod glyphs;
mod layout;
mod list;
mod metrics;
mod tables;
mod options;
mod outline;
mod pen;
mod prepared;
mod quads;
mod raster;
mod raw;
mod scene;
mod shape;
mod subpixel;

use crate::Font;
use handle::{Status, Str, borrow, deliver, release};

#[unsafe(no_mangle)]
pub extern "C" fn daegun_abi_version() -> u32 {
    // Read from the crate version rather than restated here, because a hand-kept copy drifts out
    // of step with the release it is meant to describe.
    const fn num(s: &str) -> u32 {
        let (b, mut v, mut i) = (s.as_bytes(), 0u32, 0usize);
        while i < b.len() {
            v = v * 10 + (b[i] - b'0') as u32;
            i += 1;
        }
        v
    }
    (num(env!("CARGO_PKG_VERSION_MAJOR")) << 16)
        | (num(env!("CARGO_PKG_VERSION_MINOR")) << 8)
        | num(env!("CARGO_PKG_VERSION_PATCH"))
}

std::thread_local! {
    static LAST_ERROR: core::cell::RefCell<Option<handle::OwnedStr>> =
        const { core::cell::RefCell::new(None) };
}

// A NULL foreground means the caller did not name one, as a NULL opts means the defaults.
pub(crate) unsafe fn rgba_of(p: *const u8) -> Option<crate::paint::Rgba> {
    if p.is_null() {
        return None;
    }
    let c = unsafe { core::slice::from_raw_parts(p, 4) };
    Some(crate::paint::Rgba { r: c[0], g: c[1], b: c[2], a: c[3] })
}

// DAEGUN_RANGE with the reason, for daegun_last_error.
pub(crate) fn range(message: &str) -> handle::Status {
    set_error(message);
    handle::Status::Range
}

// An atexit handler or a thread-exit destructor can run after the thread's storage is gone, where `with`
// would panic: a reason is then dropped, and none is given.
pub(crate) fn set_error(message: &str) {
    let _ = LAST_ERROR.try_with(|slot| *slot.borrow_mut() = Some(handle::OwnedStr::new(message)));
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_last_error() -> Str {
    LAST_ERROR
        .try_with(|slot| slot.borrow().as_ref().map_or(Str::EMPTY, handle::OwnedStr::as_str))
        .unwrap_or(Str::EMPTY)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_open(
    data: *const u8,
    len: usize,
    out: *mut *mut Font,
) -> Status {
    if data.is_null() || out.is_null() {
        return Status::Null;
    }
    let bytes = unsafe { core::slice::from_raw_parts(data, len) };
    match Font::from_bytes(bytes) {
        Ok(font) => unsafe { deliver(out, font) },
        Err(e) => {
            set_error(&alloc::format!("{e}"));
            Status::Parse
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_font_buffer_new(len: usize) -> *mut u8 {
    let Ok(layout) = core::alloc::Layout::array::<u8>(len) else { return core::ptr::null_mut() };
    if len == 0 {
        return core::ptr::null_mut();
    }
    // Exactly `len` bytes, the layout `Vec::from_raw_parts` rebuilds in the two calls that take it back.
    // Null when the allocation fails, where `vec!` would abort the caller.
    unsafe { alloc::alloc::alloc_zeroed(layout) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_buffer_free(data: *mut u8, len: usize) {
    if data.is_null() || len == 0 {
        return;
    }
    // `daegun_font_buffer_new` is the only source and allocates capacity exactly `len`, which is
    // what makes reconstructing the Vec sound. A pointer from anywhere else is undefined behavior.
    drop(unsafe { alloc::vec::Vec::from_raw_parts(data, len, len) });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_open_owned(
    data: *mut u8,
    len: usize,
    out: *mut *mut Font,
) -> Status {
    if data.is_null() || len == 0 {
        return Status::Null;
    }
    // Taken before `out` is checked, because the header hands the buffer over whatever this returns.
    let bytes = unsafe { alloc::vec::Vec::from_raw_parts(data, len, len) };
    if out.is_null() {
        return Status::Null;
    }
    match Font::from_vec(bytes) {
        Ok(font) => unsafe { deliver(out, font) },
        Err(e) => {
            set_error(&alloc::format!("{e}"));
            Status::Parse
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_open_collection(
    data: *const u8,
    len: usize,
    index: usize,
    out: *mut *mut Font,
) -> Status {
    if data.is_null() || out.is_null() {
        return Status::Null;
    }
    let bytes = unsafe { core::slice::from_raw_parts(data, len) };
    match Font::from_ttc(bytes, index) {
        Ok(font) => unsafe { deliver(out, font) },
        Err(e) => {
            set_error(&alloc::format!("{e}"));
            Status::Parse
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_free(font: *mut Font) {
    unsafe { release(font) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_id(
    font: *const Font,
    codepoint: u32,
    out: *mut u16,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    match font.glyph_id(codepoint) {
        Some(gid) => {
            unsafe { *out = gid };
            Status::Ok
        }
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_num_glyphs(font: *const Font, out: *mut u16) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.num_glyphs() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_upm(font: *const Font, out: *mut u16) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = font.upm() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ttc_font_count(
    data: *const u8,
    len: usize,
    out: *mut usize,
) -> Status {
    if data.is_null() || out.is_null() {
        return Status::Null;
    }
    let bytes = unsafe { core::slice::from_raw_parts(data, len) };
    unsafe { *out = Font::ttc_font_count(bytes) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_set_outline_cache_bytes(font: *const Font, bytes: usize) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    font.set_outline_cache_bytes(bytes);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_outline_cache_stats(
    font: *const Font,
    out_count: *mut usize,
    out_bytes: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let (count, bytes) = font.outline_cache_stats();
    if !out_count.is_null() {
        unsafe { *out_count = count };
    }
    if !out_bytes.is_null() {
        unsafe { *out_bytes = bytes };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_set_shape_cache_bytes(font: *const Font, bytes: usize) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    font.set_shape_cache_bytes(bytes);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_clear_shape_cache(font: *const Font) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    font.clear_shape_cache();
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_shape_cache_stats(
    font: *const Font,
    out_count: *mut usize,
    out_bytes: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let (count, bytes) = font.shape_cache_stats();
    if !out_count.is_null() {
        unsafe { *out_count = count };
    }
    if !out_bytes.is_null() {
        unsafe { *out_bytes = bytes };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_set_instance_cache_bytes(font: *const Font, bytes: usize) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    font.set_instance_cache_bytes(bytes);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_instance_cache_stats(
    font: *const Font,
    out_fonts: *mut usize,
    out_tables: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let (fonts, tables) = font.instance_cache_stats();
    if !out_fonts.is_null() {
        unsafe { *out_fonts = fonts };
    }
    if !out_tables.is_null() {
        unsafe { *out_tables = tables };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_set_cmap_index_allowance(font: *const Font, bytes: usize) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    font.set_cmap_index_allowance(bytes);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_cmap_index_allowance(
    font: *const Font,
    out_bytes: *mut usize,
) -> Status {
    if out_bytes.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { *out_bytes = font.cmap_index_allowance() };
    Status::Ok
}

const _: () = {
    assert!(size_of::<Status>() == 4);
    assert!(align_of::<Status>() == 4);
    assert!(size_of::<*mut Font>() == size_of::<usize>());
};

#[cfg(test)]
mod tests {
    use super::{layout, options, scene, shape, tables};
    use crate::paint::gradient::Interpolation;
    use crate::{
        Align, BreakStrategy, Cap, ClusterLevel, HintMode, Ignorables, Join, MathKernCorner, StripeOrder,
        SubpixelLayout, TextOrientation, WritingMode,
    };

    // Every variant C can ask for reads back from its own code. Each `match` names every variant, so
    // one that Rust gains does not build until C has a code for it.
    #[test]
    fn every_variant_has_a_c_code_that_reads_back_as_it() {
        for v in [Align::Start, Align::End, Align::Center, Align::Justify] {
            let code = match v {
                Align::Start => layout::ALIGN_START,
                Align::End => layout::ALIGN_END,
                Align::Center => layout::ALIGN_CENTER,
                Align::Justify => layout::ALIGN_JUSTIFY,
            };
            assert_eq!(layout::align_of(code), v);
        }
        for v in [WritingMode::Horizontal, WritingMode::VerticalRl, WritingMode::VerticalLr] {
            let code = match v {
                WritingMode::Horizontal => layout::WRITING_HORIZONTAL,
                WritingMode::VerticalRl => layout::WRITING_VERTICAL_RL,
                WritingMode::VerticalLr => layout::WRITING_VERTICAL_LR,
            };
            assert_eq!(layout::writing_mode_of(code), v);
        }
        for v in [TextOrientation::Mixed, TextOrientation::Upright, TextOrientation::Sideways] {
            let code = match v {
                TextOrientation::Mixed => layout::ORIENTATION_MIXED,
                TextOrientation::Upright => layout::ORIENTATION_UPRIGHT,
                TextOrientation::Sideways => layout::ORIENTATION_SIDEWAYS,
            };
            assert_eq!(layout::orientation_of(code), v);
        }
        for v in [BreakStrategy::Greedy, BreakStrategy::Optimal] {
            let code = match v {
                BreakStrategy::Greedy => layout::BREAK_GREEDY,
                BreakStrategy::Optimal => layout::BREAK_OPTIMAL,
            };
            assert_eq!(layout::strategy_of(code), v);
        }
        for v in [
            ClusterLevel::MonotoneGraphemes,
            ClusterLevel::MonotoneCharacters,
            ClusterLevel::Characters,
            ClusterLevel::Graphemes,
        ] {
            let code = match v {
                ClusterLevel::MonotoneGraphemes => shape::CLUSTER_MONOTONE_GRAPHEMES,
                ClusterLevel::MonotoneCharacters => shape::CLUSTER_MONOTONE_CHARACTERS,
                ClusterLevel::Characters => shape::CLUSTER_CHARACTERS,
                ClusterLevel::Graphemes => shape::CLUSTER_GRAPHEMES,
            };
            assert_eq!(shape::cluster_of(code), v);
        }
        for v in [Ignorables::Hide, Ignorables::Remove, Ignorables::Preserve] {
            let code = match v {
                Ignorables::Hide => shape::IGNORABLES_HIDE,
                Ignorables::Remove => shape::IGNORABLES_REMOVE,
                Ignorables::Preserve => shape::IGNORABLES_PRESERVE,
            };
            assert_eq!(shape::ignorables_of(code), v);
        }
        for v in [HintMode::None, HintMode::Subpixel, HintMode::Classic, HintMode::Auto, HintMode::AutoForce] {
            let code = match v {
                HintMode::None => options::HINT_NONE,
                HintMode::Subpixel => options::HINT_SUBPIXEL,
                HintMode::Classic => options::HINT_CLASSIC,
                HintMode::Auto => options::HINT_AUTO,
                HintMode::AutoForce => options::HINT_AUTO_FORCE,
            };
            assert_eq!(options::hint_of(code), v);
        }
        for v in [Cap::Butt, Cap::Round, Cap::Square] {
            let code = match v {
                Cap::Butt => options::CAP_BUTT,
                Cap::Round => options::CAP_ROUND,
                Cap::Square => options::CAP_SQUARE,
            };
            assert_eq!(options::cap_of(code), v);
        }
        for v in [Join::Miter { limit: 4.0 }, Join::Round, Join::Bevel] {
            let code = match v {
                Join::Miter { limit: _ } => options::JOIN_MITER,
                Join::Round => options::JOIN_ROUND,
                Join::Bevel => options::JOIN_BEVEL,
            };
            assert_eq!(options::join_of(code, 4.0), v);
        }
        for v in [StripeOrder::Rgb, StripeOrder::Bgr] {
            let code = match v {
                StripeOrder::Rgb => options::LAYOUT_RGB_H,
                StripeOrder::Bgr => options::LAYOUT_BGR_H,
            };
            assert_eq!(options::layout_of(code).key(), SubpixelLayout::horizontal(v).key());
        }
        for v in [
            MathKernCorner::TopRight,
            MathKernCorner::TopLeft,
            MathKernCorner::BottomRight,
            MathKernCorner::BottomLeft,
        ] {
            let code = match v {
                MathKernCorner::TopRight => tables::MATH_KERN_TOP_RIGHT,
                MathKernCorner::TopLeft => tables::MATH_KERN_TOP_LEFT,
                MathKernCorner::BottomRight => tables::MATH_KERN_BOTTOM_RIGHT,
                MathKernCorner::BottomLeft => tables::MATH_KERN_BOTTOM_LEFT,
            };
            assert_eq!(tables::corner_of(code), Some(v));
        }
        for v in [Interpolation::LinearLight, Interpolation::Srgb] {
            let code = match v {
                Interpolation::LinearLight => scene::INTERPOLATE_LINEAR_LIGHT,
                Interpolation::Srgb => scene::INTERPOLATE_SRGB,
            };
            assert_eq!(scene::interpolation_of(code), Some(v));
        }
    }
}
