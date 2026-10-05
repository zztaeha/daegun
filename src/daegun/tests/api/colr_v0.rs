use daegun::Font;

fn font(rel: &str) -> Font {
    let path = format!("{}/{rel}", crate::FONTS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_vec(bytes).expect("parses")
}

fn bungee() -> Font {
    font("bungee-tint/BungeeTint-Regular.ttf")
}

fn d_of(f: &Font) -> u16 {
    f.shape("D", &[], false).expect("shapes").glyphs[0]
}

#[test]
fn a_glyph_with_no_color_description_is_still_refused() {
    let f = font("inter/InterVariable.ttf");
    let gid = d_of(&f);
    assert!(f.colr_layers(gid).is_none(), "Inter has no COLR table");
    assert!(f.colr_scene(gid, &[], 0).is_none(), "a plain glyph must not have a color scene");
}

#[test]
fn a_colr_v0_glyph_becomes_a_color_scene() {
    let f = bungee();
    let gid = d_of(&f);
    assert!(f.colr_v1_paint(gid, &[], 0).is_none(), "the fixture has to be v0, or this proves nothing");
    let layers = f.colr_layers_for_palette(gid, 0).expect("a v0 color glyph");
    assert_eq!(layers.len(), 2, "a base and a tint layer");

    let scene = f.colr_scene(gid, &[], 0).expect("a v0 glyph has a scene");
    let fills: Vec<(usize, daegun::paint::Rgba)> = scene
        .ops()
        .iter()
        .map(|op| match op {
            daegun::paint::Op::Fill { path, paint: daegun::paint::Paint::Solid(c), .. } => (*path, *c),
            other => panic!("a v0 scene holds only solid fills, found {other:?}"),
        })
        .collect();
    assert_eq!(fills.len(), layers.len(), "one fill per layer");
    for (&(path, color), &(layer, r, g, b, a, foreground)) in fills.iter().zip(&layers) {
        assert!(!foreground, "layer {layer} defers to the text color; pick a fixture layer that does not");
        assert_eq!((color.r, color.g, color.b, color.a), (r, g, b, a), "layer {layer}: not the palette color");
        let mut want = daegun::Path::default();
        f.outline_glyph_instanced(layer, &[], &mut want).expect("the layer has an outline");
        assert_eq!(scene.path(path), Some(&want), "layer {layer}: the fill draws another outline");
    }
    assert!(fills.iter().any(|(_, c)| c.r != c.g || c.g != c.b), "no fill carries a hue");
}

#[test]
fn a_palette_changes_a_scenes_colors_and_not_its_geometry() {
    use daegun::paint::{Op, Paint};
    let f = bungee();
    let gid = d_of(&f);
    let first = f.colr_scene(gid, &[], 0).expect("palette 0");
    assert!(f.palette_count() > 1, "BungeeTint lost its extra palettes; pick another fixture");
    let mut differed = false;
    for palette in 1..f.palette_count() {
        let other = f.colr_scene(gid, &[], palette).expect("another palette");
        assert_eq!(other.ops().len(), first.ops().len(), "palette {palette} changed the layer count");
        for (a, b) in first.ops().iter().zip(other.ops()) {
            let (Op::Fill { path: pa, paint: Paint::Solid(ca), .. }, Op::Fill { path: pb, paint: Paint::Solid(cb), .. }) = (a, b)
            else { panic!("a v0 scene holds only solid fills") };
            assert_eq!(first.path(*pa), other.path(*pb), "palette {palette} changed an outline");
            differed |= ca != cb;
        }
    }
    assert!(differed, "every palette gave identical colors, so nothing was read from CPAL");
}

// Pairs each op of the default scene with the same op under `chosen`; only foreground fills may differ.
fn foreground_fills(f: &Font, gid: u16, chosen: daegun::paint::Rgba) -> Vec<(daegun::paint::Rgba, daegun::paint::Rgba)> {
    use daegun::paint::{Op, Paint};
    let plain = f.colr_scene(gid, &[], 0).expect("a color glyph");
    let with = f.colr_scene_with(gid, &[], 0, chosen).expect("a color glyph");
    assert_eq!(plain.ops().len(), with.ops().len(), "gid {gid}: the foreground color changed the op count");
    let mut id = 0;
    while plain.path(id).is_some() || with.path(id).is_some() {
        assert_eq!(plain.path(id), with.path(id), "gid {gid}: the foreground color changed path {id}");
        id += 1;
    }
    let mut changed = Vec::new();
    for (a, b) in plain.ops().iter().zip(with.ops()) {
        match (a, b) {
            (
                Op::Fill { path: pa, paint: Paint::Solid(ca), rule: ra, transform: ta },
                Op::Fill { path: pb, paint: Paint::Solid(cb), rule: rb, transform: tb },
            ) if ca != cb => {
                assert_eq!((pa, ra, ta), (pb, rb, tb), "gid {gid}: a recolored fill also moved");
                changed.push((*ca, *cb));
            }
            _ => assert_eq!(a, b, "gid {gid}: an op that is not a foreground fill changed"),
        }
    }
    changed
}

// The only two foreground solids in the fixture: layers that defer to the caller's text color.
#[test]
fn a_foreground_layer_takes_the_default_text_color() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    for gid in [154u16, 155] {
        let changed = foreground_fills(&f, gid, daegun::paint::Rgba { r: 255, g: 0, b: 0, a: 255 });
        assert!(!changed.is_empty(), "gid {gid} has no fill that takes the text color");
        // 155's layer has an alpha of 0.3 of its own, which scales the opaque black.
        for (default, _) in changed {
            assert_eq!((default.r, default.g, default.b), (0, 0, 0), "gid {gid}: the default is not black");
            assert_eq!(default.a, if gid == 155 { 76 } else { 255 }, "gid {gid}: the layer's alpha is lost");
        }
    }
}

// A layer that defers to the caller's text color has to take the color the caller names, not the
// opaque black the no-argument entry points fall back to, and scale its alpha as it scaled black's.
#[test]
fn a_caller_can_choose_the_foreground_color() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let red = daegun::paint::Rgba { r: 255, g: 0, b: 0, a: 128 };
    for gid in [154u16, 155] {
        for (default, chosen) in foreground_fills(&f, gid, red) {
            assert_eq!((chosen.r, chosen.g, chosen.b), (255, 0, 0), "gid {gid}: the color was ignored");
            let want = (f32::from(default.a) * 128.0 / 255.0).round();
            assert!((f32::from(chosen.a) - want).abs() <= 1.0, "gid {gid}: alpha {} where {want} was due", chosen.a);
        }
    }
}

// A COLR v0 layer in the text color takes the color the caller names; its sibling keeps its palette's.
#[test]
fn a_colr_v0_foreground_layer_takes_the_chosen_color() {
    use daegun::paint::{Op, Paint};
    let f = font("test-fixtures/colr-clip.ttf");
    let gid = f.glyph_id('F' as u32).expect("the fixture maps F");
    let red = daegun::paint::Rgba { r: 255, g: 0, b: 0, a: 200 };
    let scene = f.colr_scene_with(gid, &[], 0, red).expect("a v0 color glyph");
    let fills: Vec<_> = scene.ops().iter()
        .filter_map(|op| match op { Op::Fill { paint: Paint::Solid(c), .. } => Some(*c), _ => None })
        .collect();
    assert_eq!(fills.len(), 2, "{fills:?}");
    assert_eq!(fills[0], red, "the text-color layer did not take the chosen color");
    assert_ne!(fills[1], red, "the palette layer took the chosen color too");
}

// CPAL version 1 flags palettes for light and dark backgrounds: test_glyphs.ttf marks its second for
// dark and its third for light, as fontTools reads them. Version 0 has no flags to give.
#[test]
fn palette_info_reads_each_palettes_flags() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let flags: Vec<_> = f.palette_info().iter().map(|p| (p.index, p.light_safe, p.dark_safe, p.name_id)).collect();
    assert_eq!(flags, [(0, false, false, None), (1, false, true, None), (2, true, false, None)]);
    let v0 = font("bungee-tint/BungeeTint-Regular.ttf").palette_info();
    assert_eq!(v0.len(), 8, "Bungee Tint has eight palettes");
    assert!(v0.iter().all(|p| !p.light_safe && !p.dark_safe && p.name_id.is_none()), "a version 0 palette had flags");
}

// The emoji font's one strike is 109 ppem, so a smaller size is served from it, as a PNG.
#[test]
fn a_bitmap_glyph_comes_back_as_its_png() {
    let f = font("noto-color-emoji/NotoColorEmoji.ttf");
    let gid = f.glyph_id(0x1F600).expect("the font maps U+1F600");
    let b = f.glyph_bitmap(gid, 64).expect("a bitmap");
    let daegun::BitmapImage::Png(png) = &b.image else { panic!("the bitmap is not a PNG") };
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "the bitmap is not a PNG");
    assert_eq!(b.ppem, 109, "the strike's size");
    assert!(font("inter/InterVariable.ttf").glyph_bitmap(36, 64).is_none(), "an outline font gave a bitmap");
}

// Both formats report the image's top edge: CBDT's bearingY is it, and sbix's originOffsetY is the
// bottom, -27 here, so 101 once its 128-pixel height is added.
#[test]
fn sbix_and_cbdt_place_by_the_same_edge() {
    let b = font("noto-color-emoji/NotoColorEmoji.ttf").glyph_bitmap(883, 109).expect("a CBDT bitmap");
    assert_eq!((b.left, b.top), (0, 101));
    let b = font("color-fonts-samples/samples-sbix.ttf").glyph_bitmap(19, 109).expect("an sbix bitmap");
    assert_eq!((b.left, b.top, b.mirrored), (4, 101, false));
}
