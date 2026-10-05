use alloc::string::String;
use alloc::collections::BTreeMap;
use super::super::decoder::{read_u16_be, read_u32_be, read_i16_be, write_u16_be, write_i16_be};
use super::super::format::ivs::{parse_item_variation_store, compute_ivs_delta_f64, precompute_region_scalars};
use super::super::format::round::ot_round;
use crate::daecore::daetype::TableBytes;

pub fn mvar_deltas(
    table_map: &BTreeMap<String, TableBytes>,
    location:  &[f64],
) -> Result<BTreeMap<[u8; 4], i32>, String> {
    let mut out = BTreeMap::new();
    let mvar = match table_map.get("MVAR") { Some(m) => m, None => return Ok(out) };

    let value_record_size  = read_u16_be(mvar, 6).ok_or("MVAR: header truncated")? as usize;
    let value_record_count = read_u16_be(mvar, 8).ok_or("MVAR: header truncated")? as usize;
    let ivs_off            = read_u16_be(mvar, 10).ok_or("MVAR: header truncated")? as usize;
    if ivs_off == 0 || value_record_count == 0 { return Ok(out); }
    if value_record_size < 8 { return Err("MVAR: valueRecordSize too small".into()); }

    let store = parse_item_variation_store(mvar, ivs_off)?;
    let region_scalars = precompute_region_scalars(&store, location);

    let records_base = 12usize;
    for r in 0..value_record_count {
        let rec = records_base + r * value_record_size;
        let tag   = read_u32_be(mvar, rec).ok_or("MVAR: value record truncated")?;
        let outer = read_u16_be(mvar, rec + 4).ok_or("MVAR: value record truncated")? as usize;
        let inner = read_u16_be(mvar, rec + 6).ok_or("MVAR: value record truncated")? as usize;
        let delta = ot_round(compute_ivs_delta_f64(&store, outer, inner, &region_scalars));
        if delta != 0 {
            out.insert(tag.to_be_bytes(), delta);
        }
    }
    Ok(out)
}

// The tables MVAR's value tags reach, each as the instancer holds it; an absent one is empty.
pub(crate) struct MvarTargets<'a> {
    pub(crate) hhea: &'a mut [u8],
    pub(crate) vhea: &'a mut [u8],
    pub(crate) os2:  &'a mut [u8],
    pub(crate) post: &'a mut [u8],
    pub(crate) gasp: &'a mut [u8],
}

pub(crate) fn apply_mvar(
    table_map: &BTreeMap<String, TableBytes>,
    t:         MvarTargets,
    location:  &[f64],
) -> Result<(), String> {
    for (tag, delta) in mvar_deltas(table_map, location)? {
        let (buf, off, unsigned): (&mut [u8], usize, bool) = match &tag {
            b"hasc" => { bump(&mut *t.hhea, 4, delta, false); (&mut *t.os2, 68, false) }
            b"hdsc" => { bump(&mut *t.hhea, 6, delta, false); (&mut *t.os2, 70, false) }
            b"hlgp" => { bump(&mut *t.hhea, 8, delta, false); (&mut *t.os2, 72, false) }
            b"hcla" => (&mut *t.os2, 74, true),
            b"hcld" => (&mut *t.os2, 76, true),
            b"hcrs" => (&mut *t.hhea, 18, false),
            b"hcrn" => (&mut *t.hhea, 20, false),
            b"hcof" => (&mut *t.hhea, 22, false),
            b"vasc" => (&mut *t.vhea, 4, false),
            b"vdsc" => (&mut *t.vhea, 6, false),
            b"vlgp" => (&mut *t.vhea, 8, false),
            b"vcrs" => (&mut *t.vhea, 18, false),
            b"vcrn" => (&mut *t.vhea, 20, false),
            b"vcof" => (&mut *t.vhea, 22, false),
            b"xhgt" => (&mut *t.os2, 86, false),
            b"cpht" => (&mut *t.os2, 88, false),
            b"sbxs" => (&mut *t.os2, 10, false),
            b"sbys" => (&mut *t.os2, 12, false),
            b"sbxo" => (&mut *t.os2, 14, false),
            b"sbyo" => (&mut *t.os2, 16, false),
            b"spxs" => (&mut *t.os2, 18, false),
            b"spys" => (&mut *t.os2, 20, false),
            b"spxo" => (&mut *t.os2, 22, false),
            b"spyo" => (&mut *t.os2, 24, false),
            b"strs" => (&mut *t.os2, 26, false),
            b"stro" => (&mut *t.os2, 28, false),
            b"undo" => (&mut *t.post, 8, false),
            b"unds" => (&mut *t.post, 10, false),
            // gsp0 to gsp9: each gasp range's rangeMaxPPEM.
            [b'g', b's', b'p', n @ b'0'..=b'9'] => {
                let range = usize::from(n - b'0');
                if range >= read_u16_be(t.gasp, 2).map_or(0, usize::from) { continue; }
                (&mut *t.gasp, 4 + 4 * range, true)
            }
            _ => continue,
        };
        bump(buf, off, delta, unsigned);
    }
    Ok(())
}

fn bump(buf: &mut [u8], off: usize, delta: i32, unsigned: bool) {
    if off + 2 > buf.len() { return; }
    if unsigned {
        let v = read_u16_be(buf, off).unwrap_or(0) as i32;
        write_u16_be(buf, off, v.saturating_add(delta).clamp(0, 65535) as u16);
    } else {
        let v = read_i16_be(buf, off).unwrap_or(0) as i32;
        write_i16_be(buf, off, v.saturating_add(delta).clamp(-32768, 32767) as i16);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    // An MVAR whose gsp0, hcrs and vasc records move by 5, -7 and 9 at the axis's maximum.
    fn mvar() -> Vec<u8> {
        let mut t = vec![0, 1, 0, 0, 0, 0, 0, 8, 0, 3, 0, 36];
        for (tag, inner) in [(b"gsp0", 0u8), (b"hcrs", 1), (b"vasc", 2)] {
            t.extend(tag);
            t.extend([0, 0, 0, inner]);
        }
        t.extend([0, 1, 0, 0, 0, 12, 0, 1, 0, 0, 0, 22]);
        t.extend([0, 1, 0, 1, 0, 0, 0x40, 0, 0x40, 0]);
        t.extend([0, 3, 0, 0, 0, 1, 0, 0, 5, 0xF9, 9]);
        t
    }

    #[test]
    fn every_value_tag_reaches_its_table() {
        let map: BTreeMap<String, TableBytes> = [("MVAR".into(), TableBytes::from_vec(mvar()))].into_iter().collect();
        let (mut hhea, mut vhea) = (vec![0u8; 36], vec![0u8; 36]);
        hhea[18..20].copy_from_slice(&100i16.to_be_bytes());
        vhea[4..6].copy_from_slice(&500i16.to_be_bytes());
        let mut gasp = vec![0, 1, 0, 2, 0, 8, 0, 2, 0xFF, 0xFF, 0, 15];
        let targets = MvarTargets { hhea: &mut hhea, vhea: &mut vhea, os2: &mut [], post: &mut [], gasp: &mut gasp };
        apply_mvar(&map, targets, &[1.0]).expect("applies");
        assert_eq!((read_i16_be(&hhea, 18), read_i16_be(&vhea, 4), read_u16_be(&gasp, 4)), (Some(93), Some(509), Some(13)));
    }
}
