use super::glyph::{flag, Mask};
use crate::daecore::daekit::text::shape::Ignorables;

// A direction's two bits: its lines run down the page, and against the order the text is stored in.
const DOWN: isize = 0b10;
const BACK: isize = 0b01;

/// The direction a run is shaped in.
#[expect(clippy::exhaustive_enums, reason = "a line runs one of four ways")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Direction {
    /// Left to right, as Latin text runs.
    #[default]
    LeftToRight = 0,
    /// Right to left, as Arabic and Hebrew text runs.
    RightToLeft = BACK,
    /// Top to bottom, as vertical text runs.
    TopToBottom = DOWN,
    /// Bottom to top.
    BottomToTop = DOWN | BACK,
}

/// `ltr`, `rtl`, `ttb` or `btt`, the names shaping tools give a direction.
impl core::fmt::Display for Direction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(Direction::NAMES[*self as usize])
    }
}

/// A direction read by its first letter in either case, as shaping tools read one: `ltr` or `left`, `rtl`, `ttb`,
/// `btt`. Anything else is an [`InvalidArgument`](crate::ErrorKind::InvalidArgument) error.
///
/// ```
/// use daegun::shape::Direction;
///
/// assert_eq!("rtl".parse(), Ok(Direction::RightToLeft));
/// assert_eq!(Direction::TopToBottom.to_string(), "ttb");
/// ```
impl core::str::FromStr for Direction {
    type Err = crate::daecore::Error;

    fn from_str(text: &str) -> Result<Direction, Self::Err> {
        let initial = text.trim().bytes().next().map(|b| b.to_ascii_lowercase());
        let named = Direction::NAMES.iter().position(|name| name.bytes().next() == initial);
        named.map(|bits| Direction::BY_BITS[bits]).ok_or_else(|| {
            crate::daecore::Error::invalid("direction: not ltr, rtl, ttb or btt")
        })
    }
}

impl Direction {
    // Each direction at its bits, and the name shaping tools give it there; the names' initials differ.
    const BY_BITS: [Direction; 4] = [Self::LeftToRight, Self::RightToLeft, Self::TopToBottom, Self::BottomToTop];
    const NAMES: [&'static str; 4] = ["ltr", "rtl", "ttb", "btt"];

    pub(crate) fn is_horizontal(self) -> bool {
        self as isize & DOWN == 0
    }

    pub(crate) fn is_vertical(self) -> bool {
        self as isize & DOWN != 0
    }

    // Against the order the text is stored in: right to left, or bottom to top.
    pub(crate) fn is_backward(self) -> bool {
        self as isize & BACK != 0
    }

    // The other way along the same axis.
    pub(crate) fn reverse(self) -> Direction {
        Direction::BY_BITS[(self as isize ^ BACK) as usize]
    }
}

/// How a run's clusters are formed: whole graphemes or single characters, kept in text order or not.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ClusterLevel {
    /// A grapheme a cluster, clusters in text order.
    #[default]
    MonotoneGraphemes,
    /// A character a cluster, clusters in text order.
    MonotoneCharacters,
    /// A character a cluster, in whatever order shaping leaves them.
    Characters,
    /// A grapheme a cluster, in whatever order shaping leaves them.
    Graphemes,
}

impl ClusterLevel {
    /// Whether clusters stay in text order.
    pub const fn is_monotone(self) -> bool {
        !matches!(self, ClusterLevel::Characters | ClusterLevel::Graphemes)
    }

    /// Whether a cluster spans a whole grapheme.
    pub const fn is_graphemes(self) -> bool {
        !matches!(self, ClusterLevel::MonotoneCharacters | ClusterLevel::Characters)
    }
}

// What the caller asked of a run.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Settings {
    pub(crate) cluster_level: ClusterLevel,
    // The flags the caller reads: unsafe to break always, the other two where asked for.
    pub(crate) reported: Mask,
    pub(crate) ignorables: Ignorables,
    // Nothing precedes the run, so a mark opening it takes a dotted circle.
    pub(crate) starts_text: bool,
    pub(crate) dotted_circles: bool,
    // What stands in for a hidden default ignorable, a character the font lacks and a selector it can't apply.
    pub(crate) invisible: Option<u32>,
    pub(crate) missing: u32,
    pub(crate) missing_selector: Option<u32>,
}

impl Settings {
    pub(crate) fn reports(&self, flags: Mask) -> bool {
        self.reported & flags != 0
    }
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            cluster_level: ClusterLevel::MonotoneGraphemes,
            reported: flag::UNSAFE_TO_BREAK,
            ignorables: Ignorables::Hide,
            starts_text: false,
            dotted_circles: true,
            invisible: None,
            missing: 0,
            missing_selector: None,
        }
    }
}

// The most characters of the text on either side a run reads.
pub(crate) const CONTEXT_CHARS: usize = 5;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Context {
    sides: [[char; CONTEXT_CHARS]; 2],
    counts: [usize; 2],
}

impl Context {
    pub(crate) fn new(before: &str, after: &str) -> Context {
        let mut context = Context::default();
        let mut keep = |side: usize, chars: &mut dyn Iterator<Item = char>| {
            for c in chars.take(CONTEXT_CHARS) {
                context.sides[side][context.counts[side]] = c;
                context.counts[side] += 1;
            }
        };
        keep(0, &mut before.chars().rev());
        keep(1, &mut after.chars());
        context
    }

    // The text before the run as it reads, nearest first.
    pub(crate) fn before(&self) -> &[char] {
        &self.sides[0][..self.counts[0]]
    }

    pub(crate) fn after(&self) -> &[char] {
        &self.sides[1][..self.counts[1]]
    }

    // A run turned to read backward meets the text after it first.
    pub(crate) fn turn(&mut self) {
        self.sides.swap(0, 1);
        self.counts.swap(0, 1);
    }
}

// What shaping has found of a run, so a later stage with nothing to do skips it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u16)]
pub(crate) enum Fact {
    NonAscii = 1,
    Ignorables = 1 << 1,
    SpaceStandIns = 1 << 2,
    Attachments = 1 << 3,
    Cgj = 1 << 4,
    Flagged = 1 << 5,
    BrokenSyllable = 1 << 6,
    SelectorFallback = 1 << 7,
    // One fact a script's model keeps for its own later hook.
    ForScript = 1 << 8,
    Continuations = 1 << 9,
    Deleted = 1 << 10,
    // A run with a span past the marking cap unsafe to break, or to join, so marked throughout as it settles.
    Unbreakable = 1 << 11,
    Unjoinable = 1 << 12,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct Facts(u16);

impl Facts {
    pub(crate) fn has(self, fact: Fact) -> bool {
        self.0 & fact as u16 != 0
    }

    pub(crate) fn note(&mut self, fact: Fact) {
        self.0 |= fact as u16;
    }

    #[must_use]
    pub(crate) fn with(mut self, fact: Fact) -> Facts {
        self.note(fact);
        self
    }

    pub(crate) fn add(&mut self, other: Facts) {
        self.0 |= other.0;
    }
}
