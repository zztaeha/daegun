use super::*;

// One pixel a unit.
fn scale() -> Scale {
    Scale::new(64 << 16).expect("a scale")
}

fn stem(bottom: i64, top: i64, bit: u32) -> Stem {
    Stem::declared(bottom, top - bottom, bit)
}

fn zones() -> Zones {
    Zones::new(&super::super::dict::Private::default(), scale(), None)
}

// Anchored at the origin alone, as a glyph no zone aligns is, and in the order hint sets take them.
fn anchored(stems: &[Stem]) -> (Ladder, Order) {
    let mut order = Order::default();
    order.sort(stems);
    let mut anchors = Ladder::new(Vec::new(), scale());
    anchor(stems, &mut order, None, &mut anchors);
    (anchors, order)
}

fn mask(bits: &[u64]) -> Mask {
    Mask(bits.to_vec())
}

// Each stem in `stems` placed as one hint set.
fn placed(stems: &mut [Stem]) {
    let (anchors, mut order) = anchored(stems);
    let mut all = Mask::default();
    all.fill(stems.len());
    place(stems, &all, &zones(), &anchors, &mut Ladder::new(Vec::new(), scale()), &mut order);
}

// Widths -21 and -20 mark an edge hint's bottom and top edge (TN 5177, 4.3); another negative width swaps the edges.
#[test]
fn edge_hints_read_from_their_widths() {
    let (bottom, top) = (Stem::declared(121 << 16, -21 << 16, 0), Stem::declared(500 << 16, -20 << 16, 1));
    assert_eq!((bottom.bottom, bottom.top, bottom.edges), (100 << 16, 100 << 16, Edges::Bottom));
    assert_eq!((top.bottom, top.top, top.edges), (500 << 16, 500 << 16, Edges::Top));
    let swapped = Stem::declared(50 << 16, -10 << 16, 2);
    assert_eq!((swapped.bottom, swapped.top, swapped.edges), (40 << 16, 50 << 16, Edges::Both));
}

// A stem at 10.5–20.5 px is a tie and goes up; one at 40.25–50.25 goes down, the nearer way.
#[test]
fn a_free_stem_moves_the_nearer_way_a_tie_up() {
    let mut stems = [stem(0xA_8000, 0x14_8000, 0), stem(0x28_4000, 0x32_4000, 1)];
    placed(&mut stems);
    assert_eq!(stems.map(|s| s.place), [Some((11 << 16, 21 << 16)), Some((40 << 16, 50 << 16))]);
}

// Moving up would leave the counter above under half a pixel, so the stem goes down instead; with no room either way
// it stays.
#[test]
fn a_move_that_would_close_a_counter_goes_the_other_way() {
    let mut stems = [stem(0xA_8000, 0x14_A000, 0), stem(0x15_0000, 0x18_0000, 1)];
    placed(&mut stems);
    assert_eq!(stems[0].place, Some((0xA_0000, 0x14_2000)));
    let mut stems = [stem(0, 0x2_0000, 0), stem(0x2_6000, 0x4_6000, 1), stem(0x4_C000, 0x6_0000, 2)];
    placed(&mut stems);
    assert_eq!(stems[1].place, Some((0x2_6000, 0x4_6000)));
}

// Touching stems overlap: the later is left out, never placed, so the one taken first keeps its place, unless it's
// free and the later one is placed.
#[test]
fn a_stem_touching_one_taken_is_left_out() {
    let mut stems = [stem(10 << 16, 20 << 16, 0), stem(20 << 16, 30 << 16, 1), stem(15 << 16, 15 << 16, 2)];
    placed(&mut stems);
    assert_eq!(stems.map(|s| s.place.is_some()), [true, false, false]);
    let mut stems = [stem(10 << 16, 20 << 16, 0), stem(15 << 16, 30 << 16, 1)];
    stems[1].place = Some((15 << 16, 30 << 16));
    placed(&mut stems);
    assert_eq!(stems[0].place, None);
}

// A stem placed in one hint set keeps that place in the next, whatever its neighbors there; a stem that would then
// land below it is left out of that set's ladder.
#[test]
fn a_stem_keeps_its_place_in_later_hint_sets() {
    let mut stems = [stem(0xA_8000, 0x14_8000, 0), stem(0x14_C000, 0x18_0000, 1)];
    let (anchors, mut order) = anchored(&stems);
    let mut ladder = Ladder::new(Vec::new(), scale());
    place(&mut stems, &mask(&[0b01]), &zones(), &anchors, &mut ladder, &mut order);
    assert_eq!(stems[0].place, Some((11 << 16, 21 << 16)));
    place(&mut stems, &mask(&[0b11]), &zones(), &anchors, &mut ladder, &mut order);
    assert_eq!(stems[0].place, Some((11 << 16, 21 << 16)));
    // Mapped by the first stem's top alone, 22 units lands 1.5 px above its 21 px.
    assert_eq!(ladder.map(22 << 16), (22 << 16) + (1 << 15), "the second stem folded below the first");
}

// The em box's edges join a ladder where they lie beyond its stems, and a stemless glyph's anchors are the edges
// alone, the origin between them not held: -10 → -12 px under a stem at 20 → 21 px puts 0 at -1 px.
#[test]
fn the_em_box_edges_bound_the_ladder_beyond_its_stems() {
    let mut stems = [stem(20 << 16, 30 << 16, 0)];
    stems[0].place = Some((21 << 16, 31 << 16));
    let mut ladder = Ladder::new(Vec::new(), scale());
    build(&stems, &[0], Some([(-10 << 16, -12 << 16), (90 << 16, 151 << 16)]), &mut ladder);
    ladder.finish();
    assert_eq!((ladder.map(0), ladder.map(60 << 16)), (-1 << 16, 91 << 16));
    build(&stems, &[0], Some([(25 << 16, 27 << 16), (26 << 16, 28 << 16)]), &mut ladder);
    ladder.finish();
    assert_eq!(ladder.map(25 << 16), 26 << 16, "an edge between the stems' rungs joined");
    let mut anchors = Ladder::new(Vec::new(), scale());
    anchor(&[], &mut Order::default(), Some([(-10 << 16, -12 << 16), (90 << 16, 138 << 16)]), &mut anchors);
    assert_eq!(anchors.map(0), 3 << 16);
}

// A mask's bits run high bit first from its first byte; bits past the stems declared are off, however many there are.
#[test]
fn a_mask_reads_high_bit_first() {
    let mut m = Mask::default();
    m.read(&[0b1010_0000, 0xFF], 10);
    assert!(m.holds(0) && !m.holds(1) && m.holds(2) && m.holds(8) && m.holds(9) && !m.holds(10));
    m.read(&[0xFF; 26], 200);
    assert!(m.holds(199) && !m.holds(200));
    m.fill(0);
    assert_eq!(m, mask(&[]));
    m.fill(200);
    assert_eq!(m, mask(&[u64::MAX, u64::MAX, u64::MAX, 0xFF]));
}
