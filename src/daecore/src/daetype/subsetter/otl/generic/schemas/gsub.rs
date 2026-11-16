use super::super::schema::{
    Schema, StructField, CountSource, DropPolicy, RebuildPolicy, OffsetWidth, PayloadShape, EnvRef, Name, Width,
};

// Each format is written back as it came but SingleSubst's: 1 where its pairs share a delta, else 2, and 3 or 4 where
// a substitute needs 24 bits.
pub(crate) static SINGLE_SUBST: Schema = Schema::FormatSwitch(0, &[
    (1, Schema::DeltaCoverageSubst(Width::W16)),
    (2, Schema::CoveredGlyphs(Width::W16)),
    (3, Schema::DeltaCoverageSubst(Width::W24)),
    (4, Schema::CoveredGlyphs(Width::W24)),
]);

// A MultipleSubst sequence or an AlternateSubst set: glyphs of a width, counted in 16 bits.
macro_rules! glyph_list {
    ($w:expr, $policy:expr) => {
        Schema::Struct(&[
            StructField { name: "glyph_count", schema: Schema::U16, bind: Some(Name::GlyphCount) },
            StructField {
                name: "glyphs",
                schema: Schema::Glyphs($w, CountSource::Field(EnvRef(Name::GlyphCount)), $policy),
                bind: None,
            },
        ])
    };
}

// Format 1 of 16-bit glyphs, or 2 of 24-bit ones: a set for each covered glyph.
macro_rules! covered_sets {
    ($narrow:expr, $wide:expr) => {
        Schema::FormatSwitch(0, &[
            (1, Schema::Struct(&[
                StructField { name: "format", schema: Schema::U16, bind: None },
                StructField {
                    name: "entries",
                    schema: Schema::CoveredArray(&[], &$narrow, PayloadShape::Offsets(OffsetWidth::W16), Width::W16),
                    bind: None,
                },
            ])),
            (2, Schema::Struct(&[
                StructField { name: "format", schema: Schema::U16, bind: None },
                StructField {
                    name: "entries",
                    schema: Schema::CoveredArray(&[], &$wide, PayloadShape::Offsets(OffsetWidth::W24), Width::W24),
                    bind: None,
                },
            ])),
        ])
    };
}

static SEQUENCE: Schema = glyph_list!(Width::W16, DropPolicy::AllOrNothing);
static SEQUENCE_24: Schema = glyph_list!(Width::W24, DropPolicy::AllOrNothing);
pub(crate) static MULTIPLE_SUBST: Schema = covered_sets!(SEQUENCE, SEQUENCE_24);

static ALTERNATE_SET: Schema = glyph_list!(Width::W16, DropPolicy::FilterSurvivorsOrFail);
static ALTERNATE_SET_24: Schema = glyph_list!(Width::W24, DropPolicy::FilterSurvivorsOrFail);
pub(crate) static ALTERNATE_SUBST: Schema = covered_sets!(ALTERNATE_SET, ALTERNATE_SET_24);

// A ligature's glyph and its components but the first, each of a width.
macro_rules! ligature {
    ($glyph:expr, $w:expr) => {
        Schema::Struct(&[
            StructField { name: "lig_glyph", schema: $glyph, bind: None },
            StructField { name: "comp_count", schema: Schema::U16, bind: Some(Name::CompCount) },
            StructField {
                name: "components",
                schema: Schema::Glyphs(
                    $w,
                    CountSource::FieldMinusOne(EnvRef(Name::CompCount)),
                    DropPolicy::AllOrNothing,
                ),
                bind: None,
            },
        ])
    };
}

macro_rules! ligature_set {
    ($ligature:expr, $offset:expr) => {
        Schema::Struct(&[
            StructField { name: "lig_count", schema: Schema::U16, bind: Some(Name::LigCount) },
            StructField {
                name: "ligatures",
                schema: Schema::OffsetArray(
                    &$ligature,
                    CountSource::Field(EnvRef(Name::LigCount)),
                    RebuildPolicy::CompactSurvivors,
                    $offset,
                ),
                bind: None,
            },
        ])
    };
}

static LIGATURE: Schema = ligature!(Schema::GlyphId, Width::W16);
static LIGATURE_24: Schema = ligature!(Schema::GlyphId24, Width::W24);
static LIGATURE_SET: Schema = ligature_set!(LIGATURE, OffsetWidth::W16);
static LIGATURE_SET_24: Schema = ligature_set!(LIGATURE_24, OffsetWidth::W24);
pub(crate) static LIGATURE_SUBST: Schema = covered_sets!(LIGATURE_SET, LIGATURE_SET_24);
