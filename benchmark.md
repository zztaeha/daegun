# daegun benchmarks

Every latency benchmark the `latency` filter runs: 30 tests across four targets, plus the C harness.
`outline_sweep`'s three sweeps over a whole font are run by name and are not listed. Nothing here is a
summary of something else.

Re-measured for 1.2.0. Every figure below is the best of three consecutive runs, which is what the
closing note recommends.

## Machine

| | |
|---|---|
| CPU | Apple M1 Pro |
| Cores | 10 physical / 10 logical |
| Cache line | 128 bytes |
| RAM | 32 GB |
| OS | darwin 27.0.0 |

## Configuration

| | |
|---|---|
| daegun | 1.2.0 |
| rustc | 1.99.0 (b940084d7 2026-09-28) |
| Profile | release |
| `opt-level` | 3 |
| `lto` | true |
| `codegen-units` | 1 |
| `panic` | abort |
| `overflow-checks` | false |
| `strip` | debuginfo |
| Command | `cargo test --release --test <target> latency -- --ignored --nocapture --test-threads=1` |

## What changed in 1.2.0

daegun no longer rasterizes, so the two rasterizing sections are gone, and with them the
`rasterize_glyph` rows in sections 1 and 7. In their place are the calls a rasterizer of your own
makes instead, all on Inter's `g`: `prepared_outline` and `flatten` at 16px, and `glyph_quads`, which
takes no size.

Everything else was measured against 1.1.7, built from its release tag with its own fixtures, on the
same day, machine and compiler: three rounds with the two versions interleaved, best of three each.
Shaping, the glyf outlines, the autohinter's whole-face sweeps, opening a font and section 6's float
math are within a few percent of 1.1.7. These rows are not:

| | 1.1.7 | 1.2.0 | why |
|---|---|---|---|
| `glyph_id` | 63.2 ns | 44.5 ns | the best cmap subtable is chosen at the font's first lookup |
| `outline_glyph` | 248.3 ns | 270.5 ns | a TrueType outline is drawn at its origin, as FreeType and HarfBuzz draw it, which reads the glyph's side bearing: 18 ns, measured with it turned off |
| `outline_glyf_eb_garamond_composite` | 1.333 µs | 1.416 µs | components are read with the checks a malformed composite needs |
| `outline_cff_stix` | 11.166 µs | 14.042 µs | the CFF pen numbers points for hint masks, finds a seac wherever its endchar sits and draws a charstring that draws before it moves; no single cause |
| `outline_cff_sweep` | 2.304 ms | 2.467 ms | the same |
| `autohint_inter_h` | 458 ns | 625 ns | blue zones are fitted to the size, segments link by a sorted search a crafted outline cannot make quadratic, and edges are swept once; the whole-face sweeps are unchanged |
| `autohint_stix_h` | 1.208 µs | 1.500 µs | the same, and the CFF pen above |
| `hint_glyph_bytecode` | 792 ns | 1.500 µs | hinting as FreeType does: each glyph starts from the twilight zone the CVT program left (1.1.7 zeroed it), IUP interpolates from the unscaled outline, Subpixel keeps all of v40's backward compatibility, and heavy instructions are charged against a work budget |
| `hint_glyph_context_cached` | 791 ns | 1.500 µs | the same |
| `hint_glyph_context_per_glyph` | 3.375 µs | 6.208 µs | the same, and a new context keeps a copy of what the CVT program left, for each glyph to start from |
| `colr_v1_paint_graph_static` | 35.500 µs | 39.792 µs | each paint is charged against a budget, so a crafted graph cannot run without end: the largest new cost in its profile |
| `cache_colr_variable` | 39.958 µs | 43.583 µs | the same |
| `colr_v1_paint_graph_variable` | 549.9 µs | 754.7 µs | the same 16,800 region scalars a sweep, each about 1.4 times slower: every axis is checked for a range the spec says to ignore, one that runs backward or crosses zero |

`line_metrics` reads 9.9 ns against 9.2 with the same code as 1.1.7; that difference is not
attributed. The cached path a caller takes, `cache_colr_variable`, keeps the region scalars for a
location, which is why it is seventeen times faster than `colr_v1_paint_graph_variable`.

Section 6 changed with daemachine's own work. Gradients now blend as the OpenType spec says, in linear
light with alpha premultiplied: a linear one takes 11.8 ns a pixel against 14.6 for 1.1.7's sRGB blend,
a sweep 23.8 against 30.7, and a radial 23.1 against 21.0, the one row slower. The last row is a
linear gradient blended in sRGB, as Chrome does it: 7.4. SrcOver composites in 4.0 rather than 5.5, and
f64 `round` without std takes 0.78 ns rather than 1.40. The blending anchor, `blend` in SrcOver, reads
2.0 ns rather than 1.0: `#[inline]` now lets a caller's crate inline these functions, and in this LTO
build it also reshapes a loop that calls `blend` directly. `composite`, what a rasterizer calls, is
the faster for it.

`f32 round` reads 0.763 ns against 0.671 for another reason: 1.2.0 changed the benchmark's own loop,
from `chunks_exact(4)` to `as_chunks::<4>()`, and run with the old loop it reads 0.671 too. The blend
rows read the same with either loop. `blend HardLight` swings between runs: 3.38 to 4.04 ns for 1.1.7
across these three.

Compare releases on figures measured the same day.

---

## 1. Rust API

`--test api` · `api_latency` · 200 rounds after 50 warmup

| | min | median |
|---|---|---|
| `from_bytes`, borrowed | 15.208 µs | 15.333 µs |
| `from_vec`, owned | 1.125 µs | 1.333 µs |
| `flatten` | 983.8 ns | 987.3 ns |
| `prepared_outline` | 805.8 ns | 822.3 ns |
| `glyph_quads` | 454.0 ns | 456.1 ns |
| `outline_glyph` | 270.5 ns | 271.0 ns |
| `glyph_id` | 44.5 ns | 44.8 ns |
| `advance_widths` ×1 | 19.8 ns | 20.0 ns |
| `line_metrics` | 9.9 ns | 10.0 ns |
| `descender` | 6.4 ns | 6.5 ns |
| `ascender` | 6.4 ns | 6.5 ns |
| `cap_height` | 0.8 ns | 0.8 ns |
| `upm` | 0.3 ns | 0.3 ns |
| `num_glyphs` | 0.3 ns | 0.3 ns |

---

## 2. Shaping

`--test shaper` · 5 tests · one shaped run per sample

| test | font | n | min | median | p95 |
|---|---|---|---|---|---|
| `shape_cjk_sentence_source_han` | Source Han Sans JP, 30 chars | 120,000 | 917 ns | 1.041 µs | 1.083 µs |
| `shape_latin_ligatures_eb_garamond` | EB Garamond, liga fires repeatedly | 20,000 | 2.916 µs | 3.042 µs | 3.167 µs |
| `shape_arabic_joined_run_scheherazade` | Scheherazade New, one fully joined run | 15,000 | 8.250 µs | 8.458 µs | 8.750 µs |
| `shape_devanagari_conjuncts_noto` | Noto Sans Devanagari, conjuncts and matra reordering | 8,000 | 8.417 µs | 8.625 µs | 8.875 µs |
| `shape_latin_sentence_inter` | Inter, 78-character sentence | 20,000 | 9.458 µs | 9.625 µs | 9.917 µs |

---

## 3. Outlines

`--test type` · `outline_latency` · 9 tests

| test | scope | n | min | median |
|---|---|---|---|---|
| `autohint_inter_h` | Inter `H` at 13 ppem, glyf, collect + grid fit | 20,000 | 625 ns | 709 ns |
| `outline_glyf_eb_garamond_composite` | EB Garamond gid 2244, 7-component composite | 50,000 | 1.416 µs | 1.541 µs |
| `autohint_stix_h` | STIX `H` at 13 ppem, CFF, collect + grid fit | 20,000 | 1.500 µs | 1.625 µs |
| `outline_glyf_scheherazade` | Scheherazade gid 1583, 1,683 points | 20,000 | 9.041 µs | 9.167 µs |
| `outline_cff_stix` | STIX gid 2257, 5,819-byte Type 2 charstring | 2,000 | 14.042 µs | 14.166 µs |
| `outline_glyf_sweep` | EB Garamond, 3,247 glyphs, whole face | 60 | 2.356 ms | 2.423 ms |
| `outline_cff_sweep` | STIX, 5,543 glyphs, 123,321 segments | 30 | 2.467 ms | 2.511 ms |
| `autohint_sweep_inter` | Inter, 2,937 glyphs, 131,498 points | 20 | 6.862 ms | 7.034 ms |
| `autohint_sweep_stix` | STIX, 5,543 glyphs, 211,893 points | 10 | 14.540 ms | 14.837 ms |

---

## 4. Hinting

`--test type` · `hint_latency` · 3 tests

| test | scope | n | min | median |
|---|---|---|---|---|
| `hint_glyph_bytecode` | 5 hinted glyphs at 16 ppem | 40,000 | 1.500 µs | 1.625 µs |
| `hint_glyph_context_cached` | 5 glyphs through `FontCache` | 40,000 | 1.500 µs | 1.625 µs |
| `hint_glyph_context_per_glyph` | 5 glyphs, `HintContext` rebuilt each time | 4,000 | 6.208 µs | 6.625 µs |

---

## 5. Color

`--test type` · `colr_latency` · 3 tests

| test | scope | n | min | median |
|---|---|---|---|---|
| `colr_v1_paint_graph_static` | 200 base glyphs, whole sweep | 2,000 | 39.792 µs | 40.167 µs |
| `cache_colr_variable` | 200 base glyphs through `FontCache` | 2,000 | 43.583 µs | 44.083 µs |
| `colr_v1_paint_graph_variable` | 200 base glyphs, whole sweep | 2,000 | 754.708 µs | 765.334 µs |

---

## 6. Math

`--test machine` · 9 tests · daegun is `no_std` and carries its own float math

### `float_ext_against_std` – ratio > 1 means daegun is slower

| | daegun | std | ratio |
|---|---|---|---|
| `f32 abs` | 0.244 ns | 0.244 ns | 1.00× |
| `f64 abs` | 0.234 ns | 0.234 ns | 1.00× |
| `f32 round_ties_even` | 0.376 ns | 0.244 ns | 1.54× |
| `f32 floor` | 0.397 ns | 0.244 ns | 1.63× |
| `f64 trunc` | 0.539 ns | 0.234 ns | 2.30× |
| `f64 round_ties_even` | 0.549 ns | 0.234 ns | 2.35× |
| `f64 ceil` | 0.621 ns | 0.234 ns | 2.65× |
| `f32 round` | 0.763 ns | 0.244 ns | 3.13× |
| `f64 round` | 0.783 ns | 0.234 ns | 3.35× |
| `f64 floor` | 0.834 ns | 0.234 ns | 3.56× |

### `sqrt_against_std` – Newton iteration against one hardware instruction

| | daegun | std | ratio |
|---|---|---|---|
| `f32 sqrt` | 2.340 ns | 0.295 ns | 7.93× |
| `f64 sqrt` | 2.706 ns | 0.315 ns | 8.59× |

### `trig_against_std`

| | daegun | std | ratio |
|---|---|---|---|
| `f64 sin_cos` | 4.089 ns | 6.927 ns | 0.59× |
| `f64 atan2` | 7.589 ns | 7.589 ns | 1.00× |

### `rounding_old_against_new` – ratio < 1 means the new one is faster

| | new | old | ratio |
|---|---|---|---|
| `round_ties_even` | 0.549 ns | 1.933 ns | 0.28× |
| `round` | 0.783 ns | 1.404 ns | 0.56× |
| `ceil` | 0.621 ns | 0.743 ns | 0.84× |
| `floor` | 0.834 ns | 0.987 ns | 0.84× |
| `trunc` | 0.539 ns | 0.621 ns | 0.87× |

### `atan2_cost_breakdown`

| | first | second | ratio |
|---|---|---|---|
| atan2 vs one divide | 7.192 ns | 0.336 ns | 21.40× |
| atan2 vs horner20 | 7.162 ns | 3.286 ns | 2.18× |
| horner20 vs one divide | 3.296 ns | 0.336 ns | 9.81× |
| estrin20 vs horner20 | 2.156 ns | 3.286 ns | 0.66× |

### `atan_polynomial_shape` – 11 coefficients, identical inputs

| | first | second | ratio |
|---|---|---|---|
| estrin4 vs current | 1.312 ns | 1.271 ns | 1.03× |

### `atan_reduction_shape` – ratio < 1 means the first named is faster

| | first | second | ratio |
|---|---|---|---|
| hybrid, one arm | 3.571 ns | 4.028 ns | 0.89× |
| branchless, scattered | 5.605 ns | 5.666 ns | 0.99× |
| branchless, raster sweep | 5.605 ns | 5.524 ns | 1.01× |
| hybrid, mid range | 6.765 ns | 6.592 ns | 1.03× |
| hybrid, raster sweep | 6.205 ns | 5.513 ns | 1.13× |
| hybrid, scattered | 6.836 ns | 5.666 ns | 1.21× |
| branchless, one arm | 5.605 ns | 4.028 ns | 1.39× |

### `sqrt_iteration_shape`

| | first | second | ratio |
|---|---|---|---|
| rsqrt 4 vs current | 2.777 ns | 2.696 ns | 1.03× |
| rsqrt 5 vs current | 3.520 ns | 2.706 ns | 1.30× |
| current vs std sqrt | 2.706 ns | 0.305 ns | 8.87× |

Accuracy against std, worst ulp over 60,000 values in [1e-8, 1e8]:

| | worst ulp |
|---|---|
| newton on root, 4 passes | 1 |
| newton on reciprocal, 5 passes | 2 |
| newton on reciprocal, 4 passes | 3 |

### `daemath_baseline` – blending, per pixel, anchored on SrcOver

| | ns/px | anchor | ratio |
|---|---|---|---|
| blend Multiply | 1.638 | 1.953 | 0.84× |
| blend HardLight | 4.873 | 1.953 | 2.50× |
| blend HslSaturation | 7.253 | 1.973 | 3.68× |
| composite SrcOver | 3.957 | 2.045 | 1.93× |

### `daemath_baseline` – gradients, per pixel, anchored on linear

| | ns/px | anchor | ratio |
|---|---|---|---|
| gradient linear | 11.790 | anchor | 1.00× |
| gradient radial | 23.082 | 11.902 | 1.94× |
| gradient sweep | 23.793 | 11.790 | 2.02× |
| gradient linear, sRGB | 7.446 | 11.821 | 0.63× |

---

## 7. The C ABI

`src/c-wrapper/tests/latency.c` · 200 rounds after 50 warmup · both sides on a release build

| | Rust min | C min | Rust median | C median |
|---|---|---|---|---|
| `from_bytes`, borrowed | 15.208 µs | 15.000 µs | 15.333 µs | 15.000 µs |
| `from_vec`, owned | 1.125 µs | 1.000 µs | 1.333 µs | 1.000 µs |
| `flatten` | 983.8 ns | 1.004 µs | 987.3 ns | 1.010 µs |
| `prepared_outline` | 805.8 ns | 852.0 ns | 822.3 ns | 856.0 ns |
| `glyph_quads` | 454.0 ns | 486.0 ns | 456.1 ns | 490.0 ns |
| `outline_glyph` | 270.5 ns | 302.0 ns | 271.0 ns | 304.0 ns |
| `glyph_id` | 44.5 ns | 48.0 ns | 44.8 ns | 48.0 ns |
| `advance_widths` ×1 | 19.8 ns | 48.0 ns | 20.0 ns | 50.0 ns |
| `upm` | 0.3 ns | 0.0 ns | 0.3 ns | 0.0 ns |

The C column comes off `clock_gettime` over batches of 500, which quantizes it to 2 ns steps. Read
the cheap rows as an upper bound rather than a figure – `upm` at 0.0 ns means "below what this clock
can see", not free. The two open rows are timed one call a sample instead, at the clock's 1 µs
resolution on this machine, so they read low: C is not faster than Rust there.

The first C run of the session read `from_bytes` at 35 µs; every run after it, of either version,
read 15. That is one reason every figure here is the best of three.

---

## About these numbers

They are latency tests living beside the ordinary ones, marked `#[ignore]` because they measure
rather than assert. The gate runs what can fail; these report, and are run by hand.

Every figure is the best of three consecutive runs. A single run of this suite moves by several
percent between passes, so a lone number is not a baseline.

`--test-threads=1` is not optional. The reports are multi-line and interleave into nonsense without
it, which makes them look broken rather than unread.

`min` rather than mean is the number to watch for a regression, being the least polluted by whatever
else the machine was doing. Section 7 is not a cargo target and has to be built against a release
staticlib, or it measures a debug library and reports several times the real cost:

```sh
cargo rustc --release --features capi --crate-type staticlib
cc -std=c11 -O2 -I src/c-wrapper src/c-wrapper/tests/latency.c target/release/libdaegun.a \
   <system libraries, as daegun.h lists them> -o clat && ./clat assets/test-fonts/inter/InterVariable.ttf
```

Each sweep in sections 3 to 5 runs a different face, so divide by the glyph count before reading
anything across their rows. `DAEGUN_SWEEP_FONT` repoints `outline_glyf_sweep`; every other sweep is
fixed to the face named beside it.

For counters rather than wall clock, `scripts/tools/perf/pmu.sh` drives Instruments, and
`insn-diff.py` and `pmu-attribute.py` beside it attribute the difference between two builds down to
the function.
