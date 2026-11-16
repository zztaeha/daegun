use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
pub mod cff;
mod cmap;

mod ttf_dir;
mod glyf;
mod display_font;
mod os2_ranges;
mod bitmap;
pub mod math;
mod jstf;
mod device_metrics;
mod aat;
pub mod otl;
pub mod colr;

use alloc::collections::{BTreeSet, BTreeMap};
use crate::daecore::GlyphId;
use super::decoder::{
    build_ttf, read_i16_be, read_u16_be, read_u32_be, ttf_table_ranges, write_i16_be, write_u16_be, write_u32_be,
};
use cff::parse::parse_charset_sids;
use cff::build::encode_cff_index_refs;
use super::format::cff::{encode_cff_int, encode_cff_int_short};
use super::format::glyf::{is_composite, loca_table, trimmed};
pub(crate) use glyf::{active_gids_into, patch_compound_gids};
#[cfg(feature = "capi")]
pub(crate) use glyf::{loca_into, loca_len};

pub use cmap::{CharMap, CharTables, Sequence, UvsLookup};
#[cfg(test)]
pub(crate) use cmap::CmapLookup;
pub(crate) use cmap::cmap_is_symbol;
pub use ttf_dir::slice_table;
pub use cff::parse::parse_cff_index_refs;
pub(crate) use cff::parse::CffIndex;
pub(crate) use cff::parse::{cff_index_count, cff_index_end, cff_index_entry, cff_readable};
pub(crate) use cff::parse::walk_charset;
pub use glyf::parse_loca;
pub use display_font::TextCmap;
pub(crate) use display_font::{display_tables, is_variation_selector, remap_text, CmapEntries};
pub(crate) use cff::seac::{close_over_seacs, standard_encoding_sid, SidMap};
pub use otl::gdef::mark_glyph_set_count;
pub(crate) use otl::gpos::instance_gpos;
pub(crate) use aat::morx::Stackings;
// The tables' own subsetters and morx's closure, which the allocation suite calls one at a time; it builds in debug,
// or in release under `--cfg daegun_counting`.
#[cfg(all(test, any(debug_assertions, daegun_counting)))]
pub(crate) use aat::morx::morx_closure;
#[cfg(all(test, any(debug_assertions, daegun_counting)))]
pub(crate) use {
    aat::descriptive::subset_xref, aat::kerx::subset_kerx, aat::morx::subset_morx, aat::zapf::subset_zapf,
    jstf::subset_jstf, bitmap::{subset_bitmap_strikes, subset_sbix}, math::{math_closure, subset_math},
};

/// A subset font, and how its glyphs were renumbered.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[doc(alias("SubsetResult"))]
pub struct Subset {
    /// The font file.
    pub font: Vec<u8>,
    // Each old glyph's new id by index, 0 for one dropped but .notdef, a kept glyph its own where the subset keeps
    // ids in place. The CFF writers leave it empty for that, which the cache fills before handing the subset out.
    pub(crate) gid_map: Vec<u32>,
}

impl Subset {
    /// The id glyph `old` of the source has in the subset; None where the subset dropped it.
    #[doc(alias("new_gid"))]
    pub fn renumbered(&self, old: GlyphId) -> Option<GlyphId> {
        match self.gid_map.get(old.0 as usize).copied() {
            Some(0) if old.0 != 0 => None,
            other => other.map(GlyphId),
        }
    }
}

mod remap;
mod ttf;
mod closure;
mod gvar;

pub(crate) use cff::{subset_cff2_closed, subset_cff_closed};
#[cfg(test)]
pub(crate) use {cff::subset_cff_compacting, ttf::subset_ttf};
#[cfg(all(test, any(debug_assertions, daegun_counting)))]
pub(crate) use cff::subset_cff;
pub(crate) use ttf::{kept_extents, side_tables, subset_glyf_tables, KeptGlyphs};
pub(crate) use closure::{closure_loop, glyf_closure, Closed, FaceReads};
pub use remap::{fix_post_table, rebuild_metrics, remap_kern, remap_vorg};

// Tables naming no glyph, which every subset carries as they are.
pub(crate) const GLYPH_FREE_TABLES: &[&str] = &[
    "cvt ", "fpgm", "prep", "gasp", "VDMX", "meta", "PCLT", "hsty", "feat", "trak", "fdsc", "ltag", "STAT",
];

// A subset holds no more than its source, give or take what a rebuild adds. Past this it is a font
// naming its data over and over, and refused.
pub(crate) const SUBSET_GROWTH: usize = 2;
pub(crate) const SUBSET_SLACK: usize = 1 << 16;

pub(crate) fn subset_budget(source: usize) -> usize {
    source.saturating_mul(SUBSET_GROWTH).saturating_add(SUBSET_SLACK)
}

// A set of the indices below its size rounded up to a multiple of 64; one past that is never held.
#[derive(Clone)]
pub(crate) struct BitSet {
    words: Vec<u64>,
}

impl BitSet {
    pub(crate) fn new(size: usize) -> BitSet {
        BitSet { words: alloc::vec![0; size.div_ceil(64)] }
    }

    pub(crate) fn contains(&self, i: usize) -> bool {
        self.words.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }

    // Whether `i` is newly held: false for one held already or past the size.
    pub(crate) fn insert(&mut self, i: usize) -> bool {
        let Some(word) = self.words.get_mut(i / 64) else { return false };
        let bit = 1 << (i % 64);
        let fresh = *word & bit == 0;
        *word |= bit;
        fresh
    }

    // Grown to hold `i`, doubling, at most to `cap` bits.
    fn grow_for(&mut self, i: usize, cap: usize) {
        if i / 64 >= self.words.len() && i < cap {
            let len = (i / 64 + 1).max(self.words.len() * 2).min(cap.div_ceil(64));
            self.words.resize(len, 0);
        }
    }

    // Only the set bits are visited, so a set of a few indices is not a test of each.
    fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(i, &w)| set_bits(i, w))
    }

    // The set bits from `first` to `last`, a word at a time.
    fn within(&self, first: usize, last: usize) -> impl Iterator<Item = usize> + '_ {
        let end = (last / 64).min(self.words.len().saturating_sub(1));
        let words = self.words.get(first / 64..=end).unwrap_or_default();
        words.iter().zip(first / 64..).flat_map(move |(&w, i)| {
            let low = if i == first / 64 { !0 << (first % 64) } else { !0 };
            let high = if i == last / 64 { !0 >> (63 - last % 64) } else { !0 };
            set_bits(i, w & low & high)
        })
    }
}

// The set bits of the `i`th word, as indices.
fn set_bits(i: usize, word: u64) -> impl Iterator<Item = usize> {
    let mut rest = word;
    core::iter::from_fn(move || {
        let b = (rest != 0).then(|| rest.trailing_zeros())?;
        rest &= rest - 1;
        Some(i * 64 + b as usize)
    })
}

// Equal by the bits held, whatever room either has grown.
impl PartialEq for BitSet {
    fn eq(&self, other: &BitSet) -> bool {
        let (short, long) = if self.words.len() <= other.words.len() { (self, other) } else { (other, self) };
        long.words.iter().enumerate().all(|(i, &w)| w == short.words.get(i).copied().unwrap_or(0))
    }
}

#[derive(Clone, PartialEq)]
pub struct GlyphSet {
    bits: BitSet,
    count: usize,
}

impl GlyphSet {
    pub fn new() -> GlyphSet {
        GlyphSet { bits: BitSet::new(1 << 16), count: 0 }
    }

    // Any id that widens to 32 bits, so a 16-bit table's glyphs ask as they're read.
    pub fn contains<G: Copy + Into<u32>>(&self, gid: &G) -> bool {
        self.bits.contains((*gid).into() as usize)
    }

    // Room for 65,536 glyphs from the start, grown for more up to the 24 bits a table can name.
    pub fn insert(&mut self, gid: u32) -> bool {
        self.bits.grow_for(gid as usize, 1 << 24);
        let fresh = self.bits.insert(gid as usize);
        self.count += usize::from(fresh);
        fresh
    }

    pub(crate) fn len(&self) -> usize {
        self.count
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.bits.iter().map(|g| g as u32)
    }

    // The glyphs held from `first` to `last`, in order.
    pub(crate) fn within(&self, first: u32, last: u32) -> impl Iterator<Item = u32> + '_ {
        self.bits.within(first as usize, last as usize).map(|g| g as u32)
    }

    // The words `within` reads from `first` to `last`.
    pub(crate) fn words_within(&self, first: u32, last: u32) -> usize {
        let end = (last as usize / 64).min(self.words().saturating_sub(1));
        (end + 1).saturating_sub(first as usize / 64).min(self.words())
    }

    // A set with no room yet, for the many small ones a closure builds.
    pub(crate) fn empty() -> GlyphSet {
        GlyphSet { bits: BitSet::new(0), count: 0 }
    }

    // Empty, keeping its room for its next use.
    pub(crate) fn clear(&mut self) {
        if self.count > 0 {
            self.bits.words.fill(0);
            self.count = 0;
        }
    }

    pub(crate) fn is_subset(&self, other: &GlyphSet) -> bool {
        let other = |i: usize| other.bits.words.get(i).copied().unwrap_or(0);
        self.bits.words.iter().enumerate().all(|(i, &w)| w & !other(i) == 0)
    }

    // Room for `n` glyphs, the empty words past them dropped: a pass over a smaller font's set reads only its own.
    pub(crate) fn fit(&mut self, n: u32) {
        let words = &mut self.bits.words;
        while words.len() > (n as usize).div_ceil(64) && words.last() == Some(&0) {
            words.pop();
        }
    }

    // The words a pass over the set reads.
    pub(crate) fn words(&self) -> usize {
        self.bits.words.len()
    }

    pub(crate) fn union(&mut self, other: &GlyphSet) {
        let words = &mut self.bits.words;
        if words.len() < other.bits.words.len() {
            words.resize(other.bits.words.len(), 0);
        }
        for (w, &o) in words.iter_mut().zip(&other.bits.words) {
            self.count += (o & !*w).count_ones() as usize;
            *w |= o;
        }
    }
}

// A source's cmap and DMAP as the character reader takes them.
pub(crate) fn char_tables<'a>(table: impl Fn(&str) -> Option<&'a [u8]>) -> CharTables<'a> {
    CharTables { cmap: table("cmap").unwrap_or_default(), dmap: table("DMAP").unwrap_or_default() }
}

// A glyph's id in the subset, None where the subset leaves it out.
pub(crate) fn remap_gid(active: &GlyphSet, gid_map: &[u32], orig: u32) -> Option<u32> {
    if !active.contains(&orig) { return None; }
    gid_map.get(orig as usize).copied()
}

// A glyph a 16-bit table names, as the subset names it there: none where its new id needs more than 16 bits.
pub(crate) fn remap_gid16(active: &GlyphSet, gid_map: &[u32], orig: u16) -> Option<u16> {
    remap_gid(active, gid_map, orig.into()).and_then(|g| u16::try_from(g).ok())
}

impl Default for GlyphSet {
    fn default() -> GlyphSet {
        GlyphSet::new()
    }
}

impl Extend<u32> for GlyphSet {
    fn extend<T: IntoIterator<Item = u32>>(&mut self, iter: T) {
        for g in iter {
            self.insert(g);
        }
    }
}

impl FromIterator<u32> for GlyphSet {
    fn from_iter<T: IntoIterator<Item = u32>>(iter: T) -> GlyphSet {
        let mut s = GlyphSet::new();
        s.extend(iter);
        s
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/subsetter/mod.rs"]
mod tests;
