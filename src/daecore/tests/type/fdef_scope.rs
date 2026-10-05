use daegun::daecore::daetype::hinting::{HintContext, HintMode};
use daegun::daecore::daetype::TableBytes;

const PPEM: u16 = 16;

fn parts() -> (std::collections::BTreeMap<String, TableBytes>, Vec<usize>, Vec<u8>, u16) {
    let path = format!("{}/test-fixtures/hinted.ttf", crate::FONTS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    let map = daegun::daecore::daetype::decoder::extract_ttf_tables(&bytes).expect("fixture parses");
    let upm = daegun::daecore::daetype::decoder::read_u16_be(map.get("head").expect("head"), 18).expect("upm");
    let fmt = daegun::daecore::daetype::decoder::read_i16_be(map.get("head").expect("head"), 50).expect("fmt");
    let n = daegun::daecore::daetype::decoder::read_u16_be(map.get("maxp").expect("maxp"), 4).expect("n");
    let loca = daegun::daecore::daetype::instancer::parse_loca(&map, fmt, n as usize).expect("loca");
    let glyf = map.get("glyf").expect("glyf").to_owned_vec();
    (map, loca, glyf, upm)
}

const GID_A: u16 = 1;
const GID_E: u16 = 5;
const GID_F: u16 = 6;

fn hint_sequence(gids: &[u16]) -> Vec<Vec<i32>> {
    let (map, loca, glyf, upm) = parts();
    let mut ctx = HintContext::new(&map, PPEM.into(), upm, HintMode::Classic).expect("fixture hints");
    gids.iter()
        .map(|&g| {
            let out = ctx.hint_glyph(&glyf, &loca, g).unwrap_or_else(|| panic!("gid {g} hinted to nothing"));
            out.x.clone()
        })
        .collect()
}

#[test]
fn a_glyphs_own_function_does_not_leak_into_the_next_glyph() {
    let alone = hint_sequence(&[GID_F]);
    let after_e = hint_sequence(&[GID_E, GID_F]);
    assert_eq!(
        alone[0], after_e[1],
        "F hinted differently after E ran, so E's glyph-level FDEF outlived it",
    );
}

#[test]
fn the_shared_function_actually_rounds() {
    let both = hint_sequence(&[GID_A, GID_F]);
    assert_eq!(both[0], both[1], "A and F carry the same bytecode over the same box but hinted differently");

    let rounded = both[1].iter().filter(|v| *v & 0x3f == 0).count();
    assert!(
        rounded >= 2,
        "expected fn 0 to round at least the two stem edges to whole pixels, got {rounded} of {} \
         ({:?}) – the leak test above would pass on a build that never rounds at all",
        both[1].len(),
        both[1],
    );
}
