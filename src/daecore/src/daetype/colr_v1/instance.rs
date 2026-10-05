use alloc::vec::Vec;
use super::super::colr_v0::colr_v0_header;
use super::super::decoder::{read_u16_be, read_u32_be, records_fit};
use super::super::format::ivs::precompute_region_scalars;
use super::super::instancer::strip_colr_var_store;
use super::super::subsetter::subset_budget;
use super::parse_colr_v1_var_data;
use super::write::{assemble, rebuild, Location};

// The table at a location, static: every Var paint and variable clip box written as its static
// form there, and no variation data left. A table the writer cannot lay out keeps its default.
pub(crate) fn instance_colr_v1(colr: &[u8], location: &[f64]) -> Option<Vec<u8>> {
    if colr.len() < 34 { return None; }
    if read_u16_be(colr, 0)? != 1 { return None; }
    if read_u32_be(colr, 30)? == 0 { return None; }

    let var_data = parse_colr_v1_var_data(colr);
    let Some(store) = var_data.var_store.as_ref() else {
        return strip_colr_var_store(colr);
    };
    let scalars = precompute_region_scalars(store, location);
    let at = Location { store, map: var_data.var_index_map.as_deref(), scalars: &scalars };
    let v1 = rebuild(colr, &|g| Some(g), &|g| g, Some(&at), subset_budget(colr.len()));
    if v1.is_none() && read_u32_be(colr, 14)? != 0 {
        return strip_colr_var_store(colr);
    }
    assemble(1, v0_records(colr), v1).or_else(|| strip_colr_var_store(colr))
}

fn v0_records(colr: &[u8]) -> Option<(&[u8], u16, &[u8], u16)> {
    let (n, base, layers, m) = colr_v0_header(colr)?;
    if n == 0 || !records_fit(base, n, 6, colr.len()) || !records_fit(layers, m, 4, colr.len()) {
        return None;
    }
    Some((&colr[base..base + n * 6], n as u16, &colr[layers..layers + m * 4], m as u16))
}
