use daegun::{Bounds, Font, GlyphId, Location};

fn font(rel: &str) -> Font {
    let path = format!("{}/{}", crate::FONTS, rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture missing: {path} ({e})"));
    Font::from_bytes(&bytes).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn outline(
    f: &Font,
    gid: GlyphId,
    px: f32,
    opts: &daegun::outline::OutlineOptions,
) -> (daegun::outline::Path, daegun::outline::PreparedGlyph) {
    let mut p = daegun::outline::Path::default();
    let g = f.prepared_outline(gid, &Location::default(), px, opts, &mut p).expect("the glyph has an outline");
    (p, g)
}

fn size(p: &daegun::outline::Path) -> (f64, f64) {
    let Bounds { x_min: x0, y_min: y0, x_max: x1, y_max: y1 } = p.bounds().expect("ink");
    (x1 - x0, y1 - y0)
}

#[test]
fn embolden_thickens_the_outline() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H').expect("Inter maps H");
    let (plain, _) = outline(&f, gid, 48.0, &daegun::outline::OutlineOptions::default());
    let (bold, _) = outline(&f, gid, 48.0, &daegun::outline::OutlineOptions::default().with_embolden(120.0));
    let ((w0, h0), (w1, h1)) = (size(&plain), size(&bold));
    let want = 120.0 * 48.0 / f64::from(f.units_per_em());
    assert!((w1 - w0 - want).abs() < 0.5, "bold grew {}px wide, expected about {want}px", w1 - w0);
    assert!((h1 - h0 - want).abs() < 0.5, "bold grew {}px tall, expected about {want}px", h1 - h0);
}

// The ring an embolden adds must wind as the fill does, or a non-zero fill cancels where they overlap:
// CFF and CFF2 wind against TrueType, and a mirror turns any of them.
#[test]
fn an_embolden_keeps_all_of_the_glyph_whatever_its_winding() {
    let mirror = daegun::outline::OutlineOptions::default().with_transform([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
    for (rel, opts) in [
        ("stix-two-math/STIX2Math.otf", daegun::outline::OutlineOptions::default()),
        ("source-serif/SourceSerif4Variable-Roman.otf", daegun::outline::OutlineOptions::default()),
        ("inter/InterVariable.ttf", daegun::outline::OutlineOptions::default()),
        ("inter/InterVariable.ttf", mirror),
    ] {
        let f = font(rel);
        for ch in ['H', 'o', 'B'] {
            let gid = f.glyph_id(ch).expect("the font maps the letter");
            let (plain, _) = outline(&f, gid, 64.0, &opts);
            let (bold, _) = outline(&f, gid, 64.0, &opts.with_embolden(50.0));
            let fill = plain.flatten(0.01).expect("within the cap");
            let ring = bold.flatten(0.01).expect("within the cap");
            let Bounds { x_min: x0, y_min: y0, x_max: x1, y_max: y1 } = plain.bounds().expect("ink");
            let (mut ink, mut lost) = (0, 0);
            for i in 0..((x1 - x0) * 4.0) as i32 {
                for j in 0..((y1 - y0) * 4.0) as i32 {
                    let (x, y) = ((x0 + (f64::from(i) + 0.5) / 4.0) as f32, (y0 + (f64::from(j) + 0.5) / 4.0) as f32);
                    if crate::stroke::winding(&fill, x, y) != 0 {
                        ink += 1;
                        lost += i32::from(crate::stroke::winding(&ring, x, y) == 0);
                    }
                }
            }
            assert!(ink > 100, "{rel} {ch}: the plain glyph has no ink to keep");
            assert_eq!(lost, 0, "{rel} {ch}: emboldened, {lost} of {ink} samples of ink came out hollow");
        }
    }
}

#[test]
fn embolden_widens_the_advance_too() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('n').expect("Inter maps n");
    let px = 48.0;
    let (_, plain) = outline(&f, gid, px, &daegun::outline::OutlineOptions::default());
    let (_, bold) = outline(&f, gid, px, &daegun::outline::OutlineOptions::default().with_embolden(120.0));
    let delta = bold.advance_width - plain.advance_width;
    let expect = 120.0 * px / f32::from(f.units_per_em());
    assert!((delta - expect).abs() < 0.01, "the advance grew by {delta}px, expected {expect}px");
}

#[test]
fn oblique_leans_the_outline() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('I').expect("Inter maps I");
    let (upright, _) = outline(&f, gid, 64.0, &daegun::outline::OutlineOptions::default());
    let (slanted, _) = outline(&f, gid, 64.0, &daegun::outline::OutlineOptions::default().with_oblique(0.25));
    let ((w0, h0), (w1, h1)) = (size(&upright), size(&slanted));
    assert!((w1 - w0 - 0.25 * h0).abs() < 0.01, "slanting by 0.25 widened the I by {}, not {}", w1 - w0, 0.25 * h0);
    assert!((h1 - h0).abs() < 1e-3, "a shear along x changed the height");
    let Bounds { x_min: _, y_min: y0, x_max: _, y_max: y1 } = slanted.bounds().expect("ink");
    let rightmost = |near: f64| {
        slanted
            .parts()
            .1
            .iter()
            .filter(|p| (f64::from(p.y) - near).abs() < 1.0)
            .map(|p| f64::from(p.x))
            .fold(f64::MIN, f64::max)
    };
    assert!(rightmost(y1) > rightmost(y0), "the top does not lean right of the bottom");
}

#[test]
fn oblique_composes_with_a_transform() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('I').expect("Inter maps I");
    let half = [0.5f32, 0.0, 0.0, 0.5, 0.0, 0.0];
    let at = |o: daegun::outline::OutlineOptions| size(&outline(&f, gid, 48.0, &o).0);
    let scaled = at(daegun::outline::OutlineOptions::default().with_transform(half));
    let both = at(daegun::outline::OutlineOptions::default().with_transform(half).with_oblique(0.25));
    let plain = at(daegun::outline::OutlineOptions::default().with_oblique(0.25));
    assert!(both.0 > scaled.0 + 1.0, "the shear was dropped when a transform was present");
    assert!((both.1 - plain.1 / 2.0).abs() < 1e-3, "the half-scale transform was dropped");
}

#[test]
fn bad_synthetic_input_is_refused() {
    let f = font("inter/InterVariable.ttf");
    let gid = f.glyph_id('H').expect("Inter maps H");
    let mut sink = daegun::outline::Path::default();
    let nan = daegun::outline::OutlineOptions::default().with_oblique(f32::NAN);
    assert!(
        f.prepared_outline(gid, &Location::default(), 48.0, &nan, &mut sink).is_err(),
        "a NaN oblique was accepted"
    );
    let plain = outline(&f, gid, 48.0, &daegun::outline::OutlineOptions::default());
    for units in [0.0, -50.0] {
        let o = daegun::outline::OutlineOptions::default().with_embolden(units);
        assert_eq!(outline(&f, gid, 48.0, &o), plain, "embolden {units} changed the outline");
    }
    // Refused even beside the stroke that would leave it ignored, as every option must be finite.
    let stroke = daegun::outline::OutlineOptions::default().with_stroke(daegun::outline::StrokeStyle::new(20.0));
    for units in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        for o in [daegun::outline::OutlineOptions::default(), stroke].map(|o| o.with_embolden(units)) {
            let got = f.prepared_outline(gid, &Location::default(), 48.0, &o, &mut sink);
            let invalid = got.is_err_and(|e| e.kind() == daegun::ErrorKind::InvalidArgument);
            assert!(invalid && sink.is_empty(), "embolden {units} with {:?} was drawn", o.stroke);
        }
    }
}
