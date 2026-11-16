use super::*;
use alloc::string::String;
use alloc::vec::Vec;
use crate::daecore::daetype::TableBytes;

// A size in pixels per em at an em of 1,000 units.
fn at_px(px: i64) -> Scale {
    Scale::new(px * 64 * 65536 / 1000).expect("a scale")
}

fn private(bytes: &[u8]) -> Private {
    Private::read(bytes).expect("a Private DICT")
}

fn stem(bottom: i64, top: i64) -> Stem {
    Stem { bottom: bottom << 16, top: top << 16, edges: Edges::Both, bit: 0, place: None }
}

// BlueValues -15 0 500 515.
const ZONES: &[u8] = &[124, 154, 248, 136, 154, 6];

// BlueScale's default 0.039625 suppresses overshoot below 39.625 px at 1,000 units, exactly: at a scale equal to
// BlueScale it's off. A zone 30 units tall would reach a pixel at 39 px, so there it's off as well.
#[test]
fn overshoot_is_suppressed_below_blue_scales_size() {
    let p = private(ZONES);
    assert!(Zones::new(&p, at_px(39), None).suppress);
    assert!(!Zones::new(&p, at_px(40), None).suppress);
    let equal = Private { blue_scale: at_px(39).slope(), ..p };
    assert!(!Zones::new(&equal, at_px(39), None).suppress);
    // BlueValues -30 0.
    let tall = private(&[109, 169, 6]);
    assert!(!Zones::new(&tall, at_px(39), None).suppress && Zones::new(&tall, at_px(30), None).suppress);
}

// At 50 px, past BlueScale's size: an overshoot of BlueShift (7) units or more is at least a pixel past the flat
// pixel, a smaller one only where it reaches half a pixel; an edge inside the zone on the flat side lands on it.
#[test]
fn an_overshoot_follows_blue_shift_and_half_a_pixel() {
    let zones = Zones::new(&private(ZONES), at_px(50), None);
    let bottom = |edge: i64| zones.align(&Stem { edges: Edges::Bottom, ..stem(edge, edge) }).map(|(b, _)| b);
    let top = |edge: i64| zones.align(&Stem { edges: Edges::Top, ..stem(edge, edge) }).map(|(_, t)| t);
    assert_eq!(bottom(-12), Some(-ONE), "12 units, 0.6 px");
    assert_eq!(bottom(-8), Some(-ONE), "8 units, 0.4 px, past BlueShift");
    assert_eq!(bottom(-5), Some(0), "5 units, 0.25 px");
    assert_eq!(bottom(1), Some(0), "inside BlueFuzz on the flat side");
    assert_eq!(top(512), Some(26 * ONE));
    assert_eq!(bottom(100), None);
    // A stem aligned by its bottom keeps its scaled width.
    let (b, t) = zones.align(&stem(-12, 60)).expect("aligned");
    assert_eq!((b, t - b), (-ONE, at_px(50).of(72 << 16)));
}

// Below BlueScale's size an aligned edge lands on the flat pixel whatever its overshoot.
#[test]
fn a_suppressed_overshoot_lands_on_the_flat_pixel() {
    let zones = Zones::new(&private(ZONES), at_px(16), None);
    assert_eq!(zones.align(&stem(-15, 60)).map(|(b, _)| b), Some(0));
    assert_eq!(zones.align(&stem(400, 515)).map(|(_, t)| t), Some(8 * ONE));
}

// A family flat edge under a pixel from the font's replaces it: FamilyBlues' -2 for BlueValues' 0 at 16 px, where a
// pixel is 62.5 units; one 70 units off doesn't.
#[test]
fn a_flat_edge_takes_the_familys_within_a_pixel() {
    // BlueValues -15 0, FamilyBlues -17 -2.
    let zones = Zones::new(&private(&[124, 154, 6, 122, 154, 8]), at_px(16), None);
    assert_eq!(zones.zones[0].flat, -2 << 16);
    // FamilyBlues -85 -70.
    let zones = Zones::new(&private(&[124, 154, 6, 28, 0xff, 0xab, 154, 8]), at_px(16), None);
    assert_eq!(zones.zones[0].flat, 0);
}

// A stem whose bottom is in a bottom zone and top in a top zone, which Type 1 6.5 forbids, aligns by its bottom.
#[test]
fn a_stem_in_two_zones_aligns_by_its_bottom() {
    // BlueValues -15 0 100 115.
    let zones = Zones::new(&private(&[124, 154, 239, 154, 6]), at_px(16), None);
    let (b, _) = zones.align(&stem(-5, 105)).expect("aligned");
    assert_eq!(b, 0);
}

// Where overshoot is suppressed a flat edge rounds a third of a pixel outward: at 9 px an x-height of 475 units
// (4.275 px) rises to 5 px and a bottom flat edge at -150 (-1.35 px) drops to -2, while 455 (4.095 px) stays at 4.
// Past BlueScale's size, 506 units at 50 px (25.3 px) rounds to 25.
#[test]
fn a_flat_edge_rounds_a_third_of_a_pixel_outward_where_overshoot_is_suppressed() {
    let top = |zones: &Zones, edge: i64| zones.align(&Stem { edges: Edges::Top, ..stem(edge, edge) }).map(|(_, t)| t);
    // BlueValues -15 0 475 490, OtherBlues -165 -150.
    let zones = Zones::new(&private(&[124, 154, 248, 111, 154, 6, 251, 57, 154, 7]), at_px(9), None);
    assert!(zones.suppress);
    assert_eq!(top(&zones, 475), Some(5 * ONE));
    assert_eq!(zones.align(&Stem { edges: Edges::Bottom, ..stem(-150, -150) }).map(|(b, _)| b), Some(-2 * ONE));
    assert_eq!(zones.align(&Stem { edges: Edges::Bottom, ..stem(0, 0) }).map(|(b, _)| b), Some(0));
    // BlueValues -15 0 455 470.
    assert_eq!(top(&Zones::new(&private(&[124, 154, 248, 91, 154, 6]), at_px(9), None), 455), Some(4 * ONE));
    // BlueValues -15 0 506 521.
    let zones = Zones::new(&private(&[124, 154, 248, 142, 154, 6]), at_px(50), None);
    assert!(!zones.suppress);
    assert_eq!(top(&zones, 506), Some(25 * ONE));
}

// Only an ideographic FD (LanguageGroup 1) is hinted in the em box, each edge on its nearest pixel: -120 and 880
// units at 12 px are -1.44 and 10.56 px, so -1 and 11.
#[test]
fn an_ideographic_fd_holds_the_em_box_edges_on_their_nearest_pixels() {
    let em_box = Some(EmBox { bottom: -120 << 16, top: 880 << 16 });
    let ideographic = Zones::new(&private(&[140, 12, 17]), at_px(12), em_box);
    assert_eq!(ideographic.frame(), Some([(-120 << 16, -ONE), (880 << 16, 11 * ONE)]));
    assert_eq!(Zones::new(&private(&[]), at_px(12), em_box).frame(), None);
    assert_eq!(Zones::new(&private(&[140, 12, 17]), at_px(12), None).frame(), None);
}

// A horizontal BASE of baseline tags `tags`, each script's values for the first of those tags, in order.
fn base(tags: &[&[u8; 4]], scripts: &[(&[u8; 4], &[i16])]) -> Tables {
    let words = |w: &[u16]| w.iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    let tag_list = [words(&[tags.len() as u16]), tags.iter().flat_map(|t| **t).collect()].concat();
    let mut t = words(&[1, 0, 8, 0, 4, 4 + tag_list.len() as u16]);
    t.extend(tag_list);
    t.extend(words(&[scripts.len() as u16]));
    let mut at = 2 + 6 * scripts.len();
    for (tag, values) in scripts {
        t.extend(*tag);
        t.extend(words(&[at as u16]));
        at += 10 + 6 * values.len();
    }
    for (_, values) in scripts {
        let n = values.len() as u16;
        t.extend(words(&[6, 0, 0, 0, n]));
        t.extend(words(&(0..n).map(|i| 4 + 2 * n + 4 * i).collect::<Vec<_>>()));
        values.iter().for_each(|&v| t.extend(words(&[1, v as u16])));
    }
    [(String::from("BASE"), TableBytes::from(t))].into_iter().collect()
}

// The em box is 'ideo' to 'idtp', or 'ideo' and an em above it; the 'hani' script's, else the default script's
// where 'hani' has none or names no 'ideo'. Without 'ideo' there's no em box.
#[test]
fn the_em_box_reads_from_base_ideo_and_idtp() {
    let em_box = |bottom: i64, top: i64| Some(EmBox { bottom: bottom << 16, top: top << 16 });
    let both: &[&[u8; 4]] = &[b"romn", b"ideo", b"idtp"];
    let tables = base(both, &[(b"DFLT", &[0, -100, 900]), (b"hani", &[0, -120, 880])]);
    assert_eq!(EmBox::read(&tables, 1000), em_box(-120, 880));
    let tables = base(both, &[(b"DFLT", &[0, -100, 900]), (b"hani", &[0])]);
    assert_eq!(EmBox::read(&tables, 1000), em_box(-100, 900));
    assert_eq!(EmBox::read(&base(&[b"ideo"], &[(b"DFLT", &[-240])]), 2048), em_box(-240, 1808));
    assert_eq!(EmBox::read(&base(&[b"romn"], &[(b"hani", &[0])]), 1000), None);
    assert_eq!(EmBox::read(&base(both, &[(b"hani", &[0, 900, 900])]), 1000), None, "an empty box read");
}

// Without a placement glyph space is the space outlines are drawn in, so the em box is as read.
#[test]
fn the_em_box_without_a_placement_is_as_read() {
    let em_box = EmBox { bottom: -120 << 16, top: 880 << 16 };
    assert_eq!(em_box.placed(None), Some(em_box));
}
