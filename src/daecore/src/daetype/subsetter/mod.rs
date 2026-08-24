use alloc::string::String;
use alloc::string::ToString;
use alloc::vec::Vec;
pub mod cff;
mod cmap;

mod ttf_dir;
mod glyf;
mod display_font;
mod bitmap;
pub mod math;
mod jstf;
mod device_metrics;
mod aat;
pub mod otl;
pub mod colr;

use alloc::collections::{BTreeSet, BTreeMap};
use super::decoder::{build_ttf, read_i16_be, read_u16_be, read_u32_be, write_i16_be, write_u16_be, write_u32_be};
use cff::parse::{parse_charset_sids, parse_fd_select_bytes};
use cff::build::{cff_index_size, encode_cff_index, encode_cff_index_refs, encode_cff_int};
use ttf_dir::owned_table;
pub(crate) use glyf::{active_gids_into, patch_compound_gids};

pub use cmap::{cmap_glyph_id, cmap_variation_glyph_id, UvsLookup};
pub use cmap::cmap_entries;
pub use ttf_dir::map_advances_all;
pub use ttf_dir::{parse_ttf_dir, slice_table};
pub use cff::parse::{cff_index_spans, parse_cff_index, parse_cff_index_refs};
pub use cff::parse::{parse_top_dict, parse_private_subrs_offset, parse_fd_dict_private};
pub(crate) use cff::parse::{walk_charset, CharsetFlow};
pub use glyf::parse_loca;
pub use display_font::{build_unicode_cmap, build_name_table};
pub(crate) use cff::seac::{seac_offsets, standard_encoding_sid, sid_to_gid};
pub use cff::seac::seac_component_gids;
pub use otl::gdef::{has_mark_glyph_sets, subset_gdef};
pub use otl::gsub::{gsub_closure, subset_gsub};
pub use otl::gpos::subset_gpos;
pub use bitmap::{subset_bitmap_strikes, subset_sbix};
pub use math::{subset_math, math_closure};
pub use aat::prop::subset_prop;
pub use aat::kerx::subset_kerx;
pub use aat::just::subset_just;
pub use aat::morx::{subset_morx, morx_closure};
pub use aat::simple::{subset_lcar, subset_opbd, subset_ankr, subset_bsln, subset_fmtx};
pub use aat::zapf::subset_zapf;
pub use aat::descriptive::{subset_ebsc, subset_xref, strike_sizes};
pub use jstf::subset_jstf;
pub use device_metrics::{subset_hdmx, subset_ltsh};

#[derive(Debug)]
pub struct SubsetResult {
    pub ttf:     Vec<u8>,
    pub gid_map: Vec<u16>,
}

impl SubsetResult {
    pub fn new_gid(&self, old: u16) -> Option<u16> {
        if self.gid_map.is_empty() {
            return Some(old);
        }
        match self.gid_map.get(old as usize).copied() {
            Some(0) if old != 0 => None,
            other => other,
        }
    }
}

mod remap;
mod ttf;

pub use cff::{subset_cff, subset_cff_compacting};
pub use cff::cff_charstrings_for_closure;
pub use ttf::subset_ttf;
pub use remap::{fix_post_table, metric_pair, rebuild_metrics, remap_kern, remap_vorg};

#[derive(Clone, PartialEq, Eq)]
pub struct GlyphSet {
    words: alloc::vec::Vec<u64>,
    count: usize,
}

impl GlyphSet {
    pub fn new() -> GlyphSet {
        GlyphSet { words: alloc::vec![0u64; 1024], count: 0 }
    }

    pub fn contains(&self, gid: &u16) -> bool {
        let g = usize::from(*gid);
        self.words.get(g >> 6).is_some_and(|w| w & (1u64 << (g & 63)) != 0)
    }

    pub fn insert(&mut self, gid: u16) -> bool {
        let g = usize::from(gid);
        let Some(word) = self.words.get_mut(g >> 6) else { return false };
        let bit = 1u64 << (g & 63);
        let fresh = *word & bit == 0;
        *word |= bit;
        self.count += usize::from(fresh);
        fresh
    }

    pub(crate) fn len(&self) -> usize {
        self.count
    }

    pub fn iter(&self) -> impl Iterator<Item = u16> + '_ {
        self.words.iter().enumerate().flat_map(|(i, &w)| {
            (0..64u32).filter(move |b| w & (1u64 << b) != 0).map(move |b| (i * 64 + b as usize) as u16)
        })
    }
}

impl core::fmt::Debug for GlyphSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl Default for GlyphSet {
    fn default() -> GlyphSet {
        GlyphSet::new()
    }
}

impl Extend<u16> for GlyphSet {
    fn extend<T: IntoIterator<Item = u16>>(&mut self, iter: T) {
        for g in iter {
            self.insert(g);
        }
    }
}

impl<'a> Extend<&'a u16> for GlyphSet {
    fn extend<T: IntoIterator<Item = &'a u16>>(&mut self, iter: T) {
        for &g in iter {
            self.insert(g);
        }
    }
}

impl<'a> FromIterator<&'a u16> for GlyphSet {
    fn from_iter<T: IntoIterator<Item = &'a u16>>(iter: T) -> GlyphSet {
        let mut s = GlyphSet::new();
        s.extend(iter);
        s
    }
}

impl FromIterator<u16> for GlyphSet {
    fn from_iter<T: IntoIterator<Item = u16>>(iter: T) -> GlyphSet {
        let mut s = GlyphSet::new();
        s.extend(iter);
        s
    }
}
