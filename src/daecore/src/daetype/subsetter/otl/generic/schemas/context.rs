use super::super::schema::{
    Schema, StructField, CountSource, DropPolicy, RebuildPolicy, OffsetWidth, PayloadShape, EmptyPolicy, EnvRef,
    Name, Width,
};

// Context 4 to 6 and ChainContext 4 and 5, the 24-bit twins: 24-bit glyphs, counts and set offsets, 32-bit
// coverage and class offsets. Glyph rules lie at 16-bit offsets in a Context, 24-bit in a ChainContext.
const COVERAGE: Schema = Schema::Offset(OffsetWidth::W16, &Schema::Coverage(EmptyPolicy::Fail, None));
const COVERAGE_24: Schema = Schema::Offset(OffsetWidth::W24, &Schema::Coverage(EmptyPolicy::Fail, None));
const LONG_COVERAGE: Schema = Schema::Offset(OffsetWidth::W32, &Schema::Coverage(EmptyPolicy::Fail, None));
const CLASS_DEF: Schema = Schema::Offset(OffsetWidth::W16, &Schema::ClassDef(EmptyPolicy::Keep));
const LONG_CLASS_DEF: Schema = Schema::Offset(OffsetWidth::W32, &Schema::ClassDef(EmptyPolicy::Keep));
const FORMAT: StructField = StructField { name: "format", schema: Schema::U16, bind: None };

static SEQ_LOOKUP_RECORD: Schema = Schema::Struct(&[
    StructField { name: "sequence_index", schema: Schema::U16, bind: None },
    StructField { name: "lookup_list_index", schema: Schema::LookupIndex, bind: None },
]);

const LOOKUP_RECORDS: Schema = Schema::Array(
    &SEQ_LOOKUP_RECORD,
    CountSource::Field(EnvRef(Name::LrCount)),
    DropPolicy::FilterSurvivors,
);

// A RuleSet or ClassSet: offsets of a width to its rules.
macro_rules! rule_set {
    ($rule:expr, $offset:expr) => {
        Schema::Struct(&[
            StructField { name: "rule_count", schema: Schema::U16, bind: Some(Name::RuleCount) },
            StructField {
                name: "rules",
                schema: Schema::OffsetArray(
                    &$rule,
                    CountSource::Field(EnvRef(Name::RuleCount)),
                    RebuildPolicy::CompactSurvivors,
                    $offset,
                ),
                bind: None,
            },
        ])
    };
}

// A rule's glyphs of a width, or its classes.
macro_rules! values {
    (glyphs $w:expr, $count:expr) => {
        Schema::Glyphs($w, $count, DropPolicy::AllOrNothing)
    };
    (classes, $count:expr) => {
        Schema::Array(&Schema::U16, $count, DropPolicy::FilterSurvivors)
    };
}

macro_rules! rule {
    ($($values:tt)*) => {
        Schema::Struct(&[
            StructField { name: "glyph_count", schema: Schema::U16, bind: Some(Name::GlyphCount) },
            StructField { name: "lr_count", schema: Schema::U16, bind: Some(Name::LrCount) },
            StructField {
                name: "input",
                schema: values!($($values)*, CountSource::FieldMinusOne(EnvRef(Name::GlyphCount))),
                bind: None,
            },
            StructField { name: "lookup_records", schema: LOOKUP_RECORDS, bind: None },
        ])
    };
}

macro_rules! chain_rule {
    ($($values:tt)*) => {
        Schema::Struct(&[
            StructField { name: "bt_count", schema: Schema::U16, bind: Some(Name::BtCount) },
            StructField {
                name: "backtrack",
                schema: values!($($values)*, CountSource::Field(EnvRef(Name::BtCount))),
                bind: None,
            },
            StructField { name: "in_count", schema: Schema::U16, bind: Some(Name::InCount) },
            StructField {
                name: "input",
                schema: values!($($values)*, CountSource::FieldMinusOne(EnvRef(Name::InCount))),
                bind: None,
            },
            StructField { name: "la_count", schema: Schema::U16, bind: Some(Name::LaCount) },
            StructField {
                name: "lookahead",
                schema: values!($($values)*, CountSource::Field(EnvRef(Name::LaCount))),
                bind: None,
            },
            StructField { name: "lr_count", schema: Schema::U16, bind: Some(Name::LrCount) },
            StructField { name: "lookup_records", schema: LOOKUP_RECORDS, bind: None },
        ])
    };
}

static RULE: Schema = rule!(glyphs Width::W16);
static RULE_24: Schema = rule!(glyphs Width::W24);
static CLASS_RULE: Schema = rule!(classes);
static CHAIN_RULE: Schema = chain_rule!(glyphs Width::W16);
static CHAIN_RULE_24: Schema = chain_rule!(glyphs Width::W24);
static CHAIN_CLASS_RULE: Schema = chain_rule!(classes);

static RULE_SET: Schema = rule_set!(RULE, OffsetWidth::W16);
static RULE_SET_24: Schema = rule_set!(RULE_24, OffsetWidth::W16);
static CLASS_SET: Schema = rule_set!(CLASS_RULE, OffsetWidth::W16);
static CLASS_SET_24: Schema = rule_set!(CLASS_RULE, OffsetWidth::W24);
static CHAIN_RULE_SET: Schema = rule_set!(CHAIN_RULE, OffsetWidth::W16);
static CHAIN_RULE_SET_24: Schema = rule_set!(CHAIN_RULE_24, OffsetWidth::W24);
static CHAIN_CLASS_SET: Schema = rule_set!(CHAIN_CLASS_RULE, OffsetWidth::W16);
static CHAIN_CLASS_SET_24: Schema = rule_set!(CHAIN_CLASS_RULE, OffsetWidth::W24);

// A rule set for each covered glyph.
macro_rules! glyph_sets {
    ($set:expr, $offset:expr, $w:expr) => {
        Schema::Struct(&[
            FORMAT,
            StructField {
                name: "entries",
                schema: Schema::CoveredArray(&[], &$set, PayloadShape::Offsets($offset), $w),
                bind: None,
            },
        ])
    };
}

// One rule of a coverage for each place, at offsets of a width.
macro_rules! coverages {
    ($coverage:expr) => {
        Schema::Struct(&[
            FORMAT,
            StructField { name: "glyph_count", schema: Schema::U16, bind: Some(Name::GlyphCount) },
            StructField { name: "lr_count", schema: Schema::U16, bind: Some(Name::LrCount) },
            StructField {
                name: "positions",
                schema: Schema::Array(
                    &$coverage,
                    CountSource::Field(EnvRef(Name::GlyphCount)),
                    DropPolicy::AllOrNothing,
                ),
                bind: None,
            },
            StructField { name: "lookup_records", schema: LOOKUP_RECORDS, bind: None },
        ])
    };
}

pub(crate) static CONTEXT: Schema = Schema::FormatSwitch(0, &[
    (1, glyph_sets!(RULE_SET, OffsetWidth::W16, Width::W16)),
    (2, Schema::Struct(&[
        FORMAT,
        StructField { name: "coverage", schema: COVERAGE, bind: None },
        StructField { name: "class_def", schema: CLASS_DEF, bind: None },
        StructField { name: "class_set_count", schema: Schema::U16, bind: Some(Name::ClassSetCount) },
        StructField {
            name: "class_sets",
            schema: Schema::OffsetArray(
                &CLASS_SET,
                CountSource::Field(EnvRef(Name::ClassSetCount)),
                RebuildPolicy::PreserveSlotPositions,
                OffsetWidth::W16,
            ),
            bind: None,
        },
    ])),
    (3, coverages!(COVERAGE)),
    (4, glyph_sets!(RULE_SET_24, OffsetWidth::W24, Width::W24)),
    (5, Schema::Struct(&[
        FORMAT,
        StructField { name: "coverage", schema: LONG_COVERAGE, bind: None },
        StructField { name: "class_def", schema: LONG_CLASS_DEF, bind: None },
        StructField { name: "class_set_count", schema: Schema::U24, bind: Some(Name::ClassSetCount) },
        StructField {
            name: "class_sets",
            schema: Schema::OffsetArray(
                &CLASS_SET_24,
                CountSource::Field(EnvRef(Name::ClassSetCount)),
                RebuildPolicy::PreserveSlotPositions,
                OffsetWidth::W24,
            ),
            bind: None,
        },
    ])),
    (6, coverages!(COVERAGE_24)),
]);

macro_rules! chain_classes {
    ($coverage:expr, $def:expr, $set:expr, $offset:expr) => {
        Schema::Struct(&[
            FORMAT,
            StructField { name: "coverage", schema: $coverage, bind: None },
            StructField { name: "backtrack_class_def", schema: $def, bind: None },
            StructField { name: "input_class_def", schema: $def, bind: None },
            StructField { name: "lookahead_class_def", schema: $def, bind: None },
            StructField { name: "class_set_count", schema: Schema::U16, bind: Some(Name::ClassSetCount) },
            StructField {
                name: "class_sets",
                schema: Schema::OffsetArray(
                    &$set,
                    CountSource::Field(EnvRef(Name::ClassSetCount)),
                    RebuildPolicy::PreserveSlotPositions,
                    $offset,
                ),
                bind: None,
            },
        ])
    };
}

pub(crate) static CHAIN_CONTEXT: Schema = Schema::FormatSwitch(0, &[
    (1, glyph_sets!(CHAIN_RULE_SET, OffsetWidth::W16, Width::W16)),
    (2, chain_classes!(COVERAGE, CLASS_DEF, CHAIN_CLASS_SET, OffsetWidth::W16)),
    (3, Schema::Struct(&[
        FORMAT,
        StructField { name: "bt_count", schema: Schema::U16, bind: Some(Name::BtCount) },
        StructField {
            name: "backtrack",
            schema: Schema::Array(&COVERAGE, CountSource::Field(EnvRef(Name::BtCount)), DropPolicy::AllOrNothing),
            bind: None,
        },
        StructField { name: "in_count", schema: Schema::U16, bind: Some(Name::InCount) },
        StructField {
            name: "input",
            schema: Schema::Array(&COVERAGE, CountSource::Field(EnvRef(Name::InCount)), DropPolicy::AllOrNothing),
            bind: None,
        },
        StructField { name: "la_count", schema: Schema::U16, bind: Some(Name::LaCount) },
        StructField {
            name: "lookahead",
            schema: Schema::Array(&COVERAGE, CountSource::Field(EnvRef(Name::LaCount)), DropPolicy::AllOrNothing),
            bind: None,
        },
        StructField { name: "lr_count", schema: Schema::U16, bind: Some(Name::LrCount) },
        StructField { name: "lookup_records", schema: LOOKUP_RECORDS, bind: None },
    ])),
    (4, glyph_sets!(CHAIN_RULE_SET_24, OffsetWidth::W24, Width::W24)),
    (5, chain_classes!(LONG_COVERAGE, LONG_CLASS_DEF, CHAIN_CLASS_SET_24, OffsetWidth::W24)),
]);
