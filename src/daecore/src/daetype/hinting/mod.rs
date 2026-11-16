pub mod auto;
pub mod cff;
mod cleartype;
pub mod f26dot6;
mod interp;
mod state;

use alloc::vec::Vec;

use super::decoder::{glyf_tags, read_i16_be, read_u16_be};
use super::format::glyf::{
    drawn_fits, glyph_bytes, Component, ComponentPath, Components, LocaRef, Stack, GLYPH_WORK, MAX_VISITS,
    ROUND_XY_TO_GRID, USE_MY_METRICS, WE_HAVE_INSTRUCTIONS,
};
use super::format::metrics::{drawn_extents, drawn_metrics, MetricsTable};
use super::instancer::extract_points_into;
use super::outline::{moved, Deltas, Shape, Vary};
#[cfg(test)]
use super::outline::Stored;
pub use state::{HintMode, FLAG_CUBIC, FLAG_ON_CURVE};
use cleartype::ClearType;
use interp::{Interpreter, Program, Programs, Run, Skips, BASE, PER_BYTE, PER_POINT};
use state::{GraphicsState, Zone};
use crate::daecore::daetype::{TableBytes, Tables};

// A composite's program works on copies of its components' points, a few instructions a point, which nested
// composites would repeat for each level: charged as a run's work is.
const COPIED: u64 = 8;

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct HintedOutline {
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    pub flags: Vec<u8>,
    pub contour_ends: Vec<usize>,
}

// One size's bytecode state: the font and CVT programs run once, then each glyph starts from what they left.
#[derive(Clone)]
pub struct HintContext {
    vm: Interpreter,
    // Views of the font's own bytes, as are the metrics', so a context copies no table.
    fpgm: Option<TableBytes>,
    prep: Option<TableBytes>,
    twilight: Zone,
    // What the CVT program left, which each glyph program starts from: the twilight zone (empty at the origin), storage
    // and CVT, the graphics state, and whether the font declared native ClearType for its glyphs.
    base_twilight: Zone,
    base_storage: Vec<i32>,
    base_cvt: Vec<i32>,
    glyph_state: GraphicsState,
    native: bool,
    // Set by the CVT program through INSTCTRL: glyph programs are not to run at this size.
    glyphs_off: bool,
    metrics: Metrics,
    // Whether the font's glyphs are GLYF's, whose composites can name a glyph in 24 bits.
    extended: bool,
    // The zone a glyph or composite program runs in, and the hinted points as components are added to them, in
    // 26.6: kept between glyphs rather than built for each.
    zone: Zone,
    out: HintedOutline,
    // Whether hinting moved the glyph off its scaled outline (a program ran, or an offset or the origin was rounded),
    // how many glyphs it has loaded, and what working out their deltas may still spend.
    moved: bool,
    loads: usize,
    varying: usize,
}

// The four phantom points, in the order the glyf chapter adds them after a glyph's own.
type Phantoms = [(i32, i32); 4];

// A composite being loaded: its bytes, the components left, its deltas, its phantoms so far, where its points start,
// its last component's flags, and the component being loaded with where that one's points start.
struct Loading<'g> {
    glyph: &'g [u8],
    records: Components<'g>,
    deltas: Deltas,
    index: usize,
    phantoms: Phantoms,
    first: usize,
    last_flags: u16,
    loading: Option<(Component, usize)>,
}

// What the phantom points are set from: hmtx, and vmtx or its fallbacks without it.
#[derive(Clone)]
struct Metrics {
    hmtx: Option<TableBytes>,
    long_h: usize,
    vmtx: Option<(TableBytes, usize)>,
    ascender: i32,
    descender: i32,
}

impl Metrics {
    fn read(table_map: &Tables) -> Metrics {
        let (h, v) = (drawn_metrics(table_map, false), drawn_metrics(table_map, true));
        let (ascender, descender) = drawn_extents(table_map);
        Metrics {
            hmtx: h.and_then(|(_, mtx, _)| mtx.cloned()),
            long_h: h.and_then(|(.., long)| long).unwrap_or(0),
            vmtx: v.and_then(|(_, vmtx, long)| Some((vmtx.filter(|t| !t.is_empty())?.clone(), long?))),
            ascender: i32::from(ascender),
            descender: i32::from(descender),
        }
    }

    fn lookup(table: &[u8], long: usize, gid: usize) -> (i32, i32) {
        let (advance, bearing) = MetricsTable::new(table, long).pair(gid);
        (i32::from(advance), i32::from(bearing))
    }

    // The phantom points in font units per the instructing-glyphs chapter, advances as HVAR and VVAR vary them. vmtx
    // leaves the vertical origin's x open, so points 3 and 4 sit at x 0, the outline's origin.
    fn phantoms(&self, gid: usize, x_min: i32, y_max: i32, vary: &dyn Vary) -> Phantoms {
        let by = |vertical: bool, advance: i32| {
            vary.advance(gid as u32, vertical, advance as u16).map_or(advance, i32::from)
        };
        let (advance, lsb) = Self::lookup(self.hmtx.as_deref().unwrap_or_default(), self.long_h, gid);
        let (tsb, vadvance) = match &self.vmtx {
            Some((vmtx, long)) => {
                let (advance, tsb) = Self::lookup(vmtx, *long, gid);
                (tsb, by(true, advance))
            }
            None => (self.ascender - y_max, (self.ascender - self.descender).abs()),
        };
        let (left, top) = (x_min - lsb, y_max + tsb);
        [(left, 0), (left + by(false, advance), 0), (0, top), (0, top - vadvance)]
    }
}

// The buffers a thread's last context grew, lent to its next, so a context made for each glyph allocates none of
// them. A context holding more than `SPARE_BYTES` keeps them to itself, and drops them.
#[derive(Default)]
struct Spare {
    storage: Vec<i32>,
    cvt: Vec<i32>,
    functions: Vec<Option<interp::Body>>,
    idefs: Vec<(u8, interp::Body)>,
    stack: Vec<i32>,
    frames: Vec<interp::Frame>,
    skips: Skips,
    axes: Vec<i32>,
    twilight: Zone,
    base_twilight: Zone,
    base_storage: Vec<i32>,
    base_cvt: Vec<i32>,
    zone: Zone,
    out: HintedOutline,
}

#[cfg(feature = "std")]
const SPARE_BYTES: usize = 64 << 10;

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
}

// The context's buffers to the thread's spare; a thread past its locals' teardown drops them instead.
impl Drop for HintContext {
    fn drop(&mut self) {
        #[cfg(feature = "std")]
        if self.heap_bytes() <= SPARE_BYTES {
            use core::mem::take;
            let vm = &mut self.vm;
            let spare = Spare {
                storage: take(&mut vm.storage),
                cvt: take(&mut vm.cvt),
                functions: take(&mut vm.functions),
                idefs: take(&mut vm.idefs),
                stack: take(&mut vm.stack),
                frames: take(&mut vm.frames),
                skips: take(&mut vm.skips),
                axes: take(&mut vm.axes),
                twilight: take(&mut self.twilight),
                base_twilight: take(&mut self.base_twilight),
                base_storage: take(&mut self.base_storage),
                base_cvt: take(&mut self.base_cvt),
                zone: take(&mut self.zone),
                out: take(&mut self.out),
            };
            let _ = SPARE.try_with(|s| s.set(Some(spare)));
        }
    }
}

impl HintContext {
    // What the context holds on the heap, for its location's share of the instance budget.
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        let HintedOutline { x, y, flags, contour_ends } = &self.out;
        self.vm.heap_bytes() + self.twilight.heap_bytes() + self.base_twilight.heap_bytes()
            + heap_of(&self.base_storage) + heap_of(&self.base_cvt) + self.zone.heap_bytes() + heap_of(x) + heap_of(y)
            + heap_of(flags) + heap_of(contour_ends)
    }

    #[cfg(test)]
    pub fn new(table_map: &Tables, size: f26dot6::Size, upm: u16) -> Option<HintContext> {
        Self::new_at(table_map, size, upm, None, None)
    }

    // A context at a location: its cvt values in font units where they vary, and GETVARIATION's coordinates. None
    // where the font or CVT program fails, which leaves the size unhinted.
    pub(crate) fn new_at(
        table_map: &Tables,
        size: f26dot6::Size,
        upm: u16,
        cvt: Option<&[i32]>,
        coords: Option<&[i32]>,
    ) -> Option<HintContext> {
        if upm == 0 {
            return None;
        }
        let bytes = |tag: &str| table_map.get(tag).map_or(&[][..], |t| &t[..]);
        // maxp's fields as drawing reads them: MAXP's wherever it's there, a byte on past its 24-bit count.
        let (maxp, shift) = match table_map.get("MAXP") {
            Some(maxp) => (maxp, 1),
            None => (table_map.get("maxp")?, 0),
        };
        if maxp.len() < 32 + shift {
            return None;
        }
        let field = |at: usize| read_u16_be(maxp, at + shift).map_or(0, usize::from);
        let (twilight_points, storage_slots) = (field(16), field(18));
        let (idef_limit, max_instructions) = (field(22), field(26));
        let scale16 = size.scale16(upm);
        let axis_count = read_u16_be(bytes("fvar"), 8).map_or(0, usize::from);

        let Spare {
            mut storage, cvt: mut scaled, mut functions, mut idefs, mut stack, mut frames, mut skips, mut axes,
            mut twilight, base_twilight, base_storage, mut base_cvt, zone, out,
        } = Spare::take();
        storage.clear();
        storage.resize(storage_slots, 0);
        scaled.clear();
        match cvt {
            Some(cvt) => scaled.extend(cvt.iter().map(|&v| f26dot6::scale_units(v, scale16))),
            None => scaled.extend(bytes("cvt ").as_chunks::<2>().0.iter().map(|c| {
                f26dot6::scale_units(i32::from(i16::from_be_bytes(*c)), scale16)
            })),
        }
        // The CVT program starts from the CVT as scaled, whatever the font program wrote to it.
        base_cvt.clone_from(&scaled);
        axes.clear();
        match coords {
            Some(coords) => axes.extend_from_slice(coords),
            None => axes.resize(axis_count, 0),
        }
        twilight.reset_twilight(twilight_points);
        functions.clear();
        idefs.clear();
        stack.clear();
        frames.clear();
        skips.clear();

        let mut ctx = HintContext {
            vm: Interpreter {
                size,
                scale16,
                variable: axis_count > 0,
                symmetric: symmetric_smoothing(bytes("gasp"), max_instructions, size.ppem()),
                axes,
                storage,
                cvt: scaled,
                functions,
                idefs,
                idef_limit,
                stack,
                frames,
                skips,
                dirty: interp::Dirty::default(),
                left: 0,
            },
            fpgm: table_map.get("fpgm").cloned(),
            prep: table_map.get("prep").cloned(),
            twilight,
            base_twilight,
            base_storage,
            base_cvt,
            glyph_state: GraphicsState::default(),
            native: false,
            glyphs_off: false,
            metrics: Metrics::read(table_map),
            extended: glyf_tags(table_map).extended,
            zone,
            out,
            moved: false,
            loads: 0,
            varying: 0,
        };
        ctx.run_size_programs()?;
        Some(ctx)
    }

    fn programs<'p>(fpgm: &'p Option<TableBytes>, prep: &'p Option<TableBytes>, glyph: &'p [u8]) -> Programs<'p> {
        let font = fpgm.as_deref().unwrap_or_default();
        Programs { font, control_value: prep.as_deref().unwrap_or_default(), glyph }
    }

    // The font program, then the CVT program from a clean slate (default graphics state, zeroed storage, the CVT as
    // scaled), each with a budget of its own. `ClearType` governs glyph programs; these move twilight points freely.
    fn run_size_programs(&mut self) -> Option<()> {
        let mut no_glyph = Zone::default();
        let size_run = |program, state| Run { program, state, rules: ClearType::new(true), composite: false };
        let code = [&self.fpgm, &self.prep].map(|t| t.as_ref().map_or(0, |t| t.len()));
        // The CVT counts by its bytes, as the programs do: maxp bounds storage and twilight to 16 bits, not the CVT.
        let bytes = code[0] + code[1] + 2 * self.vm.cvt.len();
        let values = self.vm.storage.len() + self.twilight.len();
        let budget = BASE + PER_BYTE * bytes as u64 + PER_POINT * values as u64;
        let programs = Self::programs(&self.fpgm, &self.prep, &[]);
        let mut state = GraphicsState::default();
        self.vm.left = budget;
        self.vm.run(programs, size_run(Program::Font, &mut state), &mut self.twilight, &mut no_glyph).ok()?;
        if self.vm.dirty.storage {
            self.vm.storage.fill(0);
        }
        if self.vm.dirty.cvt {
            self.vm.cvt.clone_from(&self.base_cvt);
        }
        let programs = Self::programs(&self.fpgm, &self.prep, &[]);
        let mut state = GraphicsState::default();
        self.vm.left = budget;
        let run = size_run(Program::ControlValue, &mut state);
        let control = self.vm.run(programs, run, &mut self.twilight, &mut no_glyph).ok()?;
        // INSTCTRL's selector 2 restores glyphs' defaults and, per Microsoft's ClearType page, cancels selector 3.
        self.glyphs_off = control.glyphs_off;
        self.glyph_state = if control.glyph_defaults { GraphicsState::default() } else { state.for_glyphs() };
        self.native = control.native && !control.glyph_defaults;
        self.base_storage.clone_from(&self.vm.storage);
        self.base_cvt.clone_from(&self.vm.cvt);
        // No program pointed at the twilight zone, still at the origin: kept empty, and a glyph writing it resets it.
        if self.vm.dirty.twilight {
            self.base_twilight.clone_from(&self.twilight);
        } else {
            self.base_twilight.reset_twilight(0);
        }
        self.vm.dirty = interp::Dirty::default();
        Some(())
    }

    // A glyph hinted at the context's size, in its own outline until the next glyph. None where hinting leaves it as
    // scaled (no program runs and nothing rounds, or the font turned programs off), or it fails to decode or scale.
    #[cfg(test)]
    pub fn hint_glyph(&mut self, glyf: &[u8], loca: &[usize], gid: u32) -> Option<&HintedOutline> {
        self.hint_glyph_varied(glyf, loca.into(), gid, &Stored).map(|out| &*out)
    }

    // As `hint_glyph`, each glyph's points and advances moved as `vary` moves them, in the context's own outline,
    // which a caller may draw or take.
    pub(crate) fn hint_glyph_varied(
        &mut self, glyf: &[u8], loca: LocaRef<'_>, gid: u32, vary: &dyn Vary,
    ) -> Option<&mut HintedOutline> {
        if self.glyphs_off {
            return None;
        }
        // A glyph program's writes are its own; the next glyph starts from what the CVT program left.
        let dirty = core::mem::take(&mut self.vm.dirty);
        if dirty.storage {
            self.vm.storage.clone_from(&self.base_storage);
        }
        if dirty.cvt {
            self.vm.cvt.clone_from(&self.base_cvt);
        }
        if dirty.twilight && self.base_twilight.len() == 0 {
            self.twilight.reset_twilight(self.twilight.len());
        } else if dirty.twilight {
            self.twilight.clone_from(&self.base_twilight);
        }
        // Its deltas are worked out whole and in 26.6, each as drawing works them out.
        (self.moved, self.loads, self.varying, self.vm.left) = (false, 0, 2 * GLYPH_WORK, BASE);
        let mut out = core::mem::take(&mut self.out);
        out.x.clear();
        out.y.clear();
        out.flags.clear();
        out.contour_ends.clear();
        let loaded = self.load(glyf, loca, gid, &mut out, vary);
        self.out = out;
        let [(origin, _), ..] = loaded?;
        if !self.moved {
            return None;
        }
        // Drawn from the left side bearing point, as an unhinted outline is drawn from its origin.
        self.out.x.iter_mut().for_each(|x| *x = x.saturating_sub(origin));
        // A composite's program saw GLYF's cubic bit on every point; the outline marks only controls.
        if self.extended {
            self.out.flags.iter_mut().filter(|f| **f & FLAG_ON_CURVE != 0).for_each(|f| *f &= !FLAG_CUBIC);
        }
        Some(&mut self.out)
    }

    // Loads one glyph and its components, as many as drawing reads, hinting each as it goes, composites walked on the
    // heap. Returns its phantom points, hinted.
    fn load(
        &mut self, glyf: &[u8], loca: LocaRef<'_>, gid: u32, out: &mut HintedOutline, vary: &dyn Vary,
    ) -> Option<Phantoms> {
        let (mut inside, mut path) = (Stack::<Loading, 8>::default(), ComponentPath::default());
        let mut gid = gid;
        loop {
            self.loads += 1;
            if self.loads > MAX_VISITS || path.holds(gid) {
                return None;
            }
            let glyph = glyph_bytes(glyf, loca, gid)?;
            let contours = if glyph.is_empty() { 0 } else { read_i16_be(glyph.get(..10)?, 0)? };
            let (x_min, y_max) = match contours {
                0 => (0, 0),
                _ => (i32::from(read_i16_be(glyph, 2)?), i32::from(read_i16_be(glyph, 8)?)),
            };
            let units = self.metrics.phantoms(gid as usize, x_min, y_max, vary);
            let mut loaded = None;
            if contours > 0 {
                loaded = Some(self.load_simple(glyph, gid, contours as usize, units, out, vary)?);
            } else {
                let shape = if contours == 0 { Shape::Empty } else { Shape::Composite(glyph) };
                let deltas = vary.deltas(gid, shape, &mut self.varying).ok()?;
                let units = moved_phantoms(units, deltas.as_deref());
                let s = self.vm.scale16;
                let phantoms = units.map(|(x, y)| (f26dot6::scale_units(x, s), f26dot6::scale_units(y, s)));
                if contours == 0 {
                    loaded = Some(phantoms);
                } else {
                    path.enter(gid);
                    let records = Components::new(glyph, self.extended);
                    let (first, loading) = (out.x.len(), None);
                    inside.push(Loading { glyph, records, deltas, index: 0, phantoms, first, last_flags: 0, loading });
                }
            }
            // Each glyph loaded whole placed in its composite, a composite done once its own program has run.
            gid = loop {
                if let Some(theirs) = loaded.take() {
                    let Some(composite) = inside.last_mut() else { return Some(theirs) };
                    let (c, base) = composite.loading?;
                    // USE_MY_METRICS: the composite's phantom points are the component's, hinted.
                    if c.flags & USE_MY_METRICS != 0 {
                        composite.phantoms = theirs;
                    }
                    if out.x.len() > base {
                        self.place(&c, composite.first, base, out)?;
                    }
                    composite.last_flags = c.flags;
                }
                let composite = inside.last_mut()?;
                if let Some(c) = composite.records.next() {
                    let c = moved(c.ok()?, composite.deltas.as_deref(), composite.index);
                    (composite.index, composite.loading) = (composite.index + 1, Some((c, out.x.len())));
                    break c.gid;
                }
                let done = inside.pop()?;
                path.leave();
                let end = done.records.end();
                let program = if done.last_flags & WE_HAVE_INSTRUCTIONS != 0 {
                    let n = usize::from(read_u16_be(done.glyph, end)?);
                    done.glyph.get(end + 2..end + 2 + n)?
                } else {
                    &[]
                };
                let points = (out.x.len() - done.first) as u64;
                loaded = Some(match program.is_empty() || points == 0 || !self.vm.spend(COPIED * points) {
                    true => done.phantoms,
                    false => self.hint_composite(program, done.first, done.phantoms, out),
                });
            };
        }
    }

    // A composite's own program, over its assembled components: their hinted points are its unscaled and original
    // ones, at a scale of one.
    fn hint_composite(
        &mut self, program: &[u8], first: usize, phantoms: Phantoms, out: &mut HintedOutline,
    ) -> Phantoms {
        let zone = &mut self.zone;
        zone.current.clear();
        zone.current.x.extend(out.x[first..].iter().copied().chain(phantoms.map(|p| p.0)));
        zone.current.y.extend(out.y[first..].iter().copied().chain(phantoms.map(|p| p.1)));
        zone.flags.clear();
        zone.flags.extend(out.flags[first..].iter().copied().chain([0; 4]));
        let ends = &out.contour_ends;
        let first_contour = ends.iter().position(|&e| e >= first).unwrap_or(ends.len());
        zone.ends.clear();
        zone.ends.extend(ends[first_contour..].iter().map(|&e| e - first));
        zone.unscaled.assign(&zone.current);
        let hinted = self.hint_zone(program, true);
        let (zone, n) = (&self.zone, out.x.len() - first);
        out.x[first..].copy_from_slice(&zone.current.x[..n]);
        out.y[first..].copy_from_slice(&zone.current.y[..n]);
        let kept = FLAG_ON_CURVE | FLAG_CUBIC;
        out.flags[first..].iter_mut().zip(&zone.flags[..n]).for_each(|(o, f)| *o = f & kept);
        hinted
    }

    // A simple glyph decoded, varied, scaled and hinted onto `out`. At a location its points scale from 26.6, the
    // deltas unrounded, while its unscaled points take them rounded.
    fn load_simple(
        &mut self, glyph: &[u8], gid: u32, contours: usize, units: Phantoms, out: &mut HintedOutline,
        vary: &dyn Vary,
    ) -> Option<Phantoms> {
        // Instructions bounded by the glyph's own bytes; a length past them is refused.
        let at = 10 + 2 * contours;
        let len = usize::from(read_u16_be(glyph, at)?);
        let program = glyph.get(at + 2..at + 2 + len)?;
        // A glyph alone with no program, in backward compatibility, moves nowhere: nothing to decode.
        if program.is_empty() && self.loads == 1 && !self.native {
            return None;
        }
        let zone = &mut self.zone;
        if !extract_points_into(glyph, contours, &mut zone.unscaled.x, &mut zone.unscaled.y, &mut zone.flags,
            &mut zone.ends)
        {
            return None;
        }
        let n = zone.unscaled.x.len();
        let shape = || Shape::Simple { x: &zone.unscaled.x, y: &zone.unscaled.y, ends: &zone.ends };
        let deltas = vary.deltas(gid, shape(), &mut self.varying).ok()?;
        let fine = match vary.located() {
            true => Some(vary.deltas_f26dot6(gid, shape(), &mut self.varying).ok()?),
            false => None,
        };
        if !drawn_fits((out.x.len(), out.contour_ends.len()), n, contours) {
            return None;
        }
        // The glyph's programs may spend more for each point the glyph gathers.
        self.vm.left = self.vm.left.saturating_add(PER_POINT * n as u64);
        let s = self.vm.scale16;
        // GLYF's cubic bit, 0x80, kept as FLAG_CUBIC on a point off the curve, so a hinted outline still draws its
        // cubics; the draft holds it 0 on the curve, so it means nothing there.
        let extended = self.extended;
        let cubic = |f: u8| extended && f & (0x80 | FLAG_ON_CURVE) == 0x80;
        zone.flags.iter_mut().for_each(|f| *f = *f & FLAG_ON_CURVE | if cubic(*f) { FLAG_CUBIC } else { 0 });
        zone.flags.extend([0; 4]);
        zone.unscaled.x.extend(units.map(|p| p.0));
        zone.unscaled.y.extend(units.map(|p| p.1));
        // At a location, scaled from the points before the rounded deltas move them, each with its delta unrounded.
        zone.current.clear();
        if let Some(fine) = fine.as_ref().map(Option::as_deref) {
            let delta = |i: usize| fine.map_or((0, 0), |d| d[i]);
            let scale = |v: i32, d: i32| f26dot6::scale_f26dot6(v.saturating_mul(64).saturating_add(d), s);
            let unscaled = &zone.unscaled;
            zone.current.x.extend(unscaled.x.iter().enumerate().map(|(i, &x)| scale(x, delta(i).0)));
            zone.current.y.extend(unscaled.y.iter().enumerate().map(|(i, &y)| scale(y, delta(i).1)));
        }
        if let Some(d) = deltas.as_deref() {
            let unscaled = &mut zone.unscaled;
            for ((x, y), &(dx, dy)) in unscaled.x.iter_mut().zip(&mut unscaled.y).zip(&d[..n + 4]) {
                (*x, *y) = (x.saturating_add(i32::from(dx)), y.saturating_add(i32::from(dy)));
            }
        }
        if !f26dot6::fits_scale(zone.unscaled.x.iter().chain(&zone.unscaled.y).copied(), s) {
            return None;
        }
        if fine.is_none() {
            zone.current.x.extend(zone.unscaled.x.iter().map(|&x| f26dot6::scale_fitting(x, s)));
            zone.current.y.extend(zone.unscaled.y.iter().map(|&y| f26dot6::scale_fitting(y, s)));
        }
        let hinted = self.hint_zone(program, false);

        let zone = &self.zone;
        let offset = out.x.len();
        out.x.extend_from_slice(&zone.current.x[..n]);
        out.y.extend_from_slice(&zone.current.y[..n]);
        out.flags.extend(zone.flags[..n].iter().map(|f| f & (FLAG_ON_CURVE | FLAG_CUBIC)));
        out.contour_ends.extend(zone.ends.iter().map(|&e| e + offset));
        Some(hinted)
    }

    // Hints the zone, phantom points rounded where the rules let them move (y always, x when native), then returns
    // them as hinted. A program that stops keeps what it did.
    fn hint_zone(&mut self, program: &[u8], composite: bool) -> Phantoms {
        let zone = &mut self.zone;
        let n = zone.len() - 4;
        zone.original.assign(&zone.current);
        let origin = zone.current.x[n];
        if self.native {
            zone.current.x[n..n + 2].iter_mut().for_each(|x| *x = f26dot6::round_pixel(*x));
        }
        zone.current.y[n + 2..].iter_mut().for_each(|y| *y = f26dot6::round_pixel(*y));
        if program.is_empty() {
            self.moved |= zone.current.x[n] != origin;
        } else {
            self.moved = true;
            let mut state = self.glyph_state;
            let run = Run { program: Program::Glyph, state: &mut state, rules: ClearType::new(self.native), composite };
            let programs = Self::programs(&self.fpgm, &self.prep, program);
            let _ = self.vm.run(programs, run, &mut self.twilight, &mut self.zone);
        }
        let c = &self.zone.current;
        [0, 1, 2, 3].map(|k| (c.x[n + k], c.y[n + k]))
    }

    // Places a component: the new points transformed, then moved by the offset, or by whatever lines its point up
    // with one before it.
    fn place(&mut self, c: &Component, first: usize, base: usize, out: &mut HintedOutline) -> Option<()> {
        let [xx, yx, xy, yy] = c.matrix.map(|v| i32::from(v) * 4);
        let transform = |x: i32, y: i32| {
            (
                f26dot6::times_16_16(x, xx).saturating_add(f26dot6::times_16_16(y, xy)),
                f26dot6::times_16_16(x, yx).saturating_add(f26dot6::times_16_16(y, yy)),
            )
        };
        if c.has_matrix() {
            for i in base..out.x.len() {
                (out.x[i], out.y[i]) = transform(out.x[i], out.y[i]);
            }
        }
        let (dx, dy) = if !c.has_offset() {
            let (k, l) = (first + usize::try_from(c.args.0).ok()?, base + usize::try_from(c.args.1).ok()?);
            if k >= base || l >= out.x.len() {
                return None;
            }
            (out.x[k].saturating_sub(out.x[l]), out.y[k].saturating_sub(out.y[l]))
        } else {
            if c.args == (0, 0) {
                return Some(());
            }
            let s = self.vm.scale16;
            let (mut x, mut y) = (f26dot6::scale_units(c.args.0, s), f26dot6::scale_units(c.args.1, s));
            // Through the whole matrix, as unhinted outlines place it, so a rotated or mirrored component lands where
            // its outline does.
            if c.offset_through_matrix() {
                (x, y) = transform(x, y);
            }
            // To the grid as asked, along x only in a font declaring native ClearType, as x otherwise stays as drawn.
            if c.flags & ROUND_XY_TO_GRID != 0 {
                let rounded = (if self.native { f26dot6::round_pixel(x) } else { x }, f26dot6::round_pixel(y));
                self.moved |= rounded != (x, y);
                (x, y) = rounded;
            }
            (x, y)
        };
        if dx != 0 || dy != 0 {
            for i in base..out.x.len() {
                out.x[i] = out.x[i].saturating_add(dx);
                out.y[i] = out.y[i].saturating_add(dy);
            }
        }
        Some(())
    }
}

// A version 1 gasp range's symmetric smoothing flag where one covers the size; else smoothing for a face without glyph
// bytecode (maxp's maxSizeOfInstructions 0) or past 15 ppem, where gasp's spec says the flag is typically set.
fn symmetric_smoothing(gasp: &[u8], max_instructions: usize, ppem: u16) -> bool {
    let ranges = read_u16_be(gasp, 2).map_or(0, usize::from);
    let ranges = match read_u16_be(gasp, 0) {
        Some(1) => gasp.get(4..4 + 4 * ranges).unwrap_or_default(),
        _ => &[],
    };
    let range = ranges.as_chunks::<4>().0.iter().find(|r| ppem <= u16::from_be_bytes([r[0], r[1]]));
    range.map_or(max_instructions == 0 || ppem > 15, |r| r[3] & 8 != 0)
}

// Phantom points moved by a glyph's last four deltas.
fn moved_phantoms(units: Phantoms, deltas: Option<&[(i16, i16)]>) -> Phantoms {
    let Some(d) = deltas else { return units };
    let d = &d[d.len() - 4..];
    core::array::from_fn(|k| (units[k].0 + i32::from(d[k].0), units[k].1 + i32::from(d[k].1)))
}

pub fn draw_hinted(out: &HintedOutline, pen: &mut (impl super::outline::OutlinePen + ?Sized)) {
    let len = out.x.len().min(out.y.len()).min(out.flags.len());
    let mut start = 0usize;
    for &end in &out.contour_ends {
        if end >= len || end < start {
            break;
        }
        let contour = HintedContour { out, start, end };
        let flags = &out.flags[start..=end];
        if !flags.iter().any(|f| f & FLAG_CUBIC != 0) {
            super::outline::draw_contour_over(&contour, pen);
        } else if cff_form(flags) {
            draw_cff_contour(&contour, pen);
        } else {
            super::outline::draw_cubic_contour_over(&contour, pen);
        }
        start = end + 1;
    }
}

// Whether a contour holding cubic controls is in CFF's form, as hinting leaves a CFF glyph: on the curve first, then
// points on it and pairs of controls each followed by one, or by the end. GLYF's cubic rule holds for it.
fn cff_form(flags: &[u8]) -> bool {
    if flags.first() != Some(&FLAG_ON_CURVE) {
        return false;
    }
    let mut i = 1;
    while i < flags.len() {
        match flags[i..] {
            [FLAG_ON_CURVE, ..] => i += 1,
            [FLAG_CUBIC, FLAG_CUBIC, ..] if flags.get(i + 2).is_none_or(|&f| f == FLAG_ON_CURVE) => i += 2,
            _ => return false,
        }
    }
    true
}

// A contour in CFF's form, a segment at a time, as `draw_cubic_contour_over` draws one, without checking the rule.
fn draw_cff_contour(contour: &HintedContour, pen: &mut (impl super::outline::OutlinePen + ?Sized)) {
    let HintedContour { out, start, end } = *contour;
    let px = |k: usize| (out.x[k] as f32 / f26dot6::ONE as f32, out.y[k] as f32 / f26dot6::ONE as f32);
    let (x0, y0) = px(start);
    pen.move_to(x0, y0);
    let mut k = start + 1;
    while k <= end {
        if out.flags[k] == FLAG_ON_CURVE {
            let (x, y) = px(k);
            pen.line_to(x, y);
            k += 1;
        } else {
            let ((c1x, c1y), (c2x, c2y), (x, y)) = (px(k), px(k + 1), px(if k + 1 == end { start } else { k + 2 }));
            pen.curve_to(c1x, c1y, c2x, c2y, x, y);
            k += 3;
        }
    }
    pen.close();
}

struct HintedContour<'a> {
    out: &'a HintedOutline,
    start: usize,
    end: usize,
}

impl super::outline::ContourPoints for HintedContour<'_> {
    fn len(&self) -> usize { self.end - self.start + 1 }
    fn get(&self, i: usize) -> (f32, f32, bool) {
        let k = self.start + i;
        let px = |v: i32| v as f32 / f26dot6::ONE as f32;
        (px(self.out.x[k]), px(self.out.y[k]), self.out.flags[k] & FLAG_ON_CURVE != 0)
    }
    fn cubic(&self, i: usize) -> bool { self.out.flags[self.start + i] & FLAG_CUBIC != 0 }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/hinting/mod.rs"]
mod tests;
