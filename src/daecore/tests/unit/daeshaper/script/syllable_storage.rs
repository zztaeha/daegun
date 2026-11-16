use alloc::collections::BTreeSet;
use super::DeadEnds;

// Inserts and front drops, inline, compacted and spilled, against a set.
#[test]
fn dead_ends_match_a_set() {
    let mut seed = 7u64;
    let mut next = |m: u64| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) % m
    };
    let (mut dead, mut model) = (DeadEnds::<8>::new(), BTreeSet::new());
    let mut floor = 0;
    for step in 0..20_000 {
        if next(8) == 0 {
            let reach = if next(50) == 0 { 400 } else { 12 };
            floor += next(reach);
            dead.drop_before(floor);
            model.retain(|&k| k >= floor);
        } else {
            let reach = if next(30) == 0 { 300 } else { 40 };
            let key = floor + next(reach);
            dead.insert(key);
            model.insert(key);
        }
        assert_eq!(dead.keys(), model.iter().copied().collect::<alloc::vec::Vec<_>>(), "step {step}");
        let probe = floor + next(60);
        assert_eq!(dead.contains(probe), model.contains(&probe), "step {step}, {probe}");
    }
}
