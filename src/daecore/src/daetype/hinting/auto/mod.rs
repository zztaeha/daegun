mod blues;
mod latin;
pub mod points;

use alloc::vec::Vec;

pub use blues::BlueZones;
pub use points::{AutoPoints, CollectPen, CONIC, CUBIC, ON_CURVE};

use super::{state::{FLAG_CUBIC, FLAG_ON_CURVE}, HintedOutline};

pub struct AutoHinter {
    blues: BlueZones,
    upm: u16,
    scratch: latin::Scratch,
}

impl AutoHinter {
    pub fn new(
        upm: u16,
        resolve: &mut dyn FnMut(char) -> Option<u16>,
        outline_of: &mut dyn FnMut(u16) -> Option<AutoPoints>,
    ) -> Option<AutoHinter> {
        if upm == 0 {
            return None;
        }
        let blues = blues::compute(upm, resolve, outline_of);
        if blues.zones.len() < 2 {
            return None;
        }
        Some(AutoHinter { blues, upm, scratch: latin::Scratch::default() })
    }

    pub fn from_zones(blues: BlueZones, upm: u16) -> Option<AutoHinter> {
        (upm != 0 && blues.zones.len() >= 2).then_some(AutoHinter { blues, upm, scratch: latin::Scratch::default() })
    }

    pub fn compute_zones(
        upm: u16,
        resolve: &mut dyn FnMut(char) -> Option<u16>,
        outline_of: &mut dyn FnMut(u16) -> Option<AutoPoints>,
    ) -> BlueZones {
        if upm == 0 { return BlueZones::default() }
        blues::compute(upm, resolve, outline_of)
    }

    pub fn hint(&mut self, pts: &AutoPoints, size: super::f26dot6::Size) -> HintedOutline {
        let y = latin::fit(pts, &self.blues, size, self.upm, &mut self.scratch).to_vec();
        let x = pts
            .x
            .iter()
            .map(|&v| super::f26dot6::scale_f32(v, size, self.upm))
            .collect::<Vec<i32>>();
        let flags = pts.flags.iter().map(|&f| hinted_flag(f)).collect();
        HintedOutline { x, y, flags, contour_ends: pts.contour_ends.clone() }
    }
}

// A collected point's flag as a hinted outline carries it: on-curve, or a cubic control, which the
// outline draws as one rather than as a quadratic's.
pub(crate) fn hinted_flag(f: u8) -> u8 {
    if f & ON_CURVE != 0 {
        FLAG_ON_CURVE
    } else if f & CUBIC != 0 {
        FLAG_CUBIC
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::blues::BlueZone;

    // A variable instance's fractional coordinates scale as they are: 0.9 and -0.9 units of 64 at
    // 64 px are 58/64 px either side of zero, where truncating first would make both 0.
    #[test]
    fn a_fractional_coordinate_is_scaled_as_it_is() {
        let zone = |reference, overshoot, is_top| BlueZone { reference, overshoot, is_top };
        let blues = BlueZones { zones: alloc::vec![zone(50.0, 51.0, true), zone(0.0, -1.0, false)] };
        let mut hinter = AutoHinter::from_zones(blues, 64).expect("two zones");
        let pts = AutoPoints { x: alloc::vec![0.9, -0.9], y: alloc::vec![0.9, -0.9], flags: alloc::vec![ON_CURVE; 2], contour_ends: alloc::vec![0, 1] };
        let out = hinter.hint(&pts, super::super::f26dot6::Size::from_px(64.0).expect("a size"));
        assert_eq!((out.x, out.y), (alloc::vec![58, -58], alloc::vec![58, -58]));
    }
}
