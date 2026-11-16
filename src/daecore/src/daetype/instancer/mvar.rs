use alloc::collections::BTreeMap;
use super::super::decoder::{bump_i16, bump_u16, midpoint_search, read_i16_be, read_u16_be, read_u32_be};
use super::super::format::ivs::{round_fixed, store_delta, store_scalars, ItemVariationStore};
use crate::daecore::daetype::Tables;
use crate::daecore::Error;

// Each tag's delta, its record found by `midpoint_search`, so a record out of order varies nothing and a repeated
// tag takes the record that search meets first.
fn mvar_deltas(
    table_map: &Tables,
    location:  &[f64],
) -> Result<BTreeMap<[u8; 4], i64>, Error> {
    let mut out = BTreeMap::new();
    let mvar = match table_map.get("MVAR") { Some(m) => m, None => return Ok(out) };

    if read_u16_be(mvar, 0).ok_or(Error::malformed("MVAR: header truncated"))? != 1 {
        return Err(Error::unsupported("MVAR: major version not 1"));
    }
    let size    = read_u16_be(mvar, 6).ok_or(Error::malformed("MVAR: header truncated"))? as usize;
    let count   = read_u16_be(mvar, 8).ok_or(Error::malformed("MVAR: header truncated"))? as usize;
    let ivs_off = read_u16_be(mvar, 10).ok_or(Error::malformed("MVAR: header truncated"))? as usize;
    if size < 8 { return Err(Error::malformed("MVAR: valueRecordSize too small")); }
    if 12 + count * size > mvar.len() { return Err(Error::malformed("MVAR: value records truncated")); }
    if ivs_off == 0 { return Ok(out); }

    let store = ItemVariationStore::parse(mvar, ivs_off)?;
    let region_scalars = store_scalars(&store, location);
    let tag_at = |i: usize| read_u32_be(mvar, 12 + i * size);
    for i in 0..count {
        let tag = tag_at(i).ok_or(Error::malformed("MVAR: value record truncated"))?;
        if out.contains_key(&tag.to_be_bytes()) { continue; }
        let delta = midpoint_search(count, |j| tag_at(j).map(|t| tag.cmp(&t))).map_or(0, |found| {
            let rec = 12 + found * size;
            let [outer, inner] = [4, 6].map(|at| read_u16_be(mvar, rec + at).map_or(0, usize::from));
            store_delta(&store, outer, inner, &region_scalars)
        });
        out.insert(tag.to_be_bytes(), delta);
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
    table_map: &Tables,
    t:         MvarTargets,
    location:  &[f64],
) -> Result<(), Error> {
    for (tag, delta) in mvar_deltas(table_map, location)? {
        let (buf, off, unsigned): (&mut [u8], usize, bool) = match &tag {
            b"hasc" => { vary(t.hhea, 4, false, delta); (&mut *t.os2, 68, false) }
            b"hdsc" => { vary(t.hhea, 6, false, delta); (&mut *t.os2, 70, false) }
            b"hlgp" => { vary(t.hhea, 8, false, delta); (&mut *t.os2, 72, false) }
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
        vary(buf, off, unsigned, delta);
    }
    Ok(())
}

// A field moved by its 16.16 delta, rounded once.
fn vary(buf: &mut [u8], off: usize, unsigned: bool, delta: i64) {
    let read = if unsigned { read_u16_be(buf, off).map(i32::from) } else { read_i16_be(buf, off).map(i32::from) };
    if read.is_none() {
        return;
    }
    let by = round_fixed(delta);
    if unsigned { bump_u16(buf, off, by) } else { bump_i16(buf, off, by) }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/mvar.rs"]
mod tests;
