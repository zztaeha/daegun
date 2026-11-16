use crate::Hint;
use daegun::outline::{HintMode, HintedOutline, PointKind};
use daegun::{Font, GlyphId, Location, Tag};

use build::Case;

// Per case: y, x and on-curve flags of the case's glyph.
type Points = [&'static [i32]; 3];

// daegun's recorded points for each case's font at 16 ppem: hinted.ttf at a 1000-unit em, glyph 1 replaced by a box
// (or the given points) running the case's program under the font's own hints.
const EXPECTED_POINTS: &[(&str, Points)] = &[
    ("jrot", [&[0, 128, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("jrof", [&[0, 128, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("md current", [&[0, 717, -717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("md original", [&[0, 717, -717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("md original unscaled", [&[1, 22, -20, 1], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("rthg", [&[0, 717, 32, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("odd", [&[0, 717, 1, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("div", [&[0, 717, 42, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("div by zero", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("fdef in glyph", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    // Version 42: font variations (41) and GETVARIATION (42).
    ("getinfo 1", [&[0, 717, 42, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 16", [&[0, 717, 2048, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 32", [&[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 64", [&[0, 717, 8192, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 1024", [&[0, 717, 131072, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 4096", [&[0, 717, 524288, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mdrp minimum distance", [&[10, -54, 0, 717], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mdrp unscaled", [&[1, 21, 22, 1], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("miap twilight", [&[0, 717, 256, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mirp twilight", [&[0, 717, 384, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("miap cvt out of range", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mirp cvt out of range", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    // CVT entry -1 is outside the CVT, so MIRP leaves point 1 where it is.
    ("mirp cvt -1", [&[0, 717, 704, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mdrp rp0 out of range", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("mdap out of range", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("szp invalid", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("move after iup", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shpix untouched", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shpix along freedom", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("iup unscaled", [&[0, 335, 704, 704, 0], &[102, 102, 102, 512, 512], &[1, 1, 1, 1, 1]]),
    ("phantom advance", [&[0, 717, 717, 0], &[102, 102, 522, 522], &[1, 1, 1, 1]]),
    // Where x may not move the phantom points keep their scaled x: an advance of 614/64, unrounded.
    ("phantom distance", [&[0, 717, -614, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("prep vectors", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("prep round", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("prep zone", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("prep twilight", [&[0, 717, 256, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("twilight size", [&[0, 717, 256, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("fpgm minimum distance", [&[10, -54, 0, 717], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("fpgm storage", [&[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("fpgm cvt", [&[0, 0, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("no font programs", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("function past 4096", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("spvtl", [&[0, 753, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("sfvtl", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("spvfs", [&[0, 717, 755, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("sfvfs", [&[0, 717, 704, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("sdpvtl", [&[0, 773, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    // In backward compatibility ISECT moves its point along y alone.
    ("isect", [&[0, 359, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("alignpts", [&[358, 359, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    // IUP places an untouched point from its original position, the one touched point having not moved.
    ("utp", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("scfs", [&[0, 64, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("deltap", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("deltap touched", [&[0, 781, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("deltac", [&[0, 64, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("flippt", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 0, 1, 1]]),
    ("fliprgoff", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[0, 0, 0, 1]]),
    ("idef", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shc", [&[-13, 704, 704, -13], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shz", [&[-13, 704, 704, -13], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shp", [&[0, 704, 704, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("msirp", [&[0, 64, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("ip", [&[0, 335, 704, 717, 0], &[102, 102, 102, 512, 512], &[1, 1, 1, 1, 1]]),
    ("alignrp", [&[0, 0, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("near-axis projection", [&[0, 717, 102, 1], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("origin at phantom 1", [&[0, 704, 717, 0], &[163, 163, 573, 573], &[1, 1, 1, 1]]),
    // Phantom points 3 and 4 sit at x 0, the outline's origin.
    ("vertical phantom x", [&[0, 717, 0, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("composite", [&[64, 768, 768, 64], &[112, 112, 522, 522], &[1, 1, 1, 1]]),
    ("mps", [&[0, 717, 1024, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("getinfo 2048", [&[0, 717, 262144, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("native mdap", [&[0, 717, 717, 0], &[128, 102, 512, 512], &[1, 1, 1, 1]]),
    ("native round", [&[0, 717, 128, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("native minimum distance", [&[0, 717, 717, 0], &[102, 166, 512, 512], &[1, 1, 1, 1]]),
    ("native cut-in", [&[0, 717, 717, 0], &[128, 102, 512, 512], &[1, 1, 1, 1]]),
    ("native in y", [&[0, 704, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("native mirp", [&[0, 717, 717, 0], &[102, 166, 512, 512], &[1, 1, 1, 1]]),
    ("native mirp round", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("native mirp cut-in", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("native mdrp round", [&[0, 717, 717, 0], &[102, 102, 512, 486], &[1, 1, 1, 1]]),
    ("horizontal projection", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    // The freedom vector exactly rounded to 2.14 moves the point a 64th less.
    ("near-orthogonal freedom", [&[0, 717, 717, -1911], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("nearly perpendicular freedom", [&[0, 717, 717, 0], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("near-axis freedom", [&[0, 717, 717, 1], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("near-vertical freedom", [&[0, 717, 0, 0], &[102, 103, 512, 512], &[1, 1, 1, 1]]),
    ("twilight near-axis freedom", [&[0, 717, 0, 1], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
    ("shz named zone", [&[64, 781, 781, 64], &[102, 102, 512, 512], &[1, 1, 1, 1]]),
];

fn base() -> Vec<u8> {
    std::fs::read(format!("{}/test-fixtures/hinted.ttf", crate::FONTS)).expect("hinted.ttf")
}

fn hinted(c: &Case, gid: u32, mode: HintMode) -> Option<HintedOutline> {
    Font::from_vec(build::font(&base(), c), 0).expect("the case builds").hinted(
        GlyphId(gid),
        &Location::default(),
        16.0,
        mode,
    )
}

// The points as the expectations hold them: y and x in 26.6, then the flags, 1 on the curve and 2 a cubic control.
fn f26dot6(o: &HintedOutline) -> (Vec<i32>, Vec<i32>, Vec<i32>) {
    let at = |v: f32| (v * 64.0) as i32;
    let flag = |k| match k {
        PointKind::OnCurve => 1,
        PointKind::Cubic => 2,
        PointKind::Quadratic => 0,
    };
    (
        o.points().map(|(p, _)| at(p.y)).collect(),
        o.points().map(|(p, _)| at(p.x)).collect(),
        o.points().map(|(_, k)| flag(k)).collect(),
    )
}

#[test]
fn each_instruction_moves_points_to_their_expected_place() {
    assert_eq!(build::CASES.len(), EXPECTED_POINTS.len());
    let mut wrong = Vec::new();
    for ((name, case), (expected_name, want)) in build::CASES.iter().zip(EXPECTED_POINTS) {
        assert_eq!(name, expected_name);
        let gid = if case.composite.is_some() { 2 } else { 1 };
        let [y, x, flags] = *want;
        let got = hinted(case, gid, HintMode::Font).map(|o| f26dot6(&o));
        if got.as_ref().is_none_or(|(gy, gx, gf)| (&gy[..], &gx[..], &gf[..]) != (y, x, flags)) {
            wrong.push(format!("{name}: {got:?}\n  want y {y:?} x {x:?} flags {flags:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} of {} runs differ from the expected points:\n{}",
        wrong.len(),
        EXPECTED_POINTS.len(),
        wrong.join("\n")
    );
}

// The CVT program can turn glyph programs off (INSTCTRL selector 1), as a font hinted up to a size
// does past it: the glyph is then drawn unhinted.
#[test]
fn the_cvt_program_can_turn_glyph_programs_off() {
    let off = Case { prep: &[0xB1, 0x01, 0x01, 0x8E], glyph: &[0x00, 0xB0, 0x01, 0x2F], ..build::BOX };
    assert!(hinted(&off, 1, HintMode::Font).is_none(), "ran the glyph program the CVT program switched off");
    let own = Case { fpgm: &[0xB0, 0x00, 0x21], ..off };
    assert!(hinted(&own, 1, HintMode::Auto).is_none(), "Auto hinted a glyph its own font turned its programs off for");
}

// Auto autohints a whole font with no font program and a CVT program of at most 7 bytes, glyph programs or not, as such
// programs set up nothing to hint with; any other font is left to its programs. 7 bytes is a dropout setting alone.
#[test]
fn auto_autohints_a_font_whose_programs_hint_nothing() {
    let glyph: &[u8] = &[0x00, 0xB0, 0x01, 0x2F];
    let dropout: &[u8] = &[0xB8, 0x01, 0xFF, 0x85, 0xB0, 0x04, 0x8D];
    let autohinted = [
        ("a 3-byte prep", Case { glyph, ..build::BOX }),
        ("a 7-byte prep", Case { glyph, prep: dropout, ..build::BOX }),
        ("no fpgm, prep or cvt", Case { glyph, font_tables: false, ..build::BOX }),
        ("a prep turning glyph programs off", Case { glyph, prep: &[0xB1, 0x01, 0x01, 0x8E], ..build::BOX }),
    ];
    for (name, case) in &autohinted {
        let forced = hinted(case, 1, HintMode::AutoForce).unwrap_or_else(|| panic!("{name}: AutoForce hinted nothing"));
        assert_ne!(hinted(case, 1, HintMode::Font).as_ref(), Some(&forced), "{name}: the hinters agree");
        assert_eq!(hinted(case, 1, HintMode::Auto), Some(forced), "{name}: Auto didn't autohint");
    }
    let own = [
        ("an 8-byte prep", Case { glyph, prep: &[0xB8, 0x01, 0xFF, 0x85, 0xB0, 0x04, 0x8D, 0x00], ..build::BOX }),
        ("a 3-byte fpgm", Case { glyph, fpgm: &[0xB0, 0x00, 0x21], ..build::BOX }),
    ];
    for (name, case) in &own {
        let bytecode = hinted(case, 1, HintMode::Font).unwrap_or_else(|| panic!("{name}: no bytecode ran"));
        assert_ne!(hinted(case, 1, HintMode::AutoForce).as_ref(), Some(&bytecode), "{name}: the hinters agree");
        assert_eq!(hinted(case, 1, HintMode::Auto), Some(bytecode), "{name}: Auto didn't run the font's programs");
    }
}

// A prep pushing 4,335 values past its maxStackElements of 4,139 runs to its stores, as the stack holds 65,535 whatever
// maxp declares; a fixed 4,096 would stop it first.
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
    let y = f26dot6(&hinted(&deep, 1, HintMode::Font).expect("hinted")).0;
    assert_eq!(y, [0, 717, 7, 0], "the prep's store was lost");
}

// Hinting reads maxp's fields from MAXP, a byte on past its 24-bit count: an IDEF hints under a maxInstructionDefs
// of 1 and fails under 0, from MAXP as from maxp; read a byte early, 0 would be 16,384.
#[test]
fn hinting_reads_maxp_s_fields_from_its_uppercase_twin() {
    for idefs in [0, 1] {
        let case = Case { fpgm: &[0xB0, 0xA0, 0x89, 0x2D], glyph: &[0x00, 0xB0, 0x01, 0x2F], idefs, ..build::BOX };
        let font = Font::from_vec(build::font(&base(), &case), 0).expect("the case builds");
        let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
            font.table_tags().map(|t| (t, font.table(t).expect("listed").to_vec())).collect();
        let maxp = tables[&Tag::new(b"maxp")].clone();
        tables.insert(Tag::new(b"MAXP"), [&maxp[..4], &[0], &maxp[4..]].concat());
        let twin = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the twin parses");
        let want = font.hinted(GlyphId(1), &Location::default(), 16.0, HintMode::Font);
        assert_eq!(want.is_some(), idefs == 1, "{idefs} IDEFs declared");
        assert_eq!(twin.hinted(GlyphId(1), &Location::default(), 16.0, HintMode::Font), want, "{idefs} IDEFs");
    }
}

// A font program that fails leaves the size unhinted, and a glyph whose instruction length runs past
// its own bytes is refused rather than run on the next glyph's; neither comes back hinted.
#[test]
fn a_broken_program_is_not_run() {
    let bad_prep = Case { prep: &[0x28], glyph: &[0x00, 0xB0, 0x01, 0x2F], ..build::BOX };
    let overlong = Case { overlong: 4, glyph: &[0x00, 0xB0, 0x01, 0x2F], ..build::BOX };
    assert!(hinted(&bad_prep, 1, HintMode::Font).is_none(), "hinted after the prep failed");
    assert!(hinted(&overlong, 1, HintMode::Font).is_none(), "ran instructions past the glyph");
}

// A glyph program's CVT, storage and twilight writes end with its glyph: a case hinted twice comes out alike, as do
// reads before writes. Within a glyph, loaded once with its components, a component's CVT write reaches the composite.
#[test]
fn a_glyph_program_leaves_nothing_to_the_next() {
    // Each moves point 2 to the value it reads (CVT 0, then MIAP[0]) and writes it back 64 further on.
    let storage = Case {
        glyph: &[
            0x00, 0xB0, 0x00, 0x43, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E,
            0xB0, 0x00, 0xB0, 0x00, 0x43, 0xB0, 0x40, 0x60, 0x42,
        ],
        ..build::BOX
    };
    let twilight = Case {
        glyph: &[
            0x00, 0xB0, 0x00, 0x15, 0xB0, 0x00, 0x46, 0xB0, 0x01, 0x15, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E,
            0xB0, 0x00, 0x15, 0xB1, 0x00, 0x40, 0x38,
        ],
        ..build::BOX
    };
    for (name, case) in build::CASES.iter().copied().chain([("storage", storage), ("twilight", twilight)]) {
        let font = Font::from_vec(build::font(&base(), &case), 0).expect("the case builds");
        let gid = if case.composite.is_some() { 2 } else { 1 };
        let first = font.hinted(GlyphId(gid), &Location::default(), 16.0, HintMode::Font);
        assert_eq!(
            font.hinted(GlyphId(gid), &Location::default(), 16.0, HintMode::Font),
            first,
            "{name}: moved when hinted again"
        );
    }
    let carried = Case {
        glyph: &[0xB1, 0x00, 0x40, 0x44],
        composite: Some((0, 0, 0, &[0x00, 0xB1, 0x02, 0x00, 0x3E])),
        ..build::BOX
    };
    let y = f26dot6(&hinted(&carried, 2, HintMode::Font).expect("the composite hints")).0;
    assert_eq!(y[2], 64, "the composite didn't read the CVT its component wrote: {y:?}");
}

// IUP over 20,000 points in a backward-jump loop: billions of point visits an opcode count alone
// allows, so the work it does stops it.
#[test]
fn an_iup_loop_stops_at_its_work_budget() {
    let points: Vec<(i16, i16)> = (0..20_000).map(|i| (900 - (i % 2) as i16, 400 + (i / 1000) as i16)).collect();
    let glyph: &[u8] = &[0x00, 0xB8, 0x00, 0x00, 0x2E, 0xB8, 0x27, 0x10, 0x2E, 0x30, 0xB8, 0xFF, 0xFC, 0x1C];
    let looping = Case { points: Box::leak(points.into_boxed_slice()), glyph, ..build::BOX };
    let font = Font::from_vec(build::font(&base(), &looping), 0).expect("builds");
    let started = std::time::Instant::now();
    let _ = font.hinted(GlyphId(1), &Location::default(), 16.0, HintMode::Font);
    assert!(started.elapsed().as_secs_f64() < 2.0, "hinting took {:?}", started.elapsed());
}

// The bundled Irianis face, whose programs use IP, SHP, SPVFS, SPVTL, ALIGNRP, MIRP and GETINFO, pinned to
// daegun's recorded output: every glyph it hints, points and flags, hashed per size.
#[test]
fn a_hinted_face_matches_its_recorded_points() {
    let bytes = std::fs::read(format!("{}/Irianisadfstd/IrianisadfstdRegular.ttc", crate::FONTS)).expect("Irianis");
    let font = Font::from_collection(&bytes, 0).expect("face 0");
    let fnv = |h: u64, v: i64| v.to_le_bytes().iter().fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3));
    for (px, count, want) in [
        (12.0, 467, 0xd8ed_58b2_bd8a_9ca5u64),
        (24.0, 467, 0x6660_004b_d94f_1703),
    ] {
        let (mut h, mut hinted) = (0xcbf2_9ce4_8422_2325u64, 0);
        for gid in 0..font.glyph_count() {
            let Some(o) = font.hinted(GlyphId(gid), &Location::default(), px, HintMode::Font) else { continue };
            let (y, x, flags) = f26dot6(&o);
            assert!(flags.iter().all(|&f| f <= 1), "gid {gid}: flags {flags:?} carry more than on-curve");
            hinted += 1;
            h = fnv(h, i64::from(gid));
            for v in y.into_iter().chain(x).chain(flags).map(i64::from) {
                h = fnv(h, v);
            }
        }
        assert_eq!((hinted, h), (count, want), "{px} px: hinted glyphs and their hash");
    }
}

// With no program, a hinted load still rounds a composite's offsets along y; x keeps the origin and the offsets, as
// a font without native ClearType moves nothing there. Inter has no programs, at 16 px.
#[test]
fn a_glyph_without_a_program_is_still_rounded_along_y() {
    let path = format!("{}/inter/InterVariable.ttf", crate::FONTS);
    let font = Font::from_vec(std::fs::read(&path).expect("Inter"), 0).expect("parses");
    let points = |f: &Font, gid, at: usize| {
        f.hinted(GlyphId(gid), &Location::default(), 16.0, HintMode::Font)
            .map(|o| f26dot6(&o))
            .map(|(y, x, _)| (x[at], y[at]))
    };
    // Eta with dasia: its breathing mark 30/64 px down to the grid.
    assert_eq!(points(&font, 36, 18), Some((171, 564)));
    // A with its left side bearing 10 units larger: its origin 5/64 px off the grid, which stays.
    let mut tables: std::collections::BTreeMap<Tag, Vec<u8>> =
        font.table_tags().map(|t| (t, font.table(t).expect("listed").to_vec())).collect();
    let hmtx = tables.get_mut(&Tag::new(b"hmtx")).expect("hmtx");
    let lsb = i16::from_be_bytes([hmtx[10], hmtx[11]]) + 10;
    hmtx[10..12].copy_from_slice(&lsb.to_be_bytes());
    let moved = Font::from_vec(daegun::raw::build_font(&tables).unwrap(), 0).expect("the patched font parses");
    assert_eq!(points(&moved, 2, 0), None, "the origin is kept, so nothing to round");
}

pub(crate) mod build {
    use std::collections::BTreeMap;

    use daegun::{Font, Tag};

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
    // Freedom vectors one 26.6 unit off an axis over the box's width or height, which normalize onto it.
    const RISING: &[(i16, i16)] = &[(100, 0), (100, 700), (500, 700), (500, 1)];
    const LEANING: &[(i16, i16)] = &[(100, 0), (101, 700), (500, 700), (500, 0)];
    // A CVT program declaring native ClearType (INSTCTRL selector 3), so x moves by the rules y does.
    const NATIVE: &[u8] = &[0xB1, 0x04, 0x03, 0x8E];

    // Most store a measured value in CVT 0 and move point 2 to it (PUSHB 0, SWAP, WCVTP, PUSHB 2 0,
    // MIAP[0]), so the value shows in the outline.
    pub const CASES: &[(&str, Case)] = &[
        (
            "jrot",
            Case {
                glyph: &[
                    0x00, 0xB0, 0x00, 0xB1, 0x05, 0x01, 0x78, 0xB0, 0x40, 0x21, 0x4B, 0xB0, 0x80, 0x60, 0xB0, 0x00,
                    0x23, 0x44, 0xB1, 0x01, 0x00, 0x3E,
                ],
                ..BOX
            },
        ),
        (
            "jrof",
            Case {
                glyph: &[
                    0x00, 0xB0, 0x00, 0xB1, 0x05, 0x00, 0x79, 0xB0, 0x40, 0x21, 0x4B, 0xB0, 0x80, 0x60, 0xB0, 0x00,
                    0x23, 0x44, 0xB1, 0x01, 0x00, 0x3E,
                ],
                ..BOX
            },
        ),
        (
            "md current",
            Case { glyph: &[0x00, 0xB1, 0x00, 0x01, 0x49, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "md original",
            Case { glyph: &[0x00, 0xB1, 0x00, 0x01, 0x4A, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "md original unscaled",
            Case {
                glyph: &[0x00, 0xB1, 0x00, 0x01, 0x4A, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E],
                points: &[(100, 1), (100, 21), (500, 21), (500, 1)],
                ..BOX
            },
        ),
        (
            "rthg",
            Case { glyph: &[0x00, 0x19, 0xB0, 0x28, 0x68, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "odd",
            Case {
                glyph: &[
                    0x00, 0x7D, 0xB0, 0x7A, 0x56, 0xB0, 0x40, 0x63, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E,
                ],
                ..BOX
            },
        ),
        (
            "div",
            Case {
                glyph: &[0x00, 0xB9, 0x00, 0x80, 0x00, 0xC0, 0x62, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E],
                ..BOX
            },
        ),
        ("div by zero", Case { glyph: &[0x00, 0xB1, 0x01, 0x00, 0x62, 0xB0, 0x01, 0x2F], ..BOX }),
        ("fdef in glyph", Case { glyph: &[0x00, 0xB0, 0x00, 0x2C, 0x2D, 0xB0, 0x01, 0x2F], ..BOX }),
        (
            "getinfo 1",
            Case { glyph: &[0x00, 0xB0, 0x01, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "getinfo 16",
            Case { glyph: &[0x00, 0xB0, 0x10, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "getinfo 32",
            Case { glyph: &[0x00, 0xB0, 0x20, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "getinfo 64",
            Case { glyph: &[0x00, 0xB0, 0x40, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "getinfo 1024",
            Case { glyph: &[0x00, 0xB8, 0x04, 0x00, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "getinfo 4096",
            Case { glyph: &[0x00, 0xB8, 0x10, 0x00, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "mdrp minimum distance",
            Case { glyph: &[0x00, 0xB0, 0x40, 0x1A, 0xB0, 0x00, 0x10, 0xB0, 0x01, 0xCC], points: NEAR, ..BOX },
        ),
        (
            "mdrp unscaled",
            Case {
                glyph: &[0x00, 0xB0, 0x00, 0x10, 0xB0, 0x01, 0xC0],
                points: &[(100, 1), (100, 21), (500, 21), (500, 1)],
                ..BOX
            },
        ),
        (
            "miap twilight",
            Case {
                glyph: &[
                    0x00, 0xB0, 0x00, 0x13, 0xB1, 0x00, 0x00, 0x3F, 0xB0, 0x00, 0x15, 0xB0, 0x00, 0x46, 0xB0, 0x01,
                    0x16, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E,
                ],
                cvt: &[250],
                ..BOX
            },
        ),
        (
            "mirp twilight",
            Case {
                glyph: &[
                    0x00, 0xB0, 0x00, 0x16, 0xB1, 0x00, 0x00, 0x3E, 0xB1, 0x01, 0x01, 0xE4, 0xB0, 0x01, 0x46, 0xB0,
                    0x01, 0x16, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E,
                ],
                cvt: &[250, 100],
                ..BOX
            },
        ),
        ("miap cvt out of range", Case { glyph: &[0x00, 0xB1, 0x01, 0x32, 0x3E], cvt: &[100], ..BOX }),
        (
            "mirp cvt out of range",
            Case { glyph: &[0x00, 0xB0, 0x00, 0x10, 0xB1, 0x01, 0x32, 0xE0], cvt: &[100], ..BOX },
        ),
        (
            "mirp cvt -1",
            Case { glyph: &[0x00, 0xB0, 0x00, 0x10, 0xB9, 0x00, 0x01, 0xFF, 0xFF, 0xE0, 0xB0, 0x02, 0x2F], ..BOX },
        ),
        ("mdrp rp0 out of range", Case { glyph: &[0x00, 0xB0, 0x32, 0x10, 0xB0, 0x01, 0xC4], ..BOX }),
        ("mdap out of range", Case { glyph: &[0x00, 0xB0, 0x01, 0x10, 0xB0, 0x32, 0x2E, 0xB0, 0x02, 0xC4], ..BOX }),
        ("szp invalid", Case { glyph: &[0x00, 0xB0, 0x00, 0x13, 0xB0, 0x02, 0x13, 0xB0, 0x01, 0x2F], ..BOX }),
        ("move after iup", Case { glyph: &[0x00, 0xB0, 0x00, 0x2E, 0x30, 0x31, 0xB0, 0x01, 0x2F], ..BOX }),
        ("shpix untouched", Case { glyph: &[0x00, 0xB1, 0x01, 0x40, 0x38], ..BOX }),
        ("shpix along freedom", Case { glyph: &[0x02, 0x05, 0xB1, 0x01, 0x40, 0x38], ..BOX }),
        ("iup unscaled", Case { glyph: &[0x00, 0xB0, 0x00, 0x2E, 0xB0, 0x02, 0x2F, 0x30], points: IUP_POINTS, ..BOX }),
        (
            "phantom advance",
            Case {
                glyph: &[0x01, 0xB0, 0x05, 0x10, 0xB0, 0x02, 0xC4],
                points: &[(100, 0), (100, 700), (510, 700), (510, 0)],
                ..BOX
            },
        ),
        (
            "phantom distance",
            Case {
                glyph: &[0x01, 0xB1, 0x04, 0x05, 0x49, 0xB0, 0x00, 0x23, 0x44, 0x00, 0xB1, 0x02, 0x00, 0x3E],
                ..BOX
            },
        ),
        ("prep vectors", Case { prep: &[0x00], glyph: &[0xB0, 0x01, 0x2F], ..BOX }),
        ("prep round", Case { prep: &[0x7A], glyph: &[0x00, 0xB0, 0x01, 0x2F], ..BOX }),
        ("prep zone", Case { prep: &[0xB0, 0x00, 0x16], glyph: &[0x00, 0xB0, 0x01, 0x2F], ..BOX }),
        (
            "prep twilight",
            Case {
                prep: &[0x00, 0xB0, 0x00, 0x13, 0xB1, 0x00, 0x00, 0x3E],
                glyph: &[
                    0x00, 0xB0, 0x00, 0x15, 0xB0, 0x00, 0x46, 0xB0, 0x01, 0x16, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02,
                    0x00, 0x3E,
                ],
                cvt: &[250],
                ..BOX
            },
        ),
        (
            "twilight size",
            Case {
                twilight: 50,
                glyph: &[
                    0x00, 0xB0, 0x00, 0x13, 0xB1, 0x14, 0x00, 0x3E, 0xB0, 0x00, 0x15, 0xB0, 0x14, 0x46, 0xB0, 0x01,
                    0x16, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E,
                ],
                cvt: &[250],
                ..BOX
            },
        ),
        (
            "fpgm minimum distance",
            Case { fpgm: &[0xB0, 0x80, 0x1A], glyph: &[0x00, 0xB0, 0x00, 0x10, 0xB0, 0x01, 0xC8], points: NEAR, ..BOX },
        ),
        (
            "fpgm storage",
            Case {
                fpgm: &[0xB1, 0x00, 0x64, 0x42],
                glyph: &[0x00, 0xB0, 0x00, 0x43, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E],
                ..BOX
            },
        ),
        ("fpgm cvt", Case { fpgm: &[0xB1, 0x00, 0x7B, 0x44], glyph: &[0x00, 0xB1, 0x01, 0x00, 0x3E], ..BOX }),
        ("no font programs", Case { font_tables: false, glyph: &[0x00, 0xB0, 0x01, 0x2F], ..BOX }),
        (
            "function past 4096",
            Case {
                functions: 1,
                fpgm: &[0xB8, 0x13, 0x88, 0x2C, 0xB0, 0x01, 0x2F, 0x2D],
                glyph: &[0x00, 0xB8, 0x13, 0x88, 0x2B],
                ..BOX
            },
        ),
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
        (
            "ip",
            Case {
                glyph: &[
                    0x00, 0xB0, 0x00, 0x2E, 0xB0, 0x02, 0x2F, 0xB0, 0x00, 0x11, 0xB0, 0x02, 0x12, 0xB0, 0x01, 0x39,
                ],
                points: IUP_POINTS,
                ..BOX
            },
        ),
        ("alignrp", Case { glyph: &[0x00, 0xB0, 0x00, 0x2F, 0xB0, 0x01, 0x3C], ..BOX }),
        (
            "near-axis projection",
            Case {
                glyph: &[
                    0xB1, 0x03, 0x00, 0x06, 0xB0, 0x01, 0x46, 0xB0, 0x00, 0x23, 0x44, 0x00, 0xB1, 0x02, 0x00, 0x3E,
                ],
                points: &[(100, 0), (100, 700), (500, 700), (500, 1)],
                ..BOX
            },
        ),
        ("origin at phantom 1", Case { lsb: 60, glyph: &[0x00, 0xB0, 0x01, 0x2F], ..BOX }),
        (
            "vertical phantom x",
            Case { glyph: &[0x01, 0xB0, 0x06, 0x46, 0xB0, 0x00, 0x23, 0x44, 0x00, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        (
            "composite",
            Case {
                glyph: &[0x00, 0xB0, 0x01, 0x2F],
                composite: Some((0x0004, 10, 37, &[0x00, 0xB0, 0x02, 0x2F])),
                ..BOX
            },
        ),
        ("mps", Case { glyph: &[0x00, 0x4C, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX }),
        (
            "getinfo 2048",
            Case { glyph: &[0x00, 0xB8, 0x08, 0x00, 0x88, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E], ..BOX },
        ),
        ("native mdap", Case { prep: NATIVE, glyph: &[0x01, 0xB0, 0x00, 0x2F], ..BOX }),
        (
            "native round",
            Case {
                prep: NATIVE,
                glyph: &[0x01, 0xB0, 0x64, 0x68, 0xB0, 0x00, 0x23, 0x44, 0x00, 0xB1, 0x02, 0x00, 0x3E],
                ..BOX
            },
        ),
        (
            "native minimum distance",
            Case { prep: NATIVE, glyph: &[0x01, 0xB0, 0x00, 0x10, 0xB0, 0x01, 0xC8], ..BOX },
        ),
        ("native cut-in", Case { prep: NATIVE, glyph: &[0x01, 0xB1, 0x00, 0x00, 0x3F], cvt: &[110], ..BOX }),
        ("native in y", Case { prep: NATIVE, glyph: &[0x00, 0xB0, 0x01, 0x2F], ..BOX }),
        (
            "native mirp",
            Case { prep: NATIVE, glyph: &[0x01, 0xB0, 0x00, 0x10, 0xB1, 0x01, 0x00, 0xEC], cvt: &[30], ..BOX },
        ),
        (
            "native mirp round",
            Case { prep: NATIVE, glyph: &[0x01, 0xB0, 0x00, 0x10, 0xB1, 0x01, 0x00, 0xE4], cvt: &[2], ..BOX },
        ),
        (
            "native mirp cut-in",
            Case { prep: NATIVE, glyph: &[0x01, 0xB0, 0x00, 0x10, 0xB1, 0x01, 0x00, 0xE4], cvt: &[30], ..BOX },
        ),
        ("native mdrp round", Case { prep: NATIVE, glyph: &[0x01, 0xB0, 0x00, 0x10, 0xB0, 0x03, 0xC4], ..BOX }),
        (
            "horizontal projection",
            Case { glyph: &[0x05, 0xB9, 0xC0, 0x00, 0x00, 0x00, 0x0A, 0xB1, 0x03, 0x00, 0x48], ..BOX },
        ),
        (
            "near-orthogonal freedom",
            Case { glyph: &[0x03, 0xB9, 0x10, 0x90, 0x3D, 0xD3, 0x0B, 0xB1, 0x03, 0x00, 0x48], ..BOX },
        ),
        // f·p is 1,000 / 16,384, under 1/16, so SCFS moves nothing.
        (
            "nearly perpendicular freedom",
            Case { glyph: &[0x03, 0xB9, 0x03, 0xE8, 0x3F, 0xE1, 0x0B, 0xB1, 0x03, 0x00, 0x48], ..BOX },
        ),
        (
            "near-axis freedom",
            Case { glyph: &[0x03, 0xB1, 0x03, 0x00, 0x08, 0xB1, 0x03, 0x00, 0x48], points: RISING, ..BOX },
        ),
        (
            "near-vertical freedom",
            Case { glyph: &[0x02, 0xB1, 0x01, 0x00, 0x08, 0xB1, 0x02, 0x00, 0x48], points: LEANING, ..BOX },
        ),
        (
            "twilight near-axis freedom",
            Case {
                glyph: &[
                    0x03, 0xB1, 0x03, 0x00, 0x08, 0xB0, 0x00, 0x14, 0xB0, 0x00, 0x10, 0xB0, 0x00, 0xB8, 0x02, 0x80,
                    0x3A, 0x00, 0xB0, 0x00, 0x15, 0xB0, 0x00, 0x46, 0xB0, 0x00, 0x23, 0x44, 0xB1, 0x02, 0x00, 0x3E,
                ],
                points: RISING,
                ..BOX
            },
        ),
        ("shz named zone", Case { glyph: &[0x00, 0xB1, 0x00, 0x40, 0x48, 0xB0, 0x00, 0x15, 0xB0, 0x01, 0x37], ..BOX }),
    ];

    fn be(v: &[i16]) -> Vec<u8> {
        v.iter().flat_map(|x| x.to_be_bytes()).collect()
    }

    fn simple(points: &[(i16, i16)], program: &[u8], overlong: u16) -> Vec<u8> {
        let (xs, ys): (Vec<i16>, Vec<i16>) = points.iter().copied().unzip();
        let bbox = [
            *xs.iter().min().unwrap(),
            *ys.iter().min().unwrap(),
            *xs.iter().max().unwrap(),
            *ys.iter().max().unwrap(),
        ];
        let mut g = be(&[1, bbox[0], bbox[1], bbox[2], bbox[3], (points.len() - 1) as u16 as i16]);
        g.extend((program.len() as u16 + overlong).to_be_bytes());
        g.extend(program);
        g.extend(std::iter::repeat_n(0x01u8, points.len()));
        let delta = |v: &[i16]| {
            v.iter()
                .scan(0, |prev, &x| {
                    let d = x - *prev;
                    *prev = x;
                    Some(d)
                })
                .collect::<Vec<_>>()
        };
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
        let f = Font::from_bytes(base).unwrap();
        let mut map: BTreeMap<Tag, Vec<u8>> =
            f.table_tags().map(|t| (t, f.table(t).unwrap().to_vec())).collect();
        let u16_at = |table: &[u8], at: usize| usize::from(u16::from_be_bytes([table[at], table[at + 1]]));
        let n = u16_at(&map[&Tag::new(b"maxp")], 4);
        let long = u16_at(&map[&Tag::new(b"hhea")], 34);
        let loca_long = map[&Tag::new(b"head")][51] == 1;
        let old = |gid: usize| -> Vec<u8> {
            let at = |i: usize| if loca_long {
                u32::from_be_bytes(map[&Tag::new(b"loca")][4 * i..4 * i + 4].try_into().unwrap()) as usize
            } else {
                2 * usize::from(u16::from_be_bytes(map[&Tag::new(b"loca")][2 * i..2 * i + 2].try_into().unwrap()))
            };
            map[&Tag::new(b"glyf")][at(gid)..at(gid + 1)].to_vec()
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
            let hmtx = &map[&Tag::new(b"hmtx")];
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
        map.get_mut(&Tag::new(b"hhea")).unwrap()[34..36].copy_from_slice(&(n as u16).to_be_bytes());
        let head = map.get_mut(&Tag::new(b"head")).unwrap();
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[50..52].copy_from_slice(&1i16.to_be_bytes());
        let maxp = map.get_mut(&Tag::new(b"maxp")).unwrap();
        for (at, v) in [
            (6, 64u16),
            (8, 4),
            (14, 2),
            (16, c.twilight),
            (18, 64),
            (20, c.functions),
            (22, c.idefs),
            (24, c.stack),
            (26, 8000),
        ] {
            maxp[at..at + 2].copy_from_slice(&v.to_be_bytes());
        }
        map.insert(Tag::new(b"glyf"), glyf);
        map.insert(Tag::new(b"loca"), loca);
        map.insert(Tag::new(b"hmtx"), hmtx);
        if c.font_tables {
            map.insert(Tag::new(b"cvt "), be(c.cvt));
            map.insert(Tag::new(b"fpgm"), c.fpgm.to_vec());
            map.insert(Tag::new(b"prep"), c.prep.to_vec());
        } else {
            for tag in [b"cvt ", b"fpgm", b"prep"] {
                map.remove(&Tag::new(tag));
            }
        }
        daegun::raw::build_font(&map).unwrap()
    }
}
