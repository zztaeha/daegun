use super::*;
use crate::daecore::Tag;
use alloc::vec::Vec;

fn font(rel: &str) -> Font {
    let path = alloc::format!("{}/assets/test-fonts/{rel}", env!("CARGO_MANIFEST_DIR"));
    Font::from_vec(std::fs::read(path).expect("the fixture"), 0).expect("it parses")
}

const WGHT: Tag = Tag::new(b"wght");
const OPSZ: Tag = Tag::new(b"opsz");

// A location held past the font that made it names no font open since, which reads it from the values asked for.
#[test]
fn a_location_outlives_the_font_that_made_it() {
    let at = font("inter/InterVariable.ttf").location(&[(WGHT, 700.0)]);
    let again = font("inter/InterVariable.ttf");
    let advance = |at: &Location| again.advance(crate::GlyphId(36), at, crate::Orientation::Horizontal);
    let own = again.location(&[(WGHT, 700.0)]);
    assert_eq!(advance(&at), advance(&own));
    assert_ne!(advance(&own), advance(&Location::default()));
}

// Inter's opsz runs 14 to 32 and wght 100 to 900: past either end a value is clamped, at its default it is left
// out, a tag Inter lacks is ignored, and the key is in fvar's order however the axes are named.
#[test]
fn a_location_is_resolved_as_the_font_ranges_it() {
    let inter = font("inter/InterVariable.ttf");
    let at = inter.location(&[(WGHT, 1000.0), (Tag::new(b"XXXX"), 3.0), (OPSZ, 14.0)]);
    assert_eq!(at.coords(), [(WGHT, 900.0)]);
    assert_eq!(at.normalized(), [0.0, 1.0]);
    let named = inter.location(&[(OPSZ, 20.0), (WGHT, 700.0)]);
    assert_eq!(named, inter.location(&[(WGHT, 700.0), (OPSZ, 20.0)]));
    assert_eq!(named.coords(), [(OPSZ, 20.0), (WGHT, 700.0)]);
    assert_eq!(inter.location(&[(WGHT, f64::NAN)]), Location::default());
}

// The default is every font's, and a font draws a location naming nothing it varies as its default; such a location
// still keeps what it was asked for, so it isn't the default, which another font may vary.
#[test]
fn the_default_location_is_every_fonts() {
    let inter = font("inter/InterVariable.ttf");
    assert!(Location::default().coords().is_empty() && Location::default().normalized().is_empty());
    assert_eq!(inter.location(&[]), Location::default());
    let at_default = inter.location(&[(WGHT, 400.0)]);
    assert!(at_default.coords().is_empty() && at_default.normalized().is_empty());
    assert_ne!(at_default, Location::default());
    assert!(inter.key(&Location::default()).is_empty() && inter.key(&at_default).is_empty());
    let stix = font("stix-two-math/STIX2Math.otf");
    assert!(stix.location(&[(WGHT, 700.0)]).coords().is_empty());
}

// A location is the font's own key as made; on another font it is resolved again from the values asked for, by that
// font's ranges: Source Serif's opsz of 10 is under Inter's 14, so Inter takes it as its default.
#[test]
fn another_fonts_location_is_resolved_again() {
    let (inter, serif) = (font("inter/InterVariable.ttf"), font("source-serif/SourceSerif4Variable-Roman.otf"));
    let own = inter.location(&[(WGHT, 650.0)]);
    assert!(matches!(inter.key(&own), Key::Made(_)));
    let theirs = serif.location(&[(WGHT, 650.0), (OPSZ, 10.0)]);
    assert_eq!(theirs.coords(), [(WGHT, 650.0), (OPSZ, 10.0)]);
    let key = inter.key(&theirs);
    assert!(matches!(key, Key::Interned(_)));
    assert_eq!(&**key, own.coords());
}

// Inter's wght runs 100 to 900 from 400, Source Han Sans's 250 to 900 from 250: Inter's 400, its default, is 400 to
// Source Han, and its 1000, clamped to 900 by Inter, is clamped by Source Han alone; a tag Inter lacks still counts.
#[test]
fn another_font_reads_the_values_asked_for() {
    let (inter, han) = (font("inter/InterVariable.ttf"), font("source-han-sans/SourceHanSansJP-VF.otf"));
    assert_eq!(&**han.key(&inter.location(&[(WGHT, 400.0)])), [(WGHT, 400.0)]);
    assert_eq!(&**han.key(&inter.location(&[(WGHT, 1000.0)])), [(WGHT, 900.0)]);
    let stix = font("stix-two-math/STIX2Math.otf");
    assert_eq!(&**han.key(&stix.location(&[(WGHT, 700.0)])), [(WGHT, 700.0)]);
    assert!(han.key(&inter.location(&[(WGHT, 250.0)])).is_empty());
}

// Equal locations asked for the same values: in any order, the last of a tag named twice, a non-finite value left
// out, -0.0 as 0.0; a value the making font clamps alike is still another location. Equal ones hash alike.
#[test]
fn locations_are_equal_by_what_was_asked() {
    use core::hash::BuildHasher;
    let inter = font("inter/InterVariable.ttf");
    let asked = inter.location(&[(OPSZ, 20.0), (WGHT, 700.0)]);
    let same = inter.location(&[(WGHT, 300.0), (OPSZ, 20.0), (WGHT, 700.0), (WGHT, f64::NAN)]);
    assert_eq!(asked, same);
    let hasher = std::hash::RandomState::new();
    assert_eq!(hasher.hash_one(&asked), hasher.hash_one(&same));
    assert_eq!(inter.location(&[(OPSZ, -0.0)]), inter.location(&[(OPSZ, 0.0)]));
    assert_ne!(inter.location(&[(WGHT, 1000.0)]), inter.location(&[(WGHT, 900.0)]));
    assert_eq!(inter.location(&[(WGHT, 1000.0)]).coords(), inter.location(&[(WGHT, 900.0)]).coords());
}

// A thread makes a new location in the allocations of the oldest it keeps, where no location holds them still: made
// and dropped one after another, each is its own, and one held across them all keeps its values.
#[test]
fn a_location_keeps_its_values_as_newer_ones_are_made() {
    let (inter, twin) = (font("inter/InterVariable.ttf"), font("inter/InterVariable.ttf"));
    let weights: Vec<f64> = (0..40).map(|n| 101.0 + 19.0 * f64::from(n)).collect();
    // Each held, so none is made in another's allocations.
    let want: Vec<Location> = weights.iter().map(|&w| twin.location(&[(WGHT, w)])).collect();
    let held = inter.location(&[(WGHT, 650.0)]);
    let normalized = held.normalized().to_vec();
    for (&w, want) in weights.iter().zip(&want) {
        let at = inter.location(&[(WGHT, w)]);
        assert_eq!((at.coords(), at.normalized()), (want.coords(), want.normalized()), "at wght {w}");
    }
    assert_eq!((held.coords(), held.normalized()), (&[(WGHT, 650.0)][..], &normalized[..]));
}
