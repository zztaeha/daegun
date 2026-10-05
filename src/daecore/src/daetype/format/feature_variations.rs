use alloc::collections::BTreeMap;
use super::super::decoder::{read_i16_be, read_u16_be, read_u32_be};

pub struct FeatureVariations<'a> {
    data: &'a [u8],
    at: usize,
    axis_count: Option<usize>,
}

impl<'a> FeatureVariations<'a> {
    pub fn parse(layout: &'a [u8]) -> Option<Self> {
        if layout.len() < 14 || read_u16_be(layout, 2)? < 1 {
            return None;
        }
        match read_u32_be(layout, 10)? {
            0 => None,
            off => Some(Self { data: layout, at: off as usize, axis_count: None }),
        }
    }

    pub fn at(layout: &'a [u8], at: usize) -> Self {
        Self { data: layout, at, axis_count: None }
    }

    // The font's fvar axis count. A condition on an axis past it makes its record ignored, as the
    // spec says; without it, such an axis reads as 0, as HarfBuzz does.
    pub fn with_axis_count(mut self, count: usize) -> Self {
        self.axis_count = Some(count);
        self
    }

    pub fn find(&self, coords: &[i32]) -> Option<u16> {
        let count = read_u32_be(self.data, off(self.at, 4))?;
        // Records may share a condition set, and each distinct set is paid for by its own bytes, so
        // evaluating each once keeps the work linear in the table's size.
        let mut seen = BTreeMap::new();
        for i in 0..count {
            let rec = off(self.at, record(8, i as usize, 8));
            let cond_off = read_u32_be(self.data, rec)? as usize;
            let matches = cond_off == 0
                || *seen
                    .entry(cond_off)
                    .or_insert_with(|| self.condition_set_matches(off(self.at, cond_off), coords));
            if matches && self.substitution_version_supported(rec) {
                return u16::try_from(i).ok();
            }
        }
        None
    }

    // A record whose substitution table is of a version this reader does not know is passed over.
    fn substitution_version_supported(&self, rec: usize) -> bool {
        match read_u32_be(self.data, off(rec, 4)) {
            Some(0) => true,
            Some(rel) => read_u16_be(self.data, off(self.at, rel as usize)) == Some(1),
            None => false,
        }
    }

    fn condition_set_matches(&self, at: usize, coords: &[i32]) -> bool {
        let Some(count) = read_u16_be(self.data, at) else { return false };
        (0..count).all(|i| {
            read_u32_be(self.data, off(at, record(2, i as usize, 4)))
                .is_some_and(|rel| self.condition_matches(off(at, rel as usize), coords))
        })
    }

    fn condition_matches(&self, at: usize, coords: &[i32]) -> bool {
        if read_u16_be(self.data, at) != Some(1) {
            return false;
        }
        let (Some(axis), Some(min), Some(max)) = (
            read_u16_be(self.data, off(at, 2)),
            read_i16_be(self.data, off(at, 4)),
            read_i16_be(self.data, off(at, 6)),
        ) else {
            return false;
        };
        if self.axis_count.is_some_and(|n| usize::from(axis) >= n) {
            return false;
        }
        let v = coords.get(axis as usize).copied().unwrap_or(0);
        v >= min as i32 && v <= max as i32
    }

    pub fn substitute(&self, variation: u16, feature: u16) -> Option<usize> {
        let mut out = None;
        self.each_substitution(variation, |index, alternate| {
            if index < feature {
                return true;
            }
            if index == feature {
                out = alternate;
            }
            false
        });
        out
    }

    // Every feature the variation substitutes, in one pass: the spec's search for a feature stops at
    // the first record with an index at or above it, so a record counts only past every earlier one.
    pub(crate) fn substitutions(&self, variation: u16, mut apply: impl FnMut(u16, usize)) {
        let mut highest = None;
        self.each_substitution(variation, |index, alternate| {
            if highest.is_none_or(|h| index > h) {
                if let Some(at) = alternate {
                    apply(index, at);
                }
                highest = Some(index);
            }
            true
        });
    }

    fn each_substitution(&self, variation: u16, mut visit: impl FnMut(u16, Option<usize>) -> bool) {
        let rec = off(self.at, record(8, variation as usize, 8));
        let Some(subst_off) = read_u32_be(self.data, off(rec, 4)) else { return };
        if subst_off == 0 {
            return;
        }
        let at = off(self.at, subst_off as usize);
        let Some(count) = read_u16_be(self.data, off(at, 4)) else { return };
        for i in 0..count {
            let r = off(at, record(6, i as usize, 6));
            let (Some(index), Some(rel)) = (read_u16_be(self.data, r), read_u32_be(self.data, off(r, 2))) else {
                return;
            };
            if !visit(index, (rel != 0).then(|| off(at, rel as usize))) {
                return;
            }
        }
    }
}

// Offsets come from the font and `at` from the caller, so a sum may pass usize::MAX. Saturated, it
// reads past the end and answers None, as every other offset out of range does.
fn off(base: usize, rel: usize) -> usize {
    base.saturating_add(rel)
}

fn record(header: usize, index: usize, size: usize) -> usize {
    header.saturating_add(index.saturating_mul(size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;

    fn u16s(t: &mut Vec<u8>, v: &[u16]) {
        v.iter().for_each(|x| t.extend_from_slice(&x.to_be_bytes()));
    }

    fn u32s(t: &mut Vec<u8>, v: &[u32]) {
        v.iter().for_each(|x| t.extend_from_slice(&x.to_be_bytes()));
    }

    // A FeatureVariations table at 0 whose records are (condition set offset, substitution offset),
    // with the sets and substitutions appended by the caller at the offsets it names.
    fn table(records: &[(u32, u32)]) -> Vec<u8> {
        let mut t = vec![0, 1, 0, 0];
        u32s(&mut t, &[records.len() as u32]);
        records.iter().for_each(|&(c, s)| u32s(&mut t, &[c, s]));
        t
    }

    // A substitution table of the given version holding (feature, alternate offset) records.
    fn substitution(t: &mut Vec<u8>, major: u16, records: &[(u16, u32)]) {
        u16s(t, &[major, 0, records.len() as u16]);
        records.iter().for_each(|&(f, a)| {
            u16s(t, &[f]);
            u32s(t, &[a]);
        });
    }

    #[test]
    fn a_record_with_no_condition_set_matches_everywhere() {
        let mut t = table(&[(0, 16)]);
        substitution(&mut t, 1, &[(3, 12)]);
        let fv = FeatureVariations::at(&t, 0);
        assert_eq!(fv.find(&[]), Some(0));
        assert_eq!(fv.find(&[16384, -16384]), Some(0));
        assert_eq!(fv.substitute(0, 3), Some(16 + 12));
    }

    #[test]
    fn a_substitution_table_of_an_unknown_version_passes_its_record_over() {
        let mut t = table(&[(24, 26), (24, 38)]);
        u16s(&mut t, &[0]);
        substitution(&mut t, 2, &[(0, 126)]);
        substitution(&mut t, 1, &[(0, 240)]);
        let fv = FeatureVariations::at(&t, 0);
        assert_eq!(fv.find(&[]), Some(1), "both empty condition sets match; the first table is 2.0");
        assert_eq!(fv.substitute(1, 0), Some(38 + 240));
    }

    #[test]
    fn the_search_for_a_feature_stops_at_a_higher_index() {
        let mut t = table(&[(0, 16)]);
        substitution(&mut t, 1, &[(2, 100), (9, 200), (3, 300), (9, 400), (12, 0), (14, 500)]);
        let fv = FeatureVariations::at(&t, 0);
        let one_by_one: Vec<_> = (0..16).filter_map(|f| fv.substitute(0, f).map(|a| (f, a))).collect();
        assert_eq!(one_by_one, [(2, 116), (9, 216), (14, 516)], "feature 3 follows 9 and is never reached");
        let mut in_one_pass = Vec::new();
        fv.substitutions(0, |f, a| in_one_pass.push((f, a)));
        assert_eq!(in_one_pass, one_by_one);
    }

    // 20,000 records naming one set of 20,000 conditions, of which the last fails: four hundred
    // million condition reads if each record evaluates the set again.
    #[test]
    fn records_sharing_a_condition_set_evaluate_it_once() {
        const N: usize = 20_000;
        let set = 8 + 8 * N;
        let mut t = table(&vec![(set as u32, 0); N]);
        let conditions = set + 2 + 4 * N;
        u16s(&mut t, &[N as u16]);
        for i in 0..N {
            let failing = if i == N - 1 { 8 } else { 0 };
            u32s(&mut t, &[(conditions - set + failing) as u32]);
        }
        u16s(&mut t, &[1, 0, 0xC000, 0x4000, 1, 0, 0x2000, 0x4000]);
        let fv = FeatureVariations::at(&t, 0);
        let started = std::time::Instant::now();
        assert_eq!(fv.find(&[0]), None);
        assert!(started.elapsed().as_secs_f64() < 2.0, "find took {:?}", started.elapsed());
    }
}
