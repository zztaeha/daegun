mod io;
pub(crate) mod ttf;

mod name;
mod name_languages;
mod fvar;

mod os2;

pub use io::{
    records_fit, window, read_fixed, read_u16_be, read_u32_be, read_u24_be, read_i16_be, read_offset24, search_records,
    write_u16_be, write_u32_be, write_i16_be, write_offset24,
};
pub(crate) use io::{
    add_i16, bin_search_header, bump_i16, bump_u16, clamp_i16, f2dot14, Fingerprint, FingerprintIndex, fitting_count,
    fixed, midpoint_search, offset16_in, read_f2dot14, read_i32_be, resolve_offset16, Placed, OFFSET24_MAX,
};
pub(crate) use io::read_uint;
pub use os2::{parse_os2, Os2Fields, FS_BOLD, FS_ITALIC, FS_OBLIQUE, FS_REGULAR, FS_USE_TYPO_METRICS};
pub use ttf::{build_ttf, extract_ttf_tables, extract_ttc_tables, pad4};
pub use ttf::{face_count, is_collection};
pub(crate) use ttf::ONE_FACE;
pub(crate) use ttf::{
    copies_held, extract_tables_in_place, glyf_tags, glyph_count, num_glyphs, ttf_table_ranges, GlyfTags,
};
pub(crate) use name::{
    each_name_string, encode_name, is_english, lang_tags, mac_roman_byte, mac_roman_char, mac_turkish_byte,
    mac_turkish_char, name_records, read_records_only, write_name_table, NameKey, NameRecords, NameSink, NameText,
};
pub use name::{
    read_font_family_name, read_font_style, parse_all_name_strings, read_localized_names, read_name_string, FontStyle,
    LocalizedName,
};
#[cfg(feature = "capi")]
pub(crate) use name::each_localized_name;
pub use fvar::{is_variable_font, read_fvar_instances, parse_fvar_axes, Axis, NamedInstance};
pub(crate) use fvar::{axis_records, instance_records};
