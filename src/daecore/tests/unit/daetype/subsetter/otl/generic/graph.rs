use super::*;
use crate::daecore::daetype::decoder::read_u16_be;

fn link(at: usize, width: usize, to: usize) -> Link {
    Link { at, width, to }
}

// Objects with the same bytes naming the same objects are one; a different byte or target is another.
#[test]
fn an_object_held_already_is_that_one() {
    let mut graph = Graph::default();
    let leaf = graph.add(alloc::vec![1, 2], Vec::new());
    assert_eq!(graph.add(alloc::vec![1, 2], Vec::new()), leaf);
    let other = graph.add(alloc::vec![1, 3], Vec::new());
    let parent = graph.add(alloc::vec![0, 0], alloc::vec![link(0, 2, leaf)]);
    assert_eq!(graph.add(alloc::vec![0, 0], alloc::vec![link(0, 2, leaf)]), parent);
    assert_ne!(graph.add(alloc::vec![0, 0], alloc::vec![link(0, 2, other)]), parent);
    assert_eq!(graph.held(), 8);
}

// A child two tables name follows both, once, and each offset counts from its own table; one named once follows its
// parent. A table's children come smallest first.
#[test]
fn a_shared_child_follows_every_table_naming_it() {
    let mut graph = Graph::default();
    let shared = graph.add(alloc::vec![0xAA], Vec::new());
    let own = graph.add(alloc::vec![0xBB], Vec::new());
    let first = graph.add(alloc::vec![0, 0, 0, 0], alloc::vec![link(0, 2, shared), link(2, 2, own)]);
    let second = graph.add(alloc::vec![0, 0], alloc::vec![link(0, 2, shared)]);
    let root = graph.add(alloc::vec![0, 0, 0, 0], alloc::vec![link(0, 2, first), link(2, 2, second)]);
    // root 0..4, second 4..6, first 6..10, shared 10, own 11
    assert_eq!(graph.pack(root), Some(alloc::vec![0, 6, 0, 4, 0, 6, 0, 4, 0, 5, 0xAA, 0xBB]));
}

// A 64,000-byte table naming a 1,000-byte child, then 100 of 6 bytes: in its offsets' order the last small ones
// would start past 64 KB, as a PairPos's Device tables behind its class definitions would. Smallest first, all fit.
#[test]
fn a_tables_small_children_come_before_its_large_ones() {
    let mut graph = Graph::default();
    let large = graph.add(alloc::vec![1; 1_000], Vec::new());
    let mut links = alloc::vec![link(0, 2, large)];
    links.extend((0..100u8).map(|i| link(2 + 2 * usize::from(i), 2, graph.add(alloc::vec![i; 6], Vec::new()))));
    let table = graph.add(alloc::vec![0; 64_000], links);
    let out = graph.pack(table).expect("every offset reaches");
    assert_eq!([0, 2].map(|at| read_u16_be(&out, at)), [Some(64_600), Some(64_000)]);
}

// A 16-bit offset past 65,535 bytes leaves the graph unpacked; at 32 bits it reaches, and what only 32-bit offsets
// name waits for everything else.
#[test]
fn an_offset_that_cant_reach_leaves_the_graph_unpacked() {
    let mut graph = Graph::default();
    let far = graph.add(alloc::vec![0xFF], Vec::new());
    let big = graph.add(alloc::vec![0; 0x1_0000], alloc::vec![link(0, 2, far)]);
    assert_eq!(graph.pack(big), None);
    let mut graph = Graph::default();
    let far = graph.add(alloc::vec![0xFF], Vec::new());
    let near = graph.add(alloc::vec![0xEE], Vec::new());
    let root = graph.add(alloc::vec![0; 6], alloc::vec![link(0, 4, far), link(4, 2, near)]);
    assert_eq!(graph.pack(root), Some(alloc::vec![0, 0, 0, 7, 0, 6, 0xEE, 0xFF]));
}

// A's offset to S, which B's child D names too, can't reach past the 70,000-byte B: a budget with room for A's copy
// packs, a byte less refuses. 1,000 tables that would each copy a 30,000-byte object stop at a budget of 1 MB.
#[test]
fn copies_a_pack_makes_stay_within_the_budget() {
    let shared = |budget: usize| {
        let mut graph = Graph::within(budget);
        let s = graph.add(alloc::vec![0xAA; 10], Vec::new());
        let d = graph.add(alloc::vec![0, 0], alloc::vec![link(0, 2, s)]);
        let b = graph.add(alloc::vec![0xBB; 70_000], alloc::vec![link(0, 4, d)]);
        let a = graph.add(alloc::vec![0, 1], alloc::vec![link(0, 2, s)]);
        let root = graph.add(alloc::vec![0; 8], alloc::vec![link(0, 4, a), link(4, 4, b)]);
        (graph.held(), graph.pack(root))
    };
    let (held, _) = shared(usize::MAX);
    assert!(shared(held + 10).1.is_some(), "the copy was refused");
    assert_eq!(shared(held + 9).1, None);

    let (mut graph, root) = copying(1 << 20);
    assert_eq!(graph.pack(root), None);
    let held: usize = graph.objects.iter().map(|o| o.bytes.len()).sum();
    assert!(held <= 1 << 20, "the copies came to {held} bytes");
}

// 1,000 tables, within `budget`, that would each copy a 30,000-byte object out of reach.
fn copying(budget: usize) -> (Graph, usize) {
    let mut graph = Graph::within(budget);
    let t = graph.add(alloc::vec![0xAA; 30_000], Vec::new());
    let x = graph.add(alloc::vec![0xBB; 70_000], Vec::new());
    let mut tables = alloc::vec![graph.add(alloc::vec![0, 0, 0xFF, 0xFF], alloc::vec![link(0, 2, x)])];
    for i in 1..1_000u16 {
        tables.push(graph.add([0, i].map(u16::to_be_bytes).concat(), alloc::vec![link(0, 2, t)]));
    }
    tables.push(graph.add(alloc::vec![0, 0, 0, 0, 0xEE, 0xEE], alloc::vec![link(0, 2, t), link(2, 2, x)]));
    let links = tables.iter().enumerate().map(|(i, &table)| link(4 * i, 4, table)).collect();
    let root = graph.add(alloc::vec![0; 4 * tables.len()], links);
    (graph, root)
}

// Four packs of one graph, as a table's plans try them, share its budget: the copies one leaves count against the
// next, rather than each having the budget's room afresh.
#[test]
fn copies_every_pack_of_a_graph_makes_share_its_budget() {
    let (mut graph, root) = copying(1 << 20);
    (0..4).for_each(|_| assert_eq!(graph.pack(root), None));
    let held: usize = graph.objects.iter().map(|o| o.bytes.len()).sum();
    assert!(held <= 1 << 20, "the copies came to {held} bytes");
}
