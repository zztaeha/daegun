// raster.h: daegun's rasterizer, subpixel layouts and an atlas packer.

use crate::color::{Rgba, SceneOptions};
use crate::ffi::color::{FILL_EVENODD, FILL_NONZERO, SceneOptionsC};
use crate::ffi::font::LocationHandle;
use crate::ffi::handle::{
    Options, Status, array_of, borrow, borrow_mut, cleared, defaults_into, deliver_reused, failed, invalid,
    options_of, or_return, put, put_some, release, reused, reused_box, slice_mut_of, slice_of, view,
};
use crate::ffi::outline::{OutlineOptionsC, outline_options};
use crate::outline::{FillRule, Path};
use crate::raster::{
    Field, Gamma, Image, RasterOptions, Rasterizer, Rect, ShelfPacker, StripeOrder, SubpixelLayout, Target,
};
use crate::{Font, GlyphId};

const _: () = assert!(size_of::<Rect>() == 4 * size_of::<usize>());

// subpixel layouts

pub const STRIPES_RGB: i32 = 0;
pub const STRIPES_BGR: i32 = 1;

fn order(code: i32) -> Result<StripeOrder, Status> {
    match code {
        STRIPES_RGB => Ok(StripeOrder::Rgb),
        STRIPES_BGR => Ok(StripeOrder::Bgr),
        _ => Err(invalid(format_args!("stripe order {code} is none of DAEGUN_STRIPES_*"))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_grayscale(out: *mut *mut SubpixelLayout) -> Status {
    unsafe { deliver_reused(out, SubpixelLayout::grayscale()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_horizontal(order_code: i32, out: *mut *mut SubpixelLayout) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let order = or_return!(order(order_code));
    unsafe { deliver_reused(out, SubpixelLayout::horizontal(order)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_vertical(order_code: i32, out: *mut *mut SubpixelLayout) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let order = or_return!(order(order_code));
    unsafe { deliver_reused(out, SubpixelLayout::vertical(order)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_unfiltered(
    order_code: i32,
    orientation: i32,
    out: *mut *mut SubpixelLayout,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let order = or_return!(order(order_code));
    let orientation = or_return!(crate::ffi::orientation(orientation));
    unsafe { deliver_reused(out, SubpixelLayout::unfiltered(order, orientation)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_from_weights(
    oversample_x: u8,
    oversample_y: u8,
    taps_x: u8,
    taps_y: u8,
    origin_x: i8,
    origin_y: i8,
    weights: *const f32,
    weights_len: usize,
    out: *mut *mut SubpixelLayout,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(weights) = (unsafe { slice_of(weights, weights_len) }) else { return Status::Null };
    if !weights_len.is_multiple_of(3) {
        return invalid(format_args!("{weights_len} weights are not three channels' worth"));
    }
    let channel = weights_len / 3;
    let (first, rest) = weights.split_at(channel);
    let (second, third) = rest.split_at(channel);
    let layout = SubpixelLayout::from_weights(
        (oversample_x, oversample_y),
        (taps_x, taps_y),
        (origin_x, origin_y),
        [first, second, third],
    );
    match layout {
        Ok(layout) => unsafe { deliver_reused(out, layout) },
        Err(e) => failed(e),
    }
}

// A layout owns no memory, so the spare keeps whichever it held rather than writing an empty one over it.
impl crate::ffi::handle::Reused for SubpixelLayout {
    fn empty() -> SubpixelLayout {
        SubpixelLayout::default()
    }

    fn emptied(&mut self) -> usize {
        0
    }

    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<alloc::boxed::Box<SubpixelLayout>>>> {
        std::thread_local! {
            static SPARE: core::cell::Cell<Option<alloc::boxed::Box<SubpixelLayout>>> =
                const { core::cell::Cell::new(None) };
        }
        &SPARE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_free(layout: *mut SubpixelLayout) {
    unsafe { crate::ffi::handle::give_back(layout) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_is_grayscale(layout: *const SubpixelLayout, out: *mut bool) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    unsafe { put(out, layout.is_grayscale()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_channels(layout: *const SubpixelLayout, out: *mut u8) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    unsafe { put(out, layout.channels()) }
}

// A pair answered through two outs, both required.
unsafe fn put_pair<T>(out_x: *mut T, out_y: *mut T, (x, y): (T, T)) -> Status {
    if out_x.is_null() || out_y.is_null() {
        return Status::Null;
    }
    unsafe {
        out_x.write(x);
        out_y.write(y);
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_oversample(
    layout: *const SubpixelLayout,
    out_x: *mut u8,
    out_y: *mut u8,
) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    unsafe { put_pair(out_x, out_y, layout.oversample()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_taps(
    layout: *const SubpixelLayout,
    out_x: *mut u8,
    out_y: *mut u8,
) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    unsafe { put_pair(out_x, out_y, layout.taps()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_origin(
    layout: *const SubpixelLayout,
    out_x: *mut i8,
    out_y: *mut i8,
) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    unsafe { put_pair(out_x, out_y, layout.origin()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_pad(
    layout: *const SubpixelLayout,
    out_x: *mut usize,
    out_y: *mut usize,
) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    unsafe { put_pair(out_x, out_y, layout.pad()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_weights(
    layout: *const SubpixelLayout,
    channel: usize,
    out_count: *mut usize,
) -> *const f32 {
    unsafe { view(layout, out_count, |l| l.weights(channel)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_subpixel_layout_key(layout: *const SubpixelLayout, out: *mut u64) -> Status {
    let Some(layout) = (unsafe { borrow(layout) }) else { return Status::Null };
    unsafe { put(out, layout.key()) }
}

// atlases

reused_box!(daegun_shelf_packer_free, ShelfPacker, ShelfPacker::new(0, 0));

#[unsafe(no_mangle)]
pub extern "C" fn daegun_shelf_packer_new(width: usize, height: usize) -> *mut ShelfPacker {
    let mut packer = reused::<ShelfPacker>();
    *packer = ShelfPacker::new(width, height);
    alloc::boxed::Box::into_raw(packer)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shelf_packer_insert(
    packer: *mut ShelfPacker,
    w: usize,
    h: usize,
    out: *mut Rect,
) -> Status {
    let Some(packer) = (unsafe { borrow_mut(packer) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { put_some(out, packer.insert(w, h)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_shelf_packer_reset(packer: *mut ShelfPacker) -> Status {
    let Some(packer) = (unsafe { borrow_mut(packer) }) else { return Status::Null };
    packer.reset();
    Status::Ok
}

// rasterizing

pub const GAMMA_LINEAR: i32 = 0;
pub const GAMMA_SRGB: i32 = 1;
pub const FIELD_SDF: i32 = 0;
pub const FIELD_MSDF: i32 = 1;

#[repr(C)]
pub struct ImageC {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    stride: usize,
    pixels: *const u8,
    pixels_len: usize,
    channels: u8,
}

#[repr(C)]
pub struct TargetC {
    left: i32,
    top: i32,
    width: u32,
    height: u32,
    stride: usize,
    pixels: *mut u8,
    pixels_len: usize,
}

// The caller's pixels as a target; None for a NULL target, or NULL pixels with a length.
unsafe fn target<'a>(target: *const TargetC) -> Option<Target<'a>> {
    let t = unsafe { target.as_ref() }?;
    let pixels = unsafe { slice_mut_of(t.pixels, t.pixels_len) }?;
    Some(Target::new(t.left, t.top, t.width, t.height, t.stride, pixels))
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RasterOptionsC {
    size: usize,
    outline: *const OutlineOptionsC,
    layout: *const SubpixelLayout,
    offset: [f32; 2],
    gamma: i32,
    text: Rgba,
    contrast: f32,
    darken: u8,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    size_of::<ImageC>() == 48
        && size_of::<RasterOptionsC>() == 48
        && core::mem::offset_of!(RasterOptionsC, offset) == 24
        && core::mem::offset_of!(RasterOptionsC, darken) == 44
);

// Rule 5: a rasterizer may be handed to another thread and freed there.
const _: () = {
    const fn shareable<T: Send + Sync>() {}
    shareable::<Rasterizer>();
};

// Every default is zero.
impl Options for RasterOptionsC {
    unsafe fn set_defaults(_: *mut Self) {}
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_raster_options_default(out: *mut RasterOptionsC, size: usize) -> Status {
    unsafe { defaults_into(out, size) }
}

// Rust's options, the outline's and the layout read where the caller keeps them.
unsafe fn raster_options<'a>(opts: &RasterOptionsC) -> Result<RasterOptions<'a>, Status> {
    let outline = outline_options(&unsafe { options_of(opts.outline) }?)?;
    let gamma = match opts.gamma {
        GAMMA_LINEAR => Gamma::Linear,
        GAMMA_SRGB => Gamma::Srgb { text: opts.text, contrast: opts.contrast },
        code => return Err(invalid(format_args!("gamma {code} is none of DAEGUN_GAMMA_*"))),
    };
    let layout = unsafe { opts.layout.as_ref() }.unwrap_or(RasterOptions::DEFAULT.layout);
    Ok(RasterOptions::DEFAULT
        .with_outline(outline)
        .with_offset(opts.offset[0], opts.offset[1])
        .with_layout(layout)
        .with_darken(opts.darken != 0)
        .with_gamma(gamma))
}

// The image a call lent into `out`, or its failure's status, `out` left as it was.
unsafe fn lend(out: *mut ImageC, image: crate::Result<Image<'_>>) -> Status {
    match image {
        Ok(image) => {
            let pixels = image.pixels;
            let (left, top, width, height, stride, channels) =
                (image.left, image.top, image.width, image.height, image.stride, image.channels);
            let (pixels, pixels_len) = (pixels.as_ptr(), pixels.len());
            unsafe { out.write(ImageC { left, top, width, height, stride, pixels, pixels_len, channels }) };
            Status::Ok
        }
        Err(e) => failed(e),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_rasterizer_new() -> *mut Rasterizer {
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Rasterizer::new()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_rasterizer_free(rasterizer: *mut Rasterizer) {
    unsafe { release(rasterizer) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_rasterizer_set_cache_budget(rasterizer: *mut Rasterizer, bytes: usize) -> Status {
    let Some(r) = (unsafe { borrow_mut(rasterizer) }) else { return Status::Null };
    r.set_cache_budget(bytes);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_rasterizer_glyph(
    rasterizer: *mut Rasterizer,
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    px: f32,
    opts: *const RasterOptionsC,
    out: *mut ImageC,
) -> Status {
    let (Some(r), Some(font)) = (unsafe { borrow_mut(rasterizer) }, unsafe { borrow(font) }) else {
        return Status::Null;
    };
    if out.is_null() {
        return Status::Null;
    }
    let opts = or_return!(unsafe { options_of(opts) });
    let options = or_return!(unsafe { raster_options(&opts) });
    let at = unsafe { crate::ffi::at(location) };
    unsafe { lend(out, r.glyph(font, GlyphId(gid), at, px, &options)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_rasterizer_glyph_into(
    rasterizer: *mut Rasterizer,
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    px: f32,
    opts: *const RasterOptionsC,
    target: *const TargetC,
) -> Status {
    let (Some(r), Some(font)) = (unsafe { borrow_mut(rasterizer) }, unsafe { borrow(font) }) else {
        return Status::Null;
    };
    let Some(target) = (unsafe { self::target(target) }) else { return Status::Null };
    let opts = or_return!(unsafe { options_of(opts) });
    let options = or_return!(unsafe { raster_options(&opts) });
    let at = unsafe { crate::ffi::at(location) };
    match r.glyph_into(font, GlyphId(gid), at, px, &options, target) {
        Ok(()) => Status::Ok,
        Err(e) => failed(e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_rasterizer_color_glyph(
    rasterizer: *mut Rasterizer,
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    px: f32,
    opts: *const RasterOptionsC,
    scene: *const SceneOptionsC,
    out: *mut ImageC,
) -> Status {
    let (Some(r), Some(font)) = (unsafe { borrow_mut(rasterizer) }, unsafe { borrow(font) }) else {
        return Status::Null;
    };
    if out.is_null() {
        return Status::Null;
    }
    let opts = or_return!(unsafe { options_of(opts) });
    let options = or_return!(unsafe { raster_options(&opts) });
    let scene = or_return!(unsafe { options_of(scene) });
    let scene = SceneOptions { palette: scene.palette, foreground: scene.foreground };
    let at = unsafe { crate::ffi::at(location) };
    match r.color_glyph(font, GlyphId(gid), at, px, &options, &scene) {
        Ok(Some(image)) => unsafe { lend(out, Ok(image)) },
        Ok(None) => Status::Absent,
        Err(e) => failed(e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_rasterizer_color_glyph_into(
    rasterizer: *mut Rasterizer,
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    px: f32,
    opts: *const RasterOptionsC,
    scene: *const SceneOptionsC,
    target: *const TargetC,
) -> Status {
    let (Some(r), Some(font)) = (unsafe { borrow_mut(rasterizer) }, unsafe { borrow(font) }) else {
        return Status::Null;
    };
    let Some(target) = (unsafe { self::target(target) }) else { return Status::Null };
    let opts = or_return!(unsafe { options_of(opts) });
    let options = or_return!(unsafe { raster_options(&opts) });
    let scene = or_return!(unsafe { options_of(scene) });
    let scene = SceneOptions { palette: scene.palette, foreground: scene.foreground };
    let at = unsafe { crate::ffi::at(location) };
    match r.color_glyph_into(font, GlyphId(gid), at, px, &options, &scene, target) {
        Ok(true) => Status::Ok,
        Ok(false) => Status::Absent,
        Err(e) => failed(e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_rasterizer_path(
    rasterizer: *mut Rasterizer,
    path: *const Path,
    rule: i32,
    transform: *const f64,
    out: *mut ImageC,
) -> Status {
    let (Some(r), Some(path)) = (unsafe { borrow_mut(rasterizer) }, unsafe { borrow(path) }) else {
        return Status::Null;
    };
    if out.is_null() {
        return Status::Null;
    }
    let rule = match rule {
        FILL_NONZERO => FillRule::NonZero,
        FILL_EVENODD => FillRule::EvenOdd,
        code => return invalid(format_args!("fill rule {code} is none of DAEGUN_FILL_*")),
    };
    let transform = unsafe { array_of::<f64, 6>(transform) };
    unsafe { lend(out, r.path(path, rule, transform.as_ref())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_rasterizer_path_into(
    rasterizer: *mut Rasterizer,
    path: *const Path,
    rule: i32,
    transform: *const f64,
    target: *const TargetC,
) -> Status {
    let (Some(r), Some(path)) = (unsafe { borrow_mut(rasterizer) }, unsafe { borrow(path) }) else {
        return Status::Null;
    };
    let Some(target) = (unsafe { self::target(target) }) else { return Status::Null };
    let rule = match rule {
        FILL_NONZERO => FillRule::NonZero,
        FILL_EVENODD => FillRule::EvenOdd,
        code => return invalid(format_args!("fill rule {code} is none of DAEGUN_FILL_*")),
    };
    let transform = unsafe { array_of::<f64, 6>(transform) };
    match r.path_into(path, rule, transform.as_ref(), target) {
        Ok(()) => Status::Ok,
        Err(e) => failed(e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_rasterizer_distance_field(
    rasterizer: *mut Rasterizer,
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    px: f32,
    field: i32,
    spread: f32,
    out: *mut ImageC,
) -> Status {
    let (Some(r), Some(font)) = (unsafe { borrow_mut(rasterizer) }, unsafe { borrow(font) }) else {
        return Status::Null;
    };
    if out.is_null() {
        return Status::Null;
    }
    let field = match field {
        FIELD_SDF => Field::Sdf { spread },
        FIELD_MSDF => Field::Msdf { spread },
        code => return invalid(format_args!("field {code} is none of DAEGUN_FIELD_*")),
    };
    let at = unsafe { crate::ffi::at(location) };
    unsafe { lend(out, r.distance_field(font, GlyphId(gid), at, px, field)) }
}

pub(crate) fn release_thread() {
    crate::ffi::handle::drop_spare::<SubpixelLayout>();
    crate::ffi::handle::drop_spare::<ShelfPacker>();
}

#[cfg(test)]
#[path = "tests/raster.rs"]
mod tests;
