// SAFETY, once for the file. Every entry point null-checks its pointers and answers
// `Status::Null` before dereferencing anything, so the `unsafe` blocks below rest on a check
// immediately above them. Where a call takes a raw buffer, its length is the caller's promise
// from `daegun.h` and is not checkable here.

use alloc::borrow::ToOwned;
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::ffi::{c_char, c_void};

use crate::{Font, bytes, format};

use crate::ffi::handle::{Bytes, OwnedStr, Status, Str, borrow, deliver, release, slice_of, str_of};
use crate::ffi::list::{Axis, Blob, F64List, GlyphValue, GlyphValueList, StrList, U16List, UsizeList, axes_of};
use crate::ffi::pen::{Pen, PenBridge};
use crate::ffi::set_error;

#[inline]
unsafe fn put<T>(out: *mut T, value: T) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    unsafe { *out = value };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_table(
    font: *const Font,
    tag: *const c_char,
    out: *mut Bytes,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(tag) = (unsafe { str_of(tag) }) else { return Status::Null };
    match font.table(tag) {
        Some(data) => unsafe { put(out, Bytes::of(data)) },
        None => {
            if out.is_null() {
                return Status::Null;
            }
            // Cleared so a caller who ignores the status reads an empty run rather than whatever
            // was in the variable.
            unsafe { *out = Bytes::EMPTY };
            Status::Absent
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_table_tags(
    font: *const Font,
    out: *mut *mut StrList,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let tags = font.table_tags().into_iter().map(str::to_owned);
    unsafe { deliver(out, StrList::new(tags)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_has_table(
    font: *const Font,
    tag: *const c_char,
    out: *mut i32,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(tag) = (unsafe { str_of(tag) }) else { return Status::Null };
    unsafe { put(out, i32::from(font.has_table(tag))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_instance_tables(
    font: *const Font,
    axes: *const Axis,
    axis_count: usize,
    out: *mut *mut TableMap,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(axes) = (unsafe { axes_of(axes, axis_count) }) else { return Status::Null };
    let Some(tables) = font.instance_tables(&axes) else { return Status::Absent };
    let owned = tables.into_iter().map(|(tag, data)| (tag, data.into_owned())).collect();
    unsafe { deliver(out, TableMap::from_map(owned)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_instance_table(
    font: *const Font,
    axes: *const Axis,
    axis_count: usize,
    tag: *const c_char,
    out: *mut *mut Blob,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(tag) = (unsafe { str_of(tag) }) else { return Status::Null };
    let Some(axes) = (unsafe { axes_of(axes, axis_count) }) else { return Status::Null };
    let Some(tables) = font.instance_tables(&axes) else { return Status::Absent };
    let Some(data) = tables.get(tag) else { return Status::Absent };
    unsafe { deliver(out, Blob(data.to_vec())) }
}

pub struct TableMap {
    tables: BTreeMap<String, Vec<u8>>,
    tags: Vec<OwnedStr>,
}

impl TableMap {
    fn from_map(tables: BTreeMap<String, Vec<u8>>) -> TableMap {
        let tags = tables.keys().map(|t| OwnedStr::new(t)).collect();
        TableMap { tables, tags }
    }

    fn resync(&mut self) {
        self.tags = self.tables.keys().map(|t| OwnedStr::new(t)).collect();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_table_map_new() -> *mut TableMap {
    alloc::boxed::Box::into_raw(alloc::boxed::Box::new(TableMap::from_map(BTreeMap::new())))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_count(map: *const TableMap, out: *mut usize) -> Status {
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    unsafe { put(out, map.tables.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_tag_at(
    map: *const TableMap,
    index: usize,
    out: *mut Str,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    let Some(tag) = map.tags.get(index) else {
        return crate::ffi::range("no tag at that index: it is past the map's end");
    };
    unsafe { put(out, tag.as_str()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_bytes_at(
    map: *const TableMap,
    index: usize,
    out: *mut Bytes,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    let Some((_, data)) = map.tables.iter().nth(index) else {
        return crate::ffi::range("no table at that index: it is past the map's end");
    };
    unsafe { put(out, Bytes::of(data)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_get(
    map: *const TableMap,
    tag: *const c_char,
    out: *mut Bytes,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    let Some(tag) = (unsafe { str_of(tag) }) else { return Status::Null };
    match map.tables.get(tag) {
        Some(data) => unsafe { put(out, Bytes::of(data)) },
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_set(
    map: *mut TableMap,
    tag: *const c_char,
    data: *const u8,
    len: usize,
) -> Status {
    if map.is_null() {
        return Status::Null;
    }
    let Some(tag) = (unsafe { str_of(tag) }) else { return Status::Null };
    let map = unsafe { &mut *map };
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    map.tables.insert(tag.to_string(), data.to_vec());
    map.resync();
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_remove(
    map: *mut TableMap,
    tag: *const c_char,
) -> Status {
    if map.is_null() {
        return Status::Null;
    }
    let Some(tag) = (unsafe { str_of(tag) }) else { return Status::Null };
    let map = unsafe { &mut *map };
    if map.tables.remove(tag).is_none() {
        return Status::Absent;
    }
    map.resync();
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_build(
    map: *const TableMap,
    out: *mut *mut Blob,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    if map.tables.is_empty() {
        set_error("cannot build a font from an empty table map");
        return Status::Range;
    }
    unsafe { deliver(out, Blob(crate::build_font(&map.tables))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_free(map: *mut TableMap) {
    unsafe { release(map) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_parse_loca(
    loca: *const u8,
    len: usize,
    format: i16,
    num_glyphs: usize,
    out: *mut *mut UsizeList,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(loca) = (unsafe { slice_of(loca, len) }) else { return Status::Null };
    if num_glyphs > usize::from(u16::MAX) {
        crate::ffi::set_error("num_glyphs is past 65,535, which no font has");
        return Status::Range;
    }
    unsafe { deliver(out, UsizeList(crate::parse_loca(loca, format, num_glyphs))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_outline_glyf_bytes(
    glyf: *const u8,
    glyf_len: usize,
    loca: *const usize,
    loca_len: usize,
    glyph: u16,
    pen: *const Pen,
) -> Status {
    let Some(pen) = (unsafe { borrow(pen) }) else { return Status::Null };
    let Some(glyf) = (unsafe { slice_of(glyf, glyf_len) }) else { return Status::Null };
    let Some(loca) = (unsafe { slice_of(loca, loca_len) }) else { return Status::Null };
    let mut bridge = PenBridge(*pen);
    match crate::outline_glyf_bytes(glyf, loca, glyph, &mut bridge) {
        Ok(()) => Status::Ok,
        Err(e) => {
            set_error(&e);
            Status::Parse
        }
    }
}

macro_rules! reader {
    ($name:ident, $call:path, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            data: *const u8,
            len: usize,
            off: usize,
            out: *mut $ty,
        ) -> Status {
            if out.is_null() {
                return Status::Null;
            }
            let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
            match $call(data, off) {
                Some(v) => unsafe { put(out, v) },
                None => Status::Range,
            }
        }
    };
}

reader!(daegun_read_u16_be, bytes::read_u16_be, u16);
reader!(daegun_read_i16_be, bytes::read_i16_be, i16);
reader!(daegun_read_u24_be, bytes::read_u24_be, u32);
reader!(daegun_read_u32_be, bytes::read_u32_be, u32);
reader!(daegun_read_offset24, bytes::read_offset24, usize);

macro_rules! writer {
    ($name:ident, $call:path, $ty:ty, $width:expr) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(
            data: *mut u8,
            len: usize,
            off: usize,
            value: $ty,
        ) -> Status {
            if data.is_null() {
                return Status::Null;
            }
            if off.checked_add($width).is_none_or(|end| end > len) {
                return Status::Range;
            }
            let data = unsafe { core::slice::from_raw_parts_mut(data, len) };
            $call(data, off, value);
            Status::Ok
        }
    };
}

writer!(daegun_write_u16_be, bytes::write_u16_be, u16, 2);
writer!(daegun_write_i16_be, bytes::write_i16_be, i16, 2);
writer!(daegun_write_u32_be, bytes::write_u32_be, u32, 4);
writer!(daegun_write_offset24, bytes::write_offset24, usize, 3);

#[unsafe(no_mangle)]
pub extern "C" fn daegun_records_fit(
    start: usize,
    count: usize,
    stride: usize,
    len: usize,
) -> i32 {
    i32::from(bytes::records_fit(start, count, stride, len))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_bytes_window(
    data: *const u8,
    len: usize,
    off: usize,
    n: usize,
) -> *const u8 {
    let Some(slice) = (unsafe { slice_of(data, len) }) else { return core::ptr::null() };
    match off.checked_add(n) {
        Some(end) if end <= slice.len() => slice[off..end].as_ptr(),
        _ => core::ptr::null(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_search_records(
    count: usize,
    target: u32,
    key_at: Option<extern "C" fn(usize, *mut c_void, *mut u32) -> i32>,
    user: *mut c_void,
    out_index: *mut usize,
    out_found: *mut i32,
) -> Status {
    let Some(key_at) = key_at else { return Status::Null };
    if out_index.is_null() || out_found.is_null() {
        return Status::Null;
    }
    let found = bytes::search_records(count, target, |i| {
        let mut key = 0u32;
        (key_at(i, user, &mut key) != 0).then_some(key)
    });
    let Some(result) = found else { return Status::Absent };
    let (index, hit) = match result {
        Ok(i) => (i, 1),
        Err(i) => (i, 0),
    };
    unsafe {
        *out_index = index;
        *out_found = hit;
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_ot_round(value: f64) -> i32 {
    format::ot_round(value)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_coverage_index(
    data: *const u8,
    len: usize,
    glyph: u16,
    out: *mut u16,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    match format::coverage_index(data, glyph) {
        Some(i) => unsafe { put(out, i) },
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_coverage_glyphs(
    buf: *const u8,
    len: usize,
    off: usize,
    out: *mut *mut U16List,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(buf) = (unsafe { slice_of(buf, len) }) else { return Status::Null };
    match format::coverage_glyphs(buf, off) {
        Ok(glyphs) => unsafe { deliver(out, U16List(glyphs)) },
        Err(e) => {
            set_error(&e);
            Status::Parse
        }
    }
}

pub struct AatLookup {
    data: Vec<u8>,
    num_glyphs: u16,
}

impl AatLookup {
    fn view(&self) -> Option<format::Lookup<'_>> {
        format::Lookup::parse(&self.data, self.num_glyphs)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_lookup_open(
    data: *const u8,
    len: usize,
    num_glyphs: u16,
    out: *mut *mut AatLookup,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    if format::Lookup::parse(data, num_glyphs).is_none() {
        set_error("aat lookup did not parse");
        return Status::Parse;
    }
    unsafe { deliver(out, AatLookup { data: data.to_vec(), num_glyphs }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_lookup_value(
    lookup: *const AatLookup,
    glyph: u16,
    out: *mut u16,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(lookup) = (unsafe { borrow(lookup) }) else { return Status::Null };
    let Some(view) = lookup.view() else { return Status::Parse };
    match view.value(glyph) {
        Some(v) => unsafe { put(out, v) },
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_lookup_entries(
    lookup: *const AatLookup,
    out: *mut *mut GlyphValueList,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(lookup) = (unsafe { borrow(lookup) }) else { return Status::Null };
    let Some(view) = lookup.view() else { return Status::Parse };
    let entries =
        view.entries().into_iter().map(|(glyph, value)| GlyphValue { glyph, value }).collect();
    unsafe { deliver(out, GlyphValueList(entries)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_lookup_free(lookup: *mut AatLookup) {
    unsafe { release(lookup) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AatEntry {
    pub new_state: u16,
    pub flags: u16,
    pub word1: u16,
    pub word2: u16,
}

const _: () = assert!(size_of::<AatEntry>() == 8);
const _: () = assert!(align_of::<AatEntry>() == 2);

pub struct AatStateTable {
    data: Vec<u8>,
    extra_words: usize,
    num_glyphs: u16,
}

impl AatStateTable {
    fn view(&self) -> Option<format::StateTable<'_>> {
        format::StateTable::parse(&self.data, self.extra_words, self.num_glyphs)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_state_table_open(
    data: *const u8,
    len: usize,
    extra_words: usize,
    num_glyphs: u16,
    out: *mut *mut AatStateTable,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    if format::StateTable::parse(data, extra_words, num_glyphs).is_none() {
        set_error("aat state table did not parse");
        return Status::Parse;
    }
    unsafe { deliver(out, AatStateTable { data: data.to_vec(), extra_words, num_glyphs }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_state_table_class(
    table: *const AatStateTable,
    glyph: u16,
    out: *mut u16,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(table) = (unsafe { borrow(table) }) else { return Status::Null };
    let Some(view) = table.view() else { return Status::Parse };
    unsafe { put(out, view.class(glyph)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_state_table_entry(
    table: *const AatStateTable,
    state: u16,
    class: u16,
    out: *mut AatEntry,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(table) = (unsafe { borrow(table) }) else { return Status::Null };
    let Some(view) = table.view() else { return Status::Parse };
    let Some(format::Entry { new_state, flags, word1, word2 }) = view.entry(state, class) else { return Status::Range };
    unsafe { put(out, AatEntry { new_state, flags, word1, word2 }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_state_table_free(table: *mut AatStateTable) {
    unsafe { release(table) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_version(
    data: *const u8,
    len: usize,
    out: *mut u16,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    match format::ankr_version(data) {
        Some(v) => unsafe { put(out, v) },
        None => Status::Range,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_control_point(
    data: *const u8,
    len: usize,
    at: usize,
    out_x: *mut i16,
    out_y: *mut i16,
) -> Status {
    if out_x.is_null() || out_y.is_null() {
        return Status::Null;
    }
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    let Some((x, y)) = format::control_point(data, at) else { return Status::Range };
    unsafe {
        *out_x = x;
        *out_y = y;
    }
    Status::Ok
}

pub struct Ankr {
    data: Vec<u8>,
    num_glyphs: u16,
}

impl Ankr {
    fn view(&self) -> Option<format::Ankr<'_>> {
        format::Ankr::parse(&self.data, self.num_glyphs)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_open(
    data: *const u8,
    len: usize,
    num_glyphs: u16,
    out: *mut *mut Ankr,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    if format::Ankr::parse(data, num_glyphs).is_none() {
        set_error("ankr table did not parse");
        return Status::Parse;
    }
    unsafe { deliver(out, Ankr { data: data.to_vec(), num_glyphs }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_point_count(
    ankr: *const Ankr,
    glyph: u16,
    out: *mut u32,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(ankr) = (unsafe { borrow(ankr) }) else { return Status::Null };
    let Some(view) = ankr.view() else { return Status::Parse };
    unsafe { put(out, view.point_count(glyph)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_anchor_point(
    ankr: *const Ankr,
    glyph: u16,
    index: u16,
    out_x: *mut i16,
    out_y: *mut i16,
) -> Status {
    if out_x.is_null() || out_y.is_null() {
        return Status::Null;
    }
    let Some(ankr) = (unsafe { borrow(ankr) }) else { return Status::Null };
    let Some(view) = ankr.view() else { return Status::Parse };
    let Some((x, y)) = view.anchor_point(glyph, index) else { return Status::Absent };
    unsafe {
        *out_x = x;
        *out_y = y;
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_free(ankr: *mut Ankr) {
    unsafe { release(ankr) }
}

pub struct FeatureVariations {
    layout: Vec<u8>,
    at: Option<usize>,
}

impl FeatureVariations {
    fn view(&self) -> Option<format::FeatureVariations<'_>> {
        match self.at {
            Some(at) => Some(format::FeatureVariations::at(&self.layout, at)),
            None => format::FeatureVariations::parse(&self.layout),
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_variations_open(
    layout: *const u8,
    len: usize,
    out: *mut *mut FeatureVariations,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(layout) = (unsafe { slice_of(layout, len) }) else { return Status::Null };
    if format::FeatureVariations::parse(layout).is_none() {
        return Status::Absent;
    }
    unsafe { deliver(out, FeatureVariations { layout: layout.to_vec(), at: None }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_variations_at(
    layout: *const u8,
    len: usize,
    at: usize,
    out: *mut *mut FeatureVariations,
) -> Status {
    let Some(layout) = (unsafe { slice_of(layout, len) }) else { return Status::Null };
    unsafe { deliver(out, FeatureVariations { layout: layout.to_vec(), at: Some(at) }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_variations_find(
    vars: *const FeatureVariations,
    coords: *const i32,
    coord_count: usize,
    out: *mut u16,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(vars) = (unsafe { borrow(vars) }) else { return Status::Null };
    let Some(view) = vars.view() else { return Status::Absent };
    let Some(coords) = (unsafe { slice_of(coords, coord_count) }) else { return Status::Null };
    match view.with_axis_count(coord_count).find(coords) {
        Some(v) => unsafe { put(out, v) },
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_variations_substitute(
    vars: *const FeatureVariations,
    variation: u16,
    feature: u16,
    out: *mut usize,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(vars) = (unsafe { borrow(vars) }) else { return Status::Null };
    let Some(view) = vars.view() else { return Status::Absent };
    match view.substitute(variation, feature) {
        Some(off) => unsafe { put(out, off) },
        None => Status::Absent,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_variations_free(vars: *mut FeatureVariations) {
    unsafe { release(vars) }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct RegionAxis {
    pub start: f64,
    pub peak: f64,
    pub end: f64,
}

const _: () = assert!(size_of::<RegionAxis>() == 24);
const _: () = assert!(align_of::<RegionAxis>() == 8);

pub struct Ivs(crate::format::ItemVariationStore);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_parse(
    buf: *const u8,
    len: usize,
    base: usize,
    out: *mut *mut Ivs,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(buf) = (unsafe { slice_of(buf, len) }) else { return Status::Null };
    match format::parse_item_variation_store(buf, base) {
        Ok(store) => unsafe { deliver(out, Ivs(store)) },
        Err(e) => {
            set_error(&e);
            Status::Parse
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_axis_count(ivs: *const Ivs, out: *mut usize) -> Status {
    let Some(Ivs(format::ItemVariationStore { regions: _, ivd_data: _, axis_count })) = (unsafe { borrow(ivs) }) else {
        return Status::Null;
    };
    unsafe { put(out, *axis_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_region_count(ivs: *const Ivs, out: *mut usize) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    unsafe { put(out, ivs.0.regions.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_region_axis(
    ivs: *const Ivs,
    region: usize,
    axis: usize,
    out: *mut RegionAxis,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    let Some(&format::RegionAxis { start, peak, end }) = ivs.0.regions.get(region).and_then(|r| r.get(axis)) else {
        return Status::Range;
    };
    unsafe { put(out, RegionAxis { start, peak, end }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_ivd_count(ivs: *const Ivs, out: *mut usize) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    unsafe { put(out, ivs.0.ivd_data.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_ivd_rows(
    ivs: *const Ivs,
    ivd: usize,
    out: *mut usize,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    let Some(data) = ivs.0.ivd_data.get(ivd) else { return Status::Range };
    unsafe { put(out, data.rows()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_ivd_region_indices(
    ivs: *const Ivs,
    ivd: usize,
    out_count: *mut usize,
) -> *const usize {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return core::ptr::null() };
    if out_count.is_null() {
        return core::ptr::null();
    }
    let Some(data) = ivs.0.ivd_data.get(ivd) else { return core::ptr::null() };
    unsafe { *out_count = data.region_indices.len() };
    data.region_indices.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_ivd_row(
    ivs: *const Ivs,
    ivd: usize,
    inner: usize,
    out_count: *mut usize,
) -> *const i32 {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return core::ptr::null() };
    if out_count.is_null() {
        return core::ptr::null();
    }
    let Some(row) = ivs.0.ivd_data.get(ivd).and_then(|d| d.row(inner)) else {
        return core::ptr::null();
    };
    unsafe { *out_count = row.len() };
    row.as_ptr()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_region_scalars(
    ivs: *const Ivs,
    location: *const f64,
    axis_count: usize,
    out: *mut *mut F64List,
) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    let Some(location) = (unsafe { slice_of(location, axis_count) }) else { return Status::Null };
    unsafe { deliver(out, F64List(format::precompute_region_scalars(&ivs.0, location))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_delta(
    ivs: *const Ivs,
    outer: usize,
    inner: usize,
    scalars: *const f64,
    scalar_count: usize,
    out: *mut f64,
) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    let Some(scalars) = (unsafe { slice_of(scalars, scalar_count) }) else { return Status::Null };
    unsafe { put(out, format::compute_ivs_delta_f64(&ivs.0, outer, inner, scalars)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_free(ivs: *mut Ivs) {
    unsafe { release(ivs) }
}

pub struct DeltaSetIndexMap(Vec<(u32, u32)>);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_delta_set_index_map_parse(
    buf: *const u8,
    len: usize,
    base: usize,
    out: *mut *mut DeltaSetIndexMap,
) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(buf) = (unsafe { slice_of(buf, len) }) else { return Status::Null };
    match format::parse_delta_set_index_map(buf, base) {
        Ok(map) => unsafe { deliver(out, DeltaSetIndexMap(map)) },
        Err(e) => {
            set_error(&e);
            Status::Parse
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_delta_set_index_map_count(
    map: *const DeltaSetIndexMap,
    out: *mut usize,
) -> Status {
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    unsafe { put(out, map.0.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_delta_set_index_map_lookup(
    map: *const DeltaSetIndexMap,
    index: usize,
    out_outer: *mut usize,
    out_inner: *mut usize,
) -> Status {
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    if out_outer.is_null() || out_inner.is_null() {
        return Status::Null;
    }
    let (outer, inner) = format::delta_set_index_map_lookup(&map.0, index);
    unsafe {
        *out_outer = outer;
        *out_inner = inner;
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_delta_set_index_map_free(map: *mut DeltaSetIndexMap) {
    unsafe { release(map) }
}
