use alloc::string::String;
use alloc::collections::BTreeMap;

// Each table's (offset, length), from `decoder::ttf_table_ranges`.
pub type Dir = BTreeMap<String, (usize, usize)>;

pub fn slice_table<'a>(ttf: &'a [u8], map: &Dir, tag: &str) -> Option<&'a [u8]> {
    map.get(tag).and_then(|&(offset, length)| ttf.get(offset..offset.checked_add(length)?))
}
