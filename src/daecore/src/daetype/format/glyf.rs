use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use super::super::decoder::{read_i16_be, read_u16_be, read_u24_be, read_u32_be};
use crate::daecore::daetype::TableBytes;

pub(crate) const ARG_1_AND_2_ARE_WORDS:     u16 = 0x0001;
pub(crate) const ARGS_ARE_XY_VALUES:        u16 = 0x0002;
pub(crate) const ROUND_XY_TO_GRID:          u16 = 0x0004;
pub(crate) const WE_HAVE_A_SCALE:           u16 = 0x0008;
pub(crate) const MORE_COMPONENTS:           u16 = 0x0020;
pub(crate) const WE_HAVE_AN_X_AND_Y_SCALE:  u16 = 0x0040;
pub(crate) const WE_HAVE_A_TWO_BY_TWO:      u16 = 0x0080;
pub(crate) const WE_HAVE_INSTRUCTIONS:      u16 = 0x0100;
pub(crate) const USE_MY_METRICS:            u16 = 0x0200;
pub(crate) const SCALED_COMPONENT_OFFSET:   u16 = 0x0800;
pub(crate) const UNSCALED_COMPONENT_OFFSET: u16 = 0x1000;
pub(crate) const GID_IS_24BIT:              u16 = 0x2000;
pub(crate) const OVERLAP_COMPOUND:          u16 = 0x0400;
// A simple glyph's, on its first point.
pub(crate) const OVERLAP_SIMPLE:            u8 = 0x40;

// The bounds every walk over a glyph keeps, the glyph failing past either: OpenType numbers its points (phantoms
// counted) and contours in 16 bits, in endPtsOfContours, a component's point arguments and gvar's packed point numbers.
pub(crate) const MAX_POINTS: usize = 0xFFFF;
pub(crate) const MAX_CONTOURS: usize = 0xFFFF;
// One glyph reads 65,535 glyphs at most, as many as its 16-bit point numbers can name (maxp's maxComponentElements).
pub(crate) const MAX_VISITS: usize = 0xFFFF;
// The work one glyph's draw may do: the largest glyph the spec allows, 4,095 tuples each reading 65,535 bytes and
// moving and inferring 65,535 points at 65,535 axes, takes under 2^30, and a hostile draw within it a few seconds.
pub(crate) const GLYPH_WORK: usize = 1 << 30;

// Whether a simple glyph of `points` and `contours` fits beside the points and contours a glyph has `gathered`.
pub(crate) fn drawn_fits(gathered: (usize, usize), points: usize, contours: usize) -> bool {
    gathered.1 + contours <= MAX_CONTOURS && gathered.0 + points + 4 <= MAX_POINTS
}

// loca read where it lies, as a font's cache keeps it: an entry past its bytes reads 0, as `parse_loca` reads it.
#[derive(Clone)]
pub struct GlyfLoca {
    bytes: TableBytes,
    long: bool,
    entries: usize,
}

impl GlyfLoca {
    // `entries` offsets from `bytes`, short at indexToLocFormat 0 and long at any other.
    pub(crate) fn new(bytes: TableBytes, format: i16, entries: usize) -> GlyfLoca {
        GlyfLoca { bytes, long: format != 0, entries }
    }

    pub fn len(&self) -> usize {
        self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries == 0
    }

    pub fn get(&self, i: usize) -> Option<usize> {
        if i >= self.entries {
            return None;
        }
        Some(match self.long {
            true => read_u32_be(&self.bytes, i * 4).map_or(0, |v| v as usize),
            false => read_u16_be(&self.bytes, i * 2).map_or(0, |v| usize::from(v) * 2),
        })
    }
}

// Where each glyph's data lies in glyf: offsets the instancer built, or loca as a font's cache reads it.
#[derive(Clone, Copy)]
pub enum LocaRef<'a> {
    Offsets(&'a [usize]),
    Table(&'a GlyfLoca),
}

impl LocaRef<'_> {
    pub fn len(self) -> usize {
        match self {
            LocaRef::Offsets(offsets) => offsets.len(),
            LocaRef::Table(loca) => loca.len(),
        }
    }

    pub fn get(self, i: usize) -> Option<usize> {
        match self {
            LocaRef::Offsets(offsets) => offsets.get(i).copied(),
            LocaRef::Table(loca) => loca.get(i),
        }
    }
}

impl<'a> From<&'a [usize]> for LocaRef<'a> {
    fn from(offsets: &'a [usize]) -> LocaRef<'a> {
        LocaRef::Offsets(offsets)
    }
}

impl<'a> From<&'a Vec<usize>> for LocaRef<'a> {
    fn from(offsets: &'a Vec<usize>) -> LocaRef<'a> {
        LocaRef::Offsets(offsets)
    }
}

impl<'a> From<&'a GlyfLoca> for LocaRef<'a> {
    fn from(loca: &'a GlyfLoca) -> LocaRef<'a> {
        LocaRef::Table(loca)
    }
}

// A glyph's bytes as they're drawn and hinted: a range running backward or past glyf's end holds nothing, so the glyph
// is empty, as a subset writes it. None past loca.
pub(crate) fn glyph_bytes<'a>(glyf: &'a [u8], loca: LocaRef<'_>, gid: u32) -> Option<&'a [u8]> {
    let gid = gid as usize;
    Some(glyf.get(loca.get(gid)?..loca.get(gid + 1)?).unwrap_or_default())
}

// A glyph's bytes as they're shaped, measured, subset and instanced: None for a range outside glyf, running backward
// or past loca, an empty glyph with no metrics of its own.
pub(crate) fn glyph_record<'a>(glyf: &'a [u8], loca: LocaRef<'_>, gid: u32) -> Option<&'a [u8]> {
    let gid = gid as usize;
    glyf.get(loca.get(gid)?..loca.get(gid + 1)?)
}

// Whether a glyph carries an overlap flag: OVERLAP_SIMPLE on it or a glyph it's built from, or OVERLAP_COMPOUND on
// any component, not only the first the spec asks for. Each glyph met spends a visit, and once they're spent the walk
// reads on only to the next component's flag.
pub(crate) fn overlap_flagged(glyf: &[u8], loca: LocaRef<'_>, gid: u32, extended: bool, visits: &mut usize) -> bool {
    let mut inside: Stack<Components<'_>, 8> = Stack::default();
    let mut next = Some(gid);
    loop {
        let mut entered = false;
        if let Some(gid) = next.take() {
            let Some(left) = visits.checked_sub(1) else { return false };
            *visits = left;
            let glyph = glyph_bytes(glyf, loca, gid).unwrap_or_default();
            match read_i16_be(glyph, 0) {
                Some(..=-1) => {
                    inside.push(Components::new(glyph, extended));
                    entered = true;
                }
                Some(n @ 1..) => {
                    let flags = 12 + 2 * n as usize;
                    let first = read_u16_be(glyph, flags - 2).and_then(|len| glyph.get(flags + usize::from(len)));
                    if first.is_some_and(|f| f & OVERLAP_SIMPLE != 0) {
                        return true;
                    }
                }
                _ => {}
            }
        }
        if !entered && *visits == 0 {
            return false;
        }
        let Some(components) = inside.last_mut() else { return false };
        match components.next() {
            Some(Ok(c)) if c.flags & OVERLAP_COMPOUND != 0 => return true,
            Some(Ok(c)) => next = Some(c.gid),
            _ => drop(inside.pop()),
        }
    }
}

// A simple glyph's point count from its last end point, None where its end points or instruction length don't read
// or it has fewer points than contours.
pub(crate) fn simple_point_count(glyph: &[u8], n_contours: usize) -> Option<usize> {
    read_u16_be(glyph, 10 + 2 * n_contours)?;
    let count = usize::from(read_u16_be(glyph, 8 + 2 * n_contours)?) + 1;
    (count >= n_contours).then_some(count)
}

// A glyph as a subset copies it, trimmed: a simple glyph cut after its last coordinate, and empty where its
// instruction length is its last field or its flags don't count its points exactly; any other as stored.
pub(crate) fn trimmed(glyph: &[u8]) -> &[u8] {
    let Some(contours) = read_i16_be(glyph, 0).filter(|&n| n > 0 && glyph.len() >= 10) else { return glyph };
    let mut at = 10 + 2 * contours as usize;
    if at + 2 >= glyph.len() {
        return &[];
    }
    let points = usize::from(read_u16_be(glyph, at - 2).unwrap_or(0)) + 1;
    at += 2 + usize::from(read_u16_be(glyph, at).unwrap_or(0));
    let mut flags = glyph.get(at..).unwrap_or_default().iter();
    let (mut coordinates, mut seen) = (0, 0);
    while let Some(&flag) = flags.next() {
        let repeat = match flag & 8 {
            0 => 1,
            _ => match flags.next() {
                Some(&n) => usize::from(n) + 1,
                None => return &[],
            },
        };
        coordinates += usize::from(COORDINATE_BYTES[usize::from(flag & 0x3F)]) * repeat;
        seen += repeat;
        if seen >= points {
            break;
        }
    }
    if seen != points {
        return &[];
    }
    &glyph[..(glyph.len() - flags.len() + coordinates).min(glyph.len())]
}

// The bytes a flag gives its point's x and y: 1 each where short, none where the same as the last point's, else 2.
const COORDINATE_BYTES: [u8; 64] = {
    let mut bytes = [0; 64];
    let mut flag = 0;
    while flag < 64 {
        let x = if flag & 2 != 0 { 1 } else if flag & 0x10 == 0 { 2 } else { 0 };
        let y = if flag & 4 != 0 { 1 } else if flag & 0x20 == 0 { 2 } else { 0 };
        bytes[flag] = x + y;
        flag += 1;
    }
    bytes
};

// head's indexToLocFormat, 0 or 1, with glyphDataFormat 0, the formats the spec defines. None past them, where glyf
// holds no glyph.
pub(crate) fn loca_format(head: &[u8]) -> Option<i16> {
    let format = read_u16_be(head, 50)?;
    (format <= 1 && read_u16_be(head, 52).unwrap_or(0) == 0).then_some(format as i16)
}

// How many glyphs glyf holds: those loca gives an end, and no more than maxp counts.
pub(crate) fn glyf_glyph_count(loca_len: usize, format: i16, maxp: usize) -> usize {
    (loca_len / if format == 0 { 2 } else { 4 }).saturating_sub(1).min(maxp)
}

// loca for glyphs starting at `offsets`, short wherever every offset fits one, with the
// indexToLocFormat that says which. None past a long offset's reach.
pub(crate) fn loca_table(offsets: &[usize]) -> Option<(Vec<u8>, i16)> {
    if offsets.iter().all(|&o| o % 2 == 0 && o <= 0x1_FFFE) {
        return Some((offsets.iter().flat_map(|&o| ((o / 2) as u16).to_be_bytes()).collect(), 0));
    }
    let long: Option<Vec<[u8; 4]>> = offsets.iter().map(|&o| u32::try_from(o).ok().map(u32::to_be_bytes)).collect();
    Some((long?.concat(), 1))
}

// Any negative contour count is a composite, as the spec and every reader take it, not only -1.
pub(crate) fn is_composite(glyph: &[u8]) -> bool {
    read_i16_be(glyph, 0).is_some_and(|n| n < 0)
}

// One component record as stored: its glyph (24 bits in GLYF where GID_IS_24BIT says so), its arguments (an offset or
// two point numbers), its F2Dot14 2x2 (xx, yx, xy, yy; the identity if none), and where it and the next start.
#[derive(Clone, Copy)]
pub(crate) struct Component {
    pub(crate) flags: u16,
    pub(crate) gid: u32,
    pub(crate) wide: bool,
    pub(crate) args: (i32, i32),
    pub(crate) matrix: [i16; 4],
    pub(crate) at: usize,
    pub(crate) next: usize,
}

impl Component {
    pub(crate) fn has_offset(&self) -> bool {
        self.flags & ARGS_ARE_XY_VALUES != 0
    }

    pub(crate) fn has_matrix(&self) -> bool {
        self.flags & (WE_HAVE_A_SCALE | WE_HAVE_AN_X_AND_Y_SCALE | WE_HAVE_A_TWO_BY_TWO) != 0
    }

    // Its matrix as x' = x * a + y * c, y' = x * b + y * d.
    pub(crate) fn transform(&self) -> (f64, f64, f64, f64) {
        let [a, b, c, d] = self.matrix.map(super::super::decoder::f2dot14);
        (a, b, c, d)
    }

    // Its offset, through the matrix where its flags say.
    pub(crate) fn offset(&self) -> (f64, f64) {
        let (dx, dy) = (f64::from(self.args.0), f64::from(self.args.1));
        if !self.offset_through_matrix() {
            return (dx, dy);
        }
        let (a, b, c, d) = self.transform();
        (dx * a + dy * c, dx * b + dy * d)
    }

    // The bytes its glyph id takes, after its flags.
    pub(crate) fn gid_len(&self) -> usize {
        if self.wide { 3 } else { 2 }
    }

    pub(crate) fn args_len(&self) -> usize {
        if self.flags & ARG_1_AND_2_ARE_WORDS != 0 { 4 } else { 2 }
    }

    // SCALED_COMPONENT_OFFSET, unless UNSCALED_COMPONENT_OFFSET is set too, puts the offset through the
    // whole matrix, as Apple defined it, hinted or not.
    pub(crate) fn offset_through_matrix(&self) -> bool {
        self.flags & SCALED_COMPONENT_OFFSET != 0 && self.flags & UNSCALED_COMPONENT_OFFSET == 0
    }

    // Always inlined, with `next`: left to the inliner, drawing a 16-bit font took 12 to 25 instructions a glyph more.
    #[inline(always)]
    fn read(glyph: &[u8], at: usize, extended: bool) -> Option<Component> {
        let flags = read_u16_be(glyph, at)?;
        let wide = extended && flags & GID_IS_24BIT != 0;
        let gid = if wide { read_u24_be(glyph, at + 2)? } else { u32::from(read_u16_be(glyph, at + 2)?) };
        let xy = flags & ARGS_ARE_XY_VALUES != 0;
        let mut pos = at + if wide { 5 } else { 4 };
        let args = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
            let (a, b) = (read_u16_be(glyph, pos)?, read_u16_be(glyph, pos + 2)?);
            pos += 4;
            if xy { (i32::from(a as i16), i32::from(b as i16)) } else { (i32::from(a), i32::from(b)) }
        } else {
            let (a, b) = (*glyph.get(pos)?, *glyph.get(pos + 1)?);
            pos += 2;
            if xy { (i32::from(a as i8), i32::from(b as i8)) } else { (i32::from(a), i32::from(b)) }
        };
        // The scale flags in the spec's order, WE_HAVE_A_SCALE first.
        let mut f2 = || -> Option<i16> {
            let v = read_i16_be(glyph, pos)?;
            pos += 2;
            Some(v)
        };
        let matrix = if flags & WE_HAVE_A_SCALE != 0 {
            let s = f2()?;
            [s, 0, 0, s]
        } else if flags & WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            let (a, d) = (f2()?, f2()?);
            [a, 0, 0, d]
        } else if flags & WE_HAVE_A_TWO_BY_TWO != 0 {
            [f2()?, f2()?, f2()?, f2()?]
        } else {
            [0x4000, 0, 0, 0x4000]
        };
        Some(Component { flags, gid, wide, args, matrix, at, next: pos })
    }
}

pub(crate) struct Truncated;

// A composite's component records in order, after its 10-byte header, `extended` where the glyph is GLYF's. A record
// cut short ends the walk as an Err, which each reader takes by its own rule; `end` is then where the records stop.
pub(crate) struct Components<'a> {
    glyph: &'a [u8],
    at: usize,
    extended: bool,
    done: bool,
}

impl<'a> Components<'a> {
    pub(crate) fn new(glyph: &'a [u8], extended: bool) -> Components<'a> {
        Components { glyph, at: 10, extended, done: false }
    }

    pub(crate) fn end(&self) -> usize {
        self.at
    }
}

impl Iterator for Components<'_> {
    type Item = Result<Component, Truncated>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let Some(c) = Component::read(self.glyph, self.at, self.extended) else {
            self.done = true;
            return Some(Err(Truncated));
        };
        self.at = c.next;
        self.done = c.flags & MORE_COMPONENTS == 0;
        Some(Ok(c))
    }
}

// A walk's stack: the first `N` entries held in place, the rest on the heap, so a walk over real glyphs, nested a few
// deep, rarely allocates, and a chain as deep as its glyphs go needs no call stack.
pub(crate) struct Stack<T, const N: usize> {
    near: [Option<T>; N],
    far: Vec<T>,
    len: usize,
}

impl<T, const N: usize> Default for Stack<T, N> {
    fn default() -> Self {
        Stack { near: [const { None }; N], far: Vec::new(), len: 0 }
    }
}

impl<T, const N: usize> Stack<T, N> {
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub(crate) fn push(&mut self, value: T) {
        match self.len < N {
            true => self.near[self.len] = Some(value),
            false => self.far.push(value),
        }
        self.len += 1;
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        self.len = self.len.checked_sub(1)?;
        match self.len < N {
            true => self.near[self.len].take(),
            false => self.far.pop(),
        }
    }

    pub(crate) fn last_mut(&mut self) -> Option<&mut T> {
        match self.len {
            0 => None,
            n if n <= N => self.near[n - 1].as_mut(),
            _ => self.far.last_mut(),
        }
    }

    pub(crate) fn truncate(&mut self, len: usize) {
        while self.len > len {
            self.pop();
        }
    }

    // Bottom first.
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        self.near[..self.len.min(N)].iter().flatten().chain(&self.far)
    }
}

// The composites a walk is inside, outermost first. Drawn, a component naming one of them is refused; read for its
// metrics, it's passed over.
#[derive(Default)]
pub(crate) struct ComponentPath {
    gids: Stack<u32, SHALLOW>,
    // The same glyphs once there are more than a few, found in log time.
    deep: BTreeSet<u32>,
}

const SHALLOW: usize = 16;

impl ComponentPath {
    pub(crate) fn depth(&self) -> usize {
        self.gids.len()
    }

    pub(crate) fn holds(&self, gid: u32) -> bool {
        match self.gids.len() > SHALLOW {
            true => self.deep.contains(&gid),
            false => self.gids.iter().any(|&g| g == gid),
        }
    }

    // Into composite `gid`.
    pub(crate) fn enter(&mut self, gid: u32) {
        self.gids.push(gid);
        if self.gids.len() == SHALLOW + 1 {
            self.deep.extend(self.gids.iter().copied());
        } else if self.gids.len() > SHALLOW {
            self.deep.insert(gid);
        }
    }

    // Out of the innermost composite.
    pub(crate) fn leave(&mut self) {
        let Some(gid) = self.gids.pop() else { return };
        if self.gids.len() == SHALLOW {
            self.deep.clear();
        } else if self.gids.len() > SHALLOW {
            self.deep.remove(&gid);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/glyf.rs"]
mod tests;
