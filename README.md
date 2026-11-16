# daegun

<img src="https://raw.githubusercontent.com/zztaeha/daegun/images-v1/assets/images/Logo/daegun.png" align="right" width="80" alt="">

**Text and shaping engine. Rust, `no_std`, zero dependencies**

[Documentation](https://zztaeha.dev/daegun) · [MIT, its data under its own licenses](#license) · Rust 1.99 or newer ·
Rust API and C ABI

## What it is

daegun turns a font file and a string into positioned glyphs, and those into pixels or the outlines
to draw them with. It parses TrueType and OpenType, shapes complex scripts, breaks and lays out lines,
hints, and subsets a font down to the glyphs a page's text can reach. The pixels are daegun's
rasterizer's, or yours: whatever you draw with instead, on the CPU or the GPU, daegun hands it what it
needs. A C ABI sits alongside the Rust API rather than behind it, so that holds from C too.

Each script is set as it's written: Arabic joins and runs right to left, Devanagari draws a vowel before the
consonant it was typed after, Khmer stacks subscripts under the base, and Japanese sets top to bottom.

There are no dependencies at all, so the supply chain is the Rust compiler and nothing else. The
parsers, the shaper and the rasterizer forbid unsafe code outright, enforced by the compiler rather than by
discipline: a font file is data from strangers, and daegun treats it that way.

## What it supports

| | |
|---|---|
| **Outlines** | TrueType `glyf`, CFF, CFF2, VARC's variable composites, and variable fonts on any number of axes; fonts past 65,535 glyphs, through the tables that name them in 24 bits |
| **Color** | COLR v0 and v1 paint graphs, CPAL palettes, and `sbix`, `CBDT` and `EBDT` bitmap strikes |
| **Shaping** | GSUB and GPOS in full, Apple `morx` and `kerx`, features over a whole text or a range of it, and shapers for Arabic and Syriac, Hebrew, Thai and Lao, Hangul, Khmer, Myanmar, nine Indic scripts and 91 more through the Universal Shaping Engine |
| **Text** | Unicode 18.0.0: bidi, line breaking greedy or optimal, with Thai, Lao, Khmer and Myanmar broken between words, justification, vertical writing, math |
| **Rasterizing** | Glyphs and paths to coverage, exact where contours overlap and the same bytes on every CPU; a display's stripes, darkened stems and sRGB alpha; COLR glyphs composited in linear light; SDF and MSDF fields; a cache of what's drawn |
| **For your rasterizer** | Outlines in pixels with hinting, transforms, stroking and synthetic bold and oblique; quadratic curves for GPU shaders; flattened polygons with overlaps resolved; color glyphs as scenes, with gradient sampling and compositing; subpixel filters |
| **And** | Subsetting to the glyphs a text can reach, TrueType and CFF hinting, an autohinter, collections, and caches held to byte budgets, which a group of fonts can share |
| **File formats** | TrueType and OpenType fonts and collections. Decompress WOFF and WOFF2 first. Formats that predate OpenType, such as Type 1, BDF and PCF, are refused |

## Installing

Rust:

```toml
[dependencies]
daegun = "2.0.0"
```

The timings in [benchmark.md](benchmark.md) are built with `lto = true` and `codegen-units = 1`, which a dependency
can't set for you: put them in your own `[profile.release]` where shaping or drawing is the hot path.

Thai, Lao, Khmer and Myanmar are written without spaces between words, so a line can break between them only
where a word list finds them. The `dictionaries` feature builds in lists for all four, about 1 MB. Without it, pass
your own segmenter in `LayoutOptions::segmenter`, or such text breaks only at spaces, and between graphemes where a
word won't fit the line. The lists come under their own licenses, whose notices are in
`src/daecore/src/daeshaper/generated/word_lists.rs`; a binary built with them carries those notices.

C: each release's archives, at [github.com/zztaeha/daegun/releases](https://github.com/zztaeha/daegun/releases), hold
the headers, the static library and, where the platform takes one, the shared one, a `daegun.pc`, CMake's package
files and the license notices, for macOS, iOS, Mac Catalyst, tvOS, visionOS, Linux with glibc or musl, Android and
Windows, beside a universal macOS archive and an XCFramework for Xcode and SwiftPM. With an archive's folder in
`CMAKE_PREFIX_PATH`, CMake finds it with `find_package(daegun 2.0)` and links `daegun::daegun`.

Or install the library, its headers and a `daegun.pc` from source with [cargo-c](https://github.com/lu-zero/cargo-c)
(`cargo install --locked cargo-c`, which on Linux needs git, curl, pkg-config and OpenSSL's headers, `libssl-dev` on
Debian, or `--features vendored-openssl` for the last), which builds the `capi` feature. Install as yourself into a
folder, then copy it into place as root, as `sudo cargo` wouldn't find a rustup of your own:

```sh
cargo cinstall --release --prefix /usr/local --destdir stage
sudo cp -R stage/usr/local/. /usr/local/
```

On Linux, run `sudo ldconfig` after installing, so the loader finds the shared library. Fedora and its relatives look
in neither `/usr/local/lib` for libraries nor `/usr/local/lib/pkgconfig` for `.pc` files: name the first in a file
under `/etc/ld.so.conf.d` before running `ldconfig`, and the second in `PKG_CONFIG_PATH`.

A build system then finds daegun through pkg-config. The `.pc` carries the C ABI's version, so `>=2.0` and `<3`
take this ABI or a later minor, never a major whose calls differ:

```cmake
find_package(PkgConfig REQUIRED)
pkg_check_modules(DAEGUN REQUIRED IMPORTED_TARGET "daegun>=2.0" "daegun<3")
target_link_libraries(app PRIVATE PkgConfig::DAEGUN)
```

```meson
executable('app', 'app.c', dependencies: dependency('daegun', version: ['>=2.0', '<3']))
```

Or `cc app.c $(pkg-config --cflags --libs daegun)`.

Or build the library in the shape you want. The ABI is behind the `capi` feature, which implies `threading`.
`--features capi,dictionaries`, or `--features dictionaries` to `cargo cinstall`, builds in the word lists.

```sh
cargo rustc --release --features capi --crate-type staticlib  # libdaegun.a, daegun.lib
# Windows: daegun.dll with the VC runtime linked in, so it needs only the UCRT, part of Windows since 10
cargo rustc --release --features capi --crate-type cdylib -- -C target-feature=+crt-static \
  -C link-arg=/NODEFAULTLIB:libucrt.lib -C link-arg=/DEFAULTLIB:ucrt.lib
```

A shared library on macOS or Linux carries the ABI's major in the name a program loads it by, 2 for this release,
so a program never loads one whose calls differ from those it was built against. Install it under that name:

```sh
# macOS: libdaegun.dylib, installed as libdaegun.2.dylib on the program's rpath, versioned as cargo-c versions it
cargo rustc --release --features capi --crate-type cdylib -- -C link-arg=-Wl,-install_name,@rpath/libdaegun.2.dylib \
  -C link-arg=-Wl,-current_version,2.0.0 -C link-arg=-Wl,-compatibility_version,2 \
  -C link-arg=-Wl,-headerpad_max_install_names
# Linux: libdaegun.so, installed as libdaegun.so.2, with libdaegun.so a link to it for the linker
cargo rustc --release --features capi --crate-type cdylib -- -C link-arg=-Wl,-soname,libdaegun.so.2
```

Then compile with `-I src/api/c/include` and include `daegun.h`. A static library needs the system libraries
Rust's standard library links against, and a shared one carries its own:

| | |
|---|---|
| macOS, iOS | `cc app.c libdaegun.a` |
| Linux | `cc app.c libdaegun.a -lgcc_s -lutil -lrt -lpthread -lm -ldl` |
| Linux, musl | `cc app.c libdaegun.a` |
| Android | `cc app.c libdaegun.a -llog -lm` |
| Windows | `cl /std:c11 /MD app.c daegun.lib kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib` |
| MinGW | `gcc app.c libdaegun.a -lkernel32 -lntdll -luserenv -lws2_32 -ldbghelp` |

A program calling part of the API drops the rest of a static library at the link with `-Wl,-dead_strip` on macOS
and iOS, or `-Wl,--gc-sections` on Linux: one calling 29 of the 349 comes out at under half the size.

On Windows the DLL carries the same major, as `daegun-2.dll`, and a program linking it defines `DAEGUN_DLL` and links
its import library instead: `cl /std:c11 /MD /DDAEGUN_DLL app.c daegun-2.dll.lib`. MinGW links the same DLL the same
way, `gcc -DDAEGUN_DLL app.c daegun-2.dll.lib`, or a static library built with `--target x86_64-pc-windows-gnu`.
Building the DLL yourself, rename `daegun.dll` to `daegun-2.dll`, and make the import library, as cargo's names the
old name: a `daegun-2.def` holding `LIBRARY daegun-2.dll`, `EXPORTS` and each name `dumpbin /exports` lists, then
`lib /def:daegun-2.def /machine:x64 /out:daegun-2.dll.lib`.

Every Rust static library carries its own copy of Rust's standard library, so a program linking daegun's beside
another stops at the link on the symbols both define, `rust_eh_personality` among them. Link one of them shared, or
merge daegun's archive into one object that exports its calls alone:

```sh
# macOS
cc -r -nostdlib -o daegun.o -Wl,-all_load libdaegun.a -Wl,-exported_symbol,'_daegun_*'
# Linux
ld -r --whole-archive libdaegun.a -o daegun.o && objcopy --wildcard --keep-global-symbol='daegun_*' daegun.o
# either: the object alone as the archive
rm libdaegun.a && ar rcs libdaegun.a daegun.o
```

On Windows, link `daegun-2.dll`: neither Microsoft's tools nor LLVM's hide a symbol in an MSVC object.

A shared library of yours that links `libdaegun.a` exports daegun's calls as its own unless it hides them, and on
Linux two such libraries in one process, built against different releases, would then call each other's daegun.
Hide them with `-Wl,--exclude-libs,libdaegun.a` on Linux, Android and MinGW, or `-Wl,-hidden-ldaegun` on macOS in
place of `-ldaegun`.

C++ includes `daegun.h` as it is and links the same way, with `c++` in place of `cc`. No call throws, as GCC, clang
and MSVC's C++ are told, so C++ around one needs no cleanup; a callback you hand daegun must not throw either, so
declare it `noexcept`. A handle fits a `std::unique_ptr` whose deleter calls its `_free`, a stateless one keeping the
owner a pointer wide, and C++23's `std::out_ptr` hands the owner to a call that writes the handle, as
`src/api/c/tests/handles.cpp` shows.

A freed handle is kept for the thread's next one of its kind, so a loop making and freeing one allocates nothing,
and a sanitizer can't see a handle used after its free. Checking a program under ASan or Valgrind, build daegun
with `RUSTFLAGS="--cfg daegun_no_spares"`, which frees each at once.

MemorySanitizer reads every value a library it didn't instrument writes as uninitialized, so on Linux build daegun,
and Rust's standard library with it, instrumented, which takes nightly:

```sh
RUSTFLAGS=-Zsanitizer=memory cargo +nightly rustc -Zbuild-std --features capi --crate-type staticlib \
  --target x86_64-unknown-linux-gnu
```

Then link with clang's `-fsanitize=memory`. On Arm, put `$(clang -print-libgcc-file-name)` before the library: MSan
starts by calling an atomic that Rust's instrumented copy would answer before MSan's memory is mapped. In a
container, MSan runs a program again without address randomization, which takes `--security-opt
seccomp=unconfined` under Docker.

## The Rust API

Open a font, shape a string, and hand each glyph to your rasterizer. That is the whole loop.

```rust
use daegun::outline::{OutlineOptions, Path};
use daegun::shape::ShapeOptions;
use daegun::{Font, Location};

let bytes = std::fs::read("Inter.ttf")?;
let font = Font::from_bytes(&bytes)?;

let run = font.shape("Wave", &Location::default(), &ShapeOptions::DEFAULT);
// glyphs [459, 507, 980, 614], advances [1914.0, 1120.0, 1111.0, 1194.0]

let mut outline = Path::default();
font.prepared_outline(run.glyphs[0], &Location::default(), 32.0, &OutlineOptions::default(), &mut outline)?;
// one contour, 15 lines and 16 quadratics, about 30 by 23 pixels, y up from the baseline
```

`prepared_outline` draws onto any `OutlinePen`, so a rasterizer of your own implements the trait and
takes the calls directly, or reads them back out of a `Path` as here.

`from_bytes` copies the file. `from_vec` keeps a `Vec` as it is given, and `from_static` reads bytes that
live as long as the program where they lie; both open a collection's faces by index, copying nothing.

Every number is in the font's own units, `f64`, y up and unrounded: advances, metrics, boxes and
outlines alike, so a pen position is `advance * px / f64::from(font.units_per_em())`. Only the calls
that take a pixel size, `prepared_outline` and `hinted_outline`, answer in pixels.

A variable font's axes are resolved once into a `Location`, which every call that depends on one
takes by reference. `Location::default()` is the default instance of any font:

```rust
use daegun::Tag;

let bold = font.location(&[(Tag::new(b"wght"), 700.0), (Tag::new(b"opsz"), 28.0)]);
let run = font.shape("Wave", &bold, &ShapeOptions::DEFAULT);
font.prepared_outline(gid, &bold, 32.0, &OutlineOptions::default(), &mut outline)?;
```

A `Feature` parses from a string as CSS and shaping tools write one: `"-liga"` turns ligatures off, and
`"kern[3:5]=0"` turns kerning off over bytes `3..5` of the text alone. `ShapeOptions::with_features` takes them for a
run, and `LayoutOptions::with_features` for a paragraph.

`OutlineOptions` carries the rest, and is a builder so you name only what you change:

```rust
use daegun::outline::{HintMode, OutlineOptions};

let opts = OutlineOptions::default().with_hinting(HintMode::Auto).with_embolden(40.0);
let glyph = font.prepared_outline(gid, &Location::default(), 32.0, &opts, &mut outline)?;
// glyph.advance_width includes the embolden, and glyph.hinted says a hinter ran
```

Also on it: `with_transform` for an affine, `with_stroke`, `with_oblique`. A stroke or an embolden
overlaps itself, which a non-zero fill hides and a rasterizer that sums coverage does not, and
`has_overlap_flag` says which glyphs a font flags as overlapping on their own.

A GPU rasterizer that evaluates curves in a shader wants quadratics only, and one that walks straight
edges wants polygons:

```rust
use daegun::outline::{max_area_for, resolve_overlaps};

let curves = font.quads(gid, &Location::default())?;
// 32 curves, each a Quad { start, control, end } in font units, wound clockwise

let mut units = Path::default();
font.outline(gid, &Location::default(), &mut units)?;
let polygons = units.flatten(max_area_for(32.0, f32::from(font.units_per_em())))
    .expect("a real glyph stays under MAX_FLATTEN_POINTS");
let merged = resolve_overlaps(&polygons);
// Some for this W, whose one contour crosses itself where its strokes meet; None for an H
```

`daegun::raster::SubpixelLayout` describes a display's stripes, for a rasterizer that filters
subpixels itself: how many samples a pixel takes, the filter's taps and weights, and how far a glyph's
box has to grow to fit them.

Laying out a paragraph wraps, breaks and positions in one call, in font units too, so divide by
`px / upm` going in and multiply coming out:

```rust
use daegun::layout::{Align, BreakStrategy, LayoutOptions};

let scale = 16.0 / f64::from(font.units_per_em());
let options = LayoutOptions::default()
    .with_max_inline_size(240.0 / scale)
    .with_line_height(20.0 / scale)
    .with_strategy(BreakStrategy::Optimal)
    .with_align(Align::Start);
let layout = font.layout(text, &Location::default(), &options);

for line in &layout.lines {
    for run in &line.runs {
        // run.run.glyphs, run.run.advances, run.offset
    }
}
```

`BreakStrategy::Greedy` breaks at the last opportunity that fits; `Optimal` searches the paragraph.

Subsetting takes the text rather than glyph ids. It keeps every glyph those characters can reach, so
ligatures, joining forms, components and color layers survive, and pins the font to a location:

```rust
let subset = font.subset_text("Type is the voice of the page.", &Location::default())?;
std::fs::write("subset.ttf", &subset.font)?;
// 879,708 bytes to 9,824
```

The result carries a real `cmap`, so it loads straight into `@font-face`. `Font::subset` takes glyph
ids instead when you already know them; its `cmap` keeps the source's characters for the glyphs it
keeps.

A color glyph comes back as a scene rather than one outline, since it has more than one color in it:

```rust
use daegun::color::SceneOptions;

let scene = font.color_scene(gid, &Location::default(), &SceneOptions::default())?
    .expect("is a color glyph");
for op in scene.ops() {
    // color::Op::Fill { path, paint, rule, transform }, and clips and layers pushed and popped;
    // scene.path(path) is the outline a fill draws, in font units
}
```

`daegun::color` has the rest of what drawing it takes: `gradient::Ramp` samples a gradient as COLR
defines it, and `Blend::composite` lays a layer over the one below in any of COLR's 28 modes.

A call that can fail returns `daegun::Result`, whose `Error` has a `kind()` to act on and a
`message()` naming the table or step. An `Option` means the font has none of something.

That is the shape of it. The reference at [zztaeha.dev/daegun](https://zztaeha.dev/daegun/reference/rust-methods/)
gives each method what it returns, what it does when it cannot, and the units it answers in.

## The C API

C can do everything the Rust API does, bar a handful of helpers that would be pointless there.
`src/api/c/include/daegun.h` is the contract: it includes a header an area, as the Rust modules are, and
`daegun/base.h` states five rules that hold for every call. A fallible call returns `daegun_status` and answers
through an out parameter, NULL where a pointer is required returns `DAEGUN_NULL` and is never dereferenced,
daegun allocates and daegun frees, a borrowed view lasts until the handle it came from is freed or changed, and a
handle a call only reads may be shared by threads while one a call changes may not, and any handle may be freed
on any thread. Every failure has a status of its kind, as Rust's `ErrorKind` names them, its reason in
`daegun_last_error()` and its name in `daegun_status_name()`. Each pointer that may be NULL is marked
`DAEGUN_NULLABLE`, so clang warns of a NULL where one is required, and each array carries the argument counting
it, so GCC, MSVC's `/analyze` and clang's bounds safety warn where yours is shorter than you say.

The same loop:

```c
#include "daegun.h"

daegun_font *font = NULL;
if (daegun_font_open(data, len, &font) != DAEGUN_OK) return 1;

daegun_shaped_run *run = NULL;
if (daegun_font_shape(font, "Wave", 4, NULL, NULL, &run) != DAEGUN_OK) return 1;

size_t n = 0;
const uint32_t *gids = daegun_shaped_run_glyphs(run, &n);
const double *advances = daegun_shaped_run_advances(run, &n); /* in font units */

/* your rasterizer's callbacks, and the state they draw into */
daegun_pen pen = { on_move, on_line, on_quad, on_cubic, on_close, &my_rasterizer };
daegun_prepared_glyph glyph;
if (daegun_font_prepared_outline(font, gids[0], NULL, 32.0f, NULL, &pen, &glyph) != DAEGUN_OK) return 1;
/* the callbacks got the outline in pixels, and glyph.advance_width is in pixels too */

daegun_shaped_run_free(run);
daegun_font_free(font);
```

A location is resolved once into a handle, and every call that depends on one takes it, NULL being the
default instance:

```c
const daegun_coord coords[] = {
    { DAEGUN_TAG('w', 'g', 'h', 't'), 700.0 },
    { DAEGUN_TAG('o', 'p', 's', 'z'), 28.0 },
};
daegun_location *bold = NULL;
if (daegun_font_location(font, coords, 2, &bold) != DAEGUN_OK) return 1;
if (daegun_font_shape(font, "Wave", 4, bold, NULL, &run) != DAEGUN_OK) return 1;
daegun_location_free(bold);
```

An options struct opens with its size, which its `_default` fills, so a newer library still reads an older
caller's:

```c
daegun_shape_options opts;
if (daegun_shape_options_default(&opts, sizeof opts) != DAEGUN_OK) return 1;
opts.language = "tr";
if (daegun_font_shape(font, "fit", 3, NULL, &opts, &run) != DAEGUN_OK) return 1;
```

A list of fixed-size items, such as tags, axes or glyph ids, fills your buffer, so one kept between calls
means none allocates. A NULL buffer and a length of 0 ask for the count alone, and one too small answers
`DAEGUN_INVALID_ARGUMENT` and still sets the count:

```c
size_t count = 0;
if (daegun_font_table_tags(font, NULL, 0, &count) != DAEGUN_OK) return 1;
daegun_tag *tags = malloc(count * sizeof *tags);
if (daegun_font_table_tags(font, tags, count, &count) != DAEGUN_OK) return 1;
```

Subsetting mirrors the Rust call, and the bytes are borrowed until the subset is freed:

```c
const char *text = "Type is the voice of the page.";
daegun_subset *subset = NULL;
if (daegun_font_subset_text(font, text, strlen(text), NULL, &subset) != DAEGUN_OK) return 1;

size_t ttf_len = 0;
const uint8_t *ttf = daegun_subset_bytes(subset, &ttf_len);
fwrite(ttf, 1, ttf_len, out);
daegun_subset_free(subset);
```

`daegun_font_prepared_outline` takes a `daegun_outline_options`, and passing NULL for it means the
defaults, so you need not build the struct to get them. Quadratic curves, flattening and overlap
resolution, color scenes with their gradients and compositing, and subpixel filters all have their C
forms; each area's header says what its calls borrow.

The reference at [zztaeha.dev/daegun](https://zztaeha.dev/daegun/reference/c-functions/) says which pointers each
call borrows and how long each stays valid.

## Rasterizing

A `Rasterizer` draws a glyph into an image it lends, placed by its `left` and `top` from the glyph's origin on the
baseline. It keeps its buffers between calls, so once warm it allocates nothing. Coverage is the exact area under
the outline, exact too where contours overlap, and every CPU gets the same bytes. A run, set into a canvas a byte a
pixel:

```rust
use daegun::raster::{RasterOptions, Rasterizer};

let (px, baseline) = (32.0, 30);
let scale = f64::from(px) / f64::from(font.units_per_em());
let run = font.shape("Wave", &Location::default(), &ShapeOptions::DEFAULT);
let (width, height) = (96, 40);
let mut canvas = vec![0u8; width * height];

let mut rasterizer = Rasterizer::new();
let mut pen = 2.0;
for ((&gid, advance), offset) in run.glyphs.iter().zip(&run.advances).zip(&run.offsets) {
    // whole pixels place the image, and the fraction moves the glyph within it
    let (x, y) = (pen + offset.x * scale, offset.y * scale);
    let options = RasterOptions::DEFAULT.with_offset(x.fract() as f32, y.fract() as f32);
    let image = rasterizer.glyph(&font, gid, &Location::default(), px, &options)?;
    let (left, top) = (x as i32 + image.left, baseline - y as i32 - image.top);
    for row in 0..image.height {
        let start = (top + row as i32) as usize * width + left as usize;
        for (dst, &src) in canvas[start..].iter_mut().zip(image.row(row)) {
            *dst = dst.saturating_add(src);
        }
    }
    pen += advance * scale;
}
// "Wave" inked across 82 by 25 pixels, each byte the share of its pixel the glyphs cover
```

`RasterOptions` is a builder, as `OutlineOptions` is, and carries one: `with_layout` draws a display's stripes,
three bytes a pixel, `with_gamma` gives the alpha a compositor blending in sRGB needs, and `with_darken` widens the
thin stems of small text. `color_glyph` draws a COLR glyph as premultiplied RGBA, `distance_field` a glyph's SDF or
MSDF, and `path` any `Path`. `glyph_into`, `color_glyph_into` and `path_into` draw into pixels of yours instead, a
`Target` naming which rectangle of the drawing they hold, so a glyph of any size draws a band of rows at a time,
without an image of its own. Give `Cache::Raster` a budget and a font keeps what's drawn of it, lent again in
place of drawing; a `CachePool` holds a group of fonts to one budget.

From C, each image lent until the rasterizer's next call:

```c
daegun_shaped_run *run = NULL;
if (daegun_font_shape(font, "Wave", 4, NULL, NULL, &run) != DAEGUN_OK) return 1;
size_t n = 0;
const uint32_t *gids = daegun_shaped_run_glyphs(run, &n);
const double *advances = daegun_shaped_run_advances(run, &n);
const daegun_offset *offsets = daegun_shaped_run_offsets(run, &n);
uint16_t upem = 0;
if (daegun_font_units_per_em(font, &upem) != DAEGUN_OK) return 1;
const double scale = 32.0 / upem;
const int baseline = 30;
static uint8_t canvas[40][96];

daegun_rasterizer *rasterizer = daegun_rasterizer_new();
daegun_raster_options opts;
if (daegun_raster_options_default(&opts, sizeof opts) != DAEGUN_OK) return 1;
double pen = 2.0;
for (size_t i = 0; i < n; i++) {
    /* whole pixels place the image, and the fraction moves the glyph within it */
    double x = pen + offsets[i].x * scale, y = offsets[i].y * scale;
    opts.offset[0] = (float)(x - (int)x);
    opts.offset[1] = (float)(y - (int)y);
    daegun_image image;
    if (daegun_rasterizer_glyph(rasterizer, font, gids[i], NULL, 32.0f, &opts, &image) != DAEGUN_OK) return 1;
    int left = (int)x + image.left, top = baseline - (int)y - image.top;
    for (uint32_t row = 0; row < image.height; row++) {
        for (uint32_t col = 0; col < image.width; col++) {
            unsigned sum = canvas[top + (int)row][left + (int)col] + image.pixels[row * image.stride + col];
            canvas[top + (int)row][left + (int)col] = sum > 255 ? 255 : (uint8_t)sum;
        }
    }
    pen += advances[i] * scale;
}
daegun_rasterizer_free(rasterizer);
daegun_shaped_run_free(run);
```

## Subsetting

![Four texts subset with subset_text. A Japanese sentence in Source Han Sans JP goes from 8.0 MB to
10.0 KB, 99.9% removed. Below it, Latin body text in Inter goes from 859 KB to 14.9 KB, an Arabic
sentence in Scheherazade New from 324 KB to 13.9 KB, and mathematical notation in STIX Two Math from
789 KB to 11.1 KB.](https://raw.githubusercontent.com/zztaeha/daegun/images-v3/assets/images/01-subsetting.png)

A page of Japanese wanting 37 characters ships 10.0 KB rather than 8.0 MB. Every glyph the text can
reach survives, joining forms and ligatures included, and the rest goes.

## Untrusted input

![Tallies of 12 inputs, 8 refused, 4 opened and 0 crashes, above a table of the twelve damaged
fonts: empty, one byte, header only, truncated at 1, 50 and 99 percent, wrong magic, 65,535 tables,
offsets past the end of the file, lengths of 4 GB, 4,096 flipped bytes, and random noise, each with
its size and whether it was refused or opened.](https://raw.githubusercontent.com/zztaeha/daegun/images-v3/assets/images/02-untrusted-input.png)

Each of those is handed to every public entry point that will take it. Eight are refused with an
error. The other four open leniently, skipping what is missing, and are then driven through every
public `Font` call. None of them crashes or panics.

## What holds it up

| | |
|---|---|
| Dependencies | None at all, so the supply chain is the Rust compiler and nothing else. |
| Unsafe code | Forbidden in the whole engine, every parser, the shaper and the rasterizer included. The compiler enforces it: an inner `allow` is a hard error rather than a warning. Only the C layer opts back in, and it says so once, at the top of `src/api/c/translation/mod.rs`. |
| C ABI | Every call in the Rust API is reachable from C bar a handful that would be pointless there, the header matches the library's own symbol table in both directions, and every constant in it matches the Rust value it copies. The gate below fails if any of that stops being true. |
| Compilers | Clang, GCC, MSVC and MinGW: the headers compile clean alone and together as C11 to C23 and C++11 to C++23, and as the drafts after them, C2y and C++26, under Clang and GCC, and on i386. Under Wine, MSVC builds them as its default C, C11 to the latest C and C++14 to the latest C++, and runs the round trip, and MinGW's GCC runs it against the static library and the DLL both. The headers are plain ASCII, and MSVC builds them at code page 949, where one byte outside ASCII would stop it. |
| Hostile input | 2,000 mutated fonts on every gate run, seeded from the repository's fonts and macOS's, each one reproducible from its seed, and 500 more on 32-bit x86. The inputs past runs caught are kept and replayed. The C ABI is fuzzed too: 20,000 runs under libFuzzer, both sides under AddressSanitizer. |
| Sanitizers | The C round trip runs under AddressSanitizer and UndefinedBehaviorSanitizer, then again with the Rust side instrumented too, so a read past a caller's array is caught where it happens. It runs under MemorySanitizer too, Rust's standard library instrumented as well, and a C program sharing one font across 8 threads runs under ThreadSanitizer, both sides instrumented, beside a planted race it must report. The library's own tests run under AddressSanitizer, its threading tests under ThreadSanitizer, and Miri checks the C layer, each pointer it hands C and that nothing leaks, and the threading code across many schedules. |
| Same pixels everywhere | Every kind of image the rasterizer draws is the same bytes on aarch64 macOS, with `std` and without, i686 and x86_64 Linux, and x86_64 Windows. |
| Targets | The tests run on 32-bit x86 too, overflow checks on. Without `std`, daegun builds for Cortex-M (`thumbv7em-none-eabi`), and `wasm32-unknown-unknown` builds with `std` and without; a target lacking atomics at pointer width is told so. |
| Panics | `unwrap` and `expect` are linted against outside tests, and the C library is built with `panic = "abort"`, so it never unwinds into a C caller. |

One command runs all of it, along with the tests, clippy, rustdoc and the minimum supported Rust version. It needs
Rust 1.99.0 and a nightly with Miri, Xcode with its iOS Simulator, GCC, a clang on nightly's LLVM, Docker,
cargo-zigbuild with zig, MinGW, and MSVC under Wine, and names the install for any that is missing:

```sh
sh scripts/tools/perf/gate.sh
```

[zztaeha.dev/daegun](https://zztaeha.dev/daegun) is the reference for the Rust and C APIs, so this file can stay a
tour. [SECURITY.md](SECURITY.md) sets out the threat model and how to report a vulnerability.

## License

daegun's own code is MIT: see [LICENSE](LICENSE). Its Unicode tables are built from the Unicode Character Database,
and its language tags and the languages a font records names in with CLDR's likely subtags, under the Unicode License
v3; its Universal Shaping Engine data from Microsoft's, under the MIT license. Their notices are in
[CREDITS.txt](CREDITS.txt), so software shipping daegun ships that file too, as the release archives and
`cargo cinstall` do. A C library built from daegun also carries Rust's standard library, whose notices the release
archives hold in `share/doc/daegun` as `LICENSE-rust-std.html`; with `cargo cinstall`, ship the toolchain's
`share/doc/rust/COPYRIGHT-library.html` beside it. The `dictionaries` feature's word lists come under Unicode's
licenses, ICU's older one among them, and, for the Lao and Myanmar lists, BSD licenses; CREDITS.txt holds their
notices too, and thanks those whose work daegun was built and tested with. As one SPDX expression:
`MIT AND Unicode-3.0 AND ICU AND BSD-2-Clause AND BSD-3-Clause`.
