use alloc::vec::Vec;
use super::cleartype::ClearType;
use super::f26dot6::{self, ONE};
use super::state::{GraphicsState, Round, Vector, Zone};

mod geometry;

// Bytecode can loop forever, so a run spends from a budget in units of about one machine instruction: an opcode costs
// its price in the decode table, and work no opcode count sees costs its price an element.
const OPCODE: u16 = 64;
// Vectors made from points or the stack, and ISECT, take a square root each; a point IP moves, a division.
const VECTOR: u16 = 256;
const INTERPOLATED: u64 = 256;
// A point ALIGNRP moves, one SHP, SHPIX, SHC, SHZ or FLIPPT does, a DELTA pair, an IUP point, an opcode skipped or
// scanned.
const ALIGNED: u64 = 128;
const MOVED: u64 = 64;
const DELTA: u64 = 32;
const IUP_POINT: u64 = 16;
const SKIPPED: u64 = 32;
const DEFINED: u64 = 64;
// A flag FLIPRG sets, a stack value pushed or moved, a point an IP with no reference point passes over.
const ELEMENT: u64 = 1;
// A call deeper than any before in the run, for the 24 bytes its frame holds: frames stay within a budget's 1/4096.
const FRAME: u64 = 4096;
// Every run may spend BASE, about ten times what the costliest real glyph spends; the font and CVT programs PER_BYTE
// more a byte of program or CVT and PER_POINT a storage slot or twilight point, and a glyph PER_POINT a point.
pub(crate) const BASE: u64 = 1 << 26;
pub(crate) const PER_BYTE: u64 = 128;
pub(crate) const PER_POINT: u64 = 1 << 11;
// maxp's maxStackElements and maxFunctionDefs are uint16 and the instruction set names no error past what they
// declare, so a program is held to the most such a field could declare.
const MAX_STACK: usize = 0xFFFF;
const MAX_FUNCTION: usize = 0xFFFF;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Program {
    Font,
    ControlValue,
    Glyph,
}

// A function or instruction definition: the program holding it, and its body from the opcode after FDEF or IDEF to
// the ENDF closing it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Body {
    program: Program,
    start: u32,
    end: u32,
}

// Why a run stopped short. A glyph program keeps what it did; the font or CVT program stopping leaves the size
// unhinted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stop {
    PastEnd,
    StackOverflow,
    StackUnderflow,
    UnknownOpcode,
    UndefinedFunction,
    BadArgument,
    DivideByZero,
    MisplacedDefinition,
    StrayEndf,
    TooManyDefinitions,
    Debug,
    OverBudget,
}

type Step<T = ()> = Result<T, Stop>;

// What the CVT program set through INSTCTRL: selector 1 turns glyph programs off, 2 starts them from the default
// graphics state and, per Microsoft's ClearType page, cancels 3, which declares native ClearType.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Control {
    pub glyphs_off: bool,
    pub glyph_defaults: bool,
    pub native: bool,
}

// Which parts of the CVT program's state a glyph program wrote, so only those are put back for the next glyph.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Dirty {
    pub cvt: bool,
    pub storage: bool,
    pub twilight: bool,
}

// A call in progress: where its body runs, how many more times (LOOPCALL), and where to go back to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    body: Body,
    left: u32,
    caller: Program,
    back: u32,
}

// Where an untaken branch in the font program lands, kept because its functions run for every glyph: `slot` holds,
// for an IF or ELSE's IP, 1 + the index of its landing and the opcodes passed, or 0 before the first skip from it.
#[derive(Clone, Default, Debug)]
pub(crate) struct Skips {
    slot: Vec<u16>,
    landings: Vec<(u32, u32)>,
}

impl Skips {
    pub(crate) fn clear(&mut self) {
        self.slot.clear();
        self.landings.clear();
    }
}

// What a size keeps between its programs: the storage and CVT they share, the definitions the font and CVT programs
// made, GETINFO's and GETVARIATION's answers, and buffers each run reuses.
#[derive(Clone)]
pub(crate) struct Interpreter {
    pub size: f26dot6::Size,
    pub scale16: i64,
    pub variable: bool,
    // GETINFO's symmetric smoothing at this size, as gasp sets it.
    pub symmetric: bool,
    // GETVARIATION's normalized coordinates in 2.14, zero at a variable font's default.
    pub axes: Vec<i32>,
    pub storage: Vec<i32>,
    pub cvt: Vec<i32>,
    pub functions: Vec<Option<Body>>,
    pub idefs: Vec<(u8, Body)>,
    // maxp's maxInstructionDefs: IDEFs are looked up by scanning, so the table holds no more than the font declares.
    pub idef_limit: usize,
    pub stack: Vec<i32>,
    pub frames: Vec<Frame>,
    pub skips: Skips,
    pub dirty: Dirty,
    // What the next run may spend: set for each font or CVT program, and for a hinted glyph shared by every program
    // of the glyphs it loads.
    pub left: u64,
}

impl Interpreter {
    pub(crate) fn heap_bytes(&self) -> usize {
        use crate::daecore::daekit::base::sync::heap_of;
        heap_of(&self.axes) + heap_of(&self.storage) + heap_of(&self.cvt) + heap_of(&self.functions)
            + heap_of(&self.idefs) + heap_of(&self.stack) + heap_of(&self.frames) + heap_of(&self.skips.slot)
            + heap_of(&self.skips.landings)
    }

    // `units` spent outside a run, as a run spends; false where the budget can't pay them, which leaves none.
    pub(crate) fn spend(&mut self, units: u64) -> bool {
        let paid = units <= self.left;
        self.left = self.left.saturating_sub(units);
        paid
    }

    // Runs one program from `state`, left as the program ends it, under `rules` over the twilight zone and a glyph's
    // (empty for the font and CVT programs). Returns what INSTCTRL set in the CVT program.
    pub(crate) fn run(
        &mut self, programs: Programs<'_>, run: Run<'_>, twilight: &mut Zone, glyph: &mut Zone,
    ) -> Step<Control> {
        let stack = core::mem::take(&mut self.stack);
        let mut exec = Exec {
            unscaled: if run.composite { 0x10000 } else { self.scale16 },
            left: self.left,
            vm: self,
            stack,
            programs,
            zones: [twilight, glyph],
            gs: run.state,
            rules: run.rules,
            program: run.program,
            composite: run.composite,
            deepest: 0,
            motion: Motion::default(),
            control: Control::default(),
        };
        exec.stack.clear();
        exec.vm.frames.clear();
        exec.refresh_motion();
        let ended = exec.execute();
        let Exec { stack, control, left, .. } = exec;
        (self.stack, self.left) = (stack, left);
        ended.map(|()| control)
    }
}

// How a run starts: the program, its graphics state and `ClearType` state, and whether the glyph is a composite's own.
pub(crate) struct Run<'a> {
    pub program: Program,
    pub state: &'a mut GraphicsState,
    pub rules: ClearType,
    pub composite: bool,
}

pub(crate) struct Programs<'a> {
    pub font: &'a [u8],
    pub control_value: &'a [u8],
    pub glyph: &'a [u8],
}

impl<'a> Programs<'a> {
    fn get(&self, program: Program) -> &'a [u8] {
        match program {
            Program::Font => self.font,
            Program::ControlValue => self.control_value,
            Program::Glyph => self.glyph,
        }
    }
}

// What each projected 26.6 unit moves a point by along x and y, in 16.16, and which axes a move reaches.
#[derive(Clone, Copy, Default)]
struct Motion {
    x: i32,
    y: i32,
    reach: Reach,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Reach {
    x: bool,
    y: bool,
}

struct Exec<'a> {
    vm: &'a mut Interpreter,
    // The interpreter's stack, held here for the run so an instruction reaches it without going through `vm`.
    stack: Vec<i32>,
    programs: Programs<'a>,
    zones: [&'a mut Zone; 2],
    gs: &'a mut GraphicsState,
    rules: ClearType,
    program: Program,
    composite: bool,
    // Font units to 26.6 in 16.16 for unscaled distances; 1.0 for a composite's own program, whose unscaled points
    // are its assembled ones.
    unscaled: i64,
    left: u64,
    // The most frames the run has held, each charged once.
    deepest: usize,
    motion: Motion,
    control: Control,
}

// Each opcode as the dispatch knows it, with the values it pops and pushes; a loop instruction's points, DELTA's pairs
// and the pushes are counted where they're read.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Op {
    VectorsToAxis,
    VectorToLine,
    VectorFromStack,
    GetVector,
    FreedomToProjection,
    Intersect,
    SetReference,
    SetZone,
    SetLoop,
    SetRound,
    SetMinimumDistance,
    Else,
    Jump,
    SetCutIn,
    SetSingleWidthCutIn,
    SetSingleWidth,
    Duplicate,
    Pop,
    Clear,
    Swap,
    Depth,
    CopyIndexed,
    MoveIndexed,
    AlignPoints,
    Untouch,
    LoopCall,
    Call,
    DefineFunction,
    EndFunction,
    MoveDirectAbsolute,
    InterpolateUntouched,
    ShiftPoints,
    ShiftContour,
    ShiftZone,
    ShiftByPixels,
    Interpolate,
    MoveStackIndirect,
    AlignToReference,
    MoveIndirectAbsolute,
    PushCounted,
    WriteStorage,
    ReadStorage,
    WriteCvtPixels,
    ReadCvt,
    GetCoordinate,
    SetCoordinate,
    MeasureDistance,
    MeasurePpem,
    MeasurePointSize,
    SetAutoFlip,
    Debug,
    Compare,
    OddOrEven,
    If,
    EndIf,
    Logic,
    Not,
    DeltaPoint,
    SetDeltaBase,
    SetDeltaShift,
    Arithmetic,
    Absolute,
    Negate,
    Floor,
    Ceiling,
    RoundValue,
    CompensateOnly,
    WriteCvtUnits,
    DeltaCvt,
    SuperRound,
    JumpIf,
    Discard,
    FlipPoint,
    FlipRange,
    DualToLine,
    GetInfo,
    DefineInstruction,
    Roll,
    MaxOrMin,
    InstructionControl,
    GetVariation,
    Push,
    MoveDirectRelative,
    MoveIndirectRelative,
    Unknown,
}

#[derive(Clone, Copy)]
struct Decoded {
    op: Op,
    pops: u8,
    pushes: u8,
    price: u16,
}

// The instruction set's opcode map: what each opcode is, its fixed pops and pushes, and what it costs.
const fn decode(byte: u8) -> Decoded {
    use Op::*;
    let (op, pops, pushes) = match byte {
        0x00..=0x05 => (VectorsToAxis, 0, 0),
        0x06..=0x09 => (VectorToLine, 2, 0),
        0x0A | 0x0B => (VectorFromStack, 2, 0),
        0x0C | 0x0D => (GetVector, 0, 2),
        0x0E => (FreedomToProjection, 0, 0),
        0x0F => (Intersect, 5, 0),
        0x10..=0x12 => (SetReference, 1, 0),
        0x13..=0x16 => (SetZone, 1, 0),
        0x17 => (SetLoop, 1, 0),
        0x18 | 0x19 | 0x3D | 0x7A | 0x7C | 0x7D => (SetRound, 0, 0),
        0x1A => (SetMinimumDistance, 1, 0),
        0x1B => (Else, 0, 0),
        0x1C => (Jump, 1, 0),
        0x1D => (SetCutIn, 1, 0),
        0x1E => (SetSingleWidthCutIn, 1, 0),
        0x1F => (SetSingleWidth, 1, 0),
        0x20 => (Duplicate, 1, 2),
        0x21 => (Pop, 1, 0),
        0x22 => (Clear, 0, 0),
        0x23 => (Swap, 2, 2),
        0x24 => (Depth, 0, 1),
        0x25 => (CopyIndexed, 1, 1),
        0x26 => (MoveIndexed, 1, 0),
        0x27 => (AlignPoints, 2, 0),
        0x29 => (Untouch, 1, 0),
        0x2A => (LoopCall, 2, 0),
        0x2B => (Call, 1, 0),
        0x2C => (DefineFunction, 1, 0),
        0x2D => (EndFunction, 0, 0),
        0x2E | 0x2F => (MoveDirectAbsolute, 1, 0),
        0x30 | 0x31 => (InterpolateUntouched, 0, 0),
        0x32 | 0x33 => (ShiftPoints, 0, 0),
        0x34 | 0x35 => (ShiftContour, 1, 0),
        0x36 | 0x37 => (ShiftZone, 1, 0),
        0x38 => (ShiftByPixels, 1, 0),
        0x39 => (Interpolate, 0, 0),
        0x3A | 0x3B => (MoveStackIndirect, 2, 0),
        0x3C => (AlignToReference, 0, 0),
        0x3E | 0x3F => (MoveIndirectAbsolute, 2, 0),
        0x40 | 0x41 => (PushCounted, 0, 0),
        0x42 => (WriteStorage, 2, 0),
        0x43 => (ReadStorage, 1, 1),
        0x44 => (WriteCvtPixels, 2, 0),
        0x45 => (ReadCvt, 1, 1),
        0x46 | 0x47 => (GetCoordinate, 1, 1),
        0x48 => (SetCoordinate, 2, 0),
        0x49 | 0x4A => (MeasureDistance, 2, 1),
        0x4B => (MeasurePpem, 0, 1),
        0x4C => (MeasurePointSize, 0, 1),
        0x4D | 0x4E => (SetAutoFlip, 0, 0),
        0x4F => (Debug, 1, 0),
        0x50..=0x55 => (Compare, 2, 1),
        0x56 | 0x57 => (OddOrEven, 1, 1),
        0x58 => (If, 1, 0),
        0x59 => (EndIf, 0, 0),
        0x5A | 0x5B => (Logic, 2, 1),
        0x5C => (Not, 1, 1),
        0x5D | 0x71 | 0x72 => (DeltaPoint, 1, 0),
        0x5E => (SetDeltaBase, 1, 0),
        0x5F => (SetDeltaShift, 1, 0),
        0x60..=0x63 => (Arithmetic, 2, 1),
        0x64 => (Absolute, 1, 1),
        0x65 => (Negate, 1, 1),
        0x66 => (Floor, 1, 1),
        0x67 => (Ceiling, 1, 1),
        0x68..=0x6B => (RoundValue, 1, 1),
        0x6C..=0x6F => (CompensateOnly, 1, 1),
        0x70 => (WriteCvtUnits, 2, 0),
        0x73..=0x75 => (DeltaCvt, 1, 0),
        0x76 | 0x77 => (SuperRound, 1, 0),
        0x78 | 0x79 => (JumpIf, 2, 0),
        0x7E | 0x7F | 0x85 | 0x8D => (Discard, 1, 0),
        0x80 => (FlipPoint, 0, 0),
        0x81 | 0x82 => (FlipRange, 2, 0),
        0x86 | 0x87 => (DualToLine, 2, 0),
        0x88 => (GetInfo, 1, 1),
        0x89 => (DefineInstruction, 1, 0),
        0x8A => (Roll, 3, 3),
        0x8B | 0x8C => (MaxOrMin, 2, 1),
        0x8E => (InstructionControl, 2, 0),
        0x91 => (GetVariation, 0, 0),
        0xB0..=0xB7 => (Push, 0, byte - 0xAF),
        0xB8..=0xBF => (Push, 0, byte - 0xB7),
        0xC0..=0xDF => (MoveDirectRelative, 1, 0),
        0xE0..=0xFF => (MoveIndirectRelative, 2, 0),
        _ => (Unknown, 0, 0),
    };
    let price = match op {
        VectorToLine | VectorFromStack | DualToLine | Intersect => VECTOR,
        _ => OPCODE,
    };
    Decoded { op, pops, pushes, price }
}

const DECODED: [Decoded; 256] = {
    let mut table = [Decoded { op: Op::Unknown, pops: 0, pushes: 0, price: 0 }; 256];
    let mut byte = 0;
    while byte < 256 {
        table[byte] = decode(byte as u8);
        byte += 1;
    }
    table
};

// The bytes an instruction takes, its inline data included; None where NPUSHB or NPUSHW has no count byte.
fn width(code: &[u8], at: usize) -> Option<usize> {
    let byte = *code.get(at)?;
    Some(match byte {
        0x40 => 2 + usize::from(*code.get(at + 1)?),
        0x41 => 2 + 2 * usize::from(*code.get(at + 1)?),
        0xB0..=0xB7 => 1 + usize::from(byte - 0xAF),
        0xB8..=0xBF => 1 + 2 * usize::from(byte - 0xB7),
        _ => 1,
    })
}

impl Exec<'_> {
    fn execute(&mut self) -> Step {
        let mut program = self.program;
        let mut code = self.programs.get(program);
        let mut ip = 0usize;
        while ip < code.len() {
            let byte = code[ip];
            let Decoded { op, pops, pushes, price } = DECODED[usize::from(byte)];
            self.charge(1, u64::from(price))?;
            let depth = self.stack.len();
            if depth < usize::from(pops) {
                return Err(Stop::StackUnderflow);
            }
            if depth - usize::from(pops) + usize::from(pushes) > MAX_STACK {
                return Err(Stop::StackOverflow);
            }
            let mut next = ip + 1;
            match op {
                Op::Push => {
                    let words = byte >= 0xB8;
                    next = self.push_inline(code, ip + 1, usize::from(pushes), words)?;
                }
                Op::PushCounted => {
                    let n = usize::from(*code.get(ip + 1).ok_or(Stop::PastEnd)?);
                    if self.stack.len() + n > MAX_STACK {
                        return Err(Stop::StackOverflow);
                    }
                    self.charge(n, ELEMENT)?;
                    next = self.push_inline(code, ip + 2, n, byte == 0x41)?;
                }
                Op::MoveDirectRelative => self.move_direct_relative(byte),
                Op::MoveIndirectRelative => self.move_indirect_relative(byte),
                Op::If => {
                    if self.pop() == 0 {
                        next = self.skip(program, code, ip, true)?;
                    }
                }
                Op::Else => next = self.skip(program, code, ip, false)?,
                Op::EndIf => {}
                Op::Jump | Op::JumpIf => {
                    let taken = if op == Op::Jump { true } else { (self.pop() != 0) == (byte == 0x78) };
                    let offset = self.pop();
                    if taken {
                        next = self.jump_target(code, ip, offset)?;
                    }
                }
                Op::Call | Op::LoopCall => {
                    let id = self.pop();
                    let times = if op == Op::LoopCall { self.pop() } else { 1 };
                    let body = self.function(id)?;
                    if times > 0 {
                        self.enter(body, times as u32, program, ip + 1)?;
                        (program, code, next) = (body.program, self.programs.get(body.program), body.start as usize);
                    }
                }
                Op::EndFunction => {
                    let frame = self.vm.frames.last_mut().ok_or(Stop::StrayEndf)?;
                    frame.left -= 1;
                    if frame.left > 0 {
                        next = frame.body.start as usize;
                    } else {
                        let Frame { caller, back, .. } = *frame;
                        self.vm.frames.pop();
                        (program, code, next) = (caller, self.programs.get(caller), back as usize);
                    }
                }
                Op::DefineFunction | Op::DefineInstruction => {
                    next = self.define(program, code, ip, op == Op::DefineFunction)?;
                }
                Op::GetVariation if self.vm.variable => {
                    let n = self.vm.axes.len();
                    if self.stack.len() + n > MAX_STACK {
                        return Err(Stop::StackOverflow);
                    }
                    // A value an axis, charged as pushed, as a CLEAR after it makes room again at once.
                    self.charge(n, ELEMENT)?;
                    self.stack.extend_from_slice(&self.vm.axes);
                }
                Op::Unknown | Op::GetVariation => {
                    let body = self.vm.idefs.iter().find(|(o, _)| *o == byte).map(|&(_, b)| b);
                    let body = body.ok_or(Stop::UnknownOpcode)?;
                    self.enter(body, 1, program, ip + 1)?;
                    (program, code, next) = (body.program, self.programs.get(body.program), body.start as usize);
                }
                _ => self.simple(op, byte)?,
            }
            ip = next;
        }
        if self.vm.frames.is_empty() { Ok(()) } else { Err(Stop::PastEnd) }
    }

    // The instructions that change no flow: stack, storage, state, measures and the point instructions.
    fn simple(&mut self, op: Op, byte: u8) -> Step {
        match op {
            Op::VectorsToAxis => self.vectors_to_axis(byte),
            Op::VectorToLine => self.vector_to_line(byte),
            Op::VectorFromStack => self.vector_from_stack(byte),
            Op::GetVector => {
                let v = if byte == 0x0C { self.gs.projection } else { self.gs.freedom };
                self.push(v.x);
                self.push(v.y);
            }
            Op::FreedomToProjection => {
                self.gs.freedom = self.gs.projection;
                self.refresh_motion();
            }
            Op::DualToLine => self.dual_to_line(byte),
            Op::Intersect => self.intersect(),
            Op::SetReference => {
                let p = self.pop_point();
                self.gs.rp[usize::from(byte - 0x10)] = p;
            }
            Op::SetZone => self.set_zone(byte),
            Op::SetLoop => {
                let n = self.pop();
                // A count past 65,535 is as good as one, every point it names already costing work.
                self.gs.repeat = u32::try_from(n).map_err(|_| Stop::BadArgument)?.min(0xFFFF);
            }
            Op::SetRound => {
                self.gs.round = match byte {
                    0x18 => Round::Grid,
                    0x19 => Round::HalfGrid,
                    0x3D => Round::DoubleGrid,
                    0x7A => Round::Off,
                    0x7C => Round::UpToGrid,
                    _ => Round::DownToGrid,
                }
            }
            Op::SuperRound => {
                let selector = self.pop();
                // A pixel, and for S45ROUND half the square root of two, in 2.14.
                self.gs.round = Round::lattice(selector, if byte == 0x76 { 0x4000 } else { 0x2D41 });
            }
            Op::SetMinimumDistance => self.gs.minimum_distance = self.pop(),
            Op::SetCutIn => self.gs.cut_in = self.pop(),
            Op::SetSingleWidthCutIn => self.gs.single_width_cut_in = self.pop(),
            Op::SetSingleWidth => {
                let units = self.pop();
                self.gs.single_width = f26dot6::scale_units(units, self.vm.scale16);
            }
            Op::SetAutoFlip => self.gs.auto_flip = byte == 0x4D,
            Op::SetDeltaBase => self.gs.delta_base = self.pop() as u32,
            // A step of 1/2^shift pixel is a whole number of 64ths only up to a shift of 6.
            Op::SetDeltaShift => {
                self.gs.delta_shift = u32::try_from(self.pop()).ok().filter(|&s| s <= 6).ok_or(Stop::BadArgument)?;
            }
            Op::Duplicate => {
                let v = self.top();
                self.push(v);
            }
            Op::Pop | Op::Discard => {
                self.pop();
            }
            Op::Clear => self.stack.clear(),
            Op::Swap => {
                let n = self.stack.len();
                self.stack.swap(n - 1, n - 2);
            }
            Op::Depth => {
                let n = self.stack.len() as i32;
                self.push(n);
            }
            Op::CopyIndexed => {
                let at = self.indexed()?;
                self.push(self.stack[at]);
            }
            Op::MoveIndexed => {
                let at = self.indexed()?;
                self.charge(self.stack.len() - at, ELEMENT)?;
                let v = self.stack.remove(at);
                self.push(v);
            }
            Op::Roll => {
                let n = self.stack.len();
                self.stack[n - 3..].rotate_left(1);
            }
            Op::WriteStorage => {
                let (value, at) = (self.pop(), self.pop());
                if let Some(slot) = self.vm.storage.get_mut(index(at)) {
                    *slot = value;
                    self.vm.dirty.storage = true;
                }
            }
            Op::ReadStorage => {
                let at = self.pop();
                self.push(self.vm.storage.get(index(at)).copied().unwrap_or(0));
            }
            Op::WriteCvtPixels | Op::WriteCvtUnits => {
                let (value, at) = (self.pop(), self.pop());
                let value = if op == Op::WriteCvtUnits { f26dot6::scale_units(value, self.vm.scale16) } else { value };
                self.write_cvt(at, |_| value);
            }
            Op::ReadCvt => {
                let at = self.pop();
                self.push(self.vm.cvt.get(index(at)).copied().unwrap_or(0));
            }
            Op::MeasurePpem => self.push(i32::from(self.vm.size.ppem())),
            // MPS is in points; with no device resolution, a point is a pixel.
            Op::MeasurePointSize => self.push(self.vm.size.units()),
            Op::Debug => return Err(Stop::Debug),
            Op::Compare => {
                let (b, a) = (self.pop(), self.pop());
                let holds = match byte {
                    0x50 => a < b,
                    0x51 => a <= b,
                    0x52 => a > b,
                    0x53 => a >= b,
                    0x54 => a == b,
                    _ => a != b,
                };
                self.push(i32::from(holds));
            }
            Op::OddOrEven => {
                // A rounded value off whole pixels is neither odd nor even.
                let v = self.gs.round.apply(self.pop()) & (2 * ONE - 1);
                self.push(i32::from(v == if byte == 0x56 { ONE } else { 0 }));
            }
            Op::Logic => {
                let (b, a) = (self.pop() != 0, self.pop() != 0);
                self.push(i32::from(if byte == 0x5A { a && b } else { a || b }));
            }
            Op::Not => {
                let v = self.pop();
                self.push(i32::from(v == 0));
            }
            Op::Arithmetic => {
                let (b, a) = (self.pop(), self.pop());
                let v = match byte {
                    0x60 => a.saturating_add(b),
                    0x61 => a.saturating_sub(b),
                    0x62 if b == 0 => return Err(Stop::DivideByZero),
                    0x62 => f26dot6::ratio_truncated(a, ONE, b),
                    _ => f26dot6::ratio(a, b, ONE),
                };
                self.push(v);
            }
            Op::Absolute => {
                let v = self.pop();
                self.push(v.saturating_abs());
            }
            Op::Negate => {
                let v = self.pop();
                self.push(v.saturating_neg());
            }
            Op::Floor => {
                let v = self.pop();
                self.push(f26dot6::floor_pixel(v));
            }
            Op::Ceiling => {
                let v = self.pop();
                self.push(f26dot6::ceil_pixel(v));
            }
            Op::RoundValue => {
                let v = self.pop();
                self.push(self.gs.round.apply(v));
            }
            // NROUND only compensates for the device, which here needs none.
            Op::CompensateOnly => {}
            Op::MaxOrMin => {
                let (b, a) = (self.pop(), self.pop());
                self.push(if byte == 0x8B { a.max(b) } else { a.min(b) });
            }
            Op::DeltaPoint => self.delta_points(byte)?,
            Op::DeltaCvt => self.delta_cvt(byte)?,
            Op::GetInfo => {
                let selector = self.pop();
                self.push(self.info(selector));
            }
            Op::InstructionControl => self.instruction_control(),
            Op::MoveDirectAbsolute => self.move_direct_absolute(byte),
            Op::MoveIndirectAbsolute => self.move_indirect_absolute(byte),
            Op::MoveStackIndirect => self.move_stack_indirect(byte),
            Op::AlignPoints => self.align_points(),
            Op::AlignToReference => self.align_to_reference()?,
            Op::Untouch => self.untouch(),
            Op::InterpolateUntouched => self.interpolate_untouched(byte == 0x31)?,
            Op::ShiftPoints => self.shift_points(byte)?,
            Op::ShiftContour => self.shift_contour(byte)?,
            Op::ShiftZone => self.shift_zone(byte)?,
            Op::ShiftByPixels => self.shift_by_pixels()?,
            Op::Interpolate => self.interpolate()?,
            Op::FlipPoint => self.flip_points()?,
            Op::FlipRange => self.flip_range(byte == 0x81)?,
            Op::GetCoordinate => self.get_coordinate(byte == 0x47),
            Op::SetCoordinate => self.set_coordinate(),
            Op::MeasureDistance => self.measure_distance(byte == 0x4A),
            Op::Push | Op::PushCounted | Op::MoveDirectRelative | Op::MoveIndirectRelative | Op::If | Op::Else
            | Op::EndIf | Op::Jump | Op::JumpIf | Op::Call | Op::LoopCall | Op::EndFunction | Op::DefineFunction
            | Op::DefineInstruction | Op::GetVariation | Op::Unknown => {}
        }
        Ok(())
    }

    // `n` elements at `price` each; a run that can't pay stops, and leaves nothing for the next.
    fn charge(&mut self, n: usize, price: u64) -> Step {
        match self.left.checked_sub((n as u64).saturating_mul(price)) {
            Some(left) => self.left = left,
            None => {
                self.left = 0;
                return Err(Stop::OverBudget);
            }
        }
        Ok(())
    }

    // A missing value reads 0; each instruction's fixed pops are checked before it runs.
    fn pop(&mut self) -> i32 {
        self.stack.pop().unwrap_or(0)
    }

    fn top(&self) -> i32 {
        self.stack.last().copied().unwrap_or(0)
    }

    fn push(&mut self, v: i32) {
        self.stack.push(v);
    }

    // `n` bytes or words from `at` onto the stack, and where the next instruction starts.
    fn push_inline(&mut self, code: &[u8], at: usize, n: usize, words: bool) -> Step<usize> {
        let end = at + if words { 2 * n } else { n };
        let data = code.get(at..end).ok_or(Stop::PastEnd)?;
        if words {
            self.stack.extend(data.as_chunks::<2>().0.iter().map(|&w| i32::from(i16::from_be_bytes(w))));
        } else {
            self.stack.extend(data.iter().map(|&b| i32::from(b)));
        }
        Ok(end)
    }

    // CINDEX's and MINDEX's element: the popped depth, 1 the top, as a stack index.
    fn indexed(&mut self) -> Step<usize> {
        let k = self.pop();
        let k = usize::try_from(k).ok().filter(|&k| k > 0).ok_or(Stop::BadArgument)?;
        self.stack.len().checked_sub(k).ok_or(Stop::StackUnderflow)
    }

    fn write_cvt(&mut self, at: i32, value: impl FnOnce(i32) -> i32) {
        if let Some(slot) = self.vm.cvt.get_mut(index(at)) {
            *slot = value(*slot);
            self.vm.dirty.cvt = true;
        }
    }

    fn set_zone(&mut self, byte: u8) {
        let zone = match self.pop() {
            0 => 0,
            1 => 1,
            _ => return,
        };
        if zone == 0 {
            self.vm.dirty.twilight = true;
        }
        match byte {
            0x13..=0x15 => self.gs.zp[usize::from(byte - 0x13)] = zone,
            _ => self.gs.zp = [zone; 3],
        }
    }

    // A jump `offset` bytes from the instruction at `ip`. It lands inside the function it's in, or its program, whose
    // end it may reach.
    fn jump_target(&self, code: &[u8], ip: usize, offset: i32) -> Step<usize> {
        let target = ip as i64 + i64::from(offset);
        let (low, high) = match self.vm.frames.last() {
            Some(f) => (i64::from(f.body.start), i64::from(f.body.end)),
            None => (0, code.len() as i64),
        };
        if (low..=high).contains(&target) { Ok(target as usize) } else { Err(Stop::BadArgument) }
    }

    fn function(&self, id: i32) -> Step<Body> {
        self.vm.functions.get(index(id)).copied().flatten().ok_or(Stop::UndefinedFunction)
    }

    // Calls nest as deep as the budget pays for, the frames on the heap.
    fn enter(&mut self, body: Body, times: u32, caller: Program, back: usize) -> Step {
        if self.vm.frames.len() == self.deepest {
            self.charge(1, FRAME)?;
            self.deepest += 1;
        }
        self.vm.frames.push(Frame { body, left: times, caller, back: back as u32 });
        Ok(())
    }

    // Past an IF whose condition failed, to the ELSE or EIF at its depth (`to_else`), or past an ELSE to its EIF. The
    // opcodes passed are charged; the font program's landings are kept, as it never changes.
    fn skip(&mut self, program: Program, code: &[u8], ip: usize, to_else: bool) -> Step<usize> {
        let kept = program == Program::Font;
        if kept && let Some(&slot @ 1..) = self.vm.skips.slot.get(ip) {
            let (landing, cost) = self.vm.skips.landings[usize::from(slot) - 1];
            self.charge(cost as usize, SKIPPED)?;
            return Ok(landing as usize);
        }
        let (mut at, mut nesting, mut passed) = (ip + width(code, ip).unwrap_or(1), 0usize, 0usize);
        let landing = loop {
            passed += 1;
            let Some(len) = width(code, at) else { break None };
            match code[at] {
                0x58 => nesting += 1,
                0x1B if to_else && nesting == 0 => break Some(at + 1),
                0x59 if nesting == 0 => break Some(at + 1),
                0x59 => nesting -= 1,
                _ => {}
            }
            at += len;
        };
        self.charge(passed, SKIPPED)?;
        let landing = landing.ok_or(Stop::PastEnd)?;
        if kept && let Ok(slot) = u16::try_from(self.vm.skips.landings.len() + 1) {
            if self.vm.skips.slot.is_empty() {
                self.vm.skips.slot.resize(code.len(), 0);
            }
            self.vm.skips.slot[ip] = slot;
            self.vm.skips.landings.push((landing as u32, passed as u32));
        }
        Ok(landing)
    }

    // FDEF or IDEF at `ip`: its body runs to the ENDF closing it, with no definition inside. Only the font and CVT
    // programs define. Returns where the program goes on, past the ENDF.
    fn define(&mut self, program: Program, code: &[u8], ip: usize, function: bool) -> Step<usize> {
        if self.program == Program::Glyph {
            return Err(Stop::MisplacedDefinition);
        }
        let id = self.pop();
        let start = ip + 1;
        let mut at = start;
        loop {
            self.charge(1, DEFINED)?;
            match code.get(at) {
                None => return Err(Stop::PastEnd),
                Some(0x2C | 0x89) => return Err(Stop::MisplacedDefinition),
                Some(0x2D) => break,
                Some(_) => at += width(code, at).ok_or(Stop::PastEnd)?,
            }
        }
        let body = Body { program, start: start as u32, end: at as u32 };
        if function {
            let id = usize::try_from(id).ok().filter(|&id| id <= MAX_FUNCTION).ok_or(Stop::TooManyDefinitions)?;
            if self.vm.functions.len() <= id {
                self.vm.functions.resize(id + 1, None);
            }
            self.vm.functions[id] = Some(body);
        } else {
            let opcode = u8::try_from(id).map_err(|_| Stop::BadArgument)?;
            let limit = self.vm.idef_limit;
            let idefs = &mut self.vm.idefs;
            match idefs.iter().position(|(o, _)| *o == opcode) {
                Some(at) => idefs[at].1 = body,
                None if idefs.len() < limit => idefs.push((opcode, body)),
                None => return Err(Stop::TooManyDefinitions),
            }
        }
        Ok(at + 1)
    }

    // GETINFO by the instruction set's table: version 42 (variations, GETVARIATION), no rotation or stretch, variations
    // where the font varies, vertical phantoms, gray ClearType with subpixel positions, symmetric smoothing per gasp.
    fn info(&self, selector: i32) -> i32 {
        let answers = [
            (0, 42),
            (3, i32::from(self.vm.variable) << 10),
            (4, 1 << 11),
            (6, 1 << 13),
            (10, 1 << 17),
            (11, i32::from(self.vm.symmetric) << 18),
            (12, 1 << 19),
        ];
        answers.iter().filter(|&&(bit, _)| selector & (1 << bit) != 0).fold(0, |info, &(_, answer)| info | answer)
    }

    // INSTCTRL: selectors 1 and 2 in the CVT program only, 3 there or in a glyph program, where it holds for that
    // glyph alone; a value other than 0 or the selector's own flag is ignored.
    fn instruction_control(&mut self) {
        let (selector, value) = (self.pop(), self.pop());
        let on = match (selector, value) {
            (1..=3, 0) => false,
            (1, 1) | (2, 2) | (3, 4) => true,
            _ => return,
        };
        match (self.program, selector) {
            (Program::ControlValue, 1) => self.control.glyphs_off = on,
            (Program::ControlValue, 2) => self.control.glyph_defaults = on,
            (Program::ControlValue, _) => self.control.native = on,
            (Program::Glyph, 3) => self.rules.native = on,
            _ => {}
        }
    }

    fn vectors_to_axis(&mut self, byte: u8) {
        let axis = if byte & 1 != 0 { Vector::X_AXIS } else { Vector::Y_AXIS };
        if byte <= 0x03 {
            (self.gs.projection, self.gs.dual) = (axis, axis);
        }
        if byte <= 0x01 || byte >= 0x04 {
            self.gs.freedom = axis;
        }
        self.refresh_motion();
    }
}

// An index argument, read unsigned: a negative one is out of any range.
fn index(v: i32) -> usize {
    usize::try_from(v).unwrap_or(usize::MAX)
}
