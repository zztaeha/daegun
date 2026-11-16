use super::super::schema::{
    Schema, StructField, CountSource, DropPolicy, RebuildPolicy, OffsetWidth, PayloadShape, EmptyPolicy, EnvRef,
    Name, Width,
};

pub(crate) static CLASS_DEF: Schema = Schema::ClassDef(EmptyPolicy::Fail);

static ATTACH_POINT: Schema = Schema::Struct(&[
    StructField { name: "point_count", schema: Schema::U16, bind: Some(Name::PointCount) },
    StructField {
        name: "points",
        schema: Schema::Array(&Schema::U16, CountSource::Field(EnvRef(Name::PointCount)), DropPolicy::FilterSurvivors),
        bind: None,
    },
]);

pub(crate) static ATTACH_LIST: Schema = Schema::CoveredArray(
    &[],
    &ATTACH_POINT,
    PayloadShape::Offsets(OffsetWidth::W16),
    Width::W16,
);

static LIG_GLYPH: Schema = Schema::Struct(&[
    StructField { name: "caret_count", schema: Schema::U16, bind: Some(Name::CaretCount) },
    StructField {
        name: "carets",
        schema: Schema::OffsetArray(
            &Schema::CaretValue,
            CountSource::Field(EnvRef(Name::CaretCount)),
            RebuildPolicy::CompactSurvivors,
            OffsetWidth::W16,
        ),
        bind: None,
    },
]);

pub(crate) static LIG_CARET_LIST: Schema = Schema::CoveredArray(
    &[],
    &LIG_GLYPH,
    PayloadShape::Offsets(OffsetWidth::W16),
    Width::W16,
);

// GDEF 1.4's LigCaretList2: its coverage at a 32-bit offset, its ligatures counted in 24 bits at 24-bit offsets.
pub(crate) static LIG_CARET_LIST_24: Schema = Schema::CoveredArray(
    &[],
    &LIG_GLYPH,
    PayloadShape::Offsets(OffsetWidth::W24),
    Width::W24,
);
