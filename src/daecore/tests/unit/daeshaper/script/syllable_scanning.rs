use alloc::vec::Vec;
use core::cell::Cell;
use super::{for_each, segment, DEAD};
use crate::daecore::daeshaper::run::{Run, Glyph};
use crate::daecore::daeshaper::generated::syllable_tables::*;

// The scan without dead ends: from each start, on until the machine dies, taking the last accept.
fn rescanned<const N: usize, T: Copy + Into<u8>>(
    categories: &[u8],
    transitions: &[[u8; N]],
    accept: impl Fn(u8) -> Option<T>,
) -> Vec<(usize, usize, u8)> {
    let (mut out, mut at) = (Vec::new(), 0);
    while at < categories.len() {
        let (mut state, mut last) = (0u8, None);
        for (i, &c) in categories.iter().enumerate().skip(at) {
            state = transitions[usize::from(state)][usize::from(c)];
            if state == DEAD {
                break;
            }
            if let Some(kind) = accept(state) {
                last = Some((i + 1, kind.into()));
            }
        }
        match last {
            Some((end, kind)) => {
                out.push((at, end, kind));
                at = end;
            }
            None => at += 1,
        }
    }
    out
}

// What setup wrote for these syllables, with the kinds they hold.
fn marked(syllables: &[(usize, usize, u8)], len: usize) -> (Vec<u8>, u16) {
    let mut out = alloc::vec![0; len];
    for (n, &(start, end, kind)) in syllables.iter().enumerate() {
        out[start..end].fill(((n % 15) as u8 + 1) << 4 | kind);
    }
    (out, syllables.iter().fold(0, |kinds, s| kinds | 1 << s.2))
}

fn scanned<const N: usize, T: Copy + Into<u8>>(
    len: usize,
    transitions: &[[u8; N]],
    category_of: impl Fn(&[Glyph], usize) -> Option<u8>,
    accept: impl Fn(u8) -> Option<T>,
) -> (Vec<u8>, u16) {
    let mut info = alloc::vec![Glyph::of_char(0, 0); len];
    let kinds = segment(&mut info, transitions, category_of, accept);
    (info.iter().map(|g| g.syllable).collect(), kinds)
}

fn same_as_rescanned<const N: usize, T: Copy + Into<u8>>(
    transitions: &[[u8; N]],
    accept: impl Fn(u8) -> Option<T> + Copy,
    seed: u64,
) {
    let mut seed = seed;
    let mut next = |m: usize| {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % m
    };
    for round in 0..3000 {
        let categories: Vec<u8> = (0..1 + next(40)).map(|_| next(N) as u8).collect();
        assert_eq!(
            scanned(categories.len(), transitions, |_, i| Some(categories[i]), accept),
            marked(&rescanned(&categories, transitions, accept), categories.len()),
            "round {round}: {categories:?}",
        );

        // Positions without a category: the rest scanned alone, each syllable running to where the next begins.
        let some: Vec<Option<u8>> = categories.iter().map(|&c| (next(4) != 0).then_some(c)).collect();
        let kept: Vec<usize> = (0..some.len()).filter(|&i| some[i].is_some()).collect();
        let packed: Vec<u8> = kept.iter().map(|&i| categories[i]).collect();
        let packed = rescanned(&packed, transitions, accept);
        let spans: Vec<_> = packed.iter().enumerate().map(|(n, &(start, _, kind))| {
            (kept[start], packed.get(n + 1).map_or(some.len(), |next| kept[next.0]), kind)
        }).collect();
        assert_eq!(
            scanned(some.len(), transitions, |_, i| some[i], accept),
            marked(&spans, some.len()),
            "round {round}: {some:?}",
        );
    }
}

// Dead ends only skip a continuation already known to find no syllable, so every machine splits as before.
#[test]
fn dead_ends_change_no_syllable() {
    same_as_rescanned(&INDIC_TRANSITIONS, indic_accept, 1);
    same_as_rescanned(&KHMER_TRANSITIONS, khmer_accept, 2);
    same_as_rescanned(&MYANMAR_TRANSITIONS, myanmar_accept, 3);
    same_as_rescanned(&USE_TRANSITIONS, use_accept, 4);
    same_as_rescanned(&LANA_TRANSITIONS, lana_accept, 5);
}

// No start is ever passed over, so a syllable running to the next position with a category ends where the
// next one begins.
#[test]
fn every_category_alone_is_a_syllable() {
    fn check<const N: usize, T>(name: &str, transitions: &[[u8; N]], accept: impl Fn(u8) -> Option<T>) {
        for (c, &state) in transitions[0].iter().enumerate() {
            assert!(state != DEAD && accept(state).is_some(), "{name}: category {c} alone is no syllable");
        }
    }
    check("indic", &INDIC_TRANSITIONS, indic_accept);
    check("khmer", &KHMER_TRANSITIONS, khmer_accept);
    check("myanmar", &MYANMAR_TRANSITIONS, myanmar_accept);
    check("use", &USE_TRANSITIONS, use_accept);
    check("lana", &LANA_TRANSITIONS, lana_accept);
}

// KA then 40,000 ZWJs, which scanned to the end from every start take hundreds of millions of steps: each (position,
// state) past an accept is passed twice at most, scanned and then recorded as a dead end.
#[test]
fn a_failed_continuation_is_scanned_once() {
    for zwjs in [2_000, 40_000] {
        let categories: Vec<u8> = core::iter::once(1).chain(core::iter::repeat_n(6, zwjs)).collect();
        let steps = Cell::new(0usize);
        let category_of = |_: &[Glyph], i: usize| {
            steps.set(steps.get() + 1);
            Some(categories[i])
        };
        let found = scanned(categories.len(), &INDIC_TRANSITIONS, category_of, indic_accept);
        assert!(steps.get() < 8 * categories.len(), "{} steps for {} glyphs", steps.get(), categories.len());
        if zwjs == 2_000 {
            assert_eq!(found, marked(&rescanned(&categories, &INDIC_TRANSITIONS, indic_accept), categories.len()));
        }
    }
}

// A walk takes a syllable 64 glyphs at a time.
#[test]
fn a_walk_takes_at_most_64_glyphs() {
    let mut buffer = Run::default();
    (0..130).for_each(|i| buffer.push(0x915, i));
    buffer.glyphs[..130].iter_mut().for_each(|g| g.syllable = 0x11);
    let mut parts = Vec::new();
    for_each(&mut buffer, |_, start, end| parts.push((start, end)));
    assert_eq!(parts, [(0, 64), (64, 128), (128, 130)]);
}

// Serials cycle 1 to 15 over every syllable, so the 255th and 256th differ as all neighbors do.
#[test]
fn neighbors_never_share_a_serial() {
    let one_glyph = [[1, DEAD], [DEAD, DEAD]];
    let (marks, _) = scanned(300, &one_glyph, |_, _| Some(0), |s| (s == 1).then_some(1u8));
    assert!(marks.windows(2).all(|w| w[0] != w[1]));
}
