use alloc::vec::Vec;
use super::f26dot6::{self, ONE};
use super::state::{GraphicsState, HintMode, RoundState, Vector, Zone, FLAG_ON_CURVE, FLAG_TOUCHED_X, FLAG_TOUCHED_Y};

// FreeType's limits: a call stack of 32 and a million opcodes per program run. Heavy instructions
// (IUP over every point, loops, deltas, MINDEX) also charge the work they do, which the opcode
// count alone does not bound: a short loop of IUPs over a 20,000-point glyph is billions of visits.
const MAX_CALL_DEPTH: usize = 32;
const MAX_OPCODES: usize = 1_000_000;
const MAX_WORK: usize = 10_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ProgramKind {
    Font,
    ControlValue,
    Glyph,
}

// A function or instruction definition: where its body starts and the IP of its ENDF.
#[derive(Clone, Copy)]
pub(crate) struct Function {
    pub program: ProgramKind,
    pub start: usize,
    pub end: usize,
}

// Why a run stopped short; a glyph program keeps what it did before, as FreeType's non-pedantic
// mode keeps it, while the font and CVT programs failing leaves the size unhinted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stop {
    CodeOverflow,
    StackOverflow,
    InvalidOpcode,
    InvalidReference,
    BadArgument,
    DivideByZero,
    NestedDefinition,
    DefinitionInGlyph,
    EndfOutsideCall,
    TooManyDefinitions,
    Debug,
    TooLong,
}

type Run<T = ()> = Result<T, Stop>;

// The v40 interpreter's backward compatibility (FreeType's ttinterp.h): bit 2 says it is on, and
// bits 0 and 1 that IUP has run on y and on x. With all three, no point moves further.
pub(crate) const COMPAT_ON: u8 = 4;
const COMPAT_DONE: u8 = 7;

pub(crate) struct Interpreter {
    pub gs: GraphicsState,
    pub retained: GraphicsState,
    pub storage: Vec<i32>,
    pub cvt: Vec<i32>,
    pub functions: Vec<Option<Function>>,
    pub max_functions: usize,
    pub defined_functions: usize,
    pub instructions: Vec<(u8, Function)>,
    pub max_instructions: usize,
    pub stack: Vec<i32>,
    pub stack_limit: usize,
    pub calls: Vec<Frame>,
    pub mode: HintMode,
    pub size: f26dot6::Size,
    pub scale16: i64,
    pub compat: u8,
    pub variable: bool,
    // GETVARIATION's normalized coordinates in 2.14, zero at a variable font's default.
    pub axes: Vec<i32>,
    pub num_glyphs: usize,
}

pub(crate) struct Programs<'a> {
    pub font: &'a [u8],
    pub control_value: &'a [u8],
    pub glyph: &'a [u8],
}

impl<'a> Programs<'a> {
    fn get(&self, kind: ProgramKind) -> &'a [u8] {
        match kind {
            ProgramKind::Font => self.font,
            ProgramKind::ControlValue => self.control_value,
            ProgramKind::Glyph => self.glyph,
        }
    }
}

pub(crate) struct Frame {
    caller: ProgramKind,
    return_to: usize,
    remaining: i32,
    def: Function,
}

// Pops and pushes per opcode, FreeType's Pop_Push_Count. Missing arguments read as zeros, and the
// loop instructions take their points separately.
const STACK_EFFECT: [(u8, u8); 256] = {
    let mut t = [(0u8, 0u8); 256];
    let mut i = 0;
    while i < 256 {
        let op = i as u8;
        t[i] = match op {
            0x06..=0x0B | 0x27 | 0x2A | 0x3A | 0x3B | 0x3E | 0x3F | 0x42 | 0x44 | 0x48 | 0x70 | 0x78 | 0x79
            | 0x81 | 0x82 | 0x86 | 0x87 | 0x8E => (2, 0),
            0x0C | 0x0D => (0, 2),
            0x0F => (5, 0),
            0x10..=0x17 | 0x1A | 0x1C..=0x1F | 0x21 | 0x26 | 0x29 | 0x2B | 0x2C | 0x2E | 0x2F | 0x34..=0x38
            | 0x4F | 0x58 | 0x5D..=0x5F | 0x71..=0x77 | 0x7E | 0x7F | 0x85 | 0x89 | 0x8D => (1, 0),
            0x20 => (1, 2),
            0x23 => (2, 2),
            0x24 | 0x4B | 0x4C => (0, 1),
            0x25 | 0x43 | 0x45..=0x47 | 0x56 | 0x57 | 0x5C | 0x64..=0x6F | 0x88 => (1, 1),
            0x49 | 0x4A | 0x50..=0x55 | 0x5A | 0x5B | 0x60..=0x63 | 0x8B | 0x8C => (2, 1),
            0x8A => (3, 3),
            0x92 => (0, 1),
            0xB0..=0xB7 => (0, op - 0xB0 + 1),
            0xB8..=0xBF => (0, op - 0xB8 + 1),
            0xC0..=0xFF => (1, 0),
            _ => (0, 0),
        };
        i += 1;
    }
    t
};

pub(crate) struct Machine<'a, 'z> {
    interp: &'a mut Interpreter,
    programs: Programs<'a>,
    zones: [&'z mut Zone; 2],
    initial: ProgramKind,
    is_composite: bool,
    // Font units to 26.6 for the `orus` distances; 1.0 for a composite's own program, whose `orus`
    // are already its assembled points.
    orus_scale: i64,
    twilight_len: usize,
    opcodes: usize,
    work: usize,
    loopcalls: usize,
    back_jumps: usize,
    loop_limit: usize,
    move_vector: (i32, i32),
}

impl<'a, 'z> Machine<'a, 'z> {
    pub fn new(
        interp: &'a mut Interpreter,
        programs: Programs<'a>,
        twilight: &'z mut Zone,
        glyph: &'z mut Zone,
        initial: ProgramKind,
        is_composite: bool,
    ) -> Machine<'a, 'z> {
        let orus_scale = if is_composite { 0x10000 } else { interp.scale16 };
        // FreeType's heuristics: twilight capped by the glyph and CVT sizes, and LOOPCALL counts and
        // backward jumps by about ten per point.
        let (points, cvt) = (glyph.len(), interp.cvt.len());
        let twilight_len = twilight.len().min((2 * (points + cvt)).clamp(30, 0xFFFF));
        let loop_limit = if points > 0 { (10 * points).max(50) + (cvt / 10).max(50) } else { 300 + 22 * cvt };
        let loop_limit = loop_limit.min(100 * interp.num_glyphs.max(1));
        let mut m = Machine {
            interp,
            programs,
            zones: [twilight, glyph],
            initial,
            is_composite,
            orus_scale,
            twilight_len,
            opcodes: 0,
            work: 0,
            loopcalls: 0,
            back_jumps: 0,
            loop_limit,
            move_vector: (0, 0),
        };
        m.compute_funcs();
        m
    }

    pub(crate) fn run(&mut self) -> Run {
        self.interp.stack.clear();
        self.interp.calls.clear();
        if self.initial == ProgramKind::Glyph {
            self.interp.compat &= !3;
        }
        self.execute()
    }

    fn gs(&self) -> &GraphicsState {
        &self.interp.gs
    }

    fn len_of(&self, z: usize) -> usize {
        if z == 0 { self.twilight_len } else { self.zones[1].len() }
    }

    fn in_zone(&self, z: usize, p: usize) -> bool {
        p < self.len_of(z)
    }

    fn charge(&mut self, n: usize) -> Run {
        self.work = self.work.saturating_add(n);
        if self.work > MAX_WORK { Err(Stop::TooLong) } else { Ok(()) }
    }

    fn pop(&mut self) -> i32 {
        self.interp.stack.pop().unwrap_or(0)
    }

    fn push(&mut self, v: i32) {
        self.interp.stack.push(v);
    }

    // FT_ULong and FT_UShort as FreeType casts an argument: a negative index is out of range.
    fn index(v: i32) -> usize {
        if v < 0 { usize::MAX } else { v as usize }
    }

    fn point(v: i32) -> usize {
        usize::from(v as u16)
    }

    fn execute(&mut self) -> Run {
        let mut range = self.initial;
        let mut code = self.programs.get(range);
        let mut ip = 0usize;
        if code.is_empty() {
            return Ok(());
        }
        loop {
            self.opcodes += 1;
            if self.opcodes > MAX_OPCODES {
                return Err(Stop::TooLong);
            }
            let op = code[ip];
            let (pops, pushes) = STACK_EFFECT[usize::from(op)];
            let (pops, pushes) = (usize::from(pops), usize::from(pushes));
            if self.interp.stack.len() < pops {
                self.interp.stack.clear();
                self.interp.stack.resize(pops, 0);
            }
            if self.interp.stack.len() - pops + pushes > self.interp.stack_limit {
                return Err(Stop::StackOverflow);
            }
            let mut len = 1usize;
            match op {
                0x00..=0x05 => self.svtca(op),
                0x06..=0x09 => {
                    let (top, second) = (self.pop(), self.pop());
                    self.sxvtl(op, Self::point(top), Self::point(second));
                }
                0x0A | 0x0B => {
                    let (y, x) = (self.pop(), self.pop());
                    if let Some((x, y)) = f26dot6::normalize(i32::from(x as i16), i32::from(y as i16)) {
                        let v = Vector { x, y };
                        if op == 0x0A {
                            self.interp.gs.projection = v;
                            self.interp.gs.dual_projection = v;
                        } else {
                            self.interp.gs.freedom = v;
                        }
                    }
                    self.compute_funcs();
                }
                0x0C | 0x0D => {
                    let v = if op == 0x0C { self.gs().projection } else { self.gs().freedom };
                    self.push(v.x);
                    self.push(v.y);
                }
                0x0E => {
                    self.interp.gs.freedom = self.gs().projection;
                    self.compute_funcs();
                }
                0x0F => self.isect(),
                0x10 => self.interp.gs.rp0 = Self::point(self.pop()),
                0x11 => self.interp.gs.rp1 = Self::point(self.pop()),
                0x12 => self.interp.gs.rp2 = Self::point(self.pop()),
                0x13..=0x16 => {
                    let z = self.pop();
                    if let 0 | 1 = z {
                        let z = z as usize;
                        match op {
                            0x13 => self.interp.gs.zp0 = z,
                            0x14 => self.interp.gs.zp1 = z,
                            0x15 => self.interp.gs.zp2 = z,
                            _ => {
                                let gs = &mut self.interp.gs;
                                (gs.zp0, gs.zp1, gs.zp2) = (z, z, z);
                            }
                        }
                    }
                }
                0x17 => {
                    let n = self.pop();
                    if n < 0 {
                        return Err(Stop::BadArgument);
                    }
                    self.interp.gs.loop_count = n.min(0xFFFF);
                }
                0x18 => self.interp.gs.round_state = RoundState::ToGrid,
                0x19 => self.interp.gs.round_state = RoundState::ToHalfGrid,
                0x1A => self.interp.gs.minimum_distance = self.pop(),
                0x1B => {
                    let mut depth = 1;
                    while depth != 0 {
                        match self.skip(code, &mut ip, &mut len)? {
                            0x58 => depth += 1,
                            0x59 => depth -= 1,
                            _ => {}
                        }
                    }
                }
                0x1C | 0x78 | 0x79 => {
                    let (offset, take) = if op == 0x1C {
                        (self.pop(), true)
                    } else {
                        let (cond, offset) = (self.pop(), self.pop());
                        (offset, (cond != 0) == (op == 0x78))
                    };
                    if take {
                        if offset == 0 && self.interp.stack.is_empty() {
                            return Err(Stop::BadArgument);
                        }
                        let target = ip as i64 + i64::from(offset);
                        let past_function = self.interp.calls.last().is_some_and(|f| target > f.def.end as i64);
                        if target < 0 || past_function {
                            return Err(Stop::BadArgument);
                        }
                        ip = target as usize;
                        len = 0;
                        if offset < 0 {
                            self.back_jumps += 1;
                            if self.back_jumps > self.loop_limit {
                                return Err(Stop::TooLong);
                            }
                        }
                    }
                }
                0x1D => self.interp.gs.control_value_cut_in = self.pop(),
                0x1E => self.interp.gs.single_width_cut_in = self.pop(),
                0x1F => {
                    let v = self.pop();
                    self.interp.gs.single_width_value = f26dot6::scale_fix(v, self.interp.scale16);
                }
                0x20 => {
                    let v = self.pop();
                    self.push(v);
                    self.push(v);
                }
                0x21 => {
                    self.pop();
                }
                0x22 => self.interp.stack.clear(),
                0x23 => {
                    let (b, a) = (self.pop(), self.pop());
                    self.push(b);
                    self.push(a);
                }
                0x24 => {
                    let depth = self.interp.stack.len() as i32;
                    self.push(depth);
                }
                0x25 => {
                    let k = self.pop();
                    let n = self.interp.stack.len();
                    let v = if k <= 0 || k as usize > n { 0 } else { self.interp.stack[n - k as usize] };
                    self.push(v);
                }
                0x26 => {
                    let k = self.pop();
                    let n = self.interp.stack.len();
                    if k > 0 && k as usize <= n {
                        self.charge(k as usize)?;
                        let v = self.interp.stack.remove(n - k as usize);
                        self.push(v);
                    }
                }
                0x27 => self.alignpts(),
                0x29 => self.utp(),
                0x2A | 0x2B => {
                    let id = self.pop();
                    let count = if op == 0x2A { self.pop() } else { 1 };
                    let def = self.function(id)?;
                    if self.interp.calls.len() >= MAX_CALL_DEPTH {
                        return Err(Stop::StackOverflow);
                    }
                    if count > 0 {
                        self.interp.calls.push(Frame { caller: range, return_to: ip + 1, remaining: count, def });
                        (range, ip, len) = (def.program, def.start, 0);
                        code = self.programs.get(range);
                        if ip > code.len() {
                            return Err(Stop::CodeOverflow);
                        }
                        if op == 0x2A {
                            self.loopcalls = self.loopcalls.saturating_add(count as usize);
                            if self.loopcalls > self.loop_limit {
                                return Err(Stop::TooLong);
                            }
                        }
                    }
                }
                0x2C | 0x89 => {
                    if self.initial == ProgramKind::Glyph {
                        return Err(Stop::DefinitionInGlyph);
                    }
                    let id = self.pop();
                    let start = ip + 1;
                    let slot = if op == 0x2C { self.define_function(id)? } else { self.define_instruction(id)? };
                    loop {
                        match self.skip(code, &mut ip, &mut len)? {
                            0x2C | 0x89 => return Err(Stop::NestedDefinition),
                            0x2D => break,
                            _ => {}
                        }
                    }
                    let def = Function { program: range, start, end: ip };
                    match slot {
                        Slot::Function(id) => self.interp.functions[id] = Some(def),
                        Slot::Instruction(i) => self.interp.instructions[i].1 = def,
                    }
                }
                0x2D => {
                    let Some(mut frame) = self.interp.calls.pop() else { return Err(Stop::EndfOutsideCall) };
                    frame.remaining -= 1;
                    if frame.remaining > 0 {
                        ip = frame.def.start;
                        self.interp.calls.push(frame);
                    } else {
                        range = frame.caller;
                        code = self.programs.get(range);
                        ip = frame.return_to;
                    }
                    len = 0;
                }
                0x2E | 0x2F => self.mdap(op),
                0x30 | 0x31 => self.iup(op)?,
                0x32 | 0x33 => self.shp(op)?,
                0x34 | 0x35 => self.shc(op)?,
                0x36 | 0x37 => self.shz(op)?,
                0x38 => self.shpix()?,
                0x39 => self.ip()?,
                0x3A | 0x3B => self.msirp(op),
                0x3C => self.alignrp()?,
                0x3D => self.interp.gs.round_state = RoundState::ToDoubleGrid,
                0x3E | 0x3F => self.miap(op),
                0x40 | 0x41 => {
                    let n = usize::from(*code.get(ip + 1).ok_or(Stop::CodeOverflow)?);
                    let width = if op == 0x40 { 1 } else { 2 };
                    if ip + 1 + width * n >= code.len() {
                        return Err(Stop::CodeOverflow);
                    }
                    if self.interp.stack.len() + n > self.interp.stack_limit {
                        return Err(Stop::StackOverflow);
                    }
                    self.push_run(code, ip + 2, n, width == 2);
                    ip += 1 + width * n;
                }
                0xB0..=0xBF => {
                    let (n, words) = if op < 0xB8 { (usize::from(op - 0xB0) + 1, false) } else { (usize::from(op - 0xB8) + 1, true) };
                    let width = if words { 2 } else { 1 };
                    if ip + width * n >= code.len() {
                        return Err(Stop::CodeOverflow);
                    }
                    self.push_run(code, ip + 1, n, words);
                    ip += width * n;
                }
                0x42 => {
                    let (v, i) = (self.pop(), Self::index(self.pop()));
                    if let Some(slot) = self.interp.storage.get_mut(i) {
                        *slot = v;
                    }
                }
                0x43 => {
                    let i = Self::index(self.pop());
                    let v = self.interp.storage.get(i).copied().unwrap_or(0);
                    self.push(v);
                }
                0x44 | 0x70 => {
                    let (v, i) = (self.pop(), Self::index(self.pop()));
                    let v = if op == 0x70 { f26dot6::scale_fix(v, self.interp.scale16) } else { v };
                    if let Some(slot) = self.interp.cvt.get_mut(i) {
                        *slot = v;
                    }
                }
                0x45 => {
                    let i = Self::index(self.pop());
                    let v = self.interp.cvt.get(i).copied().unwrap_or(0);
                    self.push(v);
                }
                0x46 | 0x47 => {
                    let p = Self::index(self.pop());
                    let z = self.gs().zp2;
                    let v = if !self.in_zone(z, p) {
                        0
                    } else if op == 0x47 {
                        self.dual_project(i64::from(self.zones[z].org_x[p]), i64::from(self.zones[z].org_y[p]))
                    } else {
                        self.project(i64::from(self.zones[z].cur_x[p]), i64::from(self.zones[z].cur_y[p]))
                    };
                    self.push(v);
                }
                0x48 => self.scfs(),
                0x49 | 0x4A => self.md(op),
                0x4B => {
                    let ppem = i32::from(self.interp.size.ppem());
                    self.push(ppem);
                }
                // The spec's point size in 26.6, which FreeType's v40 gives; its v35 gives the ppem.
                0x4C => {
                    let size = self.interp.size.units();
                    self.push(size);
                }
                0x4D => self.interp.gs.auto_flip = true,
                0x4E => self.interp.gs.auto_flip = false,
                0x4F => return Err(Stop::Debug),
                0x50..=0x55 => {
                    let (b, a) = (self.pop(), self.pop());
                    let r = match op {
                        0x50 => a < b,
                        0x51 => a <= b,
                        0x52 => a > b,
                        0x53 => a >= b,
                        0x54 => a == b,
                        _ => a != b,
                    };
                    self.push(i32::from(r));
                }
                0x56 | 0x57 => {
                    let a = self.pop();
                    let odd = self.gs().round_state.apply(a) & ONE != 0;
                    self.push(i32::from(odd == (op == 0x56)));
                }
                0x58 => {
                    if self.pop() == 0 {
                        let mut depth = 1;
                        loop {
                            match self.skip(code, &mut ip, &mut len)? {
                                0x58 => depth += 1,
                                0x1B if depth == 1 => break,
                                0x59 => {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                0x59 => {}
                0x5A | 0x5B => {
                    let (b, a) = (self.pop(), self.pop());
                    let r = if op == 0x5A { a != 0 && b != 0 } else { a != 0 || b != 0 };
                    self.push(i32::from(r));
                }
                0x5C => {
                    let a = self.pop();
                    self.push(i32::from(a == 0));
                }
                0x5D | 0x71 | 0x72 => self.delta(op, false)?,
                0x73..=0x75 => self.delta(op, true)?,
                0x5E => self.interp.gs.delta_base = i32::from(self.pop() as u16),
                0x5F => {
                    let s = self.pop();
                    if !(0..=6).contains(&s) {
                        return Err(Stop::BadArgument);
                    }
                    self.interp.gs.delta_shift = s;
                }
                0x60 | 0x61 => {
                    let (b, a) = (self.pop(), self.pop());
                    self.push(if op == 0x60 { a.wrapping_add(b) } else { a.wrapping_sub(b) });
                }
                0x62 => {
                    let (b, a) = (self.pop(), self.pop());
                    if b == 0 {
                        return Err(Stop::DivideByZero);
                    }
                    self.push(f26dot6::mul_div_no_round(a, ONE, b));
                }
                0x63 => {
                    let (b, a) = (self.pop(), self.pop());
                    self.push(f26dot6::mul_div(a, b, ONE));
                }
                0x64 => {
                    let a = self.pop();
                    self.push(if a < 0 { a.wrapping_neg() } else { a });
                }
                0x65 => {
                    let a = self.pop();
                    self.push(a.wrapping_neg());
                }
                0x66 => {
                    let a = self.pop();
                    self.push(f26dot6::floor_pixel(a));
                }
                0x67 => {
                    let a = self.pop();
                    self.push(a.wrapping_add(ONE - 1) & !(ONE - 1));
                }
                0x68..=0x6B => {
                    let a = self.pop();
                    let r = self.gs().round_state.apply(a);
                    self.push(r);
                }
                0x6C..=0x6F => {}
                0x76 | 0x77 => {
                    let n = self.pop();
                    self.interp.gs.round_state = if op == 0x76 {
                        RoundState::sround(n, 0x4000, false)
                    } else {
                        RoundState::sround(n, 0x2D41, true)
                    };
                }
                0x7A => self.interp.gs.round_state = RoundState::Off,
                0x7C => self.interp.gs.round_state = RoundState::UpToGrid,
                0x7D => self.interp.gs.round_state = RoundState::DownToGrid,
                0x7E | 0x7F | 0x85 | 0x8D => {
                    self.pop();
                }
                0x80 => self.flippt()?,
                0x81 | 0x82 => self.fliprg(op),
                0x86 | 0x87 => self.sdpvtl(op),
                0x88 => {
                    let selector = self.pop();
                    let v = self.get_info(selector);
                    self.push(v);
                }
                0x8A => {
                    let (c, b, a) = (self.pop(), self.pop(), self.pop());
                    self.push(b);
                    self.push(c);
                    self.push(a);
                }
                0x8B | 0x8C => {
                    let (b, a) = (self.pop(), self.pop());
                    self.push(if op == 0x8B { a.max(b) } else { a.min(b) });
                }
                0x8E => self.instctrl(),
                0x91 if self.interp.variable => {
                    let n = self.interp.axes.len();
                    if self.interp.stack.len() + n > self.interp.stack_limit {
                        return Err(Stop::StackOverflow);
                    }
                    for i in 0..n {
                        let v = self.interp.axes[i];
                        self.push(v);
                    }
                }
                0x92 if self.interp.variable => self.push(17),
                0xC0..=0xDF => self.mdrp(op),
                0xE0..=0xFF => self.mirp(op),
                _ => {
                    let Some(def) = self.interp.instructions.iter().find(|(o, _)| *o == op).map(|&(_, d)| d) else {
                        return Err(Stop::InvalidOpcode);
                    };
                    if self.interp.calls.len() >= MAX_CALL_DEPTH {
                        return Err(Stop::StackOverflow);
                    }
                    self.interp.calls.push(Frame { caller: range, return_to: ip + 1, remaining: 1, def });
                    (range, ip, len) = (def.program, def.start, 0);
                    code = self.programs.get(range);
                }
            }
            ip += len;
            if ip >= code.len() {
                return if self.interp.calls.is_empty() { Ok(()) } else { Err(Stop::CodeOverflow) };
            }
        }
    }

    // FreeType's SkipCode: past the current instruction to the next, whose opcode it returns.
    fn skip(&mut self, code: &[u8], ip: &mut usize, len: &mut usize) -> Run<u8> {
        self.charge(1)?;
        *ip += *len;
        let op = *code.get(*ip).ok_or(Stop::CodeOverflow)?;
        *len = match op {
            0x40 => 2 + usize::from(*code.get(*ip + 1).ok_or(Stop::CodeOverflow)?),
            0x41 => 2 + 2 * usize::from(*code.get(*ip + 1).ok_or(Stop::CodeOverflow)?),
            0xB0..=0xB7 => usize::from(op - 0xB0) + 2,
            0xB8..=0xBF => 2 * usize::from(op - 0xB8) + 3,
            _ => 1,
        };
        Ok(op)
    }

    fn push_run(&mut self, code: &[u8], at: usize, n: usize, words: bool) {
        for k in 0..n {
            let v = if words {
                i32::from(i16::from_be_bytes([code[at + 2 * k], code[at + 2 * k + 1]]))
            } else {
                i32::from(code[at + k])
            };
            self.push(v);
        }
    }

    fn function(&self, id: i32) -> Run<Function> {
        self.interp.functions.get(Self::index(id)).copied().flatten().ok_or(Stop::InvalidReference)
    }

    fn define_function(&mut self, id: i32) -> Run<Slot> {
        let id = Self::index(id);
        let defined = self.interp.functions.get(id).is_some_and(Option::is_some);
        if !defined {
            if self.interp.defined_functions >= self.interp.max_functions {
                return Err(Stop::TooManyDefinitions);
            }
            self.interp.defined_functions += 1;
        }
        if id > 0xFFFF {
            return Err(Stop::TooManyDefinitions);
        }
        if self.interp.functions.len() <= id {
            self.interp.functions.resize(id + 1, None);
        }
        Ok(Slot::Function(id))
    }

    fn define_instruction(&mut self, id: i32) -> Run<Slot> {
        let at = self.interp.instructions.iter().position(|(o, _)| i32::from(*o) == id);
        if at.is_none() && self.interp.instructions.len() >= self.interp.max_instructions {
            return Err(Stop::TooManyDefinitions);
        }
        let op = u8::try_from(id).map_err(|_| Stop::TooManyDefinitions)?;
        let placeholder = Function { program: ProgramKind::Font, start: 0, end: 0 };
        Ok(Slot::Instruction(at.unwrap_or_else(|| {
            self.interp.instructions.push((op, placeholder));
            self.interp.instructions.len() - 1
        })))
    }

    // FreeType's Compute_Funcs: the move along the freedom vector per unit of projected distance.
    fn compute_funcs(&mut self) {
        let (p, f) = (self.gs().projection, self.gs().freedom);
        let f_dot_p = (i64::from(p.x) * i64::from(f.x) + i64::from(p.y) * i64::from(f.y) + 0x2000) >> 14;
        self.move_vector = if f_dot_p >= 0x3FFE {
            (f.x * 4, f.y * 4)
        } else if (-0x3FF..=0x3FF).contains(&f_dot_p) {
            (0, 0)
        } else {
            ((i64::from(f.x) * 0x10000 / f_dot_p) as i32, (i64::from(f.y) * 0x10000 / f_dot_p) as i32)
        };
    }

    fn project(&self, dx: i64, dy: i64) -> i32 {
        along(self.gs().projection, dx, dy)
    }

    fn dual_project(&self, dx: i64, dy: i64) -> i32 {
        along(self.gs().dual_projection, dx, dy)
    }

    // The projected distance between two points' current positions, p minus q.
    fn project_cur(&self, z1: usize, p: usize, z2: usize, q: usize) -> i32 {
        let (a, b) = (&self.zones[z1], &self.zones[z2]);
        self.project(i64::from(a.cur_x[p]) - i64::from(b.cur_x[q]), i64::from(a.cur_y[p]) - i64::from(b.cur_y[q]))
    }

    fn dual_project_org(&self, z1: usize, p: usize, z2: usize, q: usize) -> i32 {
        let (a, b) = (&self.zones[z1], &self.zones[z2]);
        self.dual_project(i64::from(a.org_x[p]) - i64::from(b.org_x[q]), i64::from(a.org_y[p]) - i64::from(b.org_y[q]))
    }

    // An original distance: from the unscaled points, scaled once, so it does not carry two
    // roundings; the twilight zone, which has none, measures its originals.
    fn original_distance(&self, z1: usize, p: usize, z2: usize, q: usize) -> i32 {
        if self.gs().zp0 == 0 || self.gs().zp1 == 0 {
            return self.dual_project_org(z1, p, z2, q);
        }
        let (a, b) = (&self.zones[z1], &self.zones[z2]);
        let d = self.dual_project(i64::from(a.orus_x[p]) - i64::from(b.orus_x[q]), i64::from(a.orus_y[p]) - i64::from(b.orus_y[q]));
        f26dot6::scale_fix(d, self.orus_scale)
    }

    // FreeType's Direct_Move. Under v40's backward compatibility x never moves, and nothing moves
    // once IUP has run on both axes; the point is marked touched either way.
    fn move_point(&mut self, z: usize, p: usize, distance: i32) {
        let (mx, my) = self.move_vector;
        let compat = self.interp.compat;
        let zone = &mut *self.zones[z];
        if mx != 0 {
            if compat == 0 {
                zone.cur_x[p] = zone.cur_x[p].saturating_add(f26dot6::mul_fix(distance, mx));
            }
            zone.flags[p] |= FLAG_TOUCHED_X;
        }
        if my != 0 {
            if compat != COMPAT_DONE {
                zone.cur_y[p] = zone.cur_y[p].saturating_add(f26dot6::mul_fix(distance, my));
            }
            zone.flags[p] |= FLAG_TOUCHED_Y;
        }
    }

    fn move_original(&mut self, z: usize, p: usize, distance: i32) {
        let (mx, my) = self.move_vector;
        let zone = &mut *self.zones[z];
        zone.org_x[p] = zone.org_x[p].saturating_add(f26dot6::mul_fix(distance, mx));
        zone.org_y[p] = zone.org_y[p].saturating_add(f26dot6::mul_fix(distance, my));
    }

    // FreeType's Move_Zp2_Point: a shift already split into x and y.
    fn shift_zp2(&mut self, p: usize, dx: i32, dy: i32, touch: bool) {
        let (f, compat, z) = (self.gs().freedom, self.interp.compat, self.gs().zp2);
        let zone = &mut *self.zones[z];
        if f.x != 0 {
            if compat == 0 {
                zone.cur_x[p] = zone.cur_x[p].saturating_add(dx);
            }
            if touch {
                zone.flags[p] |= FLAG_TOUCHED_X;
            }
        }
        if f.y != 0 {
            if compat != COMPAT_DONE {
                zone.cur_y[p] = zone.cur_y[p].saturating_add(dy);
            }
            if touch {
                zone.flags[p] |= FLAG_TOUCHED_Y;
            }
        }
    }

    fn svtca(&mut self, op: u8) {
        let v = if op & 1 != 0 { Vector::X_AXIS } else { Vector::Y_AXIS };
        let gs = &mut self.interp.gs;
        if op < 4 {
            gs.projection = v;
            gs.dual_projection = v;
        }
        if op & 2 == 0 {
            gs.freedom = v;
        }
        self.compute_funcs();
    }

    // SPVTL and SFVTL: the line from zp2's point `a`, the top argument, to zp1's point `b`, or
    // its perpendicular.
    fn sxvtl(&mut self, op: u8, a: usize, b: usize) {
        let (z1, z2) = (self.gs().zp1, self.gs().zp2);
        if !self.in_zone(z2, a) || !self.in_zone(z1, b) {
            return;
        }
        let (p1, p2) = (&self.zones[z1], &self.zones[z2]);
        let mut dx = p1.cur_x[b].wrapping_sub(p2.cur_x[a]);
        let mut dy = p1.cur_y[b].wrapping_sub(p2.cur_y[a]);
        let mut perpendicular = op & 1 != 0;
        if dx == 0 && dy == 0 {
            (dx, dy, perpendicular) = (0x4000, 0, false);
        }
        if perpendicular {
            (dx, dy) = (dy.wrapping_neg(), dx);
        }
        if let Some((x, y)) = f26dot6::normalize(dx, dy) {
            let v = Vector { x, y };
            if op < 0x08 {
                self.interp.gs.projection = v;
                self.interp.gs.dual_projection = v;
            } else {
                self.interp.gs.freedom = v;
            }
        }
        self.compute_funcs();
    }

    fn sdpvtl(&mut self, op: u8) {
        let (b, a) = (Self::point(self.pop()), Self::point(self.pop()));
        let (z1, z2) = (self.gs().zp1, self.gs().zp2);
        if !self.in_zone(z1, a) || !self.in_zone(z2, b) {
            return;
        }
        let mut perpendicular = op & 1 != 0;
        let mut vector = |dx: i32, dy: i32| {
            let (mut dx, mut dy) = (dx, dy);
            if dx == 0 && dy == 0 {
                (dx, dy, perpendicular) = (0x4000, 0, false);
            }
            if perpendicular {
                (dx, dy) = (dy.wrapping_neg(), dx);
            }
            f26dot6::normalize(dx, dy)
        };
        let (p1, p2) = (&self.zones[z1], &self.zones[z2]);
        let dual = vector(p1.org_x[a].wrapping_sub(p2.org_x[b]), p1.org_y[a].wrapping_sub(p2.org_y[b]));
        let proj = vector(p1.cur_x[a].wrapping_sub(p2.cur_x[b]), p1.cur_y[a].wrapping_sub(p2.cur_y[b]));
        if let Some((x, y)) = dual {
            self.interp.gs.dual_projection = Vector { x, y };
        }
        if let Some((x, y)) = proj {
            self.interp.gs.projection = Vector { x, y };
        }
        self.compute_funcs();
    }

    fn isect(&mut self) {
        let b1 = Self::point(self.pop());
        let b0 = Self::point(self.pop());
        let a1 = Self::point(self.pop());
        let a0 = Self::point(self.pop());
        let p = Self::point(self.pop());
        let gs = self.gs();
        let (z0, z1, z2) = (gs.zp0, gs.zp1, gs.zp2);
        if !self.in_zone(z0, b0) || !self.in_zone(z0, b1) || !self.in_zone(z1, a0) || !self.in_zone(z1, a1) || !self.in_zone(z2, p) {
            return;
        }
        let (za, zb) = (&self.zones[z1], &self.zones[z0]);
        let (ax0, ay0, ax1, ay1) = (za.cur_x[a0], za.cur_y[a0], za.cur_x[a1], za.cur_y[a1]);
        let (bx0, by0, bx1, by1) = (zb.cur_x[b0], zb.cur_y[b0], zb.cur_x[b1], zb.cur_y[b1]);
        let (dbx, dby) = (bx1.wrapping_sub(bx0), by1.wrapping_sub(by0));
        let (dax, day) = (ax1.wrapping_sub(ax0), ay1.wrapping_sub(ay0));
        let (dx, dy) = (bx0.wrapping_sub(ax0), by0.wrapping_sub(ay0));
        let md = |a, b| f26dot6::mul_div(a, b, 0x40);
        let discriminant = md(dax, dby.wrapping_neg()).wrapping_add(md(day, dbx));
        let dot = md(dax, dbx).wrapping_add(md(day, dby));
        // Crossing at under about 3 degrees (|tan| below 1/19) is taken as no crossing.
        let (x, y) = if 19 * i64::from(discriminant).abs() > i64::from(dot).abs() {
            let v = md(dx, dby.wrapping_neg()).wrapping_add(md(dy, dbx));
            (ax0.wrapping_add(f26dot6::mul_div(v, dax, discriminant)), ay0.wrapping_add(f26dot6::mul_div(v, day, discriminant)))
        } else {
            let mid = |a: i32, b: i32, c: i32, d: i32| ((i64::from(a) + i64::from(b) + i64::from(c) + i64::from(d)) / 4) as i32;
            (mid(ax0, ax1, bx0, bx1), mid(ay0, ay1, by0, by1))
        };
        let zone = &mut *self.zones[z2];
        zone.cur_x[p] = x;
        zone.cur_y[p] = y;
        zone.flags[p] |= FLAG_TOUCHED_X | FLAG_TOUCHED_Y;
    }

    fn alignpts(&mut self) {
        let (p2, p1) = (Self::point(self.pop()), Self::point(self.pop()));
        let (z0, z1) = (self.gs().zp0, self.gs().zp1);
        if !self.in_zone(z1, p1) || !self.in_zone(z0, p2) {
            return;
        }
        let distance = self.project_cur(z0, p2, z1, p1) / 2;
        self.move_point(z1, p1, distance);
        self.move_point(z0, p2, distance.wrapping_neg());
    }

    fn utp(&mut self) {
        let p = Self::point(self.pop());
        let z = self.gs().zp0;
        if !self.in_zone(z, p) {
            return;
        }
        let f = self.gs().freedom;
        let mut mask = 0xFF;
        if f.x != 0 {
            mask &= !FLAG_TOUCHED_X;
        }
        if f.y != 0 {
            mask &= !FLAG_TOUCHED_Y;
        }
        self.zones[z].flags[p] &= mask;
    }

    fn mdap(&mut self, op: u8) {
        let p = Self::point(self.pop());
        let z = self.gs().zp0;
        if !self.in_zone(z, p) {
            return;
        }
        let distance = if op & 1 != 0 {
            let cur = self.project(i64::from(self.zones[z].cur_x[p]), i64::from(self.zones[z].cur_y[p]));
            self.gs().round_state.apply(cur).wrapping_sub(cur)
        } else {
            0
        };
        self.move_point(z, p, distance);
        self.interp.gs.rp0 = p;
        self.interp.gs.rp1 = p;
    }

    fn miap(&mut self, op: u8) {
        let cvt_index = Self::index(self.pop());
        let p = Self::point(self.pop());
        let z = self.gs().zp0;
        if self.in_zone(z, p) && cvt_index < self.interp.cvt.len() {
            let mut distance = self.interp.cvt[cvt_index];
            // In the twilight zone the point is first placed at the CVT distance along the freedom
            // vector, original and current, as Microsoft's rasterizer does (FreeType's Ins_MIAP).
            if z == 0 {
                let f = self.gs().freedom;
                let zone = &mut *self.zones[0];
                zone.org_x[p] = f26dot6::mul_fix14(distance, f.x);
                zone.org_y[p] = f26dot6::mul_fix14(distance, f.y);
                zone.cur_x[p] = zone.org_x[p];
                zone.cur_y[p] = zone.org_y[p];
            }
            let cur = self.project(i64::from(self.zones[z].cur_x[p]), i64::from(self.zones[z].cur_y[p]));
            if op & 1 != 0 {
                if (i64::from(distance) - i64::from(cur)).abs() > i64::from(self.gs().control_value_cut_in) {
                    distance = cur;
                }
                distance = self.gs().round_state.apply(distance);
            }
            self.move_point(z, p, distance.wrapping_sub(cur));
        }
        self.interp.gs.rp0 = p;
        self.interp.gs.rp1 = p;
    }

    fn mdrp(&mut self, op: u8) {
        let p = Self::point(self.pop());
        let gs = self.gs();
        let (z0, z1, rp0) = (gs.zp0, gs.zp1, gs.rp0);
        if self.in_zone(z1, p) && self.in_zone(z0, rp0) {
            let mut org = self.original_distance(z1, p, z0, rp0);
            let gs = self.gs();
            let (sw, swc) = (gs.single_width_value, gs.single_width_cut_in);
            if swc > 0 && org < sw.saturating_add(swc) && org > sw.saturating_sub(swc) {
                org = if org >= 0 { sw } else { sw.saturating_neg() };
            }
            let mut distance = if op & 4 != 0 { gs.round_state.apply(org) } else { org };
            if op & 8 != 0 {
                distance = minimum_distance(org, distance, gs.minimum_distance);
            }
            let cur = self.project_cur(z1, p, z0, rp0);
            self.move_point(z1, p, distance.wrapping_sub(cur));
        }
        let gs = &mut self.interp.gs;
        gs.rp1 = gs.rp0;
        gs.rp2 = p;
        if op & 16 != 0 {
            gs.rp0 = p;
        }
    }

    fn mirp(&mut self, op: u8) {
        let cvt_entry = i64::from(self.pop()) + 1;
        let p = Self::point(self.pop());
        let gs = self.gs();
        let (z0, z1, rp0) = (gs.zp0, gs.zp1, gs.rp0);
        // CVT entry -1 always reads 0 (FreeType's Ins_MIRP, matching Windows).
        if self.in_zone(z1, p) && (0..=self.interp.cvt.len() as i64).contains(&cvt_entry) && self.in_zone(z0, rp0) {
            let mut cvt = if cvt_entry == 0 { 0 } else { self.interp.cvt[cvt_entry as usize - 1] };
            let (sw, swc) = (gs.single_width_value, gs.single_width_cut_in);
            if (i64::from(cvt) - i64::from(sw)).abs() < i64::from(swc) {
                cvt = if cvt >= 0 { sw } else { sw.saturating_neg() };
            }
            if z1 == 0 {
                let f = self.gs().freedom;
                let (ox, oy) = (self.zones[z0].org_x[rp0], self.zones[z0].org_y[rp0]);
                let zone = &mut *self.zones[0];
                zone.org_x[p] = ox.wrapping_add(f26dot6::mul_fix14(cvt, f.x));
                zone.org_y[p] = oy.wrapping_add(f26dot6::mul_fix14(cvt, f.y));
                zone.cur_x[p] = zone.org_x[p];
                zone.cur_y[p] = zone.org_y[p];
            }
            let org = self.dual_project_org(z1, p, z0, rp0);
            let cur = self.project_cur(z1, p, z0, rp0);
            let gs = self.gs();
            if gs.auto_flip && (org ^ cvt) < 0 {
                cvt = cvt.saturating_neg();
            }
            let distance = if op & 4 != 0 {
                // The cut-in test only when both points are in one zone, as Windows does.
                if gs.zp0 == gs.zp1 && (i64::from(cvt) - i64::from(org)).abs() > i64::from(gs.control_value_cut_in) {
                    cvt = org;
                }
                gs.round_state.apply(cvt)
            } else {
                cvt
            };
            let distance = if op & 8 != 0 { minimum_distance(org, distance, gs.minimum_distance) } else { distance };
            self.move_point(z1, p, distance.wrapping_sub(cur));
        }
        let gs = &mut self.interp.gs;
        gs.rp1 = gs.rp0;
        gs.rp2 = p;
        if op & 16 != 0 {
            gs.rp0 = p;
        }
    }

    fn msirp(&mut self, op: u8) {
        let distance = self.pop();
        let p = Self::point(self.pop());
        let gs = self.gs();
        let (z0, z1, rp0) = (gs.zp0, gs.zp1, gs.rp0);
        if !self.in_zone(z1, p) || !self.in_zone(z0, rp0) {
            return;
        }
        if z1 == 0 {
            let (ox, oy) = (self.zones[z0].org_x[rp0], self.zones[z0].org_y[rp0]);
            self.zones[0].org_x[p] = ox;
            self.zones[0].org_y[p] = oy;
            self.move_original(0, p, distance);
            let zone = &mut *self.zones[0];
            zone.cur_x[p] = zone.org_x[p];
            zone.cur_y[p] = zone.org_y[p];
        }
        let cur = self.project_cur(z1, p, z0, rp0);
        self.move_point(z1, p, distance.wrapping_sub(cur));
        let gs = &mut self.interp.gs;
        gs.rp1 = gs.rp0;
        gs.rp2 = p;
        if op & 1 != 0 {
            gs.rp0 = p;
        }
    }

    fn scfs(&mut self) {
        let (value, p) = (self.pop(), Self::point(self.pop()));
        let z = self.gs().zp2;
        if !self.in_zone(z, p) {
            return;
        }
        let cur = self.project(i64::from(self.zones[z].cur_x[p]), i64::from(self.zones[z].cur_y[p]));
        self.move_point(z, p, value.wrapping_sub(cur));
        if z == 0 {
            let zone = &mut *self.zones[0];
            zone.org_x[p] = zone.cur_x[p];
            zone.org_y[p] = zone.cur_y[p];
        }
    }

    // MD: zp0's point, the one below the top, less zp1's, the top one, as FreeType and Windows
    // measure it.
    fn md(&mut self, op: u8) {
        let (k, l) = (Self::point(self.pop()), Self::point(self.pop()));
        let (z0, z1) = (self.gs().zp0, self.gs().zp1);
        let d = if !self.in_zone(z0, l) || !self.in_zone(z1, k) {
            0
        } else if op & 1 != 0 {
            self.project_cur(z0, l, z1, k)
        } else {
            self.original_distance(z0, l, z1, k)
        };
        self.push(d);
    }

    // The reference point's move so far, along the freedom vector, for SHP, SHC and SHZ.
    fn displacement(&self, op: u8) -> Option<(usize, usize, i32, i32)> {
        let gs = self.gs();
        let (z, p) = if op & 1 != 0 { (gs.zp0, gs.rp1) } else { (gs.zp1, gs.rp2) };
        if !self.in_zone(z, p) {
            return None;
        }
        let zone = &self.zones[z];
        let d = self.project(i64::from(zone.cur_x[p]) - i64::from(zone.org_x[p]), i64::from(zone.cur_y[p]) - i64::from(zone.org_y[p]));
        Some((z, p, f26dot6::mul_fix(d, self.move_vector.0), f26dot6::mul_fix(d, self.move_vector.1)))
    }

    // The loop count's points, top first; None when the stack holds too few, which FreeType skips.
    fn loop_points(&mut self) -> Run<Option<Vec<usize>>> {
        let n = self.gs().loop_count.max(0) as usize;
        self.interp.gs.loop_count = 1;
        if self.interp.stack.len() < n {
            return Ok(None);
        }
        self.charge(n)?;
        let at = self.interp.stack.len() - n;
        let points = self.interp.stack.drain(at..).rev().map(Self::point).collect();
        Ok(Some(points))
    }

    fn shp(&mut self, op: u8) -> Run {
        let n = self.gs().loop_count.max(0) as usize;
        if self.interp.stack.len() < n {
            self.interp.gs.loop_count = 1;
            return Ok(());
        }
        let at = self.interp.stack.len() - n;
        let Some((_, _, dx, dy)) = self.displacement(op) else {
            self.interp.stack.truncate(at);
            return Ok(());
        };
        let Some(points) = self.loop_points()? else { return Ok(()) };
        let z = self.gs().zp2;
        for p in points {
            if self.in_zone(z, p) {
                self.shift_zp2(p, dx, dy, true);
            }
        }
        Ok(())
    }

    fn shc(&mut self, op: u8) -> Run {
        let contour = Self::point(self.pop());
        let z = self.gs().zp2;
        let bound = if z == 0 { 1 } else { self.zones[1].contour_ends.len() };
        if contour >= bound {
            return Ok(());
        }
        let Some((rz, rp, dx, dy)) = self.displacement(op) else { return Ok(()) };
        let ends = &self.zones[1].contour_ends;
        let start = if contour == 0 { 0 } else { ends[contour - 1] + 1 };
        let limit = if z == 0 { self.len_of(0) } else { ends[contour] + 1 };
        self.charge(limit.saturating_sub(start))?;
        for i in start..limit.min(self.len_of(z)) {
            if rz != z || rp != i {
                self.shift_zp2(i, dx, dy, true);
            }
        }
        Ok(())
    }

    // SHZ shifts zp2, whichever zone it names (it only checks the name), and leaves the phantom
    // points and the touched flags alone.
    fn shz(&mut self, op: u8) -> Run {
        let named = self.pop();
        if !(0..2).contains(&named) {
            return Ok(());
        }
        let Some((rz, rp, dx, dy)) = self.displacement(op) else { return Ok(()) };
        let z = self.gs().zp2;
        let limit = if z == 0 { self.len_of(0) } else { self.zones[1].contour_ends.last().map_or(0, |&e| e + 1) };
        self.charge(limit)?;
        for i in 0..limit.min(self.len_of(z)) {
            if rz != z || rp != i {
                self.shift_zp2(i, dx, dy, false);
            }
        }
        Ok(())
    }

    // SHPIX moves along the freedom vector alone. Under v40's backward compatibility it moves only
    // twilight points, and before IUP finishes only y of points touched in y (or any in a composite).
    fn shpix(&mut self) -> Run {
        let amount = self.pop();
        let Some(points) = self.loop_points()? else { return Ok(()) };
        let gs = self.gs();
        let (f, z) = (gs.freedom, gs.zp2);
        let in_twilight = gs.zp0 == 0 || gs.zp1 == 0 || gs.zp2 == 0;
        let (dx, dy) = (f26dot6::mul_fix14(amount, f.x), f26dot6::mul_fix14(amount, f.y));
        let compat = self.interp.compat;
        for p in points {
            if !self.in_zone(z, p) {
                continue;
            }
            if compat == 0 {
                self.shift_zp2(p, dx, dy, true);
            } else if in_twilight
                || (compat != COMPAT_DONE
                    && ((self.is_composite && f.y != 0) || self.zones[z].flags[p] & FLAG_TOUCHED_Y != 0))
            {
                self.shift_zp2(p, 0, dy, true);
            }
        }
        Ok(())
    }

    fn ip(&mut self) -> Run {
        let n = self.gs().loop_count.max(0) as usize;
        if self.interp.stack.len() < n {
            self.interp.gs.loop_count = 1;
            return Ok(());
        }
        let gs = self.gs();
        let (z0, z1, z2, rp1, rp2) = (gs.zp0, gs.zp1, gs.zp2, gs.rp1, gs.rp2);
        if !self.in_zone(z0, rp1) {
            let at = self.interp.stack.len() - n;
            self.interp.stack.truncate(at);
            self.interp.gs.loop_count = 1;
            return Ok(());
        }
        let twilight = z0 == 0 || z1 == 0 || z2 == 0;
        // Outside the twilight zone, from the unscaled points: only the ratio of two such distances
        // is used, so FreeType leaves them in font units, and so the one quirk below is in them too.
        let original = |m: &Self, z: usize, p: usize| -> i32 {
            let (a, b) = (&m.zones[z], &m.zones[z0]);
            if twilight {
                m.dual_project(i64::from(a.org_x[p]) - i64::from(b.org_x[rp1]), i64::from(a.org_y[p]) - i64::from(b.org_y[rp1]))
            } else {
                m.dual_project(i64::from(a.orus_x[p]) - i64::from(b.orus_x[rp1]), i64::from(a.orus_y[p]) - i64::from(b.orus_y[rp1]))
            }
        };
        let (old_range, cur_range) = if self.in_zone(z1, rp2) {
            (original(self, z1, rp2), self.project_cur(z1, rp2, z0, rp1))
        } else {
            (0, 0)
        };
        let Some(points) = self.loop_points()? else { return Ok(()) };
        for p in points {
            if !self.in_zone(z2, p) {
                continue;
            }
            let org = original(self, z2, p);
            let cur = self.project_cur(z2, p, z0, rp1);
            // With no original range Windows moves the point to its original distance, which here is
            // in font units, as it is in FreeType.
            let new = if org == 0 {
                0
            } else if old_range != 0 {
                f26dot6::mul_div(org, cur_range, old_range)
            } else {
                org
            };
            self.move_point(z2, p, new.wrapping_sub(cur));
        }
        Ok(())
    }

    fn alignrp(&mut self) -> Run {
        let n = self.gs().loop_count.max(0) as usize;
        if self.interp.stack.len() < n {
            self.interp.gs.loop_count = 1;
            return Ok(());
        }
        let (z0, z1, rp0) = (self.gs().zp0, self.gs().zp1, self.gs().rp0);
        if !self.in_zone(z0, rp0) {
            let at = self.interp.stack.len() - n;
            self.interp.stack.truncate(at);
            self.interp.gs.loop_count = 1;
            return Ok(());
        }
        let Some(points) = self.loop_points()? else { return Ok(()) };
        for p in points {
            if self.in_zone(z1, p) {
                let d = self.project_cur(z1, p, z0, rp0);
                self.move_point(z1, p, d.wrapping_neg());
            }
        }
        Ok(())
    }

    fn flippt(&mut self) -> Run {
        let Some(points) = self.loop_points()? else { return Ok(()) };
        if self.interp.compat == COMPAT_DONE {
            return Ok(());
        }
        for p in points {
            if self.in_zone(1, p) {
                self.zones[1].flags[p] ^= FLAG_ON_CURVE;
            }
        }
        Ok(())
    }

    fn fliprg(&mut self, op: u8) {
        let (hi, lo) = (Self::point(self.pop()), Self::point(self.pop()));
        if self.interp.compat == COMPAT_DONE || !self.in_zone(1, hi) || !self.in_zone(1, lo) {
            return;
        }
        for f in self.zones[1].flags.get_mut(lo..=hi).into_iter().flatten() {
            if op == 0x81 { *f |= FLAG_ON_CURVE } else { *f &= !FLAG_ON_CURVE }
        }
    }

    // DELTAP and DELTAC: pairs whose ppem nibble matches move a point, or a CVT entry, by the
    // step nibble in units of 1/2^delta_shift pixel.
    fn delta(&mut self, op: u8, cvt: bool) -> Run {
        let n = self.pop();
        let pairs = self.interp.stack.len() / 2;
        let n = if n < 0 || n as usize > pairs { pairs } else { n as usize };
        self.charge(n)?;
        let at = self.interp.stack.len() - 2 * n;
        let args: Vec<i32> = self.interp.stack.drain(at..).collect();
        let shift = match op {
            0x5D | 0x73 => 0,
            0x71 | 0x74 => 16,
            _ => 32,
        };
        let ppem = i32::from(self.interp.size.ppem()) - self.gs().delta_base - shift;
        if !(0..16).contains(&ppem) {
            return Ok(());
        }
        let step = 1 << (6 - self.gs().delta_shift);
        let z = self.gs().zp0;
        let (compat, fy) = (self.interp.compat, self.gs().freedom.y);
        for &[b, a] in args.as_chunks::<2>().0.iter().rev() {
            if b & 0xF0 != ppem << 4 {
                continue;
            }
            let mut amount = (b & 0xF) - 8;
            if amount >= 0 {
                amount += 1;
            }
            let amount = amount * step;
            if cvt {
                if let Some(slot) = self.interp.cvt.get_mut(Self::index(a)) {
                    *slot = slot.wrapping_add(amount);
                }
                continue;
            }
            let p = Self::point(a);
            if !self.in_zone(z, p) {
                continue;
            }
            let allowed = compat == 0
                || (compat != COMPAT_DONE && ((self.is_composite && fy != 0) || self.zones[z].flags[p] & FLAG_TOUCHED_Y != 0));
            if allowed {
                self.move_point(z, p, amount);
            }
        }
        Ok(())
    }

    // IUP over the glyph zone, interpolating untouched points from the unscaled outline as FreeType
    // does. Under v40's backward compatibility it runs once per axis and no more.
    fn iup(&mut self, op: u8) -> Run {
        let x_axis = op & 1 != 0;
        if self.interp.compat == COMPAT_DONE {
            return Ok(());
        }
        if self.interp.compat != 0 {
            self.interp.compat |= 1 << (op & 1);
        }
        let zone = &mut *self.zones[1];
        if zone.contour_ends.is_empty() {
            return Ok(());
        }
        let n = zone.len();
        self.work = self.work.saturating_add(n);
        if self.work > MAX_WORK {
            return Err(Stop::TooLong);
        }
        let mask = if x_axis { FLAG_TOUCHED_X } else { FLAG_TOUCHED_Y };
        let mut iup = Iup {
            org: if x_axis { &zone.org_x } else { &zone.org_y },
            orus: if x_axis { &zone.orus_x } else { &zone.orus_y },
            cur: if x_axis { &mut zone.cur_x } else { &mut zone.cur_y },
        };
        let mut point = 0usize;
        for &end in &zone.contour_ends {
            let first = point;
            let end = if end >= n { n - 1 } else { end };
            while point <= end && zone.flags[point] & mask == 0 {
                point += 1;
            }
            if point <= end {
                let first_touched = point;
                let mut cur_touched = point;
                point += 1;
                while point <= end {
                    if zone.flags[point] & mask != 0 {
                        iup.interpolate(cur_touched + 1, point - 1, cur_touched, point);
                        cur_touched = point;
                    }
                    point += 1;
                }
                if cur_touched == first_touched {
                    iup.shift(first, end, cur_touched);
                } else {
                    iup.interpolate(cur_touched + 1, end, cur_touched, first_touched);
                    if first_touched > 0 {
                        iup.interpolate(first, first_touched - 1, cur_touched, first_touched);
                    }
                }
            }
            point = point.max(end + 1);
        }
        Ok(())
    }

    fn get_info(&self, selector: i32) -> i32 {
        let v40 = self.interp.mode.is_v40();
        let mut k = 0;
        if selector & 1 != 0 {
            k = if v40 { 40 } else { 35 };
        }
        if selector & 8 != 0 && self.interp.variable {
            k |= 1 << 10;
        }
        if v40 {
            // Grayscale ClearType, as FreeType reports for its normal target: subpixel hinting,
            // positioning, symmetric smoothing and gray ClearType.
            for (bit, result) in [(64, 13), (1024, 17), (2048, 18), (4096, 19)] {
                if selector & bit != 0 {
                    k |= 1 << result;
                }
            }
        } else if selector & 32 != 0 {
            k |= 1 << 12;
        }
        k
    }

    // INSTCTRL: the CVT program sets selectors 1 to 3; a glyph program may only lift v40's
    // backward compatibility for itself, through selector 3.
    fn instctrl(&mut self) {
        let (selector, value) = (self.pop(), self.pop());
        if !(1..=3).contains(&selector) {
            return;
        }
        let flag = 1u8 << (selector - 1);
        if value != 0 && value != i32::from(flag) {
            return;
        }
        match self.initial {
            ProgramKind::ControlValue => {
                let gs = &mut self.interp.gs;
                gs.instruct_control = (gs.instruct_control & !flag) | value as u8;
            }
            ProgramKind::Glyph if selector == 3 && self.interp.mode.is_v40() => {
                self.interp.compat = (value as u8 & 4) ^ 4;
            }
            _ => {}
        }
    }
}

enum Slot {
    Function(usize),
    Instruction(usize),
}

// A distance along a vector. One component at exactly 1.0 reads that axis alone, whatever the other
// holds, as FreeType's Project_x and Project_y do: a normalized near-axis vector such as (1.0, -34)
// would otherwise move points a 64th from where FreeType puts them.
fn along(v: Vector, dx: i64, dy: i64) -> i32 {
    if v.x == 0x4000 {
        f26dot6::clamp_i32(dx)
    } else if v.y == 0x4000 {
        f26dot6::clamp_i32(dy)
    } else {
        f26dot6::dot_fix14(dx, dy, v.x, v.y)
    }
}

fn minimum_distance(org: i32, distance: i32, minimum: i32) -> i32 {
    if org >= 0 { distance.max(minimum) } else { distance.min(minimum.saturating_neg()) }
}

struct Iup<'a> {
    org: &'a [i32],
    orus: &'a [i32],
    cur: &'a mut [i32],
}

impl Iup<'_> {
    fn shift(&mut self, first: usize, end: usize, touched: usize) {
        let d = self.cur[touched].wrapping_sub(self.org[touched]);
        if d == 0 {
            return;
        }
        for i in (first..=end).filter(|&i| i != touched) {
            self.cur[i] = self.cur[i].wrapping_add(d);
        }
    }

    // Between two touched points, by their unscaled positions, with the scale divided once per
    // segment; beyond them, by the nearer one's move.
    fn interpolate(&mut self, from: usize, to: usize, mut r1: usize, mut r2: usize) {
        if from > to || r1 >= self.cur.len() || r2 >= self.cur.len() {
            return;
        }
        if self.orus[r1] > self.orus[r2] {
            (r1, r2) = (r2, r1);
        }
        let (orus1, orus2) = (self.orus[r1], self.orus[r2]);
        let (org1, org2) = (self.org[r1], self.org[r2]);
        let (cur1, cur2) = (self.cur[r1], self.cur[r2]);
        let (d1, d2) = (cur1.wrapping_sub(org1), cur2.wrapping_sub(org2));
        let mut scale = None;
        for i in from..=to {
            let x = self.org[i];
            self.cur[i] = if x <= org1 {
                x.wrapping_add(d1)
            } else if x >= org2 {
                x.wrapping_add(d2)
            } else if cur1 == cur2 || orus1 == orus2 {
                cur1
            } else {
                let s = *scale.get_or_insert_with(|| f26dot6::div_fix(cur2.wrapping_sub(cur1), orus2.wrapping_sub(orus1)));
                cur1.wrapping_add(f26dot6::mul_fix(self.orus[i].wrapping_sub(orus1), s))
            };
        }
    }
}
