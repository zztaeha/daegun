use alloc::collections::BTreeSet;
use alloc::rc::Rc;
use alloc::vec::Vec;
pub mod schema;
mod value;
mod parse;
mod build;
mod graph;
mod vary;
pub mod schemas;

use schema::Schema;
use build::generic_build;
use parse::{generic_parse, Env};
pub(crate) use graph::{Graph, Link};
pub(crate) use value::Value;
pub(crate) use parse::{Devices, Kept, Work};
pub(crate) use vary::{InPlace, Stop};

// A subtable parsed against its schema and rebuilt: the glyphs `kept` keeps, under their new IDs, its
// Device tables kept, dropped or applied; `work` is spent across the whole table.
pub(crate) fn subset_subtable(
    buf: &[u8], off: usize, schema: &Schema, kept: Kept, devices: Devices, work: &mut Work,
) -> Option<Vec<u8>> {
    let value = parse_subtable(buf, off, schema, kept, devices, work)?;
    generic_build(&value, work)
}

// A subtable read for a layout subset: its value, written once the table's lookups are numbered, or bytes.
pub(crate) enum Parsed<'b> {
    Value(Value<'b>),
    Bytes(Vec<u8>),
}

// A value format keeps only the Device fields some record of it fills, so one whose records left a field empty, as
// an instance's VariationIndex tables leave theirs, is narrowed with its records.
pub(crate) fn parse_subtable<'b>(
    buf: &'b [u8], off: usize, schema: &Schema, kept: Kept, devices: Devices, work: &mut Work,
) -> Option<Value<'b>> {
    let mut env = Env::new(devices, kept, work);
    let mut value = generic_parse(buf, off, off, schema, &mut env).ok()?.0?;
    if let Some(written) = env.unfilled() {
        drop(env);
        value::narrow(&mut value, &written);
    }
    Some(value)
}

pub(crate) fn build_value(value: &Value<'_>, work: &mut Work) -> Option<Vec<u8>> {
    generic_build(value, work)
}

// A subtable as an object of `graph`.
pub(crate) fn emit_value(value: &Value<'_>, graph: &mut Graph, work: &mut Work) -> Option<usize> {
    graph::generic_emit(value, graph, &mut work.built_coverages, work.lookups.as_deref())
}

// The lookups a subtable's records name. A child many offsets share is walked once: `seen` holds those walked.
pub(crate) fn named_lookups<'b>(value: &Value<'b>, seen: &mut BTreeSet<*const Value<'b>>, out: &mut Vec<u16>) {
    let mut each = |v: &Value<'b>| named_lookups(v, seen, out);
    match value {
        Value::LookupIndex(lookup) => out.push(*lookup),
        Value::Array(values) | Value::Struct(values) => values.iter().for_each(&mut each),
        Value::Offset(_, Some(child)) => named_once(child, seen, out),
        Value::OffsetArray(_, slots) => slots.iter().flatten().for_each(|child| named_once(child, seen, out)),
        Value::ZippedWithBoundCoverage(_, _, slots) => slots.iter().flatten().for_each(&mut each),
        Value::CoveredArray(c) => {
            c.fields.iter().for_each(&mut each);
            c.entries.iter().for_each(|(_, v)| each(v));
        }
        _ => {}
    }
}

fn named_once<'b>(child: &Rc<Value<'b>>, seen: &mut BTreeSet<*const Value<'b>>, out: &mut Vec<u16>) {
    if seen.insert(Rc::as_ptr(child)) {
        named_lookups(child, seen, out);
    }
}

#[cfg(test)]
#[path = "../../../../../tests/unit/daetype/subsetter/otl/generic/mod.rs"]
mod tests;
