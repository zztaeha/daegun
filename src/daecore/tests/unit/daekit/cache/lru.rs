use super::ByteLru;

fn by_value(_: &u32, cost: &u32) -> usize {
    *cost as usize
}

fn keys(c: &mut ByteLru<u32, u32>, all: &[u32]) -> alloc::vec::Vec<u32> {
    all.iter().copied().filter(|k| c.get(k).is_some()).collect()
}

#[test]
fn the_least_recently_used_goes_first() {
    let mut c = ByteLru::new(3, by_value);
    for k in [1, 2, 3] {
        c.insert(k, 1);
    }
    assert_eq!(c.get(&1), Some(&1));
    c.insert(4, 1);
    assert_eq!(c.bytes(), 3);
    // Checking membership stamps every key read, so it comes after the eviction it observes.
    assert_eq!(keys(&mut c, &[1, 2, 3, 4]), [1, 3, 4]);
}

#[test]
fn a_key_used_again_and_again_is_never_the_one_evicted() {
    let mut c = ByteLru::new(2, by_value);
    c.insert(0, 1);
    for k in 1..100 {
        assert!(c.get(&0).is_some(), "key 0 was evicted before inserting {k}");
        c.insert(k, 1);
    }
    assert_eq!((c.len(), c.bytes()), (2, 2));
}

#[test]
fn replacing_a_key_refunds_its_old_cost() {
    let mut c = ByteLru::new(10, by_value);
    c.insert(1, 6);
    c.insert(1, 3);
    assert_eq!((c.len(), c.bytes(), c.get(&1)), (1, 3, Some(&3)));
    c.insert(2, 7);
    assert_eq!((c.len(), c.bytes()), (2, 10));
}

#[test]
fn a_replaced_key_counts_as_just_used() {
    let mut c = ByteLru::new(3, by_value);
    for k in [1, 2, 3] {
        c.insert(k, 1);
    }
    c.insert(1, 1);
    c.insert(4, 1);
    assert_eq!(keys(&mut c, &[1, 2, 3, 4]), [1, 3, 4]);
}

#[test]
fn tightening_keeps_what_still_fits() {
    let mut c = ByteLru::new(10, by_value);
    for k in 0..5 {
        c.insert(k, 2);
    }
    c.get(&0);
    c.set_budget(5);
    assert_eq!(c.bytes(), 4);
    assert_eq!(keys(&mut c, &[0, 1, 2, 3, 4]), [0, 4]);
}

#[test]
fn a_value_over_budget_is_refused_unless_the_newest_is_kept() {
    let mut plain = ByteLru::new(4, by_value);
    plain.insert(1, 2);
    assert_eq!(plain.insert(2, 5), None);
    assert_eq!((plain.len(), plain.bytes()), (1, 2));

    let mut newest = ByteLru::keeping_newest(4, by_value);
    newest.insert(1, 2);
    assert_eq!(newest.insert(2, 5), Some(1));
    assert_eq!((newest.len(), newest.bytes(), newest.get(&2)), (1, 5, Some(&5)));
}

// A value that grows is charged its growth, the others making room for it; each leaving refunds what it costs then,
// and is told it left.
#[test]
fn a_value_charged_its_growth_is_refunded_its_cost_as_it_leaves() {
    use core::cell::Cell;
    fn by_cell(_: &u32, cost: &alloc::rc::Rc<Cell<usize>>) -> usize {
        cost.get()
    }
    let mut c = ByteLru::new(10, by_cell).telling(|cost| cost.set(0));
    let (first, grows) = (alloc::rc::Rc::new(Cell::new(4)), alloc::rc::Rc::new(Cell::new(2)));
    c.insert(1, alloc::rc::Rc::clone(&first));
    c.insert(2, alloc::rc::Rc::clone(&grows));
    grows.set(7);
    assert_eq!(c.bytes(), 6, "growth was charged before it was told");
    assert_eq!(c.adjust(5), 1);
    assert_eq!((c.len(), c.bytes(), first.get()), (1, 7, 0));
    c.clear();
    assert_eq!((c.bytes(), grows.get()), (0, 0));
}

#[test]
fn past_the_budget_the_most_recently_used_is_the_one_kept() {
    let mut c = ByteLru::keeping_newest(10, by_value);
    for k in [1, 2, 3] {
        c.insert(k, k);
    }
    c.get(&2);
    c.set_budget(0);
    assert_eq!((c.len(), c.bytes(), c.get(&2)), (1, 2, Some(&2)));
}

type List = crate::daecore::daekit::base::sync::Mutable<ByteLru<u32, u32>>;

fn pooled(pool: &super::Shared<super::Pool>, list: ByteLru<u32, u32>) -> super::Shared<List> {
    use crate::daecore::daekit::base::sync::{mutable, write};
    let list = super::Shared::new(mutable(list));
    let me = super::member(&list);
    super::rejoin(core::slice::from_mut(&mut write(&list)), Some(pool), &me, |_| 0);
    list
}

// As a cache keeps a value: the pool it went over settled once the list's lock is let go.
fn put(list: &List, key: u32, cost: u32) {
    let over = {
        let mut list = crate::daecore::daekit::base::sync::write(list);
        list.insert(key, cost);
        list.over()
    };
    super::settle(over);
}

fn len(list: &List) -> usize {
    crate::daecore::daekit::base::sync::read(list).len()
}

// Past the pool's budget the entry queued longest goes, whichever list holds it; one read since goes round again.
#[test]
fn a_pool_evicts_the_oldest_across_its_lists() {
    let pool = super::Shared::new(super::Pool::new(4));
    let (a, b) = (pooled(&pool, ByteLru::new(100, by_value)), pooled(&pool, ByteLru::new(100, by_value)));
    for (list, key) in [(&a, 1), (&b, 2), (&a, 3), (&b, 4), (&b, 5)] {
        put(list, key, 1);
    }
    assert_eq!((len(&a), len(&b), pool.stats()), (1, 3, (4, 4)), "the oldest, a's first, didn't go");
    assert!(crate::daecore::daekit::base::sync::read(&b).get(&2).is_some());
    put(&a, 6, 1);
    assert_eq!(pool.stats(), (4, 4));
    let held = |list: &List, all: &[u32]| keys(&mut crate::daecore::daekit::base::sync::write(list), all);
    assert_eq!((held(&a, &[1, 3, 6]), held(&b, &[2, 4, 5])), (alloc::vec![6], alloc::vec![2, 4, 5]));
}

// A list keeps to its own budget in a pool, and a lower pool budget applies at once.
#[test]
fn a_pooled_list_keeps_to_both_budgets() {
    let pool = super::Shared::new(super::Pool::new(100));
    let a = pooled(&pool, ByteLru::new(2, by_value));
    let b = pooled(&pool, ByteLru::new(100, by_value));
    for key in 0..3 {
        put(&a, key, 1);
        put(&b, key, 1);
    }
    assert_eq!((len(&a), len(&b), pool.stats()), (2, 3, (5, 5)));
    pool.set_budget(2);
    assert_eq!((len(&a), len(&b), pool.stats()), (1, 1, (2, 2)), "each list's newest isn't what's left");
}

// A list leaving keeps what it holds and gives the pool its bytes back, as one dropped does; one joining brings them.
#[test]
fn a_list_leaving_or_dropped_gives_the_pool_its_bytes_back() {
    use crate::daecore::daekit::base::sync::write;
    let pool = super::Shared::new(super::Pool::new(100));
    let a = pooled(&pool, ByteLru::new(100, by_value));
    put(&a, 1, 3);
    put(&a, 2, 4);
    assert_eq!(pool.stats(), (2, 7));
    let me = super::member(&a);
    super::rejoin(core::slice::from_mut(&mut write(&a)), None, &me, |_| 0);
    assert_eq!((pool.stats(), len(&a)), ((0, 0), 2));
    put(&a, 3, 1);
    assert_eq!(pool.stats(), (0, 0), "a list out of the pool drew from it");
    super::rejoin(core::slice::from_mut(&mut write(&a)), Some(&pool), &me, |_| 0);
    assert_eq!(pool.stats(), (3, 8));
    drop(a);
    assert_eq!(pool.stats(), (0, 0));
    assert!(crate::daecore::daekit::base::sync::read(&pool.heads).is_empty(), "a dropped list was left listed");
}

// A list that keeps its newest past any budget keeps it past the pool's too.
#[test]
fn a_pool_leaves_a_list_its_newest_where_the_list_keeps_it() {
    let pool = super::Shared::new(super::Pool::new(0));
    let a = pooled(&pool, ByteLru::keeping_newest(100, by_value));
    put(&a, 1, 5);
    put(&a, 2, 5);
    assert_eq!((len(&a), pool.stats()), (1, (1, 5)));
    assert!(crate::daecore::daekit::base::sync::read(&a).get(&2).is_some());
}
