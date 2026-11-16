use super::handle::Status;
use super::testing::{CFont, asset, last_error};
use crate::Orientation;
use crate::unicode::BaseDirection;
use core::ptr::{null, null_mut};

// Each reason is written over the last: a shorter one leaves nothing of a longer behind it, each ends in the NUL C
// reads to, and a NUL inside one reads as U+FFFD, as C would read the reason as ending there.
#[test]
fn a_reason_replaces_the_last_whole() {
    super::set_error("a reason longer than the next");
    super::set_error("short");
    assert_eq!(last_error(), "short");
    let reason = super::daegun_last_error();
    assert_eq!(unsafe { *reason.data.add(reason.len) }, 0, "the reason ends in a NUL");
    super::set_error(format_args!("a{}b", '\0'));
    assert_eq!(last_error(), "a\u{fffd}b");
}

// A _default writes every byte of the caller's struct, those between fields as zero, whatever the struct held.
#[test]
fn a_default_writes_every_byte_of_the_struct() {
    fn each<T: super::handle::Options>(name: &str) {
        let given = |byte: u8| {
            let mut room = core::mem::MaybeUninit::<T>::uninit();
            unsafe {
                room.as_mut_ptr().cast::<u8>().write_bytes(byte, size_of::<T>());
                assert_eq!(super::handle::defaults_into(room.as_mut_ptr(), size_of::<T>()), Status::Ok, "{name}");
                core::slice::from_raw_parts(room.as_ptr().cast::<u8>(), size_of::<T>()).to_vec()
            }
        };
        assert_eq!(given(0xAA), given(0x55), "{name}'s _default left bytes as it found them");
    }
    each::<super::shape::ShapeOptionsC>("shape");
    each::<super::outline::OutlineOptionsC>("outline");
    each::<super::layout::LayoutOptionsC>("layout");
    each::<super::layout::JustifyOptionsC>("justify");
    each::<super::color::SceneOptionsC>("scene");

    // A struct grown past 2.0, its new field defaulting to 1: a 2.0 caller's smaller struct gets its own bytes, the
    // byte between fields zero, and nothing written past its end.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Grown {
        size: usize,
        old: u8,
        new: f64,
    }
    const GIVEN: usize = size_of::<usize>() + 1;
    impl super::handle::Options for Grown {
        const FIRST: usize = GIVEN;
        unsafe fn set_defaults(p: *mut Self) {
            unsafe { (&raw mut (*p).new).write(1.0) };
        }
    }
    each::<Grown>("grown");
    let mut room = [u64::from_ne_bytes([0xAA; 8]); 3];
    assert_eq!(unsafe { super::handle::defaults_into(room.as_mut_ptr().cast::<Grown>(), GIVEN) }, Status::Ok);
    let bytes: alloc::vec::Vec<u8> = room.iter().flat_map(|w| w.to_ne_bytes()).collect();
    assert_eq!(bytes[..GIVEN], [GIVEN.to_ne_bytes().as_slice(), &[0]].concat(), "a 2.0 caller's struct");
    assert!(bytes[GIVEN..].iter().all(|&b| b == 0xAA), "written past the {GIVEN} bytes given: {bytes:?}");
}

// Every failure leaves its reason, so a caller can take daegun_last_error at its word: each call here finds a stale
// reason in the slot and must replace it.
#[test]
fn every_kind_of_failure_gives_a_fresh_reason() {
    fn fresh(name: &str, got: Status, want: Status) {
        assert_eq!(got, want, "{name}");
        let reason = last_error();
        assert!(!reason.is_empty() && reason != "stale", "{name} left {reason:?}");
        super::set_error("stale");
    }
    super::set_error("stale");
    let inter = CFont::of(asset("inter/InterVariable.ttf"));
    let stix = CFont::of(asset("stix-two-math/STIX2Math.otf"));
    let font = inter.handle.cast_const();
    let junk = [0xffu8; 16];
    unsafe {
        let mut opened = null_mut();
        let st = super::font::daegun_font_open(junk.as_ptr(), junk.len(), &mut opened);
        fresh("a font that doesn't parse", st, Status::Malformed);
        let mut lookup = null_mut();
        let st = super::raw::daegun_aat_lookup_parse(junk.as_ptr(), 1, 10, &mut lookup);
        fresh("a lookup too short to name its format", st, Status::Malformed);
        let st = super::raw::daegun_aat_lookup_parse(junk.as_ptr(), junk.len(), 10, &mut lookup);
        fresh("a lookup in a format no reader reads", st, Status::Unsupported);

        let mut gid = 0;
        fresh("a surrogate", super::glyphs::daegun_font_glyph_id(font, 0xD800, &mut gid), Status::InvalidArgument);
        let mut run = null_mut();
        let st = super::shape::daegun_font_shape(font, b"a\xff".as_ptr().cast(), 2, null(), null(), &mut run);
        fresh("text that isn't UTF-8", st, Status::InvalidArgument);
        let mut lm = core::mem::MaybeUninit::uninit();
        let st = super::metrics::daegun_font_line_metrics(font, null(), 2, lm.as_mut_ptr());
        fresh("a code naming no constant", st, Status::InvalidArgument);
        let mut kern = 0.0;
        let st = super::math::daegun_font_math_kern(stix.handle, 0, core::ptr::null(), 0, f64::NAN, &mut kern);
        fresh("a height that isn't a number", st, Status::InvalidArgument);
        let (mut bounds, mut n) = ([0usize; 1], 0);
        let st = super::unicode::daegun_grapheme_boundaries(c"abc".as_ptr(), 3, bounds.as_mut_ptr(), 1, &mut n);
        fresh("a buffer too small", st, Status::InvalidArgument);
        assert_eq!(n, 4, "a buffer too small still says what it needs");

        let mut stat = null_mut();
        assert_eq!(super::metrics::daegun_font_stat(font, null(), &mut stat), Status::Ok);
        let mut name = super::handle::Str::EMPTY;
        let st = super::metrics::daegun_stat_axis_name(stat, 99, &mut name);
        fresh("an index past the end", st, Status::InvalidArgument);
        super::metrics::daegun_stat_free(stat);

        // A struct of 1 byte is below any 2.0 struct, and one 8 bytes larger is from a header newer than this.
        let mut room = [0u64; 32];
        let opts = room.as_mut_ptr().cast();
        let st = super::shape::daegun_shape_options_default(opts, 1);
        fresh("an options struct too small", st, Status::InvalidArgument);
        let size = size_of::<super::shape::ShapeOptionsC>() + 8;
        let st = super::shape::daegun_shape_options_default(opts, size);
        fresh("an options struct too large", st, Status::Unsupported);
    }
}

// Each status base.h defines is named by its own name, and a code it doesn't define, as -5 retired, is "unknown".
#[test]
fn every_status_names_itself() {
    let base = include_str!("../../include/daegun/base.h");
    let statuses_block = base.split("typedef int32_t daegun_status;").nth(1);
    let block = statuses_block.and_then(|b| b.split("/* borrowed views */").next()).expect("base.h's statuses");
    let mut statuses = 0;
    for line in block.lines() {
        let mut words = line.split_whitespace();
        let (Some("#define"), Some(name), Some(code)) = (words.next(), words.next(), words.next()) else { continue };
        let code = code.parse::<i32>().expect(name);
        let named = super::daegun_status_name(code);
        let bytes = unsafe { core::slice::from_raw_parts(named.data.cast::<u8>(), named.len) };
        assert_eq!(bytes, name.as_bytes(), "{code}");
        statuses += 1;
    }
    assert_eq!(statuses, 10);
    for unknown in [-5, -11, 1, i32::MIN, i32::MAX] {
        let named = super::daegun_status_name(unknown);
        assert_eq!(unsafe { core::slice::from_raw_parts(named.data.cast::<u8>(), named.len) }, b"unknown");
    }
}

#[test]
fn the_version_is_the_crates_with_its_nul() {
    let v = super::daegun_version();
    let with_nul = unsafe { core::slice::from_raw_parts(v.data.cast::<u8>(), v.len + 1) };
    assert_eq!(with_nul, [env!("CARGO_PKG_VERSION").as_bytes(), b"\0"].concat());
}

// Every variant C can ask for reads back from its own code, and a code naming none is refused. Each `match` names
// every variant, so one that Rust gains does not build until C has a code for it.
#[test]
fn every_orientation_and_base_direction_has_a_code() {
    for v in [Orientation::Horizontal, Orientation::Vertical] {
        let code = match v {
            Orientation::Horizontal => super::HORIZONTAL,
            Orientation::Vertical => super::VERTICAL,
        };
        assert_eq!(super::orientation(code), Ok(v));
    }
    for v in [BaseDirection::Auto, BaseDirection::LeftToRight, BaseDirection::RightToLeft] {
        let code = match v {
            BaseDirection::Auto => super::BASE_AUTO,
            BaseDirection::LeftToRight => super::BASE_LTR,
            BaseDirection::RightToLeft => super::BASE_RTL,
        };
        assert_eq!(super::base_direction(code), Ok(v));
    }
    assert_eq!(super::orientation(2), Err(Status::InvalidArgument));
    assert_eq!(super::base_direction(2), Err(Status::InvalidArgument));
}
