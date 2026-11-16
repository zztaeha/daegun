use super::*;

// The level a plain sRGB blend of `text` over `back` lands on at alpha `t`.
fn blended(text: u8, back: u8, t: f64) -> f64 {
    f64::from(back) + (f64::from(text) - f64::from(back)) * t
}

// The level linear light lands on: coverage `a` of the text over its backdrop, encoded.
fn linear(text: u8, back: u8, a: f64) -> f64 {
    let l = f64::from(to_linear(text)) * a + f64::from(to_linear(back)) * (1.0 - a);
    f64::from(from_linear(l as f32))
}

#[test]
fn a_plain_blend_of_the_table_lands_where_linear_light_does() {
    for text in [0u8, 40, 100, 127, 128, 200, 255] {
        let back = 255 - text;
        let table = coverage_table(text, 0.0);
        assert_eq!((table[0], table[255]), (0, 255), "text {text}");
        assert!(table.windows(2).all(|w| w[0] <= w[1]), "text {text}: not monotone");
        // A level apart in the alpha moves the blend by |text - back| / 255, plus the encoded level's own half.
        let slack = 0.5 + f64::from(text.abs_diff(back)) / 255.0;
        for (i, &t) in table.iter().enumerate() {
            let a = i as f64 / 255.0;
            let gap = (blended(text, back, f64::from(t) / 255.0) - linear(text, back, a)).abs();
            assert!(gap <= slack + 0.5, "text {text}, coverage {i}: {gap} levels apart");
        }
    }
}

#[test]
fn contrast_thickens_dark_text_and_leaves_light_text_nearly_alone() {
    let (plain, thick) = (coverage_table(0, 0.0), coverage_table(0, 1.0));
    assert!(plain.iter().zip(&thick).all(|(p, t)| t >= p));
    assert!(thick[128] > plain[128] + 20, "{} against {}", thick[128], plain[128]);
    let (plain, thick) = (coverage_table(255, 0.0), coverage_table(255, 1.0));
    assert!(plain.iter().zip(&thick).all(|(p, t)| t.abs_diff(*p) <= 1));
    assert_eq!(coverage_table(0, 7.0), coverage_table(0, 1.0), "a contrast past 1 is 1");
    assert_eq!(coverage_table(0, f32::NAN), coverage_table(0, 0.0), "a contrast not a number is none");
}

#[test]
fn the_darkening_curve_runs_through_its_points() {
    for (stem, amount) in [(0.0, 0.4), (0.5, 0.4), (0.75, 0.3375), (1.0, 0.275), (1.5, 0.275), (2.0, 0.1375),
        (7.0 / 3.0, 0.0), (9.0, 0.0)]
    {
        assert!((darkening(stem) - amount).abs() < 1e-6, "{stem} px: {}", darkening(stem));
    }
    assert_eq!(darkening(f32::NAN), 0.0);
    assert_eq!(darkening(f32::INFINITY), 0.0);
    assert_eq!(darkening(-3.0), 0.4);
}
