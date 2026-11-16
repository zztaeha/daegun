// A sweep down monotone lines, top first: their order across kept as it passes their points, where they enter and
// leave and where neighbors cross, and the winding beside each read as it goes.
use alloc::vec::Vec;

#[derive(Clone, Copy, Default)]
pub(super) struct Pt {
    pub(super) x: f64,
    pub(super) y: f64,
}

// A run of a contour going one way in y, its points stored top first.
#[derive(Clone, Copy)]
pub(super) struct Chain {
    pub(super) start: u32,
    pub(super) end: u32,
    pub(super) dir: i32,
}

#[inline]
pub(super) fn x_at(p: Pt, q: Pt, y: f64) -> f64 {
    if y == p.y {
        p.x
    } else if y == q.y {
        q.x
    } else {
        p.x + (y - p.y) * (q.x - p.x) / (q.y - p.y)
    }
}

// A distance field's sweep counts every pair of chords meeting where one ends too, a few a chord for any outline,
// and allows MEETINGS a chord past MEETINGS_BASE.
pub(super) const MEETINGS: u64 = 64;
pub(super) const MEETINGS_BASE: u64 = 1 << 16;

const NIL: u32 = u32::MAX;

// A sweep's lines in order across, as a splay tree over slots linked to their neighbors: a line goes in or out in log
// time amortized whatever the input, and finds a neighbor or swaps slots with one in constant time.
#[derive(Default)]
pub(super) struct Order {
    up: Vec<u32>,
    kids: Vec<[u32; 2]>,
    beside: Vec<[u32; 2]>,
    line: Vec<u32>,
    slot: Vec<u32>,
    root: u32,
}

impl Order {
    pub(super) fn reset(&mut self, n: usize) {
        self.up.clear();
        self.up.resize(n, NIL);
        self.kids.clear();
        self.kids.resize(n, [NIL; 2]);
        self.beside.clear();
        self.beside.resize(n, [NIL; 2]);
        self.line.clear();
        self.line.extend(0..n as u32);
        self.slot.clear();
        self.slot.extend(0..n as u32);
        self.root = NIL;
    }

    fn rotate(&mut self, x: u32) {
        let p = self.up[x as usize];
        let g = self.up[p as usize];
        let side = usize::from(self.kids[p as usize][1] == x);
        let b = self.kids[x as usize][1 - side];
        self.kids[p as usize][side] = b;
        if b != NIL {
            self.up[b as usize] = p;
        }
        self.kids[x as usize][1 - side] = p;
        (self.up[p as usize], self.up[x as usize]) = (x, g);
        if g == NIL {
            self.root = x;
        } else {
            let side = usize::from(self.kids[g as usize][1] == p);
            self.kids[g as usize][side] = x;
        }
    }

    fn splay(&mut self, x: u32) {
        while self.up[x as usize] != NIL {
            let p = self.up[x as usize];
            let g = self.up[p as usize];
            if g != NIL {
                let zig = (self.kids[g as usize][1] == p) == (self.kids[p as usize][1] == x);
                self.rotate(if zig { p } else { x });
            }
            self.rotate(x);
        }
    }

    // Line i put in where `left_of` first says it goes left of the line held, else right.
    pub(super) fn insert(&mut self, i: u32, mut left_of: impl FnMut(u32) -> bool) {
        let s = self.slot[i as usize];
        (self.up[s as usize], self.kids[s as usize], self.beside[s as usize]) = (NIL, [NIL; 2], [NIL; 2]);
        let mut at = self.root;
        if at == NIL {
            self.root = s;
            return;
        }
        loop {
            let side = usize::from(!left_of(self.line[at as usize]));
            let next = self.kids[at as usize][side];
            if next == NIL {
                self.kids[at as usize][side] = s;
                self.up[s as usize] = at;
                // Its parent is its neighbor on the other side, and that one's old neighbor now its own.
                let far = self.beside[at as usize][side];
                self.beside[s as usize] = if side == 0 { [far, at] } else { [at, far] };
                self.beside[at as usize][side] = s;
                if far != NIL {
                    self.beside[far as usize][1 - side] = s;
                }
                break;
            }
            at = next;
        }
        self.splay(s);
    }

    pub(super) fn remove(&mut self, i: u32) {
        let s = self.slot[i as usize];
        let [l, r] = self.beside[s as usize];
        if l != NIL {
            self.beside[l as usize][1] = r;
        }
        if r != NIL {
            self.beside[r as usize][0] = l;
        }
        self.splay(s);
        let [l, r] = self.kids[s as usize];
        for k in [l, r].into_iter().filter(|&k| k != NIL) {
            self.up[k as usize] = NIL;
        }
        self.root = l;
        if l == NIL {
            self.root = r;
            return;
        }
        let mut m = l;
        while self.kids[m as usize][1] != NIL {
            m = self.kids[m as usize][1];
        }
        self.splay(m);
        self.kids[m as usize][1] = r;
        if r != NIL {
            self.up[r as usize] = m;
        }
    }

    // The line beside line i, left (0) or right (1).
    pub(super) fn beside(&self, i: u32, side: usize) -> Option<u32> {
        let at = self.beside[self.slot[i as usize] as usize][side];
        (at != NIL).then(|| self.line[at as usize])
    }

    pub(super) fn swap(&mut self, a: u32, b: u32) {
        let (sa, sb) = (self.slot[a as usize], self.slot[b as usize]);
        (self.line[sa as usize], self.line[sb as usize]) = (b, a);
        (self.slot[a as usize], self.slot[b as usize]) = (sb, sa);
    }
}

// Each line's crossing with its right neighbor (its height's key, the neighbor, both segments), one slot a line in a
// heap by height, overwritten as either changes: it holds no more than the lines however often they cross.
#[derive(Default)]
pub(super) struct Crossings {
    heap: Vec<u32>,
    place: Vec<u32>,
    next: Vec<(u64, u32, u32, u32)>,
}

impl Crossings {
    fn reset(&mut self, n: usize) {
        self.heap.clear();
        self.place.clear();
        self.place.resize(n, NIL);
        self.next.clear();
        self.next.resize(n, (0, 0, 0, 0));
    }

    fn before(&self, a: u32, b: u32) -> bool {
        (self.next[a as usize].0, a) < (self.next[b as usize].0, b)
    }

    fn set(&mut self, l: u32, crossing: (u64, u32, u32, u32)) {
        self.next[l as usize] = crossing;
        if self.place[l as usize] == NIL {
            self.place[l as usize] = self.heap.len() as u32;
            self.heap.push(l);
        }
        let at = self.place[l as usize] as usize;
        self.up(at);
        self.down(self.place[l as usize] as usize);
    }

    fn first(&self) -> Option<u64> {
        self.heap.first().map(|&l| self.next[l as usize].0)
    }

    fn pop(&mut self) -> Option<(u32, (u64, u32, u32, u32))> {
        let l = *self.heap.first()?;
        let last = self.heap.pop()?;
        if last != l {
            (self.heap[0], self.place[last as usize]) = (last, 0);
            self.down(0);
        }
        self.place[l as usize] = NIL;
        Some((l, self.next[l as usize]))
    }

    fn up(&mut self, mut at: usize) {
        while at > 0 && self.before(self.heap[at], self.heap[(at - 1) / 2]) {
            self.trade(at, (at - 1) / 2);
            at = (at - 1) / 2;
        }
    }

    fn down(&mut self, mut at: usize) {
        loop {
            let (a, b) = (2 * at + 1, 2 * at + 2);
            let mut least = at;
            for k in [a, b] {
                if k < self.heap.len() && self.before(self.heap[k], self.heap[least]) {
                    least = k;
                }
            }
            if least == at {
                return;
            }
            self.trade(at, least);
            at = least;
        }
    }

    fn trade(&mut self, a: usize, b: usize) {
        self.heap.swap(a, b);
        self.place[self.heap[a] as usize] = a as u32;
        self.place[self.heap[b] as usize] = b as u32;
    }
}

// What a sweep down monotone lines keeps: their order across, each one's segment, the winding right of it and where
// its traced stretch began with its way (0 for none), the events still to come and the lines an event touched.
#[derive(Default)]
pub(super) struct Sweep {
    pub(super) order: Order,
    pub(super) at: Vec<u32>,
    pub(super) w: Vec<i32>,
    pub(super) open: Vec<(f64, u32, i32)>,
    pub(super) dirty: Vec<bool>,
    pub(super) alive: Vec<bool>,
    pub(super) events: Vec<(f64, u32)>,
    // The events as `sort` deals them out by height, and where each slot begins.
    spare: Vec<(f64, u32)>,
    starts: Vec<u32>,
    crossings: Crossings,
    pub(super) touched: Vec<u32>,
    // Each touched line's state in `tests`: 0 untouched, 1 touched, 2 tested; and the pair `cross` just swapped,
    // which can't cross again.
    state: Vec<u8>,
    crossed: (u32, u32),
    // The level segments at a height, by their span across.
    levels: Vec<(f64, f64, u32)>,
    // Swaps made where neighbors cross, windings changed beside level stretches, and the meetings a field allows.
    pub(super) swaps: u64,
    pub(super) steps: u64,
    pub(super) allowed: u64,
}

// An event's kind, in the order a y's are taken: a line passing a point, leaving, entering.
pub(super) const PASS: u32 = 0;
pub(super) const LEAVE: u32 = 1 << 30;
pub(super) const ENTER: u32 = 2 << 30;
pub(super) const LINE: u32 = (1 << 30) - 1;

// The x a segment of a line going `dir` leaves y at, and its x per y. Worked out from the end its contour leaves, so a
// point on it reads the same side whichever way the line is stored.
fn leaving(pts: &[Pt], j: u32, dir: i32, y: f64) -> (f64, f64) {
    let (p, q) = (pts[j as usize], pts[j as usize + 1]);
    (if dir < 0 { x_at(q, p, y) } else { x_at(p, q, y) }, (q.x - p.x) / (q.y - p.y))
}

// A height as a key ordered as the height is, below 0 too: a crossing's place in the heap.
pub(super) fn height_key(y: f64) -> u64 {
    let b = y.to_bits();
    if b >> 63 == 1 { !b } else { b | 1 << 63 }
}

fn height(key: u64) -> f64 {
    f64::from_bits(if key >> 63 == 1 { key & !(1 << 63) } else { !key })
}

fn before(a: (f64, f64), b: (f64, f64)) -> bool {
    a.0 < b.0 || a.0 == b.0 && a.1 < b.1
}

impl Sweep {
    pub(super) fn reset(&mut self, lines: &[Chain]) {
        self.order.reset(lines.len());
        self.at.clear();
        self.at.extend(lines.iter().map(|l| l.start));
        self.w.clear();
        self.w.resize(lines.len(), 0);
        self.open.clear();
        self.open.resize(lines.len(), (0.0, 0, 0));
        self.dirty.clear();
        self.dirty.resize(lines.len(), false);
        self.alive.clear();
        self.alive.resize(lines.len(), false);
        self.events.clear();
        self.crossings.reset(lines.len());
        self.touched.clear();
        self.state.clear();
        self.state.resize(lines.len(), 0);
        self.levels.clear();
    }

    // The events in order, dealt out by height to as many slots as there are, `height` rows deep, then each slot's
    // sorted: linear in them where they're spread out down the rows, as a glyph's are.
    pub(super) fn sort(&mut self, height: usize) {
        let n = self.events.len();
        let scale = n as f64 / height as f64;
        let slot = |y: f64| ((y * scale).max(0.0) as usize).min(n - 1);
        self.starts.clear();
        self.starts.resize(n + 1, 0);
        for e in &self.events {
            self.starts[slot(e.0) + 1] += 1;
        }
        for r in 1..=n {
            self.starts[r] += self.starts[r - 1];
        }
        self.spare.clear();
        self.spare.resize(n, (0.0, 0));
        for &e in &self.events {
            let at = &mut self.starts[slot(e.0)];
            self.spare[*at as usize] = e;
            *at += 1;
        }
        let mut from = 0;
        for &to in &self.starts[..n] {
            if to as usize > from + 1 {
                self.spare[from..to as usize].sort_unstable_by_key(|&(y, e)| (height_key(y), e));
            }
            from = to as usize;
        }
        core::mem::swap(&mut self.events, &mut self.spare);
    }

    // The order just below y built afresh, as if every event to y had been taken but no crossing traced: each line
    // then alive entered at y, every stretch closed.
    pub(super) fn restart(&mut self, pts: &[Pt], lines: &[Chain], y: f64) {
        self.order.reset(lines.len());
        self.crossings.reset(lines.len());
        self.touched.clear();
        for (i, l) in lines.iter().enumerate() {
            (self.alive[i], self.open[i].2) = (false, 0);
            if pts[l.start as usize].y <= y && pts[l.end as usize - 1].y > y {
                self.enter(pts, lines, i as u32, y);
            }
        }
    }

    // Line i onto the segment leaving y, past any ending there, level ones too.
    pub(super) fn pass(&mut self, pts: &[Pt], lines: &[Chain], i: u32, y: f64) {
        let (at, end) = (&mut self.at[i as usize], lines[i as usize].end);
        while *at + 2 < end && pts[*at as usize + 1].y <= y {
            *at += 1;
        }
    }

    pub(super) fn key(&mut self, pts: &[Pt], lines: &[Chain], i: u32, y: f64) -> (f64, f64) {
        self.pass(pts, lines, i, y);
        leaving(pts, self.at[i as usize], lines[i as usize].dir, y)
    }

    pub(super) fn enter(&mut self, pts: &[Pt], lines: &[Chain], i: u32, y: f64) {
        let k = self.key(pts, lines, i, y);
        let mut order = core::mem::take(&mut self.order);
        order.insert(i, |j| before(k, self.key(pts, lines, j, y)));
        self.order = order;
        (self.alive[i as usize], self.dirty[i as usize]) = (true, true);
        self.touched.push(i);
    }

    // Line i out, its neighbors marked, as lines between a pair leaving at one point change sides.
    pub(super) fn leave(&mut self, i: u32) {
        for side in [0, 1] {
            if let Some(j) = self.order.beside(i, side) {
                self.dirty[j as usize] = true;
                self.touched.push(j);
            }
        }
        self.order.remove(i);
        self.alive[i as usize] = false;
    }

    pub(super) fn swap(&mut self, a: u32, b: u32) {
        self.order.swap(a, b);
        self.swaps += 1;
        for i in [a, b] {
            self.dirty[i as usize] = true;
            self.touched.push(i);
        }
    }

    // Line i, its segment just changed at y, swapped past each neighbor it now leaves y on the far side of.
    pub(super) fn settle(&mut self, pts: &[Pt], lines: &[Chain], i: u32, y: f64) {
        let a = self.key(pts, lines, i, y);
        for side in [0, 1] {
            while let Some(j) = self.order.beside(i, side) {
                let b = self.key(pts, lines, j, y);
                if !if side == 0 { before(a, b) } else { before(b, a) } {
                    break;
                }
                self.swap(i, j);
            }
        }
    }

    // Each line marked since and those right of it, until the winding is as it was, `each` seeing both sides' and
    // false to stop. An unmarked line whose winding changes crosses a level stretch, counted as a swap.
    pub(super) fn wind(&mut self, lines: &[Chain], mut each: impl FnMut(&mut Self, u32, i32, i32) -> bool) -> bool {
        for k in 0..self.touched.len() {
            let mut i = self.touched[k];
            if !self.alive[i as usize] || !self.dirty[i as usize] {
                continue;
            }
            while let Some(j) = self.order.beside(i, 0).filter(|&j| self.dirty[j as usize]) {
                i = j;
            }
            let mut w = self.order.beside(i, 0).map_or(0, |j| self.w[j as usize]);
            loop {
                let next = w + lines[i as usize].dir;
                let marked = self.dirty[i as usize];
                if !marked && next == self.w[i as usize] {
                    break;
                }
                self.steps += u64::from(!marked);
                (self.w[i as usize], self.dirty[i as usize]) = (next, false);
                if !each(self, i, w, next) {
                    return false;
                }
                w = next;
                let Some(j) = self.order.beside(i, 1) else { break };
                i = j;
            }
        }
        true
    }

    // Where two neighbors' segments leaving y swap before either ends: their order there reversed.
    pub(super) fn test(&mut self, pts: &[Pt], lines: &[Chain], l: u32, r: u32, y: f64) {
        let (jl, jr) = (self.at[l as usize], self.at[r as usize]);
        let yb = pts[jl as usize + 1].y.min(pts[jr as usize + 1].y);
        if yb <= y {
            return;
        }
        let x = |i: u32, j: u32, y: f64| leaving(pts, j, lines[i as usize].dir, y).0;
        let (d0, d1) = (x(r, jr, y) - x(l, jl, y), x(r, jr, yb) - x(l, jl, yb));
        if d1 < 0.0 {
            let t = if d0 > 0.0 { d0 / (d0 - d1) } else { 0.0 };
            let at = (y + (yb - y) * t).clamp(y, yb);
            self.crossings.set(l, (height_key(at), r, jl, jr));
        }
    }

    // The height of the next event, `event` or the nearest crossing.
    pub(super) fn ahead(&self, event: Option<f64>) -> Option<f64> {
        match (event, self.crossings.first().map(height)) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    // The nearest crossing, its lines swapped if they still lie side by side on those segments.
    pub(super) fn cross(&mut self) {
        let Some((l, (_, r, jl, jr))) = self.crossings.pop() else { return };
        let alive = self.alive[l as usize] && self.alive[r as usize];
        let current = alive && (self.at[l as usize], self.at[r as usize]) == (jl, jr);
        if current && self.order.beside(l, 1) == Some(r) {
            self.swap(l, r);
            self.crossed = (r, l);
        }
    }

    // Each neighbor pair an event touched, tested once: a touched line tests its left neighbor only if that one
    // wasn't touched too.
    pub(super) fn tests(&mut self, pts: &[Pt], lines: &[Chain], y: f64) {
        for &i in &self.touched {
            self.state[i as usize] = 1;
        }
        for k in 0..self.touched.len() {
            let i = self.touched[k];
            if !self.alive[i as usize] || self.state[i as usize] == 2 {
                continue;
            }
            self.state[i as usize] = 2;
            if let Some(l) = self.order.beside(i, 0).filter(|&l| self.state[l as usize] == 0) {
                self.test(pts, lines, l, i, y);
            }
            if let Some(r) = self.order.beside(i, 1).filter(|&r| (i, r) != self.crossed) {
                self.test(pts, lines, i, r, y);
            }
        }
        for &i in &self.touched {
            self.state[i as usize] = 0;
        }
        self.touched.clear();
        self.crossed = (NIL, NIL);
    }
}

impl Order {
    // The last line a point lies right of or on, as `right_of` tells it by the line held; none left of it.
    fn locate(&mut self, mut right_of: impl FnMut(u32) -> bool) -> Option<u32> {
        let (mut at, mut last, mut found) = (self.root, NIL, NIL);
        while at != NIL {
            last = at;
            let side = usize::from(right_of(self.line[at as usize]));
            if side == 1 {
                found = at;
            }
            at = self.kids[at as usize][side];
        }
        if last != NIL {
            self.splay(last);
        }
        (found != NIL).then(|| self.line[found as usize])
    }
}

impl Sweep {
    // The winding just right of x at y, as the line left of it has it.
    fn winding_at(&mut self, pts: &[Pt], lines: &[Chain], (y, x): (f64, f64)) -> i32 {
        let mut order = core::mem::take(&mut self.order);
        let left = order.locate(|j| self.key(pts, lines, j, y).0 <= x);
        self.order = order;
        left.map_or(0, |j| self.w[j as usize])
    }

    // Line i's neighbors at y, and those beyond them as far as they meet it there, each handed to `pair` and counted
    // with the swaps, as lines meeting at a point meet as often as they cross; false once the swaps pass `allowed`.
    fn meet(&mut self, pts: &[Pt], lines: &[Chain], i: u32, y: f64, pair: &mut impl FnMut(u32, u32)) -> bool {
        let x = self.key(pts, lines, i, y).0;
        for side in [0, 1] {
            let mut at = i;
            while let Some(j) = self.order.beside(at, side) {
                pair(self.at[i as usize], self.at[j as usize]);
                self.swaps += 1;
                if self.swaps > self.allowed {
                    return false;
                }
                if self.key(pts, lines, j, y).0 != x {
                    break;
                }
                at = j;
            }
        }
        true
    }

    /// `lines`, each one segment, swept top to bottom. `pair` takes two segments, by their first points, each time
    /// they become neighbors or meet where one ends; a level one, every segment it passes. Each of `queries`, (y, x)
    /// in order of y, takes the winding right of x into `windings`. False once the swaps pass `allowed`.
    pub(super) fn run(
        &mut self, pts: &[Pt], lines: &[Chain], queries: &[(f64, f64)], windings: &mut Vec<i32>,
        mut pair: impl FnMut(u32, u32),
    ) -> bool {
        self.reset(lines);
        (self.swaps, self.steps) = (0, 0);
        for (i, l) in lines.iter().enumerate() {
            let (top, bottom) = (pts[l.start as usize], pts[l.start as usize + 1]);
            self.events.push((top.y, ENTER | i as u32));
            if bottom.y > top.y {
                self.events.push((bottom.y, LEAVE | i as u32));
            }
        }
        self.events.sort_unstable_by_key(|&(y, e)| (height_key(y), e));
        windings.clear();
        let (mut k, mut q, mut y) = (0, 0, f64::NEG_INFINITY);
        loop {
            self.wind(lines, |_, _, _, _| true);
            self.swaps += core::mem::take(&mut self.steps);
            for t in 0..self.touched.len() {
                let i = self.touched[t];
                if self.alive[i as usize] && !self.meet(pts, lines, i, y, &mut pair) {
                    return false;
                }
            }
            self.tests(pts, lines, y);
            if self.swaps > self.allowed {
                return false;
            }
            let next = self.events.get(k).map(|e| e.0);
            let ahead = self.ahead(next);
            // A query at an event's height is read before it: a line counts below its top and down to its bottom.
            if let Some(&query) = queries.get(q).filter(|p| ahead.is_none_or(|a| p.0 <= a)) {
                windings.push(self.winding_at(pts, lines, query));
                q += 1;
                continue;
            }
            let Some(at) = ahead else { return true };
            y = at;
            if next == Some(y) {
                let from = k;
                while self.events.get(k).is_some_and(|e| e.0 == y) {
                    k += 1;
                }
                // A level segment meets every line across its span: those leaving there before they leave, those
                // entering after they enter, and the level ones beside it.
                self.levels.clear();
                for &(_, e) in &self.events[from..k] {
                    let l = lines[(e & LINE) as usize];
                    let (a, b) = (pts[l.start as usize], pts[l.start as usize + 1]);
                    if e & ENTER != 0 && b.y == y {
                        self.levels.push((a.x.min(b.x), a.x.max(b.x), l.start));
                    }
                }
                if !self.level(pts, lines, y, &mut pair) {
                    return false;
                }
                for t in from..k {
                    let e = self.events[t].1;
                    let i = e & LINE;
                    if e & ENTER == 0 {
                        if !self.meet(pts, lines, i, y, &mut pair) {
                            return false;
                        }
                        self.leave(i);
                    } else if pts[lines[i as usize].start as usize + 1].y > y {
                        self.enter(pts, lines, i, y);
                    }
                }
                if !self.level(pts, lines, y, &mut pair) {
                    return false;
                }
                self.levels.sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
                for (n, &(_, hi, a)) in self.levels.iter().enumerate() {
                    for &(_, _, b) in self.levels[n + 1..].iter().take_while(|l| l.0 <= hi) {
                        pair(a, b);
                        self.swaps += 1;
                        if self.swaps > self.allowed {
                            return false;
                        }
                    }
                }
                continue;
            }
            self.cross();
        }
    }
}

impl Sweep {
    // Each level segment at y paired with every line its span reaches there; false once the swaps pass `allowed`.
    fn level(&mut self, pts: &[Pt], lines: &[Chain], y: f64, pair: &mut impl FnMut(u32, u32)) -> bool {
        let (mut order, levels) = (core::mem::take(&mut self.order), core::mem::take(&mut self.levels));
        let mut within = true;
        'levels: for &(lo, hi, l) in &levels {
            let mut at = order.locate(|j| self.key(pts, lines, j, y).0 < lo);
            at = match at { Some(j) => order.beside(j, 1), None => order.first() };
            while let Some(j) = at.filter(|&j| self.key(pts, lines, j, y).0 <= hi) {
                pair(l, self.at[j as usize]);
                self.swaps += 1;
                if self.swaps > self.allowed {
                    within = false;
                    break 'levels;
                }
                at = order.beside(j, 1);
            }
        }
        (self.order, self.levels) = (order, levels);
        within
    }
}

impl Order {
    fn first(&mut self) -> Option<u32> {
        let mut at = self.root;
        if at == NIL {
            return None;
        }
        while self.kids[at as usize][0] != NIL {
            at = self.kids[at as usize][0];
        }
        self.splay(at);
        Some(self.line[at as usize])
    }
}
