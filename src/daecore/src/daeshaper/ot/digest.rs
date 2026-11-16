// A glyph's bits from 4, 0 and 6 up, each in a mask of WORDS words: a set that can only say a glyph is surely absent.
// Folded to 64 bits a mask is the narrower digest, whose false positives would-substitute's answers depend on.
const SHIFTS: [u32; 3] = [4, 0, 6];
const WORDS: usize = 4;
const TOP: u32 = 64 * WORDS as u32 - 1;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct Digest {
    masks: [[u64; WORDS]; 3],
}

impl Digest {
    pub(crate) fn of(glyphs: &[crate::daecore::daeshaper::run::Glyph]) -> Digest {
        let mut digest = Digest::default();
        glyphs.iter().for_each(|info| digest.add(info.id));
        digest
    }

    // Every glyph may be in it, for a set that isn't read.
    pub(crate) fn full() -> Digest {
        Digest { masks: [[u64::MAX; WORDS]; 3] }
    }

    pub(crate) fn add(&mut self, glyph: u32) {
        for (mask, shift) in self.masks.iter_mut().zip(SHIFTS) {
            let bit = (glyph >> shift) & TOP;
            mask[bit as usize / 64] |= 1 << (bit % 64);
        }
    }

    // A reversed range wraps around, filling every mask its ends differ in.
    pub(crate) fn add_range(&mut self, first: u32, last: u32) {
        for (mask, shift) in self.masks.iter_mut().zip(SHIFTS) {
            let (a, b) = (first >> shift, last >> shift);
            let (lo, hi) = (a & TOP, b & TOP);
            match b.wrapping_sub(a) {
                TOP.. => *mask = [u64::MAX; WORDS],
                _ if lo <= hi => fill(mask, lo, hi),
                _ => {
                    fill(mask, lo, TOP);
                    fill(mask, 0, hi);
                }
            }
        }
    }

    pub(crate) fn union(&mut self, other: &Digest) {
        for (mask, other) in self.masks.iter_mut().zip(other.masks) {
            mask.iter_mut().zip(other).for_each(|(word, other)| *word |= other);
        }
    }

    pub(crate) fn may_intersect(&self, other: &Digest) -> bool {
        self.masks.iter().zip(&other.masks).all(|(mask, other)| mask.iter().zip(other).any(|(a, b)| a & b != 0))
    }

    pub(crate) fn may_have(&self, glyph: u32) -> bool {
        self.masks.iter().zip(SHIFTS).all(|(mask, shift)| {
            let bit = (glyph >> shift) & TOP;
            mask[bit as usize / 64] & (1 << (bit % 64)) != 0
        })
    }

    // `may_have` of the masks folded to 64 bits.
    pub(crate) fn may_have_folded(&self, glyph: u32) -> bool {
        self.masks.iter().zip(SHIFTS).all(|(mask, shift)| {
            let bit = (glyph >> shift) & 63;
            mask.iter().any(|word| word & (1 << bit) != 0)
        })
    }
}

// Bits `lo` through `hi` of a mask.
fn fill(mask: &mut [u64; WORDS], lo: u32, hi: u32) {
    for (w, word) in (0u32..).step_by(64).zip(mask.iter_mut()) {
        if hi >= w && lo < w + 64 {
            let (from, to) = (lo.max(w) - w, hi.min(w + 63) - w);
            *word |= (u64::MAX >> (63 - to)) & (u64::MAX << from);
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/ot/digest.rs"]
mod tests;
