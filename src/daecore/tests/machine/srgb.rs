use daegun::daecore::daemachine::daemath::srgb::{from_linear, to_linear};

fn curve(v: f32) -> u8 {
    let v = f64::from(v);
    let s = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
}

fn linear(level: u8) -> f32 {
    let c = f64::from(level) / 255.0;
    (if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }) as f32
}

#[test]
fn every_level_comes_back_through_linear_light() {
    for level in 0..=255u8 {
        assert_eq!(to_linear(level), linear(level), "level {level}");
        assert_eq!(from_linear(to_linear(level)), level, "level {level} did not come back");
    }
}

#[test]
fn linear_light_rounds_to_the_nearest_level_outside_0_to_1_too() {
    for (v, want) in [(f32::NAN, 0), (-1.0, 0), (-0.0, 0), (0.0, 0), (1.0, 255), (2.0, 255), (f32::INFINITY, 255)] {
        assert_eq!(from_linear(v), want, "{v}");
    }
    let mut checked = 0;
    for b in (0..=1.0f32.to_bits()).step_by(61) {
        let v = f32::from_bits(b);
        assert_eq!(from_linear(v), curve(v), "{v:e}");
        checked += 1;
    }
    assert!(checked > 17_000_000, "only {checked} values checked");
}

// Every f32 from 0 to 1, about a billion of them. Run by name; it takes a few seconds in release.
#[test]
#[ignore]
fn linear_light_rounds_to_the_nearest_level_everywhere() {
    for b in 0..=1.0f32.to_bits() {
        let v = f32::from_bits(b);
        assert_eq!(from_linear(v), curve(v), "{v:e}");
    }
}
