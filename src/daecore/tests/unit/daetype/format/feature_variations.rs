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

// A substitution table holding (feature, alternate offset) records.
fn substitution(t: &mut Vec<u8>, records: &[(u16, u32)]) {
    u16s(t, &[1, 0, records.len() as u16]);
    records.iter().for_each(|&(f, a)| {
        u16s(t, &[f]);
        u32s(t, &[a]);
    });
}

#[test]
fn a_record_with_no_condition_set_matches_everywhere() {
    let mut t = table(&[(0, 16)]);
    substitution(&mut t, &[(3, 12)]);
    let fv = FeatureVariations::at(&t, 0);
    assert_eq!(fv.find(&[], None), Some(0));
    assert_eq!(fv.find(&[16384, -16384], None), Some(0));
    assert_eq!(fv.substitute(0, 3), Some(Substitution::Feature(16 + 12)));
}

#[test]
fn a_feature_is_substituted_by_its_first_record() {
    let mut t = table(&[(0, 16)]);
    substitution(&mut t, &[(2, 100), (9, 200), (3, 300), (9, 400), (12, 0), (14, 500)]);
    let fv = FeatureVariations::at(&t, 0);
    let one_by_one: Vec<_> = (0..16).filter_map(|f| fv.alternate(0, f).map(|a| (f, a))).collect();
    let expected = [(2, Some(116)), (3, Some(316)), (9, Some(216)), (12, None), (14, Some(516))];
    assert_eq!(one_by_one, expected, "feature 3 follows 9, and 12's null alternate is the empty feature");
    assert_eq!(fv.substitute(0, 12), Some(Substitution::Empty), "a null alternate is the empty feature");
    assert_eq!(fv.substitute(0, 14), Some(Substitution::Feature(516)));
    let mut in_one_pass = Vec::new();
    fv.substitutions(0, |f, a| in_one_pass.push((f, a)));
    in_one_pass.sort();
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
    assert_eq!(fv.find(&[0], None), None);
    assert!(started.elapsed().as_secs_f64() < 2.0, "find took {:?}", started.elapsed());
}

// Ten levels of conditions, each all of 255 references to the next, over an axis range that holds: 255^10
// evaluations unless each condition is worked out once.
#[test]
fn a_condition_named_many_times_is_worked_out_once() {
    let mut t = table(&[(16, 0)]);
    u16s(&mut t, &[1, 0, 6]);
    for _ in 0..10 {
        u16s(&mut t, &[3]);
        t.push(255);
        (0..255).for_each(|_| t.extend_from_slice(&[0, 3, 0]));
    }
    u16s(&mut t, &[1, 0, 0xC000, 0x4000]);
    let started = std::time::Instant::now();
    assert_eq!(FeatureVariations::at(&t, 0).find(&[], None), Some(0));
    assert!(started.elapsed().as_secs_f64() < 2.0, "find took {:?}", started.elapsed());
}

// A store's bytes from its words, a delta of -32,767 among them.
fn store(words: &[i32]) -> ItemVariationStore {
    let bytes: Vec<u8> = words.iter().flat_map(|&w| (w as u16).to_be_bytes()).collect();
    super::super::ivs::ItemVariationStore::parse(&bytes, 0).expect("a store")
}

// A null condition and one of format 0 are the true condition, and negated they fail.
#[test]
fn a_null_condition_and_one_of_format_0_hold() {
    let set = |condition: &[u8]| {
        let mut t = table(&[(16, 0)]);
        u16s(&mut t, &[1, 0, if condition.is_empty() { 0 } else { 6 }]);
        t.extend_from_slice(condition);
        FeatureVariations::at(&t, 0).find(&[], None)
    };
    assert_eq!(set(&[]), Some(0), "a null condition");
    assert_eq!(set(&[0, 0]), Some(0), "format 0");
    assert_eq!(set(&[0, 5, 0, 0, 0]), None, "the negation of a null condition");
    assert_eq!(set(&[0, 5, 0, 0, 5, 0, 0]), None, "the negation of format 0");
}

// A value condition's default and delta are summed in double precision: 32,767 and a delta a hair above
// -32,767, which single precision rounds to -32,767, sum to a hair above 0.
#[test]
fn a_value_condition_sums_its_delta_in_double_precision() {
    let regions = [0x4000, 0x4000, 0x4000, 0, 0, 0, 0, 0, 0, 0, 0x3000, 0x4000];
    let store = store(&[&[1, 0, 12, 1, 0, 40, 2, 2][..], &regions[..], &[1, 2, 2, 0, 1, -32767, 1][..]].concat());
    let mut t = table(&[(16, 0)]);
    u16s(&mut t, &[1, 0, 6, 2, 32767, 0, 0]);
    let fv = FeatureVariations::at(&t, 0);
    assert_eq!(fv.find(&[16384, 1], Some(&store)), Some(0));
    assert_eq!(fv.find(&[16384, 0], Some(&store)), None, "without the second axis the sum is 0");
}

// Coordinates that are all 0 are the default, where a value condition adds no delta, though a region whose axes
// all peak at 0 applies everywhere else.
#[test]
fn a_value_condition_adds_no_delta_at_the_default() {
    let store = store(&[1, 0, 12, 1, 0, 22, 1, 1, 0, 0, 0, 1, 1, 1, 0, 2]);
    let mut t = table(&[(16, 0)]);
    u16s(&mut t, &[1, 0, 6, 2, 0, 0, 0]);
    let fv = FeatureVariations::at(&t, 0);
    assert_eq!(fv.find(&[0], Some(&store)), None);
    assert_eq!(fv.find(&[1], Some(&store)), Some(0));
}

// A version 1.1 table with no substitutions whose lookup variations name features 3 and 5, by one FeatureLookups table:
// lookups 7 and 2 where axis 0 is from 0.5 to 1, lookup 4 always. `flags` 1 adds the feature's own lookups.
fn lookup_variations(flags: u16) -> Vec<u8> {
    let mut t = vec![0, 1, 0, 1];
    u32s(&mut t, &[0, 2]);
    u16s(&mut t, &[3, 0, 24, 5, 0, 24]);
    u16s(&mut t, &[1, 0, flags, 0, 2]);
    u32s(&mut t, &[36, 26, 0, 32]);
    u16s(&mut t, &[2, 7, 2, 1, 4, 1, 0, 0x2000, 0x4000]);
    t
}

#[test]
fn lookup_variations_turn_on_the_lookups_whose_conditions_hold() {
    let t = lookup_variations(1);
    let fv = FeatureVariations::at(&t, 0);
    assert_eq!(fv.lookup_conditions(&[0x3000], None), [60], "the condition, by its offset, once");
    assert_eq!(fv.lookup_conditions(&[0], None), [0u32; 0]);
    let varied = |feature, conditions: &[u32]| fv.varied_lookups(feature, conditions, [9, 2].into_iter(), &mut 999);
    assert_eq!(varied(3, &[60]), Some([2, 4, 7, 9].into()));
    assert_eq!(varied(5, &[]), Some([2, 4, 9].into()), "the null condition holds, and the feature's own stay");
    assert_eq!(varied(4, &[60]), None, "no record names feature 4");
    let t = lookup_variations(0);
    let fv = FeatureVariations::at(&t, 0);
    let own = fv.varied_lookups(3, &[], [9].into_iter(), &mut 999);
    assert_eq!(own, Some([4].into()), "without its flag, not its own");
    let mut t = lookup_variations(1);
    t[3] = 0;
    let fv = FeatureVariations::at(&t, 0);
    let conditions = fv.lookup_conditions(&[0x3000], None);
    assert_eq!((conditions.len(), fv.varied_lookups(3, &[], [9].into_iter(), &mut 999)), (0, None));
}

// 100,000 negations, each of the next: a table that passes its checks nests no deeper than 64, and a chain past
// that fails there rather than overflowing the stack.
#[test]
fn a_chain_of_conditions_stops_64_deep() {
    let mut t = table(&[(16, 0)]);
    u16s(&mut t, &[1, 0, 6]);
    (0..100_000).for_each(|_| t.extend_from_slice(&[0, 5, 0, 0, 5]));
    u16s(&mut t, &[1, 0, 0xC000, 0x4000]);
    assert_eq!(FeatureVariations::at(&t, 0).find(&[], None), None, "64 negations of a failure fail");
}

// Records naming sets of one condition each, every one of 255 references to a condition that fails, then a record
// with no set: 256 reads a set, past the reads one search may make at 1,000 sets, though not at 100.
#[test]
fn a_search_past_its_reads_finds_no_record() {
    let find = |sets: usize| {
        let at = 8 + 8 * (sets + 1);
        let mut t = table(&(0..sets).map(|i| ((at + 6 * i) as u32, 0)).chain([(0, 0)]).collect::<Vec<_>>());
        for i in 0..sets {
            u16s(&mut t, &[1]);
            u32s(&mut t, &[(6 * sets - 3 * i) as u32]);
        }
        // Format 4 of 255 references 1,279 bytes on, which lands inside another and reads as an unknown format.
        (0..sets + 427).for_each(|_| t.extend_from_slice(&[0, 4, 0xFF]));
        FeatureVariations::at(&t, 0).find(&[], None)
    };
    assert_eq!(find(100), Some(100));
    assert_eq!(find(1_000), None);
}

// FeatureLookups tables eight bytes apart in one run, each 400 records of conditions past the end, then one whose
// condition holds: found after 10 such tables, but after 400 the reads one search may make run out before it.
#[test]
fn lookup_tables_that_overlap_past_the_reads_give_the_conditions_found_before() {
    let conditions = |tables: usize| {
        const RECORDS: u32 = 400;
        let mut t = vec![0, 1, 0, 1];
        u32s(&mut t, &[0, tables as u32 + 1]);
        let run = 12 + 6 * (tables + 1);
        let last = run + 2 + 8 * (tables + RECORDS as usize + 1);
        (0..tables).for_each(|j| {
            u16s(&mut t, &[0]);
            u32s(&mut t, &[(run + 8 * j) as u32]);
        });
        u16s(&mut t, &[0]);
        u32s(&mut t, &[last as u32]);
        // Each table's flags and count are the unit before its records: a condition 16 MB on and a count of 400.
        u16s(&mut t, &[0]);
        (0..tables + RECORDS as usize + 1).for_each(|_| u32s(&mut t, &[1 << 24, RECORDS]));
        u16s(&mut t, &[1, 0, 0]);
        u32s(&mut t, &[1, 18, 0]);
        u16s(&mut t, &[0]);
        (FeatureVariations::at(&t, 0).lookup_conditions(&[], None), last as u32 + 18)
    };
    let (found, last) = conditions(10);
    assert_eq!(found, [last]);
    assert_eq!(conditions(400).0, [0u32; 0]);
}

// Records naming 1,024-lookup lists two bytes apart: 600 pass one walk's reads, so the feature keeps its own lookups,
// and 20 don't. At 50 one walk fits, but not a second sharing its budget, as a plan's features share one.
#[test]
fn lookup_lists_that_overlap_past_the_reads_leave_the_feature_its_own() {
    let table = |records: usize| {
        let mut t = vec![0, 1, 0, 1];
        u32s(&mut t, &[0, 1]);
        u16s(&mut t, &[3]);
        u32s(&mut t, &[18]);
        u16s(&mut t, &[1, 0, 0]);
        u32s(&mut t, &[records as u32]);
        (0..records).for_each(|i| u32s(&mut t, &[0, (10 + 8 * records + 2 * i) as u32]));
        u16s(&mut t, &vec![1_024; records + 1_024]);
        t
    };
    let varied = |t: &[u8], left: &mut usize| {
        FeatureVariations::at(t, 0).varied_lookups(3, &[], [9].into_iter(), left)
    };
    let alone = |records| {
        let t = table(records);
        varied(&t, &mut FeatureVariations::at(&t, 0).budget())
    };
    assert_eq!(alone(20), Some([1_024].into()));
    assert_eq!(alone(600), None);
    let t = table(50);
    let mut left = FeatureVariations::at(&t, 0).budget();
    assert_eq!(varied(&t, &mut left), Some([1_024].into()));
    assert_eq!(varied(&t, &mut left), None, "the second walk shares what the first left");
}
