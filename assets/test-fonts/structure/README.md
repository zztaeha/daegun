# Structural fixtures

Two fonts that exercise table-level decisions rather than shaping. Neither behavior is reachable
with an ordinary font, because both fixtures are deliberately malformed or archaic in one specific
way.

From [unicode-org/text-rendering-tests](https://github.com/unicode-org/text-rendering-tests), under
the Unicode License V3 in `LICENSE` beside them, which permits redistribution provided that notice
travels with the files.

| File | Bytes | What it covers |
|---|---:|---|
| `TestSFNTTwo.ttf` | 3,228 | Carries both `glyf` and `CFF `. `sfntVersion` breaks the tie |
| `TestCMAPMacTurkish.ttf` | 19,644 | `cmap` format 0, platform 1, encoding 0, language 18 |
| `TestGVAREight.ttf` | 4,692 | six `fvar` axes tagged with two letters, so short tags need padding |
| `TestShapeLana.ttf` | 87,032 | Tai Tham, whose vowel orders the USE pattern forbids |

`TestSFNTTwo.ttf` is contributed by Simon Cozens and is malformed on purpose: a font carrying two
outline formats has no valid reading except the one `sfntVersion` names. Its glyphs spell out which
table they came from, so a wrong choice is visible rather than subtle.

`TestShapeLana.ttf` is the only Tai Tham font in this tree, and Tai Tham is the one script whose
cluster grammar diverges from the specification's – see `scripts/data/grammars/lana.grammar`. No
other fixture can tell that grammar apart from the ordinary one.

`TestGVAREight.ttf` names its axes `CK`, `FR`, `HV`, `CN`, `BR` and `TC`. Every other variable font
in this tree uses four-letter tags such as `wght`, where padding is a no-op, so nothing else here can
tell a padded tag from an unpadded one.

`TestCMAPMacTurkish.ttf` is the only fixture here with a non-Roman Macintosh encoding. MacOS Turkish
reassigns seven byte values away from MacRoman, six of them to the letters Turkish actually needs
(Ğ ğ İ ı Ş ş). Reading it as MacRoman loses exactly those six.
