# daegun 2.0.0 – benchmarks

daegun's own timings, from one run of its bench on 2026-11-16 on the machine below. Each section's method says how
it was measured.

## System

| | |
|---|---|
| Model | MacBook Pro 14-inch |
| OS | macOS 27.2.1 |
| CPU | Apple M1 Pro, 10 cores: 8 performance, 2 efficiency |
| Memory | 32 GB |
| Storage | 512 GB |
| Build | daegun's release profile: LTO, one codegen unit |

## Operations

### Method

| | |
|---|---|
| What | daegun's latency tests in `src/`, each timing one operation, run one at a time on one thread |
| Rounds | 24 kept, each a fresh process, after a warm-up round that's thrown away |
| Figure | Each run's median, then each operation's best median across the 24 rounds |
| Per round | The lowest and highest median a single round gave |
| To run it | One round: `cargo test --release --lib -- --ignored --nocapture --test-threads=1 tests::daeshaper::latency:: tests::daetype::outline_latency:: tests::daetype::hint_latency:: tests::daetype::colr_latency::`, then `cargo test --release --test api -- --ignored --nocapture --test-threads=1 latency` |

### Results

| # | Operation | Time | Per round |
|---|---|---|---|
| 1 | `shape_arabic_joined_run_scheherazade` | 5.47 µs | 5.47 µs–5.66 µs |
| 2 | `shape_cjk_sentence_source_han` | 692.0 ns | 692.0 ns–724.0 ns |
| 3 | `shape_devanagari_conjuncts_noto` | 5.42 µs | 5.42 µs–5.67 µs |
| 4 | `shape_latin_ligatures_eb_garamond` | 2.10 µs | 2.10 µs–2.23 µs |
| 5 | `shape_latin_sentence_inter` | 5.89 µs | 5.89 µs–6.21 µs |
| 6 | `shape_myanmar_syllables_noto` | 13.88 µs | 13.88 µs–14.34 µs |
| 7 | `cache_colr_variable` | 28.46 µs | 28.46 µs–29.46 µs |
| 8 | `colr_v1_paint_graph_static` | 19.42 µs | 19.42 µs–20.25 µs |
| 9 | `colr_v1_paint_graph_variable` | 28.46 µs | 28.46 µs–30.38 µs |
| 10 | `hint_glyph_bytecode` | 875.0 ns | 875.0 ns–958.0 ns |
| 11 | `hint_glyph_context_cached` | 917.0 ns | 917.0 ns–958.0 ns |
| 12 | `hint_glyph_context_per_glyph` | 2.50 µs | 2.50 µs–2.67 µs |
| 13 | `autohint_inter_H` | 375.0 ns | 375.0 ns–375.0 ns |
| 14 | `autohint_stix_H` | 1.04 µs | 1.04 µs–1.08 µs |
| 15 | `autohint_sweep_inter` | 4.10 ms | 4.10 ms–4.37 ms |
| 16 | `autohint_sweep_stix` | 8.82 ms | 8.82 ms–9.31 ms |
| 17 | `outline_cff_stix` | 12.50 µs | 12.50 µs–13.33 µs |
| 18 | `outline_cff_sweep` | 2.39 ms | 2.39 ms–2.52 ms |
| 19 | `outline_glyf_eb_garamond_composite` | 2.08 µs | 2.08 µs–2.21 µs |
| 20 | `outline_glyf_scheherazade` | 8.83 µs | 8.83 µs–9.17 µs |
| 21 | `outline_glyf_sweep` | 2.58 ms | 2.58 ms–2.74 ms |
| 22 | `font_open` | 15.54 µs | 15.54 µs–17.79 µs |
| 23 | `font_open_owned` | 260.4 ns | 260.4 ns–281.2 ns |
| 24 | `units_per_em` | 0.3 ns | 0.3 ns–0.4 ns |
| 25 | `glyph_id` | 0.7 ns | 0.7 ns–0.7 ns |
| 26 | `advance` | 4.3 ns | 4.3 ns–4.6 ns |
| 27 | `advance at wght 700` | 11.5 ns | 11.5 ns–12.3 ns |
| 28 | `outline` | 273.0 ns | 273.0 ns–290.2 ns |
| 29 | `ascender` | 0.4 ns | 0.4 ns–0.5 ns |
| 30 | `descender` | 0.4 ns | 0.4 ns–0.5 ns |
| 31 | `cap_height` | 0.8 ns | 0.8 ns–0.8 ns |
| 32 | `glyph_count` | 0.3 ns | 0.3 ns–0.4 ns |
| 33 | `line_metrics` | 2.0 ns | 2.0 ns–2.2 ns |
| 34 | `quads` | 455.5 ns | 455.5 ns–475.8 ns |
| 35 | `prepared_outline` | 330.6 ns | 330.6 ns–354.8 ns |
| 36 | `flatten` | 897.2 ns | 897.2 ns–950.2 ns |
| 37 | `raster_glyph` | 689.4 ns | 689.4 ns–732.3 ns |

## Scenes

### Method

| | |
|---|---|
| What | Real work through the public API, each scene's rows run once an iteration |
| Figure | Each row's median iteration, over 6 seconds a scene |
| Caches | Nothing drawn before is replayed: Outlines draws glyphs new to the font object, Variable moves to a weight not seen before, and `shape`'s cache is cleared before each shape, outside the clock |

### Results

| # | Scene | Work | Time |
|---|---|---|---|
| 1 | Scripts | Latin, Inter | 7.17 µs |
| 2 | Scripts | Latin ligatures, EB Garamond | 2.92 µs |
| 3 | Scripts | Arabic, Scheherazade New | 6.75 µs |
| 4 | Scripts | Devanagari, Noto Sans | 6.46 µs |
| 5 | Scripts | Khmer, Noto Sans | 7.00 µs |
| 6 | Scripts | Japanese, Source Han Sans | 1.29 µs |
| 7 | Scripts | Myanmar, Noto Sans | 15.33 µs |
| 8 | Outlines | glyf, Inter | 15.88 µs |
| 9 | Outlines | glyf with composites, EB Garamond | 31.12 µs |
| 10 | Outlines | CFF, STIX Two | 15.79 µs |
| 11 | Outlines | CFF2, Source Han Sans | 56.33 µs |
| 12 | Variable | Inter, weight animating | 25.71 µs |
| 13 | Variable | Source Serif 4 (CFF2), weight animating | 62.79 µs |
| 14 | Variable | Source Han Sans (CFF2), weight animating | 49.04 µs |
| 15 | Hinting | Bytecode, Irianis ADF | 214.04 µs |
| 16 | Hinting | Autohinter, Inter | 88.12 µs |
| 17 | Hinting | Autohinter, STIX Two | 98.21 µs |
| 18 | Picker | Open 14 fonts, bytes handed over | 4.42 µs |
| 19 | Picker | Open 14 fonts, bytes copied | 645.92 µs |
| 20 | Picker | First metrics read, 14 fonts | 750.0 ns |
| 21 | Color | Bungee Tint (COLR v0), every color glyph | 979.96 µs |
| 22 | Color | COLR v1 test glyphs, location moving | 104.71 µs |

## Rasterizer

### Method

| | |
|---|---|
| Text | "Sphinx of black quartz, judge my vow." in Inter (TrueType, whose contours overlap) and STIX Two Math (CFF), unhinted, outlines warm in daegun's cache |
| End to end | From the font, as a caller draws each glyph |
| Coverage alone | The sentence's outlines, prepared once in pixels, each filled through `Rasterizer::path` |
| Stripes (LCD) | End to end, three samples a pixel through the five-tap filter |
| Rounds | 21, interleaved, each a batch of at least 2 ms a job, the median batch kept |
| Accuracy | Each pixel against the exact coverage of the curves, each curve cut into 512 chords: the worst and the mean gap, in levels of 255 |

`path` takes any path, so it finds where contours meet and resolves those rows exactly, where summing them would
over-darken. Inter's contours overlap, which is what its coverage rows pay for; end to end, daegun learns each
glyph's overlaps once and sums it straight.

### Inter, TrueType

| px | End to end | Coverage alone | Stripes (LCD) | Worst gap | Mean gap |
|---:|---:|---:|---:|---:|---:|
| 10 | 11.18 µs | 42.93 µs | 25.08 µs | 6.2 | 0.95 |
| 20 | 14.98 µs | 43.96 µs | 40.87 µs | 6.6 | 0.73 |
| 40 | 24.62 µs | 54.01 µs | 84.89 µs | 7.2 | 0.45 |
| 80 | 50.96 µs | 94.34 µs | 219.42 µs | 9.2 | 0.25 |
| 160 | 138.23 µs | 216.85 µs | 537.75 µs | 10.0 | 0.14 |
| 200 | 203.08 µs | 304.04 µs | 707.69 µs | 9.9 | 0.11 |

### STIX Two Math, CFF

| px | End to end | Coverage alone | Stripes (LCD) | Worst gap | Mean gap |
|---:|---:|---:|---:|---:|---:|
| 10 | 11.44 µs | 25.14 µs | 24.65 µs | 5.5 | 0.92 |
| 20 | 16.63 µs | 26.15 µs | 42.99 µs | 6.2 | 0.72 |
| 40 | 27.95 µs | 37.34 µs | 88.71 µs | 6.1 | 0.43 |
| 80 | 56.56 µs | 69.31 µs | 238.99 µs | 8.1 | 0.23 |
| 160 | 156.90 µs | 182.81 µs | 581.48 µs | 8.1 | 0.12 |
| 200 | 224.69 µs | 270.75 µs | 734.75 µs | 9.3 | 0.10 |

### Distance fields

The sentence's SDF and MSDF from the same outlines. Each field is kept as bytes, as an atlas keeps it, and read back
between texels, bilinear, 4 by 4 a texel, against whether the curves wind each point: a misread is a point read on
the wrong side, and in brackets those more than a tenth of a pixel from the curves.

Inter, TrueType:

| px | Spread | SDF | MSDF | SDF misreads | MSDF misreads | Points read |
|---:|---:|---:|---:|---:|---:|---:|
| 16 | 2 | 821.19 µs | 2.08 ms | 1,068 (333) | 624 (92) | 72,624 |
| 32 | 4 | 2.09 ms | 4.56 ms | 1,227 (293) | 678 (19) | 297,248 |
| 64 | 4 | 3.07 ms | 7.28 ms | 1,597 (282) | 973 (9) | 829,408 |
| 128 | 8 | 8.17 ms | 20.59 ms | 5,184 (262) | 4,747 (6) | 3,320,224 |

STIX Two Math, CFF:

| px | Spread | SDF | MSDF | SDF misreads | MSDF misreads | Points read |
|---:|---:|---:|---:|---:|---:|---:|
| 16 | 2 | 1.14 ms | 2.86 ms | 2,415 (1,121) | 1,684 (677) | 70,560 |
| 32 | 4 | 2.93 ms | 6.00 ms | 1,772 (446) | 1,084 (122) | 286,464 |
| 64 | 4 | 4.27 ms | 9.42 ms | 1,471 (309) | 807 (29) | 794,672 |
| 128 | 8 | 11.22 ms | 25.31 ms | 2,258 (247) | 1,545 (17) | 3,182,336 |
