#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OffsetWidth {
    W16,
    W24,
    W32,
}

impl OffsetWidth {
    pub(crate) fn bytes(self) -> usize {
        match self {
            OffsetWidth::W16 => 2,
            OffsetWidth::W24 => 3,
            OffsetWidth::W32 => 4,
        }
    }
}

// The width of a subtable's glyphs and own counts: 16 bits, or 24 in the beyond-64K formats, which reach their
// first tables at 32-bit offsets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Width {
    W16,
    W24,
}

impl Width {
    pub(crate) fn bytes(self) -> usize {
        match self {
            Width::W16 => 2,
            Width::W24 => 3,
        }
    }

    pub(crate) fn long(self) -> OffsetWidth {
        match self {
            Width::W16 => OffsetWidth::W16,
            Width::W24 => OffsetWidth::W32,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum CountSource {
    Field(EnvRef),
    FieldMinusOne(EnvRef),
}

#[derive(Clone, Copy, Debug)]
pub enum DropPolicy {
    FilterSurvivors,
    AllOrNothing,
    // Like FilterSurvivors, but an empty result is None rather than a valid empty array – a choice
    // menu with no surviving choices is meaningless, not merely smaller.
    FilterSurvivorsOrFail,
}

#[derive(Clone, Copy, Debug)]
pub enum PayloadShape {
    Inline,
    Offsets(OffsetWidth),
}

#[derive(Clone, Copy, Debug)]
pub enum RebuildPolicy {
    CompactSurvivors,
    PreserveSlotPositions,
}

// A field a schema binds and a later one reads back: a count, a value format or a coverage.
#[derive(Clone, Copy, Debug)]
pub enum Name {
    BaseCoverage, BtCount, CaretCount, ClassCount, ClassSetCount, CompCount, ComponentCount,
    GlyphCount, InCount, LaCount, LigCount, LigCoverage, LrCount, MarkCoverage, PairValueCount,
    PointCount, RuleCount, Vf, Vf1, Vf2,
}

impl Name {
    pub(crate) const COUNT: usize = Name::Vf2 as usize + 1;
}

#[derive(Clone, Copy, Debug)]
pub struct EnvRef(pub(crate) Name);

pub struct StructField {
    pub(crate) name: &'static str,
    pub(crate) schema: Schema,
    pub(crate) bind: Option<Name>,
}

// A table's layout. Schemas are statics, built by the compiler rather than per call.
pub enum Schema {
    U16,
    // A nested lookup record's lookup, written as the subset numbers its lookups.
    LookupIndex,
    // A 24-bit count.
    U24,
    GlyphId,
    GlyphId24,
    Offset(OffsetWidth, &'static Schema),
    Array(&'static Schema, CountSource, DropPolicy),
    // An array of glyph IDs, held four bytes a glyph rather than a node each.
    Glyphs(Width, CountSource, DropPolicy),
    OffsetArray(&'static Schema, CountSource, RebuildPolicy, OffsetWidth),
    Struct(&'static [StructField]),
    FormatSwitch(usize, &'static [(u16, Schema)]),
    ValueRecordField(EnvRef),
    Coverage(EmptyPolicy, Option<EnvRef>),
    ClassDef(EmptyPolicy),
    // Its coverage's offset, the fields, then a count and a payload for each covered glyph; 32-bit and 24-bit for
    // a 24-bit format.
    CoveredArray(&'static [StructField], &'static Schema, PayloadShape, Width),
    ValueFormatField(EnvRef),
    Anchor,
    ZippedWithBoundCoverage(EnvRef, &'static Schema, PayloadShape, DropPolicy, Width),
    CaretValue,
    // A PairPos 2 or 4 class matrix, each cell a record in either value format, its class definitions at 16-bit or
    // 32-bit offsets.
    ClassMatrix(EnvRef, EnvRef, Width),
    // SingleSubst 1 or 3, and 2 or 4: all of the subtable, written 2 or 4 by its widest substitute.
    DeltaCoverageSubst(Width),
    CoveredGlyphs(Width),
}

#[derive(Clone, Copy, Debug)]
pub enum EmptyPolicy {
    Fail,
    Keep,
}
