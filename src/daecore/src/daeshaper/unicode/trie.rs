// The index the generators build over every code point (`scripts/tools/gen/trie.py`): the code point's top bits pick
// a mid block, its middle bits a leaf block within it, its low bits a record within that. Equal blocks are stored once.
pub(crate) struct Trie<R: 'static, T: 'static, M: 'static, L: 'static> {
    pub(crate) leaf_bits: u32,
    pub(crate) mid_bits: u32,
    pub(crate) records: &'static [R],
    pub(crate) top: &'static [T],
    pub(crate) mid: &'static [M],
    pub(crate) leaf: &'static [L],
}

// A stage is u8 or u16 as its largest entry needs. Each pairing the tables use gets its own `const fn` reader, so a
// table can be read while the crate builds.
macro_rules! reader {
    ($($mid:ty, $leaf:ty);*) => {$(
        impl<R> Trie<R, u8, $mid, $leaf> {
            // A few loads: as a call, every non-ASCII code paid for one, 8% of a CJK or Hangul shape.
            #[inline(always)]
            pub(crate) const fn get(&self, cp: u32) -> &'static R {
                let cp = cp as usize;
                let (mid_low, leaf_low) = ((1 << self.mid_bits) - 1, (1 << self.leaf_bits) - 1);
                let block = self.top[cp >> (self.leaf_bits + self.mid_bits)] as usize;
                let row = (block << self.mid_bits) + ((cp >> self.leaf_bits) & mid_low);
                let slot = ((self.mid[row] as usize) << self.leaf_bits) + (cp & leaf_low);
                &self.records[self.leaf[slot] as usize]
            }
        }
    )*};
}
reader!(u8, u8; u16, u16);
