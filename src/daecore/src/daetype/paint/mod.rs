use alloc::vec::Vec;
use crate::daecore::daetype::outline::{FillRule, Path};

pub mod colr;

pub use crate::daecore::daemachine::daemath::{
    resolve_stops, Blend, Extend, Gradient, GradientKind, Rgba, Stop, Stops,
};

#[derive(Clone, PartialEq, Debug)]
pub enum Paint {
    Solid(Rgba),
    Gradient(Gradient),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ClipShape {
    pub path: PathId,
    pub rule: FillRule,
    pub transform: [f64; 6],
}

pub type PathId = usize;

#[derive(Clone, PartialEq, Debug)]
pub enum Op {
    Fill { path: PathId, paint: Paint, rule: FillRule, transform: [f64; 6] },
    PushClip { shapes: Vec<ClipShape> },
    PopClip,
    PushLayer { opacity: f32, blend: Blend },
    PopLayer,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct DisplayList {
    paths: Vec<Path>,
    ops: Vec<Op>,
}

impl DisplayList {
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    pub fn path(&self, id: PathId) -> Option<&Path> {
        self.paths.get(id)
    }

    pub fn push_path(&mut self, path: Path) -> PathId {
        self.paths.push(path);
        self.paths.len() - 1
    }

    pub fn push(&mut self, op: Op) {
        self.ops.push(op);
    }

    pub(crate) fn path_mut(&mut self, id: PathId) -> Option<&mut Path> {
        self.paths.get_mut(id)
    }

    pub(crate) fn truncate(&mut self, to: usize) {
        self.ops.truncate(to);
    }

    pub(crate) fn remove(&mut self, at: usize) {
        self.ops.remove(at);
    }
}
