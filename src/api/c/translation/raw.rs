// raw.h: bounds-checked reads and writes, loca and glyf, building a font, and the structures several tables share.
// The handles over table bytes keep a copy and parse it again on each call, as Rust's readers borrow what they read.

use alloc::vec::Vec;
use core::ffi::c_void;

use crate::ffi::base::{Blob, GlyphValue};
use crate::ffi::font::TableMap;
use crate::ffi::handle::{
    Reused, Status, apart, borrow, cleared, deliver_reused, deliver_result, failed, free, give_back, into_buffer,
    invalid, keep_spare, or_return, past_end, put, put_some, reused, room_for, slice_mut_of, slice_of, span, view,
};
use crate::ffi::outline::{Pen, PenBridge};
use crate::raw::bytes;
use crate::raw::format::{self, Entry, ItemVariationStore, RegionAxis};

const _: () = assert!(size_of::<Entry>() == 8 && size_of::<RegionAxis>() == 24);

// building fonts

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_table_map_build(map: *const TableMap, out: *mut *mut Blob) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(map) = (unsafe { borrow(map) }) else { return Status::Null };
    match crate::raw::build_font(map.0.iter().map(|(t, d)| (t, d))) {
        Ok(bytes) => unsafe { deliver_reused(out, Blob::Owned(bytes)) },
        Err(e) => failed(e),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_parse_loca(
    loca: *const u8,
    loca_len: usize,
    format: i16,
    num_glyphs: usize,
    out: *mut usize,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(loca) = (unsafe { slice_of(loca, loca_len) }) else { return Status::Null };
    if out.is_null() && out_len > 0 {
        return Status::Null;
    }
    or_return!(apart(&[span(loca.as_ptr(), loca.len()), span(out, out_len), span(out_count, 1)]));
    use crate::daecore::daetype::subsetter::{loca_into, loca_len as count_of};
    // Read straight into the caller's buffer: Rust's list copied out took 1.25× Rust's time.
    let count = count_of(loca.len(), format, num_glyphs);
    if !or_return!(unsafe { room_for(count, !out.is_null(), out_len, out_count) }) {
        return Status::Ok;
    }
    let Some(offsets) = (unsafe { slice_mut_of(out, count) }) else { return Status::Null };
    loca_into(loca, format, offsets);
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_outline_glyf_bytes(
    glyf: *const u8,
    glyf_len: usize,
    extended: bool,
    loca: *const usize,
    loca_len: usize,
    gid: u32,
    pen: *const Pen,
) -> Status {
    let Some(pen) = (unsafe { borrow(pen) }) else { return Status::Null };
    let (Some(glyf), Some(loca)) = (unsafe { (slice_of(glyf, glyf_len), slice_of(loca, loca_len)) }) else {
        return Status::Null;
    };
    match crate::raw::outline_glyf_bytes(glyf, extended, loca, gid, &mut PenBridge(*pen)) {
        Ok(()) => Status::Ok,
        Err(e) => failed(e),
    }
}

// reads and writes

macro_rules! reader {
    ($name:ident, $call:path, $ty:ty) => {
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(data: *const u8, len: usize, off: usize, out: *mut $ty) -> Status {
            if out.is_null() {
                return Status::Null;
            }
            let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
            match $call(data, off) {
                Some(v) => unsafe { put(out, v) },
                None => invalid(format_args!("reading at offset {off} runs past the {len} bytes there are")),
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
        pub unsafe extern "C" fn $name(data: *mut u8, len: usize, off: usize, value: $ty) -> Status {
            let Some(data) = (unsafe { slice_mut_of(data, len) }) else { return Status::Null };
            match $call(data, off, value) {
                true => Status::Ok,
                false => invalid(format_args!(
                    "writing {value} as {} bytes at offset {off} of {len} runs past the end or the field",
                    $width
                )),
            }
        }
    };
}

writer!(daegun_write_u16_be, bytes::write_u16_be, u16, 2);
writer!(daegun_write_i16_be, bytes::write_i16_be, i16, 2);
writer!(daegun_write_u32_be, bytes::write_u32_be, u32, 4);
writer!(daegun_write_offset24, bytes::write_offset24, usize, 3);

#[unsafe(no_mangle)]
pub extern "C" fn daegun_records_fit(start: usize, count: usize, stride: usize, len: usize) -> bool {
    bytes::records_fit(start, count, stride, len)
}

#[unsafe(no_mangle)]
pub extern "C" fn daegun_bytes_window(data: *const u8, len: usize, off: usize, n: usize) -> *const u8 {
    if data.is_null() || !bytes::records_fit(off, n, 1, len) {
        return core::ptr::null();
    }
    // Offset from `data` itself: a slice of length 0 points nowhere C could use.
    data.wrapping_add(off)
}

pub type KeyAt = unsafe extern "C" fn(*mut c_void, usize, *mut u32) -> bool;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_search_records(
    count: usize,
    target: u32,
    key_at: Option<KeyAt>,
    user: *mut c_void,
    out_index: *mut usize,
    out_found: *mut bool,
) -> Status {
    let Some(key_at) = key_at else { return Status::Null };
    if out_index.is_null() || out_found.is_null() {
        return Status::Null;
    }
    let found = bytes::search_records(count, target, |i| {
        let mut key = 0u32;
        unsafe { key_at(user, i, &mut key) }.then_some(key)
    });
    let Some(result) = found else { return Status::Absent };
    let (index, hit) = match result {
        Ok(i) => (i, true),
        Err(i) => (i, false),
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

// Coverage

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_coverage_index(data: *const u8, len: usize, glyph: u32, out: *mut u32) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    unsafe { put_some(out, format::coverage_index(data, glyph)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_coverage_glyphs(
    data: *const u8,
    len: usize,
    off: usize,
    out: *mut u32,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    or_return!(apart(&[span(data.as_ptr(), data.len()), span(out, out_len), span(out_count, 1)]));
    let coverage = or_return!(crate::daecore::daetype::subsetter::otl::Coverage::read(data, off).map_err(failed));
    // Rule 2, as `into_buffer` keeps it: a buffer with room is required.
    if out.is_null() && out_len > 0 {
        return Status::Null;
    }
    if !or_return!(unsafe { room_for(coverage.len, !out.is_null(), out_len, out_count) }) {
        return Status::Ok;
    }
    let Some(out) = (unsafe { slice_mut_of(out, coverage.len) }) else { return Status::Null };
    let mut slots = out.iter_mut();
    coverage.runs(|first, last| slots.by_ref().zip(first..last + 1).for_each(|(slot, glyph)| *slot = glyph));
    Status::Ok
}

// A handle over a copy of the caller's bytes, kept for the thread's next with its copy's room, and its free: a new
// box and copy took each parse 30 to 57 ns over Rust's, which borrows the bytes.
macro_rules! kept_bytes {
    ($free:ident, $handle:ident, $bytes:ident) => {
        impl Reused for $handle {
            fn empty() -> $handle {
                $handle::default()
            }

            fn emptied(&mut self) -> usize {
                self.$bytes.clear();
                crate::daecore::daekit::base::sync::heap_of(&self.$bytes)
            }

            fn spare() -> &'static std::thread::LocalKey<core::cell::Cell<Option<alloc::boxed::Box<$handle>>>> {
                std::thread_local! {
                    static SPARE: core::cell::Cell<Option<alloc::boxed::Box<$handle>>> =
                        const { core::cell::Cell::new(None) };
                }
                &SPARE
            }
        }

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $free(handle: *mut $handle) {
            unsafe { give_back(handle) }
        }
    };
}

// A handle from the thread's spare, handed out where its bytes read, else given back with the failure.
unsafe fn handed<T: Reused>(out: *mut *mut T, handle: alloc::boxed::Box<T>, read: Result<(), Status>) -> Status {
    match read {
        Ok(()) => {
            unsafe { *out = alloc::boxed::Box::into_raw(handle) };
            Status::Ok
        }
        Err(status) => {
            keep_spare(handle);
            status
        }
    }
}

// AAT lookups, state tables and ankr

#[derive(Default)]
pub struct AatLookup {
    data: Vec<u8>,
    num_glyphs: u32,
}

impl AatLookup {
    fn view(&self) -> Result<format::Lookup<'_>, Status> {
        format::Lookup::parse(&self.data, self.num_glyphs).map_err(failed)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_lookup_parse(
    data: *const u8,
    len: usize,
    num_glyphs: u32,
    out: *mut *mut AatLookup,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    let mut lookup = reused::<AatLookup>();
    lookup.data.extend_from_slice(data);
    lookup.num_glyphs = num_glyphs;
    let read = lookup.view().map(drop);
    unsafe { handed(out, lookup, read) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_lookup_value(lookup: *const AatLookup, glyph: u32, out: *mut u16) -> Status {
    let Some(lookup) = (unsafe { borrow(lookup) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let view = or_return!(lookup.view());
    unsafe { put_some(out, view.value(glyph)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_lookup_entries(
    lookup: *const AatLookup,
    out: *mut GlyphValue,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(lookup) = (unsafe { borrow(lookup) }) else { return Status::Null };
    if out_count.is_null() {
        return Status::Null;
    }
    let view = or_return!(lookup.view());
    unsafe { into_buffer(view.entries().into_iter(), out, out_len, out_count) }
}

kept_bytes!(daegun_aat_lookup_free, AatLookup, data);

#[derive(Default)]
pub struct AatStateTable {
    data: Vec<u8>,
    extra_words: usize,
    num_glyphs: u32,
}

impl AatStateTable {
    fn view(&self) -> Result<format::StateTable<'_>, Status> {
        format::StateTable::parse(&self.data, self.extra_words, self.num_glyphs).map_err(failed)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_state_table_parse(
    data: *const u8,
    len: usize,
    extra_words: usize,
    num_glyphs: u32,
    out: *mut *mut AatStateTable,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    let mut table = reused::<AatStateTable>();
    table.data.extend_from_slice(data);
    (table.extra_words, table.num_glyphs) = (extra_words, num_glyphs);
    let read = table.view().map(drop);
    unsafe { handed(out, table, read) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_state_table_class(
    table: *const AatStateTable,
    glyph: u32,
    out: *mut u16,
) -> Status {
    let Some(table) = (unsafe { borrow(table) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let view = or_return!(table.view());
    unsafe { put(out, view.class(glyph)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_aat_state_table_entry(
    table: *const AatStateTable,
    state: u16,
    class_id: u16,
    out: *mut Entry,
) -> Status {
    let Some(table) = (unsafe { borrow(table) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let view = or_return!(table.view());
    unsafe { put_some(out, view.entry(state, class_id)) }
}

kept_bytes!(daegun_aat_state_table_free, AatStateTable, data);

// A pair answered through two outs, both required.
unsafe fn put_point(out_x: *mut i16, out_y: *mut i16, point: format::AnchorPoint) -> Status {
    unsafe {
        *out_x = point.x;
        *out_y = point.y;
    }
    Status::Ok
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_version(data: *const u8, len: usize, out: *mut u16) -> Status {
    if out.is_null() {
        return Status::Null;
    }
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    match format::Ankr::version(data) {
        Some(v) => unsafe { put(out, v) },
        None => invalid(format_args!("an ankr version is 2 bytes, and there are {len}")),
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
    match format::control_point(data, at) {
        Some(point) => unsafe { put_point(out_x, out_y, point) },
        None => invalid(format_args!("a control point is 4 bytes at offset {at}, past the {len} there are")),
    }
}

#[derive(Default)]
pub struct Ankr {
    data: Vec<u8>,
    num_glyphs: u32,
}

impl Ankr {
    fn view(&self) -> Result<format::Ankr<'_>, Status> {
        format::Ankr::parse(&self.data, self.num_glyphs).map_err(failed)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_parse(
    data: *const u8,
    len: usize,
    num_glyphs: u32,
    out: *mut *mut Ankr,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    let mut ankr = reused::<Ankr>();
    ankr.data.extend_from_slice(data);
    ankr.num_glyphs = num_glyphs;
    let read = ankr.view().map(drop);
    unsafe { handed(out, ankr, read) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_point_count(ankr: *const Ankr, glyph: u32, out: *mut u32) -> Status {
    let Some(ankr) = (unsafe { borrow(ankr) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let view = or_return!(ankr.view());
    unsafe { put(out, view.point_count(glyph)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ankr_anchor_point(
    ankr: *const Ankr,
    glyph: u32,
    index: u16,
    out_x: *mut i16,
    out_y: *mut i16,
) -> Status {
    let Some(ankr) = (unsafe { borrow(ankr) }) else { return Status::Null };
    if out_x.is_null() || out_y.is_null() {
        return Status::Null;
    }
    let view = or_return!(ankr.view());
    match view.anchor_point(glyph, index) {
        Some(point) => unsafe { put_point(out_x, out_y, point) },
        None => Status::Absent,
    }
}

kept_bytes!(daegun_ankr_free, Ankr, data);

// item variation stores

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_parse(
    data: *const u8,
    len: usize,
    base: usize,
    out: *mut *mut ItemVariationStore,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    unsafe { deliver_result(out, format::ItemVariationStore::parse(data, base)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_axis_count(ivs: *const ItemVariationStore, out: *mut usize) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    unsafe { put(out, ivs.axis_count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_region_count(ivs: *const ItemVariationStore, out: *mut usize) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    unsafe { put(out, ivs.regions.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_region(
    ivs: *const ItemVariationStore,
    region: usize,
    out_count: *mut usize,
) -> *const RegionAxis {
    unsafe { view(ivs, out_count, |s| s.regions.get(region).map(Vec::as_slice)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_ivd_count(ivs: *const ItemVariationStore, out: *mut usize) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    unsafe { put(out, ivs.ivd_data.len()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_ivd_rows(ivs: *const ItemVariationStore, ivd: usize, out: *mut usize) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    if out.is_null() {
        return Status::Null;
    }
    let Some(data) = ivs.ivd_data.get(ivd) else { return past_end("subtable", ivd, ivs.ivd_data.len()) };
    unsafe { put(out, data.rows()) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_ivd_region_indices(
    ivs: *const ItemVariationStore,
    ivd: usize,
    out_count: *mut usize,
) -> *const usize {
    unsafe { view(ivs, out_count, |s| s.ivd_data.get(ivd).map(|d| d.region_indices.as_slice())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_ivd_row(
    ivs: *const ItemVariationStore,
    ivd: usize,
    row: usize,
    out_count: *mut usize,
) -> *const i32 {
    unsafe { view(ivs, out_count, |s| s.ivd_data.get(ivd).and_then(|d| d.row(row))) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_region_scalars(
    ivs: *const ItemVariationStore,
    location: *const f64,
    location_len: usize,
    out: *mut f64,
    out_len: usize,
    out_count: *mut usize,
) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    let Some(location) = (unsafe { slice_of(location, location_len) }) else { return Status::Null };
    or_return!(apart(&[span(location.as_ptr(), location.len()), span(out, out_len), span(out_count, 1)]));
    let scalars = ivs.region_scalars(location);
    unsafe { into_buffer(scalars.into_iter(), out, out_len, out_count) }
}

// An index past u32 names no row, as u32::MAX doesn't, so both answer 0.
fn saturated(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_ivs_delta(
    ivs: *const ItemVariationStore,
    outer: usize,
    inner: usize,
    scalars: *const f64,
    scalars_len: usize,
    out: *mut f64,
) -> Status {
    let Some(ivs) = (unsafe { borrow(ivs) }) else { return Status::Null };
    let Some(scalars) = (unsafe { slice_of(scalars, scalars_len) }) else { return Status::Null };
    let index = format::DeltaSetIndex { outer: saturated(outer), inner: saturated(inner) };
    unsafe { put(out, ivs.delta(index, scalars)) }
}

free!(daegun_ivs_free, ItemVariationStore);

pub struct DeltaSetIndexMap(Vec<format::DeltaSetIndex>);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_delta_set_index_map_parse(
    data: *const u8,
    len: usize,
    base: usize,
    out: *mut *mut DeltaSetIndexMap,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(data) = (unsafe { slice_of(data, len) }) else { return Status::Null };
    unsafe { deliver_result(out, format::parse_delta_set_index_map(data, base).map(DeltaSetIndexMap)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_delta_set_index_map_count(map: *const DeltaSetIndexMap, out: *mut usize) -> Status {
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
    let found = format::delta_set_index_map_lookup(&map.0, index);
    unsafe {
        *out_outer = found.outer as usize;
        *out_inner = found.inner as usize;
    }
    Status::Ok
}

free!(daegun_delta_set_index_map_free, DeltaSetIndexMap);

// FeatureVariations

#[derive(Default)]
pub struct FeatureVariations {
    layout: Vec<u8>,
    at: usize,
}

impl FeatureVariations {
    fn table(&self) -> format::FeatureVariations<'_> {
        format::FeatureVariations::at(&self.layout, self.at)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_variations_parse(
    layout: *const u8,
    len: usize,
    out: *mut *mut FeatureVariations,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(layout) = (unsafe { slice_of(layout, len) }) else { return Status::Null };
    let Some(at) = format::FeatureVariations::parse(layout).map(|v| v.offset()) else { return Status::Absent };
    unsafe { kept_layout(out, layout, at) }
}

unsafe fn kept_layout(out: *mut *mut FeatureVariations, layout: &[u8], at: usize) -> Status {
    let mut variations = reused::<FeatureVariations>();
    variations.layout.extend_from_slice(layout);
    variations.at = at;
    unsafe { handed(out, variations, Ok(())) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_variations_at(
    layout: *const u8,
    len: usize,
    at: usize,
    out: *mut *mut FeatureVariations,
) -> Status {
    let Some(out) = (unsafe { cleared(out) }) else { return Status::Null };
    let Some(layout) = (unsafe { slice_of(layout, len) }) else { return Status::Null };
    unsafe { kept_layout(out, layout, at) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_variations_find(
    variations: *const FeatureVariations,
    coords: *const i32,
    coords_len: usize,
    store: *const ItemVariationStore,
    out: *mut u32,
) -> Status {
    let Some(variations) = (unsafe { borrow(variations) }) else { return Status::Null };
    let Some(coords) = (unsafe { slice_of(coords, coords_len) }) else { return Status::Null };
    let store = unsafe { borrow(store) };
    unsafe { put_some(out, variations.table().find(coords, store)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn daegun_feature_variations_substitute(
    variations: *const FeatureVariations,
    variation: u32,
    feature: u16,
    out: *mut usize,
) -> Status {
    let Some(variations) = (unsafe { borrow(variations) }) else { return Status::Null };
    let substituted = variations.table().substitute(variation, feature).map(|s| match s {
        format::Substitution::Feature(at) => at,
        format::Substitution::Empty => 0,
    });
    unsafe { put_some(out, substituted) }
}

kept_bytes!(daegun_feature_variations_free, FeatureVariations, layout);

pub(crate) fn release_thread() {
    crate::ffi::handle::drop_spare::<AatLookup>();
    crate::ffi::handle::drop_spare::<AatStateTable>();
    crate::ffi::handle::drop_spare::<Ankr>();
    crate::ffi::handle::drop_spare::<FeatureVariations>();
}

#[cfg(test)]
#[path = "tests/raw.rs"]
mod tests;
