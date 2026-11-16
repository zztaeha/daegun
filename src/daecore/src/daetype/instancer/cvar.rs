use super::super::decoder::{read_i16_be, write_i16_be};
use super::super::format::ivs::{fixed_coords, round_fixed};
use super::drawn::cvt_deltas;
use crate::daecore::daetype::Tables;
use crate::daecore::Error;

// cvt at `location`, each entry moved by its delta rounded once, wrapping past its 16 bits.
pub fn apply_cvar(table_map: &Tables, cvt: &mut [u8], location: &[f64]) -> Result<(), Error> {
    let Some(cvar) = table_map.get("cvar") else { return Ok(()) };
    let num_cvts = cvt.len() / 2;
    if num_cvts == 0 {
        return Ok(());
    }
    let deltas = cvt_deltas(cvar, num_cvts, &fixed_coords(location)).ok_or(Error::malformed("cvar: header malformed"))?;
    for (i, &delta) in deltas.iter().enumerate() {
        if let Some(v) = read_i16_be(cvt, i * 2) {
            write_i16_be(cvt, i * 2, v.wrapping_add(round_fixed(delta) as i16));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/cvar.rs"]
mod tests;
