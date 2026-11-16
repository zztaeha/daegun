// mathtable.h: MATH's constants, glyph metrics and stretching glyphs. Its structs are Rust's, laid out as C reads them.

use crate::ffi::handle::{
    Status, borrow, cleared, deliver_reused, field_view, invalid, or_return, put, put_some, reused_box,
};
use crate::math::{GlyphPart, MathConstants, MathGlyphConstruction, MathGlyphVariant, MathKernCorner};
use crate::ffi::font::LocationHandle;
use crate::{Font, GlyphId};

const _: () = assert!(size_of::<MathConstants>() == 56 * size_of::<f64>());

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_constants(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut MathConstants,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { put_some(out, font.math_constants(crate::ffi::at(location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_italics_correction(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { put_some(out, font.math_italics_correction(GlyphId(gid), crate::ffi::at(location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_top_accent_attachment(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { put(out, font.math_top_accent_attachment(GlyphId(gid), crate::ffi::at(location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_is_extended_shape(font: *const Font, gid: u32, out: *mut bool) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put(out, font.math_is_extended_shape(GlyphId(gid))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_min_connector_overlap(font: *const Font, out: *mut f64) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put_some(out, font.math_min_connector_overlap()) }
}

pub const MATH_KERN_TOP_RIGHT: i32 = 0;
pub const MATH_KERN_TOP_LEFT: i32 = 1;
pub const MATH_KERN_BOTTOM_RIGHT: i32 = 2;
pub const MATH_KERN_BOTTOM_LEFT: i32 = 3;

fn corner_of(code: i32) -> Result<MathKernCorner, Status> {
    match code {
        MATH_KERN_TOP_RIGHT => Ok(MathKernCorner::TopRight),
        MATH_KERN_TOP_LEFT => Ok(MathKernCorner::TopLeft),
        MATH_KERN_BOTTOM_RIGHT => Ok(MathKernCorner::BottomRight),
        MATH_KERN_BOTTOM_LEFT => Ok(MathKernCorner::BottomLeft),
        _ => Err(invalid(format_args!("corner {code} is none of DAEGUN_MATH_KERN_*"))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_kern(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    corner: i32,
    height: f64,
    out: *mut f64,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let corner = or_return!(corner_of(corner));
    match font.math_kern(GlyphId(gid), unsafe { crate::ffi::at(location) }, corner, height) {
        Ok(kern) => unsafe { put(out, kern) },
        Err(e) => crate::ffi::handle::failed(e),
    }
}

// stretching glyphs

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GlyphAssemblyC {
    italics_correction: f64,
    parts: *const GlyphPart,
    parts_len: usize,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    size_of::<MathGlyphVariant>() == 16 && size_of::<GlyphPart>() == 40 && size_of::<GlyphAssemblyC>() == 24
);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_math_glyph_variants(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    orientation: i32,
    out: *mut *mut MathGlyphConstruction,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let orientation = or_return!(crate::ffi::orientation(orientation));
    match font.math_glyph_variants(GlyphId(gid), unsafe { crate::ffi::at(location) }, orientation) {
        Some(construction) => unsafe { deliver_reused(out, construction) },
        None => Status::Absent,
    }
}

field_view!(daegun_math_construction_variants, MathGlyphConstruction, variants, MathGlyphVariant);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_math_construction_assembly(
    construction: *const MathGlyphConstruction,
    out: *mut GlyphAssemblyC,
) -> Status {
    let Some(construction) = (unsafe { borrow(construction) }) else { return Status::Null };
    let assembly = construction.assembly.as_ref().map(|a| GlyphAssemblyC {
        italics_correction: a.italics_correction,
        parts: a.parts.as_ptr(),
        parts_len: a.parts.len(),
    });
    unsafe { put_some(out, assembly) }
}

reused_box!(daegun_math_construction_free, MathGlyphConstruction, MathGlyphConstruction {
    assembly: None,
    variants: alloc::vec::Vec::new(),
});

pub(crate) fn release_thread() {
    crate::ffi::handle::drop_spare::<MathGlyphConstruction>();
}

#[cfg(test)]
#[path = "tests/math.rs"]
mod tests;
