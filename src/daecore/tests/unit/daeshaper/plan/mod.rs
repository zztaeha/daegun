use super::*;
use crate::daecore::daeshaper::face::LayoutVariation;

fn tag(name: &[u8; 4]) -> Tag {
    Tag::new(name)
}

// With no tables, only features kept for a fallback get bits, so each request below is planned as asked.
fn compiled(ask: impl FnOnce(&mut Planner)) -> Features {
    let mut planner = Planner::new([None; 2], &[], &[]);
    ask(&mut planner);
    planner.compile(&LayoutVariation::default()).0
}

// The three flags take bits 0 to 2, so the first feature starts at bit 3; each takes as many bits as its largest
// value needs, and every global one with value 1 shares bit 31.
#[test]
fn feature_bits_start_above_the_flags_and_on_off_globals_share_one() {
    let features = compiled(|p| {
        p.add(tag(b"aaaa"), FeatureUse::FALLBACK);
        p.add_value(tag(b"bbbb"), FeatureUse::FALLBACK, 3);
        p.on(tag(b"cccc"), FeatureUse::FALLBACK);
        p.on(tag(b"dddd"), FeatureUse::FALLBACK);
    });
    assert_eq!((features.mask(tag(b"aaaa")), features.shift(tag(b"aaaa"))), (1 << 3, 3));
    assert_eq!((features.mask(tag(b"bbbb")), features.shift(tag(b"bbbb"))), (0b11 << 4, 4));
    assert_eq!(features.mask(tag(b"cccc")), 1 << 31);
    assert_eq!(features.mask(tag(b"dddd")), 1 << 31);
    assert_eq!(features.global(), 1 << 31);
    assert!(features.lacks_table(tag(b"aaaa")));
}

// A feature no table has and that keeps no fallback gets no bits.
#[test]
fn a_feature_without_a_table_or_fallback_takes_no_bits() {
    let features = compiled(|p| p.add(tag(b"aaaa"), FeatureUse::PLAIN));
    assert_eq!(features.mask(tag(b"aaaa")), 0);
}

// A later global request sets the value; a later ranged one keeps the larger value and makes the feature ranged.
#[test]
fn requests_for_one_tag_merge_in_order() {
    let features = compiled(|p| {
        p.add_value(tag(b"aaaa"), FeatureUse::FALLBACK, 3);
        p.on(tag(b"aaaa"), FeatureUse::PLAIN);
        p.add_value(tag(b"bbbb"), FeatureUse::GLOBAL.and(FeatureUse::FALLBACK), 2);
        p.add_value(tag(b"bbbb"), FeatureUse::PLAIN, 5);
        p.off(tag(b"cccc"));
        p.add(tag(b"cccc"), FeatureUse::FALLBACK);
    });
    assert_eq!(features.mask(tag(b"aaaa")), 1 << 31);
    let (mask, shift) = (features.mask(tag(b"bbbb")), features.shift(tag(b"bbbb")));
    assert_eq!(mask, 0b111 << shift);
    // The first request's value stays the glyphs' starting one.
    assert_eq!(features.global() & mask, 2 << shift);
    assert_eq!(features.mask(tag(b"cccc")).count_ones(), 1);
}

// A feature asked for again after a pause keeps the stage of its first request.
#[test]
fn a_feature_takes_the_earliest_stage_it_was_asked_in() {
    let features = compiled(|p| {
        p.add(tag(b"aaaa"), FeatureUse::FALLBACK);
        p.end_stage(None);
        p.add(tag(b"bbbb"), FeatureUse::FALLBACK);
        p.add(tag(b"aaaa"), FeatureUse::FALLBACK);
    });
    assert_eq!(features.stage(Table::Gsub, tag(b"aaaa")), Some(0));
    assert_eq!(features.stage(Table::Gsub, tag(b"bbbb")), Some(1));
}
