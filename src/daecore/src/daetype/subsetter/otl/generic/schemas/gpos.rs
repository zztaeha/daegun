use super::super::schema::{
    Schema, StructField, CountSource, DropPolicy, OffsetWidth, PayloadShape, EmptyPolicy, EnvRef, Name, Width,
};

// The 24-bit formats, 3 and 4 of SinglePos and PairPos and 2 of the rest, are the beyond-64K twins: 24-bit glyphs
// and counts, 24-bit offsets to sets and anchors, 32-bit ones to coverages, classes and arrays.
const COVERAGE: Schema = Schema::Offset(OffsetWidth::W16, &Schema::Coverage(EmptyPolicy::Fail, None));
const LONG_COVERAGE: Schema = Schema::Offset(OffsetWidth::W32, &Schema::Coverage(EmptyPolicy::Fail, None));
const ANCHOR: Schema = Schema::Offset(OffsetWidth::W16, &Schema::Anchor);
const ANCHOR_24: Schema = Schema::Offset(OffsetWidth::W24, &Schema::Anchor);

const FORMAT: StructField = StructField { name: "format", schema: Schema::U16, bind: None };
const VALUE_FORMAT: StructField =
    StructField { name: "value_format", schema: Schema::ValueFormatField(EnvRef(Name::Vf)), bind: None };
const VALUE_FORMAT_1: StructField =
    StructField { name: "value_format1", schema: Schema::ValueFormatField(EnvRef(Name::Vf1)), bind: None };
const VALUE_FORMAT_2: StructField =
    StructField { name: "value_format2", schema: Schema::ValueFormatField(EnvRef(Name::Vf2)), bind: None };
const VALUE: StructField =
    StructField { name: "value", schema: Schema::ValueRecordField(EnvRef(Name::Vf)), bind: None };

macro_rules! single_pos_list {
    ($w:expr) => {
        Schema::Struct(&[
            FORMAT,
            StructField {
                name: "entries",
                schema: Schema::CoveredArray(
                    &[VALUE_FORMAT],
                    &Schema::ValueRecordField(EnvRef(Name::Vf)),
                    PayloadShape::Inline,
                    $w,
                ),
                bind: None,
            },
        ])
    };
}

const SINGLE_POS_3: Schema =
    Schema::Struct(&[FORMAT, StructField { name: "coverage", schema: LONG_COVERAGE, bind: None }, VALUE_FORMAT, VALUE]);
const SINGLE_POS_4: Schema = single_pos_list!(Width::W24);

// Each format is written as it's read, as every subtable but SingleSubst's is, whose rebuild picks its own.
pub(crate) static SINGLE_POS: Schema = Schema::FormatSwitch(0, &[
    (1, Schema::Struct(&[FORMAT, StructField { name: "coverage", schema: COVERAGE, bind: None }, VALUE_FORMAT, VALUE])),
    (2, single_pos_list!(Width::W16)),
    (3, SINGLE_POS_3),
    (4, SINGLE_POS_4),
]);

static PAIR_VALUE_RECORD: Schema = Schema::Struct(&[
    StructField { name: "second_glyph", schema: Schema::GlyphId, bind: None },
    StructField { name: "value1", schema: Schema::ValueRecordField(EnvRef(Name::Vf1)), bind: None },
    StructField { name: "value2", schema: Schema::ValueRecordField(EnvRef(Name::Vf2)), bind: None },
]);
static PAIR_VALUE_RECORD_24: Schema = Schema::Struct(&[
    StructField { name: "second_glyph", schema: Schema::GlyphId24, bind: None },
    StructField { name: "value1", schema: Schema::ValueRecordField(EnvRef(Name::Vf1)), bind: None },
    StructField { name: "value2", schema: Schema::ValueRecordField(EnvRef(Name::Vf2)), bind: None },
]);
static PAIR_SET: Schema = Schema::Struct(&[
    StructField { name: "pair_value_count", schema: Schema::U16, bind: Some(Name::PairValueCount) },
    StructField {
        name: "pairs",
        schema: Schema::Array(
            &PAIR_VALUE_RECORD,
            CountSource::Field(EnvRef(Name::PairValueCount)),
            DropPolicy::FilterSurvivorsOrFail,
        ),
        bind: None,
    },
]);
static PAIR_SET_24: Schema = Schema::Struct(&[
    StructField { name: "pair_value_count", schema: Schema::U24, bind: Some(Name::PairValueCount) },
    StructField {
        name: "pairs",
        schema: Schema::Array(
            &PAIR_VALUE_RECORD_24,
            CountSource::Field(EnvRef(Name::PairValueCount)),
            DropPolicy::FilterSurvivorsOrFail,
        ),
        bind: None,
    },
]);

macro_rules! pair_sets {
    ($set:expr, $offset:expr, $w:expr) => {
        Schema::Struct(&[
            FORMAT,
            StructField {
                name: "entries",
                schema: Schema::CoveredArray(
                    &[VALUE_FORMAT_1, VALUE_FORMAT_2],
                    &$set,
                    PayloadShape::Offsets($offset),
                    $w,
                ),
                bind: None,
            },
        ])
    };
}

macro_rules! pair_classes {
    ($coverage:expr, $w:expr) => {
        Schema::Struct(&[
            FORMAT,
            StructField { name: "coverage", schema: $coverage, bind: None },
            VALUE_FORMAT_1,
            VALUE_FORMAT_2,
            StructField {
                name: "class_matrix",
                schema: Schema::ClassMatrix(EnvRef(Name::Vf1), EnvRef(Name::Vf2), $w),
                bind: None,
            },
        ])
    };
}

pub(crate) static PAIR_POS: Schema = Schema::FormatSwitch(0, &[
    (1, pair_sets!(PAIR_SET, OffsetWidth::W16, Width::W16)),
    (2, pair_classes!(COVERAGE, Width::W16)),
    (3, pair_sets!(PAIR_SET_24, OffsetWidth::W24, Width::W24)),
    (4, pair_classes!(LONG_COVERAGE, Width::W24)),
]);

static ENTRY_EXIT: Schema = Schema::Struct(&[
    StructField { name: "entry_anchor", schema: ANCHOR, bind: None },
    StructField { name: "exit_anchor", schema: ANCHOR, bind: None },
]);
static ENTRY_EXIT_24: Schema = Schema::Struct(&[
    StructField { name: "entry_anchor", schema: ANCHOR_24, bind: None },
    StructField { name: "exit_anchor", schema: ANCHOR_24, bind: None },
]);

macro_rules! cursive_pos {
    ($entry_exit:expr, $w:expr) => {
        Schema::Struct(&[
            FORMAT,
            StructField {
                name: "entries",
                schema: Schema::CoveredArray(&[], &$entry_exit, PayloadShape::Inline, $w),
                bind: None,
            },
        ])
    };
}

pub(crate) static CURSIVE_POS: Schema = Schema::FormatSwitch(0, &[
    (1, cursive_pos!(ENTRY_EXIT, Width::W16)),
    (2, cursive_pos!(ENTRY_EXIT_24, Width::W24)),
]);

static MARK_RECORD: Schema = Schema::Struct(&[
    StructField { name: "mark_class", schema: Schema::U16, bind: None },
    StructField { name: "mark_anchor", schema: ANCHOR, bind: None },
]);
static MARK_RECORD_24: Schema = Schema::Struct(&[
    StructField { name: "mark_class", schema: Schema::U16, bind: None },
    StructField { name: "mark_anchor", schema: ANCHOR_24, bind: None },
]);

// A base's anchors, or a ligature component's: one per mark class.
static CLASS_ANCHORS: Schema = Schema::Array(
    &ANCHOR,
    CountSource::Field(EnvRef(Name::ClassCount)),
    DropPolicy::FilterSurvivors,
);
static CLASS_ANCHORS_24: Schema = Schema::Array(
    &ANCHOR_24,
    CountSource::Field(EnvRef(Name::ClassCount)),
    DropPolicy::FilterSurvivors,
);

const MARKS: Schema = Schema::Offset(
    OffsetWidth::W16,
    &Schema::ZippedWithBoundCoverage(
        EnvRef(Name::MarkCoverage),
        &MARK_RECORD,
        PayloadShape::Inline,
        DropPolicy::FilterSurvivorsOrFail,
        Width::W16,
    ),
);
const MARKS_24: Schema = Schema::Offset(
    OffsetWidth::W32,
    &Schema::ZippedWithBoundCoverage(
        EnvRef(Name::MarkCoverage),
        &MARK_RECORD_24,
        PayloadShape::Inline,
        DropPolicy::FilterSurvivorsOrFail,
        Width::W24,
    ),
);

// MarkBase or MarkMark: the marks' coverage and the bases' (or other marks'), then their arrays.
macro_rules! mark_attach {
    ($offset:expr, $marks:expr, $rows:expr, $anchors:expr) => {
        Schema::Struct(&[
            FORMAT,
            StructField {
                name: "mark_coverage",
                schema: Schema::Offset($offset, &Schema::Coverage(EmptyPolicy::Fail, Some(EnvRef(Name::MarkCoverage)))),
                bind: None,
            },
            StructField {
                name: "base_coverage",
                schema: Schema::Offset($offset, &Schema::Coverage(EmptyPolicy::Fail, Some(EnvRef(Name::BaseCoverage)))),
                bind: None,
            },
            StructField { name: "class_count", schema: Schema::U16, bind: Some(Name::ClassCount) },
            StructField { name: "marks", schema: $marks, bind: None },
            StructField {
                name: "bases",
                schema: Schema::Offset(
                    $offset,
                    &Schema::ZippedWithBoundCoverage(
                        EnvRef(Name::BaseCoverage),
                        &$anchors,
                        PayloadShape::Inline,
                        DropPolicy::FilterSurvivorsOrFail,
                        $rows,
                    ),
                ),
                bind: None,
            },
        ])
    };
}

// The 24-bit MarkBase counts its bases in 24 bits, the 24-bit MarkMark its marks in 16.
pub(crate) static MARK_BASE_POS: Schema = Schema::FormatSwitch(0, &[
    (1, mark_attach!(OffsetWidth::W16, MARKS, Width::W16, CLASS_ANCHORS)),
    (2, mark_attach!(OffsetWidth::W32, MARKS_24, Width::W24, CLASS_ANCHORS_24)),
]);
pub(crate) static MARK_MARK_POS: Schema = Schema::FormatSwitch(0, &[
    (1, mark_attach!(OffsetWidth::W16, MARKS, Width::W16, CLASS_ANCHORS)),
    (2, mark_attach!(OffsetWidth::W32, MARKS_24, Width::W16, CLASS_ANCHORS_24)),
]);

static LIGATURE_ATTACH: Schema = Schema::Struct(&[
    StructField { name: "component_count", schema: Schema::U16, bind: Some(Name::ComponentCount) },
    StructField {
        name: "components",
        schema: Schema::Array(
            &CLASS_ANCHORS,
            CountSource::Field(EnvRef(Name::ComponentCount)),
            DropPolicy::FilterSurvivors,
        ),
        bind: None,
    },
]);
static LIGATURE_ATTACH_24: Schema = Schema::Struct(&[
    StructField { name: "component_count", schema: Schema::U16, bind: Some(Name::ComponentCount) },
    StructField {
        name: "components",
        schema: Schema::Array(
            &CLASS_ANCHORS_24,
            CountSource::Field(EnvRef(Name::ComponentCount)),
            DropPolicy::FilterSurvivors,
        ),
        bind: None,
    },
]);

macro_rules! mark_lig {
    ($offset:expr, $marks:expr, $attach:expr, $attach_offset:expr, $w:expr) => {
        Schema::Struct(&[
            FORMAT,
            StructField {
                name: "mark_coverage",
                schema: Schema::Offset($offset, &Schema::Coverage(EmptyPolicy::Fail, Some(EnvRef(Name::MarkCoverage)))),
                bind: None,
            },
            StructField {
                name: "ligature_coverage",
                schema: Schema::Offset($offset, &Schema::Coverage(EmptyPolicy::Fail, Some(EnvRef(Name::LigCoverage)))),
                bind: None,
            },
            StructField { name: "class_count", schema: Schema::U16, bind: Some(Name::ClassCount) },
            StructField { name: "marks", schema: $marks, bind: None },
            StructField {
                name: "ligatures",
                schema: Schema::Offset(
                    $offset,
                    &Schema::ZippedWithBoundCoverage(
                        EnvRef(Name::LigCoverage),
                        &$attach,
                        PayloadShape::Offsets($attach_offset),
                        DropPolicy::FilterSurvivorsOrFail,
                        $w,
                    ),
                ),
                bind: None,
            },
        ])
    };
}

pub(crate) static MARK_LIG_POS: Schema = Schema::FormatSwitch(0, &[
    (1, mark_lig!(OffsetWidth::W16, MARKS, LIGATURE_ATTACH, OffsetWidth::W16, Width::W16)),
    (2, mark_lig!(OffsetWidth::W32, MARKS_24, LIGATURE_ATTACH_24, OffsetWidth::W24, Width::W24)),
]);
