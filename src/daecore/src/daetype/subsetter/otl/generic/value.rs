use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use super::super::gdef;
use super::schema::{OffsetWidth, PayloadShape};

// What a subtable parses into, 32 bytes a node: the larger shapes are boxed, being few.
#[allow(clippy::enum_variant_names)]
pub(crate) enum Value<'b> {
    U16(u16),
    Glyph(u16),
    Offset(OffsetWidth, Option<Rc<Value<'b>>>),
    Array(Vec<Value<'b>>),
    OffsetArray(OffsetWidth, Vec<Option<Rc<Value<'b>>>>),
    Struct(Vec<Value<'b>>),
    ValueRecord(Record<'b>),
    Coverage(Rc<[u16]>),
    ClassDef(Vec<(u16, u16)>),
    CoveredArray(Box<Covered<'b>>),
    // An anchor's x, y, contour point and x and y Device tables.
    Anchor(i16, i16, Option<u16>, Option<Box<[Option<&'b [u8]>; 2]>>),
    // One entry per glyph of the bound coverage; None is a NULL offset. Child tables are shared by
    // every naming of them in a subtable, parsed once.
    ZippedWithBoundCoverage(PayloadShape, Vec<Option<Rc<Value<'b>>>>),
    // A format 3 caret's Device or VariationIndex table, kept with it.
    CaretValue(gdef::CaretValue, Option<&'b [u8]>),
    ClassMatrix(Box<Matrix<'b>>),
}

// A ValueRecord as written: its format, its values in field order, and a Device table for each
// device field the format holds.
pub(crate) struct Record<'b> {
    pub(crate) format: u16,
    pub(crate) values: [i16; 4],
    pub(crate) devices: Option<Box<[Option<&'b [u8]>; 4]>>,
}

impl Record<'_> {
    pub(crate) fn width(&self) -> usize {
        (self.format & 0x00FF).count_ones() as usize * 2
    }
}

pub(crate) struct Covered<'b> {
    pub(crate) fields: Vec<Value<'b>>,
    pub(crate) shape: PayloadShape,
    pub(crate) entries: Vec<(u16, Rc<Value<'b>>)>,
}

// A PairPos 2 class matrix, its grid two records a cell.
pub(crate) struct Matrix<'b> {
    pub(crate) class_def1: Vec<(u16, u16)>,
    pub(crate) class_def2: Vec<(u16, u16)>,
    pub(crate) class1_count: u16,
    pub(crate) class2_count: u16,
    pub(crate) grid: Vec<Record<'b>>,
}

#[cfg(test)]
const _: () = assert!(core::mem::size_of::<Value>() == 32 && core::mem::size_of::<Record>() == 24);
