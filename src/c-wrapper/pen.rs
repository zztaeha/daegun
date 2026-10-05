use core::ffi::c_void;

use crate::OutlinePen;

// Reentrancy is safe, as the header promises: daegun holds no guard across a callback, the instanced
// path locking only to clone an `Arc` out of the cache. A null callback skips that event, and one that
// unwinds through these frames is undefined behavior nothing here can prevent.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Pen {
    pub move_to: Option<extern "C" fn(*mut c_void, f32, f32)>,
    pub line_to: Option<extern "C" fn(*mut c_void, f32, f32)>,
    pub quad_to: Option<extern "C" fn(*mut c_void, f32, f32, f32, f32)>,
    pub curve_to: Option<extern "C" fn(*mut c_void, f32, f32, f32, f32, f32, f32)>,
    pub close: Option<extern "C" fn(*mut c_void)>,
    pub user: *mut c_void,
}

const _: () = assert!(size_of::<Pen>() == 6 * size_of::<usize>());
const _: () = assert!(size_of::<Option<extern "C" fn(*mut c_void)>>() == size_of::<usize>());

pub struct PenBridge(pub Pen);

impl OutlinePen for PenBridge {
    fn move_to(&mut self, x: f32, y: f32) {
        if let Some(f) = self.0.move_to {
            f(self.0.user, x, y);
        }
    }

    fn line_to(&mut self, x: f32, y: f32) {
        if let Some(f) = self.0.line_to {
            f(self.0.user, x, y);
        }
    }

    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if let Some(f) = self.0.quad_to {
            f(self.0.user, cx, cy, x, y);
        }
    }

    fn curve_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        if let Some(f) = self.0.curve_to {
            f(self.0.user, c1x, c1y, c2x, c2y, x, y);
        }
    }

    fn close(&mut self) {
        if let Some(f) = self.0.close {
            f(self.0.user);
        }
    }
}
