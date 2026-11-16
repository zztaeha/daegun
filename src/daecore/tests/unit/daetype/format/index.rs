use super::*;
use alloc::vec;

impl SparseIndex {
    // The list builder `build` replaced, kept to check it against: entries sorted and unique.
    pub(crate) fn from_sorted(entries: &[(u32, u16)], absent: u16, budget: usize) -> Option<SparseIndex> {
        let first = entries.first()?.0;
        let last = entries.last()?.0;
        debug_assert!(entries.windows(2).all(|w| w[0].0 < w[1].0), "entries must be sorted, unique");

        let span = (last - first) as usize + 1;
        let direct_bytes = span.saturating_mul(2);

        let first_page = first >> PAGE_BITS;
        let last_page = last >> PAGE_BITS;
        let directory = (last_page - first_page) as usize + 1;
        let populated = {
            let mut n = 0usize;
            let mut seen = u32::MAX;
            for &(key, _) in entries {
                let page = key >> PAGE_BITS;
                if page != seen {
                    seen = page;
                    n += 1;
                }
            }
            n
        };
        let paged_bytes = directory
            .saturating_mul(4)
            .saturating_add(populated.saturating_mul(PAGE * 2));

        if direct_bytes.min(paged_bytes) > budget {
            return None;
        }

        // Direct up to 4 KB past paged: a small table read per glyph skips the page directory's extra load.
        let kind = if direct_bytes <= budget && direct_bytes <= paged_bytes.saturating_add(4096) {
            let mut values = alloc::vec![absent; span];
            for &(key, value) in entries {
                values[(key - first) as usize] = value;
            }
            Kind::Direct { base: first, values }
        } else {
            let mut page_of = alloc::vec![NO_PAGE; directory];
            let mut values: Vec<u16> = Vec::with_capacity(populated * PAGE);
            for &(key, value) in entries {
                let page = ((key >> PAGE_BITS) - first_page) as usize;
                if page_of[page] == NO_PAGE {
                    page_of[page] = values.len() as u32;
                    values.resize(values.len() + PAGE, absent);
                }
                values[page_of[page] as usize + (key as usize & (PAGE - 1))] = value;
            }
            Kind::Paged { base_page: first_page, page_of, values }
        };

        Some(SparseIndex { absent, kind })
    }
}

fn of(entries: &[(u32, u16)], absent: u16, budget: usize) -> Option<SparseIndex> {
    SparseIndex::build(&|e| { entries.iter().for_each(|&(k, v)| e.put(k, v.into())); Some(()) }, absent, budget)
}

// Two populated pages, 1,068 bytes paged, whatever the gap between them: 5,164 bytes direct at a last key of
// 2,581, exactly 4 KB more.
fn ends(last: u32) -> Vec<(u32, u16)> {
    vec![(0, 7), (5, 3), (last - 11, 4), (last, 9)]
}

fn is_direct(index: &SparseIndex) -> bool {
    matches!(index.kind, Kind::Direct { .. })
}

#[test]
fn a_table_within_4_kb_of_its_paged_size_goes_direct() {
    assert!(is_direct(&of(&ends(2581), 0, usize::MAX).unwrap()));
    assert!(!is_direct(&of(&ends(2582), 0, usize::MAX).unwrap()));
}

#[test]
fn a_budget_short_of_the_direct_size_pages_instead() {
    assert!(!is_direct(&of(&ends(2581), 0, 5163).unwrap()));
    assert!(of(&ends(2581), 0, 1067).is_none());
}

#[test]
fn direct_and_paged_answer_alike() {
    let direct = of(&ends(2581), 0, usize::MAX).unwrap();
    let paged = of(&ends(2581), 0, 5163).unwrap();
    assert!(is_direct(&direct) && !is_direct(&paged));
    for key in (0..3000).chain([65_535, u32::MAX]) {
        assert_eq!(direct.lookup(key), paged.lookup(key), "key {key}");
    }
}

// Keys in any order, some given twice, build what the sorted list of each key's first value builds, at every
// budget either choice turns on; a key past Unicode, or none, builds nothing.
#[test]
fn entries_in_any_order_build_the_sorted_lists_index() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % u64::from(n)) as u32
    };
    for round in 0..400 {
        let spread = [300, 5_000, 70_000, 0x11_0000][round % 4];
        let given: Vec<(u32, u16)> = (0..1 + next(600)).map(|_| (next(spread), 1 + next(900) as u16)).collect();
        let mut sorted: Vec<(u32, u16)> = Vec::new();
        for &(k, v) in &given {
            if !sorted.iter().any(|e| e.0 == k) {
                sorted.push((k, v));
            }
        }
        sorted.sort_unstable();
        let full = SparseIndex::from_sorted(&sorted, 0, usize::MAX).unwrap();
        for budget in [usize::MAX, full.bytes(), full.bytes() - 1, 1_100, 0] {
            assert_eq!(of(&given, 0, budget), SparseIndex::from_sorted(&sorted, 0, budget), "round {round}");
        }
    }
    assert!(of(&[(3, 1), (0x11_0000, 2)], 0, usize::MAX).is_none());
    assert!(of(&[], 0, usize::MAX).is_none());
    assert!(SparseIndex::build(&|e| { e.put(1, 1); None }, 0, usize::MAX).is_none());
}

// Runs, rising or level, within a page or over many, build what their keys build one by one; a run past Unicode,
// or one whose values rise past 16 bits, builds nothing.
#[test]
fn runs_build_what_their_keys_build() {
    let runs: [(u32, u32, u32, u32); 5] = [
        (3, 3, 9, 0), (10, 600, 64_945, 1), (700, 1_300, 4, 0), (70_000, 70_255, 1, 1),
        (0x10_FF00, 0x10_FFFF, 2, 0),
    ];
    let value = |(a, _, v, k): (u32, u32, u32, u32), key: u32| (v + k * (key - a)) as u16;
    let keys: Vec<(u32, u16)> =
        runs.iter().flat_map(|&r| (r.0..=r.1).map(move |key| (key, value(r, key)))).collect();
    let each = |e: &mut Entries| {
        runs.iter().for_each(|&(a, b, v, k)| e.put_run(a, b, v, k));
        Some(())
    };
    for budget in [usize::MAX, 20_000, 0] {
        assert_eq!(SparseIndex::build(&each, 0, budget), of(&keys, 0, budget), "budget {budget}");
    }
    assert!(SparseIndex::build(&|e| { e.put_run(0x10_FF00, u32::MAX, 1, 0); Some(()) }, 0, usize::MAX).is_none());
    assert!(SparseIndex::build(&|e| { e.put_run(0x11_0005, 0x20_0000, 1, 0); Some(()) }, 0, usize::MAX).is_none());
    assert!(SparseIndex::build(&|e| { e.put_run(10, 600, 64_946, 1); Some(()) }, 0, usize::MAX).is_none());
    assert!(SparseIndex::build(&|e| { e.put(10, 0x1_0000); Some(()) }, 0, usize::MAX).is_none());
}
