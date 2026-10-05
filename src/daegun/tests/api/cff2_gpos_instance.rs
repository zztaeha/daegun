use std::collections::BTreeMap;

use daegun::Font;
use daegun::daecore::cache::FontCache;
use daegun::daecore::daeshaper::{buffer::Buffer, face::Face, plan::ShapePlan, shape};
use daegun::daecore::daetype::decoder::{build_ttf, extract_ttf_tables};

fn bytes_of(rel: &str) -> Vec<u8> {
    std::fs::read(format!("{}/{rel}", crate::FONTS)).expect("fixture")
}

fn u16_at(t: &[u8], at: usize) -> usize {
    usize::from(u16::from_be_bytes([t[at], t[at + 1]]))
}

// Inter kerns H against _ through a PairPos format 1 record whose VariationIndex gives +69 at
// wght 900, as HarfBuzz shapes it.
#[test]
fn a_pair_sets_device_is_read_from_the_pair_set() {
    let inter = Font::from_bytes(&bytes_of("inter/InterVariable.ttf")).unwrap();
    let unit = 1000.0 / f64::from(inter.upm());
    let kern = |font: &Font, axes: &[(&str, f64)]| {
        font.shape("H_", axes, false).unwrap().advances[0] - font.shape("H", axes, false).unwrap().advances[0]
    };
    let instance = Font::from_vec(inter.instance(&[("wght", 900.0)])).unwrap();
    assert!((kern(&instance, &[]) - 69.0 * unit).abs() < 1e-6, "instanced: {}", kern(&instance, &[]) / unit);

    let fc = FontCache::new(extract_ttf_tables(&bytes_of("inter/InterVariable.ttf")).unwrap());
    let face = Face::new(&fc, &[("wght", 900.0)]);
    let advance = |text: &str| {
        let mut buffer = Buffer::new();
        buffer.push_str(text);
        let direction = shape::guess_segment_properties(&mut buffer);
        let plan = ShapePlan::with_script(buffer.script, &face, direction, &[], &[], &[], &[], &[]);
        shape::shape(&face, &plan, &mut buffer, direction);
        shape::shaped_glyphs(&buffer)[0].x_advance
    };
    assert_eq!(advance("H_") - advance("H"), 69, "the shaper on the variable font");
}

// One PairPos format 2 claiming 65,535 classes by 65,535, and an Extension lookup of 65,535
// subtables that never resolve: billions of steps without a budget, from under 300 KB.
#[test]
fn a_hostile_gpos_is_instanced_quickly() {
    let mut gpos = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 10];
    gpos.extend([0, 2, 0, 6, 0, 14]);
    gpos.extend([0, 2, 0, 0, 0, 1, 0, 8]);
    gpos.extend([0, 2, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
    let ext = gpos.len() - 10;
    gpos.extend([0, 9, 0, 0, 0xFF, 0xFF]);
    gpos.extend(std::iter::repeat_n([0, 4], 0xFFFF).flatten());
    gpos[16..18].copy_from_slice(&(ext as u16).to_be_bytes());
    let mut map: BTreeMap<String, Vec<u8>> =
        extract_ttf_tables(&bytes_of("inter/InterVariable.ttf")).unwrap().into_iter().map(|(t, d)| (t, d.to_vec())).collect();
    map.insert("GPOS".into(), gpos);
    let font = Font::from_vec(build_ttf(&map)).unwrap();
    let started = std::time::Instant::now();
    assert!(!font.instance(&[("wght", 900.0)]).is_empty());
    assert!(started.elapsed().as_secs_f64() < 2.0, "instancing took {:?}", started.elapsed());
}

// A SinglePos whose XAdvance carries a device table of sizes, not a VariationIndex: the instance
// keeps it, as it is not variation data.
#[test]
fn a_device_table_of_sizes_survives_instancing() {
    let mut gpos = vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 10];
    gpos.extend([0, 1, 0, 4]);
    gpos.extend([0, 1, 0, 0, 0, 1, 0, 8]);
    gpos.extend([0, 1, 0, 10, 0, 0x44, 0, 50, 0, 16]);
    gpos.extend([0, 1, 0, 1, 0, 36]);
    gpos.extend([0, 12, 0, 12, 0, 1, 0x30, 0]);
    let mut map: BTreeMap<String, Vec<u8>> =
        extract_ttf_tables(&bytes_of("inter/InterVariable.ttf")).unwrap().into_iter().map(|(t, d)| (t, d.to_vec())).collect();
    map.insert("GPOS".into(), gpos);
    let font = Font::from_vec(build_ttf(&map)).unwrap();
    let out = extract_ttf_tables(&font.instance(&[("wght", 900.0)])).unwrap();
    let gpos = &out["GPOS"];
    assert_eq!(u16_at(gpos, 10 + 4 + 8 + 8), 16, "the device offset was cleared");
}

// Source Serif's CFF2 FDs carry blended blue zones and stems. FD 1 at wght 700, worked from the
// stored deltas at the avar-mapped location: BlueValues -19 0 483 498 716 736, StdVW 159.
#[test]
fn an_instances_private_dicts_keep_their_blue_zones() {
    let serif = Font::from_bytes(&bytes_of("source-serif/SourceSerif4Variable-Roman.otf")).unwrap();
    let cff = extract_ttf_tables(&serif.instance(&[("wght", 700.0)])).unwrap()["CFF "].to_vec();
    let private = fd_private(&cff, 1);
    assert_eq!(dict_operands(private, &[6]), Some(vec![-19, 19, 483, 15, 218, 20]), "BlueValues, delta-coded");
    assert_eq!(dict_operands(private, &[11]), Some(vec![159]));
}

// The Private DICT of FD `fd` in a CID-keyed CFF.
fn fd_private(cff: &[u8], fd: usize) -> &[u8] {
    let index_end = |at: usize| {
        let count = u16_at(cff, at);
        let size = usize::from(cff[at + 2]);
        let last = (0..size).fold(0, |v, i| v << 8 | usize::from(cff[at + 3 + count * size + i]));
        at + 3 + (count + 1) * size + last - 1
    };
    let names = usize::from(cff[2]);
    let top_index = index_end(names);
    let top = &cff[top_index + 3 + 2 * usize::from(cff[top_index + 2])..index_end(top_index)];
    let fd_array = dict_operands(top, &[12, 36]).expect("CID-keyed")[0] as usize;
    let size = usize::from(cff[fd_array + 2]);
    let start = fd_array + 3 + (u16_at(cff, fd_array) + 1) * size - 1;
    let offset = |i: usize| (0..size).fold(0, |v, k| v << 8 | usize::from(cff[fd_array + 3 + i * size + k]));
    let dict = &cff[start + offset(fd)..start + offset(fd + 1)];
    let p = dict_operands(dict, &[18]).unwrap();
    &cff[p[1] as usize..(p[1] + p[0]) as usize]
}

// The integer operands of operator `op` in a DICT; reals are skipped.
fn dict_operands(dict: &[u8], op: &[u8]) -> Option<Vec<i32>> {
    let (mut i, mut stack) = (0, Vec::new());
    while i < dict.len() {
        let b = dict[i];
        match b {
            0..=21 => {
                let this: &[u8] = if b == 12 { &dict[i..i + 2] } else { &dict[i..i + 1] };
                if this == op { return Some(stack); }
                stack.clear();
                i += this.len();
            }
            28 => { stack.push(i32::from(i16::from_be_bytes([dict[i + 1], dict[i + 2]]))); i += 3; }
            29 => { stack.push(i32::from_be_bytes(dict[i + 1..i + 5].try_into().unwrap())); i += 5; }
            30 => { while dict[i] & 0x0F != 0x0F && dict[i] >> 4 != 0x0F || i == 0 || dict[i] == 30 { i += 1; } i += 1; }
            32..=246 => { stack.push(i32::from(b) - 139); i += 1; }
            247..=250 => { stack.push((i32::from(b) - 247) * 256 + i32::from(dict[i + 1]) + 108); i += 2; }
            251..=254 => { stack.push(-(i32::from(b) - 251) * 256 - i32::from(dict[i + 1]) - 108); i += 2; }
            _ => return None,
        }
    }
    None
}
