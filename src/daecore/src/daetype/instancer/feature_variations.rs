#[cfg(all(not(feature = "std"), not(test)))]
use crate::daecore::daemachine::float::FloatExt;
use alloc::vec::Vec;
use super::super::decoder::{read_u16_be, write_u16_be, write_u32_be};
use super::super::format::feature_variations::FeatureVariations;

fn f2dot14_coords(location: &[f64]) -> Vec<i32> {
    location.iter().map(|&v| (v * 16384.0).round() as i32).collect()
}

// GSUB and GPOS share the header FeatureVariations hangs from, so both resolve alike. `patched` is the
// table after another pass changed it, which the resolution then starts from.
pub(crate) fn resolve_layout(stored: Option<&[u8]>, patched: Option<Vec<u8>>, location: &[f64]) -> Option<Vec<u8>> {
    let layout = patched.as_deref().or(stored)?;
    resolve_feature_variations(layout, location).or(patched)
}

pub(crate) fn resolve_feature_variations(layout: &[u8], location: &[f64]) -> Option<Vec<u8>> {
    let table = FeatureVariations::parse(layout)?.with_axis_count(location.len());
    let feature_list = usize::from(read_u16_be(layout, 6)?);
    let feature_count = usize::from(read_u16_be(layout, feature_list)?);

    let mut out = layout.to_vec();
    write_u32_be(&mut out, 10, 0);
    let Some(variation) = table.find(&f2dot14_coords(location)) else { return Some(out) };
    let mut far = Vec::new();
    table.substitutions(variation, |feature, alternate| {
        let feature = usize::from(feature);
        if feature >= feature_count {
            return;
        }
        match alternate.checked_sub(feature_list).and_then(|rel| u16::try_from(rel).ok()) {
            Some(rel) => write_u16_be(&mut out, feature_list + 2 + feature * 6 + 4, rel),
            None => far.push((feature, alternate)),
        }
    });
    if !far.is_empty() {
        append_universal_variation(&mut out, layout, feature_list, &far)?;
    }
    Some(out)
}

// A FeatureRecord's offset is 16 bits, so an alternate out of its reach stays a substitution: one
// record whose condition set is empty, so it applies everywhere, with copies of the alternates after it.
fn append_universal_variation(out: &mut Vec<u8>, layout: &[u8], feature_list: usize, far: &[(usize, usize)]) -> Option<()> {
    let at = out.len();
    let subst_at = 16;
    let tables_at = subst_at + 6 + 6 * far.len();
    let mut tables = Vec::new();
    let mut records = Vec::with_capacity(far.len());
    for &(feature, alternate) in far {
        let lookups = usize::from(read_u16_be(layout, alternate.checked_add(2)?)?);
        let len = 4 + 2 * lookups;
        let body = layout.get(alternate..alternate.checked_add(len)?)?;
        let start = tables.len();
        tables.extend_from_slice(body);
        let params = usize::from(read_u16_be(layout, alternate)?);
        let tag = layout.get(feature_list + 2 + feature * 6..feature_list + 6 + feature * 6)?;
        let params = (params != 0).then(|| feature_params(layout, alternate.checked_add(params)?, tag)).flatten();
        match params {
            Some(bytes) => {
                write_u16_be(&mut tables, start, u16::try_from(len).ok()?);
                tables.extend_from_slice(bytes);
            }
            None => write_u16_be(&mut tables, start, 0),
        }
        records.push((feature, tables_at - subst_at + start));
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

// FeatureParams have no length of their own; the registered features that carry them define it.
fn feature_params<'a>(layout: &'a [u8], at: usize, tag: &[u8]) -> Option<&'a [u8]> {
    let len = match tag {
        b"size" => 10,
        [b's', b's', a, b] if a.is_ascii_digit() && b.is_ascii_digit() => 4,
        [b'c', b'v', a, b] if a.is_ascii_digit() && b.is_ascii_digit() => {
            14 + 3 * usize::from(read_u16_be(layout, at.checked_add(12)?)?)
        }
        _ => return None,
    };
    layout.get(at..at.checked_add(len)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use super::super::super::decoder::read_u32_be;

    // A GSUB 1.1 with two features whose second has an alternate for wght >= 0.5, `gap` bytes past
    // the FeatureList, and a FeatureVariations placed after the gap.
    fn gsub_with_variation(gap: usize) -> (Vec<u8>, usize) {
        let mut t = vec![0, 1, 0, 1, 0, 0, 0, 14, 0, 0, 0, 0, 0, 0];
        let feature_list = t.len();
        t.extend_from_slice(&[0, 2]);
        t.extend_from_slice(b"liga");
        t.extend_from_slice(&14u16.to_be_bytes());
        t.extend_from_slice(b"ss01");
        t.extend_from_slice(&20u16.to_be_bytes());
        t.extend_from_slice(&[0, 0, 0, 1, 0, 0]);
        t.extend_from_slice(&[0, 0, 0, 1, 0, 1]);
        t.resize(t.len() + gap, 0);
        let fv = t.len();
        write_u32_be(&mut t, 10, fv as u32);
        t.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 1]);
        t.extend_from_slice(&16u32.to_be_bytes());
        t.extend_from_slice(&36u32.to_be_bytes());
        t.extend_from_slice(&[0, 1, 0, 0, 0, 6]);
        t.extend_from_slice(&[0, 1, 0, 0, 0x20, 0, 0x40, 0]);
        t.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        t.extend_from_slice(&[0, 1, 0, 0, 0, 1]);
        t.extend_from_slice(&[0, 1, 0, 0, 0, 12]);
        t.extend_from_slice(&[0, 0, 0, 2, 0, 1, 0, 2]);
        (t, feature_list)
    }

    fn lookups_of(layout: &[u8], at: usize) -> Vec<u16> {
        let n = read_u16_be(layout, at + 2).unwrap();
        (0..n).map(|i| read_u16_be(layout, at + 4 + 2 * usize::from(i)).unwrap()).collect()
    }

    #[test]
    fn a_near_alternate_is_written_into_its_feature_record() {
        let (t, list) = gsub_with_variation(0);
        let out = resolve_feature_variations(&t, &[1.0]).unwrap();
        assert_eq!(read_u32_be(&out, 10), Some(0), "the instance keeps no FeatureVariations");
        let rel = usize::from(read_u16_be(&out, list + 2 + 6 + 4).unwrap());
        assert_eq!(lookups_of(&out, list + rel), [1, 2]);
        let at_default = resolve_feature_variations(&t, &[0.0]).unwrap();
        assert_eq!(lookups_of(&at_default, list + 20), [1], "outside the range the feature stays");
    }

    #[test]
    fn an_alternate_past_sixteen_bits_still_applies_to_the_instance() {
        let (t, list) = gsub_with_variation(70_000);
        let out = resolve_feature_variations(&t, &[1.0]).expect("resolves");
        let table = FeatureVariations::parse(&out).expect("the alternate stays a substitution");
        let variation = table.find(&[]).expect("its record applies at every location");
        let alternate = table.substitute(variation, 1).expect("feature 1 is substituted");
        assert_eq!(lookups_of(&out, alternate), [1, 2]);
        assert_eq!(read_u16_be(&out, alternate), Some(0), "liga and ss01 here carry no params");
        assert_eq!(table.substitute(variation, 0), None);
        assert_eq!(lookups_of(&out, list + 20), [1], "the stored feature is untouched");
    }

    // 30,000 features against 30,000 substitutions that name none of them: nine hundred million
    // record reads if each feature searches the substitutions again.
    #[test]
    fn every_feature_is_resolved_in_one_pass_over_the_substitutions() {
        const N: u16 = 30_000;
        let mut t = vec![0, 1, 0, 1, 0, 0, 0, 14, 0, 0, 0, 0, 0, 0];
        t.extend_from_slice(&N.to_be_bytes());
        let fv = t.len();
        write_u32_be(&mut t, 10, fv as u32);
        t.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 1]);
        t.extend_from_slice(&16u32.to_be_bytes());
        t.extend_from_slice(&18u32.to_be_bytes());
        t.extend_from_slice(&[0, 0, 0, 1, 0, 0]);
        t.extend_from_slice(&N.to_be_bytes());
        for _ in 0..N {
            t.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0, 6]);
        }
        let started = std::time::Instant::now();
        assert!(resolve_feature_variations(&t, &[1.0]).is_some());
        assert!(started.elapsed().as_secs_f64() < 2.0, "resolving took {:?}", started.elapsed());
    }

    #[test]
    fn a_condition_on_an_axis_the_font_lacks_ignores_its_record() {
        let (mut t, _) = gsub_with_variation(0);
        let fv = read_u32_be(&t, 10).unwrap() as usize;
        write_u16_be(&mut t, fv + 16 + 6 + 2, 3);
        write_u16_be(&mut t, fv + 16 + 6 + 4, 0xC000);
        let out = resolve_feature_variations(&t, &[1.0]).unwrap();
        assert_eq!(out[14 + 2 + 6 + 4..14 + 2 + 6 + 6], [0, 20], "axis 3 of a one-axis font read as 0 and matched");
    }
}
