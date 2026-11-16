use alloc::vec::Vec;

use super::ladder::{Ladder, Rung, Scale};
use super::outline::Outline;
use super::stems::{self, Mask, Order, Stem};
use super::zones::Zones;

// What hinting costs in the charstring's steps, which pay for it, each about the time a step takes: a hint set placed
// costs PLACED, a step for each LOOKED stems the list holds, all looked over, and TAKEN for each stem it takes, whose
// rungs are divided out; a mask read, a step for each READ stems.
const PLACED: usize = 2;
const LOOKED: usize = 8;
const TAKEN: usize = 2;
const READ: usize = 64;
// ±2^31 font units in 16.16, past any outline a charstring draws, so a point times any scale fits i128.
pub(super) const GLYPH_REACH: i64 = 1 << 47;

// A charstring value in 16.16 font units, exact for every value the interpreter makes; NaN as 0.
fn glyph(v: f64) -> i64 {
    (v * 65536.0).clamp(-GLYPH_REACH as f64, GLYPH_REACH as f64) as i64
}

// One charstring's hints, fed as it's drawn in glyph space: its stems and masks, then each point mapped through the
// hint set in force when it's drawn (TN 5177, hintmask), x by the scale alone.
pub(crate) struct Hinter {
    zones: Zones,
    scale: Scale,
    // A seac accent's stems raised by its offset, in 16.16 font units.
    raise: i64,
    stems: Vec<Stem>,
    // Every stem declared, horizontal and vertical, as a mask counts them.
    declared: usize,
    // The stem list is fixed from the first mask or path operator (TN 5177, 4.3).
    frozen: bool,
    order: Order,
    anchors: Ladder,
    ladder: Ladder,
    // A hintmask read since the last point, whether there is one, and the mask the ladder was placed for, whether
    // the ladder is a hint set's (not a counter mask's, or none yet).
    pending: Mask,
    is_pending: bool,
    current: Mask,
    is_current: bool,
    // What placing hint sets has cost since the charstring last paid, in its steps.
    work: u32,
    // The current point and where it landed; the open contour's start, and whether a moveto left one waiting.
    at: (i64, i64),
    landed: (i64, i64),
    start: (i64, i64),
    open: bool,
    moved: bool,
    // Whether the last point is a line's that returned to the start, which the close draws instead.
    back_at_start: bool,
    out: Outline,
}

impl Hinter {
    // `origin` is where drawing starts: a seac accent's offset, which also raises its stems.
    pub(crate) fn new(zones: Zones, origin: (f64, f64), out: Outline) -> Hinter {
        let Spare { mut stems, order, anchors, ladder, masks: [pending, current] } = Spare::take();
        stems.clear();
        let (scale, at) = (zones.scale(), (glyph(origin.0), glyph(origin.1)));
        Hinter {
            zones,
            scale,
            raise: at.1,
            stems,
            declared: 0,
            frozen: false,
            order,
            anchors: Ladder::new(anchors, scale),
            ladder: Ladder::new(ladder, scale),
            pending,
            is_pending: false,
            current,
            is_current: false,
            work: 0,
            at,
            landed: (0, 0),
            start: at,
            open: false,
            moved: false,
            back_at_start: false,
            out,
        }
    }

    // A stem operator's operands with any width taken out: pairs of an edge, from the last pair's second edge, and
    // a width. Each counts toward a mask's bits; horizontal ones declared before the list is fixed are kept.
    pub(crate) fn stems(&mut self, operands: &[f64], vertical: bool) {
        let mut edge = 0i64;
        for &[from_last, width] in operands.as_chunks::<2>().0 {
            let (low, width) = (edge + glyph(from_last), glyph(width));
            edge = low + width;
            if !vertical && !self.frozen {
                self.stems.push(Stem::declared(low + self.raise, width, self.declared as u32));
            }
            self.declared += 1;
        }
    }

    // A hintmask's or cntrmask's bytes, its implicit vstems already given to `stems`. A hint mask applies from the
    // next point drawn; a counter mask's stems are placed at once, so the first group is placed first (TN 5177, 2.1).
    pub(crate) fn mask(&mut self, bytes: &[u8], counter: bool) {
        self.freeze();
        self.charge(self.declared / READ);
        if counter {
            self.current.read(bytes, self.declared);
            self.place();
            self.is_current = false;
        } else {
            self.pending.read(bytes, self.declared);
            self.is_pending = true;
        }
    }

    // The work since the charstring last paid for it, in its steps.
    pub(crate) fn take_work(&mut self) -> u32 {
        core::mem::take(&mut self.work)
    }

    fn charge(&mut self, steps: usize) {
        self.work = self.work.saturating_add(u32::try_from(steps).unwrap_or(u32::MAX));
    }

    pub(crate) fn move_to(&mut self, x: f64, y: f64) {
        self.close();
        self.at = (glyph(x), glyph(y));
        self.ready();
        (self.landed, self.moved) = (self.map(self.at), true);
    }

    // A line landing where the last point did adds none.
    pub(crate) fn line_to(&mut self, x: f64, y: f64) {
        let to = (glyph(x), glyph(y));
        self.open();
        let landed = self.map(to);
        self.back_at_start = landed != self.landed && to == self.start;
        if landed != self.landed {
            self.out.push(landed, true);
        }
        (self.at, self.landed) = (to, landed);
    }

    pub(crate) fn curve_to(&mut self, c1: (f64, f64), c2: (f64, f64), to: (f64, f64)) {
        self.open();
        for (point, on_curve) in [(c1, false), (c2, false), (to, true)] {
            let point = (glyph(point.0), glyph(point.1));
            let landed = self.map(point);
            self.out.push(landed, on_curve);
            (self.at, self.landed) = (point, landed);
        }
        self.back_at_start = false;
    }

    // The outline once the last contour closes.
    pub(crate) fn finish(mut self) -> Outline {
        self.close();
        core::mem::take(&mut self.out)
    }

    fn close(&mut self) {
        if self.open {
            self.out.close(self.back_at_start);
        }
        (self.open, self.moved, self.back_at_start) = (false, false, false);
    }

    // A contour begun where the moveto left it, or where drawing stands when a charstring draws without one.
    fn open(&mut self) {
        self.ready();
        if self.open {
            return;
        }
        if !self.moved {
            self.landed = self.map(self.at);
        }
        self.start = self.at;
        self.out.push(self.landed, true);
        (self.open, self.moved) = (true, false);
    }

    fn map(&self, (x, y): (i64, i64)) -> (i64, i64) {
        (self.scale.of(x), self.ladder.map(y))
    }

    // The stem list fixed, its aligned stems placed, put in the order hint sets take them, and the anchors built.
    fn freeze(&mut self) {
        if self.frozen {
            return;
        }
        self.frozen = true;
        for stem in &mut self.stems {
            stem.place = self.zones.align(stem);
        }
        self.order.sort(&self.stems);
        stems::anchor(&self.stems, &mut self.order, self.zones.frame(), &mut self.anchors);
        self.charge(self.stems.len() * TAKEN);
    }

    // The hint set for the next point: a hintmask's where one came since, every stem where none has come.
    fn ready(&mut self) {
        self.freeze();
        if !self.is_pending {
            if self.is_current {
                return;
            }
            self.pending.fill(self.declared);
        }
        self.is_pending = false;
        if !self.is_current || self.pending != self.current {
            core::mem::swap(&mut self.pending, &mut self.current);
            self.place();
            self.is_current = true;
        }
    }

    // The current mask's hint set placed, charged by what it looked over and took.
    fn place(&mut self) {
        let taken = stems::place(&mut self.stems, &self.current, &self.zones, &self.anchors, &mut self.ladder,
            &mut self.order);
        self.charge(PLACED + self.stems.len() / LOOKED + taken * TAKEN);
    }
}

impl Drop for Hinter {
    fn drop(&mut self) {
        use core::mem::take;
        let (anchors, ladder) = (self.anchors.take_rungs(), self.ladder.take_rungs());
        let masks = [take(&mut self.pending), take(&mut self.current)];
        Spare { stems: take(&mut self.stems), order: take(&mut self.order), anchors, ladder, masks }.give();
    }
}

// The stems, rungs and masks a thread's last charstring grew, lent to its next so a glyph allocates none of them.
#[derive(Default)]
struct Spare {
    stems: Vec<Stem>,
    order: Order,
    anchors: Vec<Rung>,
    ladder: Vec<Rung>,
    masks: [Mask; 2],
}

#[cfg(feature = "std")]
std::thread_local! {
    static SPARE: core::cell::Cell<Option<Spare>> = const { core::cell::Cell::new(None) };
}

#[cfg(feature = "std")]
pub(crate) fn release_thread() {
    let _ = SPARE.try_with(core::cell::Cell::take);
}

impl Spare {
    fn take() -> Spare {
        #[cfg(feature = "std")]
        return SPARE.try_with(core::cell::Cell::take).ok().flatten().unwrap_or_default();
        #[cfg(not(feature = "std"))]
        Spare::default()
    }

    // A thread past its locals' teardown drops the buffers instead.
    fn give(self) {
        #[cfg(feature = "std")]
        let _ = SPARE.try_with(|spare| spare.set(Some(self)));
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/cff/hinter.rs"]
mod tests;
