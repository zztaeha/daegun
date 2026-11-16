use super::*;
use alloc::vec;

// Glyph 2 holds 1,000 components of the empty glyph 1, and each of glyphs 3 on, `layers` of them, 64 of glyph 2: read
// whole, 64,065 glyphs, but drawing no point. The last glyph's COLR version 0 layers are those.
fn hollow(layers: u8) -> Font {
    let words = |w: &[i16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let composite = |gid: u8, n: usize| {
        let mut g = words(&[-1, 0, 0, 0, 0]);
        (0..n).for_each(|i| g.extend([0, if i + 1 < n { 0x22 } else { 0x02 }, 0, gid, 0, 0]));
        g
    };
    let base = 3 + i16::from(layers);
    let glyphs = [vec![Vec::new(), Vec::new(), composite(1, 1000)], vec![composite(2, 64); layers.into()]].concat();
    let (mut glyf, mut loca) = (Vec::new(), Vec::new());
    for g in &glyphs {
        loca.extend((glyf.len() as u32).to_be_bytes());
        glyf.extend(g);
    }
    loca.extend([glyf.len() as u32; 2].map(u32::to_be_bytes).concat());
    let mut head = vec![0; 54];
    head[..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&[0x5F, 0x0F, 0x3C, 0xF5]);
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = 1;
    let maxp = [&[0u8, 0, 0x50, 0][..], &(base as u16 + 1).to_be_bytes()].concat();
    let mut colr = words(&[0, 1, 0, 14, 0, 20, layers.into(), base, 0, layers.into()]);
    colr.extend((3..base).flat_map(|g| words(&[g, 0])));
    let cpal = [&[0u16, 1, 1, 1, 0, 14, 0].map(u16::to_be_bytes).concat()[..], &[0, 0, 0, 255]].concat();
    let tables = [("head", head), ("maxp", maxp), ("loca", loca), ("glyf", glyf), ("COLR", colr), ("CPAL", cpal)];
    let bytes = crate::daecore::daetype::decoder::build_ttf(tables.iter().map(|(tag, bytes)| (*tag, bytes))).unwrap();
    Font::from_bytes(&bytes).expect("loads")
}

// A scene's glyphs cost it what reading them took though they draw no point: one layer of 64,065 glyphs read draws,
// and 17 pass the 2^20 points a small font's scene may hold.
#[test]
fn layers_drawing_no_point_cost_a_scene_what_reading_them_took() {
    let scene = |font: &Font, gid| font.color_scene(GlyphId(gid), &Location::default(), &SceneOptions::default());
    assert!(scene(&hollow(1), 4).is_ok_and(|s| s.is_some()));
    assert_eq!(scene(&hollow(17), 20).map_err(|e| e.kind()).err(), Some(crate::ErrorKind::LimitExceeded));
}
