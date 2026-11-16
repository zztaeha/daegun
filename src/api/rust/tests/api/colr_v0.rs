use daegun::color::{DisplayList, Rgba, SceneOptions};
use daegun::shape::ShapeOptions;
use daegun::{Font, GlyphId, Location};

fn font(rel: &str) -> Font {
    let path = format!("{}/{rel}", crate::FONTS);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_vec(bytes, 0).expect("parses")
}

fn bungee() -> Font {
    font("bungee-tint/BungeeTint-Regular.ttf")
}

fn d_of(f: &Font) -> GlyphId {
    f.shape("D", &Location::default(), &ShapeOptions::default()).glyphs[0]
}

fn scene_with(f: &Font, gid: GlyphId, options: &SceneOptions) -> Option<DisplayList> {
    f.color_scene(gid, &Location::default(), options).expect("the scene reads")
}

fn scene(f: &Font, gid: GlyphId, palette: u16) -> Option<DisplayList> {
    scene_with(f, gid, &SceneOptions::default().with_palette(palette))
}

fn palette_count(f: &Font) -> u16 {
    f.palettes().len() as u16
}

#[test]
fn a_glyph_with_no_color_description_is_still_refused() {
    let f = font("inter/InterVariable.ttf");
    let gid = d_of(&f);
    assert!(f.colr_layers(gid, 0).is_none(), "Inter has no COLR table");
    assert!(scene(&f, gid, 0).is_none(), "a plain glyph must not have a color scene");
}

#[test]
fn a_colr_v0_glyph_becomes_a_color_scene() {
    let f = bungee();
    let gid = d_of(&f);
    assert!(f.paint_graph(gid, &Location::default(), 0).is_none(), "the fixture has to be v0, or this proves nothing");
    let layers = f.colr_layers(gid, 0).expect("a v0 color glyph");
    assert_eq!(layers.len(), 2, "a base and a tint layer");

    let scene = scene(&f, gid, 0).expect("a v0 glyph has a scene");
    let fills: Vec<(daegun::color::PathId, daegun::color::Rgba)> = scene
        .ops()
        .iter()
        .map(|op| match op {
            daegun::color::Op::Fill { path, paint: daegun::color::Paint::Solid(c), .. } => (*path, *c),
            other => panic!("a v0 scene holds only solid fills, found {other:?}"),
        })
        .collect();
    assert_eq!(fills.len(), layers.len(), "one fill per layer");
    for (&(path, color), l) in fills.iter().zip(&layers) {
        let layer = l.glyph;
        assert!(!l.foreground, "layer {layer} defers to the text color; pick a fixture layer that does not");
        assert_eq!(color, l.color, "layer {layer}: not the palette color");
        let mut want = daegun::outline::Path::default();
        f.outline(layer, &Location::default(), &mut want).expect("the layer has an outline");
        assert_eq!(scene.path(path), Some(want.view()), "layer {layer}: the fill draws another outline");
    }
    assert!(fills.iter().any(|(_, c)| c.r != c.g || c.g != c.b), "no fill carries a hue");
}

#[test]
fn a_palette_changes_a_scenes_colors_and_not_its_geometry() {
    use daegun::color::{Op, Paint};
    let f = bungee();
    let gid = d_of(&f);
    let first = scene(&f, gid, 0).expect("palette 0");
    assert!(palette_count(&f) > 1, "BungeeTint lost its extra palettes; pick another fixture");
    let mut differed = false;
    for palette in 1..palette_count(&f) {
        let other = scene(&f, gid, palette).expect("another palette");
        assert_eq!(other.ops().len(), first.ops().len(), "palette {palette} changed the layer count");
        for (a, b) in first.ops().iter().zip(other.ops()) {
            let (
                Op::Fill { path: pa, paint: Paint::Solid(ca), .. },
                Op::Fill { path: pb, paint: Paint::Solid(cb), .. },
            ) = (a, b)
            else {
                panic!("a v0 scene holds only solid fills")
            };
            assert_eq!(first.path(*pa), other.path(*pb), "palette {palette} changed an outline");
            differed |= ca != cb;
        }
    }
    assert!(differed, "every palette gave identical colors, so nothing was read from CPAL");
}

// Pairs each op of the default scene with the same op under `chosen`; only foreground fills may differ.
fn foreground_fills(
    f: &Font, gid: GlyphId, chosen: daegun::color::Rgba,
) -> Vec<(daegun::color::Rgba, daegun::color::Rgba)> {
    use daegun::color::{Op, Paint};
    let plain = scene(f, gid, 0).expect("a color glyph");
    let with = scene_with(f, gid, &SceneOptions::default().with_foreground(chosen)).expect("a color glyph");
    assert_eq!(plain.ops().len(), with.ops().len(), "gid {gid}: the foreground color changed the op count");
    let (p, w): (Vec<_>, Vec<_>) = (plain.paths().collect(), with.paths().collect());
    assert_eq!(p, w, "gid {gid}: the foreground color changed a path");
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
    for gid in [GlyphId(154), GlyphId(155)] {
        let changed = foreground_fills(&f, gid, daegun::color::Rgba { r: 255, g: 0, b: 0, a: 255 });
        assert!(!changed.is_empty(), "gid {gid} has no fill that takes the text color");
        // 155's layer has an alpha of 0.3 of its own, which scales the opaque black.
        for (default, _) in changed {
            assert_eq!((default.r, default.g, default.b), (0, 0, 0), "gid {gid}: the default is not black");
            assert_eq!(default.a, if gid == GlyphId(155) { 76 } else { 255 }, "gid {gid}: the layer's alpha is lost");
        }
    }
}

// A layer that defers to the caller's text color has to take the color the caller names, not the
// opaque black the no-argument entry points fall back to, and scale its alpha as it scaled black's.
#[test]
fn a_caller_can_choose_the_foreground_color() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let red = daegun::color::Rgba { r: 255, g: 0, b: 0, a: 128 };
    for gid in [GlyphId(154), GlyphId(155)] {
        for (default, chosen) in foreground_fills(&f, gid, red) {
            assert_eq!((chosen.r, chosen.g, chosen.b), (255, 0, 0), "gid {gid}: the color was ignored");
            let want = (f32::from(default.a) * 128.0 / 255.0).round();
            assert!((f32::from(chosen.a) - want).abs() <= 1.0, "gid {gid}: alpha {} where {want} was due", chosen.a);
        }
    }
}

// A palette index at or past the count reads palette 0, the default, in version 0 layers, version 1
// paints and scenes alike, rather than leaving the glyph uncolored.
#[test]
fn a_palette_past_the_count_reads_palette_0() {
    let f = font("bungee-tint/BungeeTint-Regular.ttf");
    let gid = f.glyph_id('A').expect("A");
    let count = palette_count(&f);
    assert_eq!(count, 8, "Bungee Tint has eight palettes");
    for past in [count, count + 1, u16::MAX] {
        assert_eq!(f.colr_layers(gid, past), f.colr_layers(gid, 0), "layers, palette {past}");
        assert_eq!(scene(&f, gid, past), scene(&f, gid, 0), "scene, palette {past}");
    }
    assert_ne!(f.colr_layers(gid, 1), f.colr_layers(gid, 0), "palette 1 is palette 0");

    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let gid = f.glyph_id('\u{F0100}').expect("a version 1 glyph");
    let count = palette_count(&f);
    let graph = |palette| f.paint_graph(gid, &Location::default(), palette);
    assert!(graph(0).is_some(), "no version 1 paint");
    assert_eq!(graph(count), graph(0), "paint, palette {count}");
    assert_eq!(scene(&f, gid, count), scene(&f, gid, 0), "scene, palette {count}");
}

// A COLR v0 layer in the text color takes the color the caller names; its sibling keeps its palette's.
#[test]
fn a_colr_v0_foreground_layer_takes_the_chosen_color() {
    use daegun::color::{Op, Paint};
    let f = font("test-fixtures/colr-clip.ttf");
    let gid = f.glyph_id('F').expect("the fixture maps F");
    let red = daegun::color::Rgba { r: 255, g: 0, b: 0, a: 200 };
    let scene = scene_with(&f, gid, &SceneOptions::default().with_foreground(red)).expect("a v0 color glyph");
    let fills: Vec<_> = scene.ops().iter()
        .filter_map(|op| match op { Op::Fill { paint: Paint::Solid(c), .. } => Some(*c), _ => None })
        .collect();
    assert_eq!(fills.len(), 2, "{fills:?}");
    assert_eq!(fills[0], red, "the text-color layer did not take the chosen color");
    assert_ne!(fills[1], red, "the palette layer took the chosen color too");
}

// CPAL version 1 flags palettes for light and dark backgrounds: test_glyphs.ttf marks its second for
// dark and its third for light, in the table's stored values. Version 0 has no flags to give.
#[test]
fn each_palette_carries_its_flags() {
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let flags: Vec<_> = f.palettes().iter().map(|p| (p.index, p.light_safe, p.dark_safe, p.name_id)).collect();
    assert_eq!(flags, [(0, false, false, None), (1, false, true, None), (2, true, false, None)]);
    let v0 = font("bungee-tint/BungeeTint-Regular.ttf").palettes();
    assert_eq!(v0.len(), 8, "Bungee Tint has eight palettes");
    assert_eq!([f.palette_count(), font("bungee-tint/BungeeTint-Regular.ttf").palette_count()], [3, 8]);
    assert!(v0.iter().all(|p| !p.light_safe && !p.dark_safe && p.name_id.is_none()), "a version 0 palette had flags");
}

// Each palette's colors, the table's stored values; a palette past the font's reads palette 0, as drawing does.
#[test]
fn a_palette_gives_its_colors() {
    let rgba = |r, g, b| Rgba { r, g, b, a: 255 };
    let glyphs = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let first = glyphs.palette_colors(0);
    assert_eq!(first.len(), 14, "test_glyphs.ttf's palettes hold 14 entries");
    assert_eq!(first[..3], [rgba(255, 0, 0), rgba(255, 165, 0), rgba(255, 255, 0)]);
    assert_eq!(glyphs.palette_colors(2)[..2], [rgba(252, 113, 24), rgba(251, 129, 21)]);
    assert_eq!(glyphs.palette_colors(3), first, "a palette past the font's is not palette 0's");
    let bungee = font("bungee-tint/BungeeTint-Regular.ttf");
    assert_eq!(bungee.palette_colors(7), [rgba(255, 225, 11), rgba(255, 0, 53)]);
    assert!(font("inter/InterVariable.ttf").palette_colors(0).is_empty(), "a font without CPAL gave colors");
}

// A color goes to and from the bytes of an RGBA pixel, and a list emptied keeps its room.
#[test]
fn a_color_is_its_pixel_and_a_list_clears_in_place() {
    let teal = Rgba { r: 0, g: 128, b: 128, a: 200 };
    assert_eq!(Rgba::from([0, 128, 128, 200]), teal);
    assert_eq!(<[u8; 4]>::from(teal), [0, 128, 128, 200]);
    let f = font("colr-v1-test-glyphs/test_glyphs.ttf");
    let mut list = f.color_scene(GlyphId(148), &Location::default(), &SceneOptions::default())
        .expect("the scene draws").expect("glyph 148 has one");
    assert!(!list.is_empty(), "glyph 148's scene is empty");
    list.clear();
    assert!(list.is_empty() && list.paths().len() == 0, "the list held something after clearing");
}

// The emoji font's one strike is 109 ppem, so a smaller size is served from it, as a PNG.
#[test]
fn a_bitmap_glyph_comes_back_as_its_png() {
    let f = font("noto-color-emoji/NotoColorEmoji.ttf");
    let gid = f.glyph_id('\u{1F600}').expect("the font maps U+1F600");
    let b = f.bitmap(gid, 64).expect("a bitmap");
    let daegun::color::BitmapImage::Png(png) = &b.image else { panic!("the bitmap is not a PNG") };
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"), "the bitmap is not a PNG");
    assert_eq!(b.ppem, 109, "the strike's size");
    assert!(font("inter/InterVariable.ttf").bitmap(GlyphId(36), 64).is_none(), "an outline font gave a bitmap");
}

// Both formats report the image's top edge: CBDT's bearingY is it, and sbix's originOffsetY is the
// bottom, -27 here, so 101 once its 128-pixel height is added.
#[test]
fn sbix_and_cbdt_place_by_the_same_edge() {
    let (emoji, sbix) = (font("noto-color-emoji/NotoColorEmoji.ttf"), font("color-fonts-samples/samples-sbix.ttf"));
    let b = emoji.bitmap(GlyphId(883), 109).expect("a CBDT bitmap");
    assert_eq!((b.left, b.top), (0, 101));
    let b = sbix.bitmap(GlyphId(19), 109).expect("an sbix bitmap");
    assert_eq!((b.left, b.top, b.mirrored), (4, 101, false));
}
