use alloc::boxed::Box;
use super::format_1;
use super::super::schema::{Schema, StructField, CountSource, DropPolicy, OffsetWidth, PayloadShape, EmptyPolicy, EnvRef};
use super::context::{context_schema, chain_context_schema};

fn single_pos_format1_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField { name: "coverage", schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::Coverage(EmptyPolicy::Fail, None))), bind: None },
        StructField { name: "value_format", schema: Schema::ValueFormatField(EnvRef("vf")), bind: None },
        StructField { name: "value", schema: Schema::ValueRecordField(EnvRef("vf")), bind: None },
    ])
}

fn single_pos_format2_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField {
            name: "entries",
            schema: Schema::CoveredArray(
                vec![StructField { name: "value_format", schema: Schema::ValueFormatField(EnvRef("vf")), bind: None }],
                Box::new(Schema::ValueRecordField(EnvRef("vf"))),
                PayloadShape::Inline,
            ),
            bind: None,
        },
    ])
}

pub(crate) fn single_pos_schema() -> Schema {
    Schema::FormatSwitch(0, vec![
        (1, single_pos_format1_schema()),
        (2, single_pos_format2_schema()),
    ])
}

fn pair_value_record_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "second_glyph", schema: Schema::GlyphId, bind: None },
        StructField { name: "value1", schema: Schema::ValueRecordField(EnvRef("vf1")), bind: None },
        StructField { name: "value2", schema: Schema::ValueRecordField(EnvRef("vf2")), bind: None },
    ])
}

fn pair_set_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "pair_value_count", schema: Schema::U16, bind: Some("pair_value_count") },
        StructField { name: "pairs", schema: Schema::Array(Box::new(pair_value_record_schema()), CountSource::Field(EnvRef("pair_value_count")), DropPolicy::FilterSurvivorsOrFail), bind: None },
    ])
}

fn pair_pos_format1_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField {
            name: "entries",
            schema: Schema::CoveredArray(
                vec![
                    StructField { name: "value_format1", schema: Schema::ValueFormatField(EnvRef("vf1")), bind: None },
                    StructField { name: "value_format2", schema: Schema::ValueFormatField(EnvRef("vf2")), bind: None },
                ],
                Box::new(pair_set_schema()),
                PayloadShape::Offsets(OffsetWidth::W16),
            ),
            bind: None,
        },
    ])
}

fn pair_pos_format2_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField { name: "coverage", schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::Coverage(EmptyPolicy::Fail, None))), bind: None },
        StructField { name: "value_format1", schema: Schema::ValueFormatField(EnvRef("vf1")), bind: None },
        StructField { name: "value_format2", schema: Schema::ValueFormatField(EnvRef("vf2")), bind: None },
        StructField { name: "class_matrix", schema: Schema::ClassMatrix(EnvRef("vf1"), EnvRef("vf2")), bind: None },
    ])
}

pub(crate) fn pair_pos_schema() -> Schema {
    Schema::FormatSwitch(0, vec![
        (1, pair_pos_format1_schema()),
        (2, pair_pos_format2_schema()),
    ])
}

fn cursive_pos_entry_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "entry_anchor", schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::Anchor)), bind: None },
        StructField { name: "exit_anchor", schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::Anchor)), bind: None },
    ])
}

pub(crate) fn cursive_pos_schema() -> Schema {
    format_1(Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField { name: "entries", schema: Schema::CoveredArray(vec![], Box::new(cursive_pos_entry_schema()), PayloadShape::Inline), bind: None },
    ]))
}

fn mark_record_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "mark_class", schema: Schema::U16, bind: None },
        StructField { name: "mark_anchor", schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::Anchor)), bind: None },
    ])
}

fn base_record_schema() -> Schema {
    Schema::Array(Box::new(Schema::Offset(OffsetWidth::W16, Box::new(Schema::Anchor))), CountSource::Field(EnvRef("class_count")), DropPolicy::FilterSurvivors)
}

pub(crate) fn mark_attach_schema() -> Schema {
    format_1(Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField { name: "mark_coverage", schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::Coverage(EmptyPolicy::Fail, Some(EnvRef("mark_coverage"))))), bind: None },
        StructField { name: "base_coverage", schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::Coverage(EmptyPolicy::Fail, Some(EnvRef("base_coverage"))))), bind: None },
        StructField { name: "class_count", schema: Schema::U16, bind: Some("class_count") },
        StructField {
            name: "marks",
            schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::ZippedWithBoundCoverage(EnvRef("mark_coverage"), Box::new(mark_record_schema()), PayloadShape::Inline, DropPolicy::FilterSurvivorsOrFail))),
            bind: None,
        },
        StructField {
            name: "bases",
            schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::ZippedWithBoundCoverage(EnvRef("base_coverage"), Box::new(base_record_schema()), PayloadShape::Inline, DropPolicy::FilterSurvivorsOrFail))),
            bind: None,
        },
    ]))
}

fn component_anchors_schema() -> Schema {
    Schema::Array(Box::new(Schema::Offset(OffsetWidth::W16, Box::new(Schema::Anchor))), CountSource::Field(EnvRef("class_count")), DropPolicy::FilterSurvivors)
}

fn ligature_attach_schema() -> Schema {
    Schema::Struct(vec![
        StructField { name: "component_count", schema: Schema::U16, bind: Some("component_count") },
        StructField { name: "components", schema: Schema::Array(Box::new(component_anchors_schema()), CountSource::Field(EnvRef("component_count")), DropPolicy::FilterSurvivors), bind: None },
    ])
}

pub(crate) fn mark_lig_pos_schema() -> Schema {
    format_1(Schema::Struct(vec![
        StructField { name: "format", schema: Schema::U16, bind: None },
        StructField { name: "mark_coverage", schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::Coverage(EmptyPolicy::Fail, Some(EnvRef("mark_coverage"))))), bind: None },
        StructField { name: "ligature_coverage", schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::Coverage(EmptyPolicy::Fail, Some(EnvRef("lig_coverage"))))), bind: None },
        StructField { name: "class_count", schema: Schema::U16, bind: Some("class_count") },
        StructField {
            name: "marks",
            schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::ZippedWithBoundCoverage(EnvRef("mark_coverage"), Box::new(mark_record_schema()), PayloadShape::Inline, DropPolicy::FilterSurvivorsOrFail))),
            bind: None,
        },
        StructField {
            name: "ligatures",
            schema: Schema::Offset(OffsetWidth::W16, Box::new(Schema::ZippedWithBoundCoverage(EnvRef("lig_coverage"), Box::new(ligature_attach_schema()), PayloadShape::Offsets(OffsetWidth::W16), DropPolicy::FilterSurvivorsOrFail))),
            bind: None,
        },
    ]))
}

pub(crate) fn context_pos_schema() -> Schema { context_schema() }
pub(crate) fn chain_context_pos_schema() -> Schema { chain_context_schema() }
