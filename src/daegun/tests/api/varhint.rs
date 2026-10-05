use daegun::{Font, HintMode};

fn inter() -> Vec<u8> {
    let path = format!("{}/inter/InterVariable.ttf", crate::FONTS);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"))
}

fn outline(font: &Font, gid: u16, axes: &[(&str, f64)], mode: HintMode) -> daegun::Path {
    let mut p = daegun::Path::default();
    let g = font.prepared_outline(gid, 48.0, axes, &daegun::OutlineOptions::default().with_hinting(mode), &mut p)
        .expect("H has an outline");
    // Unhinted output still varies with the axes, so only this says the hinter ran.
    assert_eq!(g.hinted, mode != HintMode::None, "{mode:?} at {axes:?}: hinted is {}", g.hinted);
    p
}

#[test]
fn variable_axes_reach_the_hinted_shape() {
    let font = Font::from_bytes(&inter()).expect("parses");
    let gid = font.glyph_id('H' as u32).expect("H");
    for hinting in [HintMode::None, HintMode::Auto, HintMode::AutoForce] {
        assert_ne!(
            outline(&font, gid, &[("wght", 100.0)], hinting),
            outline(&font, gid, &[("wght", 900.0)], hinting),
            "{hinting:?}: wght 100 and wght 900 gave the same outline, so the axes never reached it",
        );
    }
}

#[test]
fn repeated_axis_values_are_stable() {
    let font = Font::from_bytes(&inter()).expect("parses");
    let gid = font.glyph_id('H' as u32).expect("H");
    let a = outline(&font, gid, &[("wght", 300.0)], HintMode::AutoForce);
    let _ = outline(&font, gid, &[("wght", 800.0)], HintMode::AutoForce);
    let b = outline(&font, gid, &[("wght", 300.0)], HintMode::AutoForce);
    assert_eq!(a, b, "the same axes gave a different outline either side of another location");
}
