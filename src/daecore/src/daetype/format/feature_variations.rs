use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use super::super::decoder::{read_i16_be, read_u16_be, read_u24_be, read_u32_be};
use super::ivs::{store_delta, store_scalars, units, ItemVariationStore};
use super::super::subsetter::subset_budget;

/// The feature a FeatureVariations record puts in place of one of the table's own.
#[expect(clippy::exhaustive_enums, reason = "an offset or a null one, as the table stores it")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Substitution {
    /// The alternate feature table, at this offset into the layout table.
    Feature(usize),
    /// A null offset: the empty feature, which applies no lookups.
    Empty,
}

/// A GSUB or GPOS FeatureVariations table: which features a variable font swaps for others where.
#[derive(Clone, Copy)]
pub struct FeatureVariations<'a> {
    data: &'a [u8],
    at: usize,
}

impl core::fmt::Debug for FeatureVariations<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FeatureVariations").field("at", &self.at).finish_non_exhaustive()
    }
}

// One search's coordinates, the store and region scalars a value condition varies by, the answer of each
// condition set and condition already worked out, as records and conditions may share them, and the reads left.
struct Search<'s> {
    coords: &'s [i32],
    store: Option<(&'s ItemVariationStore, Vec<i64>)>,
    sets: BTreeMap<usize, bool>,
    conditions: BTreeMap<usize, bool>,
    left: Option<usize>,
}

impl<'s> Search<'s> {
    // A value condition adds `store`'s delta at the coordinates, and nothing at the default, where all are 0.
    fn new(coords: &'s [i32], store: Option<&'s ItemVariationStore>, left: usize) -> Self {
        let location: Vec<f64> = coords.iter().map(|&c| f64::from(c) / 16384.0).collect();
        let store = store.filter(|_| coords.iter().any(|&c| c != 0)).map(|s| (s, store_scalars(s, &location)));
        Search { coords, store, sets: BTreeMap::new(), conditions: BTreeMap::new(), left: Some(left) }
    }

    fn delta(&self, index: u32) -> f64 {
        self.store.as_ref().map_or(0.0, |(store, scalars)| {
            units(store_delta(store, (index >> 16) as usize, usize::from(index as u16), scalars))
        })
    }
}

impl<'a> FeatureVariations<'a> {
    /// The table a GSUB or GPOS of version 1.1 or later names; None for an earlier version or a null offset.
    pub fn parse(layout: &'a [u8]) -> Option<Self> {
        if layout.len() < 14 || read_u16_be(layout, 0)? != 1 || read_u16_be(layout, 2)? < 1 {
            return None;
        }
        match read_u32_be(layout, 10)? {
            0 => None,
            off => Some(Self { data: layout, at: off as usize }),
        }
    }

    /// The table at offset `at` in `layout`, read whatever its header says.
    pub const fn at(layout: &'a [u8], at: usize) -> Self {
        Self { data: layout, at }
    }

    #[cfg(feature = "capi")]
    pub(crate) fn offset(&self) -> usize {
        self.at
    }

    // The reads one walk over the table may make, of records, conditions and lookups alike: twice its bytes and 64 KB
    // more, well past what a table whose parts don't overlap reads.
    pub(crate) fn budget(&self) -> usize {
        subset_budget(self.data.len())
    }

    /// The first record whose conditions all hold at normalized coordinates `coords`, in F2Dot14 (16384 is 1), an
    /// axis past them at 0 and a value condition varied by `store`; None where none does, or where the reads it may
    /// make run out first.
    pub fn find(&self, coords: &[i32], store: Option<&ItemVariationStore>) -> Option<u32> {
        let mut search = Search::new(coords, store, self.budget());
        let count = read_u32_be(self.data, off(self.at, 4))?;
        for i in 0..count {
            let set = read_u32_be(self.data, off(self.at, record(8, i as usize, 8)))? as usize;
            let holds = set == 0 || self.set_holds(off(self.at, set), &mut search);
            search.left?;
            if holds {
                return Some(i);
            }
        }
        None
    }

    // Past the reads left a set or a nested condition fails, and the search drops its answer.
    fn set_holds(&self, at: usize, search: &mut Search) -> bool {
        if let Some(&known) = search.sets.get(&at) {
            return known;
        }
        let holds = read_u16_be(self.data, at).is_some_and(|count| {
            spend(&mut search.left, usize::from(count)) && (0..count).all(|i| {
                read_u32_be(self.data, off(at, record(2, i as usize, 4)))
                    .is_some_and(|rel| self.holds(at, rel as usize, search, 0))
            })
        });
        search.sets.insert(at, holds);
        holds
    }

    // The condition `rel` from `base`. A null one is of format 0, the true condition; one nested deeper than a table
    // that passes its checks can nest fails, as does one that can't be read.
    fn holds(&self, base: usize, rel: usize, search: &mut Search, depth: u8) -> bool {
        if rel == 0 {
            return true;
        }
        if depth >= 64 {
            return false;
        }
        let at = off(base, rel);
        if let Some(&known) = search.conditions.get(&at) {
            return known;
        }
        let i16_at = |o| read_i16_be(self.data, off(at, o));
        let nested = |o| read_u24_be(self.data, off(at, o)).map_or(usize::MAX, |rel| rel as usize);
        let holds = match read_u16_be(self.data, at) {
            Some(0) => true,
            Some(1) => match (read_u16_be(self.data, off(at, 2)), i16_at(4), i16_at(6)) {
                (Some(axis), Some(min), Some(max)) => {
                    let coord = search.coords.get(usize::from(axis)).copied().unwrap_or(0);
                    (i32::from(min)..=i32::from(max)).contains(&coord)
                }
                _ => false,
            },
            Some(2) => match (i16_at(2), read_u32_be(self.data, off(at, 4))) {
                (Some(default), Some(index)) => f64::from(default) + search.delta(index) > 0.0,
                _ => false,
            },
            Some(format @ (3 | 4)) => {
                let count = usize::from(self.data.get(off(at, 2)).copied().unwrap_or(0));
                spend(&mut search.left, count) && {
                    let mut each = (0..count).map(|i| self.holds(at, nested(3 + 3 * i), search, depth + 1));
                    if format == 3 { each.all(|h| h) } else { each.any(|h| h) }
                }
            }
            Some(5) => !self.holds(at, nested(2), search, depth + 1),
            _ => false,
        };
        search.conditions.insert(at, holds);
        holds
    }

    // The conditions holding at `coords` of the lookup variations read before the reads run out, by offset, sorted:
    // what sets a location's lookups apart. Each table is read once; a null condition always holds, so sets none apart.
    pub(crate) fn lookup_conditions(&self, coords: &[i32], store: Option<&ItemVariationStore>) -> Vec<u32> {
        let mut search = Search::new(coords, store, self.budget());
        let (mut tables, mut held) = (BTreeSet::new(), BTreeSet::new());
        'tables: for (_, table) in self.lookup_variations() {
            let Some(table) = table.filter(|&t| tables.insert(t)) else { continue };
            for (condition, _) in self.lookup_condition_records(table) {
                let holds =
                    spend(&mut search.left, 1) && condition != 0 && self.holds(table, condition, &mut search, 0);
                if search.left.is_none() {
                    break 'tables;
                }
                if holds {
                    held.insert((off(table, condition) - self.at) as u32);
                }
            }
        }
        held.into_iter().collect()
    }

    // The lookups of `feature` where a lookup variation record names it, the first that does, as `lookups` gives
    // them; None where none does, or where its table runs past the reads `left`, which a plan's features share.
    pub(crate) fn varied_lookups(
        &self, feature: u16, conditions: &[u32], own: impl Iterator<Item = u16>, left: &mut usize,
    ) -> Option<BTreeSet<u16>> {
        let (_, table) = self.lookup_variations().find(|&(f, _)| f == feature)?;
        self.lookups(table, conditions, own, left)
    }

    // Each feature a lookup variation record names, with the FeatureLookups table of the first that does.
    pub(crate) fn varied_features(&self) -> BTreeMap<u16, Option<usize>> {
        let mut first = BTreeMap::new();
        self.lookup_variations().for_each(|(feature, table)| {
            first.entry(feature).or_insert(table);
        });
        first
    }

    // A FeatureLookups table's lookups where `conditions` hold: `own`, the feature's lookups as substituted, where the
    // table adds them, then each list whose condition is null or among them. A null table gives none; None past the
    // reads `left`, which the caller may share among tables.
    pub(crate) fn lookups(
        &self, table: Option<usize>, conditions: &[u32], own: impl Iterator<Item = u16>, left: &mut usize,
    ) -> Option<BTreeSet<u16>> {
        let mut lookups = BTreeSet::new();
        let Some(table) = table else { return Some(lookups) };
        if read_u16_be(self.data, off(table, 4)).is_some_and(|flags| flags & 1 != 0) {
            for lookup in own {
                *left = left.checked_sub(1)?;
                lookups.insert(lookup);
            }
        }
        let mut lists = BTreeSet::new();
        for (condition, list) in self.lookup_condition_records(table) {
            *left = left.checked_sub(1)?;
            let offset = (off(table, condition) - self.at) as u32;
            if (condition == 0 || conditions.binary_search(&offset).is_ok()) && lists.insert(list) {
                for lookup in self.lookup_list(table, list) {
                    *left = left.checked_sub(1)?;
                    lookups.insert(lookup);
                }
            }
        }
        Some(lookups)
    }

    // The lookups of every alternate substituted for one of `features`, and of every list its lookup variations give,
    // whatever their conditions: what a subset keeps. Each table and list is read once; None past one walk's reads.
    pub(crate) fn variant_lookups(&self, features: &BTreeSet<u16>) -> Option<BTreeSet<u16>> {
        let mut left = Some(self.budget());
        let (mut lookups, mut tables, mut alternates, mut lists) =
            (BTreeSet::new(), BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
        // Only the records that fit are walked, whatever count the table states.
        let fit = self.data.len().saturating_sub(off(self.at, 8)) / 8;
        let count = read_u32_be(self.data, off(self.at, 4)).map_or(0, |n| (n as usize).min(fit));
        for variation in 0..count as u32 {
            let Some(at) = self.substitution_table(variation).filter(|&at| tables.insert(at)) else { continue };
            self.each_substitution(at, |feature, alternate| {
                let fresh = alternate.filter(|&a| features.contains(&feature) && alternates.insert(a));
                // A feature table's lookups are laid out as a list is, two bytes in.
                let mut read = fresh.into_iter().flat_map(|a| self.lookup_list(a, 2));
                spend(&mut left, 1) && read.all(|lookup| {
                    lookups.insert(lookup);
                    spend(&mut left, 1)
                })
            });
            left?;
        }
        let mut left = left?;
        tables.clear();
        for (_, table) in self.lookup_variations().filter(|(feature, _)| features.contains(feature)) {
            let Some(table) = table.filter(|&t| tables.insert(t)) else { continue };
            for (_, list) in self.lookup_condition_records(table) {
                left = left.checked_sub(1)?;
                if list != 0 && lists.insert(off(table, list)) {
                    for lookup in self.lookup_list(table, list) {
                        left = left.checked_sub(1)?;
                        lookups.insert(lookup);
                    }
                }
            }
        }
        Some(lookups)
    }

    // The lookups a FeatureLookups table's list names, none for a null list.
    fn lookup_list(&self, table: usize, list: usize) -> impl Iterator<Item = u16> + '_ {
        let at = off(table, list);
        let count = if list == 0 { 0 } else { read_u16_be(self.data, at).unwrap_or(0) };
        (0..usize::from(count)).map_while(move |i| read_u16_be(self.data, off(at, 2 + 2 * i)))
    }

    // From version 1.1, each lookup variation record's feature and FeatureLookups table, None where it's null.
    fn lookup_variations(&self) -> impl Iterator<Item = (u16, Option<usize>)> + '_ {
        let version = read_u32_be(self.data, self.at).unwrap_or(0);
        let records = read_u32_be(self.data, off(self.at, 4)).map_or(usize::MAX, |n| record(8, n as usize, 8));
        let at = off(self.at, records);
        let count = if version < 0x0001_0001 { 0 } else { read_u32_be(self.data, at).unwrap_or(0) };
        (0..count as usize).map_while(move |i| {
            let r = off(at, record(4, i, 6));
            let rel = read_u32_be(self.data, off(r, 2))? as usize;
            Some((read_u16_be(self.data, r)?, (rel != 0).then(|| off(self.at, rel))))
        })
    }

    // A FeatureLookups table's records, each its condition's and its lookup list's offset from the table.
    fn lookup_condition_records(&self, table: usize) -> impl Iterator<Item = (usize, usize)> + '_ {
        let count = read_u32_be(self.data, off(table, 6)).unwrap_or(0);
        (0..count as usize).map_while(move |i| {
            let r = off(table, record(10, i, 8));
            Some((read_u32_be(self.data, r)? as usize, read_u32_be(self.data, off(r, 4))? as usize))
        })
    }

    /// The feature that record `variation` puts in place of feature index `feature`; None where it substitutes
    /// none.
    pub fn substitute(&self, variation: u32, feature: u16) -> Option<Substitution> {
        self.alternate(variation, feature).map(|at| at.map_or(Substitution::Empty, Substitution::Feature))
    }

    // As `substitute`, the alternate as the shaper reads a feature: where it lies, None for the empty one.
    pub(crate) fn alternate(&self, variation: u32, feature: u16) -> Option<Option<usize>> {
        let mut out = None;
        self.each_substitution(self.substitution_table(variation)?, |index, alternate| {
            if index == feature {
                out = Some(alternate);
            }
            out.is_none()
        });
        out
    }

    // Every feature the variation substitutes, each by its first record, in one pass.
    pub(crate) fn substitutions(&self, variation: u32, mut apply: impl FnMut(u16, Option<usize>)) {
        let mut seen = BTreeSet::new();
        let Some(at) = self.substitution_table(variation) else { return };
        self.each_substitution(at, |index, alternate| {
            if seen.insert(index) {
                apply(index, alternate);
            }
            true
        });
    }

    // The FeatureTableSubstitution record `variation` names, None for a null one.
    fn substitution_table(&self, variation: u32) -> Option<usize> {
        let rec = off(self.at, record(8, variation as usize, 8));
        let rel = read_u32_be(self.data, off(rec, 4)).filter(|&rel| rel != 0)?;
        Some(off(self.at, rel as usize))
    }

    // Each record of the FeatureTableSubstitution at `at`, while `visit` asks for more.
    fn each_substitution(&self, at: usize, mut visit: impl FnMut(u16, Option<usize>) -> bool) {
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

// Takes `n` reads from those `left`, which stay None once they run out; whether any were left.
fn spend(left: &mut Option<usize>, n: usize) -> bool {
    *left = left.and_then(|left| left.checked_sub(n));
    left.is_some()
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
#[path = "../../../tests/unit/daetype/format/feature_variations.rs"]
mod tests;
