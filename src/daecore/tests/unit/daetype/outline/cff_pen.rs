use super::*;
use alloc::string::String;

// An INDEX of `subrs` that a table starts with, offSize 4, and its subroutines as drawing reads them.
fn subrs_index(subrs: &[&[u8]]) -> (Vec<u8>, Vec<u32>) {
    let mut table = [&(subrs.len() as u16).to_be_bytes()[..], &[4]].concat();
    let mut at = 1u32;
    table.extend(at.to_be_bytes());
    for subr in subrs {
        at += subr.len() as u32;
        table.extend(at.to_be_bytes());
    }
    subrs.iter().for_each(|subr| table.extend_from_slice(subr));
    let starts = subr_starts(&table, CffIndex::read(&table, 0, false).expect("the INDEX reads").0);
    (table, starts)
}

// Glyph 0 calls local subroutine 0, a bare return; glyph 1 does too after pushing a random number. The spec fixes no
// value for random, so the subroutine it calls could be any, and a subset of glyph 1 keeps them all.
#[test]
fn a_glyph_drawing_with_random_keeps_every_subroutine() {
    // A CharStrings INDEX of the two (count 2, offSize 1, offsets 1, 4, 11), then a Subrs INDEX of the subroutine.
    let cff = [&[0u8, 2, 1, 1, 4, 11][..], &[32, 10, 14], &[12, 23, 12, 18, 32, 10, 14], &[0, 1, 1, 1, 2, 11]].concat();
    let local = subr_starts(&cff, CffIndex::read(&cff, 16, false).expect("the Subrs read").0);
    let outlines = CffOutlines {
        charstrings: CffIndex::read(&cff, 0, false).expect("the INDEX reads").0, global_subrs: vec![],
        local_subrs: vec![local], fd_subrs: vec![(0, 0)], local_count: 1, fd_hints: vec![None], placements: vec![None],
        em_box: None, fd_select: None,
        charset: Charset::Predefined(0),
        sid_map: Mutable::new(None),
    };
    assert_eq!(outlines.subrs_called(&cff, &[0]), Some((vec![], vec![true])));
    assert_eq!(outlines.subrs_called(&cff, &[0, 1]), None);
}

// 96 stems make each hintmask 12 bytes, and four levels of subroutines, each calling the one below twenty times,
// repeat one hintmask 160,000 times: 7 MB of masks and their entries from a few hundred bytes.
#[test]
fn hint_masks_are_charged_to_the_work_budget() {
    let mut subrs = vec![[vec![19], vec![0xFF; 12], vec![11]].concat()];
    subrs.extend((0..3u8).map(|level| [[32 + level, 10].repeat(20), vec![11]].concat()));
    let (cff, starts) = subrs_index(&subrs.iter().map(Vec::as_slice).collect::<Vec<_>>());
    let subrs = Subrs::of(&starts);
    let main = [[139u8; 48].repeat(4), vec![1], [35u8, 10].repeat(20)].concat();

    let none = Subrs::default();
    let budget = allowed(&cff, &main, none, subrs, 0);
    let mut state = State::new(Some(CffHints::default()), TYPE2_OPERANDS, (), budget);
    let mut null = NullPen;
    let pen = &mut CloseElidingPen::new(&mut null);
    let _ = run::<false>(&cff, &main, none, subrs, subr_bias(0), subr_bias(subrs.len()), &mut state, pen);
    assert_eq!(state.n_stems, 96, "the stems did not all register");
    let held: usize = state.hints.map_or(0, |h| h.masks.iter().map(|m| m.bits.len()).sum());
    assert!(held <= budget, "{held} bytes of hint masks from one glyph");
}

// A run of operands is charged a step each, as any other token: the budget running out inside one fails the glyph
// there, the operands before it read.
#[test]
fn each_operand_of_a_run_is_charged_a_step() {
    let mut state = State::new(None, TYPE2_OPERANDS, (), 4);
    let none = Subrs::default();
    let mut null = NullPen;
    let pen = &mut CloseElidingPen::new(&mut null);
    let drawn = run::<false>(&[], &[139u8; 10], none, none, subr_bias(0), subr_bias(0), &mut state, pen);
    assert!(drawn.is_err());
    assert_eq!((state.stack.len(), state.budget), (4, 0));
}

// A mask of no stems keeps no bytes but still keeps an entry, so 990 calls of a subroutine of 1,000
// such masks would hold 990,000 entries from a charstring of 3 KB.
#[test]
fn empty_hint_masks_are_charged_too() {
    let (cff, starts) = subrs_index(&[&[vec![19u8; 1000], vec![11]].concat()]);
    let subrs = Subrs::of(&starts);
    let main = [32u8, 10].repeat(990);
    let none = Subrs::default();
    let budget = allowed(&cff, &main, none, subrs, 0);
    let mut state = State::new(Some(CffHints::default()), TYPE2_OPERANDS, (), budget);
    let mut null = NullPen;
    let pen = &mut CloseElidingPen::new(&mut null);
    let _ = run::<false>(&cff, &main, none, subrs, subr_bias(0), subr_bias(subrs.len()), &mut state, pen);
    let masks = state.hints.map_or(0, |h| h.masks.len());
    let held = masks * core::mem::size_of::<(usize, Vec<u8>)>();
    assert!(held <= budget, "{masks} empty hint masks hold {held} bytes");
}

// CFF that 1.1.7 converted from CFF2 wrote operators of up to 513 operands, past Type 2's 48: an
// hlineto of 513 draws them all, and one of 514 is refused.
#[test]
fn an_operator_of_513_operands_draws_and_one_more_does_not() {
    struct Lines(usize);
    impl OutlinePen for Lines {
        fn move_to(&mut self, _: f32, _: f32) {}
        fn line_to(&mut self, _: f32, _: f32) {
            self.0 += 1;
        }
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {}
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {}
        fn close(&mut self) {}
    }
    for (n, draws) in [(OPERAND_LIMIT, true), (OPERAND_LIMIT + 1, false)] {
        let main = [vec![139u8, 139, 21], vec![140u8; n], vec![6, 14]].concat();
        let none = Subrs::default();
        let mut state = State::new(None, TYPE2_OPERANDS, (), allowed(&[], &main, none, none, 0));
        let mut lines = Lines(0);
        let mut pen = CloseElidingPen::new(&mut lines);
        let ran = run::<false>(&[], &main, none, none, subr_bias(0), subr_bias(0), &mut state, &mut pen);
        pen.close();
        assert_eq!(ran.is_ok(), draws, "an hlineto of {n} operands: {ran:?}");
        if draws {
            assert_eq!(lines.0, n, "the hlineto drew {} of its {n} lines", lines.0);
        }
    }
    assert_eq!(OPERAND_LIMIT, 513, "CFF2's limit, which 1.1.7's conversions reach");
}

// Subroutines that push 200 each, called 4,000 times without an operator to clear them, would
// otherwise stack 800,000 operands for `record_stems` to make stems of.
#[test]
fn the_operand_stack_stops_at_513() {
    let subrs = [[vec![139u8; 200], vec![11]].concat(), [[32u8, 10].repeat(20), vec![11]].concat(),
        [[33u8, 10].repeat(20), vec![11]].concat()];
    let (cff, starts) = subrs_index(&subrs.iter().map(Vec::as_slice).collect::<Vec<_>>());
    let subrs = Subrs::of(&starts);
    let main = [[34u8, 10].repeat(10), vec![1]].concat();
    let none = Subrs::default();
    let mut state = State::new(Some(CffHints::default()), TYPE2_OPERANDS, (), allowed(&cff, &main, none, subrs, 0));
    let mut null = NullPen;
    let pen = &mut CloseElidingPen::new(&mut null);
    let ran = run::<false>(&cff, &main, none, subrs, subr_bias(0), subr_bias(subrs.len()), &mut state, pen);
    assert!(ran.is_err(), "one hstem declared {} stems", state.n_stems);
    assert!(state.stack.len() <= OPERAND_LIMIT, "{} operands stacked", state.stack.len());
}

struct Record(Vec<String>);
impl OutlinePen for Record {
    fn move_to(&mut self, x: f32, y: f32) { self.0.push(format!("M{x},{y}")); }
    fn line_to(&mut self, x: f32, y: f32) { self.0.push(format!("L{x},{y}")); }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("Q{x},{y}")); }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) { self.0.push(format!("C{x},{y}")); }
    fn close(&mut self) { self.0.push("Z".into()); }
}

// Small integers as Type 2 bytes, then the program's operator bytes.
fn n(v: i32) -> u8 { (v + 139) as u8 }

fn drawn(cs: &[u8], subrs: &[&[u8]]) -> Result<Vec<String>, crate::daecore::Error> {
    let (cff, starts) = subrs_index(subrs);
    let subrs = Subrs::of(&starts);
    let mut pen = Record(Vec::new());
    draw_charstring(&cff, (cs, subrs, 0), Subrs::default(), None, &mut 0, &mut pen)?;
    Ok(pen.0)
}

#[test]
fn dotsection_is_a_no_op() {
    let cs = [n(10), n(10), 21, n(20), 6, 12, 0, n(20), 7, 14];
    assert_eq!(drawn(&cs, &[]).unwrap(), ["M10,10", "L30,10", "L30,30", "Z"]);
}

#[test]
fn drawing_before_a_moveto_starts_at_the_current_point() {
    let cs = [n(10), n(10), 5, n(20), n(0), 5, 14];
    assert_eq!(drawn(&cs, &[]).unwrap(), ["M0,0", "L10,10", "L30,10", "Z"]);
}

#[test]
fn endchar_ends_the_glyph_wherever_it_stands() {
    let cs = [n(0), n(0), 21, n(10), n(0), 5, 14, n(50), n(50), 5];
    assert_eq!(drawn(&cs, &[]).unwrap(), ["M0,0", "L10,0", "Z"]);
    let subr = [n(10), n(0), 5, 14];
    let cs = [n(0), n(0), 21, n(-107), 10, n(0), n(50), 5, n(0), n(0), 21, n(5), n(5), 5];
    assert_eq!(drawn(&cs, &[&subr]).unwrap(), ["M0,0", "L10,0", "Z"], "endchar in a subroutine");
}

#[test]
fn the_arithmetic_operators_compute() {
    let program = |ops: &[u8]| [&[n(0), n(0), 21][..], ops, &[n(0), 5, 14]].concat();
    let x_of = |ops: &[u8]| drawn(&program(ops), &[]).unwrap()[1].clone();
    assert_eq!(x_of(&[n(4), n(2), 12, 12]), "L2,0", "div");
    assert_eq!(x_of(&[n(3), n(4), 12, 24]), "L12,0", "mul");
    assert_eq!(x_of(&[n(-7), 12, 9]), "L7,0", "abs");
    assert_eq!(x_of(&[n(5), n(6), n(1), 12, 29, 12, 10, 12, 10]), "L16,0", "index copies 5 over 6, then two adds");
    assert_eq!(x_of(&[n(1), n(2), n(3), n(3), n(1), 12, 30, 12, 18, 12, 18]), "L3,0", "roll brings 3 to the bottom");
    assert_eq!(x_of(&[n(5), n(9), n(1), n(2), 12, 22]), "L5,0", "ifelse keeps the first when v1 <= v2");
    assert_eq!(x_of(&[n(6), n(3), 12, 20, n(3), 12, 21]), "L6,0", "put then get");
    assert_eq!(x_of(&[n(2), n(2), 12, 15, n(1), 12, 3]), "L1,0", "eq then and");
}

// mul and div round a half away from zero and hold to 16.16's range; division by 0 gives the dividend's sign's largest
// value, 0 / 0 gives 0.
#[test]
fn mul_and_div_round_away_from_zero_and_saturate() {
    assert_eq!((multiply(0x8000, 1), multiply(-0x8000, 1), multiply(3 << 16, 0x8000)), (1, -1, 0x18000));
    assert_eq!((multiply(i32::MAX, i32::MAX), multiply(i32::MIN, i32::MAX)), (i32::MAX, -i32::MAX));
    assert_eq!((divide(1 << 16, 3 << 16), divide(-1 << 16, 3 << 16)), (21_845, -21_845));
    assert_eq!((divide(5, 0), divide(-5, 0), divide(0, 0)), (i32::MAX, -i32::MAX, 0));
    assert_eq!(divide(i32::MAX, 1), i32::MAX);
}

// A subroutine number is the operand's value, whole however it's written: -107 as an integer, as a 16.16 or from
// `add` calls subroutine 0; -106.5 as a 16.16 fails the glyph.
#[test]
fn a_subroutine_number_is_whole_however_it_is_written() {
    let ret: &[u8] = &[11];
    for cs in [&[n(-107), 10, 14][..], &[n(-107), n(0), 12, 10, 10, 14], &[255, 0xFF, 0x95, 0, 0, 10, 14]] {
        assert!(drawn(cs, &[ret]).is_ok(), "{cs:?}");
    }
    let err = drawn(&[255, 0xFF, 0x95, 0x80, 0, 10, 14], &[ret]).unwrap_err();
    assert!(err.message().contains("integer"), "{err}");
}

struct Half;

impl Cff2Weights for Half {
    fn weights(&self, _: u32) -> Result<Vec<i32>, crate::daecore::Error> {
        Ok([0x10000, 0x8000].to_vec())
    }

    fn work(&self, _: u32) -> usize {
        1
    }
}

struct Heavy;

impl Cff2Weights for Heavy {
    fn weights(&self, _: u32) -> Result<Vec<i32>, crate::daecore::Error> {
        unreachable!("weighed past the step budget")
    }

    fn work(&self, _: u32) -> usize {
        1_000_000
    }
}

// A blend whose weighing would take the whole step budget fails the glyph before a region is weighed.
#[test]
fn a_blend_weighing_past_the_step_budget_fails_before_weighing() {
    let (cff, empty) = subrs_index(&[&[]]);
    let cs = [n(0), 16];
    let mut state = State::new(None, OPERAND_LIMIT, (), allowed(&cff, &cs, Subrs::default(), Subrs::of(&empty), 0));
    state.width_taken = true;
    let blending = Blending { weights: &Heavy, built: None, vsindex_read: false };
    state.cff2 = Some(Cff2Run { blending, vsindex: 0 });
    let mut pen = Record(Vec::new());
    let err = draw_with(&cff, &cs, Subrs::default(), Subrs::of(&empty), &mut state, &mut pen).unwrap_err();
    assert!(err.message().contains("budget"), "{err}");
}

struct Flat;

impl Cff2Weights for Flat {
    fn weights(&self, _: u32) -> Result<Vec<i32>, crate::daecore::Error> {
        Ok([0x10000].to_vec())
    }

    fn work(&self, _: u32) -> usize {
        1
    }
}

// Under a vsindex of no regions a blend consumes nothing, so `512 blend` reads the same 512 defaults again for two
// bytes; 2,000 of those read a million, past the 420,000 steps 6,500 bytes take, as each read is a step.
#[test]
fn a_blend_is_charged_a_step_for_each_default_it_reads() {
    let (cff, empty) = subrs_index(&[&[]]);
    let cs = [vec![n(0); 512], [248, 148, 16].repeat(2_000)].concat();
    let mut state = State::new(None, OPERAND_LIMIT, (), allowed(&cff, &cs, Subrs::default(), Subrs::of(&empty), 0));
    state.width_taken = true;
    state.cff2 = Some(Cff2Run { blending: Blending { weights: &Flat, built: None, vsindex_read: false }, vsindex: 0 });
    let mut pen = Record(Vec::new());
    let err = draw_with(&cff, &cs, Subrs::default(), Subrs::of(&empty), &mut state, &mut pen).unwrap_err();
    assert!(err.message().contains("budget"), "{err}");
}

// A CFF2 table of one square whose FD's Private DICT blends its blue zones, `0 blend`, by a store of `regions`
// regions over `axes` axes.
fn blended_private(axes: u16, regions: u16) -> Vec<u8> {
    let int = |v: usize| [&[29u8][..], &(v as u32).to_be_bytes()].concat();
    let index = |item: &[u8]| [&1u32.to_be_bytes()[..], &[1, 1, 1 + item.len() as u8], item].concat();
    let square = [n(0), n(0), 21, n(100), n(0), 5, n(0), n(100), 5, n(-100), n(0), 5];
    let private = [n(-10), n(0), 28, 1, 0xF4, 28, 1, 0xFE, n(0), 23, 6];
    let mut store = [&1u16.to_be_bytes()[..], &12u32.to_be_bytes(), &1u16.to_be_bytes()].concat();
    store.extend((16 + 6 * u32::from(axes)).to_be_bytes());
    store.extend([axes.to_be_bytes(), 1u16.to_be_bytes()].concat());
    store.resize(store.len() + 6 * usize::from(axes), 0);
    store.extend([0, 0, 0, 0]);
    store.extend(regions.to_be_bytes());
    store.resize(store.len() + 2 * usize::from(regions), 0);
    let cs_at = 5 + 19 + 4;
    let fd_at = cs_at + 7 + square.len();
    let private_at = fd_at + 7 + 11;
    let store_at = private_at + private.len();
    let mut t = vec![2, 0, 5, 0, 19];
    t.extend([int(cs_at), vec![17], int(fd_at), vec![12, 36], int(store_at), vec![24]].concat());
    t.extend(0u32.to_be_bytes());
    t.extend(index(&square));
    t.extend(index(&[int(private.len()), int(private_at), vec![18]].concat()));
    t.extend(private);
    t.extend((store.len() as u16).to_be_bytes());
    t.extend(store);
    t
}

// Hinting weighs the Private DICT's blends before the charstring runs, charged to its steps as a blend's weighing
// is: 2,000 regions over 600 axes come to 1.2 million and leave the square unhinted; one over one hints it.
#[test]
fn a_private_dict_s_weighing_past_the_step_budget_leaves_the_glyph_unhinted() {
    let hinted = |axes: u16, regions: u16| {
        let cff2 = blended_private(axes, regions);
        let outlines = Cff2Outlines::parse_in(&cff2, 1000, None).expect("parses");
        hint_cff2_glyph(&outlines, &cff2, 0, Some(&vec![0; usize::from(axes)]), 1_311).is_some()
    };
    assert!(hinted(1, 1));
    assert!(!hinted(600, 2_000));
}

// A CFF2 blend sums in 16.16: 10 and 20 plus 4.0 and 6.0 at half weight are 12 and 23, past an empty subroutine
// called from each delta's slot. A blended subroutine number must be whole: -107 plus 1.0 at half weight fails.
#[test]
fn a_cff2_blend_sums_in_16_16_and_a_blended_subroutine_number_must_be_whole() {
    let drawn = |cs: &[u8]| {
        let (cff, empty) = subrs_index(&[&[]]);
        let mut state = State::new(None, OPERAND_LIMIT, (), allowed(&cff, cs, Subrs::default(), Subrs::of(&empty), 0));
        state.width_taken = true;
        let blending = Blending { weights: &Half, built: None, vsindex_read: false };
        state.cff2 = Some(Cff2Run { blending, vsindex: 0 });
        let mut pen = Record(Vec::new());
        draw_with(&cff, cs, Subrs::default(), Subrs::of(&empty), &mut state, &mut pen).map(|()| pen.0)
    };
    let (four, six, zero, one) = ([255, 0, 4, 0, 0], [255, 0, 6, 0, 0], [255, 0, 0, 0, 0], [255, 0, 1, 0, 0]);
    let freed = [&[n(10), n(20)][..], &four, &six, &[n(2), 16, n(-107), n(-107), 10, 10, 21]].concat();
    assert_eq!(drawn(&freed).unwrap(), ["M12,23", "Z"]);
    let err = drawn(&[&[n(10), n(-107)][..], &zero, &one, &[n(2), 16, 10]].concat()).unwrap_err();
    assert!(err.message().contains("integer"), "{err}");
}

// `random` from a seed: each step state * 6364136223846793005 + 1442695040888963407 in 64 bits, its top 16 bits plus 1
// a number in (0, 1] in 65,536ths, here times 300 by `mul`, first along x, then along y.
#[test]
fn random_steps_its_generator_from_the_seed() {
    let cs = [n(0), n(0), 21, 12, 23, 247, 192, 12, 24, n(0), 5, n(0), 12, 23, 247, 192, 12, 24, 5, 14];
    for seed in [0u32, 12_345] {
        let mut pen = Record(Vec::new());
        let none = Subrs::default();
        draw_charstring(&[], (&cs, none, seed), none, None, &mut 0, &mut pen).unwrap();
        let step = |s: u64| s.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let (first, second) = (step(u64::from(seed)), step(step(u64::from(seed))));
        let [a, b] = [first, second].map(|s| (s >> 48) as i32 + 1);
        assert!((1..=65_536).contains(&a) && (1..=65_536).contains(&b));
        let at = |v: i32| (f64::from(v * 300) / 65536.0) as f32;
        assert_eq!(pen.0[1..3], [format!("L{},0", at(a)), format!("L{},{}", at(a), at(b))], "seed {seed}");
    }
}

// The square root ends, and lands within half a unit of the exact one, over a sweep of 16.16 values.
#[test]
fn sqrt_fixed_rounds_to_the_nearest() {
    for v in (1..=i32::MAX as u32).step_by(9_973).chain([1, 2, 0x10000, 0x20000, i32::MAX as u32]) {
        let exact = (f64::from(v) * 65536.0).sqrt();
        assert!((f64::from(sqrt_fixed(v)) - exact).abs() <= 0.5, "sqrt of {v:#x}: {}", sqrt_fixed(v));
    }
    assert_eq!((sqrt_fixed(0x20000), sqrt_fixed(0x30000)), (92_682, 113_512));
}

// A stack value past 16.16's range holds at its ends instead of wrapping around to the other sign.
#[test]
fn a_stack_value_past_16_16_holds_at_its_ends() {
    assert_eq!([to_fixed(1.5), to_fixed(-1.5), to_fixed(1e-6)], [0x18000, -0x18000, 0]);
    assert_eq!([to_fixed(40_000.0), to_fixed(-40_000.0), to_fixed(1e300)], [i32::MAX, -i32::MAX, i32::MAX]);
}

#[test]
fn a_subroutine_that_calls_itself_is_refused() {
    let subr = [n(-107), 10];
    let err = drawn(&[n(-107), 10], &[&subr]).unwrap_err();
    assert!(err.message().contains("nesting too deep"), "{err}");
}

// The bias comes from the count of subroutines: with 1,239 it is 107, so -107 calls the first, which draws; with
// one more it would be 1,131 and call the 1,025th, a bare return.
#[test]
fn the_bias_follows_the_count_of_subroutines() {
    let first = [n(10), n(10), 21, n(20), 6, 11];
    let mut subrs = vec![&first[..]];
    subrs.extend(core::iter::repeat_n(&[11u8][..], 1238));
    assert_eq!(drawn(&[n(-107), 10, 14], &subrs).unwrap(), ["M10,10", "L30,10", "Z"]);
}

// Two contours, the first closed by a curve back to its start (a dropped duplicate) and a line to it
// (elided), then a mask: it names the second contour's first point as a collecting pen numbers it.
#[test]
fn a_hint_mask_counts_the_points_a_collecting_pen_keeps() {
    use super::super::super::hinting::auto::CollectPen;
    let cs = [
        n(10), n(20), 1,
        n(0), n(0), 21, n(50), n(0), 5, n(0), n(50), 5, n(-50), n(-50), 5,
        n(0), n(10), n(10), n(10), n(10), n(-30), 8,
        19, 0x80,
        n(100), n(0), 21, n(10), n(0), 5, 14,
    ];
    let mut pen = CollectPen::new();
    let hints = Some(CffHints::default());
    let none = Subrs::default();
    let (hints, _) = draw_charstring(&[], (&cs, none, 0), none, hints, &mut 0, &mut pen).unwrap();
    let points = pen.finish();
    let second = points.contour_ends[0] + 1;
    assert_eq!(hints.unwrap().masks.iter().map(|m| m.point).collect::<Vec<_>>(), [second]);
}

// An outline isn't held to glyf's 65,535 points, which CFF doesn't number: 100,000 lines draw, and 40,000 starts with
// a line after each.
#[test]
fn an_outline_holds_what_its_bytes_draw() {
    let lines = [[n(1), n(0)].repeat(24), alloc::vec![5]].concat();
    let cs = [alloc::vec![n(0), n(0), 21], lines.repeat(100_000 / 24 + 1), alloc::vec![14]].concat();
    let none = Subrs::default();
    assert!(draw_charstring(&[], (&cs, none, 0), none, None, &mut 0, &mut NullPen).is_ok());
    let cs = [[n(1), n(0), 21, n(1), 7].repeat(40_000), alloc::vec![14]].concat();
    assert!(draw_charstring(&[], (&cs, none, 0), none, None, &mut 0, &mut NullPen).is_ok());
}

// A subroutine of 60,000 bytes drawn 20 times takes 1.2 million steps, 20 for each of its bytes: within the 64 a byte
// a program may take. Nine levels of subroutines each calling the next 20 times would take 20^9 from 400 bytes.
#[test]
fn the_steps_a_program_takes_are_bound_by_its_bytes() {
    let big = [[[n(1), n(0)].repeat(24), alloc::vec![5]].concat().repeat(60_000 / 49), alloc::vec![11]].concat();
    let cs = [alloc::vec![n(0), n(0), 21], [n(-107), 10].repeat(20), alloc::vec![14]].concat();
    let mut steps = 0;
    assert!(drawn_counted(&cs, &[&big], &mut steps).is_ok(), "a 60 KB subroutine drawn 20 times was refused");
    assert!(steps > 1_000_000, "{steps} steps");

    let calls = |next: i32| [[n(next - 107), 10].repeat(20), alloc::vec![11]].concat();
    let mut subrs: Vec<Vec<u8>> = (1..10).map(calls).collect();
    subrs.push(alloc::vec![11]);
    let subrs: Vec<&[u8]> = subrs.iter().map(Vec::as_slice).collect();
    let mut steps = 0;
    let err = drawn_counted(&[n(-107), 10, 14], &subrs, &mut steps).unwrap_err();
    assert!(err.message().contains("budget"), "{err}");
    let bytes = subrs.iter().map(|s| s.len()).sum::<usize>() + 3;
    assert!(steps <= STEPS_PER_BYTE * bytes, "{steps} steps from {bytes} bytes");
}

fn drawn_counted(cs: &[u8], subrs: &[&[u8]], steps: &mut usize) -> Result<(), crate::daecore::Error> {
    let (cff, starts) = subrs_index(subrs);
    draw_charstring(&cff, (cs, Subrs::of(&starts), 0), Subrs::default(), None, steps, &mut NullPen).map(drop)
}

// A CFF2 table of one empty glyph whose `fds` FDs each name `size` bytes of Private DICT, FD i's from i * `step`
// bytes into one run of them: with a step of 1 their spans overlap, with 0 they name one span.
fn overlapping_privates(fds: usize, size: usize, step: usize) -> Vec<u8> {
    let int = |v: usize| [&[29u8][..], &(v as u32).to_be_bytes()].concat();
    let index = |items: &[Vec<u8>]| {
        let mut out = [&(items.len() as u32).to_be_bytes()[..], &[4]].concat();
        let mut at = 1u32;
        out.extend(at.to_be_bytes());
        for item in items {
            at += item.len() as u32;
            out.extend(at.to_be_bytes());
        }
        items.iter().for_each(|item| out.extend(item));
        out
    };
    let (cs_at, fd_at) = (5 + 13 + 4, 5 + 13 + 4 + 13);
    let private_at = fd_at + 5 + 4 * (fds + 1) + 11 * fds;
    let fonts: Vec<Vec<u8>> = (0..fds).map(|i| [int(size), int(private_at + i * step), vec![18]].concat()).collect();
    let mut t = vec![2, 0, 5, 0, 13];
    t.extend([int(cs_at), vec![17], int(fd_at), vec![12, 36]].concat());
    t.extend(0u32.to_be_bytes());
    t.extend(index(&[vec![]]));
    t.extend(index(&fonts));
    t.extend(vec![0; size + fds]);
    t
}

// FDs reading their own overlapping Private DICTs share the subset budget, twice the table and 64 KB: 100 of 300 bytes
// each fit it, 100 of 1,000 bytes refuse the font, and 100 naming one of 1,000 bytes read and charge it once.
#[test]
fn private_dicts_read_past_the_subset_budget_refuse_the_font() {
    let parsed = |size: usize, step: usize| {
        Cff2Outlines::parse(&overlapping_privates(100, size, step)).map(drop).map_err(|e| e.kind())
    };
    assert_eq!(parsed(300, 1), Ok(()));
    assert_eq!(parsed(1_000, 1), Err(ErrorKind::LimitExceeded));
    assert_eq!(parsed(1_000, 0), Ok(()));
}
