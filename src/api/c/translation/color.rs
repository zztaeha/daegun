// color.h: bitmaps, COLR layers and palettes, scenes, ramps and compositing, and paint graphs.

use alloc::vec::Vec;
use core::ffi::c_void;

use crate::color::{
    Blend, ClipShape, ColorStop, ColrLayer, DisplayList, Extend, GradientId, GradientKind, Op, Paint,
    PaintNode, PaintVisitor, PathId, Rgba, SceneOptions, Stop,
};
use crate::color::gradient::{Interpolation, Ramp};
use crate::ffi::font::LocationHandle;
use crate::daecore::daekit::base::sync::heap_of;
use crate::ffi::handle::{
    Options, Reused, Status, array_of, borrow, cleared, defaults_into, deliver_reused, failed, field_view, give_back,
    into_buffer, into_buffers, invalid, keep_spare, options_of, or_return, past_end, put, put_some, reused, reused_box,
};
use crate::outline::{FillRule, Path, Point};
use crate::{Bounds, Font, GlyphId};

const _: () = assert!(size_of::<Rgba>() == 4 && size_of::<ColrLayer>() == 12 && size_of::<Stop>() == 8);
const _: () = assert!(size_of::<ColorStop>() == 16);

// bitmaps

pub const BITMAP_PNG: i32 = 0;
pub const BITMAP_COVERAGE: i32 = 1;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_bitmap(
    font: *const Font,
    gid: u32,
    ppem: u16,
    out: *mut *mut Bitmap,
) -> Status {
    use crate::color::BitmapImage;
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(b) = font.bitmap(GlyphId(gid), ppem) else { return Status::Absent };
    let mut bitmap = reused::<Bitmap>();
    (bitmap.format, bitmap.width, bitmap.height) = match b.image {
        BitmapImage::Png(png) => {
            bitmap.data.extend_from_slice(png);
            (BITMAP_PNG, 0, 0)
        }
        BitmapImage::Coverage { width, height, pixels } => {
            bitmap.data = pixels;
            (BITMAP_COVERAGE, width, height)
        }
    };
    (bitmap.ppem, bitmap.left, bitmap.top, bitmap.mirrored) = (b.ppem, b.left, b.top, b.mirrored);
    unsafe { *out = alloc::boxed::Box::into_raw(bitmap) };
    Status::Ok
}

// A bitmap as C holds it, its image copied out of the font, which the handle may outlive, into the room a freed one
// left.
pub struct Bitmap {
    format: i32,
    width: u16,
    height: u16,
    data: Vec<u8>,
    ppem: u16,
    left: i16,
    top: i16,
    mirrored: bool,
}

impl Reused for Bitmap {
    fn empty() -> Bitmap {
        Bitmap { format: BITMAP_PNG, width: 0, height: 0, data: Vec::new(), ppem: 0, left: 0, top: 0, mirrored: false }
    }

    fn emptied(&mut self) -> usize {
        self.data.clear();
        self.data.capacity()
    }

    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<alloc::boxed::Box<Bitmap>>>> {
        std::thread_local! {
            static SPARE: core::cell::Cell<Option<alloc::boxed::Box<Bitmap>>> = const { core::cell::Cell::new(None) };
        }
        &SPARE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_bitmap_free(bitmap: *mut Bitmap) {
    unsafe { give_back(bitmap) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct BitmapInfoC {
    format: i32,
    ppem: u16,
    left: i16,
    top: i16,
    width: u16,
    height: u16,
    mirrored: bool,
    data: *const u8,
    data_len: usize,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_bitmap_info_of(bitmap: *const Bitmap, out: *mut BitmapInfoC) -> Status {
    let Some(b) = (unsafe { borrow(bitmap) }) else { return Status::Null };
    let info = BitmapInfoC {
        format: b.format,
        ppem: b.ppem,
        left: b.left,
        top: b.top,
        width: b.width,
        height: b.height,
        mirrored: b.mirrored,
        data: b.data.as_ptr(),
        data_len: b.data.len(),
    };
    unsafe { put(out, info) }
}

// COLR version 0 and palettes

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_colr_layers(
    font: *const Font,
    gid: u32,
    palette: u16,
    out: *mut ColrLayer,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    match font.colr_layers(GlyphId(gid), palette) {
        Some(layers) => unsafe { into_buffer(layers.into_iter(), out, out_len, out_count) },
        None => Status::Absent,
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PaletteC {
    index: u16,
    name_id: u16,
    light_safe: bool,
    dark_safe: bool,
    has_name_id: bool,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_palettes(
    font: *const Font,
    out: *mut PaletteC,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let palettes = font.each_palette().map(|p| PaletteC {
        index: p.index,
        name_id: p.name_id.map_or(0, |id| id.0),
        light_safe: p.light_safe,
        dark_safe: p.dark_safe,
        has_name_id: p.name_id.is_some(),
    });
    unsafe { into_buffer(palettes, out, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_palette_entry_labels(
    font: *const Font,
    out_name_ids: *mut u16,
    out_named: *mut u8,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let labels = font.palette_entry_labels().into_iter().map(|l| (l.map_or(0, |id| id.0), u8::from(l.is_some())));
    unsafe { into_buffers(labels, out_name_ids, out_named, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_palette_colors(
    font: *const Font,
    palette: u16,
    out: *mut Rgba,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { into_buffer(font.palette_colors(palette).into_iter(), out, out_len, out_count) }
}

// scenes

pub const FILL_NONZERO: i32 = 0;
pub const FILL_EVENODD: i32 = 1;
pub const EXTEND_PAD: i32 = 0;
pub const EXTEND_REPEAT: i32 = 1;
pub const EXTEND_REFLECT: i32 = 2;
pub const OP_FILL: i32 = 0;
pub const OP_PUSH_CLIP: i32 = 1;
pub const OP_POP_CLIP: i32 = 2;
pub const OP_PUSH_LAYER: i32 = 3;
pub const OP_POP_LAYER: i32 = 4;
pub const GRADIENT_LINEAR: i32 = 0;
pub const GRADIENT_RADIAL: i32 = 1;
pub const GRADIENT_SWEEP: i32 = 2;

fn rule_code(rule: FillRule) -> i32 {
    match rule {
        FillRule::NonZero => FILL_NONZERO,
        FillRule::EvenOdd => FILL_EVENODD,
    }
}

// COLR's numbering, which Extend's order is not.
fn extend_code(extend: Extend) -> i32 {
    match extend {
        Extend::Pad => EXTEND_PAD,
        Extend::Repeat => EXTEND_REPEAT,
        Extend::Reflect => EXTEND_REFLECT,
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SceneOptionsC {
    size: usize,
    pub(crate) foreground: Rgba,
    pub(crate) palette: u16,
}

impl Options for SceneOptionsC {
    unsafe fn set_defaults(p: *mut Self) {
        unsafe { (&raw mut (*p).foreground).write(SceneOptions::default().foreground) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_scene_options_default(out: *mut SceneOptionsC, size: usize) -> Status {
    unsafe { defaults_into(out, size) }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct OpC {
    transform: [f64; 6],
    kind: i32,
    rule: i32,
    blend: i32,
    opacity: f32,
    path: usize,
    gradient: usize,
    shapes_start: usize,
    shapes_end: usize,
    color: Rgba,
    has_gradient: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ClipShapeC {
    transform: [f64; 6],
    path: usize,
    rule: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GradientC {
    transform: [f64; 6],
    values: [f32; 6],
    kind: i32,
    extend: i32,
    stops_start: usize,
    stops_count: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct PathViewC {
    verbs: *const u8,
    verbs_len: usize,
    points: *const Point,
    points_len: usize,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    size_of::<SceneOptionsC>() == 16
        && size_of::<BitmapInfoC>() == 32
        && size_of::<OpC>() == 104
        && size_of::<ClipShapeC>() == 64
        && size_of::<GradientC>() == 96
        && size_of::<PathViewC>() == 32
        && size_of::<NodeC>() == 112
);

// A display list laid out as C reads it: its gradients by id with their stops in one list, and each clip's shapes
// in another, which its op indexes.
pub struct Scene {
    list: DisplayList,
    paths: usize,
    ops: Vec<OpC>,
    shapes: Vec<ClipShapeC>,
    gradients: Vec<GradientC>,
    stops: Vec<Stop>,
}

impl Scene {
    // C's lists, from the display list the scene holds.
    fn fill(&mut self) {
        let Scene { list, paths, ops, shapes, gradients, stops } = self;
        for g in list.gradients() {
            let (kind, values) = match g.kind {
                GradientKind::Linear { x0, y0, x1, y1 } => (GRADIENT_LINEAR, [x0, y0, x1, y1, 0.0, 0.0]),
                GradientKind::Radial { x0, y0, r0, x1, y1, r1 } => (GRADIENT_RADIAL, [x0, y0, r0, x1, y1, r1]),
                GradientKind::Sweep { cx, cy, start_angle, end_angle } => {
                    (GRADIENT_SWEEP, [cx, cy, start_angle, end_angle, 0.0, 0.0])
                }
            };
            let (stops_start, stops_count) = (stops.len(), g.stops.len());
            stops.extend_from_slice(g.stops);
            let extend = extend_code(g.extend);
            gradients.push(GradientC { transform: g.transform, values, kind, extend, stops_start, stops_count });
        }
        ops.extend(list.ops().iter().map(|&op| match op {
            Op::Fill { path, paint, rule, transform } => {
                let (color, gradient, has_gradient) = match paint {
                    Paint::Solid(color) => (color, 0, false),
                    Paint::Gradient(id) => (Rgba::default(), id.0, true),
                };
                let rule = rule_code(rule);
                OpC { transform, kind: OP_FILL, rule, path: path.0, gradient, color, has_gradient, ..OpC::default() }
            }
            Op::PushClip { shapes: range } => {
                let shapes_start = shapes.len();
                shapes.extend(list.shapes(range).iter().map(|&ClipShape { path, rule, transform }| ClipShapeC {
                    transform,
                    path: path.0,
                    rule: rule_code(rule),
                }));
                OpC { kind: OP_PUSH_CLIP, shapes_start, shapes_end: shapes.len(), ..OpC::default() }
            }
            Op::PopClip => OpC { kind: OP_POP_CLIP, ..OpC::default() },
            // Blend is declared in COLR's composite-mode order, so its discriminant is the DAEGUN_COMPOSITE_* value.
            Op::PushLayer { opacity, blend } => {
                OpC { kind: OP_PUSH_LAYER, blend: blend as i32, opacity, ..OpC::default() }
            }
            Op::PopLayer => OpC { kind: OP_POP_LAYER, ..OpC::default() },
        }));
        *paths = list.paths().len();
    }
}

impl Reused for Scene {
    fn empty() -> Scene {
        let list = DisplayList::default();
        Scene { list, paths: 0, ops: Vec::new(), shapes: Vec::new(), gradients: Vec::new(), stops: Vec::new() }
    }

    fn emptied(&mut self) -> usize {
        self.list.clear();
        self.ops.clear();
        self.shapes.clear();
        self.gradients.clear();
        self.stops.clear();
        let held = heap_of(&self.ops) + heap_of(&self.shapes) + heap_of(&self.gradients) + heap_of(&self.stops);
        held + self.list.heap_bytes()
    }

    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<alloc::boxed::Box<Scene>>>> {
        std::thread_local! {
            static SPARE: core::cell::Cell<Option<alloc::boxed::Box<Scene>>> = const { core::cell::Cell::new(None) };
        }
        &SPARE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_color_scene(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    opts: *const SceneOptionsC,
    out: *mut *mut Scene,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let opts = or_return!(unsafe { options_of(opts) });
    let options = SceneOptions { palette: opts.palette, foreground: opts.foreground };
    let mut scene = reused::<Scene>();
    let at = unsafe { crate::ffi::at(location) };
    let drawn = font.scene_into(GlyphId(gid), at, &options, &mut scene.list).map(|list| list.is_some());
    if let Ok(true) = drawn {
        scene.fill();
        unsafe { *out = alloc::boxed::Box::into_raw(scene) };
        return Status::Ok;
    }
    keep_spare(scene);
    drawn.map_or_else(failed, |_| Status::Absent)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_scene_free(scene: *mut Scene) {
    unsafe { give_back(scene) }
}

field_view!(daegun_scene_ops, Scene, ops, OpC);
field_view!(daegun_scene_shapes, Scene, shapes, ClipShapeC);
field_view!(daegun_scene_gradients, Scene, gradients, GradientC);
field_view!(daegun_scene_stops, Scene, stops, Stop);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_scene_path_count(scene: *const Scene, out: *mut usize) -> Status {
    let Some(scene) = (unsafe { borrow(scene) }) else { return Status::Null };
    unsafe { put(out, scene.paths) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_scene_path_at(scene: *const Scene, path: usize, out: *mut PathViewC) -> Status {
    let Some(scene) = (unsafe { borrow(scene) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(view) = scene.list.path(PathId(path)) else { return past_end("path", path, scene.paths) };
    let (verbs, points) = view.parts();
    let view = PathViewC {
        verbs: verbs.as_ptr().cast(),
        verbs_len: verbs.len(),
        points: points.as_ptr(),
        points_len: points.len(),
    };
    unsafe { put(out, view) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_scene_path_copy(scene: *const Scene, path: usize, out: *mut *mut Path) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(scene) = (unsafe { borrow(scene) }) else { return Status::Null };
    let Some(view) = scene.list.path(PathId(path)) else { return past_end("path", path, scene.paths) };
    let mut copy = reused::<Path>();
    view.copy_into(&mut copy);
    unsafe { *out = alloc::boxed::Box::into_raw(copy) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_colr_clip_box(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    out: *mut Bounds,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { put_some(out, font.colr_clip_box(GlyphId(gid), crate::ffi::at(location))) }
}

// sampling and compositing

pub const INTERPOLATE_LINEAR_LIGHT: i32 = 0;
pub const INTERPOLATE_SRGB: i32 = 1;

fn interpolation_of(code: i32) -> Result<Interpolation, Status> {
    match code {
        INTERPOLATE_LINEAR_LIGHT => Ok(Interpolation::LinearLight),
        INTERPOLATE_SRGB => Ok(Interpolation::Srgb),
        _ => Err(invalid(format_args!("interpolation {code} is none of DAEGUN_INTERPOLATE_*"))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_scene_ramp(
    scene: *const Scene,
    gradient: usize,
    to_device: *const f64,
    interpolation: i32,
    out: *mut *mut Ramp,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(scene) = (unsafe { borrow(scene) }) else { return Status::Null };
    let Some(to_device) = (unsafe { array_of::<f64, 6>(to_device) }) else { return Status::Null };
    let how = or_return!(interpolation_of(interpolation));
    let Some(g) = scene.list.gradient(GradientId(gradient)) else {
        return past_end("gradient", gradient, scene.gradients.len());
    };
    unsafe { deliver_reused(out, Ramp::with_interpolation(&g, &to_device, how)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ramp_at(ramp: *const Ramp, x: f64, y: f64, out: *mut Rgba) -> Status {
    let Some(ramp) = (unsafe { borrow(ramp) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { put_some(out, ramp.at(x, y)) }
}

reused_box!(daegun_ramp_free, Ramp, Ramp::NONE);

fn blend_mode(code: i32) -> Result<Blend, Status> {
    match u8::try_from(code) {
        Ok(mode @ 0..=27) => Ok(Blend::from_colr(mode)),
        _ => Err(invalid(format_args!("mode {code} is none of DAEGUN_COMPOSITE_*"))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_blend(mode: i32, backdrop: *const f32, source: *const f32, out: *mut f32) -> Status {
    let (Some(backdrop), Some(source)) = (unsafe { (array_of::<f32, 3>(backdrop), array_of::<f32, 3>(source)) }) else {
        return Status::Null;
    };
    if out.is_null() {
        return Status::Null;
    }
    let mode = or_return!(blend_mode(mode));
    unsafe { put(out.cast::<[f32; 3]>(), mode.blend(backdrop, source)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_composite(
    mode: i32,
    source: *const f32,
    source_alpha: f32,
    backdrop: *const f32,
    backdrop_alpha: f32,
    out: *mut f32,
    out_alpha: *mut f32,
) -> Status {
    let (Some(source), Some(backdrop)) = (unsafe { (array_of::<f32, 3>(source), array_of::<f32, 3>(backdrop)) }) else {
        return Status::Null;
    };
    if out.is_null() || out_alpha.is_null() {
        return Status::Null;
    }
    let mode = or_return!(blend_mode(mode));
    let composited = mode.composite(source, source_alpha, backdrop, backdrop_alpha);
    unsafe {
        out.cast::<[f32; 3]>().write(composited.color);
        out_alpha.write(composited.alpha);
    }
    Status::Ok
}

// paint graphs

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct NodeC {
    values: [f64; 6],
    center: [f64; 2],
    end: usize,
    stops_start: usize,
    stops_count: usize,
    glyph: u32,
    kind: i32,
    extend: i32,
    mode: i32,
    color: Rgba,
    foreground: bool,
    has_center: bool,
}

pub const PAINT_LAYERS: i32 = 0;
pub const PAINT_GLYPH: i32 = 1;
pub const PAINT_COLR_GLYPH: i32 = 2;
pub const PAINT_SOLID: i32 = 3;
pub const PAINT_LINEAR_GRADIENT: i32 = 4;
pub const PAINT_RADIAL_GRADIENT: i32 = 5;
pub const PAINT_SWEEP_GRADIENT: i32 = 6;
pub const PAINT_TRANSFORM: i32 = 7;
pub const PAINT_TRANSLATE: i32 = 8;
pub const PAINT_SCALE: i32 = 9;
pub const PAINT_SCALE_UNIFORM: i32 = 10;
pub const PAINT_ROTATE: i32 = 11;
pub const PAINT_SKEW: i32 = 12;
pub const PAINT_COMPOSITE: i32 = 13;

// A paint's own fields as C reads them, and its gradient's stops.
#[inline(always)]
fn node_of(node: PaintNode<'_>) -> (NodeC, &[ColorStop]) {
    let at = |center: Option<crate::color::Center>| center.map_or([0.0; 2], |c| [c.x, c.y]);
    let mut stops: &[ColorStop] = &[];
    let c = match node {
        PaintNode::Layers => NodeC { kind: PAINT_LAYERS, ..NodeC::default() },
        PaintNode::Glyph { glyph } => NodeC { kind: PAINT_GLYPH, glyph: glyph.0, ..NodeC::default() },
        PaintNode::ColrGlyph { glyph } => NodeC { kind: PAINT_COLR_GLYPH, glyph: glyph.0, ..NodeC::default() },
        PaintNode::Solid { color, foreground } => NodeC { kind: PAINT_SOLID, color, foreground, ..NodeC::default() },
        PaintNode::LinearGradient { extend, stops: s, x0, y0, x1, y1, x2, y2 } => {
            stops = s;
            let values = [x0, y0, x1, y1, x2, y2];
            NodeC { kind: PAINT_LINEAR_GRADIENT, values, extend: extend_code(extend), ..NodeC::default() }
        }
        PaintNode::RadialGradient { extend, stops: s, x0, y0, r0, x1, y1, r1 } => {
            stops = s;
            let values = [x0, y0, r0, x1, y1, r1];
            NodeC { kind: PAINT_RADIAL_GRADIENT, values, extend: extend_code(extend), ..NodeC::default() }
        }
        PaintNode::SweepGradient { extend, stops: s, cx, cy, start_angle, end_angle } => {
            stops = s;
            let values = [cx, cy, start_angle, end_angle, 0.0, 0.0];
            NodeC { kind: PAINT_SWEEP_GRADIENT, values, extend: extend_code(extend), ..NodeC::default() }
        }
        PaintNode::Transform { matrix } => NodeC { kind: PAINT_TRANSFORM, values: matrix, ..NodeC::default() },
        PaintNode::Translate { dx, dy } => {
            NodeC { kind: PAINT_TRANSLATE, values: [dx, dy, 0.0, 0.0, 0.0, 0.0], ..NodeC::default() }
        }
        PaintNode::Scale { sx, sy, center } => NodeC {
            kind: PAINT_SCALE,
            values: [sx, sy, 0.0, 0.0, 0.0, 0.0],
            center: at(center),
            has_center: center.is_some(),
            ..NodeC::default()
        },
        PaintNode::ScaleUniform { s, center } => NodeC {
            kind: PAINT_SCALE_UNIFORM,
            values: [s, 0.0, 0.0, 0.0, 0.0, 0.0],
            center: at(center),
            has_center: center.is_some(),
            ..NodeC::default()
        },
        PaintNode::Rotate { angle, center } => NodeC {
            kind: PAINT_ROTATE,
            values: [angle, 0.0, 0.0, 0.0, 0.0, 0.0],
            center: at(center),
            has_center: center.is_some(),
            ..NodeC::default()
        },
        PaintNode::Skew { x_angle, y_angle, center } => NodeC {
            kind: PAINT_SKEW,
            values: [x_angle, y_angle, 0.0, 0.0, 0.0, 0.0],
            center: at(center),
            has_center: center.is_some(),
            ..NodeC::default()
        },
        PaintNode::Composite { mode } => NodeC { kind: PAINT_COMPOSITE, mode: mode as i32, ..NodeC::default() },
    };
    (NodeC { stops_count: stops.len(), ..c }, stops)
}

// A graph laid out as C reads it, built as a visitor walks it: a paint's node where it is entered, its end once it
// is left. While a paint is open its `end` holds the paint it lies in, as Rust's graph keeps it.
pub struct PaintGraphC {
    nodes: Vec<NodeC>,
    stops: Vec<ColorStop>,
    // The innermost paint entered and not yet left, `NONE` where none is.
    open: usize,
}

const NONE: usize = usize::MAX;

impl PaintGraphC {
    // Each paint never left ends past every paint, as Rust's graph reads one.
    fn close(&mut self) {
        let len = self.nodes.len();
        while let Some(node) = self.nodes.get_mut(self.open) {
            self.open = node.end;
            node.end = len;
        }
    }
}

impl PaintVisitor for PaintGraphC {
    // In each arm of the walk, where the paint's kind is known, as Rust's graph builds: out of line, each paint went
    // through the stack and matched on its kind twice.
    #[inline(always)]
    fn enter(&mut self, node: PaintNode<'_>) {
        let (node, stops) = node_of(node);
        let at = self.nodes.len();
        self.nodes.push(NodeC { stops_start: self.stops.len(), end: self.open, ..node });
        self.stops.extend_from_slice(stops);
        self.open = at;
    }

    // A leave with no paint open is passed by.
    #[inline(always)]
    fn leave(&mut self) {
        let len = self.nodes.len();
        if let Some(node) = self.nodes.get_mut(self.open) {
            self.open = node.end;
            node.end = len;
        }
    }
}

impl Reused for PaintGraphC {
    fn empty() -> PaintGraphC {
        PaintGraphC { nodes: Vec::new(), stops: Vec::new(), open: NONE }
    }

    fn emptied(&mut self) -> usize {
        self.nodes.clear();
        self.stops.clear();
        self.open = NONE;
        heap_of(&self.nodes) + heap_of(&self.stops)
    }

    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<alloc::boxed::Box<PaintGraphC>>>> {
        std::thread_local! {
            static SPARE: core::cell::Cell<Option<alloc::boxed::Box<PaintGraphC>>> =
                const { core::cell::Cell::new(None) };
        }
        &SPARE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_paint_graph(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    palette: u16,
    out: *mut *mut PaintGraphC,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let mut graph = reused::<PaintGraphC>();
    if !font.visit_paint_graph(GlyphId(gid), unsafe { crate::ffi::at(location) }, palette, &mut *graph) {
        keep_spare(graph);
        return Status::Absent;
    }
    graph.close();
    unsafe { *out = alloc::boxed::Box::into_raw(graph) };
    Status::Ok
}

field_view!(daegun_paint_graph_nodes, PaintGraphC, nodes, NodeC);
field_view!(daegun_paint_graph_stops, PaintGraphC, stops, ColorStop);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_paint_graph_free(graph: *mut PaintGraphC) {
    unsafe { give_back(graph) }
}

// The caller's callbacks, unsafe to call as a pen's are.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VisitorC {
    enter: Option<unsafe extern "C" fn(*mut c_void, *const NodeC, *const ColorStop, usize)>,
    leave: Option<unsafe extern "C" fn(*mut c_void)>,
    user: *mut c_void,
}

struct VisitorBridge(VisitorC);

impl PaintVisitor for VisitorBridge {
    fn enter(&mut self, node: PaintNode<'_>) {
        if let Some(f) = self.0.enter {
            let (node, stops) = node_of(node);
            let (count, stops) = (stops.len(), if stops.is_empty() { core::ptr::null() } else { stops.as_ptr() });
            unsafe { f(self.0.user, &node, stops, count) };
        }
    }

    fn leave(&mut self) {
        if let Some(f) = self.0.leave {
            unsafe { f(self.0.user) };
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_visit_paint_graph(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    palette: u16,
    visitor: *const VisitorC,
) -> Status {
    let (Some(font), Some(visitor)) = (unsafe { (borrow(font), borrow(visitor)) }) else { return Status::Null };
    let at = unsafe { crate::ffi::at(location) };
    match font.visit_paint_graph(GlyphId(gid), at, palette, &mut VisitorBridge(*visitor)) {
        true => Status::Ok,
        false => Status::Absent,
    }
}

pub(crate) fn release_thread() {
    crate::ffi::handle::drop_spare::<Bitmap>();
    crate::ffi::handle::drop_spare::<Scene>();
    crate::ffi::handle::drop_spare::<Ramp>();
    crate::ffi::handle::drop_spare::<PaintGraphC>();
}

#[cfg(test)]
#[path = "tests/color.rs"]
mod tests;
