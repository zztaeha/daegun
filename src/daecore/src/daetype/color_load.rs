use super::format::sanitize::{colr_passes, cpal_passes};

// Whether drawing reads a font's COLR: one whose header passes beside a CPAL whose header does, as the spec has COLR
// read with CPAL. A structure inside either that leaves its table is absent where it's read.
pub(crate) fn colr_drawn(colr: &[u8], cpal: Option<&[u8]>) -> bool {
    colr_passes(colr) && cpal.is_some_and(cpal_passes)
}

#[cfg(test)]
#[path = "../../tests/unit/daetype/color_load.rs"]
mod tests;
