pub mod base;
pub mod bitmap;
pub mod colr_v0;
pub mod colr_v1;
pub(crate) mod color_load;
pub mod glyph_names;
pub mod jstf;
pub mod lig_caret;
pub mod math_table;
pub mod stat;
pub mod trak;
pub mod vorg;

pub mod decoder;
pub mod table_bytes;
pub(crate) use table_bytes::TableBytes;
pub mod tables;
pub use tables::ByTag;
pub(crate) use tables::Tables;
pub mod format;

pub mod instancer;
pub mod outline;
pub mod paint;
pub mod hinting;
pub mod subsetter;
pub(crate) mod varc;
