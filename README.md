# daegun

<img src="https://raw.githubusercontent.com/silly-tae/daegun/images-v1/assets/images/Logo/daegun.png" align="right" width="80" alt="">
<a href="https://ko-fi.com/S1T425C33Y"><img src="https://ko-fi.com/img/githubbutton_sm.svg" align="right" alt="Support daegun on ko-fi"></a>

**Your all-in-one text engine. Rust, `no_std`, zero dependencies.**

[Documentation](https://dg.calia.cc) · MIT · Rust 1.97.1 or newer · Rust API and C ABI

## What it is

daegun turns a font file and a string into positioned, rasterized glyphs. It parses TrueType and
OpenType, shapes complex scripts, breaks and lays out lines, hints, rasterizes on the CPU or the GPU,
and subsets a font down to the glyphs a page actually uses. One crate does all of it, and a C ABI
sits alongside the Rust API rather than behind it.

There are no dependencies at all, so the supply chain is the Rust compiler and nothing else. The
parsers and the shaper forbid unsafe code outright, enforced by the compiler rather than by
discipline: a font file is data from strangers, and daegun treats it that way.

## Global scripts

![Four cards side by side, one each for Arabic, Devanagari, Khmer and Japanese. Every card shows a
word set large in its own script, the count of characters given against glyphs produced, and a line
naming what that script does differently.](https://raw.githubusercontent.com/silly-tae/daegun/images-v1/assets/images/02-global-scripts.png)

Arabic joins and runs right to left, Devanagari draws a vowel before the consonant it was typed
after, Khmer stacks subscripts under the base, and Japanese sets top to bottom. daegun drew all of
it, the cards and the words on them included.

## What it supports

| | |
|---|---|
| **Outlines** | TrueType `glyf`, CFF, CFF2, and variable fonts on any number of axes |
| **Color** | COLR v0 and v1 paint graphs, CPAL palettes, and `sbix` and `CBDT` bitmap strikes |
| **Shaping** | GSUB and GPOS in full, Apple `morx` and `kerx`, and shapers for Arabic, Hebrew, Thai, Hangul, Khmer, Myanmar, nine Indic scripts and 84 more through the Universal Shaping Engine |
| **Text** | Bidi, line breaking greedy or optimal, justification, vertical writing, math |
| **Rasterizing** | CPU everywhere, and GPU through Metal on Apple platforms, Direct3D 11 and 12 on Windows, and Vulkan anywhere |
| **And** | Subsetting to the glyphs a page uses, TrueType and CFF hinting, an autohinter, and collections |

## Installing

Rust:

```toml
[dependencies]
daegun = "1.1.7"
```

C: build the library in the shape you want. The ABI is behind the `capi` feature, which implies
`threading`.

```sh
cargo rustc --release --features capi --crate-type staticlib  # libdaegun.a, daegun.lib
cargo rustc --release --features capi --crate-type cdylib     # libdaegun.dylib, .so, .dll
```

Then compile against `src/c-wrapper/daegun.h`. A static library needs the platform frameworks the GPU
backends call into, and a shared one carries its own:

| | |
|---|---|
| macOS, iOS | `cc app.c libdaegun.a -framework Metal -framework Foundation -framework QuartzCore` |
| Linux | `cc app.c libdaegun.a -lm -lpthread -ldl` |
| Windows | `cl app.c daegun.lib ws2_32.lib userenv.lib ntdll.lib` |

Vulkan and Direct3D are opened by name at run time, so neither adds anything to link. A machine
without them answers `DAEGUN_UNSUPPORTED` rather than failing to load.

## The Rust API

Open a font, shape a string, rasterize what comes back. That is the whole loop.

```rust
use daegun::Font;

let bytes = std::fs::read("Inter.ttf")?;
let font = Font::from_bytes(&bytes)?;

let shaped = font.shape("Wave", &[], false).expect("shapes");
// glyphs [459, 507, 980, 614], advances [934.6, 546.9, 542.5, 583.0]

let glyph = font.rasterize_glyph(shaped.glyphs[0], 32.0, &[]).expect("has ink");
// 31x24 coverage bytes at (0, 0), one per pixel, advance 15.4px
```

Advances come back on a 1000 unit em whatever the font's own units are, so a pen position is
`advance * px / 1000.0`. Outlines are in the font's units instead, which `Font::upm` reports. That
difference is deliberate and it is the one thing worth reading twice.

Variable axes go to any call that takes them, as `(tag, value)`:

```rust
let bold = font.shape("Wave", &[("wght", 700.0)], false).expect("shapes");
let glyph = font.rasterize_glyph(gid, 32.0, &[("wght", 700.0), ("opsz", 28.0)]);
```

`RasterOptions` carries the rest, and is a builder so you name only what you change:

```rust
use daegun::{HintMode, RasterOptions};

let opts = RasterOptions::default().with_hinting(HintMode::Auto).with_gamma(1.8);
let glyph = font.rasterize_glyph_with(gid, 32.0, &[], &opts).expect("has ink");
```

Also on it: `with_layout` for subpixel stripes, `with_transform` for an affine, `with_stroke`,
`with_embolden`, `with_oblique`.

Laying out a paragraph wraps, breaks and positions in one call. Sizes are on the same 1000 unit em,
so divide by `px / 1000.0` going in and multiply coming out:

```rust
use daegun::{Align, BreakStrategy, LayoutOptions};

let px = 16.0;
let scale = px / 1000.0;
let layout = font.layout(text, &[], &LayoutOptions {
    max_inline_size: 240.0 / scale,
    line_height: Some(20.0 / scale),
    strategy: BreakStrategy::Optimal,
    align: Align::Start,
    ..LayoutOptions::default()
}).expect("lays out");

for line in &layout.lines {
    for run in &line.runs {
        // run.run.glyphs, run.run.advances, run.offset
    }
}
```

`BreakStrategy::Greedy` breaks at the last opportunity that fits; `Optimal` searches the paragraph.

Subsetting takes the text rather than glyph ids, so it shapes first and keeps what shaping produced:

```rust
let subset = font.subset_text("Type is the voice of the page.", &[])?;
std::fs::write("subset.ttf", &subset.ttf)?;
// 879,708 bytes to 15,800
```

The result carries a real `cmap`, so it loads straight into `@font-face`. `Font::subset` takes glyph
ids instead when you already know them.

A COLR glyph renders to a scene rather than a coverage map, since it has more than one color in it:

```rust
let scene = font.render_colr_glyph(gid, 48.0, &[], 0).expect("is a color glyph");
// scene.width, scene.height, scene.rgba: straight alpha RGBA8
```

The GPU path hands you data instead of drawing, because owning a device needs `unsafe` and the
engine does not:

```rust
use daegun::GpuBatch;

let mut batch = GpuBatch::new();
let slot = font.gpu_glyph(&mut batch, gid, &[])?;
// batch.curves(), batch.bands(), batch.band_curves(), batch.hulls() go to the GPU
// slot.instance(offset, scale, em_pixels, tint) makes one GlyphInstance
```

Shader source for GLSL, HLSL and MSL ships with the crate. Backends for Metal, Vulkan, D3D11 and
D3D12 live under `daegun::gpu` if you would rather daegun drove the device. Each one can adopt a
device you already made and draw straight into your swapchain surface, so a real-time app never pays
for a readback.

That is the shape of it. All 223 methods and 49 types are written up at
[dg.calia.cc](https://dg.calia.cc/reference/rust-methods/), each with what it returns, what it does
when it cannot, and the units it answers in.

## The C API

Everything the Rust API does, C does. `src/c-wrapper/daegun.h` is the contract, and it states five
rules that hold for every call: a fallible call returns `daegun_status` and answers through an out
parameter, NULL where a pointer is required returns `DAEGUN_NULL` and is never dereferenced, daegun
allocates and daegun frees, a borrowed view lasts until the handle it came from is freed, and handles
are thread safe.

The same loop:

```c
#include "daegun.h"

daegun_font *font = NULL;
if (daegun_font_open(data, len, &font) != DAEGUN_OK) return 1;

daegun_run *run = NULL;
daegun_font_shape(font, "Wave", NULL, 0, /* vertical */ false, &run);

size_t n = 0;
const uint16_t *gids = daegun_run_glyphs(run, &n);
const double *advances = daegun_run_advances(run, &n);

daegun_bitmap *bmp = NULL;
daegun_font_rasterize_glyph(font, gids[0], 32.0f, NULL, 0, &bmp);

daegun_metrics m;
daegun_bitmap_metrics(bmp, &m);
size_t pixels_len = 0;
const uint8_t *pixels = daegun_bitmap_pixels(bmp, &pixels_len);
/* m.width x m.height, one coverage byte each */

daegun_bitmap_free(bmp);
daegun_run_free(run);
daegun_font_free(font);
```

Axes are an array of `daegun_axis`, and every call that takes them takes a count beside them:

```c
daegun_axis axes[] = { { "wght", 700.0 }, { "opsz", 28.0 } };
daegun_font_shape(font, "Wave", axes, 2, false, &run);
```

Subsetting mirrors the Rust call, and the bytes are borrowed until the subset is freed:

```c
daegun_subset *subset = NULL;
daegun_font_subset_text(font, "Type is the voice of the page.", NULL, 0, &subset);

size_t ttf_len = 0;
const uint8_t *ttf = daegun_subset_ttf(subset, &ttf_len);
fwrite(ttf, 1, ttf_len, out);
daegun_subset_free(subset);
```

`daegun_font_rasterize_glyph_with` takes a `daegun_raster_options`, and passing NULL for it means the
defaults, so you need not build the struct to get them. Layout, color, the GPU batch and the four
backends all have their C forms; the header groups them and says what each borrows.

The 440 functions, 86 types and 217 constants are all documented at
[dg.calia.cc](https://dg.calia.cc/reference/c-functions/), including which pointers are borrowed and
how long each stays valid.

## Subsetting

![A table of four fonts. For each: the original file size, the size after subsetting to one sentence,
and the percentage removed. Inter 859 KB to 24.6 KB, Source Han Sans 8.0 MB to 12.2 KB, Scheherazade
New 324 KB to 21.8 KB, STIX Two Math 789 KB to 8.7 KB.](https://raw.githubusercontent.com/silly-tae/daegun/images-v1/assets/images/01-subsetting.png)

A page of Japanese wanting 37 characters ships 12.2 KB rather than 8.0 MB. Each row is shaped before
it is subset, so the glyphs that joining and ligatures actually produced survive and the rest goes.

## Untrusted input

![Twelve cards, one per damaged font: empty, a single byte, header only, truncated at 1, 50 and 99
percent, wrong magic, 65,535 tables, offsets past the end of the file, lengths of 4 GB, 4,096 flipped
bytes, and random noise. Eleven are marked refused and one parsed and survived. A tally above reads
12 inputs, 11 refused, 0 crashes.](https://raw.githubusercontent.com/silly-tae/daegun/images-v1/assets/images/03-robustness.png)

Each of those is handed to every public entry point that will take it. Eleven are refused with an
error, the twelfth parses and is then driven over anyway, and none of them crashes or panics.

## What holds it up

| | |
|---|---|
| Dependencies | None at all, so the supply chain is the Rust compiler and nothing else. |
| Unsafe code | Forbidden in every parser and in the shaper. The compiler enforces it: an inner `allow` is a hard error rather than a warning. Only the GPU backends and the C layer opt back in, and they say so at the top of their own files. |
| C ABI | Every call in the Rust API is reachable from C bar a handful that would be pointless there, and the header matches the library's own symbol table in both directions. The build fails if either stops being true. |
| Hostile input | 500 mutated fonts on every build, each one reproducible from its seed. |
| Sanitizers | The C round trip runs under AddressSanitizer and UndefinedBehaviorSanitizer. |
| Panics | `unwrap` and `expect` are linted against outside tests, and the library is built with `panic = "abort"`, so it never unwinds into a caller. |

One command runs all of it, along with the tests, clippy, the shader compilers, two
cross-compiled Windows targets, and the minimum supported Rust version:

```sh
sh scripts/tools/perf/gate.sh
```

[dg.calia.cc](https://dg.calia.cc) documents every name in both APIs, which is what keeps this file a
tour rather than a reference. [SECURITY.md](SECURITY.md) sets out the threat model and how to report
a vulnerability. [benchmark.md](benchmark.md) carries every latency benchmark in the tree, with the
machine and build they were measured on.

## License

MIT. See [LICENSE](LICENSE).
