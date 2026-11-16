/// Whether a measurement runs along a horizontal line of text or a vertical one.
#[expect(clippy::exhaustive_enums, reason = "a line runs across or down")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub enum Orientation {
    /// Left to right or right to left: advances are widths.
    #[default]
    Horizontal,
    /// Top to bottom: advances are heights.
    Vertical,
}

impl Orientation {
    pub(crate) fn is_vertical(self) -> bool {
        self == Orientation::Vertical
    }
}

/// A box in font units, y up: from its lower left corner (`x_min`, `y_min`) to its upper right (`x_max`, `y_max`).
#[expect(clippy::exhaustive_structs, reason = "a plain value: its fields are all it is")]
#[derive(Clone, Copy, PartialEq, Default, Debug)]
#[repr(C)]
pub struct Bounds {
    /// The left edge.
    pub x_min: f64,
    /// The bottom edge.
    pub y_min: f64,
    /// The right edge.
    pub x_max: f64,
    /// The top edge.
    pub y_max: f64,
}

impl Bounds {
    // A box the font holds as integers, (x_min, y_min, x_max, y_max).
    pub(crate) fn of([x_min, y_min, x_max, y_max]: [i32; 4]) -> Bounds {
        Bounds { x_min: x_min.into(), y_min: y_min.into(), x_max: x_max.into(), y_max: y_max.into() }
    }

    pub(crate) fn corners(self) -> [f64; 4] {
        [self.x_min, self.y_min, self.x_max, self.y_max]
    }
}
