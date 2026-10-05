use daegun::{Font, HintMode};

use build::Case;

// Per case: y, x and on-curve flags of the case's glyph under v35, then under v40.
type Points = [&'static [i32]; 6];

// FreeType 2.14.3's points at 16 ppem for each case's font: hinted.ttf at a 1000-unit em, glyph 1
// replaced by a box (or the given points) running the case's program, v35 for Classic, v40 for Subpixel.
const FREETYPE: &[(&str, Points)] = &[
    ("jrot", [&[0, 128, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 128, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("jrof", [&[0, 128, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 128, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("md current", [&[0, 717, -717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, -717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("md original", [&[0, 717, -717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, -717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("md original unscaled", [&[1, 22, -20, 1], &[102, 102, 512, 512], &[1, 1, 1, 1], &[1, 22, -20, 1], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("rthg", [&[0, 717, 32, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 32, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("odd", [&[0, 717, 1, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 1, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("div", [&[0, 717, 42, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 42, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("div by zero", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("fdef in glyph", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 32", [&[0, 717, 4096, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 64", [&[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 8192, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 1024", [&[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 131072, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 4096", [&[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 524288, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mdrp minimum distance", [&[10, -54, 0, 717], &[102, 102, 512, 512], &[1, 1, 1, 1], &[10, -54, 0, 717], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mdrp unscaled", [&[1, 21, 22, 1], &[102, 102, 512, 512], &[1, 1, 1, 1], &[1, 21, 22, 1], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("miap twilight", [&[0, 717, 256, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 256, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mirp twilight", [&[0, 717, 384, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 384, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("miap cvt out of range", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mirp cvt out of range", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mirp cvt -1", [&[0, 0, 704, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 0, 704, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mdrp rp0 out of range", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mdap out of range", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("szp invalid", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("move after iup", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shpix untouched", [&[0, 781, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shpix along freedom", [&[0, 717, 717, 0], &[102, 166, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("iup unscaled", [&[0, 335, 704, 704, 0], &[102, 102, 102, 512, 512], &[1, 1, 1, 1, 1], &[0, 335, 704, 704, 0], &[102, 102, 102, 512, 512], &[1, 1, 1, 1, 1]]),
    ("phantom advance", [&[0, 717, 717, 0], &[102, 102, 576, 522], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 522, 522], &[1, 1, 1, 1]]),
    ("phantom distance", [&[0, 717, -640, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, -640, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("prep vectors", [&[0, 717, 717, 0], &[102, 128, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("prep round", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("prep zone", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("prep twilight", [&[0, 717, 256, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 256, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("twilight size", [&[0, 717, 256, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 256, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("fpgm minimum distance", [&[10, -54, 0, 717], &[102, 102, 512, 512], &[1, 1, 1, 1], &[10, -54, 0, 717], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("fpgm storage", [&[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("fpgm cvt", [&[0, 0, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 0, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("no font programs", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("function past 4096", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("spvtl", [&[0, 753, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 753, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("sfvtl", [&[0, 704, 717, 0], &[102, 95, 512, 512], &[1, 1, 1, 1], &[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("spvfs", [&[0, 717, 755, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 755, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("sfvfs", [&[0, 717, 704, 0], &[102, 102, 508, 512], &[1, 1, 1, 1], &[0, 717, 704, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("sdpvtl", [&[0, 773, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 773, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("isect", [&[0, 359, 717, 0], &[102, 307, 512, 512], &[1, 1, 1, 1], &[0, 359, 717, 0], &[102, 307, 512, 512], &[1, 1, 1, 1]]),
    ("alignpts", [&[358, 359, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[358, 359, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("utp", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("scfs", [&[0, 64, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 64, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("deltap", [&[0, 781, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("deltap touched", [&[0, 781, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 781, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("deltac", [&[0, 64, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 64, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("flippt", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 0, 1, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 0, 1, 1]]),
    ("fliprgoff", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[0, 0, 0, 1], &[0, 717, 717, 0], &[102, 102, 512, 512], &[0, 0, 0, 1]]),
    ("idef", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shc", [&[-13, 704, 704, -13], &[102, 102, 512, 512], &[1, 1, 1, 1], &[-13, 704, 704, -13], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shz", [&[-13, 704, 704, -13], &[102, 102, 512, 512], &[1, 1, 1, 1], &[-13, 704, 704, -13], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shp", [&[0, 704, 704, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 704, 704, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("msirp", [&[0, 64, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 64, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("ip", [&[0, 335, 704, 717, 0], &[102, 102, 102, 512, 512], &[1, 1, 1, 1, 1], &[0, 335, 704, 717, 0], &[102, 102, 102, 512, 512], &[1, 1, 1, 1, 1]]),
    ("alignrp", [&[0, 0, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 0, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("near-axis projection", [&[0, 717, 102, 1], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 102, 1], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("origin at phantom 1", [&[0, 704, 717, 0], &[166, 166, 576, 576], &[1, 1, 1, 1], &[0, 704, 717, 0], &[163, 163, 573, 573], &[1, 1, 1, 1]]),
    ("vertical phantom x", [&[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1], &[0, 717, 307, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("composite", [&[64, 768, 768, 64], &[102, 102, 512, 512], &[1, 1, 1, 1], &[64, 768, 768, 64], &[112, 112, 522, 522], &[1, 1, 1, 1]]),
];

fn base() -> Vec<u8> {
    std::fs::read(format!("{}/test-fixtures/hinted.ttf", crate::FONTS)).expect("hinted.ttf")
}

fn hinted(c: &Case, gid: u16, mode: HintMode) -> Option<daegun::HintedOutline> {
    Font::from_vec(build::font(&base(), c)).expect("the case builds").hinted_glyph(gid, 16.0, &[], mode)
}

#[test]
fn each_instruction_moves_points_as_freetype_does() {
    assert_eq!(build::CASES.len(), FREETYPE.len());
    let mut wrong = Vec::new();
    for ((name, case), (expected_name, want)) in build::CASES.iter().zip(FREETYPE) {
        assert_eq!(name, expected_name);
        let gid = if case.composite.is_some() { 2 } else { 1 };
        for (i, mode) in [HintMode::Classic, HintMode::Subpixel].into_iter().enumerate() {
            let (y, x, flags) = (want[3 * i], want[3 * i + 1], want[3 * i + 2]);
            let got = hinted(case, gid, mode);
            let same = got.as_ref().is_some_and(|o| {
                o.y == y && o.x == x && o.flags.iter().map(|&f| i32::from(f)).eq(flags.iter().copied())
            });
            if !same {
                wrong.push(format!("{name}, {mode:?}: {:?}\n  FreeType y {y:?} x {x:?} flags {flags:?}", got.map(|o| (o.y, o.x, o.flags))));
            }
        }
    }
    assert!(wrong.is_empty(), "{} of {} runs differ from FreeType:\n{}", wrong.len(), 2 * FREETYPE.len(), wrong.join("\n"));
}

// The CVT program can turn glyph programs off (INSTCTRL selector 1), as a font hinted up to a size
// does past it: FreeType then draws the glyph unhinted.
#[test]
fn the_cvt_program_can_turn_glyph_programs_off() {
    let off = Case { prep: &[0xB1, 0x01, 0x01, 0x8E], glyph: &[0x00, 0xB0, 0x01, 0x2F], ..build::BOX };
    for mode in [HintMode::Classic, HintMode::Subpixel] {
        assert!(hinted(&off, 1, mode).is_none(), "{mode:?} ran the glyph program the CVT program switched off");
    }
}

// Arial Unicode's prep pushes 4,105 values against a maxStackElements of 4,139. The stack holds
// half as many again, as FreeType's does; a fixed 4,096 would stop the prep before its stores.
#[test]
fn a_prep_deeper_than_4096_runs_to_its_end() {
    let mut prep: Vec<u8> = Vec::new();
    for _ in 0..17 {
        prep.extend([0x40, 255]);
        prep.extend(std::iter::repeat_n(1u8, 255));
    }
    prep.extend([0xB1, 0x00, 0x07, 0x42]);
    let glyph: &[u8] = &[0x00, 0xB0, 0x00, 0x43, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E];
    let deep = Case { stack: 4139, prep: Box::leak(prep.into_boxed_slice()), glyph, ..build::BOX };
    for mode in [HintMode::Classic, HintMode::Subpixel] {
        assert_eq!(hinted(&deep, 1, mode).expect("hinted").y, [0, 717, 7, 0], "{mode:?}: the prep's store was lost");
    }
}

// A font program that fails leaves the size unhinted, and a glyph whose instruction length runs past
// its own bytes is refused rather than run on the next glyph's; FreeType loads neither.
#[test]
fn a_broken_program_is_not_run() {
    let bad_prep = Case { prep: &[0x28], glyph: &[0x00, 0xB0, 0x01, 0x2F], ..build::BOX };
    let overlong = Case { overlong: 4, glyph: &[0x00, 0xB0, 0x01, 0x2F], ..build::BOX };
    for mode in [HintMode::Classic, HintMode::Subpixel] {
        assert!(hinted(&bad_prep, 1, mode).is_none(), "{mode:?}: hinted after the prep failed");
        assert!(hinted(&overlong, 1, mode).is_none(), "{mode:?}: ran instructions past the glyph");
    }
}

// IUP over 20,000 points in a backward-jump loop: billions of point visits an opcode count alone
// allows, so the work it does stops it.
#[test]
fn an_iup_loop_stops_at_its_work_budget() {
    let points: Vec<(i16, i16)> = (0..20_000).map(|i| (900 - (i % 2) as i16, 400 + (i / 1000) as i16)).collect();
    let glyph: &[u8] = &[0x00, 0xB8, 0x00, 0x00, 0x2E, 0xB8, 0x27, 0x10, 0x2E, 0x30, 0xB8, 0xFF, 0xFC, 0x1C];
    let looping = Case { points: Box::leak(points.into_boxed_slice()), glyph, ..build::BOX };
    let font = Font::from_vec(build::font(&base(), &looping)).expect("builds");
    let started = std::time::Instant::now();
    let _ = font.hinted_glyph(1, 16.0, &[], HintMode::Classic);
    assert!(started.elapsed().as_secs_f64() < 2.0, "hinting took {:?}", started.elapsed());
}

// The bundled Irianis face, whose programs use IP, SHP, SPVFS, SPVTL, ALIGNRP, MIRP and GETINFO,
// against FreeType 2.14.3: every glyph it hints, points and flags, hashed per size and interpreter.
#[test]
fn a_hinted_face_matches_freetype() {
    let bytes = std::fs::read(format!("{}/Irianisadfstd/IrianisadfstdRegular.ttc", crate::FONTS)).expect("Irianis");
    let font = Font::from_ttc(&bytes, 0).expect("face 0");
    let fnv = |h: u64, v: i64| v.to_le_bytes().iter().fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    for (px, mode, count, want) in [
        (12.0, HintMode::Classic, 467, 0x7a83_cc9e_6689_0df9u64),
        (12.0, HintMode::Subpixel, 467, 0x6afc_d8bb_baf8_fbdd),
        (24.0, HintMode::Classic, 467, 0x878a_e6b7_4e46_5091),
        (24.0, HintMode::Subpixel, 467, 0x8722_1554_0033_e790),
    ] {
        let (mut h, mut hinted) = (0xcbf2_9ce4_8422_2325u64, 0);
        for gid in 0..font.num_glyphs() {
            let Some(o) = font.hinted_glyph(gid, px, &[], mode) else { continue };
            assert!(o.flags.iter().all(|&f| f <= 1), "gid {gid}: flags {:?} carry more than on-curve", o.flags);
            hinted += 1;
            h = fnv(h, i64::from(gid));
            for v in o.y.iter().chain(&o.x).map(|&v| i64::from(v)).chain(o.flags.iter().map(|&f| i64::from(f))) {
                h = fnv(h, v);
            }
        }
        assert_eq!((hinted, h), (count, want), "{px} px {mode:?}: hinted glyphs and their hash");
    }
}

// A caller may build a HintedOutline: contour ends out of order, or fewer y than x, draw what they
// can rather than panic.
#[test]
fn drawing_a_malformed_hinted_outline_does_not_panic() {
    let mut path = daegun::Path::default();
    let backwards = daegun::HintedOutline { x: vec![0; 5], y: vec![0; 5], flags: vec![1; 5], contour_ends: vec![3, 1] };
    daegun::draw_hinted(&backwards, &mut path);
    let short = daegun::HintedOutline { x: vec![0; 5], y: vec![0; 3], flags: vec![1; 5], contour_ends: vec![4] };
    daegun::draw_hinted(&short, &mut path);
}

pub(crate) mod build {
    use std::collections::BTreeMap;

    use daegun::daecore::daetype::decoder::{build_ttf, extract_ttf_tables, read_u16_be};

    #[derive(Clone, Copy)]
    pub struct Case {
        pub glyph: &'static [u8],
        pub prep: &'static [u8],
        pub fpgm: &'static [u8],
        pub cvt: &'static [i16],
        pub points: &'static [(i16, i16)],
        pub twilight: u16,
        pub stack: u16,
        pub functions: u16,
        pub font_tables: bool,
        pub idefs: u16,
        // Added to glyph 1's instruction length, past its own bytes.
        pub overlong: u16,
        // Added to glyph 1's left side bearing, which otherwise is its xMin.
        pub lsb: i16,
        // Glyph 2 as a composite of glyph 1: its flags, offset and program.
        pub composite: Option<(u16, i16, i16, &'static [u8])>,
        // How many times the composite places glyph 1, all at the same offset.
        pub copies: u16,
    }

    pub const BOX: Case = Case {
        glyph: &[],
        prep: &[0xB0, 0x00, 0x21],
        fpgm: &[],
        cvt: &[0],
        points: &[(100, 0), (100, 700), (500, 700), (500, 0)],
        twilight: 16,
        stack: 256,
        functions: 64,
        font_tables: true,
        idefs: 0,
        overlong: 0,
        lsb: 0,
        composite: None,
        copies: 1,
    };

    const IUP_POINTS: &[(i16, i16)] = &[(100, 0), (100, 333), (100, 700), (500, 700), (500, 0)];
    const NEAR: &[(i16, i16)] = &[(100, 10), (100, 0), (500, 0), (500, 700)];

    // Most store a measured value in CVT 0 and move point 2 to it (PUSHB 0, SWAP, WCVTP, PUSHB 2 0,
    // MIAP[0]), so the value shows in the outline.
    pub const CASES: &[(&str, Case)] = &[
        ("jrot", Case { glyph: &[0x00, 0xB0, 0x00, 0xB1, 0x05, 0x01, 0x78, 0xB0, 0x40, 0x21, 0x4B, 0xB0, 0x80, 0x60, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x01, 0x00, 0x3E], ..BOX }),
        ("jrof", Case { glyph: &[0x00, 0xB0, 0x00, 0xB1, 0x05, 0x00, 0x79, 0xB0, 0x40, 0x21, 0x4B, 0xB0, 0x80, 0x60, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x01, 0x00, 0x3E], ..BOX }),
        ("md current", Case { glyph: &[0x00, 0xB1, 0x00, 0x01, 0x49, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("md original", Case { glyph: &[0x00, 0xB1, 0x00, 0x01, 0x4A, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("md original unscaled", Case { glyph: &[0x00, 0xB1, 0x00, 0x01, 0x4A, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], points: &[(100, 1), (100, 21), (500, 21), (500, 1)], ..BOX }),
        ("rthg", Case { glyph: &[0x00, 0x19, 0xB0, 0x28, 0x68, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("odd", Case { glyph: &[0x00, 0x7D, 0xB0, 0x7A, 0x56, 0xB0, 0x40, 0x63, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("div", Case { glyph: &[0x00, 0xB9, 0x00, 0x80, 0x00, 0xC0, 0x62, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("div by zero", Case { glyph: &[0x00, 0xB1, 0x01, 0x00, 0x62, 0xB0, 0x01, 0x2F], ..BOX }),
        ("fdef in glyph", Case { glyph: &[0x00, 0xB0, 0x00, 0x2C, 0x2D, 0xB0, 0x01, 0x2F], ..BOX }),
        ("getinfo 32", Case { glyph: &[0x00, 0xB0, 0x20, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("getinfo 64", Case { glyph: &[0x00, 0xB0, 0x40, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("getinfo 1024", Case { glyph: &[0x00, 0xB8, 0x04, 0x00, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("getinfo 4096", Case { glyph: &[0x00, 0xB8, 0x10, 0x00, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("mdrp minimum distance", Case { glyph: &[0x00, 0xB0, 0x40, 0x1A, 0xB0, 0x00, 0x10, 0xB0, 0x01, 0xCC], points: NEAR, ..BOX }),
        ("mdrp unscaled", Case { glyph: &[0x00, 0xB0, 0x00, 0x10, 0xB0, 0x01, 0xC0], points: &[(100, 1), (100, 21), (500, 21), (500, 1)], ..BOX }),
        ("miap twilight", Case { glyph: &[0x00, 0xB0, 0x00, 0x13, 0xB1, 0x00, 0x00, 0x3F, 0xB0, 0x00, 0x15, 0xB0, 0x00, 0x46, 0xB0, 0x01, 0x16, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], cvt: &[250], ..BOX }),
        ("mirp twilight", Case { glyph: &[0x00, 0xB0, 0x00, 0x16, 0xB1, 0x00, 0x00, 0x3E, 0xB1, 0x01, 0x01, 0xE4, 0xB0, 0x01, 0x46, 0xB0, 0x01, 0x16, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], cvt: &[250, 100], ..BOX }),
        ("miap cvt out of range", Case { glyph: &[0x00, 0xB1, 0x01, 0x32, 0x3E], cvt: &[100], ..BOX }),
        ("mirp cvt out of range", Case { glyph: &[0x00, 0xB0, 0x00, 0x10, 0xB1, 0x01, 0x32, 0xE0], cvt: &[100], ..BOX }),
        ("mirp cvt -1", Case { glyph: &[0x00, 0xB0, 0x00, 0x10, 0xB9, 0x00, 0x01, 0xFF, 0xFF, 0xE0, 0xB0, 0x02, 0x2F], ..BOX }),
        ("mdrp rp0 out of range", Case { glyph: &[0x00, 0xB0, 0x32, 0x10, 0xB0, 0x01, 0xC4], ..BOX }),
        ("mdap out of range", Case { glyph: &[0x00, 0xB0, 0x01, 0x10, 0xB0, 0x32, 0x2E, 0xB0, 0x02, 0xC4], ..BOX }),
        ("szp invalid", Case { glyph: &[0x00, 0xB0, 0x00, 0x13, 0xB0, 0x02, 0x13, 0xB0, 0x01, 0x2F], ..BOX }),
        ("move after iup", Case { glyph: &[0x00, 0xB0, 0x00, 0x2E, 0x30, 0x31, 0xB0, 0x01, 0x2F], ..BOX }),
        ("shpix untouched", Case { glyph: &[0x00, 0xB1, 0x01, 0x40, 0x38], ..BOX }),
        ("shpix along freedom", Case { glyph: &[0x02, 0x05, 0xB1, 0x01, 0x40, 0x38], ..BOX }),
        ("iup unscaled", Case { glyph: &[0x00, 0xB0, 0x00, 0x2E, 0xB0, 0x02, 0x2F, 0x30], points: IUP_POINTS, ..BOX }),
        ("phantom advance", Case { glyph: &[0x01, 0xB0, 0x05, 0x10, 0xB0, 0x02, 0xC4], points: &[(100, 0), (100, 700), (510, 700), (510, 0)], ..BOX }),
        ("phantom distance", Case { glyph: &[0x01, 0xB1, 0x04, 0x05, 0x49, 0xB0, 0x00, 0x23, 0x44, 0x00, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("prep vectors", Case { prep: &[0x00], glyph: &[0xB0, 0x01, 0x2F], ..BOX }),
        ("prep round", Case { prep: &[0x7A], glyph: &[0x00, 0xB0, 0x01, 0x2F], ..BOX }),
        ("prep zone", Case { prep: &[0xB0, 0x00, 0x16], glyph: &[0x00, 0xB0, 0x01, 0x2F], ..BOX }),
        ("prep twilight", Case { prep: &[0x00, 0xB0, 0x00, 0x13, 0xB1, 0x00, 0x00, 0x3E], glyph: &[0x00, 0xB0, 0x00, 0x15, 0xB0, 0x00, 0x46, 0xB0, 0x01, 0x16, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], cvt: &[250], ..BOX }),
        ("twilight size", Case { twilight: 50, glyph: &[0x00, 0xB0, 0x00, 0x13, 0xB1, 0x14, 0x00, 0x3E, 0xB0, 0x00, 0x15, 0xB0, 0x14, 0x46, 0xB0, 0x01, 0x16, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], cvt: &[250], ..BOX }),
        ("fpgm minimum distance", Case { fpgm: &[0xB0, 0x80, 0x1A], glyph: &[0x00, 0xB0, 0x00, 0x10, 0xB0, 0x01, 0xC8], points: NEAR, ..BOX }),
        ("fpgm storage", Case { fpgm: &[0xB1, 0x00, 0x64, 0x42], glyph: &[0x00, 0xB0, 0x00, 0x43, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("fpgm cvt", Case { fpgm: &[0xB1, 0x00, 0x7B, 0x44], glyph: &[0x00, 0xB1, 0x01, 0x00, 0x3E], ..BOX }),
        ("no font programs", Case { font_tables: false, glyph: &[0x00, 0xB0, 0x01, 0x2F], ..BOX }),
        ("function past 4096", Case { functions: 1, fpgm: &[0xB8, 0x13, 0x88, 0x2C, 0xB0, 0x01, 0x2F, 0x2D], glyph: &[0x00, 0xB8, 0x13, 0x88, 0x2B], ..BOX }),
        ("spvtl", Case { glyph: &[0x04, 0xB1, 0x00, 0x02, 0x06, 0xB0, 0x01, 0x2F], ..BOX }),
        ("sfvtl", Case { glyph: &[0x02, 0xB1, 0x02, 0x00, 0x08, 0xB0, 0x01, 0x2F], ..BOX }),
        ("spvfs", Case { glyph: &[0x04, 0xB9, 0x20, 0x00, 0x20, 0x00, 0x0A, 0xB0, 0x02, 0x2F], ..BOX }),
        ("sfvfs", Case { glyph: &[0x02, 0xB9, 0x10, 0x00, 0x30, 0x00, 0x0B, 0xB0, 0x02, 0x2F], ..BOX }),
        ("sdpvtl", Case { glyph: &[0xB1, 0x02, 0x00, 0x87, 0x04, 0xB0, 0x00, 0x10, 0xB0, 0x01, 0xC4], ..BOX }),
        ("isect", Case { glyph: &[0xB4, 0x01, 0x00, 0x02, 0x03, 0x01, 0x0F], ..BOX }),
        ("alignpts", Case { glyph: &[0x00, 0xB1, 0x00, 0x01, 0x27], ..BOX }),
        ("utp", Case { glyph: &[0x00, 0xB0, 0x00, 0x2E, 0xB0, 0x01, 0x2F, 0xB0, 0x01, 0x29, 0x30], ..BOX }),
        ("scfs", Case { glyph: &[0x00, 0xB1, 0x01, 0x40, 0x48], ..BOX }),
        ("deltap", Case { glyph: &[0x00, 0xB2, 0x7F, 0x01, 0x01, 0x5D], ..BOX }),
        ("deltap touched", Case { glyph: &[0x00, 0xB0, 0x01, 0x2E, 0xB2, 0x7F, 0x01, 0x01, 0x5D], ..BOX }),
        ("deltac", Case { glyph: &[0x00, 0xB2, 0x7F, 0x00, 0x01, 0x73, 0xB1, 0x01, 0x00, 0x3E], ..BOX }),
        ("flippt", Case { glyph: &[0xB0, 0x01, 0x80], ..BOX }),
        ("fliprgoff", Case { glyph: &[0xB1, 0x00, 0x02, 0x82], ..BOX }),
        ("idef", Case { idefs: 1, fpgm: &[0xB0, 0x93, 0x89, 0xB0, 0x01, 0x2F, 0x2D], glyph: &[0x00, 0x93], ..BOX }),
        ("shc", Case { glyph: &[0x00, 0xB0, 0x01, 0x2F, 0xB0, 0x00, 0x35], ..BOX }),
        ("shz", Case { glyph: &[0x00, 0xB0, 0x01, 0x2F, 0xB0, 0x01, 0x37], ..BOX }),
        ("shp", Case { glyph: &[0x00, 0xB0, 0x01, 0x2F, 0xB0, 0x02, 0x33], ..BOX }),
        ("msirp", Case { glyph: &[0x00, 0xB0, 0x00, 0x10, 0xB1, 0x01, 0x40, 0x3A], ..BOX }),
        ("ip", Case { glyph: &[0x00, 0xB0, 0x00, 0x2E, 0xB0, 0x02, 0x2F, 0xB0, 0x00, 0x11, 0xB0, 0x02, 0x12, 0xB0, 0x01, 0x39], points: IUP_POINTS, ..BOX }),
        ("alignrp", Case { glyph: &[0x00, 0xB0, 0x00, 0x2F, 0xB0, 0x01, 0x3C], ..BOX }),
        ("near-axis projection", Case { glyph: &[0xB1, 0x03, 0x00, 0x06, 0xB0, 0x01, 0x46, 0xB0, 0x00, 0x23, 0x44, 0x00, 0xB1, 0x02, 0x00, 0x3E], points: &[(100, 0), (100, 700), (500, 700), (500, 1)], ..BOX }),
        ("origin at phantom 1", Case { lsb: 60, glyph: &[0x00, 0xB0, 0x01, 0x2F], ..BOX }),
        ("vertical phantom x", Case { glyph: &[0x01, 0xB0, 0x06, 0x46, 0xB0, 0x00, 0x23, 0x44, 0x00, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        ("composite", Case { glyph: &[0x00, 0xB0, 0x01, 0x2F], composite: Some((0x0004, 10, 37, &[0x00, 0xB0, 0x02, 0x2F])), ..BOX }),
    ];

    fn be(v: &[i16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }

    fn simple(points: &[(i16, i16)], program: &[u8], overlong: u16) -> Vec<u8> {
        let (xs, ys): (Vec<i16>, Vec<i16>) = points.iter().copied().unzip();
        let bbox = [*xs.iter().min().unwrap(), *ys.iter().min().unwrap(), *xs.iter().max().unwrap(), *ys.iter().max().unwrap()];
        let mut g = be(&[1, bbox[0], bbox[1], bbox[2], bbox[3], (points.len() - 1) as u16 as i16]);
        g.extend((program.len() as u16 + overlong).to_be_bytes());
        g.extend(program);
        g.extend(std::iter::repeat_n(0x01u8, points.len()));
        let delta = |v: &[i16]| v.iter().scan(0, |prev, &x| { let d = x - *prev; *prev = x; Some(d) }).collect::<Vec<_>>();
        g.extend(be(&delta(&xs)));
        g.extend(be(&delta(&ys)));
        g
    }

    fn composite(flags: u16, dx: i16, dy: i16, program: &[u8], copies: u16) -> Vec<u8> {
        let mut g = be(&[-1, 0, 0, 0, 0]);
        for i in 1..=copies {
            let more = if i < copies { 0x0020 } else if program.is_empty() { 0 } else { 0x0100 };
            g.extend((flags | 0x0001 | 0x0002 | more).to_be_bytes());
            g.extend(1u16.to_be_bytes());
            g.extend(be(&[dx, dy]));
        }
        if !program.is_empty() {
            g.extend((program.len() as u16).to_be_bytes());
            g.extend(program);
        }
        g
    }

    pub fn font(base: &[u8], c: &Case) -> Vec<u8> {
        let mut map: BTreeMap<String, Vec<u8>> =
            extract_ttf_tables(base).unwrap().into_iter().map(|(t, d)| (t, d.to_vec())).collect();
        let n = usize::from(read_u16_be(&map["maxp"], 4).unwrap());
        let long = usize::from(read_u16_be(&map["hhea"], 34).unwrap());
        let loca_long = map["head"][51] == 1;
        let old = |gid: usize| -> Vec<u8> {
            let at = |i: usize| if loca_long {
                u32::from_be_bytes(map["loca"][4 * i..4 * i + 4].try_into().unwrap()) as usize
            } else {
                2 * usize::from(u16::from_be_bytes(map["loca"][2 * i..2 * i + 2].try_into().unwrap()))
            };
            map["glyf"][at(gid)..at(gid + 1)].to_vec()
        };
        let mut glyphs: Vec<Vec<u8>> = (0..n).map(old).collect();
        glyphs[1] = simple(c.points, c.glyph, c.overlong);
        if let Some((flags, dx, dy, program)) = c.composite {
            glyphs[2] = composite(flags, dx, dy, program, c.copies);
        }
        let (mut glyf, mut loca) = (Vec::new(), Vec::new());
        for g in &glyphs {
            loca.extend((glyf.len() as u32).to_be_bytes());
            glyf.extend(g);
            while glyf.len() % 4 != 0 {
                glyf.push(0);
            }
        }
        loca.extend((glyf.len() as u32).to_be_bytes());
        let metric = |gid: usize| -> (u16, i16) {
            let hmtx = &map["hmtx"];
            let advance = u16::from_be_bytes(hmtx[4 * gid.min(long - 1)..][..2].try_into().unwrap());
            let at = if gid < long { 4 * gid + 2 } else { 4 * long + 2 * (gid - long) };
            (advance, i16::from_be_bytes(hmtx[at..at + 2].try_into().unwrap()))
        };
        let mut hmtx = Vec::new();
        for gid in 0..n {
            let (advance, lsb) = match gid {
                1 => (600, c.points.iter().map(|p| p.0).min().unwrap() + c.lsb),
                2 if c.composite.is_some() => (600, 0),
                _ => metric(gid),
            };
            hmtx.extend(advance.to_be_bytes());
            hmtx.extend(lsb.to_be_bytes());
        }
        map.get_mut("hhea").unwrap()[34..36].copy_from_slice(&(n as u16).to_be_bytes());
        let head = map.get_mut("head").unwrap();
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[50..52].copy_from_slice(&1i16.to_be_bytes());
        let maxp = map.get_mut("maxp").unwrap();
        for (at, v) in [(6, 64u16), (8, 4), (14, 2), (16, c.twilight), (18, 64), (20, c.functions), (22, c.idefs), (24, c.stack), (26, 8000)] {
            maxp[at..at + 2].copy_from_slice(&v.to_be_bytes());
        }
        map.insert("glyf".into(), glyf);
        map.insert("loca".into(), loca);
        map.insert("hmtx".into(), hmtx);
        if c.font_tables {
            map.insert("cvt ".into(), be(c.cvt));
            map.insert("fpgm".into(), c.fpgm.to_vec());
            map.insert("prep".into(), c.prep.to_vec());
        } else {
            for tag in ["cvt ", "fpgm", "prep"] {
                map.remove(tag);
            }
        }
        build_ttf(&map)
    }
}
