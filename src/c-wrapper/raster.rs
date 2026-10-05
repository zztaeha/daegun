// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use alloc::vec::Vec;

use crate::Font;

use crate::ffi::handle::{Status, borrow, deliver, release, slice_of};
use crate::ffi::list::{Axis, axes_of};
use crate::ffi::options::hint_of;
use crate::ffi::pen::{Pen, PenBridge};

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_outline_glyph(
    font: *const Font,
    gid: u16,
    pen: *const Pen,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(pen) = (unsafe { borrow(pen) }) else { return Status::Null };
    let mut bridge = PenBridge(*pen);
    outline_status(font, gid, &[], &mut bridge)
}

// ABSENT for a glyph the font does not have, PARSE with the decoder's reason for one it has that does
// not decode.
fn outline_status(font: &Font, gid: u16, axes: &[(&str, f64)], pen: &mut PenBridge) -> Status {
    if gid >= font.num_glyphs() {
        return Status::Absent;
    }
    match font.outline_glyph_drawn(gid, &crate::daecore::cache::canonical_axes(axes), pen) {
        Ok(()) => Status::Ok,
        Err(reason) => {
            crate::ffi::set_error(&reason);
            Status::Parse
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_outline_glyph_instanced(
    font: *const Font,
    gid: u16,
    axes: *const Axis,
    axes_len: usize,
    pen: *const Pen,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(pen) = (unsafe { borrow(pen) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let mut bridge = PenBridge(*pen);
    outline_status(font, gid, &location, &mut bridge)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_prewarm(
    font: *const Font,
    gids: *const u16,
    gids_len: usize,
    axes: *const Axis,
    axes_len: usize,
    out_added: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(ids) = (unsafe { slice_of(gids, gids_len) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let added = font.prewarm(ids.iter().copied(), &location);
    if !out_added.is_null() {
        unsafe { *out_added = added };
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_clear_prewarm(font: *const Font) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    font.clear_prewarm();
    Status::Ok
}

pub struct HintedOutline(crate::HintedOutline);

impl HintedOutline {
    pub fn inner(&self) -> &crate::HintedOutline {
        &self.0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_hinted_glyph(
    font: *const Font,
    gid: u16,
    px: f32,
    axes: *const Axis,
    axes_len: usize,
    hint_mode: i32,
    out: *mut *mut HintedOutline,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let Some(h) = font.hinted_glyph(gid, px, &location, hint_of(hint_mode)) else { return Status::Absent };
    unsafe { deliver(out, HintedOutline(h)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hinted_outline_points(
    outline: *const HintedOutline,
    out_count: *mut usize,
    out_x: *mut *const i32,
    out_y: *mut *const i32,
    out_flags: *mut *const u8,
) -> Status {
    let Some(HintedOutline(crate::HintedOutline { x, y, flags, contour_ends: _ })) = (unsafe { borrow(outline) }) else {
        return Status::Null;
    };
    if out_count.is_null() {
        return Status::Null;
    }
    unsafe {
        *out_count = x.len();
        if !out_x.is_null() {
            *out_x = x.as_ptr();
        }
        if !out_y.is_null() {
            *out_y = y.as_ptr();
        }
        if !out_flags.is_null() {
            *out_flags = flags.as_ptr();
        }
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hinted_outline_contours(
    outline: *const HintedOutline,
    out_count: *mut usize,
) -> *const usize {
    let Some(o) = (unsafe { borrow(outline) }) else { return core::ptr::null() };
    if out_count.is_null() {
        return core::ptr::null();
    }
    unsafe { *out_count = o.0.contour_ends.len() };
    o.0.contour_ends.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hinted_outline_free(outline: *mut HintedOutline) {
    unsafe { release(outline) }
}

pub struct CffHints {
    stems: Vec<f64>,
    masks: Vec<(usize, Vec<u8>)>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_cff_hints(
    font: *const Font,
    gid: u16,
    out: *mut *mut CffHints,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(crate::CffHints { stems, masks }) = font.cff_hints(gid) else { return Status::Absent };
    let stems = stems
        .iter()
        .flat_map(|&crate::CffStem { min, max, vertical }| {
            [f64::from(u8::from(vertical)), f64::from(min), f64::from(max)]
        })
        .collect();
    unsafe { deliver(out, CffHints { stems, masks }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cff_hints_stems(
    hints: *const CffHints,
    out_count: *mut usize,
) -> *const f64 {
    let Some(h) = (unsafe { borrow(hints) }) else { return core::ptr::null() };
    if out_count.is_null() {
        return core::ptr::null();
    }
    unsafe { *out_count = h.stems.len() / 3 };
    h.stems.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cff_hints_mask_count(hints: *const CffHints, out: *mut usize) -> Status {
    let Some(h) = (unsafe { borrow(hints) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = h.masks.len() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cff_hints_mask_at(
    hints: *const CffHints,
    index: usize,
    out_point: *mut usize,
    out_bits: *mut *const u8,
    out_len: *mut usize,
) -> Status {
    let Some(h) = (unsafe { borrow(hints) }) else { return Status::Null };
    let Some((point, bits)) = h.masks.get(index) else { return Status::Range };
    unsafe {
        if !out_point.is_null() {
            *out_point = *point;
        }
        if !out_bits.is_null() {
            *out_bits = bits.as_ptr();
        }
        if !out_len.is_null() {
            *out_len = bits.len();
        }
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cff_hints_free(hints: *mut CffHints) {
    unsafe { release(hints) }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every STIX glyph's hint masks read through C as Rust gives them, each mask's point and bits, and
    // none past the last.
    #[test]
    fn cff_hint_masks_reach_c_as_rust_gives_them() {
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/stix-two-math/STIX2Math.otf")).unwrap();
        let font = Font::from_bytes(&bytes).unwrap();
        let mut handle = core::ptr::null_mut();
        assert_eq!(unsafe { crate::ffi::daegun_font_open(bytes.as_ptr(), bytes.len(), &mut handle) }, Status::Ok);
        let mut masked = 0;
        for gid in 0..font.num_glyphs() {
            let Some(want) = font.cff_hints(gid) else { continue };
            let mut hints = core::ptr::null_mut();
            assert_eq!(unsafe { daegun_font_cff_hints(handle, gid, &mut hints) }, Status::Ok);
            let mut count = 0;
            assert_eq!(unsafe { daegun_cff_hints_mask_count(hints, &mut count) }, Status::Ok);
            assert_eq!(count, want.masks.len(), "glyph {gid}");
            for (i, (point, bits)) in want.masks.iter().enumerate() {
                let (mut at, mut data, mut len) = (0, core::ptr::null(), 0);
                assert_eq!(unsafe { daegun_cff_hints_mask_at(hints, i, &mut at, &mut data, &mut len) }, Status::Ok);
                let got = unsafe { core::slice::from_raw_parts(data, len) };
                assert_eq!((at, got), (*point, &bits[..]), "glyph {gid} mask {i}");
            }
            let none = core::ptr::null_mut();
            let past = unsafe { daegun_cff_hints_mask_at(hints, count, none, core::ptr::null_mut(), none) };
            assert_eq!(past, Status::Range);
            masked += usize::from(count > 0);
            unsafe { daegun_cff_hints_free(hints) };
        }
        assert!(masked > 1_000, "{masked} glyphs with masks");
        unsafe { crate::ffi::daegun_font_free(handle) };
    }

    // EB Garamond with glyph `a`'s first two end points swapped, so it no longer decodes.
    #[test]
    fn a_glyph_that_does_not_decode_is_a_parse_error_not_an_absence() {
        use crate::daecore::daetype::decoder::{build_ttf, extract_ttf_tables};
        let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/eb-garamond/EBGaramond.ttf")).unwrap();
        let font = Font::from_bytes(&bytes).unwrap();
        let a = font.glyph_id('a' as u32).unwrap();
        let mut map: alloc::collections::BTreeMap<alloc::string::String, Vec<u8>> =
            extract_ttf_tables(&bytes).unwrap().into_iter().map(|(t, d)| (t, d.to_vec())).collect();
        let loca = crate::daecore::daetype::instancer::parse_loca(&extract_ttf_tables(&bytes).unwrap(), 1, usize::from(font.num_glyphs())).unwrap();
        let at = loca[usize::from(a)];
        let glyf = map.get_mut("glyf").unwrap();
        assert!(glyf[at + 1] >= 2, "a has two contours to swap");
        glyf.swap(at + 10, at + 12);
        glyf.swap(at + 11, at + 13);
        let broken = build_ttf(&map);
        let mut handle = core::ptr::null_mut();
        assert_eq!(unsafe { crate::ffi::daegun_font_open(broken.as_ptr(), broken.len(), &mut handle) }, Status::Ok);
        let pen = Pen { move_to: None, line_to: None, quad_to: None, curve_to: None, close: None, user: core::ptr::null_mut() };
        assert_eq!(unsafe { daegun_font_outline_glyph(handle, a, &pen) }, Status::Parse);
        let reason = crate::ffi::daegun_last_error();
        let reason = unsafe { core::slice::from_raw_parts(reason.data.cast::<u8>(), reason.len) };
        assert!(core::str::from_utf8(reason).unwrap().contains("end points"));
        assert_eq!(unsafe { daegun_font_outline_glyph(handle, u16::MAX, &pen) }, Status::Absent);
        unsafe { crate::ffi::daegun_font_free(handle) };
    }
}
