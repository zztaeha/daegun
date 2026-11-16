use alloc::string::String;
use alloc::vec::Vec;
use crate::daecore::daetype::{TableBytes, Tables};

// `colr` with a CPAL of one palette of one color, `bgra`: COLR is ignored without one.
pub(crate) fn tables(colr: &[u8], bgra: [u8; 4]) -> Tables {
    let cpal = [&[0u16, 1, 1, 1, 0, 14, 0].map(u16::to_be_bytes).concat()[..], &bgra].concat();
    let tables = [("COLR", colr.to_vec()), ("CPAL", cpal)];
    tables.into_iter().map(|(t, b)| (String::from(t), TableBytes::from(b))).collect()
}

// An item variation store over one axis whose `subtables` data subtables each hold one row:
// `delta` at the axis's peak.
pub(crate) fn store(subtables: u16, delta: i16) -> Vec<u8> {
    let header = 8 + 4 * usize::from(subtables);
    let mut out = 1u16.to_be_bytes().to_vec();
    out.extend((header as u32).to_be_bytes());
    out.extend(subtables.to_be_bytes());
    for _ in 0..subtables {
        out.extend((header as u32 + 10).to_be_bytes());
    }
    out.extend([1u16, 1, 0, 0x4000, 0x4000].map(u16::to_be_bytes).concat());
    out.extend([1u16, 1, 1, 0].map(u16::to_be_bytes).concat());
    out.extend(delta.to_be_bytes());
    out
}

// A COLR whose base glyphs 1 to `bases` share one PaintColrLayers of `layers` PaintGlyphs
// (glyphs 1000 on) in the foreground, a PaintVarSolid whose delta takes alpha to 0 if `varied`.
pub(crate) fn shared_layers(bases: u16, layers: u8, varied: bool) -> Vec<u8> {
    let list = 34usize;
    let colr_layers = list + 4 + 6 * usize::from(bases);
    let layer_list = colr_layers + 6;
    let paints = layer_list + 4 + 4 * usize::from(layers);
    let solid: Vec<u8> = if varied {
        [&[3u8, 0xFF, 0xFF, 0x40, 0][..], &0u32.to_be_bytes()].concat()
    } else {
        alloc::vec![2, 0xFF, 0xFF, 0x40, 0]
    };
    let paint_len = 6 + solid.len();
    let store_at = paints + paint_len * usize::from(layers);
    let mut out = alloc::vec![0u8; 34];
    out[1] = 1;
    out[14..18].copy_from_slice(&(list as u32).to_be_bytes());
    out[18..22].copy_from_slice(&(layer_list as u32).to_be_bytes());
    if varied {
        out[30..34].copy_from_slice(&(store_at as u32).to_be_bytes());
    }
    out.extend(u32::from(bases).to_be_bytes());
    for gid in 1..=bases {
        out.extend(gid.to_be_bytes());
        out.extend(((colr_layers - list) as u32).to_be_bytes());
    }
    out.extend([1, layers, 0, 0, 0, 0]);
    out.extend(u32::from(layers).to_be_bytes());
    for i in 0..usize::from(layers) {
        out.extend(((paints + i * paint_len - layer_list) as u32).to_be_bytes());
    }
    for i in 0..u16::from(layers) {
        out.extend([10, 0, 0, 6]);
        out.extend((1000 + i).to_be_bytes());
        out.extend(&solid);
    }
    if varied {
        out.extend(store(1, -16384));
    }
    out
}
