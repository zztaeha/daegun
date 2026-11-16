// Spaces a font may lack, drawn with its space glyph at the width Unicode gives each (chapter 6.2, "Space
// Characters").
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SpaceFallback {
    // As wide as the font's own space.
    Space,
    // An em divided by this.
    EmDiv(u8),
    // As wide as a digit.
    Figure,
    // As wide as a period.
    Punctuation,
    // Half as wide as the font's space.
    Narrow,
    // Four eighteenths of an em.
    Em4Of18,
}

// The byte a glyph keeps its space's kind in, never 0, which it reads as none: a fraction of an em is its divisor
// under this bit, any other kind its place in `WHOLE` plus one.
const FRACTION: u8 = 0x80;
const WHOLE: [SpaceFallback; 5] = [
    SpaceFallback::Space,
    SpaceFallback::Figure,
    SpaceFallback::Punctuation,
    SpaceFallback::Narrow,
    SpaceFallback::Em4Of18,
];

impl SpaceFallback {
    pub(crate) fn to_byte(self) -> u8 {
        match self {
            SpaceFallback::EmDiv(divisor) => FRACTION | divisor,
            kind => 1 + WHOLE.iter().position(|&k| k == kind).unwrap_or_default() as u8,
        }
    }

    pub(crate) fn from_byte(byte: u8) -> Option<SpaceFallback> {
        match byte {
            0 | FRACTION => None,
            _ if byte > FRACTION => Some(SpaceFallback::EmDiv(byte & !FRACTION)),
            _ => WHOLE.get(usize::from(byte) - 1).copied(),
        }
    }

    // What a figure or punctuation space takes its width from: the first of these the font has.
    pub(crate) fn width_sources(self) -> &'static [char] {
        match self {
            SpaceFallback::Figure => &['0', '1', '2', '3', '4', '5', '6', '7', '8', '9'],
            SpaceFallback::Punctuation => &['.', ','],
            _ => &[],
        }
    }
}

// The width each space has by its definition: em and en quads and spaces, the per-em spaces, the thin space at a fifth
// of an em, the hair space thinner still, the ideographic space as wide as an ideograph, and so on.
pub(crate) fn space_fallback(c: char) -> Option<SpaceFallback> {
    use SpaceFallback::{Em4Of18, EmDiv, Figure, Narrow, Punctuation, Space};
    let kind = match c {
        ' ' | '\u{00A0}' => Space,
        '\u{2001}' | '\u{2003}' | '\u{3000}' => EmDiv(1),
        '\u{2000}' | '\u{2002}' => EmDiv(2),
        '\u{2004}' => EmDiv(3),
        '\u{2005}' => EmDiv(4),
        '\u{2009}' => EmDiv(5),
        '\u{2006}' => EmDiv(6),
        '\u{200A}' => EmDiv(16),
        '\u{2007}' => Figure,
        '\u{2008}' => Punctuation,
        '\u{202F}' => Narrow,
        '\u{205F}' => Em4Of18,
        _ => return None,
    };
    Some(kind)
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/unicode/spaces.rs"]
mod tests;
