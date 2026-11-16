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
    let out = resolve_feature_variations(&t, &[1.0], None).unwrap();
    assert_eq!(read_u32_be(&out, 10), Some(0), "the instance keeps no FeatureVariations");
    let rel = usize::from(read_u16_be(&out, list + 2 + 6 + 4).unwrap());
    assert_eq!(lookups_of(&out, list + rel), [1, 2]);
    let at_default = resolve_feature_variations(&t, &[0.0], None).unwrap();
    assert_eq!(lookups_of(&at_default, list + 20), [1], "outside the range the feature stays");
}

// The same table as 1.2, its FeatureList named at the 32-bit field alone, resolves alike.
#[test]
fn a_version_1_2_table_resolves_through_its_32_bit_feature_list() {
    let (t, list) = gsub_with_variation(0);
    let (fv, feature_list) = (read_u32_be(&t, 10).unwrap() + 12, list as u32 + 12);
    let fields = [fv.to_be_bytes(), [0; 4], feature_list.to_be_bytes(), [0; 4]].concat();
    let t = [&[0, 1, 0, 2, 0, 0, 0, 0, 0, 0][..], &fields, &t[14..]].concat();
    let out = resolve_feature_variations(&t, &[1.0], None).unwrap();
    let rel = usize::from(read_u16_be(&out, list + 12 + 2 + 6 + 4).unwrap());
    assert_eq!(lookups_of(&out, list + 12 + rel), [1, 2]);
}

#[test]
fn an_alternate_past_sixteen_bits_still_applies_to_the_instance() {
    let (t, list) = gsub_with_variation(70_000);
    let out = resolve_feature_variations(&t, &[1.0], None).expect("resolves");
    let table = FeatureVariations::parse(&out).expect("the alternate stays a substitution");
    let variation = table.find(&[], None).expect("its record applies at every location");
    let alternate = table.alternate(variation, 1).flatten().expect("feature 1 is substituted");
    assert_eq!(lookups_of(&out, alternate), [1, 2]);
    assert_eq!(read_u16_be(&out, alternate), Some(0), "liga and ss01 here carry no params");
    assert_eq!(table.substitute(variation, 0), None);
    assert_eq!(lookups_of(&out, list + 20), [1], "the stored feature is untouched");
}

// A GSUB 1.1 whose ss01, lookup 1, has lookup variations: its own lookups, lookup 2 where axis 0 is from 0.5 to 1,
// and lookup 3 everywhere. The instance writes each feature's lookups as they stand at its location.
#[test]
fn a_feature_takes_its_varied_lookups_in_the_instance() {
    let mut t = vec![0, 1, 0, 1, 0, 0, 0, 14, 0, 0, 0, 0, 0, 40];
    let list = t.len();
    t.extend_from_slice(&[0, 2]);
    t.extend_from_slice(b"liga");
    t.extend_from_slice(&14u16.to_be_bytes());
    t.extend_from_slice(b"ss01");
    t.extend_from_slice(&20u16.to_be_bytes());
    t.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 1, 0, 1]);
    t.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 0, 0, 0, 18]);
    t.extend_from_slice(&[0, 1, 0, 0, 0, 1, 0, 0, 0, 2]);
    [34u32, 26, 0, 30].iter().for_each(|w| t.extend_from_slice(&w.to_be_bytes()));
    t.extend_from_slice(&[0, 1, 0, 2, 0, 1, 0, 3, 0, 1, 0, 0, 0x20, 0, 0x40, 0]);
    let ss01 = |out: &[u8]| lookups_of(out, list + usize::from(read_u16_be(out, list + 2 + 6 + 4).unwrap()));
    let heavy = resolve_feature_variations(&t, &[1.0], None).expect("resolves");
    assert_eq!((read_u32_be(&heavy, 10), ss01(&heavy)), (Some(0), vec![1, 2, 3]));
    assert_eq!(lookups_of(&heavy, list + 14), [0], "liga has no lookup variations");
    assert_eq!(ss01(&resolve_feature_variations(&t, &[0.0], None).expect("resolves")), [1, 3]);
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
    assert!(resolve_feature_variations(&t, &[1.0], None).is_some());
    assert!(started.elapsed().as_secs_f64() < 2.0, "resolving took {:?}", started.elapsed());
}


// Features each substituted everywhere by one alternate of 20,000 lookups beyond the FeatureList's reach, written
// again for each: past twice the table's bytes at 10 features, though not at 3, the table stays as stored.
#[test]
fn alternates_written_past_the_output_budget_leave_the_table_as_stored() {
    let resolved = |features: u16| {
        let mut t = vec![0, 1, 0, 1, 0, 0, 0, 14, 0, 0, 0, 0, 0, 0];
        t.extend_from_slice(&features.to_be_bytes());
        for _ in 0..features {
            t.extend_from_slice(b"liga");
            t.extend_from_slice(&(2 + 6 * features).to_be_bytes());
        }
        t.extend_from_slice(&[0; 4]);
        t.resize(t.len() + 70_000, 0);
        let fv = t.len();
        write_u32_be(&mut t, 10, fv as u32);
        t.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 16, 0, 1, 0, 0]);
        t.extend_from_slice(&features.to_be_bytes());
        let alternate = 6 + 6 * u32::from(features);
        for feature in 0..features {
            t.extend_from_slice(&feature.to_be_bytes());
            t.extend_from_slice(&alternate.to_be_bytes());
        }
        t.extend_from_slice(&[0, 0]);
        t.extend_from_slice(&20_000u16.to_be_bytes());
        t.resize(t.len() + 40_000, 0);
        resolve_feature_variations(&t, &[1.0], None)
    };
    assert!(resolved(3).is_some());
    assert!(resolved(10).is_none());
}

// Ten features, each with a lookup variation naming one list of 20,000 references to lookup 5, each feature's own
// table lookup 7: the walks share one budget of reads, so the features past it keep their own lookups.
#[test]
fn features_past_the_reads_their_lookup_variations_share_keep_their_own() {
    const FEATURES: u16 = 10;
    let mut t = vec![0, 1, 0, 1, 0, 0, 0, 14, 0, 0, 0, 0, 0, 0];
    let list = t.len();
    t.extend_from_slice(&FEATURES.to_be_bytes());
    for _ in 0..FEATURES {
        t.extend_from_slice(b"liga");
        t.extend_from_slice(&(2 + 6 * FEATURES).to_be_bytes());
    }
    t.extend_from_slice(&[0, 0, 0, 1, 0, 7]);
    let fv = t.len();
    write_u32_be(&mut t, 10, fv as u32);
    t.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
    t.push(FEATURES as u8);
    let table = 12 + 6 * u32::from(FEATURES);
    for feature in 0..FEATURES {
        t.extend_from_slice(&feature.to_be_bytes());
        t.extend_from_slice(&table.to_be_bytes());
    }
    t.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 18]);
    t.extend_from_slice(&20_000u16.to_be_bytes());
    (0..20_000).for_each(|_| t.extend_from_slice(&[0, 5]));
    let out = resolve_feature_variations(&t, &[0.0], None).expect("resolves");
    let lookups = |feature: usize| {
        lookups_of(&out, list + usize::from(read_u16_be(&out, list + 6 + 6 * feature).expect("an offset")))
    };
    assert_eq!((lookups(0), lookups(9)), (vec![5], vec![7]));
}
