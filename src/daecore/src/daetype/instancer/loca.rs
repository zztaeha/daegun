use alloc::vec::Vec;
use crate::daecore::daetype::Tables;
use crate::daecore::Error;

pub fn parse_loca(table_map: &Tables, loca_format: i16, num_glyphs: usize) -> Result<Vec<usize>, Error> {
    let loca = table_map.get(super::super::decoder::glyf_tags(table_map).loca).ok_or(Error::missing("missing loca"))?;
    Ok(super::super::subsetter::parse_loca(loca, loca_format, num_glyphs))
}
