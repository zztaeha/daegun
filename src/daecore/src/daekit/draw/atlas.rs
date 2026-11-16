use alloc::vec::Vec;

/// Where a glyph landed in an atlas, in pixels.
#[expect(clippy::exhaustive_structs, reason = "a plain value: its fields are all it is")]
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Rect {
    /// Its left edge.
    pub x: usize,
    /// Its top edge.
    pub y: usize,
    /// Its width.
    pub w: usize,
    /// Its height.
    pub h: usize,
}

/// A shelf packer for an atlas of one size: glyphs go left to right on shelves, each as tall as the glyph that
/// opened it, a glyph on the shortest shelf it fits.
#[derive(Clone, Debug)]
pub struct ShelfPacker {
    width: usize,
    height: usize,
    shelves: Vec<(usize, usize, usize)>,
    used_height: usize,
}

impl ShelfPacker {
    /// An empty atlas of this size, in pixels.
    #[must_use]
    pub const fn new(width: usize, height: usize) -> ShelfPacker {
        ShelfPacker { width, height, shelves: Vec::new(), used_height: 0 }
    }

    /// Room for a glyph this size; None for one empty either way or larger than the atlas, and where it's full.
    pub fn insert(&mut self, w: usize, h: usize) -> Option<Rect> {
        if w == 0 || h == 0 || w > self.width || h > self.height { return None; }

        let mut best: Option<usize> = None;
        for (i, &(_, sh, used)) in self.shelves.iter().enumerate() {
            if sh >= h && used.checked_add(w).is_some_and(|end| end <= self.width) {
                let better = best.is_none_or(|b| sh < self.shelves[b].1);
                if better { best = Some(i); }
            }
        }
        if let Some(i) = best {
            let (y, _, used) = self.shelves[i];
            self.shelves[i].2 = used + w;
            return Some(Rect { x: used, y, w, h });
        }

        if self.used_height.checked_add(h).is_none_or(|end| end > self.height) { return None; }
        let y = self.used_height;
        self.used_height += h;
        self.shelves.push((y, h, w));
        Some(Rect { x: 0, y, w, h })
    }

    /// Empties the atlas, keeping its size.
    pub fn reset(&mut self) {
        self.shelves.clear();
        self.used_height = 0;
    }
}
