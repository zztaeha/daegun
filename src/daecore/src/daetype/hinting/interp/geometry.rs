// The instructions that measure and move points, and the vectors they measure and move along.
use super::super::f26dot6::{self, F2DOT14_ONE};
use super::super::state::{Vector, FLAG_ON_CURVE, FLAG_TOUCHED_X, FLAG_TOUCHED_Y};
use super::{Exec, Motion, Reach, Step, Stop, ALIGNED, DELTA, ELEMENT, INTERPOLATED, IUP_POINT, MOVED};

// A distance along a unit vector. A component of exactly 1.0 reads that axis alone: such a vector lies within rounding
// of the axis, and the residue in its other component, as in (1.0, -34), would put points a 64th off.
fn along(v: Vector, dx: i64, dy: i64) -> i32 {
    if v.x == F2DOT14_ONE {
        f26dot6::clamp_i32(dx)
    } else if v.y == F2DOT14_ONE {
        f26dot6::clamp_i32(dy)
    } else {
        f26dot6::dot_2_14(dx, dy, v.x, v.y)
    }
}

// A distance kept at least `minimum` long on the side its original measure lies.
fn at_least(distance: i32, minimum: i32, original: i32) -> i32 {
    if original >= 0 { distance.max(minimum) } else { distance.min(minimum.saturating_neg()) }
}

// The single width in place of a distance whose length is within its cut-in of it, the distance's sign kept.
fn single_width(distance: i32, width: i32, cut_in: i32) -> i32 {
    let length = i64::from(distance).abs();
    if cut_in > 0 && (length - i64::from(width)).abs() < i64::from(cut_in) {
        if distance >= 0 { width } else { width.saturating_neg() }
    } else {
        distance
    }
}

// ISECT's crossing of lines a and b, in 128 bits and rounded once. Lines under 1/16 radian apart, where a 64th's error
// moves it past a quarter pixel, count as parallel: the point goes to the middle of the four, as the spec says.
fn crossing(a0: (i64, i64), a1: (i64, i64), b0: (i64, i64), b1: (i64, i64)) -> (i32, i32) {
    let (ax, ay, bx, by) = (a1.0 - a0.0, a1.1 - a0.1, b1.0 - b0.0, b1.1 - b0.1);
    let cross = i128::from(ax) * i128::from(by) - i128::from(ay) * i128::from(bx);
    let squared = |x: i64, y: i64| (x as f64) * (x as f64) + (y as f64) * (y as f64);
    let lengths = squared(ax, ay) * squared(bx, by);
    if cross == 0 || 256.0 * (cross as f64) * (cross as f64) < lengths {
        let middle = |a: i64, b: i64, c: i64, d: i64| f26dot6::clamp_i32((a + b + c + d) / 4);
        return (middle(a0.0, a1.0, b0.0, b1.0), middle(a0.1, a1.1, b0.1, b1.1));
    }
    let reach = i128::from(b0.0 - a0.0) * i128::from(by) - i128::from(b0.1 - a0.1) * i128::from(bx);
    let step = |start: i64, d: i64| {
        let offset = i128::from(d) * reach;
        let half = cross.abs() / 2;
        let rounded = if (offset < 0) == (cross < 0) { (offset + half * cross.signum()) / cross } else {
            (offset - half * cross.signum()) / cross
        };
        f26dot6::clamp_i32((i128::from(start) + rounded).clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
    };
    (step(a0.0, ax), step(a0.1, ay))
}

impl Exec<'_> {
    // The freedom vector over f·p: a move of one along the projection, in x and y. From 0x3FFE f·p counts as 1 (equal
    // vectors round up to 2 apart in 2.14); under 1/16, where a pixel of rounding moves a point 16, nothing moves.
    pub(super) fn refresh_motion(&mut self) {
        let (p, f) = (self.gs.projection, self.gs.freedom);
        let dot = (i64::from(p.x) * i64::from(f.x) + i64::from(p.y) * i64::from(f.y) + 0x2000) >> 14;
        let unit = dot >= 0x3FFE;
        let (x, y) = if unit {
            (f.x * 4, f.y * 4)
        } else if dot.abs() < 0x400 {
            (0, 0)
        } else {
            ((i64::from(f.x) * 0x10000 / dot) as i32, (i64::from(f.y) * 0x10000 / dot) as i32)
        };
        // A freedom vector at exactly 1.0 on an axis meeting the projection head on moves along that axis alone.
        let reach = Reach { x: x != 0 && !(unit && f.y == F2DOT14_ONE), y: y != 0 && !(unit && f.x == F2DOT14_ONE) };
        self.motion = Motion { x, y, reach };
    }

    // Point `p` of zone `z`, where the zone has it.
    fn at(&self, z: usize, p: u32) -> Option<usize> {
        let p = p as usize;
        (p < self.zones[z].len()).then_some(p)
    }

    pub(super) fn pop_point(&mut self) -> u32 {
        point_number(self.pop())
    }

    fn projected(&self, z: usize, p: usize) -> i32 {
        let c = &self.zones[z].current;
        along(self.gs.projection, c.x[p].into(), c.y[p].into())
    }

    // p's current position less q's, along the projection.
    fn current_distance(&self, zp: usize, p: usize, zq: usize, q: usize) -> i32 {
        let (a, b) = (&self.zones[zp].current, &self.zones[zq].current);
        along(self.gs.projection, i64::from(a.x[p]) - i64::from(b.x[q]), i64::from(a.y[p]) - i64::from(b.y[q]))
    }

    // p's original position less q's, along the dual projection.
    fn original_distance(&self, zp: usize, p: usize, zq: usize, q: usize) -> i32 {
        let (a, b) = (&self.zones[zp].original, &self.zones[zq].original);
        along(self.gs.dual, i64::from(a.x[p]) - i64::from(b.x[q]), i64::from(a.y[p]) - i64::from(b.y[q]))
    }

    // p's position in the font's own points less q's, along the dual projection, unscaled.
    fn unscaled_distance(&self, zp: usize, p: usize, zq: usize, q: usize) -> i32 {
        let (a, b) = (&self.zones[zp].unscaled, &self.zones[zq].unscaled);
        along(self.gs.dual, i64::from(a.x[p]) - i64::from(b.x[q]), i64::from(a.y[p]) - i64::from(b.y[q]))
    }

    // The original distance from the font's own points, scaled once, so it carries one rounding rather than two;
    // the twilight zone, which has no such points, measures its originals.
    fn design_distance(&self, zp: usize, p: usize, zq: usize, q: usize) -> i32 {
        if zp == 0 || zq == 0 {
            return self.original_distance(zp, p, zq, q);
        }
        f26dot6::scale_units(self.unscaled_distance(zp, p, zq, q), self.unscaled)
    }

    // Shifts a point by (dx, dy) on the axes `reach` names, marking it touched there with `touch`. `ClearType` decides
    // which axes it actually moves on; every instruction's move goes through here.
    fn shift(&mut self, z: usize, p: usize, (dx, dy): (i32, i32), reach: Reach, touch: bool) {
        let (moves_x, moves_y) = (self.rules.moves_x(), self.rules.moves_y());
        let zone = &mut *self.zones[z];
        if reach.x {
            if moves_x {
                zone.current.x[p] = zone.current.x[p].saturating_add(dx);
            }
            if touch {
                zone.flags[p] |= FLAG_TOUCHED_X;
            }
        }
        if reach.y {
            if moves_y {
                zone.current.y[p] = zone.current.y[p].saturating_add(dy);
            }
            if touch {
                zone.flags[p] |= FLAG_TOUCHED_Y;
            }
        }
    }

    // Moves a point along the freedom vector so its projection changes by `distance`.
    fn move_point(&mut self, z: usize, p: usize, distance: i32) {
        let m = self.motion;
        self.shift(z, p, (f26dot6::times_16_16(distance, m.x), f26dot6::times_16_16(distance, m.y)), m.reach, true);
    }

    // The freedom vector's axes, which a shift by a whole displacement reaches.
    fn freedom_reach(&self) -> Reach {
        Reach { x: self.gs.freedom.x != 0, y: self.gs.freedom.y != 0 }
    }

    // Places a twilight point's original and current position.
    fn place_twilight(&mut self, p: usize, x: i32, y: i32) {
        let zone = &mut *self.zones[0];
        (zone.original.x[p], zone.original.y[p]) = (x, y);
        (zone.current.x[p], zone.current.y[p]) = (x, y);
    }

    // The loop count's points, top first, each read in place and then all popped, at `price` a point; too few on the
    // stack underflows.
    fn each_point(&mut self, price: u64, mut f: impl FnMut(&mut Self, u32)) -> Step {
        let n = core::mem::replace(&mut self.gs.repeat, 1) as usize;
        let depth = self.stack.len();
        let first = depth.checked_sub(n).ok_or(Stop::StackUnderflow)?;
        self.charge(n, price)?;
        for k in (first..depth).rev() {
            let p = point_number(self.stack[k]);
            f(self, p);
        }
        self.stack.truncate(first);
        Ok(())
    }

    // SPVTL and SFVTL: the line from p1 (the top, in zp2) toward p2 (in zp1), or its perpendicular.
    pub(super) fn vector_to_line(&mut self, byte: u8) {
        let (p1, p2) = (self.pop_point(), self.pop_point());
        let [_, z1, z2] = self.gs.zp;
        let (Some(p1), Some(p2)) = (self.at(z2, p1), self.at(z1, p2)) else { return };
        let (a, b) = (&self.zones[z2].current, &self.zones[z1].current);
        let (dx, dy) = (i64::from(b.x[p2]) - i64::from(a.x[p1]), i64::from(b.y[p2]) - i64::from(a.y[p1]));
        let v = Vector::toward(dx, dy, byte & 1 != 0);
        if byte <= 0x07 {
            (self.gs.projection, self.gs.dual) = (v, v);
        } else {
            self.gs.freedom = v;
        }
        self.refresh_motion();
    }

    // SDPVTL: as SPVTL, the dual projection taken from the points' original positions.
    pub(super) fn dual_to_line(&mut self, byte: u8) {
        let (p1, p2) = (self.pop_point(), self.pop_point());
        let [_, z1, z2] = self.gs.zp;
        let (Some(p1), Some(p2)) = (self.at(z2, p1), self.at(z1, p2)) else { return };
        let turned = byte & 1 != 0;
        let line = |a: &super::super::state::Points, b: &super::super::state::Points| {
            Vector::toward(i64::from(b.x[p2]) - i64::from(a.x[p1]), i64::from(b.y[p2]) - i64::from(a.y[p1]), turned)
        };
        self.gs.dual = line(&self.zones[z2].original, &self.zones[z1].original);
        self.gs.projection = line(&self.zones[z2].current, &self.zones[z1].current);
        self.refresh_motion();
    }

    // SPVFS and SFVFS: x and y as 2.14 in each value's low 16 bits, made a unit vector.
    pub(super) fn vector_from_stack(&mut self, byte: u8) {
        let (y, x) = (self.pop() as i16, self.pop() as i16);
        let v = Vector::toward(x.into(), y.into(), false);
        if byte == 0x0A {
            (self.gs.projection, self.gs.dual) = (v, v);
        } else {
            self.gs.freedom = v;
        }
        self.refresh_motion();
    }

    // ISECT: point p (zp2) to where line A (zp1) crosses line B (zp0); it moves on both axes as the rules allow.
    pub(super) fn intersect(&mut self) {
        let [b1, b0, a1, a0, p] = [(); 5].map(|()| self.pop_point());
        let [z0, z1, z2] = self.gs.zp;
        let points = (self.at(z0, b1), self.at(z0, b0), self.at(z1, a1), self.at(z1, a0), self.at(z2, p));
        let (Some(b1), Some(b0), Some(a1), Some(a0), Some(p)) = points else { return };
        let point = |z: usize, i: usize| {
            let c = &self.zones[z].current;
            (i64::from(c.x[i]), i64::from(c.y[i]))
        };
        let (x, y) = crossing(point(z1, a0), point(z1, a1), point(z0, b0), point(z0, b1));
        let c = &self.zones[z2].current;
        let d = (x.saturating_sub(c.x[p]), y.saturating_sub(c.y[p]));
        self.shift(z2, p, d, Reach { x: true, y: true }, true);
    }

    // MDAP: rp0 and rp1 to p (zp0), touched where it is or rounded there.
    pub(super) fn move_direct_absolute(&mut self, byte: u8) {
        let p = self.pop_point();
        let z = self.gs.zp[0];
        if let Some(i) = self.at(z, p) {
            let distance = if byte & 1 != 0 {
                let now = self.projected(z, i);
                self.gs.round.apply(now).saturating_sub(now)
            } else {
                0
            };
            self.move_point(z, i, distance);
        }
        self.gs.rp[0] = p;
        self.gs.rp[1] = p;
    }

    // MIAP: p (zp0) to the CVT entry's position along the projection, a twilight point first placed there along the
    // freedom vector; rounded, past the cut-in its own position is.
    pub(super) fn move_indirect_absolute(&mut self, byte: u8) {
        let (entry, p) = (self.pop(), self.pop_point());
        let z = self.gs.zp[0];
        if let (Some(i), Some(&target)) = (self.at(z, p), self.vm.cvt.get(super::index(entry))) {
            if z == 0 {
                let f = self.gs.freedom;
                self.place_twilight(i, f26dot6::times_2_14(target, f.x), f26dot6::times_2_14(target, f.y));
            }
            let now = self.projected(z, i);
            let mut target = target;
            if byte & 1 != 0 {
                if (i64::from(target) - i64::from(now)).abs() > i64::from(self.gs.cut_in) {
                    target = now;
                }
                target = self.gs.round.apply(target);
            }
            self.move_point(z, i, target.saturating_sub(now));
        }
        self.gs.rp[0] = p;
        self.gs.rp[1] = p;
    }

    // The reference points after a relative move to p: rp1 takes rp0, rp2 takes p, and rp0 p with the flag set.
    fn moved_relative(&mut self, p: u32, set_rp0: bool) {
        self.gs.rp[1] = self.gs.rp[0];
        self.gs.rp[2] = p;
        if set_rp0 {
            self.gs.rp[0] = p;
        }
    }

    // MDRP: p (zp1) at its original distance from rp0 (zp0), single width, rounding and minimum distance as the
    // opcode's bits ask.
    pub(super) fn move_direct_relative(&mut self, byte: u8) {
        let p = self.pop_point();
        let [z0, z1, _] = self.gs.zp;
        if let (Some(i), Some(r)) = (self.at(z1, p), self.at(z0, self.gs.rp[0])) {
            let gs = &self.gs;
            let original = single_width(self.design_distance(z1, i, z0, r), gs.single_width, gs.single_width_cut_in);
            let mut distance = if byte & 4 != 0 { gs.round.apply(original) } else { original };
            if byte & 8 != 0 {
                distance = at_least(distance, gs.minimum_distance, original);
            }
            let now = self.current_distance(z1, i, z0, r);
            self.move_point(z1, i, distance.saturating_sub(now));
        }
        self.moved_relative(p, byte & 16 != 0);
    }

    // MIRP: p (zp1) the CVT entry's distance from rp0 (zp0), a twilight p first given that original along the freedom
    // vector. Auto flip gives the entry the original distance's sign; when rounding, past the cut-in the original wins.
    pub(super) fn move_indirect_relative(&mut self, byte: u8) {
        let (entry, p) = (self.pop(), self.pop_point());
        let [z0, z1, _] = self.gs.zp;
        let target = self.vm.cvt.get(super::index(entry)).copied();
        if let (Some(i), Some(r), Some(target)) = (self.at(z1, p), self.at(z0, self.gs.rp[0]), target) {
            let mut target = single_width(target, self.gs.single_width, self.gs.single_width_cut_in);
            if z1 == 0 {
                let (f, o) = (self.gs.freedom, &self.zones[z0].original);
                let x = o.x[r].saturating_add(f26dot6::times_2_14(target, f.x));
                let y = o.y[r].saturating_add(f26dot6::times_2_14(target, f.y));
                self.place_twilight(i, x, y);
            }
            let original = self.original_distance(z1, i, z0, r);
            let now = self.current_distance(z1, i, z0, r);
            let gs = &self.gs;
            if gs.auto_flip && (original < 0) != (target < 0) {
                target = target.saturating_neg();
            }
            let mut distance = target;
            if byte & 4 != 0 {
                // The cut-in is tested only within one zone, which the spec leaves unsaid but fonts are hinted for.
                if z0 == z1 && (i64::from(target) - i64::from(original)).abs() > i64::from(gs.cut_in) {
                    distance = original;
                }
                distance = gs.round.apply(distance);
            }
            if byte & 8 != 0 {
                distance = at_least(distance, gs.minimum_distance, original);
            }
            self.move_point(z1, i, distance.saturating_sub(now));
        }
        self.moved_relative(p, byte & 16 != 0);
    }

    // MSIRP: p (zp1) at the popped distance from rp0 (zp0); a twilight p is first placed on rp0's original, moved that
    // far along the freedom vector.
    pub(super) fn move_stack_indirect(&mut self, byte: u8) {
        let (distance, p) = (self.pop(), self.pop_point());
        let [z0, z1, _] = self.gs.zp;
        if let (Some(i), Some(r)) = (self.at(z1, p), self.at(z0, self.gs.rp[0])) {
            if z1 == 0 {
                let (m, o) = (self.motion, &self.zones[z0].original);
                let x = o.x[r].saturating_add(if m.reach.x { f26dot6::times_16_16(distance, m.x) } else { 0 });
                let y = o.y[r].saturating_add(if m.reach.y { f26dot6::times_16_16(distance, m.y) } else { 0 });
                self.place_twilight(i, x, y);
            }
            let now = self.current_distance(z1, i, z0, r);
            self.move_point(z1, i, distance.saturating_sub(now));
        }
        self.moved_relative(p, byte & 1 != 0);
    }

    // ALIGNRP: each point (zp1) onto rp0 (zp0) along the projection.
    pub(super) fn align_to_reference(&mut self) -> Step {
        let [z0, z1, _] = self.gs.zp;
        let reference = self.at(z0, self.gs.rp[0]);
        self.each_point(ALIGNED, |m, p| {
            if let (Some(r), Some(i)) = (reference, m.at(z1, p)) {
                let d = m.current_distance(z1, i, z0, r);
                m.move_point(z1, i, d.saturating_neg());
            }
        })
    }

    // ALIGNPTS: the top point (zp0) and the next (zp1) each moved half the distance between them.
    pub(super) fn align_points(&mut self) {
        let (p2, p1) = (self.pop_point(), self.pop_point());
        let [z0, z1, _] = self.gs.zp;
        let (Some(p1), Some(p2)) = (self.at(z1, p1), self.at(z0, p2)) else { return };
        let half = self.current_distance(z0, p2, z1, p1) / 2;
        self.move_point(z1, p1, half);
        self.move_point(z0, p2, half.saturating_neg());
    }

    // UTP: p (zp0) untouched on the axes the freedom vector reaches.
    pub(super) fn untouch(&mut self) {
        let p = self.pop_point();
        let z = self.gs.zp[0];
        let Some(i) = self.at(z, p) else { return };
        let f = self.gs.freedom;
        let mut keep = !0u8;
        if f.x != 0 {
            keep &= !FLAG_TOUCHED_X;
        }
        if f.y != 0 {
            keep &= !FLAG_TOUCHED_Y;
        }
        self.zones[z].flags[i] &= keep;
    }

    // How far rp1 (zp0, the opcode's low bit set) or rp2 (zp1) has moved along the projection, as a displacement along
    // the freedom vector: what SHP, SHC and SHZ shift by.
    fn reference_shift(&self, byte: u8) -> Option<(usize, usize, (i32, i32))> {
        let (z, p) = if byte & 1 != 0 { (self.gs.zp[0], self.gs.rp[1]) } else { (self.gs.zp[1], self.gs.rp[2]) };
        let p = self.at(z, p)?;
        let (c, o) = (&self.zones[z].current, &self.zones[z].original);
        let d = along(self.gs.projection, i64::from(c.x[p]) - i64::from(o.x[p]), i64::from(c.y[p]) - i64::from(o.y[p]));
        Some((z, p, (f26dot6::times_16_16(d, self.motion.x), f26dot6::times_16_16(d, self.motion.y))))
    }

    pub(super) fn shift_points(&mut self, byte: u8) -> Step {
        let shift = self.reference_shift(byte);
        let (z, reach) = (self.gs.zp[2], self.freedom_reach());
        self.each_point(MOVED, |m, p| {
            if let (Some((_, _, d)), Some(i)) = (shift, m.at(z, p)) {
                m.shift(z, i, d, reach, true);
            }
        })
    }

    // SHC: contour c of zp2, but for the reference point. The twilight zone has no contours.
    pub(super) fn shift_contour(&mut self, byte: u8) -> Step {
        let contour = self.pop_point() as usize;
        let z = self.gs.zp[2];
        let ends = &self.zones[1].ends;
        if z == 0 || contour >= ends.len() {
            return Ok(());
        }
        let first = if contour == 0 { 0 } else { ends[contour - 1] + 1 };
        let last = ends[contour].min(self.zones[1].len().saturating_sub(1));
        let Some((rz, rp, d)) = self.reference_shift(byte) else { return Ok(()) };
        self.charge((last + 1).saturating_sub(first), MOVED)?;
        let reach = self.freedom_reach();
        for i in first..=last {
            if rz != 1 || rp != i {
                self.shift(1, i, d, reach, true);
            }
        }
        Ok(())
    }

    // SHZ: every point of the zone named, but for the reference point, and in the glyph the phantom points: a zone
    // shift moves the outline against its metrics. It leaves the points as touched as they were.
    pub(super) fn shift_zone(&mut self, byte: u8) -> Step {
        let z = match self.pop() {
            0 => 0,
            1 => 1,
            _ => return Ok(()),
        };
        let Some((rz, rp, d)) = self.reference_shift(byte) else { return Ok(()) };
        let count = if z == 0 { self.zones[0].len() } else { self.zones[1].ends.last().map_or(0, |&e| e + 1) };
        let count = count.min(self.zones[z].len());
        self.charge(count, MOVED)?;
        let reach = self.freedom_reach();
        for i in 0..count {
            if rz != z || rp != i {
                self.shift(z, i, d, reach, false);
            }
        }
        Ok(())
    }

    // SHPIX: each point (zp2) by the popped amount along the freedom vector. Only a delta `ClearType` keeps moves, or
    // any point while a zone pointer names the twilight zone.
    pub(super) fn shift_by_pixels(&mut self) -> Step {
        let amount = self.pop();
        let f = self.gs.freedom;
        let d = (f26dot6::times_2_14(amount, f.x), f26dot6::times_2_14(amount, f.y));
        let (z, twilight, reach) = (self.gs.zp[2], self.gs.zp.contains(&0), self.freedom_reach());
        let composite = self.composite && f.y != 0;
        self.each_point(MOVED, |m, p| {
            let Some(i) = m.at(z, p) else { return };
            let touched_y = m.zones[z].flags[i] & FLAG_TOUCHED_Y != 0;
            if twilight || m.rules.keeps_delta(touched_y, composite) {
                m.shift(z, i, d, reach, true);
            }
        })
    }

    // IP: each point (zp2) keeps its original share of the span from rp1 (zp0) to rp2 (zp1), unscaled outside the
    // twilight zone; with no original span, it keeps its scaled original distance from rp1.
    pub(super) fn interpolate(&mut self) -> Step {
        let [z0, z1, z2] = self.gs.zp;
        let twilight = z0 == 0 || z1 == 0 || z2 == 0;
        let Some(r1) = self.at(z0, self.gs.rp[1]) else { return self.each_point(ELEMENT, |_, _| {}) };
        let r2 = self.at(z1, self.gs.rp[2]);
        let original = |m: &Self, z: usize, p: usize| {
            if twilight { m.original_distance(z, p, z0, r1) } else { m.unscaled_distance(z, p, z0, r1) }
        };
        let (was, is) = r2.map_or((0, 0), |r2| (original(self, z1, r2), self.current_distance(z1, r2, z0, r1)));
        self.each_point(INTERPOLATED, |m, p| {
            let Some(i) = m.at(z2, p) else { return };
            let from = original(m, z2, i);
            let target = match (was, twilight) {
                (0, true) => from,
                (0, false) => f26dot6::scale_units(from, m.unscaled),
                _ => f26dot6::ratio(from, is, was),
            };
            let now = m.current_distance(z2, i, z0, r1);
            m.move_point(z2, i, target.saturating_sub(now));
        })
    }

    // IUP over the glyph zone zp2 names; a twilight zp2 has no contours to work on. In backward compatibility, IUP
    // along x moves nothing, as nothing else does, and none runs once the outline is settled.
    pub(super) fn interpolate_untouched(&mut self, x_axis: bool) -> Step {
        if self.gs.zp[2] == 0 || self.rules.settled() {
            return Ok(());
        }
        self.rules.ran_iup(x_axis);
        if x_axis && !self.rules.moves_x() {
            return Ok(());
        }
        self.charge(self.zones[1].len(), IUP_POINT)?;
        let zone = &mut *self.zones[1];
        let mask = if x_axis { FLAG_TOUCHED_X } else { FLAG_TOUCHED_Y };
        let (current, original, unscaled) = if x_axis {
            (&mut zone.current.x, &zone.original.x, &zone.unscaled.x)
        } else {
            (&mut zone.current.y, &zone.original.y, &zone.unscaled.y)
        };
        let mut axis = Untouched { current, original, unscaled };
        let mut first = 0;
        for &end in &zone.ends {
            if end < first || end >= axis.current.len() {
                break;
            }
            let touched = |i: usize| zone.flags[i] & mask != 0;
            if let Some(start) = (first..=end).find(|&i| touched(i)) {
                let mut from = start;
                loop {
                    let next = |i: usize| if i == end { first } else { i + 1 };
                    let mut to = next(from);
                    while to != start && !touched(to) {
                        to = next(to);
                    }
                    axis.between(from, to, first, end);
                    if to == start {
                        break;
                    }
                    from = to;
                }
            }
            first = end + 1;
        }
        Ok(())
    }

    // FLIPPT: each point (zp0) on the curve made off it and off made on; nothing once the outline is settled.
    pub(super) fn flip_points(&mut self) -> Step {
        let (z, settled) = (self.gs.zp[0], self.rules.settled());
        self.each_point(MOVED, |m, p| {
            if let Some(i) = m.at(z, p).filter(|_| !settled) {
                m.zones[z].flags[i] ^= FLAG_ON_CURVE;
            }
        })
    }

    // FLIPRGON and FLIPRGOFF over the glyph's points from the lower number popped to the top one.
    pub(super) fn flip_range(&mut self, on: bool) -> Step {
        let (high, low) = (self.pop_point(), self.pop_point());
        let (Some(high), Some(low)) = (self.at(1, high), self.at(1, low)) else { return Ok(()) };
        if self.rules.settled() || low > high {
            return Ok(());
        }
        self.charge(high - low + 1, ELEMENT)?;
        for flag in &mut self.zones[1].flags[low..=high] {
            if on { *flag |= FLAG_ON_CURVE } else { *flag &= !FLAG_ON_CURVE }
        }
        Ok(())
    }

    // GC: p (zp2) along the projection where it is, or along the dual projection where it was.
    pub(super) fn get_coordinate(&mut self, original: bool) {
        let p = self.pop_point();
        let z = self.gs.zp[2];
        let v = self.at(z, p).map_or(0, |i| {
            if original {
                let o = &self.zones[z].original;
                along(self.gs.dual, o.x[i].into(), o.y[i].into())
            } else {
                self.projected(z, i)
            }
        });
        self.push(v);
    }

    // SCFS: p (zp2) moved along the freedom vector until its projection is the popped value; a twilight point's
    // original goes with it.
    pub(super) fn set_coordinate(&mut self) {
        let (value, p) = (self.pop(), self.pop_point());
        let z = self.gs.zp[2];
        let Some(i) = self.at(z, p) else { return };
        let now = self.projected(z, i);
        self.move_point(z, i, value.saturating_sub(now));
        if z == 0 {
            let c = &self.zones[0].current;
            let (x, y) = (c.x[i], c.y[i]);
            self.place_twilight(i, x, y);
        }
    }

    // MD: the second point (zp0) less the top one (zp1), now or originally.
    pub(super) fn measure_distance(&mut self, original: bool) {
        let (top, second) = (self.pop_point(), self.pop_point());
        let [z0, z1, _] = self.gs.zp;
        let d = match (self.at(z0, second), self.at(z1, top)) {
            (Some(a), Some(b)) if original => self.design_distance(z0, a, z1, b),
            (Some(a), Some(b)) => self.current_distance(z0, a, z1, b),
            _ => 0,
        };
        self.push(d);
    }

    // DELTA's pairs, n of them under the popped count, and the relative ppem they match against: the size less
    // delta_base, less 16 for the second group and 32 for the third. None where no pair can match.
    fn delta_pairs(&mut self, group: i64) -> Step<(usize, Option<i32>)> {
        let n = usize::try_from(self.pop()).map_err(|_| Stop::BadArgument)?;
        if n > self.stack.len() / 2 {
            return Err(Stop::StackUnderflow);
        }
        self.charge(n, DELTA)?;
        let relative = i64::from(self.vm.size.ppem()) - i64::from(self.gs.delta_base) - 16 * group;
        Ok((self.stack.len() - 2 * n, (0..16).contains(&relative).then_some(relative as i32)))
    }

    // A pair's amount when its ppem nibble matches: the low nibble's -8 to -1 and 1 to 8 steps of 1/2^delta_shift px.
    fn delta_amount(&self, arg: i32, relative: i32) -> Option<i32> {
        if (arg >> 4) & 0xF != relative {
            return None;
        }
        let step = (arg & 0xF) - 8;
        let step = if step >= 0 { step + 1 } else { step };
        Some(step * (64 >> self.gs.delta_shift))
    }

    // DELTAP1 to 3: each matching pair moves its point (zp0), the pair nearest the top first; only deltas `ClearType`
    // keeps move.
    pub(super) fn delta_points(&mut self, byte: u8) -> Step {
        let (at, relative) = self.delta_pairs(match byte { 0x5D => 0, 0x71 => 1, _ => 2 })?;
        if let Some(relative) = relative {
            let z = self.gs.zp[0];
            let composite = self.composite && self.gs.freedom.y != 0;
            for k in (at..self.stack.len()).step_by(2).rev() {
                let (arg, p) = (self.stack[k], point_number(self.stack[k + 1]));
                let (Some(amount), Some(i)) = (self.delta_amount(arg, relative), self.at(z, p)) else { continue };
                if self.rules.keeps_delta(self.zones[z].flags[i] & FLAG_TOUCHED_Y != 0, composite) {
                    self.move_point(z, i, amount);
                }
            }
        }
        self.stack.truncate(at);
        Ok(())
    }

    // DELTAC1 to 3: each matching pair changes its CVT entry.
    pub(super) fn delta_cvt(&mut self, byte: u8) -> Step {
        let (at, relative) = self.delta_pairs(i64::from(byte - 0x73))?;
        if let Some(relative) = relative {
            for k in (at..self.stack.len()).step_by(2).rev() {
                let (arg, entry) = (self.stack[k], self.stack[k + 1]);
                if let Some(amount) = self.delta_amount(arg, relative) {
                    self.write_cvt(entry, |v| v.saturating_add(amount));
                }
            }
        }
        self.stack.truncate(at);
        Ok(())
    }
}

// One axis of the glyph zone as IUP reads it: where points are, where they were, and where the font has them.
struct Untouched<'a> {
    current: &'a mut [i32],
    original: &'a [i32],
    unscaled: &'a [i32],
}

impl Untouched<'_> {
    // The untouched points from `from` round to `to` in the contour `first..=end`: inside the pair's original span a
    // point keeps its share by unscaled coordinate, outside it moves as the nearer did; a lone `from` moves them all.
    fn between(&mut self, from: usize, to: usize, first: usize, end: usize) {
        let (lo, hi) = if self.original[from] <= self.original[to] { (from, to) } else { (to, from) };
        let (o_lo, o_hi) = (i64::from(self.original[lo]), i64::from(self.original[hi]));
        let (c_lo, c_hi) = (i64::from(self.current[lo]), i64::from(self.current[hi]));
        // Positions along the span by unscaled coordinates, or scaled ones where the font has the two alike.
        let (s_lo, s_hi, by_unscaled) = match (self.unscaled[lo], self.unscaled[hi]) {
            (a, b) if a != b => (i64::from(a), i64::from(b), true),
            _ => (o_lo, o_hi, false),
        };
        // A slope kept to 2^-32 of a 64th, so each point rounds once.
        let mut slope = None;
        let mut i = if from == end { first } else { from + 1 };
        while i != to {
            let o = i64::from(self.original[i]);
            let placed = if o <= o_lo {
                i128::from(o + (c_lo - o_lo))
            } else if o >= o_hi {
                i128::from(o + (c_hi - o_hi))
            } else {
                let s = if by_unscaled { i64::from(self.unscaled[i]) } else { o };
                let slope = *slope.get_or_insert_with(|| (i128::from(c_hi - c_lo) << 32) / i128::from(s_hi - s_lo));
                i128::from(c_lo) + ((i128::from(s - s_lo) * slope + (1 << 31)) >> 32)
            };
            self.current[i] = placed.clamp(i32::MIN.into(), i32::MAX.into()) as i32;
            i = if i == end { first } else { i + 1 };
        }
    }
}

// A point number, by its low 16 bits: glyf numbers points in 16 bits, and PUSHW sign-extends what it pushes, so a
// point from 32,768 on arrives negative.
fn point_number(v: i32) -> u32 {
    u32::from(v as u16)
}
