# daegun

<img src="https://raw.githubusercontent.com/zztaeha/daegun/images-v1/assets/images/Logo/daegun.png" align="right" width="80" alt="">

**The text engine that hands your rasterizer everything it needs. Rust, `no_std`, zero dependencies.**

[Documentation](https://dg.calia.cc) · MIT · Rust 1.97.1 or newer · Rust API and C ABI

## What it is

daegun turns a font file and a string into positioned glyphs and the outlines to draw them with. It
parses TrueType and OpenType, shapes complex scripts, breaks and lays out lines, hints, and subsets a
font down to the glyphs a page's text can reach. The pixels are yours: whatever rasterizer you draw
with, on the CPU or the GPU, daegun hands it what it needs. A C ABI sits alongside the Rust API rather
than behind it, so that holds from C too.

There are no dependencies at all, so the supply chain is the Rust compiler and nothing else. The
parsers and the shaper forbid unsafe code outright, enforced by the compiler rather than by
discipline: a font file is data from strangers, and daegun treats it that way.

## Global scripts

![Four cards side by side, one each for Arabic, Devanagari, Khmer and Japanese. Every card shows a
word set large in its own script, the count of characters given against glyphs produced, and a line
naming what that script does differently.](https://raw.githubusercontent.com/zztaeha/daegun/images-v2/assets/images/02-global-scripts.png)

Arabic joins and runs right to left, Devanagari draws a vowel before the consonant it was typed
after, Khmer stacks subscripts under the base, and Japanese sets top to bottom. daegun shaped and
set every word on these cards, and wgpu drew them from the curves daegun handed it.

## What it supports

| | |
|---|---|
| **Outlines** | TrueType `glyf`, CFF, CFF2, and variable fonts on any number of axes |
| **Color** | COLR v0 and v1 paint graphs, CPAL palettes, and `sbix`, `CBDT` and `EBDT` bitmap strikes |
| **Shaping** | GSUB and GPOS in full, Apple `morx` and `kerx`, and shapers for Arabic, Hebrew, Thai, Hangul, Khmer, Myanmar, nine Indic scripts and 84 more through the Universal Shaping Engine |
| **Text** | Bidi, line breaking greedy or optimal, justification, vertical writing, math |
| **For your rasterizer** | Outlines in pixels with hinting, transforms, stroking and synthetic bold and oblique; quadratic curves for GPU shaders; flattened polygons with overlaps resolved; color glyphs as scenes, with gradient sampling and compositing; subpixel filters |
| **And** | Subsetting to the glyphs a text can reach, TrueType and CFF hinting, an autohinter, and collections |

## Installing

Rust:

```toml
[dependencies]
daegun = "1.2.0"
```

C: build the library in the shape you want. The ABI is behind the `capi` feature, which implies
`threading`.

```sh
cargo rustc --release --features capi --crate-type staticlib  # libdaegun.a, daegun.lib
cargo rustc --release --features capi --crate-type cdylib     # libdaegun.dylib, .so, daegun.dll
```

Then compile against `src/c-wrapper/daegun.h`. A static library needs the system libraries Rust's
standard library links against, and a shared one carries its own:

| | |
|---|---|
| macOS, iOS | `cc app.c libdaegun.a` |
| Linux | `cc app.c libdaegun.a -lgcc_s -lutil -lrt -lpthread -lm -ldl` |
| Windows | `cl app.c daegun.lib kernel32.lib ntdll.lib userenv.lib ws2_32.lib dbghelp.lib` |

## The Rust API

Open a font, shape a string, and hand each glyph to your rasterizer. That is the whole loop.

```rust
use daegun::{Font, OutlineOptions, Path};

let bytes = std::fs::read("Inter.ttf")?;
let font = Font::from_bytes(&bytes)?;

let shaped = font.shape("Wave", &[], false).expect("shapes");
// glyphs [459, 507, 980, 614], advances [934.6, 546.9, 542.5, 583.0]

let mut outline = Path::default();
font.prepared_outline(shaped.glyphs[0], 32.0, &[], &OutlineOptions::default(), &mut outline)
    .expect("is in the font");
// one contour, 15 lines and 16 quadratics, about 30 by 23 pixels, y up from the baseline
```

`prepared_outline` draws onto any `OutlinePen`, so a rasterizer of your own implements the trait and
takes the calls directly, or reads them back out of a `Path` as here.

The units differ on purpose, and they are the one thing worth reading twice. Shaped advances come
back on a 1000-unit em whatever the font's own units are, so a pen position is
`advance * px / 1000.0`. `outline_glyph` answers in the font's units, which `Font::upm` reports, and
`prepared_outline` in pixels.

Variable axes go to any call that takes them, as `(tag, value)`:

```rust
let axes = [("wght", 700.0), ("opsz", 28.0)];
let bold = font.shape("Wave", &axes, false).expect("shapes");
font.prepared_outline(gid, 32.0, &axes, &OutlineOptions::default(), &mut outline);
```

`OutlineOptions` carries the rest, and is a builder so you name only what you change:

```rust
use daegun::{HintMode, OutlineOptions};

let opts = OutlineOptions::default().with_hinting(HintMode::Auto).with_embolden(40.0);
let glyph = font.prepared_outline(gid, 32.0, &[], &opts, &mut outline).expect("is in the font");
// glyph.advance_width includes the embolden, and glyph.hinted says a hinter ran
```

Also on it: `with_transform` for an affine, `with_stroke`, `with_oblique`. A stroke or an embolden
overlaps itself, which a non-zero fill hides and a rasterizer that sums coverage does not.

A GPU rasterizer that evaluates curves in a shader wants quadratics only, and one that walks straight
edges wants polygons:

```rust
let curves = font.glyph_quads(gid, &[])?;
// 32 curves, each [start, control, end] in em units, wound clockwise

let mut units = Path::default();
font.outline_glyph(gid, &mut units).expect("is in the font");
let polygons = daegun::flatten(&units, daegun::max_area_for(32.0, f32::from(font.upm())))
    .expect("a real glyph stays under MAX_FLATTEN_POINTS");
let merged = daegun::resolve_overlaps(&polygons);
// Some for this W, whose one contour crosses itself where its strokes meet; None for an H
```

`SubpixelLayout` describes a display's stripes, for a rasterizer that filters subpixels itself: how
many samples a pixel takes, the filter's taps and weights, and how far a glyph's box has to grow to
fit them.

Laying out a paragraph wraps, breaks and positions in one call. Sizes are on the same 1000-unit em,
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

Subsetting takes the text rather than glyph ids. It keeps every glyph those characters can reach, so
ligatures, joining forms, components and color layers survive:

```rust
let subset = font.subset_text("Type is the voice of the page.", &[])?;
std::fs::write("subset.ttf", &subset.ttf)?;
// 879,708 bytes to 16,660
```

The result carries a real `cmap`, so it loads straight into `@font-face`. `Font::subset` takes glyph
ids instead when you already know them; its `cmap` keeps the source's characters for the glyphs it
keeps.

A COLR glyph comes back as a scene rather than one outline, since it has more than one color in it:

```rust
let scene = font.colr_scene(gid, &[], 0).expect("is a color glyph");
for op in scene.ops() {
    // paint::Op::Fill { path, paint, rule, transform }, and clips and layers pushed and popped;
    // scene.path(path) is the outline a fill draws, in font units
}
```

`daegun::paint` has the rest of what drawing it takes: `gradient::Ramp` samples a gradient as COLR
defines it, and `composite` lays a layer over the one below in any of COLR's 28 modes.

That is the shape of it. The reference at
[dg.calia.cc](https://dg.calia.cc/reference/rust-methods/) gives each method what it returns, what it
does when it cannot, and the units it answers in.

## The C API

C can do everything the Rust API does, bar a handful of helpers that would be pointless there.
`src/c-wrapper/daegun.h` is the contract, and it states five rules that hold for every call: a
fallible call returns `daegun_status` and answers through an out parameter, NULL where a pointer is
required returns `DAEGUN_NULL` and is never dereferenced, daegun allocates and daegun frees, a borrowed
view lasts until the handle it came from is freed or changed, and a handle a call only reads may be
shared by threads while one a call changes may not.

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

/* your rasterizer's callbacks, and the state they draw into */
daegun_pen pen = { on_move, on_line, on_quad, on_cubic, on_close, &my_rasterizer };
daegun_prepared_glyph glyph;
daegun_font_prepared_outline(font, gids[0], 32.0f, NULL, 0, NULL, &pen, &glyph);
/* the callbacks got the outline in pixels, and glyph.advance_width is in pixels too */

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

`daegun_font_prepared_outline` takes a `daegun_outline_options`, and passing NULL for it means the
defaults, so you need not build the struct to get them. Quadratic curves, flattening and overlap
resolution, color scenes with their gradients and compositing, and subpixel filters all have their C
forms; the header groups them and says what each borrows.

The reference at [dg.calia.cc](https://dg.calia.cc/reference/c-functions/) says which pointers each
call borrows and how long each stays valid.

## Subsetting

![A table of four fonts. For each: the original file size, the size after subsetting to one sentence,
and the percentage removed. Inter 859 KB to 25.2 KB, Source Han Sans 8.0 MB to 12.5 KB, Scheherazade
New 324 KB to 24.7 KB, STIX Two Math 789 KB to 11.5 KB.](https://raw.githubusercontent.com/zztaeha/daegun/images-v2/assets/images/01-subsetting.png)

A page of Japanese wanting 37 characters ships 12.5 KB rather than 8.0 MB. Every glyph the text can
reach survives, joining forms and ligatures included, and the rest goes.

## Untrusted input

![Twelve cards, one per damaged font: empty, a single byte, header only, truncated at 1, 50 and 99
percent, wrong magic, 65,535 tables, offsets past the end of the file, lengths of 4 GB, 4,096 flipped
bytes, and random noise. Six are marked refused and six parsed and survived. A tally above reads 12
inputs, 6 refused, 0 crashes.](https://raw.githubusercontent.com/zztaeha/daegun/images-v2/assets/images/03-robustness.png)

Each of those is handed to every public entry point that will take it. Six are refused with an error.
The other six open as FreeType would open them, skipping what is missing, and are then driven through
every public `Font` call. None of them crashes or panics.

## What holds it up

| | |
|---|---|
| Dependencies | None at all, so the supply chain is the Rust compiler and nothing else. |
| Unsafe code | Forbidden in the whole engine, every parser and the shaper included. The compiler enforces it: an inner `allow` is a hard error rather than a warning. Only the C layer opts back in, and it says so once, at the top of `src/c-wrapper/mod.rs`. |
| C ABI | Every call in the Rust API is reachable from C bar a handful that would be pointless there, the header matches the library's own symbol table in both directions, and every constant in it matches the Rust value it copies. The gate below fails if any of that stops being true. |
| Hostile input | 500 mutated fonts on every gate run, each one reproducible from its seed. |
| Sanitizers | The C round trip runs under AddressSanitizer and UndefinedBehaviorSanitizer. |
| Panics | `unwrap` and `expect` are linted against outside tests, and the C library is built with `panic = "abort"`, so it never unwinds into a C caller. |

One command runs all of it, along with the tests, clippy, rustdoc and the minimum supported Rust
version:

```sh
sh scripts/tools/perf/gate.sh
```

[dg.calia.cc](https://dg.calia.cc) is the reference for both APIs, so this file can stay a tour.
[SECURITY.md](SECURITY.md) sets out the threat model and how to report a vulnerability.
[benchmark.md](benchmark.md) carries the latency benchmarks, with the machine and build they were
measured on.

## License

MIT. See [LICENSE](LICENSE).
