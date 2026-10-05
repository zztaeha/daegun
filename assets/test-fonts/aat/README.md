# AAT fixtures

No general-purpose font carries `morx` or a legacy `kern` – the tables are Apple's, and every other
fixture in this tree is an OpenType font. These six exist so `daeshaper::ot::morx` and
`daeshaper::ot::kern` are graded against real tables rather than left unmeasured.

From [unicode-org/text-rendering-tests](https://github.com/unicode-org/text-rendering-tests), under
the Unicode License V3 in `LICENSE` beside them, which permits redistribution provided that notice
travels with the files. That is the whole reason these were chosen over the macOS system fonts, which
carry the same tables and cannot be redistributed at all.

## What each one covers

`daeshaper` implements all five `morx` subtable types. These five fonts are the minimum set that
reaches every one of them – picked by parsing each candidate's subtable coverage bytes, not by name.

| File | Bytes | morx subtable type |
|---|---:|---|
| `TestMORXTen.ttf` | 1,620 | 0 – rearrangement |
| `TestMORXTwentyfour.ttf` | 1,828 | 1 – contextual |
| `TestMORXFourtyone.ttf` | 2,248 | 2 – ligature |
| `TestMORXOne.ttf` | 2,404 | 4 – non-contextual |
| `TestMORXThirtythree.ttf` | 1,520 | 5 – insertion |
| `TestKERNOne.otf` | 1,380 | – legacy `kern` |

## What they are not

These are minimal test fonts. A latency figure from one measures per-call overhead, not what a real
AAT font costs – a production `morx` runs many subtables over thousands of glyphs, where these run
one over a handful. Treat the numbers as a floor.

For a realistically sized `morx`, HarfBuzz's in-house set has a 79 KB font with 7 subtables across
four types; macOS ships 96 more. Neither is here: the first needs its OFL provenance confirmed per
file, and the second cannot be redistributed.
