# VARC fixtures

Variable composite glyphs, for the drawing, measuring, subsetting and instancing tests. Each is built from scratch by
`scripts/tools/fixtures/build_varc_fixtures.py`, its base glyphs plain polygons and a circle.

| File | Bytes | What it covers |
|---|---:|---|
| `varc-hangul.ttf` | 1,056 | Hangul 가 and 각 from the jamo ㄱ and ㅏ, in the revised store layout: the leaves widen on wght, and two of 각's components move on it |
| `varc-short-cff2.otf` | 664 | A CFF2 font whose CharStrings hold fewer glyphs than maxp counts, the glyph past them drawn by VARC |
| `varc-hostile.ttf` | 6,200 | Cycles, a chain 20 deep, records cut short or unreadable, 66,000 points, every condition format, every transform field varied, reserved flags, 24-bit ids |
| `varc-cff2-store.otf` | 764 | A CFF2 with a store and no fvar, whose square's width blends on an axis only a VARC component sets |
| `varc-composite-leaf.ttf` | 932 | A VARC glyph whose component is a glyf composite of two squares, each varied, so every glyph a leaf loads takes its own deltas |
