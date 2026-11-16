use super::Digest;
use alloc::vec::Vec;

const NARROW: [u32; 3] = [4, 0, 6];

// Three 64-bit masks from bits 4, 0 and 6 up, the digest would-substitute's answers were written against.
#[derive(Default)]
struct Narrow([u64; 3]);

impl Narrow {
    fn add(&mut self, glyph: u32) {
        self.0.iter_mut().zip(NARROW).for_each(|(mask, shift)| *mask |= 1 << ((glyph >> shift) & 63));
    }

    fn add_range(&mut self, first: u32, last: u32) {
        let span = |lo: u32, hi: u32| (u64::MAX >> (63 - hi)) & (u64::MAX << lo);
        for (mask, shift) in self.0.iter_mut().zip(NARROW) {
            let (a, b) = (first >> shift, last >> shift);
            let (lo, hi) = (a & 63, b & 63);
            *mask |= match b.wrapping_sub(a) {
                63.. => u64::MAX,
                _ if lo <= hi => span(lo, hi),
                _ => span(lo, 63) | span(0, hi),
            };
        }
    }

    fn may_have(&self, glyph: u32) -> bool {
        self.0.iter().zip(NARROW).all(|(mask, shift)| mask & (1 << ((glyph >> shift) & 63)) != 0)
    }
}

// Glyphs and ranges of every size, reversed ones included, from a fixed generator, two digests of them joined.
fn pair(next: &mut impl FnMut() -> u64, case: u64) -> (Digest, Narrow, Vec<u32>, Vec<(u32, u32)>) {
    let (mut wide, mut narrow) = (Digest::default(), Narrow::default());
    let (mut glyphs, mut ranges) = (Vec::new(), Vec::new());
    for _ in 0..case % 6 {
        let glyph = u32::from((next() as u16) >> (next() % 16));
        wide.add(glyph);
        narrow.add(glyph);
        glyphs.push(glyph);
    }
    for _ in 0..case % 4 {
        let mut end = || u32::from((next() as u16) >> (next() % 16));
        let (first, last) = (end(), end());
        wide.add_range(first, last);
        narrow.add_range(first, last);
        ranges.push((first, last));
    }
    (wide, narrow, glyphs, ranges)
}

#[test]
fn folded_masks_answer_as_the_narrow_digest_and_the_wide_ones_miss_nothing() {
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for case in 0..96 {
        let (mut wide, mut narrow, mut glyphs, mut ranges) = pair(&mut next, case);
        let (other, other_narrow, more, more_ranges) = pair(&mut next, case / 3);
        wide.union(&other);
        other_narrow.0.iter().zip(&mut narrow.0).for_each(|(o, n)| *n |= o);
        glyphs.extend(more);
        ranges.extend(more_ranges);
        for glyph in 0..=u32::from(u16::MAX) {
            assert_eq!(wide.may_have_folded(glyph), narrow.may_have(glyph), "case {case}, glyph {glyph}");
        }
        let covered = ranges.iter().filter(|(first, last)| first <= last).flat_map(|&(first, last)| first..=last);
        for glyph in glyphs.iter().copied().chain(covered) {
            assert!(wide.may_have(glyph), "case {case}: glyph {glyph} was added");
            let mut alone = Digest::default();
            alone.add(glyph);
            assert!(wide.may_intersect(&alone), "case {case}: glyph {glyph} intersects");
        }
    }
    assert!(Digest::full().may_have_folded(7) && !Digest::default().may_have_folded(7));
}
