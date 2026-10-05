pub mod auto;
pub mod cff;
pub mod f26dot6;
mod interp;
mod state;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use super::decoder::{read_i16_be, read_u16_be};
use super::format::glyf::{
    ARGS_ARE_XY_VALUES, ARG_1_AND_2_ARE_WORDS, MORE_COMPONENTS, SCALED_COMPONENT_OFFSET, WE_HAVE_AN_X_AND_Y_SCALE,
    WE_HAVE_A_SCALE, WE_HAVE_A_TWO_BY_TWO,
};
use super::instancer::{extract_coords_into, GlyphCoords};
pub use state::{HintMode, FLAG_CUBIC, FLAG_ON_CURVE};
use interp::{Interpreter, Machine, ProgramKind, Programs, COMPAT_ON};
use state::{GraphicsState, Zone};
use crate::daecore::daetype::TableBytes;

const ROUND_XY_TO_GRID: u16 = 0x0004;
const WE_HAVE_INSTRUCTIONS: u16 = 0x0100;
const USE_MY_METRICS: u16 = 0x0200;

// Composite nesting as deep as FreeType allows, and no more points or component loads than a glyph
// could hold: a font is untrusted input, and each component runs a program of its own.
const MAX_DEPTH: usize = 100;
pub(crate) const MAX_POINTS: usize = 0xFFFF;
const MAX_LOADS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintedOutline {
    pub x: Vec<i32>,
    pub y: Vec<i32>,
    pub flags: Vec<u8>,
    pub contour_ends: Vec<usize>,
}

// One size's bytecode state: the font and CVT programs run once, then each glyph starts from what
// they left, as FreeType's TT_Size does.
pub struct HintContext {
    interp: Interpreter,
    fpgm: Vec<u8>,
    prep: Vec<u8>,
    twilight: Zone,
    prep_twilight: Zone,
    prep_storage: Vec<i32>,
    prep_cvt: Vec<i32>,
    metrics: Metrics,
    // Set by the CVT program through INSTCTRL: glyph programs are not to run at this size.
    glyphs_off: bool,
    compat: u8,
    coords: GlyphCoords,
    // The zone a glyph or composite program runs in, kept between glyphs rather than built for each.
    zone: Zone,
}

// What the phantom points are set from: hmtx, and vmtx or the fallbacks FreeType takes without it.
struct Metrics {
    hmtx: Vec<u8>,
    long_h: usize,
    vmtx: Option<(Vec<u8>, usize)>,
    ascender: i32,
    descender: i32,
}

impl Metrics {
    fn read(table_map: &BTreeMap<String, TableBytes>) -> Metrics {
        let bytes = |tag: &str| table_map.get(tag).map(TableBytes::to_owned_vec).unwrap_or_default();
        let hhea = bytes("hhea");
        let os2 = bytes("OS/2");
        let vhea = bytes("vhea");
        let vmtx = bytes("vmtx");
        let (ascender, descender) = match (read_i16_be(&os2, 68), read_i16_be(&os2, 70)) {
            (Some(a), Some(d)) => (a, d),
            _ => (read_i16_be(&hhea, 4).unwrap_or(0), read_i16_be(&hhea, 6).unwrap_or(0)),
        };
        let long_v = read_u16_be(&vhea, 34).map(usize::from);
        Metrics {
            hmtx: bytes("hmtx"),
            long_h: read_u16_be(&hhea, 34).map_or(0, usize::from),
            vmtx: long_v.filter(|_| !vmtx.is_empty()).map(|n| (vmtx, n)),
            ascender: i32::from(ascender),
            descender: i32::from(descender),
        }
    }

    // An advance and a bearing: past the long records, the last advance and the trailing bearings.
    fn lookup(table: &[u8], long: usize, gid: usize) -> (i32, i32) {
        if long == 0 {
            return (0, 0);
        }
        let advance = read_u16_be(table, 4 * gid.min(long - 1)).map_or(0, i32::from);
        let bearing = if gid < long { read_i16_be(table, 4 * gid + 2) } else { read_i16_be(table, 4 * long + 2 * (gid - long)) };
        (advance, bearing.map_or(0, i32::from))
    }

    // The four phantom points in font units: the origin and advance, and the top origin and advance
    // height, x at half the advance under v40 as FreeType places them for grayscale.
    fn phantoms(&self, gid: usize, x_min: i32, y_max: i32, v40: bool) -> [(i32, i32); 4] {
        let (advance, lsb) = Self::lookup(&self.hmtx, self.long_h, gid);
        let (tsb, vadvance) = match &self.vmtx {
            Some((vmtx, long)) => {
                let (ah, tsb) = Self::lookup(vmtx, *long, gid);
                (tsb, ah)
            }
            None => (self.ascender - y_max, (self.ascender - self.descender).abs()),
        };
        let x = if v40 { advance / 2 } else { 0 };
        let pp1 = x_min - lsb;
        let top = y_max + tsb;
        [(pp1, 0), (pp1 + advance, 0), (x, top), (x, top - vadvance)]
    }
}

// Hinted points as components are added to them, in 26.6.
#[derive(Default)]
struct Assembly {
    x: Vec<i32>,
    y: Vec<i32>,
    flags: Vec<u8>,
    ends: Vec<usize>,
    ran: bool,
    loads: usize,
}

type Phantoms = [(i32, i32); 4];

impl HintContext {
    pub fn new(
        table_map: &BTreeMap<String, TableBytes>,
        size: f26dot6::Size,
        upm: u16,
        mode: HintMode,
    ) -> Option<HintContext> {
        if !mode.runs_bytecode() || upm == 0 {
            return None;
        }
        let bytes = |tag: &str| table_map.get(tag).map(TableBytes::to_owned_vec).unwrap_or_default();
        let maxp = table_map.get("maxp")?;
        let field = |at: usize| read_u16_be(maxp, at).map_or(0, usize::from);
        if maxp.len() < 32 {
            return None;
        }
        let max_stack = field(24);
        let scale16 = size.scale16(upm);
        let fvar = bytes("fvar");
        let axes = read_u16_be(&fvar, 8).map_or(0, usize::from);

        let mut ctx = HintContext {
            interp: Interpreter {
                gs: GraphicsState::default(),
                retained: GraphicsState::default(),
                storage: alloc::vec![0; field(18)],
                cvt: scaled_cvt(&bytes("cvt "), scale16),
                functions: Vec::new(),
                max_functions: field(20),
                defined_functions: 0,
                instructions: Vec::new(),
                max_instructions: field(22),
                stack: Vec::new(),
                stack_limit: max_stack + (max_stack / 2).max(128),
                calls: Vec::new(),
                mode,
                size,
                scale16,
                compat: 0,
                variable: axes > 0,
                axes: alloc::vec![0; axes],
                num_glyphs: field(4),
            },
            fpgm: bytes("fpgm"),
            prep: bytes("prep"),
            twilight: Zone::with_len(field(16) + 4),
            prep_twilight: Zone::default(),
            prep_storage: Vec::new(),
            prep_cvt: Vec::new(),
            metrics: Metrics::read(table_map),
            glyphs_off: false,
            compat: 0,
            coords: GlyphCoords::default(),
            zone: Zone::default(),
        };
        ctx.run_font_programs(&bytes("cvt "))?;
        Some(ctx)
    }

    // The font program, then the CVT program from a clean slate (default graphics state, zeroed storage,
    // the CVT scaled afresh). A program that fails leaves the size unhinted, as FreeType's does.
    fn run_font_programs(&mut self, cvt: &[u8]) -> Option<()> {
        let mut glyph = Zone::default();
        for kind in [ProgramKind::Font, ProgramKind::ControlValue] {
            if kind == ProgramKind::ControlValue {
                self.interp.gs = GraphicsState::default();
                self.interp.storage.iter_mut().for_each(|v| *v = 0);
                self.interp.cvt = scaled_cvt(cvt, self.interp.scale16);
            }
            let programs = Programs { font: &self.fpgm, control_value: &self.prep, glyph: &[] };
            Machine::new(&mut self.interp, programs, &mut self.twilight, &mut glyph, kind, false).run().ok()?;
        }
        let control = self.interp.gs.instruct_control;
        self.glyphs_off = control & 1 != 0;
        self.interp.retained = if control & 2 != 0 { GraphicsState::default() } else { self.interp.gs.retained() };
        self.compat = if self.interp.mode.is_v40() { (self.interp.retained.instruct_control & COMPAT_ON) ^ COMPAT_ON } else { 0 };
        self.prep_storage.clone_from(&self.interp.storage);
        self.prep_cvt.clone_from(&self.interp.cvt);
        self.prep_twilight.clone_from(&self.twilight);
        Some(())
    }

    // Whether the CVT program turned glyph programs off at this size, as a font hinted up to a size
    // does past it: the glyph is to be drawn unhinted, not hinted some other way.
    pub fn glyph_programs_off(&self) -> bool {
        self.glyphs_off
    }

    // A glyph hinted at the context's size. None where no program ran (the glyph has none, or the
    // font turned them off), where it does not decode or does not scale inside the hinters' range.
    pub fn hint_glyph(&mut self, glyf: &[u8], loca: &[usize], gid: u16) -> Option<HintedOutline> {
        if self.glyphs_off {
            return None;
        }
        // A glyph program's writes are its own; the next glyph starts from what the CVT program left.
        self.interp.storage.clone_from(&self.prep_storage);
        self.interp.cvt.clone_from(&self.prep_cvt);
        self.twilight.clone_from(&self.prep_twilight);
        self.interp.compat = self.compat;
        let mut out = Assembly::default();
        let mut path = Vec::new();
        let [(pp1, _), ..] = self.load(glyf, loca, gid, &mut path, &mut out)?;
        if !out.ran {
            return None;
        }
        // The origin moved to phantom point 1, as unhinted outlines are drawn and as FreeType places
        // a hinted one: the point as hinted, or as scaled under v40's backward compatibility.
        out.x.iter_mut().for_each(|x| *x = x.saturating_sub(pp1));
        Some(HintedOutline { x: out.x, y: out.y, flags: out.flags, contour_ends: out.ends })
    }

    // FreeType's load_truetype_glyph for one glyph and its components, hinting each as it goes.
    // Returns the glyph's phantom points.
    fn load(&mut self, glyf: &[u8], loca: &[usize], gid: u16, path: &mut Vec<u16>, out: &mut Assembly) -> Option<Phantoms> {
        out.loads += 1;
        if out.loads > MAX_LOADS || path.len() > MAX_DEPTH || path.contains(&gid) {
            return None;
        }
        let g = usize::from(gid);
        let (start, end) = (*loca.get(g)?, *loca.get(g + 1)?);
        let glyph = glyf.get(start..end.max(start))?;
        let n_contours = if glyph.is_empty() { 0 } else { read_i16_be(glyph, 0)? };
        let (x_min, y_max) = if n_contours == 0 { (0, 0) } else { (i32::from(read_i16_be(glyph, 2)?), i32::from(read_i16_be(glyph, 8)?)) };
        let v40 = self.interp.mode.is_v40();
        let units = self.metrics.phantoms(g, x_min, y_max, v40);
        let scaled = units.map(|(x, y)| (f26dot6::scale_fix(x, self.interp.scale16), f26dot6::scale_fix(y, self.interp.scale16)));
        if n_contours == 0 {
            return Some(scaled);
        }
        if n_contours > 0 {
            return self.load_simple(glyph, n_contours as usize, units, out);
        }

        path.push(gid);
        let first = out.x.len();
        let mut phantoms = scaled;
        let mut pos = 10;
        let last_flags = loop {
            let c = read_component(glyph, pos)?;
            let base = out.x.len();
            let pp = self.load(glyf, loca, c.gid, path, out)?;
            if c.flags & USE_MY_METRICS != 0 {
                phantoms = pp;
            }
            if out.x.len() > base {
                self.place(&c, first, base, out)?;
            }
            pos = c.next;
            if c.flags & MORE_COMPONENTS == 0 {
                break c.flags;
            }
        };
        path.pop();

        let program = if last_flags & WE_HAVE_INSTRUCTIONS != 0 {
            let n = usize::from(read_u16_be(glyph, pos)?);
            glyph.get(pos + 2..pos + 2 + n)?
        } else {
            &[]
        };
        if program.is_empty() || out.x.len() == first {
            return Some(phantoms);
        }
        // The composite's own program refers to its hinted components: its unscaled and original
        // points are the assembled ones, at a scale of one.
        let n = out.x.len() - first;
        let mut zone = core::mem::take(&mut self.zone);
        zone.reset(n + 4);
        zone.cur_x[..n].copy_from_slice(&out.x[first..]);
        zone.cur_y[..n].copy_from_slice(&out.y[first..]);
        zone.flags[..n].copy_from_slice(&out.flags[first..]);
        for (i, &(x, y)) in phantoms.iter().enumerate() {
            (zone.cur_x[n + i], zone.cur_y[n + i]) = (x, y);
        }
        let first_contour = out.ends.iter().position(|&e| e >= first).unwrap_or(out.ends.len());
        zone.contour_ends.extend(out.ends[first_contour..].iter().map(|&e| e - first));
        zone.orus_x.clone_from(&zone.cur_x);
        zone.orus_y.clone_from(&zone.cur_y);
        let pp = self.run_glyph(&mut zone, program, true);
        out.ran = true;
        out.x[first..].copy_from_slice(&zone.cur_x[..n]);
        out.y[first..].copy_from_slice(&zone.cur_y[..n]);
        out.flags[first..].iter_mut().zip(&zone.flags[..n]).for_each(|(o, f)| *o = f & FLAG_ON_CURVE);
        self.zone = zone;
        Some(if self.interp.compat == 0 { pp } else { phantoms })
    }

    fn load_simple(&mut self, glyph: &[u8], n_contours: usize, units: Phantoms, out: &mut Assembly) -> Option<Phantoms> {
        if !extract_coords_into(glyph, 0, n_contours, &mut self.coords) {
            return None;
        }
        let n = self.coords.num_points;
        let reach = self.coords.x_coords[..n].iter().chain(&self.coords.y_coords[..n]).chain(units.iter().flat_map(|p| [&p.0, &p.1]));
        if !fits_scale(reach.copied(), self.interp.scale16) || out.x.len() + n > MAX_POINTS {
            return None;
        }
        // Instructions bounded by the glyph's own bytes, as FreeType refuses a length past them.
        let at = 10 + 2 * n_contours;
        let len = usize::from(read_u16_be(glyph, at)?);
        let program = glyph.get(at + 2..at + 2 + len)?;

        let mut zone = core::mem::take(&mut self.zone);
        zone.reset(n + 4);
        let s = self.interp.scale16;
        for i in 0..n {
            (zone.orus_x[i], zone.orus_y[i]) = (self.coords.x_coords[i], self.coords.y_coords[i]);
            zone.flags[i] = self.coords.flags[i] & FLAG_ON_CURVE;
        }
        for (i, &(x, y)) in units.iter().enumerate() {
            (zone.orus_x[n + i], zone.orus_y[n + i]) = (x, y);
        }
        for i in 0..n + 4 {
            zone.cur_x[i] = f26dot6::scale_fix(zone.orus_x[i], s);
            zone.cur_y[i] = f26dot6::scale_fix(zone.orus_y[i], s);
        }
        zone.contour_ends.clone_from(&self.coords.end_pts);
        let scaled = [0, 1, 2, 3].map(|i| (zone.cur_x[n + i], zone.cur_y[n + i]));
        let pp = if program.is_empty() {
            round_phantoms(&mut zone, n);
            [0, 1, 2, 3].map(|i| (zone.cur_x[n + i], zone.cur_y[n + i]))
        } else {
            out.ran = true;
            self.run_glyph(&mut zone, program, false)
        };

        let offset = out.x.len();
        out.x.extend_from_slice(&zone.cur_x[..n]);
        out.y.extend_from_slice(&zone.cur_y[..n]);
        out.flags.extend(zone.flags[..n].iter().map(|f| f & FLAG_ON_CURVE));
        out.ends.extend(zone.contour_ends.iter().map(|&e| e + offset));
        self.zone = zone;
        Some(if self.interp.compat == 0 { pp } else { scaled })
    }

    // FreeType's TT_Hint_Glyph: originals saved, phantom points rounded, the program run from the
    // retained graphics state. A program that stops keeps what it did, as in FreeType.
    fn run_glyph(&mut self, zone: &mut Zone, program: &[u8], is_composite: bool) -> Phantoms {
        let n = zone.len() - 4;
        zone.org_x.clone_from(&zone.cur_x);
        zone.org_y.clone_from(&zone.cur_y);
        round_phantoms(zone, n);
        self.interp.gs = self.interp.retained.clone();
        let programs = Programs { font: &self.fpgm, control_value: &self.prep, glyph: program };
        let _ = Machine::new(&mut self.interp, programs, &mut self.twilight, zone, ProgramKind::Glyph, is_composite).run();
        [0, 1, 2, 3].map(|i| (zone.cur_x[n + i], zone.cur_y[n + i]))
    }

    // FreeType's TT_Process_Composite_Component: the new points transformed, then moved by the
    // offset, or by whatever lines its point up with one before it.
    fn place(&self, c: &Component, first: usize, base: usize, out: &mut Assembly) -> Option<()> {
        let [xx, yx, xy, yy] = c.matrix;
        let scaled = c.flags & (WE_HAVE_A_SCALE | WE_HAVE_AN_X_AND_Y_SCALE | WE_HAVE_A_TWO_BY_TWO) != 0;
        if scaled {
            for i in base..out.x.len() {
                let (x, y) = (out.x[i], out.y[i]);
                out.x[i] = f26dot6::mul_fix(x, xx).saturating_add(f26dot6::mul_fix(y, xy));
                out.y[i] = f26dot6::mul_fix(x, yx).saturating_add(f26dot6::mul_fix(y, yy));
            }
        }
        let (dx, dy) = if c.flags & ARGS_ARE_XY_VALUES == 0 {
            let (k, l) = (first + usize::try_from(c.args.0).ok()?, base + usize::try_from(c.args.1).ok()?);
            if k >= base || l >= out.x.len() {
                return None;
            }
            (out.x[k].wrapping_sub(out.x[l]), out.y[k].wrapping_sub(out.y[l]))
        } else {
            let (mut x, mut y) = c.args;
            if x == 0 && y == 0 {
                return Some(());
            }
            // FreeType's guess at Apple's scaled offset, by the lengths of the matrix columns.
            if scaled && c.flags & SCALED_COMPONENT_OFFSET != 0 {
                x = f26dot6::mul_fix(x, hypot16(xx, xy));
                y = f26dot6::mul_fix(y, hypot16(yy, yx));
            }
            let s = self.interp.scale16;
            let (mut x, mut y) = (f26dot6::scale_fix(x, s), f26dot6::scale_fix(y, s));
            if c.flags & ROUND_XY_TO_GRID != 0 {
                if self.interp.compat == 0 {
                    x = pix_round(x);
                }
                y = pix_round(y);
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

fn scaled_cvt(cvt: &[u8], scale16: i64) -> Vec<i32> {
    cvt.as_chunks::<2>().0.iter().map(|c| f26dot6::scale_fix(i32::from(i16::from_be_bytes(*c)), scale16)).collect()
}

fn fits_scale(coords: impl Iterator<Item = i32>, scale16: i64) -> bool {
    let reach = coords.map(|v| i64::from(v).abs()).max().unwrap_or(0);
    (i128::from(reach) * i128::from(scale16)) >> 16 < 1 << 29
}

fn pix_round(v: i32) -> i32 {
    v.saturating_add(f26dot6::HALF) & !(f26dot6::ONE - 1)
}

fn round_phantoms(zone: &mut Zone, n: usize) {
    zone.cur_x[n] = pix_round(zone.cur_x[n]);
    zone.cur_x[n + 1] = pix_round(zone.cur_x[n + 1]);
    zone.cur_y[n + 2] = pix_round(zone.cur_y[n + 2]);
    zone.cur_y[n + 3] = pix_round(zone.cur_y[n + 3]);
}

fn hypot16(a: i32, b: i32) -> i32 {
    let (a, b) = (f64::from(a), f64::from(b));
    #[allow(unused_imports, reason = "the inherent method shadows this whenever std is linked")]
    use crate::daecore::daemachine::float::FloatExt;
    (a * a + b * b).sqrt().round() as i32
}

// A component record with its matrix in 16.16, as FreeType reads it: xx, yx, xy, yy.
struct Component {
    flags: u16,
    gid: u16,
    args: (i32, i32),
    matrix: [i32; 4],
    next: usize,
}

fn read_component(glyph: &[u8], pos: usize) -> Option<Component> {
    let flags = read_u16_be(glyph, pos)?;
    let gid = read_u16_be(glyph, pos + 2)?;
    let mut at = pos + 4;
    let xy = flags & ARGS_ARE_XY_VALUES != 0;
    let args = if flags & ARG_1_AND_2_ARE_WORDS != 0 {
        let (a, b) = (read_u16_be(glyph, at)?, read_u16_be(glyph, at + 2)?);
        at += 4;
        if xy { (i32::from(a as i16), i32::from(b as i16)) } else { (i32::from(a), i32::from(b)) }
    } else {
        let (a, b) = (*glyph.get(at)?, *glyph.get(at + 1)?);
        at += 2;
        if xy { (i32::from(a as i8), i32::from(b as i8)) } else { (i32::from(a), i32::from(b)) }
    };
    let mut f2 = || -> Option<i32> {
        let v = read_i16_be(glyph, at)?;
        at += 2;
        Some(i32::from(v) * 4)
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
        [0x10000, 0, 0, 0x10000]
    };
    Some(Component { flags, gid, args, matrix, next: at })
}

pub fn draw_hinted(out: &HintedOutline, pen: &mut dyn super::outline::OutlinePen) {
    let len = out.x.len().min(out.y.len()).min(out.flags.len());
    let mut start = 0usize;
    for &end in &out.contour_ends {
        if end >= len || end < start {
            break;
        }
        if out.flags[start..=end].iter().any(|f| f & FLAG_CUBIC != 0) {
            draw_cubic_contour(out, start, end, pen);
        } else {
            super::outline::draw_contour_over(&HintedContour { out, start, end }, pen);
        }
        start = end + 1;
    }
}

// A CFF contour: on-curve points joined by lines, or by a cubic through the two controls between
// them, the last returning to the first point where the collector dropped its closing duplicate.
fn draw_cubic_contour(out: &HintedOutline, start: usize, end: usize, pen: &mut dyn super::outline::OutlinePen) {
    let px = |k: usize| (out.x[k] as f32 / f26dot6::ONE as f32, out.y[k] as f32 / f26dot6::ONE as f32);
    let at = |i: usize| if i > end { start } else { i };
    let (x0, y0) = px(start);
    pen.move_to(x0, y0);
    let mut i = start + 1;
    while i <= end {
        if out.flags[i] & FLAG_CUBIC != 0 && i < end {
            let ((c1x, c1y), (c2x, c2y), (x, y)) = (px(i), px(i + 1), px(at(i + 2)));
            pen.curve_to(c1x, c1y, c2x, c2y, x, y);
            i += 3;
        } else {
            let (x, y) = px(i);
            pen.line_to(x, y);
            i += 1;
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(entries: &[(&str, Vec<u8>)]) -> BTreeMap<String, TableBytes> {
        entries.iter().map(|(tag, bytes)| (String::from(*tag), TableBytes::from_vec(bytes.clone()))).collect()
    }

    fn maxp() -> Vec<u8> {
        let mut maxp = alloc::vec![0u8; 32];
        maxp[18..20].copy_from_slice(&2u16.to_be_bytes());
        maxp[24..26].copy_from_slice(&16u16.to_be_bytes());
        maxp
    }

    // A negative CVT entry, read right only by a sign-extending decode, and a prep storing MPPEM and MPS and
    // writing 300 units by WCVTF: at 16.4 px MPPEM reads 16 and MPS 16.4 in 26.6, as FreeType's v40 does.
    #[test]
    fn a_negative_cvt_and_the_size_instructions_read_right() {
        let cvt: Vec<u8> = [-250i16, 100, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
        let prep = alloc::vec![0xB0, 0x00, 0x4B, 0x42, 0xB0, 0x01, 0x4C, 0x42, 0xB0, 0x02, 0xB8, 0x01, 0x2C, 0x70];
        let map = tables(&[("maxp", maxp()), ("cvt ", cvt), ("prep", prep)]);
        let size = f26dot6::Size::from_px(16.4).expect("a size");
        let ctx = HintContext::new(&map, size, 1000, HintMode::Classic).expect("the font has a prep");
        let scaled = |v| f26dot6::scale_fix(v, size.scale16(1000));
        assert_eq!(ctx.prep_cvt, [scaled(-250), scaled(100), scaled(300)]);
        assert!(ctx.prep_cvt[0] < 0, "-250 decoded as {}", ctx.prep_cvt[0]);
        assert_eq!(&ctx.prep_storage[..2], [16, 1050], "MPPEM and MPS at 16.4 px");
    }

    // SSW takes font units and MDRP and MIRP compare it with 26.6 distances, so it is scaled as it is
    // stored; FreeType holds 92, 184 and 369 for 180 units of a 1000 em at 8, 16 and 32 ppem.
    #[test]
    fn the_single_width_is_scaled_to_the_size() {
        let map = tables(&[("maxp", maxp()), ("prep", alloc::vec![0xB8, 0x00, 0xB4, 0x1F])]);
        for (ppem, want) in [(8u16, 92), (16, 184), (32, 369)] {
            let ctx = HintContext::new(&map, ppem.into(), 1000, HintMode::Classic).expect("the font has a prep");
            assert_eq!(ctx.interp.retained.single_width_value, want, "at {ppem} ppem");
        }
    }

    // Bytecode can push a point to the edge of i32 (SHPIX twice, by a product MUL clamps), and IUP then
    // works out the other points from it. Overflow there is a panic in a debug build and garbage in a release one.
    #[test]
    fn iup_from_a_point_at_the_rail_does_not_overflow() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/hinted.ttf");
        let bytes = std::fs::read(path).expect("the hinted fixture is missing");
        let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("the fixture parses");
        let upm = map.get("head").and_then(|h| read_u16_be(h, 18)).expect("units per em");
        let shift: &[u8] = &[0xB8, 0x00, 0x00, 0xB9, 0x7F, 0xFF, 0x7F, 0xFF, 0x63, 0xB8, 0x7F, 0xFF, 0x63, 0x38];
        let glyph = |program: &[u8], ys: [i16; 3]| {
            let mut glyf: Vec<u8> = Vec::new();
            for v in [1i16, 0, -100, 100, 0, 2] {
                glyf.extend(v.to_be_bytes());
            }
            glyf.extend((program.len() as u16).to_be_bytes());
            glyf.extend(program);
            glyf.extend([0x01u8; 3]);
            for v in [0i16, 100, 0].into_iter().chain(ys) {
                glyf.extend(v.to_be_bytes());
            }
            glyf
        };
        // Point 0 touched alone (MDAP[0]; v40 shifts only points touched in y), IUP shifts the rest with it;
        // with point 1 touched too (PUSHB 1, MDAP[0]), point 2 between them is interpolated from the rail.
        let start: &[u8] = &[0x00, 0xB0, 0x00, 0x2E];
        let alone = glyph(&[start, shift, shift, &[0x30]].concat(), [-100, 0, 100]);
        let between = glyph(&[start, shift, shift, &[0xB0, 0x01, 0x2E, 0x30]].concat(), [0, -100, 50]);
        for glyf in [alone, between] {
            let loca = [0usize, glyf.len()];
            for mode in [HintMode::Classic, HintMode::Subpixel] {
                let mut ctx = HintContext::new(&map, 16.into(), upm, mode).expect("the fixture is hinted");
                let out = ctx.hint_glyph(&glyf, &loca, 0).expect("the glyph hints");
                assert_eq!(out.y[0], i32::MAX, "{mode:?}: SHPIX did not take point 0 to the rail");
            }
        }
    }

    // `build_hinted_fixture.py` writes this CVT. With the negative-CVT test above, these values are what
    // checks the decode: no outline in the suite shows how far a CVT moves a point.
    #[test]
    fn the_cvt_decodes_to_the_values_the_fixture_was_built_with() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/hinted.ttf");
        let bytes = std::fs::read(path).expect("the hinted fixture is missing");
        let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("the fixture parses");
        let upm = map.get("head").and_then(|h| read_u16_be(h, 18)).expect("head names its units per em");
        assert_eq!(upm, 1024, "the fixture's units per em moved; rebuild the expected values");

        let units = [0, 100, 200, 400, 512];
        for ppem in [8u16, 11, 16, 24, 40] {
            let ctx = HintContext::new(&map, ppem.into(), upm, HintMode::Subpixel).expect("the fixture is hinted");
            let want: Vec<i32> = units.iter().map(|&v| f26dot6::scale_fix(v, f26dot6::Size::from(ppem).scale16(upm))).collect();
            assert_eq!(ctx.prep_cvt, want, "at {ppem} ppem");
        }
        let size = f26dot6::Size::from_px(16.4).expect("a size");
        let ctx = HintContext::new(&map, size, upm, HintMode::Subpixel).expect("the fixture is hinted");
        let want: Vec<i32> = units.iter().map(|&v| f26dot6::scale_fix(v, size.scale16(upm))).collect();
        assert_eq!(ctx.prep_cvt, want, "at 16.4 px");
        assert_ne!(want, units.map(|v| f26dot6::scale_fix(v, f26dot6::Size::from(16).scale16(upm))), "16.4 px scaled as 16");
    }
}
