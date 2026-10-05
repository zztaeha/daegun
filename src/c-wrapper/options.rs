// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use core::mem::offset_of;

use crate::{Cap, HintMode, Join, StripeOrder, StrokeStyle, SubpixelLayout};

use crate::ffi::handle::Status;

pub const LAYOUT_GRAYSCALE: i32 = 0;
pub const LAYOUT_RGB_H: i32 = 1;
pub const LAYOUT_BGR_H: i32 = 2;
pub const LAYOUT_RGB_V: i32 = 3;
pub const LAYOUT_BGR_V: i32 = 4;
pub const LAYOUT_RGB_H_UNFILTERED: i32 = 5;
pub const LAYOUT_BGR_H_UNFILTERED: i32 = 6;
pub const LAYOUT_RGB_V_UNFILTERED: i32 = 7;
pub const LAYOUT_BGR_V_UNFILTERED: i32 = 8;

pub(crate) fn layout_of(code: i32) -> SubpixelLayout {
    match code {
        LAYOUT_RGB_H => SubpixelLayout::horizontal(StripeOrder::Rgb),
        LAYOUT_BGR_H => SubpixelLayout::horizontal(StripeOrder::Bgr),
        LAYOUT_RGB_V => SubpixelLayout::vertical(StripeOrder::Rgb),
        LAYOUT_BGR_V => SubpixelLayout::vertical(StripeOrder::Bgr),
        LAYOUT_RGB_H_UNFILTERED => SubpixelLayout::unfiltered(StripeOrder::Rgb, true),
        LAYOUT_BGR_H_UNFILTERED => SubpixelLayout::unfiltered(StripeOrder::Bgr, true),
        LAYOUT_RGB_V_UNFILTERED => SubpixelLayout::unfiltered(StripeOrder::Rgb, false),
        LAYOUT_BGR_V_UNFILTERED => SubpixelLayout::unfiltered(StripeOrder::Bgr, false),
        LAYOUT_GRAYSCALE => SubpixelLayout::grayscale(),
        _ => SubpixelLayout::grayscale(),
    }
}

pub const HINT_NONE: i32 = 0;
pub const HINT_SUBPIXEL: i32 = 1;
pub const HINT_CLASSIC: i32 = 2;
pub const HINT_AUTO: i32 = 3;
pub const HINT_AUTO_FORCE: i32 = 4;

pub(crate) fn hint_of(code: i32) -> HintMode {
    match code {
        HINT_SUBPIXEL => HintMode::Subpixel,
        HINT_CLASSIC => HintMode::Classic,
        HINT_AUTO => HintMode::Auto,
        HINT_AUTO_FORCE => HintMode::AutoForce,
        _ => HintMode::None,
    }
}

pub const JOIN_MITER: i32 = 0;
pub const JOIN_ROUND: i32 = 1;
pub const JOIN_BEVEL: i32 = 2;

pub const CAP_BUTT: i32 = 0;
pub const CAP_ROUND: i32 = 1;
pub const CAP_SQUARE: i32 = 2;

pub(crate) fn join_of(code: i32, miter_limit: f32) -> Join {
    match code {
        JOIN_ROUND => Join::Round,
        JOIN_BEVEL => Join::Bevel,
        _ => Join::Miter { limit: miter_limit },
    }
}

pub(crate) fn cap_of(code: i32) -> Cap {
    match code {
        CAP_ROUND => Cap::Round,
        CAP_SQUARE => Cap::Square,
        _ => Cap::Butt,
    }
}

pub(crate) fn stroke_of(width: f32, join: i32, miter_limit: f32, cap: i32) -> StrokeStyle {
    StrokeStyle { width, join: join_of(join, miter_limit), cap: cap_of(cap) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OutlineOptionsC {
    pub hinting: i32,
    pub has_transform: i32,
    pub transform: [f32; 6],
    pub has_stroke: i32,
    pub stroke_width: f32,
    pub stroke_join: i32,
    pub stroke_miter_limit: f32,
    pub stroke_cap: i32,
    pub has_embolden: i32,
    pub embolden: f32,
    pub has_oblique: i32,
    pub oblique: f32,
}

// Mirrored by `_Static_assert` in `daegun.h`; a reordered field passes a size check alone.
const _: () = assert!(size_of::<OutlineOptionsC>() == 68);
const _: () = assert!(align_of::<OutlineOptionsC>() == 4);
const _: () = {
    assert!(offset_of!(OutlineOptionsC, hinting) == 0);
    assert!(offset_of!(OutlineOptionsC, has_transform) == 4);
    assert!(offset_of!(OutlineOptionsC, transform) == 8);
    assert!(offset_of!(OutlineOptionsC, has_stroke) == 32);
    assert!(offset_of!(OutlineOptionsC, stroke_width) == 36);
    assert!(offset_of!(OutlineOptionsC, stroke_join) == 40);
    assert!(offset_of!(OutlineOptionsC, stroke_miter_limit) == 44);
    assert!(offset_of!(OutlineOptionsC, stroke_cap) == 48);
    assert!(offset_of!(OutlineOptionsC, has_embolden) == 52);
    assert!(offset_of!(OutlineOptionsC, embolden) == 56);
    assert!(offset_of!(OutlineOptionsC, has_oblique) == 60);
    assert!(offset_of!(OutlineOptionsC, oblique) == 64);
};

impl OutlineOptionsC {
    pub const DEFAULT: OutlineOptionsC = OutlineOptionsC {
        hinting: HINT_NONE,
        has_transform: 0,
        transform: [0.0; 6],
        has_stroke: 0,
        stroke_width: 0.0,
        stroke_join: JOIN_MITER,
        stroke_miter_limit: 0.0,
        stroke_cap: CAP_BUTT,
        has_embolden: 0,
        embolden: 0.0,
        has_oblique: 0,
        oblique: 0.0,
    };

    pub fn to_rust(self) -> crate::OutlineOptions {
        crate::OutlineOptions {
            transform: (self.has_transform != 0).then_some(self.transform),
            hinting: hint_of(self.hinting),
            stroke: (self.has_stroke != 0)
                .then(|| stroke_of(self.stroke_width, self.stroke_join, self.stroke_miter_limit, self.stroke_cap)),
            embolden: (self.has_embolden != 0).then_some(self.embolden),
            oblique: (self.has_oblique != 0).then_some(self.oblique),
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_outline_options_default(out: *mut OutlineOptionsC) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = OutlineOptionsC::DEFAULT };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hint_mode_may_autohint(mode: i32, out: *mut i32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = i32::from(hint_of(mode).may_autohint()) };
    Status::Ok
}
