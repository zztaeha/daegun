mod io;
mod ttf;

mod name;
mod fvar;

mod os2;

pub use io::{records_fit, window, read_u16_be, read_u32_be, read_u24_be, read_i16_be, read_offset24, search_records, write_u16_be, write_u32_be, write_i16_be, write_offset24};
pub use os2::{parse_os2, Os2Fields};
pub use ttf::{build_ttf, extract_ttf_tables, extract_ttf_tables_owned, extract_ttc_tables, pad4, ttc_font_count};
pub(crate) use ttf::extract_ttf_tables_shared;
pub(crate) use name::{mac_roman_byte, mac_roman_char, mac_turkish_byte, mac_turkish_char};
pub use name::{read_font_family_name, read_font_style, parse_all_name_strings, read_name_string};
pub use fvar::{is_variable_font, read_fvar_instances, parse_fvar_axes, NamedInstance, FvarAxis};
pub(crate) use fvar::read_fvar_instances_named;
