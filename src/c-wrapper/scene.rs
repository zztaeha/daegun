// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use alloc::vec::Vec;
use core::mem::offset_of;

use crate::api::TooManyPoints;
use crate::{FillRule, Font};
use crate::paint::{Blend, ClipShape, DisplayList, Extend, Gradient, GradientKind, Op, Paint, Rgba, Stop, Stops};

use crate::ffi::handle::{Status, borrow, deliver, release};
use crate::ffi::list::{Axis, axes_of};
use crate::ffi::outline::Path;

pub const SCENE_FILL: i32 = 0;
pub const SCENE_PUSH_CLIP: i32 = 1;
pub const SCENE_POP_CLIP: i32 = 2;
pub const SCENE_PUSH_LAYER: i32 = 3;
pub const SCENE_POP_LAYER: i32 = 4;

pub const SCENE_PAINT_SOLID: i32 = 0;
pub const SCENE_PAINT_GRADIENT: i32 = 1;

pub const FILL_NONZERO: i32 = 0;
pub const FILL_EVENODD: i32 = 1;

pub const GRADIENT_LINEAR: i32 = 0;
pub const GRADIENT_RADIAL: i32 = 1;
pub const GRADIENT_SWEEP: i32 = 2;

// COLR's numbering, which is not the order of `Extend`'s variants.
pub const EXTEND_PAD: i32 = 0;
pub const EXTEND_REPEAT: i32 = 1;
pub const EXTEND_REFLECT: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SceneOp {
    pub transform: [f64; 6],
    pub kind: i32,
    pub rule: i32,
    pub paint: i32,
    pub blend: i32,
    pub path: u32,
    pub gradient: u32,
    pub clip_start: u32,
    pub clip_count: u32,
    pub opacity: f32,
    pub rgba: [u8; 4],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SceneClip {
    pub transform: [f64; 6],
    pub path: u32,
    pub rule: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SceneGradient {
    pub transform: [f64; 6],
    pub numbers: [f64; 6],
    pub kind: i32,
    pub extend: i32,
    pub stops_start: u32,
    pub stops_count: u32,
}

// Mirrored by `_Static_assert` in `daegun.h`; a reordered field passes a size check alone.
const _: () = assert!(size_of::<SceneOp>() == 88);
const _: () = assert!(size_of::<SceneClip>() == 56);
const _: () = assert!(size_of::<SceneGradient>() == 112);
const _: () = {
    assert!(offset_of!(SceneOp, transform) == 0);
    assert!(offset_of!(SceneOp, kind) == 48);
    assert!(offset_of!(SceneOp, rule) == 52);
    assert!(offset_of!(SceneOp, paint) == 56);
    assert!(offset_of!(SceneOp, blend) == 60);
    assert!(offset_of!(SceneOp, path) == 64);
    assert!(offset_of!(SceneOp, gradient) == 68);
    assert!(offset_of!(SceneOp, clip_start) == 72);
    assert!(offset_of!(SceneOp, clip_count) == 76);
    assert!(offset_of!(SceneOp, opacity) == 80);
    assert!(offset_of!(SceneOp, rgba) == 84);
};
const _: () = {
    assert!(offset_of!(SceneClip, transform) == 0);
    assert!(offset_of!(SceneClip, path) == 48);
    assert!(offset_of!(SceneClip, rule) == 52);
};
const _: () = {
    assert!(offset_of!(SceneGradient, transform) == 0);
    assert!(offset_of!(SceneGradient, numbers) == 48);
    assert!(offset_of!(SceneGradient, kind) == 96);
    assert!(offset_of!(SceneGradient, extend) == 100);
    assert!(offset_of!(SceneGradient, stops_start) == 104);
    assert!(offset_of!(SceneGradient, stops_count) == 108);
};

pub struct ColorScene {
    list: DisplayList,
    gradients: Vec<Gradient>,
    ops: Vec<SceneOp>,
    clips: Vec<SceneClip>,
    flat: Vec<SceneGradient>,
    stop_offsets: Vec<f64>,
    stop_colors: Vec<u8>,
}

fn rule_code(rule: FillRule) -> i32 {
    match rule {
        FillRule::NonZero => FILL_NONZERO,
        FillRule::EvenOdd => FILL_EVENODD,
    }
}

fn index(n: usize) -> Option<u32> {
    u32::try_from(n).ok()
}

impl ColorScene {
    fn new(list: DisplayList) -> Option<ColorScene> {
        let mut s = ColorScene {
            list: DisplayList::default(),
            gradients: Vec::new(),
            ops: Vec::with_capacity(list.ops().len()),
            clips: Vec::new(),
            flat: Vec::new(),
            stop_offsets: Vec::new(),
            stop_colors: Vec::new(),
        };
        for op in list.ops() {
            let mut out = SceneOp {
                transform: [0.0; 6],
                kind: 0,
                rule: 0,
                paint: 0,
                blend: 0,
                path: 0,
                gradient: 0,
                clip_start: 0,
                clip_count: 0,
                opacity: 0.0,
                rgba: [0; 4],
            };
            match op {
                Op::Fill { path, paint, rule, transform } => {
                    out.kind = SCENE_FILL;
                    out.path = index(*path)?;
                    out.rule = rule_code(*rule);
                    out.transform = *transform;
                    match paint {
                        &Paint::Solid(Rgba { r, g, b, a }) => {
                            out.paint = SCENE_PAINT_SOLID;
                            out.rgba = [r, g, b, a];
                        }
                        Paint::Gradient(g) => {
                            out.paint = SCENE_PAINT_GRADIENT;
                            out.gradient = index(s.flat.len())?;
                            s.push_gradient(g)?;
                        }
                    }
                }
                Op::PushClip { shapes } => {
                    out.kind = SCENE_PUSH_CLIP;
                    out.clip_start = index(s.clips.len())?;
                    out.clip_count = index(shapes.len())?;
                    for &ClipShape { path, rule, transform } in shapes {
                        s.clips.push(SceneClip { transform, path: index(path)?, rule: rule_code(rule) });
                    }
                }
                Op::PopClip => out.kind = SCENE_POP_CLIP,
                Op::PushLayer { opacity, blend } => {
                    out.kind = SCENE_PUSH_LAYER;
                    out.opacity = *opacity;
                    // `Blend` is declared in COLR's composite-mode order, so this is the spec's number.
                    out.blend = *blend as i32;
                }
                Op::PopLayer => out.kind = SCENE_POP_LAYER,
            }
            s.ops.push(out);
        }
        s.list = list;
        Some(s)
    }

    fn push_gradient(&mut self, g: &Gradient) -> Option<()> {
        let Gradient { kind, stops, extend, transform } = g;
        let (kind, numbers) = match *kind {
            GradientKind::Linear { x0, y0, x1, y1 } => (GRADIENT_LINEAR, [x0, y0, x1, y1, 0.0, 0.0]),
            GradientKind::Radial { x0, y0, r0, x1, y1, r1 } => {
                (GRADIENT_RADIAL, [x0, y0, r0, x1, y1, r1])
            }
            GradientKind::Sweep { cx, cy, start_angle, end_angle } => {
                (GRADIENT_SWEEP, [cx, cy, start_angle, end_angle, 0.0, 0.0])
            }
        };
        let stops = match crate::paint::resolve_stops(stops.clone()) {
            Stops::Nothing => Vec::new(),
            Stops::Solid(c) => alloc::vec![crate::paint::Stop { offset: 0.0, color: c }],
            Stops::Many(s) => s,
        };
        self.flat.push(SceneGradient {
            transform: *transform,
            numbers: numbers.map(f64::from),
            kind,
            extend: match extend {
                Extend::Pad => EXTEND_PAD,
                Extend::Repeat => EXTEND_REPEAT,
                Extend::Reflect => EXTEND_REFLECT,
            },
            stops_start: index(self.stop_offsets.len())?,
            stops_count: index(stops.len())?,
        });
        for Stop { offset, color: Rgba { r, g, b, a } } in stops {
            self.stop_offsets.push(f64::from(offset));
            self.stop_colors.extend_from_slice(&[r, g, b, a]);
        }
        self.gradients.push(g.clone());
        Some(())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_colr_scene(
    font: *const Font,
    gid: u16,
    axes: *const Axis,
    axes_len: usize,
    palette_index: u16,
    out: *mut *mut ColorScene,
) -> Status {
    unsafe {
        daegun_font_colr_scene_with(font, gid, axes, axes_len, palette_index, core::ptr::null(), out)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_colr_scene_with(
    font: *const Font,
    gid: u16,
    axes: *const Axis,
    axes_len: usize,
    palette_index: u16,
    foreground: *const u8,
    out: *mut *mut ColorScene,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(location) = (unsafe { axes_of(axes, axes_len) }) else { return Status::Null };
    let fg = unsafe { crate::ffi::rgba_of(foreground) }.unwrap_or(crate::api::FOREGROUND);
    let scene = match font.scene(gid, &location, palette_index, fg) {
        Ok(Some(scene)) => scene,
        Ok(None) => return Status::Absent,
        Err(TooManyPoints) => {
            crate::ffi::set_error("the scene's outlines hold more than DAEGUN_MAX_FLATTEN_POINTS points");
            return Status::Range;
        }
    };
    let Some(flat) = ColorScene::new(scene) else { return crate::ffi::range("the scene does not fit the C form") };
    unsafe { deliver(out, flat) }
}

macro_rules! scene_view {
    ($fn_name:ident, $field:ident, $elem:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $fn_name(
            scene: *const ColorScene,
            out_count: *mut usize,
        ) -> *const $elem {
            let Some(s) = (unsafe { borrow(scene) }) else { return core::ptr::null() };
            if out_count.is_null() {
                return core::ptr::null();
            }
            unsafe { *out_count = s.$field.len() };
            s.$field.as_ptr()
        }
    };
}

scene_view!(daegun_color_scene_ops, ops, SceneOp);
scene_view!(daegun_color_scene_clips, clips, SceneClip);
scene_view!(daegun_color_scene_gradients, flat, SceneGradient);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_color_scene_stops(
    scene: *const ColorScene,
    out_count: *mut usize,
    out_offsets: *mut *const f64,
    out_colors: *mut *const u8,
) -> Status {
    let Some(s) = (unsafe { borrow(scene) }) else { return Status::Null };
    if out_count.is_null() || out_offsets.is_null() || out_colors.is_null() {
        return Status::Null;
    }
    unsafe {
        *out_count = s.stop_offsets.len();
        *out_offsets = s.stop_offsets.as_ptr();
        *out_colors = s.stop_colors.as_ptr();
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_color_scene_path(
    scene: *const ColorScene,
    path_id: u32,
    out: *mut *mut Path,
) -> Status {
    let Some(s) = (unsafe { borrow(scene) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(p) = s.list.path(path_id as usize) else { return crate::ffi::range("no path has that id") };
    unsafe { deliver(out, Path(p.clone())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_color_scene_free(scene: *mut ColorScene) {
    unsafe { release(scene) }
}

pub struct Ramp(crate::paint::gradient::Ramp);

pub const INTERPOLATE_LINEAR_LIGHT: i32 = 0;
pub const INTERPOLATE_SRGB: i32 = 1;

pub(crate) fn interpolation_of(code: i32) -> Option<crate::paint::gradient::Interpolation> {
    match code {
        INTERPOLATE_LINEAR_LIGHT => Some(crate::paint::gradient::Interpolation::LinearLight),
        INTERPOLATE_SRGB => Some(crate::paint::gradient::Interpolation::Srgb),
        _ => None,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_color_scene_ramp(
    scene: *const ColorScene,
    gradient: u32,
    to_device: *const f64,
    out: *mut *mut Ramp,
) -> Status {
    unsafe { daegun_color_scene_ramp_with(scene, gradient, to_device, INTERPOLATE_LINEAR_LIGHT, out) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_color_scene_ramp_with(
    scene: *const ColorScene,
    gradient: u32,
    to_device: *const f64,
    interpolation: i32,
    out: *mut *mut Ramp,
) -> Status {
    let Some(s) = (unsafe { borrow(scene) }) else { return Status::Null };
    if to_device.is_null() || out.is_null() {
        return Status::Null;
    }
    let Some(g) = s.gradients.get(gradient as usize) else { return crate::ffi::range("no gradient has that index") };
    let t = unsafe { core::slice::from_raw_parts(to_device, 6) };
    let to_device = [t[0], t[1], t[2], t[3], t[4], t[5]];
    if !to_device.iter().all(|v| v.is_finite()) {
        return crate::ffi::range("the transform is not finite");
    }
    let Some(how) = interpolation_of(interpolation) else {
        return crate::ffi::range("the interpolation is neither DAEGUN_INTERPOLATE_LINEAR_LIGHT nor _SRGB");
    };
    let ramp = crate::paint::gradient::Ramp::with_interpolation(g, &to_device, how);
    unsafe { deliver(out, Ramp(ramp)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ramp_sample(
    ramp: *const Ramp,
    x: f64,
    y: f64,
    out_rgba: *mut u8,
) -> Status {
    let Some(Ramp(ramp)) = (unsafe { borrow(ramp) }) else { return Status::Null };
    if out_rgba.is_null() {
        return Status::Null;
    }
    let Some(Rgba { r, g, b, a }) = ramp.at(x, y) else { return Status::Absent };
    unsafe { core::slice::from_raw_parts_mut(out_rgba, 4) }.copy_from_slice(&[r, g, b, a]);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ramp_free(ramp: *mut Ramp) {
    unsafe { release(ramp) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_composite(
    mode: i32,
    src: *const f32,
    backdrop: *const f32,
    out: *mut f32,
) -> Status {
    if src.is_null() || backdrop.is_null() || out.is_null() {
        return Status::Null;
    }
    let Some(code) = u8::try_from(mode).ok().filter(|&c| c <= Blend::HslLuminosity as u8) else {
        return crate::ffi::range("the mode is outside 0 to 27");
    };
    let s = unsafe { core::slice::from_raw_parts(src, 4) };
    let b = unsafe { core::slice::from_raw_parts(backdrop, 4) };
    if !s.iter().chain(b).all(|v| (0.0..=1.0).contains(v)) {
        return crate::ffi::range("a color or alpha is outside 0..1");
    }
    let mode = Blend::from_colr(code);
    let (rgb, a) = crate::paint::composite(mode, [s[0], s[1], s[2]], s[3], [b[0], b[1], b[2]], b[3]);
    unsafe { core::slice::from_raw_parts_mut(out, 4) }.copy_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
    Status::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(kind: GradientKind, extend: Extend, transform: [f64; 6]) -> Gradient {
        let stop = |offset, r, b| Stop { offset, color: Rgba { r, g: 0, b, a: 255 } };
        Gradient { kind, stops: alloc::vec![stop(1.5, 0, 255), stop(0.0, 255, 0)], extend, transform }
    }

    // Every field against the number the OpenType spec gives it, not against daegun's own enums.
    #[test]
    fn flattening_keeps_every_field_in_colr_numbering() {
        let id = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let moved_and_doubled = [2.0, 0.0, 0.0, 2.0, 3.0, 4.0];
        let counting = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mut list = DisplayList::default();
        let a = list.push_path(crate::Path::default());
        let b = list.push_path(crate::Path::default());
        let linear = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 100.0, y1: 0.0 };
        let radial = GradientKind::Radial { x0: 1.0, y0: 2.0, r0: 3.0, x1: 4.0, y1: 5.0, r1: 6.0 };
        let sweep = GradientKind::Sweep { cx: 7.0, cy: 8.0, start_angle: 9.0, end_angle: 10.0 };
        list.push(Op::PushClip {
            shapes: alloc::vec![
                ClipShape { path: a, rule: FillRule::NonZero, transform: id },
                ClipShape { path: b, rule: FillRule::EvenOdd, transform: moved_and_doubled },
            ],
        });
        list.push(Op::PushLayer { opacity: 0.5, blend: Blend::Multiply });
        let fill = |paint, rule| Op::Fill { path: b, paint, rule, transform: counting };
        list.push(fill(Paint::Gradient(gradient(linear, Extend::Repeat, id)), FillRule::EvenOdd));
        list.push(fill(Paint::Gradient(gradient(radial, Extend::Reflect, id)), FillRule::NonZero));
        list.push(fill(Paint::Gradient(gradient(sweep, Extend::Pad, id)), FillRule::NonZero));
        list.push(Op::PopLayer);
        list.push(Op::PopClip);
        list.push(fill(Paint::Solid(Rgba { r: 1, g: 2, b: 3, a: 4 }), FillRule::NonZero));

        let s = ColorScene::new(list).expect("a small scene flattens");
        let kinds: Vec<i32> = s.ops.iter().map(|o| o.kind).collect();
        assert_eq!(kinds, [1, 3, 0, 0, 0, 4, 2, 0], "op kinds");
        assert_eq!((s.ops[0].clip_start, s.ops[0].clip_count), (0, 2));
        assert_eq!((s.clips[1].path, s.clips[1].rule, s.clips[1].transform), (1, 1, moved_and_doubled));
        assert_eq!((s.ops[1].opacity, s.ops[1].blend), (0.5, 23), "COLR's COMPOSITE_MULTIPLY is 23");
        assert_eq!((s.ops[2].rule, s.ops[2].paint, s.ops[2].gradient, s.ops[2].path), (1, 1, 0, 1));
        assert_eq!(s.ops[2].transform, counting);
        assert_eq!((s.ops[7].paint, s.ops[7].rgba), (0, [1, 2, 3, 4]));

        let g: Vec<(i32, i32)> = s.flat.iter().map(|g| (g.kind, g.extend)).collect();
        assert_eq!(g, [(0, 1), (1, 2), (2, 0)], "COLR's EXTEND_REPEAT is 1 and EXTEND_REFLECT is 2");
        assert_eq!(s.flat[1].numbers, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
        assert_eq!(s.flat[2].numbers[..4], [7.0, 8.0, 9.0, 10.0]);
        assert_eq!((s.flat[1].stops_start, s.flat[1].stops_count), (2, 2));
        assert_eq!(s.stop_offsets[..2], [0.0, 1.0], "stops are sorted and clamped");
        assert_eq!(s.stop_colors[..8], [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn a_ramp_samples_through_the_gradient_transform() {
        let mut list = DisplayList::default();
        let p = list.push_path(crate::Path::default());
        let linear = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 100.0, y1: 0.0 };
        let moved = [1.0, 0.0, 0.0, 1.0, 100.0, 0.0];
        for extend in [Extend::Pad, Extend::Repeat] {
            let paint = Paint::Gradient(gradient(linear, extend, moved));
            list.push(Op::Fill { path: p, paint, rule: FillRule::NonZero, transform: moved });
        }
        let s = ColorScene::new(list).expect("flattens");
        let ramp = |g: u32| {
            let mut out = core::ptr::null_mut();
            let to_device = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
            let st = unsafe { daegun_color_scene_ramp(&s, g, to_device.as_ptr(), &mut out) };
            assert_eq!(st, Status::Ok);
            out
        };
        let at = |r: *mut Ramp, x: f64| {
            let mut c = [0u8; 4];
            assert_eq!(unsafe { daegun_ramp_sample(r, x, 0.0, c.as_mut_ptr()) }, Status::Ok);
            c
        };
        let (pad, repeat) = (ramp(0), ramp(1));
        // Pixel 100 is sampled at its center, 0.5 into the moved gradient: t = 0.005 of a color line that
        // runs to 1.5, so red and blue blend at 1/300 in linear light, 255 and 11 once encoded. Repeat
        // comes round every 150 pixels.
        assert_eq!(at(pad, 100.0), [255, 0, 11, 255], "the gradient starts where its transform moved it");
        assert_eq!(at(pad, 400.0), [0, 0, 255, 255], "pad holds the last stop past the end");
        assert_eq!(at(repeat, 124.5), at(repeat, 274.5), "repeat comes round at the last stop");
        assert_ne!(at(repeat, 124.5), at(repeat, 224.5), "repeat came round at 1, not at the last stop");
        unsafe {
            daegun_ramp_free(pad);
            daegun_ramp_free(repeat);
        }

        let mut out = core::ptr::null_mut();
        let bad = [f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0];
        assert_eq!(unsafe { daegun_color_scene_ramp(&s, 0, bad.as_ptr(), &mut out) }, Status::Range);
        let id = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        assert_eq!(unsafe { daegun_color_scene_ramp(&s, 9, id.as_ptr(), &mut out) }, Status::Range);
    }
}
