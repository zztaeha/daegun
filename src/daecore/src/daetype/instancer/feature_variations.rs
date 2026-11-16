#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daekit::float::FloatExt;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, write_u16_be, write_u32_be};
use super::super::format::feature_variations::FeatureVariations;
use super::super::format::ivs::ItemVariationStore;
use super::super::subsetter::otl::{layout_lists, FeatureParams};
use super::super::subsetter::subset_budget;

fn f2dot14_coords(location: &[f64]) -> Vec<i32> {
    location.iter().map(|&v| (v * 16384.0).round() as i32).collect()
}

// GSUB and GPOS share the header FeatureVariations hangs from, so both resolve alike, their value conditions by
// GDEF's `store`. `patched` is the table after another pass changed it, which the resolution then starts from.
pub(crate) fn resolve_layout(
    stored: Option<&[u8]>, patched: Option<Vec<u8>>, location: &[f64], store: Option<&ItemVariationStore>,
) -> Option<Vec<u8>> {
    let layout = patched.as_deref().or(stored)?;
    resolve_feature_variations(layout, location, store).or(patched)
}

// None where the tables it writes beyond the FeatureList's 16-bit reach would pass twice the table's bytes and 64 KB,
// which leaves the table as stored.
pub(crate) fn resolve_feature_variations(
    layout: &[u8], location: &[f64], store: Option<&ItemVariationStore>,
) -> Option<Vec<u8>> {
    let table = FeatureVariations::parse(layout)?;
    let ([_, feature_list, _], _) = layout_lists(layout)?;
    let feature_count = usize::from(read_u16_be(layout, feature_list)?);
    let coords = f2dot14_coords(location);

    let mut out = layout.to_vec();
    write_u32_be(&mut out, 10, 0);
    // Each feature the location's record substitutes, by its alternate, None for the empty one.
    let mut alternates = BTreeMap::new();
    if let Some(variation) = table.find(&coords, store) {
        table.substitutions(variation, |feature, alternate| {
            if usize::from(feature) < feature_count {
                alternates.insert(usize::from(feature), alternate);
            }
        });
    }
    let varied = table.varied_features();
    let conditions = if varied.is_empty() { Vec::new() } else { table.lookup_conditions(&coords, store) };
    let (mut far, mut far_left) = (Vec::new(), subset_budget(layout.len()));
    let mut push_far = |feature: usize, table: Option<Option<Vec<u8>>>| {
        far_left = far_left.checked_sub(table.iter().flatten().map(Vec::len).sum())?;
        far.push((feature, table));
        Some(())
    };
    for (&feature, &alternate) in alternates.iter().filter(|&(&f, _)| !varied.contains_key(&(f as u16))) {
        let rel = alternate.and_then(|a| a.checked_sub(feature_list)).and_then(|rel| u16::try_from(rel).ok());
        match rel.filter(|&rel| rel != 0) {
            Some(rel) => { write_u16_be(&mut out, feature_list + 2 + feature * 6 + 4, rel); }
            None => push_far(feature, alternate.map(|a| feature_table(layout, feature_list, feature, Some(a), None)))?,
        }
    }
    // Past the reads the lookup variations may take, a feature keeps its own lookups.
    let mut left = table.budget();
    // A feature with lookup variations takes them as they stand here, on top of its alternate or its own table.
    for (&feature, &lookups) in varied.iter().filter(|&(&f, _)| usize::from(f) < feature_count) {
        let feature = usize::from(feature);
        let record = feature_list + 2 + feature * 6;
        let own = match alternates.get(&feature) {
            Some(&alternate) => alternate,
            None => {
                let rel = usize::from(read_u16_be(layout, record + 4)?);
                (rel != 0).then_some(feature_list + rel)
            }
        };
        let own_lookups = own.into_iter().flat_map(|at| lookups_at(layout, at));
        let lookups = table.lookups(lookups, &conditions, own_lookups, &mut left);
        let bytes = feature_table(layout, feature_list, feature, own, lookups.as_ref());
        match u16::try_from(out.len() - feature_list) {
            Ok(rel) => {
                write_u16_be(&mut out, record + 4, rel);
                out.extend_from_slice(&bytes?);
            }
            Err(_) => push_far(feature, Some(bytes))?,
        }
    }
    if !far.is_empty() {
        append_universal_variation(&mut out, &far)?;
    }
    Some(out)
}

fn lookups_at(layout: &[u8], at: usize) -> impl Iterator<Item = u16> + '_ {
    let count = read_u16_be(layout, at + 2).unwrap_or(0);
    (0..usize::from(count)).map_while(move |i| read_u16_be(layout, at + 4 + 2 * i))
}

// Feature `feature`'s table written anew from the one at `at`, none for the empty one: its params, read by the tag
// its record gives, and `lookups`, or the table's own where None.
fn feature_table(
    layout: &[u8], feature_list: usize, feature: usize, at: Option<usize>, lookups: Option<&BTreeSet<u16>>,
) -> Option<Vec<u8>> {
    let lookups: Vec<u16> = match lookups {
        Some(lookups) => lookups.iter().copied().collect(),
        None => at.into_iter().flat_map(|at| lookups_at(layout, at)).collect(),
    };
    let words = [0, u16::try_from(lookups.len()).ok()?].into_iter().chain(lookups);
    let mut table: Vec<u8> = words.flat_map(u16::to_be_bytes).collect();
    let tag = layout.get(feature_list + 2 + feature * 6..feature_list + 6 + feature * 6)?;
    let params = at.and_then(|at| {
        let params = at.checked_add(usize::from(read_u16_be(layout, at)?)).filter(|&p| p != at)?;
        layout.get(params..params.checked_add(FeatureParams::of(tag)?.len(layout, params)?)?)
    });
    if let Some(bytes) = params {
        let len = u16::try_from(table.len()).ok()?;
        write_u16_be(&mut table, 0, len);
        table.extend_from_slice(bytes);
    }
    Some(table)
}

// A FeatureRecord's offset is 16 bits, so a feature table out of its reach stays a substitution, as does the empty
// one a null offset names: one record whose condition set is empty, so it applies everywhere, with the tables.
fn append_universal_variation(out: &mut Vec<u8>, far: &[(usize, Option<Option<Vec<u8>>>)]) -> Option<()> {
    let at = out.len();
    let subst_at = 16;
    let tables_at = subst_at + 6 + 6 * far.len();
    let mut tables = Vec::new();
    let mut records = Vec::with_capacity(far.len());
    for (feature, table) in far {
        match table {
            Some(table) => {
                records.push((*feature, tables_at - subst_at + tables.len()));
                tables.extend_from_slice(table.as_ref()?);
            }
            None => records.push((*feature, 0)),
        }
    }

    out.extend_from_slice(&[0, 1, 0, 0]);
    out.extend_from_slice(&1u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&(subst_at as u32).to_be_bytes());
    out.extend_from_slice(&[0, 1, 0, 0]);
    out.extend_from_slice(&u16::try_from(far.len()).ok()?.to_be_bytes());
    for (feature, rel) in records {
        out.extend_from_slice(&u16::try_from(feature).ok()?.to_be_bytes());
        out.extend_from_slice(&u32::try_from(rel).ok()?.to_be_bytes());
    }
    out.extend_from_slice(&tables);
    write_u32_be(out, 10, u32::try_from(at).ok()?);
    Some(())
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/instancer/feature_variations.rs"]
mod tests;
