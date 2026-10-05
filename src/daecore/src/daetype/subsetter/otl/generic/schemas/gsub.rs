use alloc::boxed::Box;
use super::format_1;
use super::super::schema::{Schema, StructField, CountSource, DropPolicy, RebuildPolicy, OffsetWidth, PayloadShape, EnvRef};
use super::context::{context_schema, chain_context_schema};

pub(crate) fn single_subst_schema() -> Schema {
    Schema::FormatSwitch(0, vec![
        (1, Schema::DeltaCoverageSubst),
        (2, Schema::Struct(vec![
            StructField { name: "format", schema: Schema::U16, bind: None },
            StructField { name: "entries", schema: Schema::CoveredArray(vec![], Box::new(Schema::GlyphId), PayloadShape::Inline), bind: None },
        ])),
    ])
}

fn coverage_indexed_glyph_array_schema(drop_policy: DropPolicy) -> Schema {
    Schema::Struct(vec![
        StructField { name: "glyph_count", schema: Schema::U16, bind: Some("glyph_count") },
        StructField { name: "glyphs", schema: Schema::Array(Box::new(Schema::GlyphId), CountSource::Field(EnvRef("glyph_count")), drop_policy), bind: None },
    ])
}

pub(crate) fn multiple_subst_schema() -> Schema {
    format_1(Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField {
            name: "entries",
            schema: Schema::CoveredArray(vec![], Box::new(coverage_indexed_glyph_array_schema(DropPolicy::AllOrNothing)), PayloadShape::Offsets(OffsetWidth::W16)),
            bind: None,
        },
    ]))
}

pub(crate) fn alternate_subst_schema() -> Schema {
    format_1(Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField {
            name: "entries",
            schema: Schema::CoveredArray(vec![], Box::new(coverage_indexed_glyph_array_schema(DropPolicy::FilterSurvivorsOrFail)), PayloadShape::Offsets(OffsetWidth::W16)),
            bind: None,
        },
    ]))
}

fn ligature_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "lig_glyph", schema: Schema::GlyphId, bind: None },
        StructField { name: "comp_count", schema: Schema::U16, bind: Some("comp_count") },
        StructField { name: "components", schema: Schema::Array(Box::new(Schema::GlyphId), CountSource::FieldMinusOne(EnvRef("comp_count")), DropPolicy::AllOrNothing), bind: None },
    ])
}

fn ligature_set_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "lig_count", schema: Schema::U16, bind: Some("lig_count") },
        StructField { name: "ligatures", schema: Schema::OffsetArray(Box::new(ligature_schema()), CountSource::Field(EnvRef("lig_count")), RebuildPolicy::CompactSurvivors, OffsetWidth::W16), bind: None },
    ])
}

pub(crate) fn ligature_subst_schema() -> Schema {
    format_1(Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField { name: "entries", schema: Schema::CoveredArray(vec![], Box::new(ligature_set_schema()), PayloadShape::Offsets(OffsetWidth::W16)), bind: None },
    ]))
}

pub(crate) fn context_subst_schema() -> Schema { context_schema() }
pub(crate) fn chain_context_subst_schema() -> Schema { chain_context_schema() }
