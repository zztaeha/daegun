use alloc::sync::Arc;

use super::lru::ByteLru;
use super::ResolvedAxes;
use crate::daecore::daekit::base::sync::{Lined, Shared};
use crate::daecore::daekit::draw::bitmap::Bitmap;

/// How many words of bits a raster's options take in its key.
pub(crate) const OPTION_WORDS: usize = 20;

/// A glyph, its interned location and every option that changes its pixels, as bits.
pub(crate) type RasterKey = (u32, Shared<Lined<ResolvedAxes>>, [u32; OPTION_WORDS]);

/// Images drawn, kept under `Cache::Raster`'s budget, 0 until set. An `Arc` in every build, so a rasterizer holding
/// one it lends stays `Send`.
pub(crate) type RasterCache = ByteLru<RasterKey, Arc<Bitmap>>;

// What a kept image costs beyond its pixels: the Arc around it and its key, held twice, in the cache's two maps,
// 425 to 466 bytes measured on Inter, 370 to 407 on 32 bits, as their nodes fill by the order keys come in.
const ENTRY_BYTES: usize = if cfg!(target_pointer_width = "64") { 448 } else { 388 };

pub(crate) fn raster_cache() -> RasterCache {
    RasterCache::new(0, |_, image| image.pixels.capacity() + ENTRY_BYTES)
}
