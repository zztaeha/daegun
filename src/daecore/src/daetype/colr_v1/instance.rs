use alloc::vec::Vec;
use super::super::colr_v0::colr_v0_header;
use super::super::decoder::{read_u16_be, records_fit};
use super::super::instancer::strip_colr_var_store;
use super::super::subsetter::subset_budget;
use super::parse_colr_v1_var_data;
use super::write::{assemble, rebuild, Location};

// The table at a location, static: every Var paint and variable clip box written as its static
// form there, and no variation data left. A table the writer cannot lay out keeps its default.
pub(crate) fn instance_colr_v1(colr: &[u8], location: &[f64]) -> Option<Vec<u8>> {
    let header = super::header(colr)?;
    header.var_store?;

    let Some(source) = parse_colr_v1_var_data(colr) else {
        return strip_colr_var_store(colr);
    };
    let scalars = source.scalars(location);
    let at = Location { source: &source, scalars: &scalars };
    let v1 = rebuild(colr, &|g| Some(g), &|g| g, None, Some(&at), subset_budget(colr.len()));
    if v1.is_none() && header.base_glyph_list.is_some() {
        return strip_colr_var_store(colr);
    }
    assemble(read_u16_be(colr, 0)?, v0_records(colr), v1).or_else(|| strip_colr_var_store(colr))
}

fn v0_records(colr: &[u8]) -> Option<(&[u8], u16, &[u8], u16)> {
    let (n, base, layers, m) = colr_v0_header(colr)?;
    if n == 0 || !records_fit(base, n, 6, colr.len()) || !records_fit(layers, m, 4, colr.len()) {
        return None;
    }
    Some((&colr[base..base + n * 6], n as u16, &colr[layers..layers + m * 4], m as u16))
}
