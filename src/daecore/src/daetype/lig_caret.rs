use alloc::vec::Vec;

use super::format::coverage::coverage_index;
use super::format::ivs::{DeviceDeltas, ItemVariationStore};
use crate::daecore::sync::Shared;
use super::decoder::{read_i16_be, read_u16_be, read_u32_be, records_fit};

const LIG_CARET_LIST_OFF: usize = 8;
const ITEM_VAR_STORE_OFF: usize = 14;

// A ligature's carets in design units, x or y by direction, None where one is unresolved. `points` gives
// the glyph's points at the location and `store` GDEF's variation store, each asked only by a caret that
// needs it.
pub fn ligature_carets<P, F>(
    gdef: &[u8],
    gid: u16,
    points: P,
    location: &[f64],
    vertical: bool,
    store: F,
) -> Vec<Option<f64>>
where
    P: FnOnce() -> Option<(Vec<f64>, Vec<f64>)>,
    F: FnOnce(usize) -> Option<Shared<ItemVariationStore>>,
{
    let mut out = Vec::new();
    let Some(list) = lig_caret_list(gdef) else { return out };

    let Some(cov_off) = read_u16_be(gdef, list).map(usize::from) else { return out };
    if cov_off == 0 { return out; }
    let Some(cov) = gdef.get(list + cov_off..) else { return out };
    let Some(index) = coverage_index(cov, gid) else { return out };

    let Some(count) = read_u16_be(gdef, list + 2) else { return out };
    if index >= count { return out; }

    let Some(lig_off) = read_u16_be(gdef, list + 4 + index as usize * 2).map(usize::from) else { return out };
    if lig_off == 0 { return out; }
    let lig = list + lig_off;

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

fn lig_caret_list(gdef: &[u8]) -> Option<usize> {
    let off = read_u16_be(gdef, LIG_CARET_LIST_OFF)? as usize;
    if off == 0 { return None; }
    (off < gdef.len()).then_some(off)
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

impl<F: FnOnce(usize) -> Option<Shared<ItemVariationStore>>> Deltas<'_, F> {
    fn get(&mut self) -> Option<&mut DeviceDeltas> {
        if self.made.is_none() {
            let (gdef, location, store) = (self.gdef, self.location, self.store.take());
            let made = (|| {
                if read_u16_be(gdef, 2)? < 3 || location.iter().all(|&c| c == 0.0) { return None; }
                let at = read_u32_be(gdef, ITEM_VAR_STORE_OFF)? as usize;
                if at == 0 { return None; }
                DeviceDeltas::from_store(store?(at)?, location)
            })();
            self.made = Some(made);
        }
        self.made.as_mut()?.as_mut()
    }
}

fn resolve<P, F>(gdef: &[u8], at: usize, vertical: bool, points: &mut Points<P>, deltas: &mut Deltas<'_, F>) -> Option<f64>
where
    P: FnOnce() -> Option<(Vec<f64>, Vec<f64>)>,
    F: FnOnce(usize) -> Option<Shared<ItemVariationStore>>,
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
            let varies = device != 0 && read_u16_be(gdef, at + device + 4) == Some(0x8000);
            let delta = if varies { deltas.get().map_or(0.0, |d| d.delta(gdef, at + device)) } else { 0.0 };
            Some(base + delta)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::format::ivs::parse_item_variation_store;

    // GDEF's own store, as a font keeps it.
    fn parsed_store(gdef: &[u8]) -> impl FnOnce(usize) -> Option<Shared<ItemVariationStore>> + '_ {
        move |at| parse_item_variation_store(gdef, at).ok().map(Shared::new)
    }

    fn words(w: &[u16]) -> Vec<u8> {
        w.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    // GDEF 1.0 whose ligature, glyph 1, lists `count` carets at the offsets `offsets` from its LigGlyph,
    // the caret tables `carets` after them.
    fn gdef(count: u16, offsets: &[u16], carets: &[u8]) -> Vec<u8> {
        let lig_glyph = 2 + 2 * offsets.len() as u16 + carets.len() as u16;
        let mut t = words(&[1, 0, 0, 0, 12, 0, 6 + lig_glyph, 1, 6, count]);
        t.extend(words(offsets));
        t.extend(carets);
        t.extend(words(&[1, 1, 1]));
        t
    }

    // A point with no outline to read it from cannot be resolved; the caret after it keeps its place
    // rather than taking the first's.
    #[test]
    fn a_caret_that_cannot_be_resolved_keeps_its_place() {
        let t = gdef(2, &[6, 10], &words(&[2, 1, 1, 600]));
        assert_eq!(ligature_carets(&t, 1, || None, &[], false, parsed_store(&t)), [None, Some(600.0)]);
    }

    // Four carets listed and the table ending after one offset and the caret it names: no carets, not
    // the one read before the array runs out.
    #[test]
    fn a_caret_list_cut_short_gives_none() {
        let t = words(&[1, 0, 0, 0, 12, 0, 6, 1, 12, 1, 1, 1, 4, 4, 1, 600]);
        assert_eq!(ligature_carets(&t, 1, || None, &[], false, parsed_store(&t)), []);
    }

    // 8,192 carets naming one format 3 caret at 300 whose delta row spans 16,384 regions, each adding 1
    // at the peak: worked out per caret, 134 million multiply-adds a call.
    #[test]
    fn a_shared_delta_is_worked_out_once() {
        const CARETS: u16 = 8_192;
        const REGIONS: u16 = 16_384;
        let caret_at = 2 + 2 * CARETS;
        let mut t = words(&[1, 3, 0, 0, 18, 0, 0]);
        let coverage = 18 + 6 + caret_at as usize + 12;
        let store = coverage + 6;
        t.extend((store as u32).to_be_bytes());
        t.extend(words(&[(coverage - 18) as u16, 1, 6, CARETS]));
        (0..CARETS).for_each(|_| t.extend(caret_at.to_be_bytes()));
        t.extend(words(&[3, 300, 6, 0, 0, 0x8000, 1, 1, 1]));
        t.extend(words(&[1, 0, 12, 1]));
        t.extend((16 + 6 * u32::from(REGIONS)).to_be_bytes());
        t.extend(words(&[1, REGIONS]));
        (0..REGIONS).for_each(|_| t.extend(words(&[0, 0x4000, 0x4000])));
        t.extend(words(&[1, 0, REGIONS]));
        t.extend((0..REGIONS).flat_map(u16::to_be_bytes));
        t.extend(core::iter::repeat_n(1u8, usize::from(REGIONS)));
        let started = std::time::Instant::now();
        let carets = ligature_carets(&t, 1, || None, &[1.0], false, parsed_store(&t));
        assert!(started.elapsed().as_secs_f64() < 1.0, "took {:?}", started.elapsed());
        assert_eq!(carets.len(), usize::from(CARETS));
        assert!(carets.iter().all(|&c| c == Some(300.0 + f64::from(REGIONS))), "{:?}", &carets[..1]);
    }
}
