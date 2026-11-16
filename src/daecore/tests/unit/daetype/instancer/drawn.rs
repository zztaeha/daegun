use super::*;

// gvar's glyph data ranges read as stored, each between its offset and the next; one running backward or past the
// table's 68 bytes is absent, the rest kept, and a glyph past gvar's count has none.
#[test]
fn a_gvar_range_running_backward_or_past_the_table_is_absent() {
    let gvar = |offsets: &[u16]| {
        let glyphs = offsets.len() as u16 - 1;
        let data_at = 20 + 2 * u32::from(glyphs + 1);
        let mut t = [&[0u8, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 20][..], &glyphs.to_be_bytes(), &[0, 0]].concat();
        t.extend(data_at.to_be_bytes());
        offsets.iter().for_each(|o| t.extend(o.to_be_bytes()));
        t.extend([0u8; 40]);
        let map = Tables::from([("gvar", TableBytes::from_vec(t))]);
        let gvar = Gvar::load(&map).expect("reads").expect("a gvar");
        (0..offsets.len()).map(|gid| gvar.range(gid)).collect::<Vec<_>>()
    };
    let at = |a: usize, b: usize| Some((a, b));
    assert_eq!(gvar(&[0, 2, 6, 6]), [at(28, 32), at(32, 40), at(40, 40), None]);
    assert_eq!(gvar(&[0, 6, 2, 8]), [at(28, 40), None, at(32, 44), None]);
    assert_eq!(gvar(&[0, 2, 30, 4]), [at(28, 32), None, None, None]);
    assert_eq!(gvar(&[0, 2, 20, 31]), [at(28, 32), at(32, 68), None, None]);
}

// A room is kept for the next glyph while it holds 64 KB or less: past that, by its lists too, it's dropped.
#[cfg(feature = "std")]
#[test]
fn a_room_past_its_bound_is_not_kept() {
    let mut room = DeltaRoom::default();
    assert!(room.kept());
    room.private = Vec::with_capacity(ROOM_KEPT_BYTES / 2);
    assert!(room.kept());
    room.shared = Vec::with_capacity(1);
    assert!(!room.kept());
}

// Counts as far as 16 bits go: the limits refuse them before their sum, which a 32-bit usize can't hold.
#[test]
fn fvar_of_far_counts_is_refused() {
    let mut fvar = [0u8; 20];
    fvar[1] = 1;
    fvar[8..16].fill(0xFF);
    assert_eq!(fvar_axes(&fvar), None);
}

// gvar's inferred deltas on one axis: listed points at one coordinate give their delta if they agree and none if
// not; a point beyond both takes the nearer one's; between, the line through both, rounded once.
#[test]
fn an_inferred_delta_follows_the_listed_points_around_it() {
    let d = |v: i64| v << 16;
    assert_eq!(infer(5, (10, d(3)), (10, d(3))), d(3));
    assert_eq!(infer(5, (10, d(3)), (10, d(4))), 0);
    assert_eq!(infer(-7, (10, d(3)), (20, d(9))), d(3));
    assert_eq!(infer(25, (20, d(9)), (10, d(3))), d(9));
    assert_eq!(infer(15, (10, d(3)), (20, d(9))), d(6));
    assert_eq!(infer(11, (20, 0), (10, 1)), 1);
    assert_eq!(infer(13, (10, 0), (16, 1)), 1);
    assert_eq!(infer(13, (10, 0), (16, -1)), -1);
}

// A contour read as a ring: the points after its last listed one take from it and the first, y left where its listed
// points meet but disagree; one listed point moves its contour, none leaves it; the scalar applies after inference.
#[test]
fn a_tuple_listing_some_points_infers_the_rest_by_contour() {
    let mut inference = Inference::default();
    let (x, y) = ([0, 100, 200, 300, 0, 10, 20, 50, 60], [0, 0, 0, 0, 7, 7, 7, 0, 0]);
    inference.lay_out((&x, &y), 9);
    let mut deltas = [(0, 0); 9];
    let points = [0u16, 2, 5];
    let (xs, ys) = ([0, 20 << 16, 4 << 16], [10 << 16, 30 << 16, -(2 << 16)]);
    inference.apply(&mut deltas, &points, (&xs, &ys), 1 << 15, &[3, 6, 8]);
    let half = |v: i64| v << 15;
    assert_eq!(deltas[..4], [(0, half(10)), (half(10), 0), (half(20), half(30)), (half(20), 0)]);
    assert_eq!(deltas[4..7], [(half(4), -half(2)); 3]);
    assert_eq!(deltas[7..], [(0, 0); 2]);
}

// 16.16 products and quotients round halves away from zero and never wrap: past i64 they hold at its ends; a zero
// divisor gives 16.16's largest value with the dividend's sign, and 0 / 0 gives 0.
#[test]
fn fixed_products_and_quotients_round_away_from_zero_and_never_wrap() {
    assert_eq!([mul_fix(3, ONE / 2), mul_fix(-3, ONE / 2), mul_fix(1, ONE / 2), mul_fix(-1, ONE / 2)], [2, -2, 1, -1]);
    assert_eq!(mul_fix(i64::MAX, 4 * ONE), i64::MAX);
    assert_eq!(mul_fix(i64::MIN, 4 * ONE), i64::MIN);
    assert_eq!([div_fix(1, 2 * ONE), div_fix(-1, 2 * ONE), div_fix(3 * ONE, 2 * ONE)], [1, -1, 3 * ONE / 2]);
    assert_eq!(div_fix(i64::MAX, 1), i64::MAX);
    assert_eq!([div_fix(5, 0), div_fix(-5, 0), div_fix(0, 0)], [0x7FFF_FFFF, -0x7FFF_FFFF, 0]);
}

// A delta past 16 bits is cut to them alike whole and in 26.6, so a hinted point starts where the unhinted outline
// puts it: 40,000.25 units give -25,536.25, and 2^25 + 3 units give 3, where an i32 cut would flip the sign.
#[test]
fn a_delta_past_16_bits_is_cut_alike_whole_and_in_26_6() {
    let fine = |units: i64, quarters: i64| fixed_to_f26dot6(units * ONE + quarters * ONE / 4);
    let cases =
        [(100, 2, 100 * 64 + 32), (40_000, 1, -25_536 * 64 + 16), (-40_000, 0, 25_536 * 64), ((1 << 25) + 3, 0, 192)];
    for (units, quarters, want) in cases {
        assert_eq!(fine(units, quarters), want, "{units} and {quarters} quarters");
        assert_eq!(fine(units, 0), i32::from(fixed_to_int(units * ONE)) * 64, "{units}");
    }
}

// gvar with no axes of its own beside an fvar of one, which gvar's count overrides, and a tuple naming shared tuple
// 0: without axes no shared tuple counts, so the glyph's data is malformed rather than a scalar looked up past none.
#[test]
fn a_gvar_without_axes_counts_no_shared_tuple() {
    let mut fvar = alloc::vec![0u8, 1, 0, 1, 0, 16, 0, 2, 0, 1, 0, 20, 0, 0, 0, 8];
    fvar.extend(b"wght");
    for v in [100i32, 400, 900] {
        fvar.extend((v << 16).to_be_bytes());
    }
    fvar.extend([0, 0, 1, 0]);
    let mut gvar = alloc::vec![0u8, 1, 0, 0, 0, 0, 0, 1, 0, 0, 0, 24, 0, 1, 0, 0, 0, 0, 0, 24];
    gvar.extend([0, 0, 0, 5]);
    gvar.extend([0, 1, 0, 8, 0, 2, 0, 0, 0x83, 0x83]);
    let map = Tables::from([("fvar", TableBytes::from_vec(fvar)), ("gvar", TableBytes::from_vec(gvar))]);
    let loaded = Gvar::load(&map).map(|g| g.map(Shared::new));
    assert!(matches!(loaded, Ok(Some(_))), "gvar loads");
    let key = [(crate::daecore::Tag::new(b"wght"), 700.0)];
    let location = DrawnLocation::new(&map, &key, loaded, &DrawnAdvances::load(&map)).expect("a drawn location");
    let mut room = DeltaRoom::default();
    assert_eq!(location.source().deltas(0, 4, (&[], &[]), &[], &mut room, &mut 1_000), Ok(None));
}

// gvar on one axis whose one shared tuple peaks at 1.0, and glyph 0's data: `tuples` tuples over shared point numbers
// naming point 0, each a zero delta. At 1.0 each costs its unit and axis, its 2 bytes and twice the glyph's points.
pub(crate) fn gvar_of_tuples(glyphs: usize, tuples: u16) -> Vec<u8> {
    let mut data = [(0x8000 | tuples).to_be_bytes(), (4 + 4 * tuples).to_be_bytes()].concat();
    (0..tuples).for_each(|_| data.extend([0, 2, 0, 0]));
    data.extend([1, 0, 0]);
    (0..tuples).for_each(|_| data.extend([0x80, 0x80]));
    let data_at = 20 + 4 * (glyphs + 1) + 2;
    let mut t = [&[0u8, 1, 0, 0, 0, 1, 0, 1][..], &((data_at - 2) as u32).to_be_bytes()].concat();
    t.extend([(glyphs as u16).to_be_bytes(), [0, 1]].concat());
    t.extend((data_at as u32).to_be_bytes());
    let size = data.len() as u32;
    (0..=glyphs).for_each(|g| t.extend(if g == 0 { 0 } else { size }.to_be_bytes()));
    t.extend([0x40, 0]);
    t.extend(data);
    t
}

// 100 tuples over a glyph of 3 points and its 4 phantoms take the shared points' 3 bytes and 100 times 2 + 2 + 14 of
// what's left; given less, the deltas stop where they run past it, and none are given.
#[test]
fn a_glyph_s_tuples_are_spent_from_what_s_left_and_stop_past_it() {
    let map = Tables::from([("gvar", TableBytes::from_vec(gvar_of_tuples(1, 100)))]);
    let gvar = Gvar::load(&map).expect("reads").expect("a gvar");
    let source = DeltaSource { coords: &[ONE], gvar: Some(&gvar), scalars: None, hvar: false, vvar: false };
    let (mut room, ends) = (DeltaRoom::default(), [2]);
    let (x, y) = ([0, 100, 200], [0, 0, 300]);
    let mut left = 1_000_000;
    assert!(matches!(source.deltas(0, 7, (&x, &y), &ends, &mut room, &mut left), Ok(Some(_))));
    assert_eq!(1_000_000 - left, 3 + 100 * 18);
    let mut left = 500;
    let deltas = source.deltas(0, 7, (&x, &y), &ends, &mut room, &mut left).map_err(|e| e.kind());
    assert_eq!((deltas, left), (Err(crate::daecore::ErrorKind::LimitExceeded), 0));
}
