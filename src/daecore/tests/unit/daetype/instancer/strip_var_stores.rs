use super::*;
use alloc::vec;

fn u16s(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_be_bytes()).collect()
}

// A MATH whose MathLeading (100), italics correction (10), top accent (15), MathKern value (30) and assembly's
// correction (40) name devices (0, 0) to (0, 4), and AxisHeight (200) a device of sizes; and each one's place.
fn math() -> (Vec<u8>, [usize; 6]) {
    let (var, coverage) = (|inner: u16| u16s(&[0, inner, 0x8000]), u16s(&[1, 1, 1]));
    let constants = [u16s(&[0, 0, 0, 0, 100, 214, 200, 220]), vec![0; 49 * 4 + 2], var(0), u16s(&[8, 8, 1, 0x4000])]
        .concat();
    let corrections = |value: u16, inner: u16| [u16s(&[8, 1, value, 14]), coverage.clone(), var(inner)].concat();
    let kern = [u16s(&[12, 1, 18, 0, 0, 0]), coverage.clone(), u16s(&[0, 30, 6]), var(3)].concat();
    let info = [u16s(&[8, 28, 0, 48]), corrections(10, 1), corrections(15, 2), kern].concat();
    let variants = [u16s(&[0, 12, 0, 1, 0, 18]), coverage, u16s(&[4, 0, 40, 6, 0]), var(4)].concat();
    let at = (10, 10 + constants.len(), 10 + constants.len() + info.len());
    let header = u16s(&[1, 0, at.0 as u16, at.1 as u16, at.2 as u16]);
    let values = [at.0 + 8, at.0 + 12, at.1 + 12, at.1 + 32, at.1 + 68, at.2 + 22];
    ([header, constants, info, variants].concat(), values)
}

// A one-axis store of one region, peaking at 1, whose rows' deltas are 10, -21, 7, 40 and 1.
fn store() -> ItemVariationStore {
    let bytes = [u16s(&[1, 0, 12, 1, 0, 22, 1, 1, 0, 0x4000, 0x4000, 5, 0, 1, 0]), vec![10, 0xEB, 7, 40, 1]];
    let bytes = bytes.concat();
    ItemVariationStore::parse(&bytes, 0).expect("a store")
}

// GDEF 1.4's LigCaretList2, of 24-bit counts and offsets past a 32-bit coverage offset: its caret of format 3,
// at 300 with VariationIndex (0, 0), takes 5 at 0.5 as format 1, and the store goes.
#[test]
fn a_24_bit_ligature_caret_list_is_baked() {
    let u24 = |v: u32| v.to_be_bytes()[1..].to_vec();
    let longs = |w: &[u32]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let caret = u16s(&[1, 4, 3, 300, 6, 0, 0, 0x8000]);
    let carets = [longs(&[10]), u24(1), u24(18), u16s(&[3]), u24(1), u24(70_001), caret].concat();
    let store = [u16s(&[1, 0, 12, 1, 0, 22, 1, 1, 0, 0x4000, 0x4000, 5, 0, 1, 0]), vec![10, 0xEB, 7, 40, 1]];
    let lists = longs(&[38 + carets.len() as u32, 0, 0, 38, 0, 0]);
    let gdef = [u16s(&[1, 4, 0, 0, 0, 0, 0]), lists, carets, store.concat()].concat();
    let baked = bake_gdef(&gdef, &[0.5]).expect("baked");
    let read = (read_u32_be(&baked, 14), read_u16_be(&baked, 60), read_i16_be(&baked, 62));
    assert_eq!(read, (Some(0), Some(1), Some(305)));
}

// Each VariationIndex device's delta at the location, rounded alone and half up, is baked into its value and the device
// dropped; a device of sizes stays. Without GDEF's store only the devices go.
#[test]
fn math_values_take_their_deltas_and_drop_their_devices() {
    let (math, at) = math();
    let read = |out: &[u8]| at.map(|at| (read_i16_be(out, at).unwrap(), read_u16_be(out, at + 2).unwrap()));
    let baked = bake_math(&math, Some(&store()), &[0.5]).expect("baked");
    assert_eq!(read(&baked), [(105, 0), (200, 220), (0, 0), (19, 0), (50, 0), (41, 0)]);
    let dropped = bake_math(&math, None, &[0.5]).expect("devices dropped");
    assert_eq!(read(&dropped), [(100, 0), (200, 220), (10, 0), (15, 0), (30, 0), (40, 0)]);
}

// LigGlyphs one byte apart in one run of 0x40s, each read as 16,448 carets: past the subset budget at 20 ligatures,
// though not at 2, where GDEF stays as stored.
#[test]
fn ligatures_that_overlap_past_the_budget_leave_gdef_as_stored() {
    let gdef = |ligatures: usize| {
        let offsets: Vec<u16> = (0..ligatures).map(|i| (4 + 2 * ligatures + i) as u16).collect();
        let header = [u16s(&[1, 3, 0, 0, 18, 0, 0, 0, 18, 0, ligatures as u16]), u16s(&offsets)].concat();
        [header, vec![0x40; ligatures + 2 * 0x4040 + 64]].concat()
    };
    assert!(bake_gdef(&gdef(2), &[0.5]).is_some());
    assert!(bake_gdef(&gdef(20), &[0.5]).is_none());
}

// MathKerns one byte apart in one run of 0x40s, each read as 16,448 heights: past the subset budget at 20 kerns,
// though not at 4, where MATH stays as stored though its MathLeading names a device.
#[test]
fn math_kerns_that_overlap_past_the_budget_leave_math_as_stored() {
    let math = |records: usize| {
        let constants = [u16s(&[0, 0, 0, 0, 100, 214]), vec![0; 202], u16s(&[0, 0, 0x8000])].concat();
        let corners: Vec<u16> = (0..4 * records).map(|i| (4 + 8 * records + i) as u16).collect();
        let info = [u16s(&[0, 0, 0, 8, 2, records as u16]), u16s(&corners)].concat();
        [u16s(&[1, 0, 10, 230, 0]), constants, info, vec![0x40; 4 * records + 4 * (2 * 0x4040 + 1) + 64]].concat()
    };
    assert!(bake_math(&math(1), None, &[0.5]).is_some());
    assert!(bake_math(&math(5), None, &[0.5]).is_none());
}
