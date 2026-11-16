// outline.h: pens, glyph outlines, paths, strokes, flattening, quadratic curves, hinting, CFF hints and prepared
// outlines.

use alloc::vec::Vec;
use core::ffi::c_void;

use crate::ffi::font::LocationHandle;
use crate::ffi::handle::{
    Options, Reused, Status, array_of, borrow, borrow_mut, cleared, deliver, deliver_reused, defaults_into, failed,
    field_view, free, give_back, invalid, keep_spare, options_of, or_return, past_end, put, put_some, reused,
    reused_box, slice_of, view,
};
use crate::outline::{
    Cap, CffHints, CffStem, HintMode, HintedOutline, Join, OutlineOptions, OutlinePen, Path, Point, PreparedGlyph,
    Quad, QuadraticPen, StrokeStyle,
};
use crate::{Bounds, Font, GlyphId};

// pens

// Each callback is the caller's code, so calling one is unsafe; base.h's CALLBACKS has it return normally.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Pen {
    move_to: Option<unsafe extern "C" fn(*mut c_void, f32, f32)>,
    line_to: Option<unsafe extern "C" fn(*mut c_void, f32, f32)>,
    quad_to: Option<unsafe extern "C" fn(*mut c_void, f32, f32, f32, f32)>,
    curve_to: Option<unsafe extern "C" fn(*mut c_void, f32, f32, f32, f32, f32, f32)>,
    close: Option<unsafe extern "C" fn(*mut c_void)>,
    user: *mut c_void,
}

const _: () = assert!(size_of::<Pen>() == 6 * size_of::<usize>());

// A caller's pen as Rust draws onto one, a NULL callback skipping its event.
pub struct PenBridge(pub Pen);

impl OutlinePen for PenBridge {
    fn move_to(&mut self, x: f32, y: f32) {
        if let Some(f) = self.0.move_to {
            unsafe { f(self.0.user, x, y) };
        }
    }

    fn line_to(&mut self, x: f32, y: f32) {
        if let Some(f) = self.0.line_to {
            unsafe { f(self.0.user, x, y) };
        }
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if let Some(f) = self.0.quad_to {
            unsafe { f(self.0.user, cx, cy, x, y) };
        }
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        if let Some(f) = self.0.curve_to {
            unsafe { f(self.0.user, c1x, c1y, c2x, c2y, x, y) };
        }
    }

    fn close(&mut self) {
        if let Some(f) = self.0.close {
            unsafe { f(self.0.user) };
        }
    }
}

const _: () = assert!(size_of::<Point>() == 8 && size_of::<Quad>() == 24 && size_of::<CffStem>() == 12);
const _: () = assert!(size_of::<PreparedGlyph>() == 12);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_outline(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    pen: *const Pen,
) -> Status {
    let (Some(font), Some(pen)) = (unsafe { (borrow(font), borrow(pen)) }) else { return Status::Null };
    match font.outline(GlyphId(gid), unsafe { crate::ffi::at(location) }, &mut PenBridge(*pen)) {
        Ok(()) => Status::Ok,
        Err(e) => failed(e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_has_overlap_flag(font: *const Font, gid: u32, out: *mut bool) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put(out, font.has_overlap_flag(GlyphId(gid))) }
}

// paths

// A freed path is the thread's next, its room kept, as a new box and lists cost C's small paths 50 ns.
impl Reused for Path {
    fn empty() -> Path {
        Path::new()
    }

    fn emptied(&mut self) -> usize {
        self.clear();
        self.heap_bytes()
    }

    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<alloc::boxed::Box<Path>>>> {
        std::thread_local! {
            static SPARE: core::cell::Cell<Option<alloc::boxed::Box<Path>>> = const { core::cell::Cell::new(None) };
        }
        &SPARE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_free(path: *mut Path) {
    unsafe { give_back(path) }
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_path_new() -> *mut Path {
    alloc::boxed::Box::into_raw(reused::<Path>())
}

// One of the five calls a pen makes, or clear, onto a path.
macro_rules! path_call {
    ($name:ident, $method:ident $(, $arg:ident)*) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(path: *mut Path $(, $arg: f32)*) -> Status {
            let Some(path) = (unsafe { borrow_mut(path) }) else { return Status::Null };
            path.$method($($arg),*);
            Status::Ok
        }
    };
}

path_call!(daegun_path_move_to, move_to, x, y);
path_call!(daegun_path_line_to, line_to, x, y);
path_call!(daegun_path_quad_to, quad_to, cx, cy, x, y);
path_call!(daegun_path_curve_to, curve_to, c1x, c1y, c2x, c2y, x, y);
path_call!(daegun_path_close, close);
path_call!(daegun_path_clear, clear);

// The pen's callbacks take the path as `user`, which daegun_path_as_pen set to a live path.
unsafe fn pen_path<'a>(user: *mut c_void) -> Option<&'a mut Path> {
    unsafe { borrow_mut(user.cast::<Path>()) }
}

extern "C" fn path_move_to(user: *mut c_void, x: f32, y: f32) {
    if let Some(p) = unsafe { pen_path(user) } {
        p.move_to(x, y);
    }
}

extern "C" fn path_line_to(user: *mut c_void, x: f32, y: f32) {
    if let Some(p) = unsafe { pen_path(user) } {
        p.line_to(x, y);
    }
}

extern "C" fn path_quad_to(user: *mut c_void, cx: f32, cy: f32, x: f32, y: f32) {
    if let Some(p) = unsafe { pen_path(user) } {
        p.quad_to(cx, cy, x, y);
    }
}

extern "C" fn path_curve_to(user: *mut c_void, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
    if let Some(p) = unsafe { pen_path(user) } {
        p.curve_to(c1x, c1y, c2x, c2y, x, y);
    }
}

extern "C" fn path_close(user: *mut c_void) {
    if let Some(p) = unsafe { pen_path(user) } {
        p.close();
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_as_pen(path: *mut Path, out: *mut Pen) -> Status {
    if path.is_null() {
        return Status::Null;
    }
    let pen = Pen {
        move_to: Some(path_move_to),
        line_to: Some(path_line_to),
        quad_to: Some(path_quad_to),
        curve_to: Some(path_curve_to),
        close: Some(path_close),
        user: path.cast(),
    };
    unsafe { put(out, pen) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_is_empty(path: *const Path, out: *mut bool) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    unsafe { put(out, path.is_empty()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_cost(path: *const Path, out: *mut usize) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    unsafe { put(out, path.cost()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_bounds(path: *const Path, out: *mut Bounds) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { put_some(out, path.bounds()) }
}

// A verb is a byte of its DAEGUN_VERB_* value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_verbs(path: *const Path, out_count: *mut usize) -> *const u8 {
    unsafe { view(path, out_count, |p| Some(p.parts().0)) }.cast()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_points(path: *const Path, out_count: *mut usize) -> *const Point {
    unsafe { view(path, out_count, |p| Some(p.parts().1)) }
}

std::thread_local! {
    // The copy a replay or a stroke reads, kept for the thread's next unless past the spares' bound: a new one each
    // call took C's replay 1.56× Rust's.
    static REPLAYED: core::cell::RefCell<Path> = const { core::cell::RefCell::new(Path::new()) };
}

// `f` on a copy of the path, as a pen may draw into this very path, which would move what is being read. The copy is
// the thread's, kept between calls while it's small; a call the pen makes meanwhile copies into its own.
fn copied<R>(path: &Path, f: impl FnOnce(&Path) -> R) -> R {
    let take = |kept: &core::cell::RefCell<Path>| kept.try_borrow_mut().ok().map(|mut k| core::mem::take(&mut *k));
    let mut copy = REPLAYED.try_with(take).ok().flatten().unwrap_or_default();
    copy.clone_from(path);
    let out = f(&copy);
    if copy.cost() <= crate::ffi::handle::SPARE_BYTES {
        let _ = REPLAYED.try_with(|kept| kept.try_borrow_mut().map(|mut k| *k = copy));
    }
    out
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_replay(path: *const Path, transform: *const f64, pen: *const Pen) -> Status {
    let (Some(path), Some(pen)) = (unsafe { (borrow(path), borrow(pen)) }) else { return Status::Null };
    let transform = unsafe { array_of::<f64, 6>(transform) };
    // Through the pen's table: the bridge inlined into the loop holds the five callbacks across each call, which
    // took the replay 1.19× as long.
    let replay = |copy: &Path| copy.replay(transform.as_ref(), &mut PenBridge(*pen) as &mut dyn OutlinePen);
    copied(path, replay);
    Status::Ok
}

// strokes

pub const JOIN_MITER: i32 = 0;
pub const JOIN_ROUND: i32 = 1;
pub const JOIN_BEVEL: i32 = 2;
pub const CAP_BUTT: i32 = 0;
pub const CAP_ROUND: i32 = 1;
pub const CAP_SQUARE: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StrokeStyleC {
    width: f32,
    cap: i32,
    join: i32,
    miter_limit: f32,
}

const _: () = assert!(size_of::<StrokeStyleC>() == 16);

impl StrokeStyleC {
    fn rust(self) -> Result<StrokeStyle, Status> {
        let cap = match self.cap {
            CAP_BUTT => Cap::Butt,
            CAP_ROUND => Cap::Round,
            CAP_SQUARE => Cap::Square,
            code => return Err(invalid(format_args!("cap {code} is none of DAEGUN_CAP_*"))),
        };
        let join = match self.join {
            JOIN_MITER => Join::Miter { limit: self.miter_limit },
            JOIN_ROUND => Join::Round,
            JOIN_BEVEL => Join::Bevel,
            code => return Err(invalid(format_args!("join {code} is none of DAEGUN_JOIN_*"))),
        };
        Ok(StrokeStyle { width: self.width, join, cap })
    }
}

unsafe fn stroke_onto(
    path: *const Path,
    style: *const StrokeStyleC,
    tolerance: f32,
    pen: *const Pen,
    simplified: bool,
) -> Status {
    let (Some(path), Some(style), Some(pen)) = (unsafe { (borrow(path), borrow(style), borrow(pen)) }) else {
        return Status::Null;
    };
    let style = or_return!(style.rust());
    let mut pen = PenBridge(*pen);
    let stroke = |copy: &Path| match simplified {
        true => copy.stroke_simplified(&style, tolerance, &mut pen),
        false => copy.stroke(&style, tolerance, &mut pen),
    };
    match copied(path, stroke) {
        Ok(()) => Status::Ok,
        Err(e) => failed(e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_stroke(
    path: *const Path,
    style: *const StrokeStyleC,
    tolerance: f32,
    pen: *const Pen,
) -> Status {
    unsafe { stroke_onto(path, style, tolerance, pen, false) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_stroke_simplified(
    path: *const Path,
    style: *const StrokeStyleC,
    tolerance: f32,
    pen: *const Pen,
) -> Status {
    unsafe { stroke_onto(path, style, tolerance, pen, true) }
}

// flattening and overlaps

#[unsafe(no_mangle)]
pub extern "C" fn daegun_max_area_for(px: f32, units_per_em: f32) -> f32 {
    crate::outline::max_area_for(px, units_per_em)
}

pub struct Contours(Vec<Vec<Point>>);

// Freed contours are the thread's next, their lists' room kept, which flattening fills again without allocating.
impl Reused for Contours {
    fn empty() -> Contours {
        Contours(Vec::new())
    }

    fn emptied(&mut self) -> usize {
        let lists = self.0.iter().map(|c| c.capacity() * size_of::<Point>()).sum::<usize>();
        lists + self.0.capacity() * size_of::<Vec<Point>>()
    }

    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<alloc::boxed::Box<Contours>>>> {
        std::thread_local! {
            static SPARE: core::cell::Cell<Option<alloc::boxed::Box<Contours>>> =
                const { core::cell::Cell::new(None) };
        }
        &SPARE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_flatten(path: *const Path, max_area: f32, out: *mut *mut Contours) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    let mut contours = reused::<Contours>();
    match path.flatten_into(max_area, &mut contours.0) {
        Ok(()) => {
            unsafe { *out = alloc::boxed::Box::into_raw(contours) };
            Status::Ok
        }
        Err(e) => {
            keep_spare(contours);
            failed(e)
        }
    }
}

// Lengths summing past a size_t, which no caller's points, held in memory, can reach.
fn overflowing_lengths() -> Status {
    invalid("the contours' lengths overflow a size_t")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_contours_new(
    points: *const Point,
    lengths: *const usize,
    count: usize,
    out: *mut *mut Contours,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(lengths) = (unsafe { slice_of(lengths, count) }) else { return Status::Null };
    let Some(total) = lengths.iter().try_fold(0usize, |sum, &n| sum.checked_add(n)) else {
        return overflowing_lengths();
    };
    let Some(mut points) = (unsafe { slice_of(points, total) }) else { return Status::Null };
    let mut contours = Vec::with_capacity(count);
    for &n in lengths {
        let (contour, rest) = points.split_at(n);
        contours.push(contour.to_vec());
        points = rest;
    }
    unsafe { deliver_reused(out, Contours(contours)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_contours_count(contours: *const Contours, out: *mut usize) -> Status {
    let Some(contours) = (unsafe { borrow(contours) }) else { return Status::Null };
    unsafe { put(out, contours.0.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_contours_points(
    contours: *const Contours,
    index: usize,
    out_count: *mut usize,
) -> *const Point {
    unsafe { view(contours, out_count, |c| c.0.get(index).map(Vec::as_slice)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_contours_free(contours: *mut Contours) {
    unsafe { give_back(contours) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_contours_resolve_overlaps(
    contours: *const Contours,
    out: *mut *mut Contours,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(contours) = (unsafe { borrow(contours) }) else { return Status::Null };
    match crate::outline::resolve_overlaps(&contours.0) {
        Some(resolved) => unsafe { deliver_reused(out, Contours(resolved)) },
        None => Status::Absent,
    }
}

// quadratic curves

pub struct Quads(Vec<Quad>);

// Freed curves are the thread's next, their room kept, which the next glyph fills again without allocating.
impl Reused for Quads {
    fn empty() -> Quads {
        Quads(Vec::new())
    }

    fn emptied(&mut self) -> usize {
        self.0.clear();
        self.0.capacity() * size_of::<Quad>()
    }

    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<alloc::boxed::Box<Quads>>>> {
        std::thread_local! {
            static SPARE: core::cell::Cell<Option<alloc::boxed::Box<Quads>>> = const { core::cell::Cell::new(None) };
        }
        &SPARE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_quads(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    out: *mut *mut Quads,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let mut quads = reused::<Quads>();
    let drawn = font.quads_into(GlyphId(gid), unsafe { crate::ffi::at(location) }, &mut quads.0);
    unsafe { quads_out(out, quads, drawn) }
}

unsafe fn quads_out(out: *mut *mut Quads, quads: alloc::boxed::Box<Quads>, drawn: crate::Result<()>) -> Status {
    match drawn {
        Ok(()) => {
            unsafe { *out = alloc::boxed::Box::into_raw(quads) };
            Status::Ok
        }
        Err(e) => {
            keep_spare(quads);
            failed(e)
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_quads(path: *const Path, tolerance: f32, out: *mut *mut Quads) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    let mut quads = reused::<Quads>();
    let mut pen = QuadraticPen::reusing(tolerance, core::mem::take(&mut quads.0));
    path.replay(None, &mut pen);
    let (curves, drawn) = pen.finish_apart();
    quads.0 = curves;
    unsafe { quads_out(out, quads, drawn) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_quads_normalize_winding(quads: *mut Quads) -> Status {
    let Some(quads) = (unsafe { borrow_mut(quads) }) else { return Status::Null };
    crate::outline::normalize_winding(&mut quads.0);
    Status::Ok
}

field_view!(daegun_quads_data, Quads, 0, Quad);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_quads_free(quads: *mut Quads) {
    unsafe { give_back(quads) }
}

// hinting

pub const HINT_NONE: i32 = 0;
pub const HINT_FONT: i32 = 1;
pub const HINT_AUTO: i32 = 3;
pub const HINT_AUTO_FORCE: i32 = 4;

fn hint_mode(code: i32) -> Result<HintMode, Status> {
    match code {
        HINT_NONE => Ok(HintMode::None),
        HINT_FONT => Ok(HintMode::Font),
        HINT_AUTO => Ok(HintMode::Auto),
        HINT_AUTO_FORCE => Ok(HintMode::AutoForce),
        _ => Err(invalid(format_args!("hint mode {code} is none of DAEGUN_HINT_*"))),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hint_mode_may_autohint(mode: i32, out: *mut bool) -> Status {
    let mode = or_return!(hint_mode(mode));
    unsafe { put(out, mode.may_autohint()) }
}

// Rust's outline, its points in pixels and their kinds apart, as C reads them, made when first asked for, once:
// made on every call, they took the autohinter's outline 5% over Rust's.
pub struct Hinted {
    outline: HintedOutline,
    points: crate::sync::Once<(Vec<Point>, Vec<u8>)>,
}

impl Hinted {
    fn points(&self) -> &(Vec<Point>, Vec<u8>) {
        self.points.get_or_init(|| self.outline.points().map(|(p, kind)| (p, kind as u8)).unzip())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_hinted_outline(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    px: f32,
    mode: i32,
    out: *mut *mut Hinted,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let mode = or_return!(hint_mode(mode));
    let outline = match font.hinted_outline(GlyphId(gid), unsafe { crate::ffi::at(location) }, px, mode) {
        Ok(Some(outline)) => outline,
        Ok(None) => return Status::Absent,
        Err(e) => return failed(e),
    };
    unsafe { deliver_reused(out, Hinted { outline, points: crate::sync::Once::new() }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hinted_outline_points(outline: *const Hinted, out_count: *mut usize) -> *const Point {
    unsafe { view(outline, out_count, |h| Some(&h.points().0[..])) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hinted_outline_kinds(outline: *const Hinted, out_count: *mut usize) -> *const u8 {
    unsafe { view(outline, out_count, |h| Some(&h.points().1[..])) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hinted_outline_contours(outline: *const Hinted, out_count: *mut usize) -> *const usize {
    unsafe { view(outline, out_count, |h| Some(h.outline.contours())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hinted_outline_draw(outline: *const Hinted, pen: *const Pen) -> Status {
    let (Some(outline), Some(pen)) = (unsafe { (borrow(outline), borrow(pen)) }) else { return Status::Null };
    outline.outline.draw(&mut PenBridge(*pen));
    Status::Ok
}

reused_box!(daegun_hinted_outline_free, Hinted, Hinted {
    outline: HintedOutline(Default::default()),
    points: crate::sync::Once::new(),
});

#[repr(C)]
#[derive(Clone, Copy)]
pub struct CffMaskC {
    point: usize,
    bits: *const u8,
    bits_len: usize,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_cff_hints(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    out: *mut *mut CffHints,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    match font.cff_hints(GlyphId(gid), unsafe { crate::ffi::at(location) }) {
        Some(hints) => unsafe { deliver(out, hints) },
        None => Status::Absent,
    }
}

field_view!(daegun_cff_hints_stems, CffHints, stems, CffStem);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cff_hints_mask_count(hints: *const CffHints, out: *mut usize) -> Status {
    let Some(hints) = (unsafe { borrow(hints) }) else { return Status::Null };
    unsafe { put(out, hints.masks.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cff_hints_mask_at(hints: *const CffHints, index: usize, out: *mut CffMaskC) -> Status {
    let Some(hints) = (unsafe { borrow(hints) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(mask) = hints.masks.get(index) else { return past_end("mask", index, hints.masks.len()) };
    unsafe { put(out, CffMaskC { point: mask.point, bits: mask.bits.as_ptr(), bits_len: mask.bits.len() }) }
}

free!(daegun_cff_hints_free, CffHints);

// prepared outlines

#[repr(C)]
#[derive(Clone, Copy)]
pub struct OutlineOptionsC {
    size: usize,
    pub(crate) hinting: i32,
    transform: [f32; 6],
    stroke: StrokeStyleC,
    embolden: f32,
    oblique: f32,
    has_transform: u8,
    has_stroke: u8,
    has_embolden: u8,
    has_oblique: u8,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(
    size_of::<OutlineOptionsC>() == 64
        && core::mem::offset_of!(OutlineOptionsC, stroke) == 36
        && core::mem::offset_of!(OutlineOptionsC, has_transform) == 60
);

// Every default is zero.
impl Options for OutlineOptionsC {
    unsafe fn set_defaults(_: *mut Self) {}
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_outline_options_default(out: *mut OutlineOptionsC, size: usize) -> Status {
    unsafe { defaults_into(out, size) }
}

pub(crate) fn outline_options(opts: &OutlineOptionsC) -> Result<OutlineOptions, Status> {
    let stroke = match opts.has_stroke != 0 {
        true => Some(opts.stroke.rust()?),
        false => None,
    };
    Ok(OutlineOptions {
        transform: (opts.has_transform != 0).then_some(opts.transform),
        hinting: hint_mode(opts.hinting)?,
        stroke,
        embolden: (opts.has_embolden != 0).then_some(opts.embolden),
        oblique: (opts.has_oblique != 0).then_some(opts.oblique),
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_prepared_outline(
    font: *const Font,
    gid: u32,
    location: *const LocationHandle,
    px: f32,
    opts: *const OutlineOptionsC,
    pen: *const Pen,
    out: *mut PreparedGlyph,
) -> Status {
    let (Some(font), Some(pen)) = (unsafe { (borrow(font), borrow(pen)) }) else { return Status::Null };
    let opts = or_return!(unsafe { options_of(opts) });
    let options = or_return!(outline_options(&opts));
    let at = unsafe { crate::ffi::at(location) };
    match font.prepared_outline(GlyphId(gid), at, px, &options, &mut PenBridge(*pen)) {
        Ok(glyph) => {
            unsafe { crate::ffi::handle::fill(out, glyph) };
            Status::Ok
        }
        Err(e) => failed(e),
    }
}

pub(crate) fn release_thread() {
    crate::ffi::handle::drop_spare::<Path>();
    crate::ffi::handle::drop_spare::<Contours>();
    crate::ffi::handle::drop_spare::<Quads>();
    crate::ffi::handle::drop_spare::<Hinted>();
    let _ = REPLAYED.try_with(|kept| kept.try_borrow_mut().map(|mut kept| core::mem::take(&mut *kept)));
}

#[cfg(test)]
#[path = "tests/outline.rs"]
mod tests;
