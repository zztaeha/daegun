use super::*;
use alloc::string::String;

fn tables(entries: &[(&str, Vec<u8>)]) -> Tables {
    entries.iter().map(|(tag, bytes)| (String::from(*tag), TableBytes::from_vec(bytes.clone()))).collect()
}

// A face with bytecode smooths symmetrically past 15 ppem, one without at every size, unless a version 1 gasp
// range covering the size says otherwise; a version 0 or 2 gasp, or ranges cut by its end, say nothing.
#[test]
fn symmetric_smoothing_follows_bytecode_and_gasp() {
    let gasp = |version: u16, ranges: &[(u16, u16)]| {
        let mut t = [version, ranges.len() as u16].map(u16::to_be_bytes).concat();
        ranges.iter().for_each(|&(max, flags)| t.extend([max, flags].map(u16::to_be_bytes).concat()));
        t
    };
    assert_eq!([15, 16].map(|ppem| symmetric_smoothing(&[], 1, ppem)), [false, true]);
    assert!(symmetric_smoothing(&[], 0, 9));
    let ranges = [(8, 0x0f), (16, 0x07), (65535, 0x0f)];
    let by_gasp = [8, 9, 16, 17, 30].map(|ppem| symmetric_smoothing(&gasp(1, &ranges), 1, ppem));
    assert_eq!(by_gasp, [true, false, false, true, true]);
    assert!(!symmetric_smoothing(&gasp(0, &ranges), 1, 8));
    assert!(!symmetric_smoothing(&gasp(2, &ranges), 1, 8));
    assert!(!symmetric_smoothing(&gasp(1, &ranges)[..12], 1, 8));
}

// A CVT program declaring native ClearType (INSTCTRL selector 3), under which points move along x as along y.
const NATIVE: [u8; 4] = [0xB1, 0x04, 0x03, 0x8E];

fn maxp() -> Vec<u8> {
    let mut maxp = alloc::vec![0u8; 32];
    maxp[18..20].copy_from_slice(&2u16.to_be_bytes());
    maxp[24..26].copy_from_slice(&16u16.to_be_bytes());
    maxp
}

// A negative CVT entry, read right only by a sign-extending decode, and a prep storing MPPEM and MPS and writing 300 by
// WCVTF, which scales font units: at 16.4 px MPPEM reads whole pixels, 16, and MPS 26.6, 1,050.
#[test]
fn a_negative_cvt_and_the_size_instructions_read_right() {
    let cvt: Vec<u8> = [-250i16, 100, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
    let prep = alloc::vec![0xB0, 0x00, 0x4B, 0x42, 0xB0, 0x01, 0x4C, 0x42, 0xB0, 0x02, 0xB8, 0x01, 0x2C, 0x70];
    let map = tables(&[("maxp", maxp()), ("cvt ", cvt), ("prep", prep)]);
    let size = f26dot6::Size::from_px(16.4).expect("a size");
    let scaled = |v| f26dot6::scale_units(v, size.scale16(1000));
    let ctx = HintContext::new(&map, size, 1000).expect("the font has a prep");
    assert_eq!(ctx.base_cvt, [scaled(-250), scaled(100), scaled(300)]);
    assert!(ctx.base_cvt[0] < 0, "-250 decoded as {}", ctx.base_cvt[0]);
    assert_eq!(&ctx.base_storage[..2], [16, 1050], "MPPEM and MPS at 16.4 px");
}

// SSW takes font units and MDRP and MIRP compare it with 26.6 distances, so it is scaled as it is stored: 180 units
// of a 1000 em are 180 * 64 * ppem / 1000, 92.16, 184.32 and 368.64 at 8, 16 and 32 ppem, rounded.
#[test]
fn the_single_width_is_scaled_to_the_size() {
    let map = tables(&[("maxp", maxp()), ("prep", alloc::vec![0xB8, 0x00, 0xB4, 0x1F])]);
    for (ppem, want) in [(8u16, 92), (16, 184), (32, 369)] {
        let ctx = HintContext::new(&map, ppem.into(), 1000).expect("the font has a prep");
        assert_eq!(ctx.glyph_state.single_width, want, "at {ppem} ppem");
    }
}

// Bytecode can push a point to the edge of i32 (SHPIX twice, by a product MUL clamps), and IUP then
// works out the other points from it. Overflow there is a panic in a debug build and garbage in a release one.
#[test]
fn iup_from_a_point_at_the_rail_does_not_overflow() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/hinted.ttf");
    let bytes = std::fs::read(path).expect("the hinted fixture is missing");
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("the fixture parses");
    let upm = map.get("head").and_then(|h| read_u16_be(h, 18)).expect("units per em");
    let shift: &[u8] = &[0xB8, 0x00, 0x00, 0xB9, 0x7F, 0xFF, 0x7F, 0xFF, 0x63, 0xB8, 0x7F, 0xFF, 0x63, 0x38];
    let glyph = |program: &[u8], ys: [i16; 3]| {
        let mut glyf: Vec<u8> = Vec::new();
        for v in [1i16, 0, -100, 100, 0, 2] {
            glyf.extend(v.to_be_bytes());
        }
        glyf.extend((program.len() as u16).to_be_bytes());
        glyf.extend(program);
        glyf.extend([0x01u8; 3]);
        for v in [0i16, 100, 0].into_iter().chain(ys) {
            glyf.extend(v.to_be_bytes());
        }
        glyf
    };
    // Point 0 touched alone (MDAP[0]; SHPIX moves only points touched in y), IUP shifts the rest with it;
    // with point 1 touched too (PUSHB 1, MDAP[0]), point 2 between them is interpolated from the rail.
    let start: &[u8] = &[0x00, 0xB0, 0x00, 0x2E];
    let alone = glyph(&[start, shift, shift, &[0x30]].concat(), [-100, 0, 100]);
    let between = glyph(&[start, shift, shift, &[0xB0, 0x01, 0x2E, 0x30]].concat(), [0, -100, 50]);
    for glyf in [alone, between] {
        let loca = [0usize, glyf.len()];
        let mut ctx = HintContext::new(&map, 16.into(), upm).expect("the fixture is hinted");
        let out = ctx.hint_glyph(&glyf, &loca, 0).expect("the glyph hints");
        assert_eq!(out.y[0], i32::MAX, "SHPIX did not take point 0 to the rail");
    }
}

// `build_hinted_fixture.py` writes this CVT. With the negative-CVT test above, these values are what
// checks the decode: no outline in the suite shows how far a CVT moves a point.
#[test]
fn the_cvt_decodes_to_the_values_the_fixture_was_built_with() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/test-fixtures/hinted.ttf");
    let bytes = std::fs::read(path).expect("the hinted fixture is missing");
    let map = crate::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("the fixture parses");
    let upm = map.get("head").and_then(|h| read_u16_be(h, 18)).expect("head names its units per em");
    assert_eq!(upm, 1024, "the fixture's units per em moved; rebuild the expected values");

    let units = [0, 100, 200, 400, 512];
    for ppem in [8u16, 11, 16, 24, 40] {
        let ctx = HintContext::new(&map, ppem.into(), upm).expect("the fixture is hinted");
        let want: Vec<i32> =
            units.iter().map(|&v| f26dot6::scale_units(v, f26dot6::Size::from(ppem).scale16(upm))).collect();
        assert_eq!(ctx.base_cvt, want, "at {ppem} ppem");
    }
    let size = f26dot6::Size::from_px(16.4).expect("a size");
    let ctx = HintContext::new(&map, size, upm).expect("the fixture is hinted");
    let want: Vec<i32> = units.iter().map(|&v| f26dot6::scale_units(v, size.scale16(upm))).collect();
    assert_eq!(ctx.base_cvt, want, "at 16.4 px");
    assert_ne!(
        want,
        units.map(|v| f26dot6::scale_units(v, f26dot6::Size::from(16).scale16(upm))),
        "16.4 px scaled as 16"
    );
}

// MIRP pops a CVT entry and a point. Short of two it underflows the stack and stops the program, so the SHPIX after
// it, moving point 0 a pixel on its own, never runs, whichever lone value is left.
#[test]
fn an_instruction_short_of_its_arguments_stops_the_program() {
    let cvt: Vec<u8> = [100i16, 0, 0, 500, 0, 0, 0, 300].iter().flat_map(|v| v.to_be_bytes()).collect();
    let map = tables(&[("maxp", maxp()), ("cvt ", cvt), ("prep", NATIVE.to_vec())]);
    let x_after = |program: &[u8]| {
        let glyf =
            [&[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1][..], &(program.len() as u16).to_be_bytes(), program, &[0x31, 0x31]]
                .concat();
        let mut ctx = HintContext::new(&map, 16.into(), 1000).expect("a context");
        ctx.hint_glyph(&glyf, &[0, glyf.len()], 0).expect("the glyph hints").x[0]
    };
    let (still, shift) = (x_after(&[0xB0, 0, 0x21]), [0xB1, 0, 64, 0x38]);
    assert_eq!(x_after(&shift), still + 64);
    for lone in [3, 7] {
        assert_eq!(x_after(&[&[0xB0, lone, 0xE0][..], &shift].concat()), still, "MIRP on {lone} alone");
    }
}

// A two-point glyph at the origin, its program run under native ClearType so points move along x: point 0's x after.
fn x_after(map: &Tables, program: &[u8]) -> i32 {
    let header = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let glyf = [&header[..], &(program.len() as u16).to_be_bytes(), program, &[0x31, 0x31]].concat();
    let mut ctx = HintContext::new(map, 16.into(), 1000).expect("a context");
    ctx.hint_glyph(&glyf, &[0, glyf.len()], 0).expect("the glyph hints").x[0]
}

// ODD and EVEN on a value rounded by RTG or RTHG, SHPIX moving point 0 by the answer in 64ths: a value RTHG puts on a
// half pixel, 1.5 px from 1 or 0.5 px from 0, is neither odd nor even.
#[test]
fn a_value_rounded_off_whole_pixels_is_neither_odd_nor_even() {
    let map = tables(&[("maxp", maxp()), ("prep", NATIVE.to_vec())]);
    let (rtg, rthg, odd, even) = (0x18, 0x19, 0x56, 0x57);
    let cases = [(rtg, 64, odd), (rtg, 128, even), (rtg, 64, even), (rthg, 64, odd), (rthg, 0, even)];
    let answers = cases.map(|(round, value, op)| x_after(&map, &[0xB1, 0, value, round, op, 0x38]));
    assert_eq!(answers, [1, 1, 0, 0, 0]);
}

// MIRP[rnd] point 0 to a CVT of 3.2 px from rp0 at an original distance of 0, past the cut-in: from twilight point 0
// the points lie in two zones, so the CVT holds, 3 px; from glyph point 1 they share one, and the original wins.
#[test]
fn mirp_tests_its_cut_in_only_within_one_zone() {
    let mut maxp = maxp();
    maxp[16..18].copy_from_slice(&1u16.to_be_bytes());
    let map = tables(&[("maxp", maxp), ("cvt ", words(&[200])), ("prep", NATIVE.to_vec())]);
    let from = |zone: u8, rp0: u8| x_after(&map, &[0xB0, zone, 0x13, 0xB0, rp0, 0x10, 0xB1, 0, 0, 0xE4]);
    assert_eq!([from(0, 0), from(1, 1)], [192, 0]);
}

// Function numbers run to 65,535 whatever maxp's maxFunctionDefs declares: 40 FDEFs under a declared 10 hint the
// font, as does function 65,535 (0x7FFF twice, plus 1); function 65,536 fails the font program.
#[test]
fn function_numbers_run_to_65535_whatever_maxp_declares() {
    let mut maxp = maxp();
    maxp[20..22].copy_from_slice(&10u16.to_be_bytes());
    let context = |fpgm: Vec<u8>| {
        HintContext::new(&tables(&[("maxp", maxp.clone()), ("fpgm", fpgm)]), 16.into(), 1000)
    };
    assert!(context((0..40).flat_map(|i| [0xB0, i, 0x2C, 0x2D]).collect()).is_some(), "40 under a declared 10");
    let numbered = |plus: u8| [&[0xB9, 0x7F, 0xFF, 0x7F, 0xFF, 0x60, 0xB0, plus, 0x60][..], &[0x2C, 0x2D]].concat();
    assert!(context(numbered(1)).is_some(), "function 65,535 left the font unhinted");
    assert!(context(numbered(2)).is_none(), "function 65,536 was taken");
}

// FLIPRGON over all 65,531 points of a glyph, `times` times in a loop, then one FLIPRGOFF: at a unit a flip, 3,047
// passes fit the 201.3 million the points grow the budget to, and past them the points stay on the curve.
#[test]
fn fliprg_over_a_whole_glyph_charges_the_budget_its_points_grow() {
    const POINTS: u16 = 65_531;
    let mut maxp = maxp();
    maxp[4..6].copy_from_slice(&u16::MAX.to_be_bytes());
    let map = tables(&[("maxp", maxp)]);
    let on_curve_after = |times: u16| {
        let last = (POINTS - 1).to_be_bytes();
        let program = [
            &[0xB8][..], &times.to_be_bytes(),
            &[0xB9, 0, 0, last[0], last[1], 0x81],
            &[0xB0, 1, 0x61, 0x20, 0xB8, 0xFF, 0xF2, 0x23, 0x78],
            &[0xB9, 0, 0, last[0], last[1], 0x82],
        ].concat();
        let glyf = [
            &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0][..], &last, &(program.len() as u16).to_be_bytes(), &program,
            &alloc::vec![0x31; usize::from(POINTS)],
        ].concat();
        let mut ctx = HintContext::new(&map, 16.into(), 1000).expect("a context");
        let out = ctx.hint_glyph(&glyf, &[0, glyf.len()], 0).expect("the glyph hints");
        out.flags.iter().all(|f| f & FLAG_ON_CURVE != 0)
    };
    assert!(!on_curve_after(3_047), "3,047 passes fit the budget, and the FLIPRGOFF after them did not run");
    assert!(on_curve_after(3_048), "3,048 passes of 65,531 flips ran past the budget to the FLIPRGOFF");
}

// fvar claims 65,535 axes and function 0 is GETVARIATION then CLEAR: each call pushes every axis, a unit each, so
// 1,021 calls fit the CVT program's 67.1 million and 1,022 run past it, failing the program.
#[test]
fn getvariation_charges_each_axis_it_pushes() {
    let mut fvar = alloc::vec![0u8; 16];
    fvar[8..10].copy_from_slice(&u16::MAX.to_be_bytes());
    let fpgm = alloc::vec![0xB0, 0, 0x2C, 0x91, 0x22, 0x2D];
    let runs = |calls: u16| {
        let prep = [&[0xB9][..], &calls.to_be_bytes(), &[0, 0, 0x2A]].concat();
        let map = tables(&[("maxp", maxp()), ("fvar", fvar.clone()), ("fpgm", fpgm.clone()), ("prep", prep)]);
        HintContext::new(&map, 16.into(), 1000).is_some()
    };
    assert_eq!([1_021, 1_022].map(runs), [true, false]);
}

#[derive(Default)]
struct Verbs(String);

impl super::super::outline::OutlinePen for Verbs {
    fn move_to(&mut self, _: f32, _: f32) { self.0.push('M') }
    fn line_to(&mut self, _: f32, _: f32) { self.0.push('L') }
    fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) { self.0.push('Q') }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) { self.0.push('C') }
    fn close(&mut self) { self.0.push('Z') }
}

// A point on the curve and two controls, each with GLYF's cubic bit, which hinting keeps as FLAG_CUBIC on the controls
// alone, as the draft holds it 0 on the curve: FLIPPT takes the point off as a quadratic one, its contour then mixed.
#[test]
fn glyf_s_uppercase_twin_hints_its_cubic_controls() {
    let map = tables(&[("maxp", maxp())]);
    let (on, cubic) = (FLAG_ON_CURVE, FLAG_CUBIC);
    let cases = [
        (true, false, [on, cubic, cubic], "MCZ"),
        (true, true, [0, cubic, cubic], "MQQQZ"),
        (false, false, [on, 0, 0], "MQQZ"),
        (false, true, [0; 3], "MQQQZ"),
    ];
    for (extended, flip, flags, verbs) in cases {
        let program: &[u8] = if flip { &[0xB0, 0, 0x80] } else { &[0xB0, 0, 0x21] };
        let points = [0xB1, 0xB6, 0x96, 10, 10, 10, 10];
        let header = [0, 1, 0, 0, 0, 0, 0, 20, 0, 10, 0, 2];
        let glyf = [&header[..], &(program.len() as u16).to_be_bytes(), program, &points].concat();
        let mut ctx = HintContext::new(&map, 16.into(), 1000).expect("a context");
        ctx.extended = extended;
        let out = ctx.hint_glyph(&glyf, &[0, glyf.len()], 0).expect("the glyph hints");
        let mut drawn = Verbs::default();
        draw_hinted(out, &mut drawn);
        assert_eq!((&out.flags[..], &drawn.0[..]), (&flags[..], verbs), "GLYF {extended}, flipped {flip}");
    }
}

// A composite's program sees its component's cubic controls as FLAG_CUBIC, and its outline keeps them.
#[test]
fn a_hinted_composite_keeps_its_component_s_cubic_controls() {
    let map = tables(&[("maxp", maxp())]);
    let simple = [0, 1, 0, 0, 0, 0, 0, 20, 0, 10, 0, 2, 0, 0, 0x31, 0xB6, 0x96, 10, 10, 10, 10];
    let composite = [0xFF, 0xFF, 0, 0, 0, 0, 0, 20, 0, 10, 0x01, 0x02, 0, 1, 0, 0, 0, 3, 0xB0, 0, 0x21];
    let glyf = [&composite[..], &simple].concat();
    let mut ctx = HintContext::new(&map, 16.into(), 1000).expect("a context");
    ctx.extended = true;
    let out = ctx.hint_glyph(&glyf, &[0, composite.len(), glyf.len()], 0).expect("the composite hints");
    assert_eq!(out.flags, [FLAG_ON_CURVE, FLAG_CUBIC, FLAG_CUBIC]);
}

// A two-point glyph running `program` with function 0 defined by `body`, hinted twice on one context.
fn calling(body: &[u8], program: &[u8]) -> [HintedOutline; 2] {
    let fpgm = [&[0xB0, 0, 0x2C][..], body, &[0x2D]].concat();
    let map = tables(&[("maxp", maxp()), ("fpgm", fpgm), ("prep", NATIVE.to_vec())]);
    let header = [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
    let glyf = [&header[..], &(program.len() as u16).to_be_bytes(), program, &[0x31, 0x31]].concat();
    let mut ctx = HintContext::new(&map, 16.into(), 1000).expect("a context");
    [(); 2].map(|()| ctx.hint_glyph(&glyf, &[0, glyf.len()], 0).expect("the glyph hints").clone())
}

// An untaken IF over 199,999 opcodes, called `calls` times, then a FLIPRGOFF on point 0. Each call costs the 200,000
// opcodes it passes, 32 units each, so 10 calls leave room in the glyph's 67.1 million for the flip and 11 use it up.
#[test]
fn a_skipped_branch_charges_its_opcodes_on_every_call() {
    let body = [&[0xB0, 0, 0x58][..], &alloc::vec![0x20; 199_999], &[0x59]].concat();
    let flipped = |calls: u8| {
        calling(&body, &[0xB1, calls, 0, 0x2A, 0xB1, 0, 0, 0x82]).map(|o| o.flags[0] & FLAG_ON_CURVE == 0)
    };
    assert_eq!(flipped(10), [true; 2], "10 calls of 200,000 ran out of budget before the flip");
    assert_eq!(flipped(11), [false; 2], "11 calls of 200,000 left budget for the flip");
}

// A function shifting point 0 by 1 px when its argument is set, else by 2, called with 0, 0, 1 and 1 after the
// glyph skips an IF at the IP of the function's: each call lands where a scan would, in a glyph and the next.
#[test]
fn a_function_lands_past_its_untaken_branch_on_every_call() {
    let body = [0x58, 0xB1, 0, 64, 0x38, 0x1B, 0xB1, 0, 128, 0x38, 0x59];
    let skipped = [0xB1, 0, 0, 0x58, 0xB0, 0, 0x2B, 0x59];
    let program = [&skipped[..], &[0xB3, 1, 1, 0, 0], &[0xB0, 0, 0x2B].repeat(4)].concat();
    assert_eq!(calling(&body, &program).map(|o| o.x[0]), [384; 2]);
}

// IF, ELSE, ELSE and EIF with the IF taken: the first ELSE jumps to the EIF that ends the IF, as the instruction set
// has it, past the second. Point 0 moves by the IF's 1 px alone.
#[test]
fn an_else_skips_a_stray_else_to_its_eif() {
    let body = [0x58, 0xB1, 0, 64, 0x38, 0x1B, 0xB1, 0, 128, 0x38, 0x1B, 0xB1, 0, 192, 0x38, 0x59];
    assert_eq!(calling(&body, &[0xB1, 1, 0, 0x2B]).map(|o| o.x[0]), [64; 2]);
}

// A CVT program fails, leaving its font unhinted, where a push reads past its end, where it pushes past 65,535 values
// whatever maxp's maxStackElements of 16 says, and past its budget: 106 calls of 10,000 opcodes fit, and 107 don't.
#[test]
fn a_program_stops_at_its_end_its_stack_limit_and_its_opcode_limit() {
    let runs = |fpgm: &[u8], prep: &[u8]| {
        let map = tables(&[("maxp", maxp()), ("fpgm", fpgm.to_vec()), ("prep", prep.to_vec())]);
        HintContext::new(&map, 16.into(), 1000).is_some()
    };
    assert_eq!([&[0xB0, 7][..], &[0xB0], &[0xB8, 1]].map(|prep| runs(&[], prep)), [true, false, false]);
    let full = [&[0x40, 255][..], &[0; 255]].concat().repeat(257);
    assert_eq!([full.clone(), [&full[..], &[0xB0, 0]].concat()].map(|prep| runs(&[], &prep)), [true, false]);
    let fpgm = [&[0xB0, 0, 0x2C][..], &[0x00; 9_999], &[0x2D]].concat();
    assert_eq!([106, 107].map(|calls| runs(&fpgm, &[0xB1, calls, 0, 0x2A])), [true, false]);
}

fn words(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

// A 100-unit square with a program that moves nothing, so a glyph holding it hints.
fn square() -> Vec<u8> {
    let mut glyph = words(&[1, 0, 0, 100, 100, 3, 1]);
    glyph.push(0x00);
    glyph.extend([0x01; 4]);
    glyph.extend(words(&[0, 0, 100, 0, 0, 100, 0, -100]));
    glyph
}

// Glyphs one after another in glyf, with their loca.
fn font(glyphs: &[Vec<u8>]) -> (Vec<u8>, Vec<usize>) {
    let mut loca = alloc::vec![0];
    let glyf: Vec<u8> = glyphs.concat();
    loca.extend(glyphs.iter().scan(0, |end, g| { *end += g.len(); Some(*end) }));
    (glyf, loca)
}

// A composite naming glyph `gid` once, at an offset of (dx, dy), with no matrix.
fn holding(gid: i16, dx: i16, dy: i16) -> Vec<u8> {
    words(&[-1, 0, 0, 0, 0, 0x0003, gid, dx, dy])
}

// The square turned a quarter left with SCALED_COMPONENT_OFFSET: its offset (100, 0) turns with it
// to (0, 100), hinted as unhinted. At 16 px and 1,024 units a unit is one 26.6 step, so both compare exactly.
#[test]
fn a_scaled_offset_is_placed_alike_hinted_and_unhinted() {
    let turned = words(&[-1, 0, 0, 0, 0, 0x0883, 1, 100, 0, 0, 0x4000, -0x4000, 0]);
    let (glyf, loca) = font(&[turned, square()]);
    let mut ctx =
        HintContext::new(&tables(&[("maxp", maxp())]), 16.into(), 1024).expect("a context");
    let hinted = ctx.hint_glyph(&glyf, &loca, 0).expect("the composite hints");
    let points = crate::daecore::daetype::outline::glyph_points(&glyf, false, (&loca).into(), 0);
    let (x, y) = points.expect("the composite draws");
    assert_eq!((x, y), (alloc::vec![0.0, -100.0, -100.0, 0.0], alloc::vec![100.0, 100.0, 200.0, 200.0]));
    let want = (&[0, -100, -100, 0][..], &[100, 100, 200, 200][..]);
    assert_eq!((&hinted.x[..], &hinted.y[..]), want, "hinted elsewhere");
}

// Composites 12 to 1,000 deep draw and hint alike; a composite that holds itself does neither.
#[test]
fn composites_nest_as_deep_as_their_glyphs_hinted_and_unhinted() {
    let map = tables(&[("maxp", maxp())]);
    for (depth, nests) in [(12i16, true), (100, true), (101, true), (1_000, true)] {
        let mut glyphs = alloc::vec![square()];
        glyphs.extend((0..depth).map(|k| holding(k, 0, 0)));
        let (glyf, loca) = font(&glyphs);
        let top = depth as u16;
        let mut ctx = HintContext::new(&map, 16.into(), 1024).expect("a context");
        let points = crate::daecore::daetype::outline::glyph_points(&glyf, false, (&loca).into(), top.into());
        let drawn = points.is_ok();
        assert_eq!((drawn, ctx.hint_glyph(&glyf, &loca, top.into()).is_some()), (nests, nests), "{depth} deep");
    }
    let (glyf, loca) = font(&[holding(0, 0, 0)]);
    let mut ctx = HintContext::new(&map, 16.into(), 1024).expect("a context");
    let drawn = crate::daecore::daetype::outline::glyph_points(&glyf, false, (&loca).into(), 0);
    assert!(drawn.is_err() && ctx.hint_glyph(&glyf, &loca, 0).is_none());
}

// A composite of five one-point components, each running function 0 `calls` times and then FLIPRGOFF on its point:
// from one budget, 30 calls of 10,000 opcodes leave the last two unflipped, and 4 of 200,000 the last three.
#[test]
fn a_composite_s_component_programs_share_one_budget() {
    let flipped = |body: &[u8], calls: u8| {
        let fpgm = [&[0xB0, 0, 0x2C][..], body, &[0x2D]].concat();
        let map = tables(&[("maxp", maxp()), ("fpgm", fpgm)]);
        let program = [0xB1, calls, 0, 0x2A, 0xB1, 0, 0, 0x82];
        let point = [&[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8][..], &program, &[0x31]].concat();
        let mut composite = words(&[-1, 0, 0, 0, 0]);
        (0..5).for_each(|k| composite.extend(words(&[if k < 4 { 0x22 } else { 0x02 }, 1, 0])));
        let (glyf, loca) = font(&[composite, point]);
        let mut ctx = HintContext::new(&map, 16.into(), 1000).expect("a context");
        let out = ctx.hint_glyph(&glyf, &loca, 0).expect("the composite hints");
        out.flags.iter().map(|f| f & FLAG_ON_CURVE == 0).collect::<Vec<_>>()
    };
    let opcodes = alloc::vec![0x00; 9_999];
    assert_eq!(flipped(&opcodes, 30), [true, true, true, false, false]);
    let skips = [&[0xB0, 0, 0x58][..], &alloc::vec![0x20; 199_999], &[0x59]].concat();
    assert_eq!(flipped(&skips, 4), [true, true, false, false, false]);
}

// One contour of `n` points at the origin, on the curve, running `program`.
fn points_at_origin(n: usize, program: &[u8]) -> Vec<u8> {
    let mut glyph = words(&[1, 0, 0, 0, 0, n as i16 - 1, program.len() as i16]);
    glyph.extend(program);
    for k in (0..n).step_by(256).map(|at| (n - at).min(256)) {
        glyph.extend(if k == 1 { alloc::vec![0x31] } else { alloc::vec![0x39, (k - 1) as u8] });
    }
    glyph
}

// A glyph of 60,000 points whose program shifts each a pixel in a loop of 23 opcodes a point: 1.38 million opcodes,
// 88 million units, past BASE and within what the points add to it.
#[test]
fn a_glyph_may_spend_in_proportion_to_its_points() {
    let map = tables(&[("maxp", maxp()), ("prep", NATIVE.to_vec())]);
    // 60,000, past PUSHW's reach, under point i: SHPIX i by 64, ten RTGs, then i + 1 and back while under 60,000.
    let start = [0xB8, 0x75, 0x30, 0x20, 0x60, 0xB0, 0];
    let body = [&[0x20, 0xB0, 64, 0x38][..], &[0x18; 10], &[0xB0, 1, 0x60, 0x20, 0xB0, 3, 0x25, 0x50]].concat();
    let back = -(body.len() as i16 + 4);
    let program = [&start[..], &body, &[0xB8], &back.to_be_bytes(), &[0x23, 0x78]].concat();
    let glyf = points_at_origin(60_000, &program);
    let mut ctx = HintContext::new(&map, 16.into(), 1000).expect("a context");
    let out = ctx.hint_glyph(&glyf, &[0, glyf.len()], 0).expect("the glyph hints");
    let unmoved = out.x.iter().position(|&x| x != 64);
    assert_eq!(unmoved, None, "the program stopped before it shifted every point");
}

// A composite of 2,000 one-point components, each running a program, hints every component drawing reads.
#[test]
fn a_composite_hints_as_many_components_as_it_draws() {
    let mut composite = words(&[-1, 0, 0, 0, 0]);
    (0..2_000).for_each(|k| composite.extend(words(&[if k < 1_999 { 0x22 } else { 0x02 }, 1, 0])));
    let (glyf, loca) = font(&[composite, points_at_origin(1, &[0xB0, 0, 0x21])]);
    assert!(crate::daecore::daetype::outline::glyph_points(&glyf, false, (&loca).into(), 0).is_ok());
    let mut ctx = HintContext::new(&tables(&[("maxp", maxp())]), 16.into(), 1000).expect("a context");
    assert_eq!(ctx.hint_glyph(&glyf, &loca, 0).map(|o| o.x.len()), Some(2_000));
}

// Function 0 calls itself, its argument less one, down to 0, and then the glyph shifts point 0: 100 frames deep, it
// does. Calling itself forever, it stops once its frames, 4,096 each new depth, have spent the budget.
#[test]
fn calls_nest_as_deep_as_the_budget_pays_for() {
    let body = [0x20, 0x58, 0xB0, 1, 0x61, 0xB0, 0, 0x2B, 0x1B, 0x21, 0x59];
    let shifted = |depth: u8| calling(&body, &[0xB1, depth, 0, 0x2B, 0xB1, 0, 64, 0x38]).map(|o| o.x[0]);
    assert_eq!(shifted(100), [64; 2], "100 calls deep stopped the program");
    let fpgm = [0xB0, 0, 0x2C, 0xB0, 0, 0x2B, 0x2D];
    let map = tables(&[("maxp", maxp()), ("fpgm", fpgm.to_vec())]);
    let glyf = points_at_origin(2, &[0xB0, 0, 0x2B]);
    let mut ctx = HintContext::new(&map, 16.into(), 1000).expect("a context");
    assert!(ctx.hint_glyph(&glyf, &[0, glyf.len()], 0).is_some());
    let deepest = ctx.vm.frames.len() as u64;
    assert!(deepest > 1_000 && deepest * 4_096 <= interp::BASE + 2 * interp::PER_POINT, "{deepest} frames");
}

// Contour ends out of order, or fewer y than x, draw what they can rather than panic.
#[test]
fn drawing_a_malformed_hinted_outline_does_not_panic() {
    use alloc::vec;
    let mut path = super::super::outline::Path::default();
    let backwards = HintedOutline { x: vec![0; 5], y: vec![0; 5], flags: vec![1; 5], contour_ends: vec![3, 1] };
    draw_hinted(&backwards, &mut path);
    let short = HintedOutline { x: vec![0; 5], y: vec![0; 3], flags: vec![1; 5], contour_ends: vec![4] };
    draw_hinted(&short, &mut path);
}
