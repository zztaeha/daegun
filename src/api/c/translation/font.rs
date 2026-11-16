// font.h: opening a font, what it is, its axes, named instances and locations, names, tables and caches.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::mem::MaybeUninit;

use crate::ffi::base::{Blob, OnNul, StrList, Text};
use crate::ffi::handle::{
    Bytes, Status, Str, borrow, borrow_mut, cleared, deliver, deliver_result, deliver_reused, failed, free,
    into_buffer, invalid, past_end, put, put_some, reused_box, slice_of, view,
};
use crate::{Cache, CachePool, Font, FontStyle, GlyphId, Location, NameId, Tag};

// opening

std::thread_local! {
    // A freed font's box, its font dropped, for the thread's next open: a new box each open took C's static open
    // 25 ns over Rust's.
    static SPARE_FONT: core::cell::Cell<Option<Box<MaybeUninit<Font>>>> = const { core::cell::Cell::new(None) };
}

// A font opened into the box the thread's last freed one left, else a new one; `out` is already cleared.
unsafe fn opened(out: *mut *mut Font, font: crate::Result<Font>) -> Status {
    match font {
        Ok(font) => {
            let boxed = match SPARE_FONT.try_with(core::cell::Cell::take).ok().flatten() {
                Some(spare) => Box::write(spare, font),
                None => Box::new(font),
            };
            unsafe { *out = Box::into_raw(boxed) };
            Status::Ok
        }
        Err(e) => failed(e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_open(data: *const u8, len: usize, out: *mut *mut Font) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(bytes) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    unsafe { opened(out, Font::from_bytes(bytes)) }
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_font_buffer_new(len: usize) -> *mut u8 {
    let Ok(layout) = core::alloc::Layout::array::<u8>(len) else { return core::ptr::null_mut() };
    if len == 0 {
        return core::ptr::null_mut();
    }
    // Exactly `len` bytes, the layout `Vec::from_raw_parts` rebuilds in the two calls that take it back. NULL
    // when the allocation fails, where `vec!` would abort the caller.
    unsafe { alloc::alloc::alloc_zeroed(layout) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_buffer_free(data: *mut u8, len: usize) {
    if data.is_null() || len == 0 {
        return;
    }
    // daegun_font_buffer_new is the only source and allocates exactly `len`, which makes rebuilding the Vec sound.
    drop(unsafe { Vec::from_raw_parts(data, len, len) });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_open_owned(
    data: *mut u8,
    len: usize,
    index: usize,
    out: *mut *mut Font,
) -> Status {
    // Taken before `out` is looked at, as the header hands the buffer over whatever this returns.
    let bytes = match (len, data.is_null()) {
        (0, _) => Vec::new(),
        (_, true) => {
            let _ = unsafe { cleared(out) };
            return Status::Null;
        }
        _ => unsafe { Vec::from_raw_parts(data, len, len) },
    };
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    unsafe { opened(out, Font::from_vec(bytes, index)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_open_collection(
    data: *const u8,
    len: usize,
    index: usize,
    out: *mut *mut Font,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(bytes) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    unsafe { opened(out, Font::from_collection(bytes, index)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_open_static(
    data: *const u8,
    len: usize,
    index: usize,
    out: *mut *mut Font,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    // The header's contract: the bytes outlive every handle made from them, unchanged.
    let Some(bytes): Option<&'static [u8]> = (unsafe { slice_of(data, len) }) else { return Status::Null };
    unsafe { opened(out, Font::from_static(bytes, index)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_free(font: *mut Font) {
    if font.is_null() {
        return;
    }
    unsafe { core::ptr::drop_in_place(font) };
    let emptied = unsafe { Box::from_raw(font.cast::<MaybeUninit<Font>>()) };
    if cfg!(not(daegun_no_spares)) {
        let _ = SPARE_FONT.try_with(|spare| spare.set(Some(emptied)));
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_face_count(data: *const u8, len: usize, out: *mut usize) -> Status {
    let Some(bytes) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    unsafe { put(out, Font::face_count(bytes)) }
}

// what it is

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_glyph_count(font: *const Font, out: *mut u32) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put(out, font.glyph_count()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_units_per_em(font: *const Font, out: *mut u16) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put(out, font.units_per_em()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_is_variable(font: *const Font, out: *mut bool) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put(out, font.is_variable()) }
}

// axes

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AxisC {
    tag: u32,
    name_id: u16,
    hidden: bool,
    min: f64,
    default_value: f64,
    max: f64,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<AxisC>() == 32 && core::mem::offset_of!(AxisC, min) == 8);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_axes(
    font: *const Font,
    out: *mut AxisC,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let axes = font.axes().into_iter().map(|a| AxisC {
        tag: a.tag.0,
        name_id: a.name_id.0,
        hidden: a.hidden,
        min: a.min,
        default_value: a.default,
        max: a.max,
    });
    unsafe { into_buffer(axes, out, out_len, out_count) }
}

// named instances

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Coord {
    pub(crate) tag: u32,
    pub(crate) value: f64,
}

#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Coord>() == 16);

impl crate::daecore::daekit::cache::AxisPair for Coord {
    fn axis_id(&self) -> Option<u32> {
        Some(self.tag)
    }

    fn value(&self) -> f64 {
        self.value
    }
}

fn coords_of(coords: &[(Tag, f64)]) -> Vec<Coord> {
    coords.iter().map(|&(tag, value)| Coord { tag: tag.0, value }).collect()
}

struct Instance {
    has_name: bool,
    has_postscript_name: bool,
    coords: core::ops::Range<usize>,
}

// Every instance's coordinates in one list, and its name and PostScript name in two slots of one string buffer.
pub struct NamedInstances {
    instances: Vec<Instance>,
    coords: Vec<Coord>,
    names: StrList,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_named_instances(font: *const Font, out: *mut *mut NamedInstances) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let named = font.named_instances();
    let mut coords = Vec::with_capacity(named.iter().map(|i| i.coords.len()).sum());
    let instances = (named.iter())
        .map(|i| {
            let start = coords.len();
            coords.extend(i.coords.iter().map(|&(tag, value)| Coord { tag: tag.0, value }));
            let (has_name, has_postscript_name) = (i.name.is_some(), i.postscript_name.is_some());
            Instance { has_name, has_postscript_name, coords: start..coords.len() }
        })
        .collect();
    let names = named.iter().flat_map(|i| [i.name.as_deref(), i.postscript_name.as_deref()]);
    let names = StrList::new(names.map(|n| n.unwrap_or("")), OnNul::Replace);
    unsafe { deliver(out, NamedInstances { instances, coords, names }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_named_instances_count(list: *const NamedInstances, out: *mut usize) -> Status {
    let Some(list) = (unsafe { borrow(list) }) else { return Status::Null };
    unsafe { put(out, list.instances.len()) }
}

// Slot 0 of an instance's two is its name, slot 1 its PostScript name.
unsafe fn instance_name(list: *const NamedInstances, index: usize, out: *mut Str, slot: usize) -> Status {
    let Some(list) = (unsafe { borrow(list) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(i) = list.instances.get(index) else { return past_end("named instance", index, list.instances.len()) };
    let named = if slot == 0 { i.has_name } else { i.has_postscript_name };
    unsafe { put_some(out, named.then(|| list.names.get(2 * index + slot)).flatten()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_named_instances_name(
    list: *const NamedInstances,
    index: usize,
    out: *mut Str,
) -> Status {
    unsafe { instance_name(list, index, out, 0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_named_instances_postscript_name(
    list: *const NamedInstances,
    index: usize,
    out: *mut Str,
) -> Status {
    unsafe { instance_name(list, index, out, 1) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_named_instances_coords(
    list: *const NamedInstances,
    index: usize,
    out_count: *mut usize,
) -> *const Coord {
    unsafe { view(list, out_count, |l| l.instances.get(index).and_then(|i| l.coords.get(i.coords.clone()))) }
}

free!(daegun_named_instances_free, NamedInstances);

// locations

// The coordinates it resolved to are converted for C only once a caller asks for them.
pub struct LocationHandle {
    pub location: Location,
    coords: crate::sync::Once<Vec<Coord>>,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_location(
    font: *const Font,
    coords: *const Coord,
    coords_len: usize,
    out: *mut *mut LocationHandle,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(given) = (unsafe { slice_of(coords, coords_len) }) else { return Status::Null };
    unsafe { deliver_reused(out, LocationHandle { location: font.resolve(given), coords: crate::sync::Once::new() }) }
}

reused_box!(daegun_location_free, LocationHandle, LocationHandle {
    location: Location::default(),
    coords: crate::sync::Once::new(),
});

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_location_coords(
    location: *const LocationHandle,
    out_count: *mut usize,
) -> *const Coord {
    unsafe { view(location, out_count, |l| Some(&l.coords.get_or_init(|| coords_of(l.location.coords()))[..])) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_location_normalized(
    location: *const LocationHandle,
    out_count: *mut usize,
) -> *const f64 {
    unsafe { view(location, out_count, |l| Some(l.location.normalized())) }
}

// names

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_family_name(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut *mut Text,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    match font.family_name(unsafe { crate::ffi::at(location) }) {
        Some(name) => unsafe { give_text(out, name) },
        None => Status::Absent,
    }
}

// Into an out `cleared` made sure of.
unsafe fn give_text(out: *mut *mut Text, name: alloc::string::String) -> Status {
    unsafe { *out = Text::new(name) };
    Status::Ok
}

pub const STYLE_NORMAL: i32 = 0;
pub const STYLE_ITALIC: i32 = 1;
pub const STYLE_OBLIQUE: i32 = 2;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_style(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut i32,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let style = match font.style(unsafe { crate::ffi::at(location) }) {
        FontStyle::Normal => STYLE_NORMAL,
        FontStyle::Italic => STYLE_ITALIC,
        FontStyle::Oblique => STYLE_OBLIQUE,
    };
    unsafe { put(out, style) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_name(
    font: *const Font,
    name_id: u16,
    location: *const LocationHandle,
    out: *mut *mut Text,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    match font.name(NameId(name_id), unsafe { crate::ffi::at(location) }) {
        Some(name) => unsafe { give_text(out, name) },
        None => Status::Absent,
    }
}

// Each name's ID, ascending, and at the same index its string, decoded straight into the buffer.
pub struct Names {
    pub(crate) ids: Vec<u16>,
    pub(crate) strings: StrList,
}

impl Names {
    pub(crate) fn new() -> Names {
        Names { ids: Vec::new(), strings: StrList::with_capacity(0, 0) }
    }
}

impl crate::ffi::handle::Reused for Names {
    fn empty() -> Names {
        Names::new()
    }

    fn emptied(&mut self) -> usize {
        self.ids.clear();
        crate::daecore::daekit::base::sync::heap_of(&self.ids) + self.strings.emptied()
    }

    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<Box<Names>>>> {
        std::thread_local! {
            static SPARE: core::cell::Cell<Option<Box<Names>>> = const { core::cell::Cell::new(None) };
        }
        &SPARE
    }
}

impl crate::daecore::daetype::decoder::NameSink for Names {
    // A name as stored takes at least the bytes it takes decoded, but for CJK's two of UTF-16 to three of UTF-8.
    fn reserve(&mut self, count: usize, stored: usize) {
        self.ids.reserve(count);
        self.strings.reserve(stored + count, count);
    }

    fn put(&mut self, name_id: u16, name: &str) {
        self.ids.push(name_id);
        self.strings.push_str(name);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_names(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut *mut Names,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let mut names = crate::ffi::handle::reused::<Names>();
    font.names_into(unsafe { crate::ffi::at(location) }, &mut *names);
    names.strings = core::mem::take(&mut names.strings).finish(OnNul::Replace);
    unsafe { *out = Box::into_raw(names) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_names_count(names: *const Names, out: *mut usize) -> Status {
    let Some(names) = (unsafe { borrow(names) }) else { return Status::Null };
    unsafe { put(out, names.ids.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_names_at(names: *const Names, index: usize, out_id: *mut u16, out: *mut Str) -> Status {
    let Some(names) = (unsafe { borrow(names) }) else { return Status::Null };
    if out_id.is_null() || out.is_null() {
        return Status::Null;
    }
    let (Some(&id), Some(name)) = (names.ids.get(index), names.strings.get(index)) else {
        return past_end("name", index, names.ids.len());
    };
    unsafe { *out_id = id };
    unsafe { put(out, name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_names_free(names: *mut Names) {
    unsafe { crate::ffi::handle::give_back(names) }
}

// Each language's tag, sorted, and at the same index the name in it.
pub struct LocalizedNames {
    languages: StrList,
    names: StrList,
}

impl crate::ffi::handle::Reused for LocalizedNames {
    fn empty() -> LocalizedNames {
        LocalizedNames { languages: StrList::default(), names: StrList::default() }
    }

    fn emptied(&mut self) -> usize {
        self.languages.emptied() + self.names.emptied()
    }

    fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<Box<LocalizedNames>>>> {
        std::thread_local! {
            static SPARE: core::cell::Cell<Option<Box<LocalizedNames>>> = const { core::cell::Cell::new(None) };
        }
        &SPARE
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_localized_names(
    font: *const Font,
    name_id: u16,
    location: *const LocationHandle,
    out: *mut *mut LocalizedNames,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let mut held = crate::ffi::handle::reused::<LocalizedNames>();
    let LocalizedNames { languages, names } = &mut *held;
    font.localized_names_into(NameId(name_id), unsafe { crate::ffi::at(location) }, |language, name| {
        languages.push_str(language);
        names.push_str(name);
    });
    *languages = core::mem::take(languages).finish(OnNul::Replace);
    *names = core::mem::take(names).finish(OnNul::Replace);
    unsafe { *out = Box::into_raw(held) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_localized_names_count(names: *const LocalizedNames, out: *mut usize) -> Status {
    let Some(names) = (unsafe { borrow(names) }) else { return Status::Null };
    unsafe { put(out, names.languages.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_localized_names_at(
    names: *const LocalizedNames,
    index: usize,
    out_language: *mut Str,
    out: *mut Str,
) -> Status {
    let Some(names) = (unsafe { borrow(names) }) else { return Status::Null };
    if out_language.is_null() || out.is_null() {
        return Status::Null;
    }
    let (Some(language), Some(name)) = (names.languages.get(index), names.names.get(index)) else {
        return past_end("localized name", index, names.languages.len());
    };
    unsafe { *out_language = language };
    unsafe { put(out, name) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_localized_names_free(names: *mut LocalizedNames) {
    unsafe { crate::ffi::handle::give_back(names) }
}

// tables and instances

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_table(font: *const Font, tag: u32, out: *mut Bytes) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put_some(out, font.table(Tag(tag)).map(Bytes::of)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_table_tags(
    font: *const Font,
    out: *mut u32,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { into_buffer(font.table_tags().map(|t| t.0), out, out_len, out_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_has_table(font: *const Font, tag: u32, out: *mut bool) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put(out, font.has_table(Tag(tag))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_instance(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut *mut Blob,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    match font.instance(unsafe { crate::ffi::at(location) }) {
        Ok(bytes) => unsafe { deliver_reused(out, Blob::Shared(bytes)) },
        Err(e) => failed(e),
    }
}

// Sorted by tag, each once: a C caller walks it by index, which a tree took a step per table to reach.
pub struct TableMap(pub Vec<(Tag, Vec<u8>)>);

impl TableMap {
    fn find(&self, tag: u32) -> Result<usize, usize> {
        self.0.binary_search_by_key(&Tag(tag), |&(t, _)| t)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_instance_tables(
    font: *const Font,
    location: *const LocationHandle,
    out: *mut *mut TableMap,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let tables = font.instance_tables(unsafe { crate::ffi::at(location) });
    let copied = tables.map(|t| TableMap(t.into_iter().map(|(tag, data)| (tag, data.into_owned())).collect()));
    unsafe { deliver_result(out, copied) }
}

free!(daegun_table_map_free, TableMap);

#[unsafe(no_mangle)]
pub extern "C" fn daegun_table_map_new() -> *mut TableMap {
    crate::ffi::handle::new(TableMap(Vec::new()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_count(map: *const TableMap, out: *mut usize) -> Status {
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    unsafe { put(out, map.0.len()) }
}

unsafe fn table_at<T>(map: *const TableMap, index: usize, out: *mut T, of: fn(&Tag, &Vec<u8>) -> T) -> Status {
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    match map.0.get(index) {
        Some((tag, data)) => unsafe { put(out, of(tag, data)) },
        None => past_end("table", index, map.0.len()),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_tag_at(map: *const TableMap, index: usize, out: *mut u32) -> Status {
    unsafe { table_at(map, index, out, |tag, _| tag.0) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_bytes_at(map: *const TableMap, index: usize, out: *mut Bytes) -> Status {
    unsafe { table_at(map, index, out, |_, data| Bytes::of(data)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_get(map: *const TableMap, tag: u32, out: *mut Bytes) -> Status {
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    unsafe { put_some(out, map.find(tag).ok().map(|i| Bytes::of(&map.0[i].1))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_set(map: *mut TableMap, tag: u32, data: *const u8, len: usize) -> Status {
    let Some(map) = (unsafe { borrow_mut(map) }) else { return Status::Null };
    let Some(bytes) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    match map.find(tag) {
        Ok(i) => map.0[i].1 = bytes.to_vec(),
        Err(i) => map.0.insert(i, (Tag(tag), bytes.to_vec())),
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_remove(map: *mut TableMap, tag: u32) -> Status {
    let Some(map) = (unsafe { borrow_mut(map) }) else { return Status::Null };
    if let Ok(i) = map.find(tag) {
        map.0.remove(i);
    }
    Status::Ok
}

// caches

pub const CACHE_SHAPE: i32 = 0;
pub const CACHE_INSTANCE: i32 = 1;
pub const CACHE_OUTLINE: i32 = 2;
pub const CACHE_RASTER: i32 = 3;

fn cache_of(code: i32) -> Result<Cache, Status> {
    match code {
        CACHE_SHAPE => Ok(Cache::Shape),
        CACHE_INSTANCE => Ok(Cache::Instance),
        CACHE_OUTLINE => Ok(Cache::Outline),
        CACHE_RASTER => Ok(Cache::Raster),
        _ => Err(invalid(format_args!("cache {code} is none of DAEGUN_CACHE_*"))),
    }
}

#[repr(C)]
pub struct CacheStatsC {
    entries: usize,
    bytes: usize,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_set_cache_budget(font: *const Font, cache: i32, bytes: usize) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    font.set_cache_budget(crate::ffi::handle::or_return!(cache_of(cache)), bytes);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_cache_stats(font: *const Font, cache: i32, out: *mut CacheStatsC) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let stats = font.cache_stats(crate::ffi::handle::or_return!(cache_of(cache)));
    unsafe { put(out, CacheStatsC { entries: stats.entries, bytes: stats.bytes }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_clear_cache(font: *const Font, cache: i32) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    font.clear_cache(crate::ffi::handle::or_return!(cache_of(cache)));
    Status::Ok
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_cache_pool_new(bytes: usize) -> *mut CachePool {
    crate::ffi::handle::new(CachePool::new(bytes))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cache_pool_free(pool: *mut CachePool) {
    unsafe { crate::ffi::handle::release(pool) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cache_pool_set_budget(pool: *const CachePool, bytes: usize) -> Status {
    let Some(pool) = (unsafe { borrow(pool) }) else { return Status::Null };
    pool.set_budget(bytes);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_cache_pool_stats(pool: *const CachePool, out: *mut CacheStatsC) -> Status {
    let Some(pool) = (unsafe { borrow(pool) }) else { return Status::Null };
    let stats = pool.stats();
    unsafe { put(out, CacheStatsC { entries: stats.entries, bytes: stats.bytes }) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_set_cache_pool(font: *const Font, cache: i32, pool: *const CachePool) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let cache = crate::ffi::handle::or_return!(cache_of(cache));
    font.set_cache_pool(cache, unsafe { borrow(pool) });
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_prewarm(
    font: *const Font,
    gids: *const u32,
    gids_len: usize,
    location: *const LocationHandle,
    out_added: *mut usize,
) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    let Some(gids) = (unsafe { slice_of(gids, gids_len) }) else { return Status::Null };
    let added = font.prewarm(gids.iter().map(|&g| GlyphId(g)), unsafe { crate::ffi::at(location) });
    unsafe { crate::ffi::handle::fill(out_added, added) };
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_set_index_allowance(font: *const Font, bytes: usize) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    font.set_index_allowance(bytes);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_font_index_allowance(font: *const Font, out: *mut usize) -> Status {
    let Some(font) = (unsafe { borrow(font) }) else { return Status::Null };
    unsafe { put(out, font.index_allowance()) }
}

pub(crate) fn release_thread() {
    crate::ffi::handle::drop_spare::<LocationHandle>();
    crate::ffi::handle::drop_spare::<Names>();
    crate::ffi::handle::drop_spare::<LocalizedNames>();
    let _ = SPARE_FONT.try_with(core::cell::Cell::take);
}

#[cfg(test)]
#[path = "tests/font.rs"]
mod tests;
