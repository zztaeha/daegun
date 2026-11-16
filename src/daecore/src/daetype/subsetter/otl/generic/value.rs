use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::rc::Rc;
use alloc::vec::Vec;
use super::super::gdef;
use super::schema::{Name, OffsetWidth, PayloadShape, Width};

// What a subtable parses into, 32 bytes a node: the larger shapes are boxed, being few.
#[allow(clippy::enum_variant_names)]
#[derive(Clone)]
pub(crate) enum Value<'b> {
    U16(u16),
    // A ValueFormat, by the name its records read it as.
    ValueFormat(Name, u16),
    LookupIndex(u16),
    U24(u32),
    Glyph(Width, u32),
    Offset(OffsetWidth, Option<Rc<Value<'b>>>),
    Array(Vec<Value<'b>>),
    Glyphs(Width, Vec<u32>),
    // A covered glyph and the one glyph it maps to, by covered glyph: all a SingleSubst substitutes.
    GlyphMap(Vec<(u32, u32)>),
    OffsetArray(OffsetWidth, Vec<Option<Rc<Value<'b>>>>),
    Struct(Vec<Value<'b>>),
    ValueRecord(Record<'b>),
    Coverage(Rc<[u32]>),
    ClassDef(Vec<(u32, u32)>),
    CoveredArray(Box<Covered<'b>>),
    // An offset to an anchor table, held in place as each offset parses its own: its x, y, contour point and x and y
    // Device tables.
    Anchor(OffsetWidth, i16, i16, Option<u16>, Option<Box<[Option<&'b [u8]>; 2]>>),
    // One entry per glyph of the bound coverage, as `Covered`'s; None is a NULL offset.
    ZippedWithBoundCoverage(PayloadShape, Width, Vec<Option<Value<'b>>>),
    // A format 3 caret's Device or VariationIndex table, kept with it.
    CaretValue(gdef::CaretValue, Option<&'b [u8]>),
    ClassMatrix(Box<Matrix<'b>>),
}

// A ValueRecord as written: its format, the name of the ValueFormat it's read by, its values in field order, and a
// Device table for each device field the format holds.
#[derive(Clone)]
pub(crate) struct Record<'b> {
    pub(crate) format: u16,
    pub(crate) values: [i16; 4],
    pub(crate) name: Name,
    pub(crate) devices: Option<Box<[Option<&'b [u8]>; 4]>>,
}

impl Record<'_> {
    pub(crate) fn width(&self) -> usize {
        (self.format & 0x00FF).count_ones() as usize * 2
    }

    // The record in `format & written`, its Device tables moved down to the device fields it keeps.
    fn narrow(&mut self, written: u16) {
        let (held, format) = (self.format, self.format & written);
        if let Some(devices) = &mut self.devices {
            let mut n = 0;
            for (i, bit) in (0..4).filter(|bit| held & (0x10 << bit) != 0).enumerate() {
                if format & (0x10 << bit) != 0 {
                    devices[n] = devices[i];
                    n += 1;
                }
            }
            devices[n..].fill(None);
        }
        self.format = format;
    }
}

// Each ValueFormat and record as `written` keeps its name's fields.
pub(super) fn narrow(value: &mut Value<'_>, written: &[u16; Name::COUNT]) {
    narrow_sharing(value, written, &mut BTreeMap::new());
}

// A child more than one table names is narrowed once, as a copy every naming then takes, rather than copied for each:
// `copies` holds each shared child narrowed by its address, the child kept beside its copy so no other takes it.
type Copies<'b> = BTreeMap<*const Value<'b>, (Rc<Value<'b>>, Rc<Value<'b>>)>;

fn narrow_sharing<'b>(value: &mut Value<'b>, written: &[u16; Name::COUNT], copies: &mut Copies<'b>) {
    let mut child = |child: &mut Rc<Value<'b>>| {
        if Rc::strong_count(child) == 1 {
            return narrow_sharing(Rc::make_mut(child), written, copies);
        }
        let shared = Rc::as_ptr(child);
        if let Some((_, copy)) = copies.get(&shared) {
            *child = Rc::clone(copy);
            return;
        }
        let original = Rc::clone(child);
        narrow_sharing(Rc::make_mut(child), written, copies);
        copies.insert(shared, (original, Rc::clone(child)));
    };
    match value {
        Value::ValueFormat(name, format) => *format &= written[*name as usize],
        Value::ValueRecord(r) => r.narrow(written[r.name as usize]),
        Value::ClassMatrix(m) => m.grid.iter_mut().for_each(|r| r.narrow(written[r.name as usize])),
        Value::Array(values) | Value::Struct(values) => {
            values.iter_mut().for_each(|v| narrow_sharing(v, written, copies));
        }
        Value::Offset(_, Some(c)) => child(c),
        Value::OffsetArray(_, slots) => slots.iter_mut().flatten().for_each(child),
        Value::ZippedWithBoundCoverage(_, _, slots) => {
            slots.iter_mut().flatten().for_each(|v| narrow_sharing(v, written, copies));
        }
        Value::CoveredArray(c) => {
            c.fields.iter_mut().for_each(|v| narrow_sharing(v, written, copies));
            c.entries.iter_mut().for_each(|(_, v)| narrow_sharing(v, written, copies));
        }
        _ => {}
    }
}

// Each covered glyph's payload in place, one at an offset as a `Value::Offset`: child tables are shared by every
// naming of them in a subtable, parsed once.
#[derive(Clone)]
pub(crate) struct Covered<'b> {
    pub(crate) fields: Vec<Value<'b>>,
    pub(crate) shape: PayloadShape,
    pub(crate) entries: Vec<(u32, Value<'b>)>,
    pub(crate) width: Width,
}

// A PairPos 2 or 4 class matrix, its grid two records a cell.
#[derive(Clone)]
pub(crate) struct Matrix<'b> {
    pub(crate) width: Width,
    pub(crate) class_def1: Vec<(u32, u16)>,
    pub(crate) class_def2: Vec<(u32, u16)>,
    pub(crate) class1_count: u16,
    pub(crate) class2_count: u16,
    pub(crate) grid: Vec<Record<'b>>,
}
