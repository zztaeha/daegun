use super::*;
use crate::daecore::daetype::outline::OutlinePen;

// A planted outline no decode could produce, so this proves the lookup is wired rather than that a
// replay draws what a decode draws.
#[test]
fn the_outline_path_serves_a_prewarmed_outline() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    let f = Font::from_vec(std::fs::read(path).expect("fixture"), 0).expect("parses");
    let glyph = f.glyph_id('H').expect("H");
    let gid = glyph.0;
    let mut planted = Path::default();
    planted.move_to(1.0, 2.0);
    planted.line_to(30.0, 4.0);
    planted.line_to(5.0, 60.0);
    planted.close();
    // Planted as `prewarm` keeps an outline, its count in step, which a draw reads before the cache.
    let plant = |key| {
        let mut outlines = crate::sync::write(&f.outlines().list);
        outlines.insert(key, crate::sync::Shared::new(planted.clone()));
        f.outlines().prewarmed.set(outlines.len());
    };
    let bold = f.resolve(&[("wght", 700.0)]);
    plant((gid, f.cache.resolve_axes(&f.key(&bold))));

    let mut got = Path::default();
    f.outline(glyph, &bold, &mut got).expect("served");
    assert_eq!(got, planted, "the outline path decoded instead of replaying the prewarmed outline");
    let mut other = Path::default();
    f.outline(glyph, &f.resolve(&[("wght", 400.0)]), &mut other).expect("decoded");
    assert_ne!(other, planted, "another location was served the planted outline");
    assert_eq!(f.quads(glyph, &bold).map(|q| q.len()), Ok(3), "quads decoded past the cache");

    plant((gid, f.cache.resolve_axes(&crate::daecore::daekit::cache::AxisKey::default())));
    let mut plain = Path::default();
    f.outline(glyph, &Location::default(), &mut plain).expect("served");
    assert_eq!(plain, planted, "outline decoded instead of replaying the prewarmed outline");
}

// The farthest point found on the bits answers as `fits` over every coordinate: NaN, infinity, both zeros, the
// smallest and largest values, and either side of where 16 px of a 1,000-unit em reaches the limit.
#[test]
fn points_scale_inside_as_fits_reads_them() {
    use crate::daecore::daetype::hinting::f26dot6::fits;
    let size = Size::from_px(16.0).expect("a size");
    let edge = (1u64 << 29) as f32 * 1000.0 / (16.0 * 64.0);
    let values = [0.0, -0.0, 1.5, -700.25, f32::MIN_POSITIVE, f32::MAX, f32::MIN, f32::INFINITY, f32::NEG_INFINITY,
        f32::NAN, -f32::NAN, edge, -edge, edge.next_down(), -edge.next_down(), edge.next_up()];
    for (i, &a) in values.iter().enumerate() {
        for &b in &values[i..] {
            for upm in [1000, 0] {
                let mut pts = AutoPoints::default();
                (pts.x, pts.y) = (alloc::vec![a, 3.0], alloc::vec![-2.0, b]);
                let all = [a, 3.0, -2.0, b].map(f64::from);
                assert_eq!(scales_inside(&pts, size, upm), fits(all, size, upm), "{a} and {b} at {upm}");
            }
        }
    }
}
