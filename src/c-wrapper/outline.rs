// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use core::ffi::c_void;

use crate::daecore::daetype::outline::stroke::Stroked;

use crate::ffi::handle::{Status, borrow, release};
use crate::ffi::pen::{Pen, PenBridge};
use crate::ffi::raster::HintedOutline;

pub struct Path(pub(crate) crate::Path);

fn too_many_points() -> Status {
    crate::ffi::set_error("the stroke would take more than DAEGUN_MAX_FLATTEN_POINTS points");
    Status::Range
}

fn stroked(path: &Path, style: &StrokeStyleC, tolerance: f32) -> Result<Stroked, Status> {
    let style = style.to_rust();
    let s = Stroked::new(&path.0, &style, tolerance).ok_or_else(too_many_points)?;
    if !style.width.is_finite() || !s.finite() {
        return Err(crate::ffi::range("a stroke width or point is not finite"));
    }
    Ok(s)
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_path_new() -> *mut Path {
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(Path(crate::Path::default())))
}

macro_rules! path_verb {
    ($name:ident, $method:ident $(, $arg:ident)*) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(path: *mut Path $(, $arg: f32)*) -> Status {
            if path.is_null() {
                return Status::Null;
            }
            let path = unsafe { &mut *path };
            crate::OutlinePen::$method(&mut path.0 $(, $arg)*);
            Status::Ok
        }
    };
}

path_verb!(daegun_path_move_to, move_to, x, y);
path_verb!(daegun_path_line_to, line_to, x, y);
path_verb!(daegun_path_quad_to, quad_to, cx, cy, x, y);
path_verb!(daegun_path_curve_to, curve_to, c1x, c1y, c2x, c2y, x, y);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_close(path: *mut Path) -> Status {
    if path.is_null() {
        return Status::Null;
    }
    let path = unsafe { &mut *path };
    crate::OutlinePen::close(&mut path.0);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_is_empty(path: *const Path, out: *mut i32) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = i32::from(path.0.is_empty()) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_cost(path: *const Path, out: *mut usize) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = path.0.cost() };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_bounds(
    path: *const Path,
    out_min_x: *mut f64,
    out_min_y: *mut f64,
    out_max_x: *mut f64,
    out_max_y: *mut f64,
) -> Status {
    if out_min_x.is_null() || out_min_y.is_null() || out_max_x.is_null() || out_max_y.is_null() {
        return Status::Null;
    }
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    let Some((min_x, min_y, max_x, max_y)) = path.0.bounds() else { return Status::Absent };
    unsafe {
        *out_min_x = min_x;
        *out_min_y = min_y;
        *out_max_x = max_x;
        *out_max_y = max_y;
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_points(
    path: *const Path,
    out_x: *mut f32,
    out_y: *mut f32,
    capacity: usize,
    out_count: *mut usize,
) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    let (_, points) = path.0.parts();
    unsafe { *out_count = points.len() };
    if capacity == 0 {
        return Status::Ok;
    }
    let n = capacity.min(points.len());
    if !out_x.is_null() {
        let dst = unsafe { core::slice::from_raw_parts_mut(out_x, n) };
        for (slot, p) in dst.iter_mut().zip(points) {
            *slot = p.0;
        }
    }
    if !out_y.is_null() {
        let dst = unsafe { core::slice::from_raw_parts_mut(out_y, n) };
        for (slot, p) in dst.iter_mut().zip(points) {
            *slot = p.1;
        }
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_verbs(
    path: *const Path,
    out: *mut u8,
    capacity: usize,
    out_count: *mut usize,
) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    if capacity != 0 && out.is_null() {
        return Status::Null;
    }
    let (verbs, _) = path.0.parts();
    unsafe { *out_count = verbs.len() };
    if capacity == 0 {
        return Status::Ok;
    }
    let n = capacity.min(verbs.len());
    let dst = unsafe { core::slice::from_raw_parts_mut(out, n) };
    for (slot, verb) in dst.iter_mut().zip(verbs) {
        *slot = *verb as u8;
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_replay(
    path: *const Path,
    transform: *const f64,
    pen: *const Pen,
) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    let Some(pen) = (unsafe { borrow(pen) }) else { return Status::Null };
    // Replayed from a copy: any pen callback may add to this very path, through daegun_path_as_pen or
    // its own pointer to it, and growing it would move what is being read.
    let path = path.0.clone();
    let mut bridge = PenBridge(*pen);
    if transform.is_null() {
        path.replay(None, &mut bridge);
    } else {
        let m = unsafe { core::slice::from_raw_parts(transform, 6) };
        let m: [f64; 6] = [m[0], m[1], m[2], m[3], m[4], m[5]];
        path.replay(Some(&m), &mut bridge);
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_free(path: *mut Path) {
    unsafe { release(path) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StrokeStyleC {
    pub width: f32,
    pub cap: i32,
    pub join: i32,
    pub miter_limit: f32,
}

const _: () = assert!(size_of::<StrokeStyleC>() == 16);
const _: () = assert!(align_of::<StrokeStyleC>() == 4);

impl StrokeStyleC {
    fn to_rust(self) -> crate::StrokeStyle {
        crate::ffi::options::stroke_of(self.width, self.join, self.miter_limit, self.cap)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_stroke(
    path: *const Path,
    style: *const StrokeStyleC,
    tolerance: f32,
    pen: *const Pen,
) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    let Some(style) = (unsafe { borrow(style) }) else { return Status::Null };
    let Some(pen) = (unsafe { borrow(pen) }) else { return Status::Null };
    let stroked = match stroked(path, style, tolerance) {
        Ok(s) => s,
        Err(status) => return status,
    };
    stroked.draw(&mut PenBridge(*pen));
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_stroke_simplified(
    path: *const Path,
    style: *const StrokeStyleC,
    tolerance: f32,
    pen: *const Pen,
) -> Status {
    let Some(path) = (unsafe { borrow(path) }) else { return Status::Null };
    let Some(style) = (unsafe { borrow(style) }) else { return Status::Null };
    let Some(pen) = (unsafe { borrow(pen) }) else { return Status::Null };
    let stroked = match stroked(path, style, tolerance) {
        Ok(s) => s,
        Err(status) => return status,
    };
    stroked.draw_simplified(&mut PenBridge(*pen));
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_hinted_outline_draw(
    outline: *const HintedOutline,
    pen: *const Pen,
) -> Status {
    let Some(outline) = (unsafe { borrow(outline) }) else { return Status::Null };
    let Some(pen) = (unsafe { borrow(pen) }) else { return Status::Null };
    let mut bridge = PenBridge(*pen);
    crate::draw_hinted(outline.inner(), &mut bridge);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_path_as_pen(path: *mut Path, out: *mut Pen) -> Status {
    if path.is_null() || out.is_null() {
        return Status::Null;
    }
    unsafe {
        *out = Pen {
            move_to: Some(path_move_to),
            line_to: Some(path_line_to),
            quad_to: Some(path_quad_to),
            curve_to: Some(path_curve_to),
            close: Some(path_close),
            user: path.cast::<c_void>(),
        }
    };
    Status::Ok
}

unsafe fn pen_path<'a>(user: *mut c_void) -> Option<&'a mut Path> {
    if user.is_null() {
        return None;
    }
    Some(unsafe { &mut *user.cast::<Path>() })
}

extern "C" fn path_move_to(user: *mut c_void, x: f32, y: f32) {
    if let Some(p) = unsafe { pen_path(user) } {
        crate::OutlinePen::move_to(&mut p.0, x, y);
    }
}

extern "C" fn path_line_to(user: *mut c_void, x: f32, y: f32) {
    if let Some(p) = unsafe { pen_path(user) } {
        crate::OutlinePen::line_to(&mut p.0, x, y);
    }
}

extern "C" fn path_quad_to(user: *mut c_void, cx: f32, cy: f32, x: f32, y: f32) {
    if let Some(p) = unsafe { pen_path(user) } {
        crate::OutlinePen::quad_to(&mut p.0, cx, cy, x, y);
    }
}

extern "C" fn path_curve_to(
    user: *mut c_void,
    c1x: f32,
    c1y: f32,
    c2x: f32,
    c2y: f32,
    x: f32,
    y: f32,
) {
    if let Some(p) = unsafe { pen_path(user) } {
        crate::OutlinePen::curve_to(&mut p.0, c1x, c1y, c2x, c2y, x, y);
    }
}

extern "C" fn path_close(user: *mut c_void) {
    if let Some(p) = unsafe { pen_path(user) } {
        crate::OutlinePen::close(&mut p.0);
    }
}
