use alloc::vec::Vec;

use super::format::coverage::coverage_index;
use super::format::ivs::{units, variation_index, DeviceDeltas, ItemVariationStore};
use crate::daecore::daekit::base::sync::Shared;
use super::decoder::{read_i16_be, read_u16_be, read_uint, records_fit};
use super::subsetter::otl::gdef::GdefHeader;

// A ligature's carets in design units, x or y by direction, None where unresolved. `points` gives the glyph's points at
// the location and `store` GDEF's store, each asked only by a caret that needs it.
pub fn ligature_carets<P, F>(
    gdef: &[u8],
    gid: u32,
    points: P,
    location: &[f64],
    vertical: bool,
    store: F,
) -> Vec<Option<f64>>
where
    P: FnOnce() -> Option<(Vec<f64>, Vec<f64>)>,
    F: FnOnce() -> Option<Shared<ItemVariationStore>>,
{
    let mut out = Vec::new();
    let Some((list, width)) = lig_caret_list(gdef) else { return out };
    let long = if width == 3 { 4 } else { 2 };

    let Some(cov_off) = read_uint(gdef, list, long).map(|off| off as usize) else { return out };
    if cov_off == 0 { return out; }
    let Some(cov) = list.checked_add(cov_off).and_then(|at| gdef.get(at..)) else { return out };
    let Some(index) = coverage_index(cov, gid) else { return out };

    let Some(count) = read_uint(gdef, list + long, width) else { return out };
    if index >= count { return out; }

    let lig_off = read_uint(gdef, list + long + width + index as usize * width, width).map(|off| off as usize);
    let Some(lig_off) = lig_off else { return out };
    if lig_off == 0 { return out; }
    let Some(lig) = list.checked_add(lig_off) else { return out };

    let Some(caret_count) = read_u16_be(gdef, lig).map(usize::from) else { return out };
    if !records_fit(lig + 2, caret_count, 2, gdef.len()) { return out; }
    let mut deltas = Deltas { gdef, location, store: Some(store), made: None };
    let mut points = Points { make: Some(points), made: None };
    for i in 0..caret_count {
        let off = read_u16_be(gdef, lig + 2 + i * 2).map_or(0, usize::from);
        out.push(if off == 0 { None } else { resolve(gdef, lig + off, vertical, &mut points, &mut deltas) });
    }
    out
}

// The list and the width of its ligatures' count and offsets: 1.4's 24 bits, its coverage at a 32-bit offset.
fn lig_caret_list(gdef: &[u8]) -> Option<(usize, usize)> {
    let header = GdefHeader::read(gdef)?;
    let off = header.lig_caret_list;
    (off != 0 && off < gdef.len()).then_some((off, if header.lig_carets_24() { 3 } else { 2 }))
}

// The glyph's points, read at the first format 2 caret: most carets are not.
struct Points<P> {
    make: Option<P>,
    made: Option<Option<(Vec<f64>, Vec<f64>)>>,
}

impl<P: FnOnce() -> Option<(Vec<f64>, Vec<f64>)>> Points<P> {
    fn get(&mut self) -> Option<&(Vec<f64>, Vec<f64>)> {
        let make = &mut self.make;
        self.made.get_or_insert_with(|| make.take().and_then(|make| make())).as_ref()
    }
}

// The deltas format 3 carets need, worked out at the first that has a device: most carets are not.
struct Deltas<'a, F> {
    gdef: &'a [u8],
    location: &'a [f64],
    store: Option<F>,
    made: Option<Option<DeviceDeltas>>,
}

impl<F: FnOnce() -> Option<Shared<ItemVariationStore>>> Deltas<'_, F> {
    fn get(&mut self) -> Option<&mut DeviceDeltas> {
        if self.made.is_none() {
            let (gdef, location, store) = (self.gdef, self.location, self.store.take());
            let made = (|| {
                let at = GdefHeader::read(gdef)?.var_store;
                if at == 0 || location.iter().all(|&c| c == 0.0) { return None; }
                DeviceDeltas::from_store(store?()?, location)
            })();
            self.made = Some(made);
        }
        self.made.as_mut()?.as_mut()
    }
}

fn resolve<P, F>(
    gdef: &[u8], at: usize, vertical: bool, points: &mut Points<P>, deltas: &mut Deltas<'_, F>,
) -> Option<f64>
where
    P: FnOnce() -> Option<(Vec<f64>, Vec<f64>)>,
    F: FnOnce() -> Option<Shared<ItemVariationStore>>,
{
    match read_u16_be(gdef, at)? {
        1 => Some(f64::from(read_i16_be(gdef, at + 2)?)),
        2 => {
            let index = usize::from(read_u16_be(gdef, at + 2)?);
            let (xs, ys) = points.get()?;
            if vertical { ys.get(index).copied() } else { xs.get(index).copied() }
        }
        3 => {
            let base = f64::from(read_i16_be(gdef, at + 2)?);
            let device = usize::from(read_u16_be(gdef, at + 4)?);
            let varies = device != 0 && variation_index(gdef, at + device).is_some();
            let delta = if varies { deltas.get().map_or(0, |d| d.delta(gdef, at + device)) } else { 0 };
            Some(base + units(delta))
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/daetype/lig_caret.rs"]
mod tests;
