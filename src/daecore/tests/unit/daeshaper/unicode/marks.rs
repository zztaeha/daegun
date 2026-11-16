use super::*;

// The sort classes the mark reorderings and fallback positioning name: each combining class sorts as itself but the
// moved ones, and three characters sort apart from their class.
#[test]
fn mark_classes_keep_the_values_the_reorderings_name() {
    let moved: [(u8, u8); 23] = [
        (10, 22), (11, 15), (12, 16), (13, 17), (14, 23), (15, 18), (16, 19), (17, 20), (18, 21), (19, 14),
        (20, 24), (21, 12), (22, 25), (23, 13), (24, 10), (25, 11), (27, 28), (28, 29), (29, 30), (30, 31),
        (31, 32), (32, 33), (33, 27),
    ];
    let rest = [(84, 4), (91, 5), (103, 3), (130, 132), (132, 131)];
    for class in 0..=255u8 {
        let want = moved.iter().chain(&rest).find(|m| m.0 == class).map_or(class, |m| m.1);
        assert_eq!(SORT_CLASS[usize::from(class)], want, "class {class}");
    }
    assert_eq!(mark_class('\u{05B0}'), 22, "sheva");
    assert_eq!(mark_class('\u{0651}'), 27, "shadda");
    assert_eq!(mark_class('\u{0301}'), 230);
    assert_eq!(mark_class('a'), 0);
    assert_eq!([mark_class('\u{0F39}'), mark_class('\u{0FC6}'), mark_class('\u{1A60}')], [127, 254, 254]);
}
