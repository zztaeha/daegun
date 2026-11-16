mod blues;
mod latin;
pub mod points;

pub use blues::BlueZones;
pub use points::{AutoPoints, CollectPen, CUBIC, ON_CURVE};
#[cfg(test)]
pub(crate) use points::CONIC;

use super::{state::{FLAG_CUBIC, FLAG_ON_CURVE}, HintedOutline};

pub struct AutoHinter {
    blues: BlueZones,
    upm: u16,
    // The size the zones were last fitted at, held in what was the struct's padding: a location carries a hinter.
    fitted_at: Option<core::num::NonZeroI32>,
    scratch: latin::Scratch,
    // The last glyph hinted, lent until the next.
    out: HintedOutline,
}

impl AutoHinter {
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        let AutoHinter { blues, upm: _, fitted_at: _, scratch, out } = self;
        let HintedOutline { x, y, flags, contour_ends } = out;
        heap_of(&blues.zones) + scratch.heap_bytes() + heap_of(x) + heap_of(y) + heap_of(flags) + heap_of(contour_ends)
    }

    #[cfg(test)]
    pub fn new(
        upm: u16,
        resolve: &mut dyn FnMut(char) -> Option<u32>,
        outline_of: &mut dyn FnMut(u32) -> Option<AutoPoints>,
    ) -> Option<AutoHinter> {
        if upm == 0 {
            return None;
        }
        let blues = blues::compute(upm, resolve, outline_of);
        if blues.zones.len() < 2 {
            return None;
        }
        let (scratch, out) = (latin::Scratch::default(), HintedOutline::default());
        Some(AutoHinter { blues, upm, fitted_at: None, scratch, out })
    }

    pub fn from_zones(blues: BlueZones, upm: u16) -> Option<AutoHinter> {
        let (scratch, out) = (latin::Scratch::default(), HintedOutline::default());
        (upm != 0 && blues.zones.len() >= 2).then_some(AutoHinter { blues, upm, fitted_at: None, scratch, out })
    }

    // One with the same zones and none of the working room, for a thread hinting on its own.
    #[cfg(feature = "threading")]
    pub fn fresh(&self) -> AutoHinter {
        let (scratch, out) = (latin::Scratch::default(), HintedOutline::default());
        AutoHinter { blues: self.blues.clone(), upm: self.upm, fitted_at: self.fitted_at, scratch, out }
    }

    pub fn compute_zones(
        upm: u16,
        resolve: &mut dyn FnMut(char) -> Option<u32>,
        outline_of: &mut dyn FnMut(u32) -> Option<AutoPoints>,
    ) -> BlueZones {
        if upm == 0 { return BlueZones::default() }
        blues::compute(upm, resolve, outline_of)
    }

    pub fn hint(&mut self, pts: &AutoPoints, size: super::f26dot6::Size) -> &mut HintedOutline {
        let HintedOutline { x, y, flags, contour_ends } = &mut self.out;
        latin::fit(pts, &mut self.blues, &mut self.fitted_at, size, self.upm, &mut self.scratch, y);
        x.clear();
        super::f26dot6::scale_f32_into(&pts.x, size, self.upm, x);
        flags.clear();
        flags.extend(pts.flags.iter().map(|&f| hinted_flag(f)));
        contour_ends.clone_from(&pts.contour_ends);
        &mut self.out
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
#[path = "../../../../tests/unit/daetype/hinting/auto/mod.rs"]
mod tests;
