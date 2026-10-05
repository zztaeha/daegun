use daegun::daecore::daemachine::daemath::blend::{blend, composite, Rgb};
use daegun::daecore::daemachine::daemath::Blend;

const LEVELS: [f32; 9] = [0.0, 0.05, 0.25, 0.4, 0.5, 0.6, 0.75, 0.95, 1.0];

fn close(a: f32, b: f32, what: &str) {
    assert!(
        (a - b).abs() < 1e-5,
        "{what}: implementation gave {a}, the specification's formula gives {b}",
    );
}

fn spec_screen(cb: f32, cs: f32) -> f32 {
    cb + cs - (cb * cs)
}

fn spec_hard_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 { cb * (2.0 * cs) } else { spec_screen(cb, 2.0 * cs - 1.0) }
}

fn spec_color_dodge(cb: f32, cs: f32) -> f32 {
    if cb == 0.0 {
        0.0
    } else if cs == 1.0 {
        1.0
    } else {
        (cb / (1.0 - cs)).min(1.0)
    }
}

fn spec_color_burn(cb: f32, cs: f32) -> f32 {
    if cb == 1.0 {
        1.0
    } else if cs == 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - cb) / cs).min(1.0)
    }
}

fn spec_soft_light(cb: f32, cs: f32) -> f32 {
    if cs <= 0.5 {
        cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
    } else {
        let d = if cb <= 0.25 { ((16.0 * cb - 12.0) * cb + 4.0) * cb } else { cb.sqrt() };
        cb + (2.0 * cs - 1.0) * (d - cb)
    }
}

fn spec_separable(mode: Blend, cb: f32, cs: f32) -> f32 {
    match mode {
        Blend::Multiply => cb * cs,
        Blend::Screen => spec_screen(cb, cs),
        Blend::Overlay => spec_hard_light(cs, cb),
        Blend::Darken => cb.min(cs),
        Blend::Lighten => cb.max(cs),
        Blend::ColorDodge => spec_color_dodge(cb, cs),
        Blend::ColorBurn => spec_color_burn(cb, cs),
        Blend::HardLight => spec_hard_light(cb, cs),
        Blend::SoftLight => spec_soft_light(cb, cs),
        Blend::Difference => (cb - cs).abs(),
        Blend::Exclusion => cb + cs - 2.0 * cb * cs,
        other => panic!("{other:?} is not a separable blend mode"),
    }
}

#[test]
fn separable_blend_modes_match_the_specification() {
    let modes = [
        Blend::Multiply, Blend::Screen, Blend::Overlay, Blend::Darken, Blend::Lighten,
        Blend::ColorDodge, Blend::ColorBurn, Blend::HardLight, Blend::SoftLight,
        Blend::Difference, Blend::Exclusion,
    ];
    for mode in modes {
        for &cb in &LEVELS {
            for &cs in &LEVELS {
                let backdrop: Rgb = [cb, cs, 1.0 - cb];
                let source: Rgb = [cs, cb, 1.0 - cs];
                let got = blend(mode, backdrop, source);
                for c in 0..3 {
                    close(
                        got[c],
                        spec_separable(mode, backdrop[c], source[c]),
                        &format!("{mode:?} channel {c} over Cb={} Cs={}", backdrop[c], source[c]),
                    );
                }
            }
        }
    }
}

fn spec_lum(c: Rgb) -> f32 {
    0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn spec_sat(c: Rgb) -> f32 {
    c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

fn spec_clip_color(c: Rgb) -> Rgb {
    let l = spec_lum(c);
    let n = c[0].min(c[1]).min(c[2]);
    let x = c[0].max(c[1]).max(c[2]);
    let mut out = c;
    if n < 0.0 && (l - n) != 0.0 {
        for v in &mut out {
            *v = l + (((*v - l) * l) / (l - n));
        }
    }
    if x > 1.0 && (x - l) != 0.0 {
        for v in &mut out {
            *v = l + (((*v - l) * (1.0 - l)) / (x - l));
        }
    }
    out
}

fn spec_set_lum(c: Rgb, l: f32) -> Rgb {
    let d = l - spec_lum(c);
    spec_clip_color([c[0] + d, c[1] + d, c[2] + d])
}

fn spec_set_sat(c: Rgb, s: f32) -> Rgb {
    let mut order = [0usize, 1, 2];
    order.sort_by(|&a, &b| c[a].partial_cmp(&c[b]).unwrap());
    let (lo, mid, hi) = (order[0], order[1], order[2]);
    let mut out = [0.0f32; 3];
    if c[hi] > c[lo] {
        out[mid] = ((c[mid] - c[lo]) * s) / (c[hi] - c[lo]);
        out[hi] = s;
    }
    out
}

fn spec_non_separable(mode: Blend, cb: Rgb, cs: Rgb) -> Rgb {
    match mode {
        Blend::HslHue => spec_set_lum(spec_set_sat(cs, spec_sat(cb)), spec_lum(cb)),
        Blend::HslSaturation => spec_set_lum(spec_set_sat(cb, spec_sat(cs)), spec_lum(cb)),
        Blend::HslColor => spec_set_lum(cs, spec_lum(cb)),
        Blend::HslLuminosity => spec_set_lum(cb, spec_lum(cs)),
        other => panic!("{other:?} is not a non-separable blend mode"),
    }
}

#[test]
fn non_separable_blend_modes_match_the_specification() {
    let colors: [Rgb; 8] = [
        [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0],
        [0.2, 0.5, 0.9], [0.9, 0.5, 0.2], [0.5, 0.5, 0.5],
        [1.0, 1.0, 0.0], [0.05, 0.1, 0.95],
    ];
    let modes = [Blend::HslHue, Blend::HslSaturation, Blend::HslColor, Blend::HslLuminosity];
    for mode in modes {
        for cb in colors {
            for cs in colors {
                let got = blend(mode, cb, cs);
                let want = spec_non_separable(mode, cb, cs);
                for c in 0..3 {
                    close(got[c], want[c], &format!("{mode:?} channel {c} for Cb={cb:?} Cs={cs:?}"));
                }
            }
        }
    }
}

#[test]
fn the_hsl_modes_keep_the_luminosity_they_promise() {
    let cb: Rgb = [0.2, 0.6, 0.9];
    let cs: Rgb = [0.8, 0.1, 0.35];
    for mode in [Blend::HslHue, Blend::HslSaturation, Blend::HslColor] {
        close(spec_lum(blend(mode, cb, cs)), spec_lum(cb), &format!("{mode:?} moved the luminosity"));
    }
    close(
        spec_lum(blend(Blend::HslLuminosity, cb, cs)),
        spec_lum(cs),
        "HslLuminosity did not take the source's luminosity",
    );
}

fn spec_coefficients(mode: Blend, a_s: f32, a_b: f32) -> (f32, f32) {
    match mode {
        Blend::Clear => (0.0, 0.0),
        Blend::Src => (1.0, 0.0),
        Blend::Dest => (0.0, 1.0),
        Blend::SrcOver => (1.0, 1.0 - a_s),
        Blend::DestOver => (1.0 - a_b, 1.0),
        Blend::SrcIn => (a_b, 0.0),
        Blend::DestIn => (0.0, a_s),
        Blend::SrcOut => (1.0 - a_b, 0.0),
        Blend::DestOut => (0.0, 1.0 - a_s),
        Blend::SrcAtop => (a_b, 1.0 - a_s),
        Blend::DestAtop => (1.0 - a_b, a_s),
        Blend::Xor => (1.0 - a_b, 1.0 - a_s),
        Blend::Plus => (1.0, 1.0),
        _ => (1.0, 1.0 - a_s),
    }
}

#[test]
fn compositing_matches_the_specification_for_every_mode() {
    const ALL: [Blend; 28] = [
        Blend::Clear, Blend::Src, Blend::Dest, Blend::SrcOver, Blend::DestOver, Blend::SrcIn,
        Blend::DestIn, Blend::SrcOut, Blend::DestOut, Blend::SrcAtop, Blend::DestAtop, Blend::Xor,
        Blend::Plus, Blend::Screen, Blend::Overlay, Blend::Darken, Blend::Lighten,
        Blend::ColorDodge, Blend::ColorBurn, Blend::HardLight, Blend::SoftLight, Blend::Difference,
        Blend::Exclusion, Blend::Multiply, Blend::HslHue, Blend::HslSaturation, Blend::HslColor,
        Blend::HslLuminosity,
    ];
    let cs: Rgb = [0.8, 0.35, 0.1];
    let cb: Rgb = [0.2, 0.55, 0.95];
    for mode in ALL {
        for &a_s in &[0.0f32, 0.25, 0.6, 1.0] {
            for &a_b in &[0.0f32, 0.4, 0.75, 1.0] {
                let b = blend(mode, cb, cs);
                let mixed = [
                    (1.0 - a_b) * cs[0] + a_b * b[0],
                    (1.0 - a_b) * cs[1] + a_b * b[1],
                    (1.0 - a_b) * cs[2] + a_b * b[2],
                ];
                let (fa, fb) = spec_coefficients(mode, a_s, a_b);
                let (got, got_a) = composite(mode, cs, a_s, cb, a_b);
                for c in 0..3 {
                    let want = (a_s * fa * mixed[c] + a_b * fb * cb[c]).clamp(0.0, 1.0);
                    close(got[c], want, &format!("{mode:?} channel {c} at as={a_s} ab={a_b}"));
                }
                close(got_a, (a_s * fa + a_b * fb).clamp(0.0, 1.0), &format!("{mode:?} alpha"));
            }
        }
    }
}

#[test]
fn compositing_holds_the_identities_the_operators_are_named_for() {
    let cs: Rgb = [0.8, 0.35, 0.1];
    let cb: Rgb = [0.2, 0.55, 0.95];

    let (c, a) = composite(Blend::Clear, cs, 1.0, cb, 1.0);
    assert_eq!((c, a), ([0.0; 3], 0.0), "Clear left something behind");

    let (c, a) = composite(Blend::Src, cs, 1.0, cb, 1.0);
    for i in 0..3 {
        close(c[i], cs[i], "Src did not give the source");
    }
    close(a, 1.0, "Src alpha");

    let (c, a) = composite(Blend::Dest, cs, 1.0, cb, 1.0);
    for i in 0..3 {
        close(c[i], cb[i], "Dest did not give the backdrop");
    }
    close(a, 1.0, "Dest alpha");

    let (c, a) = composite(Blend::SrcOver, cs, 1.0, cb, 0.5);
    for i in 0..3 {
        close(c[i], cs[i], "an opaque SrcOver did not cover the backdrop");
    }
    close(a, 1.0, "an opaque SrcOver was not opaque");

    let (c, a) = composite(Blend::SrcOver, cs, 0.0, cb, 1.0);
    for i in 0..3 {
        close(c[i], cb[i], "a transparent SrcOver disturbed the backdrop");
    }
    close(a, 1.0, "a transparent SrcOver changed the alpha");

    let (_, a) = composite(Blend::Xor, cs, 1.0, cb, 1.0);
    close(a, 0.0, "Xor of two opaque layers was not empty");
}

#[test]
fn from_colr_maps_every_composite_mode_to_its_own_index() {
    const IN_ORDER: [(u8, Blend); 28] = [
        (0, Blend::Clear), (1, Blend::Src), (2, Blend::Dest), (3, Blend::SrcOver),
        (4, Blend::DestOver), (5, Blend::SrcIn), (6, Blend::DestIn), (7, Blend::SrcOut),
        (8, Blend::DestOut), (9, Blend::SrcAtop), (10, Blend::DestAtop), (11, Blend::Xor),
        (12, Blend::Plus), (13, Blend::Screen), (14, Blend::Overlay), (15, Blend::Darken),
        (16, Blend::Lighten), (17, Blend::ColorDodge), (18, Blend::ColorBurn),
        (19, Blend::HardLight), (20, Blend::SoftLight), (21, Blend::Difference),
        (22, Blend::Exclusion), (23, Blend::Multiply), (24, Blend::HslHue),
        (25, Blend::HslSaturation), (26, Blend::HslColor), (27, Blend::HslLuminosity),
    ];
    for (value, expected) in IN_ORDER {
        assert_eq!(
            Blend::from_colr(value), expected,
            "CompositeMode {value} decoded as {:?} rather than {expected:?}",
            Blend::from_colr(value),
        );
        assert_eq!(
            expected as u8, value,
            "{expected:?} sits at {} in the enum but is CompositeMode {value} in COLR",
            expected as u8,
        );
    }
    // The spec: "If an unrecognized value is encountered, COMPOSITE_CLEAR must be used."
    for value in [28u8, 29, 100, 255] {
        assert_eq!(
            Blend::from_colr(value), Blend::Clear,
            "an unknown CompositeMode {value} did not clear",
        );
    }
    assert_eq!(Blend::default(), Blend::SrcOver, "the default composite mode is not SrcOver");
}

use daegun::daecore::daemachine::daemath::gradient::{Interpolation, Ramp};
use daegun::daecore::daemachine::daemath::{resolve_stops, Extend, Gradient, GradientKind, Rgba, Stop, Stops};
use daegun::daecore::daemachine::daemath::matrix::IDENTITY;

// Black to white in sRGB as stored, so a color's red level reads back the t that made it.
fn ramp_of(kind: GradientKind, extend: Extend) -> Ramp {
    let g = Gradient {
        kind,
        stops: vec![
            Stop { offset: 0.0, color: Rgba { r: 0, g: 0, b: 0, a: 255 } },
            Stop { offset: 1.0, color: Rgba { r: 255, g: 255, b: 255, a: 255 } },
        ],
        extend,
        transform: IDENTITY,
    };
    Ramp::with_interpolation(&g, &IDENTITY, Interpolation::Srgb)
}

fn t_at(r: &Ramp, x: f64, y: f64) -> f64 {
    f64::from(r.at(x, y).expect("the gradient painted nothing here").r) / 255.0
}

#[test]
fn extend_modes_follow_their_definitions_outside_the_span() {
    let line = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 };
    let raw = |x: f64| (x + 0.5) / 10.0;

    let spec = |t: f64, e: Extend| match e {
        Extend::Pad => t.clamp(0.0, 1.0),
        Extend::Repeat => t - t.floor(),
        Extend::Reflect => {
            let f = t - 2.0 * (t / 2.0).floor();
            if f > 1.0 { 2.0 - f } else { f }
        }
    };

    for e in [Extend::Pad, Extend::Repeat, Extend::Reflect] {
        let r = ramp_of(line, e);
        for x in [-31.0, -23.0, -13.0, -3.0, 0.0, 4.5, 9.5, 12.0, 19.0, 25.0, 34.0] {
            let want = spec(raw(x), e);
            let got = t_at(&r, x, 0.0);
            assert!(
                (got - want).abs() < 0.005,
                "{e:?} at x={x}: raw t {} became {got}, the definition gives {want}", raw(x),
            );
        }
    }

    let (pad, rep, refl) = (
        ramp_of(line, Extend::Pad), ramp_of(line, Extend::Repeat), ramp_of(line, Extend::Reflect),
    );
    let x = 12.0;
    let (a, b, c) = (t_at(&pad, x, 0.0), t_at(&rep, x, 0.0), t_at(&refl, x, 0.0));
    assert!(
        (a - b).abs() > 0.2 && (b - c).abs() > 0.2 && (a - c).abs() > 0.1,
        "the three extend modes agreed at t=1.25: pad {a}, repeat {b}, reflect {c}",
    );
}

#[test]
fn the_three_gradient_geometries_give_the_parameter_their_definitions_say() {
    let line = ramp_of(GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 }, Extend::Pad);
    for (x, want) in [(-0.5, 0.0), (4.5, 0.5), (2.0, 0.25), (7.0, 0.75), (9.5, 1.0)] {
        assert!(
            (t_at(&line, x, 0.0) - want).abs() < 0.005,
            "linear at x={x} gave {}, not {want}", t_at(&line, x, 0.0),
        );
    }

    let rad = ramp_of(
        GradientKind::Radial { x0: 0.0, y0: 0.0, r0: 0.0, x1: 0.0, y1: 0.0, r1: 10.0 },
        Extend::Pad,
    );
    for (x, y, want) in [(-0.5, -0.5, 0.0), (4.5, -0.5, 0.5), (-0.5, 4.5, 0.5), (2.5, -0.5, 0.3)] {
        assert!(
            (t_at(&rad, x, y) - want).abs() < 0.005,
            "radial at ({x}, {y}) gave {}, not {want}", t_at(&rad, x, y),
        );
    }

    let cone = ramp_of(
        GradientKind::Radial { x0: 0.0, y0: 0.0, r0: 4.0, x1: 4.0, y1: 0.0, r1: 1.0 },
        Extend::Pad,
    );
    let got = t_at(&cone, 0.5, -0.5);
    assert!(
        (got - 0.7143).abs() < 0.005,
        "the larger root did not win: got {got}, the two roots are 0.7143 and -3.0 and both \
         describe a circle through this point",
    );

    let sweep = ramp_of(
        GradientKind::Sweep { cx: 0.0, cy: 0.0, start_angle: 0.0, end_angle: 360.0 },
        Extend::Pad,
    );
    for (x, y, want) in [(9.5, -0.5, 0.0), (-0.5, 9.5, 0.25), (-9.5, -0.5, 0.5), (-0.5, -9.5, 0.75)] {
        assert!(
            (t_at(&sweep, x, y) - want).abs() < 0.005,
            "sweep at ({x}, {y}) gave {}, not {want}", t_at(&sweep, x, y),
        );
    }
}

#[test]
fn a_radial_leaves_unpainted_what_no_interpolated_circle_reaches() {
    let g = Gradient {
        kind: GradientKind::Radial { x0: 0.0, y0: 0.0, r0: 1.0, x1: 4.0, y1: 0.0, r1: 2.0 },
        stops: vec![
            Stop { offset: 0.0, color: Rgba::opaque(255, 0, 0) },
            Stop { offset: 1.0, color: Rgba::opaque(0, 0, 255) },
        ],
        extend: Extend::Pad,
        transform: IDENTITY,
    };
    let r = Ramp::new(&g, &IDENTITY);
    assert!(r.at(0.0, 0.0).is_some(), "a point inside the cone was left unpainted");
    assert!(
        r.at(-400.0, 300.0).is_none(),
        "a point far behind the cone's apex was painted instead of left alone",
    );
}

#[test]
fn resolve_stops_normalizes_the_way_the_specification_asks() {
    let c = |v: u8| Rgba::opaque(v, v, v);

    let messy = vec![
        Stop { offset: 2.0, color: c(30) },
        Stop { offset: -1.0, color: c(10) },
        Stop { offset: 0.5, color: c(20) },
    ];
    let Stops::Many(sorted) = resolve_stops(messy) else { panic!("three stops did not stay many") };
    let offsets: Vec<f32> = sorted.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, vec![0.0, 0.5, 1.0], "offsets were not sorted and clamped into [0,1]");
    assert_eq!(sorted[0].color, c(10), "the sort did not carry the colors with the offsets");

    let cycle = [0.9f32, 0.5, 0.1];
    let hard: Vec<Stop> =
        (0..30u8).map(|i| Stop { offset: cycle[usize::from(i) % 3], color: c(i) }).collect();
    let Stops::Many(kept) = resolve_stops(hard) else { panic!("the stops did not stay many") };
    let order: Vec<u8> = kept.iter().map(|s| s.color.r).collect();
    let expected: Vec<u8> = [2u8, 1, 0]
        .iter()
        .flat_map(|start| (0..10u8).map(move |k| start + k * 3))
        .collect();
    assert_eq!(
        order, expected,
        "stops sharing an offset lost their document order, which reverses a hard color break",
    );

    let nan = vec![
        Stop { offset: f32::NAN, color: c(9) },
        Stop { offset: 0.5, color: c(8) },
    ];
    let Stops::Many(fixed) = resolve_stops(nan) else { panic!("two stops did not stay many") };
    assert!(fixed.iter().all(|s| s.offset.is_finite()), "a NaN offset survived normalization");

    assert!(matches!(resolve_stops(Vec::new()), Stops::Nothing));
    assert!(matches!(resolve_stops(vec![Stop { offset: 0.3, color: c(7) }]), Stops::Solid(s) if s == c(7)));
}

#[test]
fn a_sweep_reproduces_both_examples_the_specification_works_through() {
    let sweep = |start: f32, end: f32| {
        Ramp::with_interpolation(
            &Gradient {
                kind: GradientKind::Sweep { cx: 0.0, cy: 0.0, start_angle: start, end_angle: end },
                stops: vec![
                    Stop { offset: 0.0, color: Rgba::opaque(255, 255, 0) },
                    Stop { offset: 1.0, color: Rgba::opaque(255, 0, 0) },
                ],
                extend: Extend::Pad,
                transform: IDENTITY,
            },
            &IDENTITY,
            Interpolation::Srgb,
        )
    };
    let ray = |r: &Ramp, deg: f64| {
        let (rad, dist) = (deg.to_radians(), 4096.0);
        let c = r.at(dist * rad.cos() - 0.5, dist * rad.sin() - 0.5).expect("the sweep left a ray unpainted");
        f64::from(c.g) / 255.0
    };
    let yellow = |v: f64| v > 0.995;
    let red = |v: f64| v < 0.005;

    let a = sweep(330.0, 400.0);
    for deg in [0.0, 45.0, 120.0, 200.0, 329.0] {
        assert!(yellow(ray(&a, deg)), "330..400: {deg} should be yellow, got {}", ray(&a, deg));
    }
    assert!(yellow(ray(&a, 330.0)), "330..400: the start angle itself should still be yellow");
    for (deg, want) in [(345.0, 15.0 / 70.0), (359.0, 29.0 / 70.0)] {
        let got = 1.0 - ray(&a, deg);
        assert!(
            (got - want).abs() < 0.01,
            "330..400: at {deg} the ramp is {got} across, the example puts it at {want}",
        );
    }
    assert!(
        1.0 - ray(&a, 359.9) < 0.44,
        "330..400: the arc past 360 was sampled, which the example says is not drawn",
    );

    let b = sweep(210.0, 110.0);
    for deg in [211.0, 250.0, 300.0, 359.0] {
        assert!(yellow(ray(&b, deg)), "210..110: {deg} should be solid yellow, got {}", ray(&b, deg));
    }
    for (deg, want) in [(210.0, 0.0), (185.0, 0.25), (160.0, 0.5), (135.0, 0.75), (110.0, 1.0)] {
        let got = 1.0 - ray(&b, deg);
        assert!(
            (got - want).abs() < 0.01,
            "210..110: at {deg} the ramp is {got} across, the example puts it at {want}",
        );
    }
    for deg in [109.0, 90.0, 45.0, 0.0] {
        assert!(red(ray(&b, deg)), "210..110: {deg} should be solid red, got {}", ray(&b, deg));
    }
}

#[test]
fn an_ascending_sweep_can_still_reach_its_start_color() {
    let g = Gradient {
        kind: GradientKind::Sweep { cx: 0.0, cy: 0.0, start_angle: 90.0, end_angle: 180.0 },
        stops: vec![
            Stop { offset: 0.0, color: Rgba::opaque(255, 255, 0) },
            Stop { offset: 1.0, color: Rgba::opaque(255, 0, 0) },
        ],
        extend: Extend::Pad,
        transform: IDENTITY,
    };
    let r = Ramp::with_interpolation(&g, &IDENTITY, Interpolation::Srgb);
    let ray = |deg: f64| {
        let rad: f64 = deg.to_radians();
        r.at(4096.0 * rad.cos() - 0.5, 4096.0 * rad.sin() - 0.5).expect("unpainted").g
    };
    for deg in [0.0, 30.0, 89.0] {
        assert_eq!(ray(deg), 255, "at {deg}, before the sweep starts, Pad did not hold the first stop");
    }
    let mid = ray(135.0);
    assert!((126..=129).contains(&mid), "the midpoint of the arc read {mid}, not about half of 255");
    for deg in [181.0, 270.0, 359.0] {
        assert_eq!(ray(deg), 0, "at {deg}, after the sweep ends, Pad did not hold the last stop");
    }
}

// COLR requires a degenerate linear or radial to paint nothing, and a sweep with coincident angles
// to draw nothing under reflect and repeat.
#[test]
fn a_degenerate_gradient_paints_nothing_rather_than_its_last_stop() {
    use daegun::paint::gradient::Ramp;
    use daegun::paint::{Extend, Gradient, GradientKind, IDENTITY, Rgba, Stop};

    let stops = || {
        vec![
            Stop { offset: 0.0, color: Rgba::opaque(255, 0, 0) },
            Stop { offset: 1.0, color: Rgba::opaque(0, 0, 255) },
        ]
    };
    let ramp = |kind, extend| {
        Ramp::new(&Gradient { kind, stops: stops(), extend, transform: IDENTITY }, &IDENTITY)
    };
    let probes = [(5.0, 5.0), (20.0, 20.0), (35.0, 35.0), (-12.0, 7.0)];

    let degenerate: [(&str, GradientKind); 3] = [
        ("linear with p0 == p1", GradientKind::Linear { x0: 10.0, y0: 10.0, x1: 10.0, y1: 10.0 }),
        (
            "radial with identical circles",
            GradientKind::Radial { x0: 20.0, y0: 20.0, r0: 8.0, x1: 20.0, y1: 20.0, r1: 8.0 },
        ),
        (
            "sweep with coincident angles",
            GradientKind::Sweep { cx: 20.0, cy: 20.0, start_angle: 45.0, end_angle: 45.0 },
        ),
    ];

    for (what, kind) in degenerate {
        for extend in [Extend::Pad, Extend::Reflect, Extend::Repeat] {
            let r = ramp(kind, extend);
            for (x, y) in probes {
                assert_eq!(r.at(x, y), None, "{what} under {extend:?} painted at ({x}, {y})");
            }
        }
    }

    // The control: move one endpoint and the same gradients paint again, so the guard is catching
    // degeneracy rather than switching gradients off.
    let ordinary: [(&str, GradientKind); 3] = [
        ("linear", GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 40.0, y1: 0.0 }),
        ("radial", GradientKind::Radial { x0: 20.0, y0: 20.0, r0: 0.0, x1: 20.0, y1: 20.0, r1: 30.0 }),
        ("sweep", GradientKind::Sweep { cx: 20.0, cy: 20.0, start_angle: 45.0, end_angle: 315.0 }),
    ];
    for (what, kind) in ordinary {
        let r = ramp(kind, Extend::Pad);
        assert!(
            probes.iter().any(|&(x, y)| r.at(x, y).is_some()),
            "an ordinary {what} gradient painted nothing anywhere",
        );
    }
}

// A sweep half a degree wide magnifies its angle error 720 times, so 1.5 levels of 255 here bound it
// near 0.003°: six polynomial terms pass, tiny-skia's four (0.005°) do not.
#[test]
fn a_narrow_sweep_follows_the_true_angle() {
    for start in [10.0f32, 50.0, 100.0, 170.0, 200.0, 260.0, 300.0, 350.0] {
        let sweep = GradientKind::Sweep { cx: 0.0, cy: 0.0, start_angle: start, end_angle: start + 0.5 };
        let r = ramp_of(sweep, Extend::Pad);
        for k in 0..=20 {
            let deg = f64::from(start) + 0.025 * f64::from(k);
            let (x, y) = (100.0 * deg.to_radians().cos(), 100.0 * deg.to_radians().sin());
            let want = (deg - f64::from(start)) / 0.5;
            let got = t_at(&r, x - 0.5, y - 0.5);
            assert!((got - want).abs() <= 1.5 / 255.0, "at {deg}° the sweep read {got}, not {want}");
        }
    }
}

// The definition worked in f64, sRGB as stored: a linear gradient moved and scaled by its transform,
// through stops whose intervals start past 0, is within one level of 255 of it at every pixel.
#[test]
fn a_moved_linear_gradient_matches_its_definition() {
    use daegun::daecore::daemachine::daemath::matrix::invert;
    let stops = vec![
        Stop { offset: 0.2, color: Rgba { r: 200, g: 10, b: 40, a: 255 } },
        Stop { offset: 0.6, color: Rgba { r: 20, g: 220, b: 90, a: 128 } },
        Stop { offset: 0.9, color: Rgba { r: 60, g: 30, b: 250, a: 255 } },
    ];
    let (x0, y0, x1, y1) = (5.0f32, -3.0, 90.0, 40.0);
    let transform = [1.25, 0.1, -0.2, 0.8, 17.0, -9.0];
    let kind = GradientKind::Linear { x0, y0, x1, y1 };
    let r = Ramp::with_interpolation(
        &Gradient { kind, stops: stops.clone(), extend: Extend::Pad, transform },
        &IDENTITY,
        Interpolation::Srgb,
    );
    let inv = invert(&transform).expect("invertible");
    let (ax, ay) = (f64::from(x1 - x0), f64::from(y1 - y0));
    let channel = |c: Rgba| [c.r, c.g, c.b, c.a].map(f64::from);
    for y in 0..60 {
        for x in 0..140 {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let (gx, gy) = (inv[0] * px + inv[2] * py + inv[4], inv[1] * px + inv[3] * py + inv[5]);
            let t = (((gx - f64::from(x0)) * ax + (gy - f64::from(y0)) * ay) / (ax * ax + ay * ay)).clamp(0.0, 1.0);
            let want = match stops.windows(2).find(|w| t <= f64::from(w[1].offset)) {
                _ if t <= 0.2 => channel(stops[0].color),
                Some(w) => {
                    let f = (t - f64::from(w[0].offset)) / f64::from(w[1].offset - w[0].offset);
                    let (a, b) = (channel(w[0].color), channel(w[1].color));
                    [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * f)
                }
                None => channel(stops[2].color),
            };
            let got = channel(r.at(f64::from(x), f64::from(y)).expect("a color"));
            for i in 0..4 {
                assert!((got[i] - want[i]).abs() <= 1.0, "at ({x}, {y}) channel {i} is {}, not {}", got[i], want[i]);
            }
        }
    }
}

// A color between two stops rounds to the nearest level of 255; truncating would darken every gradient.
#[test]
fn a_gradient_color_rounds_to_the_nearest_level() {
    let r = ramp_of(GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 }, Extend::Pad);
    for (x, want) in [(4.5, 128), (2.0, 64), (7.0, 191)] {
        let got = r.at(x, 0.0).expect("a color");
        assert_eq!((got.r, got.g, got.b), (want, want, want), "at x {x}");
    }
}

// Halfway from opaque red to transparent blue: the spec premultiplies, so no blue leaks in; sRGB as
// stored, straight, goes through purple. Black to white: linear light lands at 188, not 128.
#[test]
fn a_gradient_blends_in_linear_light_with_alpha_premultiplied() {
    let halfway = |from: Rgba, to: Rgba, how| {
        let stops = vec![Stop { offset: 0.0, color: from }, Stop { offset: 1.0, color: to }];
        let kind = GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 };
        let g = Gradient { kind, stops, extend: Extend::Pad, transform: IDENTITY };
        let c = Ramp::with_interpolation(&g, &IDENTITY, how).at(4.5, 0.0).expect("a color");
        (c.r, c.g, c.b, c.a)
    };
    let (red, clear_blue) = (Rgba::opaque(255, 0, 0), Rgba { r: 0, g: 0, b: 255, a: 0 });
    let (black, white) = (Rgba::opaque(0, 0, 0), Rgba::opaque(255, 255, 255));
    assert_eq!(halfway(red, clear_blue, Interpolation::LinearLight), (255, 0, 0, 128));
    assert_eq!(halfway(red, clear_blue, Interpolation::Srgb), (128, 0, 128, 128));
    assert_eq!(halfway(black, white, Interpolation::LinearLight), (188, 188, 188, 255));
    assert_eq!(halfway(black, white, Interpolation::Srgb), (128, 128, 128, 255));
    let g = Gradient {
        kind: GradientKind::Linear { x0: 0.0, y0: 0.0, x1: 10.0, y1: 0.0 },
        stops: vec![Stop { offset: 0.0, color: black }, Stop { offset: 1.0, color: white }],
        extend: Extend::Pad,
        transform: IDENTITY,
    };
    let spec = Ramp::with_interpolation(&g, &IDENTITY, Interpolation::LinearLight);
    assert_eq!(Ramp::new(&g, &IDENTITY).at(4.5, 0.0), spec.at(4.5, 0.0), "the default is not the spec's way");
}

// The spec's algorithm worked in f64 with the exact sRGB curve: linearize, premultiply, interpolate,
// divide by alpha, encode. Translucent stops past 0, under a moving transform, land within one level.
#[test]
fn a_gradient_matches_the_spec_interpolation_worked_in_f64() {
    use daegun::daecore::daemachine::daemath::matrix::invert;
    let stops = vec![
        Stop { offset: 0.1, color: Rgba { r: 200, g: 10, b: 40, a: 255 } },
        Stop { offset: 0.45, color: Rgba { r: 20, g: 220, b: 90, a: 30 } },
        Stop { offset: 0.7, color: Rgba { r: 250, g: 250, b: 5, a: 0 } },
        Stop { offset: 0.9, color: Rgba { r: 60, g: 30, b: 250, a: 255 } },
    ];
    let (x0, y0, x1, y1) = (5.0f32, -3.0, 90.0, 40.0);
    let transform = [1.25, 0.1, -0.2, 0.8, 17.0, -9.0];
    let kind = GradientKind::Linear { x0, y0, x1, y1 };
    let r = Ramp::new(&Gradient { kind, stops: stops.clone(), extend: Extend::Pad, transform }, &IDENTITY);
    let inv = invert(&transform).expect("invertible");
    let (ax, ay) = (f64::from(x1 - x0), f64::from(y1 - y0));
    let decode = |v: u8| {
        let c = f64::from(v) / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let encode = |v: f64| {
        let s = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
        (s * 255.0 + 0.5).floor().clamp(0.0, 255.0)
    };
    let premultiplied = |c: Rgba| {
        let a = f64::from(c.a) / 255.0;
        [decode(c.r) * a, decode(c.g) * a, decode(c.b) * a, a]
    };
    let level = |c: Rgba| [c.r, c.g, c.b, c.a].map(f64::from);
    let mut blended = 0;
    for y in 0..60 {
        for x in 0..140 {
            let (px, py) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            let (gx, gy) = (inv[0] * px + inv[2] * py + inv[4], inv[1] * px + inv[3] * py + inv[5]);
            let t = ((gx - f64::from(x0)) * ax + (gy - f64::from(y0)) * ay) / (ax * ax + ay * ay);
            let want = if t <= 0.1 {
                level(stops[0].color)
            } else if t >= 0.9 {
                level(stops[3].color)
            } else {
                let w = stops.windows(2).find(|w| t <= f64::from(w[1].offset)).expect("an interval");
                let f = (t - f64::from(w[0].offset)) / f64::from(w[1].offset - w[0].offset);
                let (a, b) = (premultiplied(w[0].color), premultiplied(w[1].color));
                let p = [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * f);
                blended += 1;
                if p[3] <= 0.0 {
                    [0.0; 4]
                } else {
                    [encode(p[0] / p[3]), encode(p[1] / p[3]), encode(p[2] / p[3]), (p[3] * 255.0 + 0.5).floor()]
                }
            };
            let got = level(r.at(f64::from(x), f64::from(y)).expect("a color"));
            for i in 0..4 {
                assert!((got[i] - want[i]).abs() <= 1.0, "at ({x}, {y}) channel {i} is {}, not {}", got[i], want[i]);
            }
        }
    }
    assert!(blended > 3000, "only {blended} pixels fell between stops");
}
