use super::*;

#[test]
fn a_glyph_set_iterates_its_glyphs_in_order() {
    let gids = [0u32, 1, 63, 64, 1_000, 65_534, 65_535];
    let set: GlyphSet = gids.iter().rev().copied().collect();
    assert_eq!(set.iter().collect::<Vec<_>>(), gids);
    assert_eq!(GlyphSet::new().iter().next(), None);
}

#[test]
fn a_glyph_set_within_a_range_is_the_held_glyphs_there() {
    let set: GlyphSet = [0u32, 5, 63, 64, 127, 128, 1_000, 65_535].into_iter().collect();
    for (first, last) in [(0, 65_535), (5, 64), (6, 63), (64, 64), (65, 126), (128, 999), (1_000, 65_535), (9, 4)] {
        let want: Vec<u32> = set.iter().filter(|g| (first..=last).contains(g)).collect();
        assert_eq!(set.within(first, last).collect::<Vec<_>>(), want, "{first}..={last}");
    }
}

// Past 65,535 the set grows, up to the 2^24 ids a table can name; sets compare by what they hold, whatever room.
#[test]
fn a_glyph_set_grows_past_16_bits_to_24() {
    let small: GlyphSet = [7u32, 65_535].into_iter().collect();
    let mut set = small.clone();
    assert!(set.insert(65_536) && set.insert(0x80_0000) && set.insert(0xFF_FFFF) && !set.insert(1 << 24));
    assert!(set.contains(&65_536u32) && set.contains(&0xFF_FFFFu32) && !set.contains(&(1u32 << 24)));
    assert_eq!(set.iter().collect::<Vec<_>>(), [7, 65_535, 65_536, 0x80_0000, 0xFF_FFFF]);
    assert_eq!(set.len(), 5);
    assert_eq!(set.within(65_535, 70_000).collect::<Vec<_>>(), [65_535, 65_536]);
    let mut back = small.clone();
    back.bits.words.resize(set.words(), 0);
    assert!(back == small && small == back, "a grown set and an unsized one holding the same differ");
    assert!(small.is_subset(&set) && !set.is_subset(&small));
}

// Fitted to 200 glyphs, a set keeps 4 words and grows again; a word past them that holds a glyph stays.
#[test]
fn a_fitted_glyph_set_keeps_what_it_holds() {
    let mut set: GlyphSet = [3u32, 100].into_iter().collect();
    set.fit(200);
    assert_eq!((set.words(), set.iter().collect::<Vec<_>>()), (4, vec![3, 100]));
    assert!(set.insert(150) && set.insert(65_535) && set.contains(&65_535u32));
    let mut wide: GlyphSet = [3u32, 70_000].into_iter().collect();
    wide.fit(200);
    assert_eq!((wide.len(), wide.iter().collect::<Vec<_>>()), (2, vec![3, 70_000]));
}

// A cleared set holds nothing and keeps its room, so it takes glyphs again without growing and sits inside any set.
#[test]
fn a_cleared_glyph_set_holds_nothing_and_keeps_its_room() {
    let mut set: GlyphSet = [3u32, 100, 70_000].into_iter().collect();
    let words = set.words();
    set.clear();
    assert_eq!((set.len(), set.iter().next(), set.words()), (0, None, words));
    assert!(set.is_subset(&GlyphSet::empty()) && !set.contains(&3u32));
    assert!(set.insert(100) && set.len() == 1 && set.words() == words);
}
