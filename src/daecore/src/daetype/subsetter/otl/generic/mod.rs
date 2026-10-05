use crate::daecore::daetype::subsetter::GlyphSet;
use alloc::vec::Vec;
pub mod schema;
mod value;
mod parse;
mod build;
pub mod schemas;

use schema::Schema;
use build::generic_build;
use parse::{generic_parse, Env};
pub(crate) use parse::{Devices, Work};

// A subtable parsed against its schema and rebuilt: the glyphs that survive under their new IDs,
// its Device tables kept, dropped or applied; `work` is spent across the whole table being subset.
#[allow(clippy::too_many_arguments)]
pub(crate) fn subset_subtable(
    buf: &[u8],
    off: usize,
    schema: &Schema,
    active: &GlyphSet,
    gid_map: &[u16],
    devices: Devices,
    work: &mut Work,
) -> Option<Vec<u8>> {
    let mut env = Env::new(devices, work);
    let value = generic_parse(buf, off, off, schema, &mut env, active, gid_map).ok()?.0?;
    generic_build(&value, work)
}

#[cfg(test)]
mod tests;
