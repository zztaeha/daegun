// What the translation's tests share: an asset's bytes, a font opened through C and freed through C when dropped,
// and the last reason as a string.
use alloc::string::String;
use alloc::vec::Vec;

pub(crate) fn asset(path: &str) -> Vec<u8> {
    std::fs::read(alloc::format!("{}/assets/test-fonts/{path}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

pub(crate) struct CFont {
    pub(crate) bytes: Vec<u8>,
    pub(crate) handle: *mut crate::Font,
}

impl CFont {
    pub(crate) fn of(bytes: Vec<u8>) -> CFont {
        let mut handle = core::ptr::null_mut();
        let st = unsafe { super::font::daegun_font_open(bytes.as_ptr(), bytes.len(), &mut handle) };
        assert_eq!(st, super::handle::Status::Ok);
        CFont { bytes, handle }
    }
}

impl Drop for CFont {
    fn drop(&mut self) {
        unsafe { super::font::daegun_font_free(self.handle) }
    }
}

pub(crate) fn last_error() -> String {
    let reason = super::daegun_last_error();
    let reason = unsafe { core::slice::from_raw_parts(reason.data.cast::<u8>(), reason.len) };
    String::from(core::str::from_utf8(reason).unwrap())
}

// The corpus the C-against-Rust checks walk, each area's an ignored test: the test fonts, each a font or else a
// collection's first face.
pub(crate) fn corpus() -> Vec<std::path::PathBuf> {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| ["ttf", "otf", "ttc", "otc"].contains(&e.to_ascii_lowercase().as_str()))
            {
                out.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut fonts = Vec::new();
    walk(&root.join("assets/test-fonts"), &mut fonts);
    fonts.sort();
    fonts
}

// A font of the corpus opened both ways, and where it is read: the default, each axis at its minimum, its
// maximum and three tenths between alone, and every axis at once at each.
pub(crate) struct Opened {
    pub(crate) name: String,
    pub(crate) c: *mut crate::Font,
    pub(crate) rust: crate::Font,
}

impl Drop for Opened {
    fn drop(&mut self) {
        unsafe { super::font::daegun_font_free(self.c) }
    }
}

pub(crate) struct Place {
    pub(crate) label: String,
    pub(crate) c: *mut super::font::LocationHandle,
    pub(crate) rust: crate::Location,
    // Every axis set, so the area's costlier checks run here and at the default alone.
    pub(crate) whole: bool,
}

impl Drop for Place {
    fn drop(&mut self) {
        unsafe { super::font::daegun_location_free(self.c) }
    }
}

impl Opened {
    pub(crate) fn places(&self) -> Vec<Place> {
        let axes = self.rust.axes();
        let place = |label: String, coords: Vec<(crate::Tag, f64)>, whole| {
            let c_coords: Vec<super::font::Coord> =
                coords.iter().map(|&(tag, value)| super::font::Coord { tag: tag.0, value }).collect();
            let mut c = core::ptr::null_mut();
            let st = unsafe { super::font::daegun_font_location(self.c, c_coords.as_ptr(), c_coords.len(), &mut c) };
            assert_eq!(st, super::handle::Status::Ok, "{}: a location", self.name);
            Place { label, c, rust: self.rust.location(&coords), whole }
        };
        let mut places = vec![place(String::from("default"), Vec::new(), true)];
        for t in [0.0, 1.0, 0.3] {
            let at = |a: &crate::Axis| (a.tag, a.min + (a.max - a.min) * t);
            for a in &axes {
                places.push(place(alloc::format!("{}={t}", a.tag), vec![at(a)], false));
            }
            if !axes.is_empty() {
                places.push(place(alloc::format!("all={t}"), axes.iter().map(at).collect(), true));
            }
        }
        places
    }
}

// Each font of the corpus that opens both ways, its open checked first: C's status against Rust's kind.
pub(crate) fn each_font(diffs: &mut Diffs, mut check: impl FnMut(&Opened, &mut Diffs)) {
    for path in corpus() {
        let name = path.strip_prefix(env!("CARGO_MANIFEST_DIR")).unwrap_or(&path);
        let name = alloc::format!("{}", name.display());
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let mut c = core::ptr::null_mut();
        let mut st = unsafe { super::font::daegun_font_open(bytes.as_ptr(), bytes.len(), &mut c) };
        let mut rust = crate::Font::from_bytes(&bytes);
        if st != super::handle::Status::Ok {
            st = unsafe { super::font::daegun_font_open_collection(bytes.as_ptr(), bytes.len(), 0, &mut c) };
            rust = crate::Font::from_collection(&bytes, 0);
        }
        diffs.eq(&name, "open", st, status(&rust));
        match rust {
            Ok(rust) if !c.is_null() => check(&Opened { name, c, rust }, diffs),
            _ => unsafe { super::font::daegun_font_free(c) },
        }
    }
}

// What C answers for a Rust result: OK, or the status its kind maps to.
pub(crate) fn status<T>(r: &crate::Result<T>) -> super::handle::Status {
    use super::handle::Status;
    use crate::ErrorKind;
    match r {
        Ok(_) => Status::Ok,
        Err(e) => match e.kind() {
            ErrorKind::Malformed => Status::Malformed,
            ErrorKind::Unsupported => Status::Unsupported,
            ErrorKind::Missing => Status::Missing,
            ErrorKind::InvalidArgument => Status::InvalidArgument,
            ErrorKind::LimitExceeded => Status::LimitExceeded,
            ErrorKind::NoOutline => Status::NoOutline,
            ErrorKind::Overflow => Status::Overflow,
        },
    }
}

// What C answers for a Rust option: OK, or ABSENT for none.
pub(crate) fn found<T>(o: &Option<T>) -> super::handle::Status {
    if o.is_some() { super::handle::Status::Ok } else { super::handle::Status::Absent }
}

// A float as its bits, so equal means the same value, signed zeros and NaNs told apart.
pub(crate) fn bits(v: f64) -> u64 {
    v.to_bits()
}

pub(crate) fn bits32(v: f32) -> u32 {
    v.to_bits()
}

// A string as C gives it: an interior NUL replaced, as a C string can't hold one.
pub(crate) fn c_text(s: &str) -> String {
    s.replace('\0', "\u{fffd}")
}

// A borrowed C view as a slice, empty for NULL.
pub(crate) unsafe fn lent<'a, T>(data: *const T, len: usize) -> &'a [T] {
    if data.is_null() || len == 0 { &[] } else { unsafe { core::slice::from_raw_parts(data, len) } }
}

pub(crate) unsafe fn str_of(s: super::handle::Str) -> String {
    String::from_utf8_lossy(unsafe { lent(s.data.cast::<u8>(), s.len) }).into_owned()
}

// Every difference, up to fifty told in full, and how many checks ran.
#[derive(Default)]
pub(crate) struct Diffs {
    fails: Vec<String>,
    differ: usize,
    checks: usize,
}

impl Diffs {
    pub(crate) fn eq<T: PartialEq + core::fmt::Debug>(&mut self, at: &str, what: &str, c: T, rust: T) {
        self.checks += 1;
        if c != rust {
            self.differ += 1;
            if self.fails.len() < 50 {
                self.fails.push(alloc::format!("{at}: {what}: C {c:?}, Rust {rust:?}"));
            }
        }
    }

    pub(crate) fn done(self, area: &str) {
        assert!(
            self.differ == 0,
            "{area}: {} of {} checks differ:\n{}",
            self.differ,
            self.checks,
            self.fails.join("\n")
        );
        std::eprintln!("{area}: C answers as Rust in {} checks", self.checks);
        assert!(self.checks > 0, "{area}: nothing checked");
    }
}

// C's owned results read through their own calls, then freed through C.
pub(crate) unsafe fn take_text(t: *mut super::base::Text) -> Option<String> {
    if t.is_null() {
        return None;
    }
    let mut s = super::handle::Str::EMPTY;
    let st = unsafe { super::base::daegun_text_str(t, &mut s) };
    assert!(st != super::handle::Status::Ok || unsafe { *s.data.add(s.len) } == 0, "a text without its NUL");
    let text = (st == super::handle::Status::Ok).then(|| unsafe { str_of(s) });
    unsafe { super::base::daegun_text_free(t) };
    text
}

pub(crate) unsafe fn take_strs(l: *mut super::base::StrList) -> Vec<String> {
    let mut n = 0;
    let _ = unsafe { super::base::daegun_str_list_count(l, &mut n) };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let mut s = super::handle::Str::EMPTY;
        let _ = unsafe { super::base::daegun_str_list_at(l, i, &mut s) };
        out.push(unsafe { str_of(s) });
    }
    unsafe { super::base::daegun_str_list_free(l) };
    out
}

// Each name with its ID, each string ending at a NUL of its own.
pub(crate) unsafe fn take_names(l: *mut super::font::Names) -> Vec<(u16, String)> {
    let mut n = 0;
    let _ = unsafe { super::font::daegun_names_count(l, &mut n) };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (mut id, mut s) = (0, super::handle::Str::EMPTY);
        let _ = unsafe { super::font::daegun_names_at(l, i, &mut id, &mut s) };
        assert_eq!(unsafe { *s.data.add(s.len) }, 0, "name {id} has no NUL after it");
        out.push((id, unsafe { str_of(s) }));
    }
    unsafe { super::font::daegun_names_free(l) };
    out
}

pub(crate) unsafe fn take_localized_names(l: *mut super::font::LocalizedNames) -> Vec<(String, String)> {
    let mut n = 0;
    let _ = unsafe { super::font::daegun_localized_names_count(l, &mut n) };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let (mut language, mut s) = (super::handle::Str::EMPTY, super::handle::Str::EMPTY);
        let _ = unsafe { super::font::daegun_localized_names_at(l, i, &mut language, &mut s) };
        assert_eq!(unsafe { (*language.data.add(language.len), *s.data.add(s.len)) }, (0, 0), "no NUL after {i}");
        out.push((unsafe { str_of(language) }, unsafe { str_of(s) }));
    }
    unsafe { super::font::daegun_localized_names_free(l) };
    out
}

macro_rules! take_list {
    ($name:ident, $list:ty, $elem:ty, $data:ident, $free:ident) => {
        pub(crate) unsafe fn $name(l: *mut $list) -> Vec<$elem> {
            let mut n = 0;
            let items = unsafe { lent(super::base::$data(l, &mut n), n) }.to_vec();
            unsafe { super::base::$free(l) };
            items
        }
    };
}
take_list!(take_blob, super::base::Blob, u8, daegun_blob_data, daegun_blob_free);

// A result into the caller's buffers as BUFFERS says: its count asked for with none and no room, a buffer one short
// refused with the count still set, then filled into room for exactly it. The status, and none where it failed.
pub(crate) unsafe fn filled2<A, B>(
    mut call: impl FnMut(*mut A, *mut B, usize, &mut usize) -> super::handle::Status,
) -> (super::handle::Status, Vec<A>, Vec<B>) {
    use super::handle::Status;
    let mut count = usize::MAX;
    let st = call(core::ptr::null_mut(), core::ptr::null_mut(), 0, &mut count);
    if st != Status::Ok {
        return (st, Vec::new(), Vec::new());
    }
    let (mut a, mut b) = (Vec::<A>::with_capacity(count), Vec::<B>::with_capacity(count));
    if let Some(short) = count.checked_sub(1) {
        let mut again = usize::MAX;
        let st = call(a.as_mut_ptr(), b.as_mut_ptr(), short, &mut again);
        assert_eq!((st, again), (Status::InvalidArgument, count), "a buffer one short wasn't refused");
    }
    let mut again = usize::MAX;
    let st = call(a.as_mut_ptr(), b.as_mut_ptr(), count, &mut again);
    assert_eq!(again, count, "the count moved between two calls");
    if st == Status::Ok {
        unsafe { (a.set_len(count), b.set_len(count)) };
    }
    (st, a, b)
}

// `filled2` over one buffer.
pub(crate) unsafe fn filled<T>(
    mut call: impl FnMut(*mut T, usize, &mut usize) -> super::handle::Status,
) -> (super::handle::Status, Vec<T>) {
    let (st, items, _) = unsafe { filled2::<T, u8>(|out, _, len, count| call(out, len, count)) };
    (st, items)
}

// The glyphs a check walks: the first 300, the last 40 and two past the end.
pub(crate) fn glyph_list(font: &crate::Font) -> Vec<u32> {
    let count = font.glyph_count();
    let mut gids: Vec<u32> = (0..count.saturating_add(2).min(300)).collect();
    gids.extend((count.saturating_sub(40).max(300)..count.saturating_add(2)).filter(|g| *g >= 300));
    gids
}

// Text in several scripts, and a sweep of code points with a surrogate and two past Unicode among them.
pub(crate) const SAMPLE: &str = "Hello, world! fi ffi \u{644}\u{627} \u{633}\u{644}\u{627}\u{645} \
    \u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947} \u{6f22}\u{5b57} \u{1f600} e\u{301}\0x";
pub(crate) const PROBES: [u32; 15] =
    [0, 0x20, 0x41, 0x61, 0x627, 0x928, 0x3002, 0x4e00, 0xd800, 0xdfff, 0xfffd, 0x1f600, 0x10ffff, 0x110000, u32::MAX];

// A shaped run, all it holds with each float as its bits, read from C's views or from Rust's run.
pub(crate) type RunSig = (Vec<u32>, Vec<u64>, Vec<(u64, u64)>, Vec<u32>, [Vec<bool>; 3], u64, bool, bool, String);

pub(crate) unsafe fn c_run(run: *const crate::shape::ShapedRun) -> Option<RunSig> {
    use super::shape::*;
    if run.is_null() {
        return None;
    }
    let mut n = 0;
    let glyphs = unsafe { lent(daegun_shaped_run_glyphs(run, &mut n), n) }.iter().map(|g| g.0).collect();
    let advances = unsafe { lent(daegun_shaped_run_advances(run, &mut n), n) }.iter().map(|&a| bits(a)).collect();
    let offsets =
        unsafe { lent(daegun_shaped_run_offsets(run, &mut n), n) }.iter().map(|o| (bits(o.x), bits(o.y))).collect();
    let clusters = unsafe { lent(daegun_shaped_run_clusters(run, &mut n), n) }.to_vec();
    let flags = |view: unsafe extern "C" fn(*const crate::shape::ShapedRun, *mut usize) -> *const bool| {
        let mut n = 0;
        unsafe { lent(view(run, &mut n), n) }.to_vec()
    };
    let flags = [
        flags(daegun_shaped_run_unsafe_to_break),
        flags(daegun_shaped_run_unsafe_to_concat),
        flags(daegun_shaped_run_safe_to_insert_tatweel),
    ];
    let (mut width, mut complete, mut broken, mut shaper) = (0.0, false, false, -1);
    let mut name = super::handle::Str::EMPTY;
    unsafe {
        let _ = daegun_shaped_run_width(run, &mut width);
        let _ = daegun_shaped_run_complete(run, &mut complete);
        let _ = daegun_shaped_run_has_broken_syllable(run, &mut broken);
        let _ = daegun_shaped_run_shaper(run, &mut shaper);
        let _ = daegun_shaper_name(shaper, &mut name);
    }
    Some((glyphs, advances, offsets, clusters, flags, bits(width), complete, broken, unsafe { str_of(name) }))
}

pub(crate) fn rust_run(run: &crate::shape::ShapedRun) -> RunSig {
    (
        run.glyphs.iter().map(|g| g.0).collect(),
        run.advances.iter().map(|&a| bits(a)).collect(),
        run.offsets.iter().map(|o| (bits(o.x), bits(o.y))).collect(),
        run.clusters.clone(),
        [run.unsafe_to_break.clone(), run.unsafe_to_concat.clone(), run.safe_to_insert_tatweel.clone()],
        bits(run.width()),
        run.complete,
        run.has_broken_syllable,
        String::from(run.shaper.name()),
    )
}

// Texts in several scripts for the shaping, layout and justification checks.
pub(crate) const TEXTS: [&str; 6] = [
    "Hello, world! office affine AVATAR Te Ty",
    "\u{627}\u{644}\u{633}\u{644}\u{627}\u{645} \u{639}\u{644}\u{64a}\u{643}\u{645} 123 abc",
    "\u{928}\u{92e}\u{938}\u{94d}\u{924}\u{947} \u{915}\u{94d}\u{937}",
    "\u{6f22}\u{5b57}\u{304b}\u{306a} \u{ff08}\u{3002}\u{ff09} abc",
    "\u{e2a}\u{e27}\u{e31}\u{e2a}\u{e14}\u{e35}\u{e04}\u{e23}\u{e31}\u{e1a}",
    "e\u{301} a\u{200d}b \u{1f469}\u{200d}\u{1f4bb} \u{200f}!",
];

// A path's verbs and points, each coordinate as its bits.
pub(crate) type PathSig = (Vec<u8>, Vec<(u32, u32)>);

pub(crate) fn rust_path(path: &crate::outline::Path) -> PathSig {
    let (verbs, points) = path.parts();
    (verbs.iter().map(|&v| v as u8).collect(), points.iter().map(|p| (bits32(p.x), bits32(p.y))).collect())
}

pub(crate) unsafe fn c_path(path: *const crate::outline::Path) -> PathSig {
    let (mut verbs, mut points) = (0, 0);
    let v = unsafe { lent(super::outline::daegun_path_verbs(path, &mut verbs), verbs) }.to_vec();
    let p = unsafe { lent(super::outline::daegun_path_points(path, &mut points), points) };
    (v, p.iter().map(|p| (bits32(p.x), bits32(p.y))).collect())
}

// A new C path and the pen that draws into it.
pub(crate) unsafe fn c_pen() -> (*mut crate::outline::Path, super::outline::Pen) {
    let path = super::outline::daegun_path_new();
    let mut pen = core::mem::MaybeUninit::<super::outline::Pen>::zeroed();
    let _ = unsafe { super::outline::daegun_path_as_pen(path, pen.as_mut_ptr()) };
    (path, unsafe { pen.assume_init() })
}

// What a C path draws, the path then freed.
pub(crate) unsafe fn drawn(
    draw: impl FnOnce(&super::outline::Pen) -> super::handle::Status,
) -> (super::handle::Status, PathSig) {
    let (path, pen) = unsafe { c_pen() };
    let st = draw(&pen);
    let sig = unsafe { c_path(path) };
    unsafe { super::outline::daegun_path_free(path) };
    (st, sig)
}

// What a caller-buffer call answers: its status and count with room for one, then the items with room for all.
pub(crate) unsafe fn buffered<T: Copy>(
    call: impl Fn(*mut T, usize, *mut usize) -> super::handle::Status,
    room: usize,
) -> (super::handle::Status, usize, super::handle::Status, Vec<T>) {
    let (mut short, mut count) = (0, 0);
    let mut one = core::mem::MaybeUninit::<T>::zeroed();
    let st = call(one.as_mut_ptr(), 1, &mut short);
    let mut all = Vec::<T>::with_capacity(room);
    let full = call(all.as_mut_ptr(), room, &mut count);
    if full == super::handle::Status::Ok {
        unsafe { all.set_len(count) };
    }
    (st, short, full, all)
}

// The status and count a caller buffer of one answers for `n` items.
pub(crate) fn short_of(n: usize) -> (super::handle::Status, usize) {
    (if n > 1 { super::handle::Status::InvalidArgument } else { super::handle::Status::Ok }, n)
}
