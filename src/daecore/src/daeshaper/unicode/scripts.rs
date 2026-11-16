// Unicode's scripts by the id the generated tables give each (Scripts.txt's names, sorted): their names, ISO 15924
// codes, OpenType tags and the way each is written.
#[cfg(doc)]
use crate::ErrorKind::InvalidArgument;
use crate::daecore::daeshaper::generated::unicode_tables as t;
use crate::daecore::daeshaper::run::Direction;
use crate::daecore::Error;

/// A Unicode script, as [`script_runs`](crate::unicode::script_runs) finds it in text, or from its name or ISO
/// 15924 code with `parse`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Script(pub(crate) u16);

/// Which way a script is written across a line: the direction most of its letters' Bidi classes give, so a script
/// once written either way, as Old Italic was, answers left to right.
#[expect(clippy::exhaustive_enums, reason = "a line runs left or right")]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ScriptDirection {
    /// Left to right.
    LeftToRight,
    /// Right to left.
    RightToLeft,
}

impl Script {
    /// Its Unicode name, such as `"Arabic"`; `"Unknown"` for an id this build has no script for.
    pub fn name(self) -> &'static str {
        self.c_name().to_str().unwrap_or("Unknown")
    }

    // Its name as the C ABI lends it.
    pub(crate) fn c_name(self) -> &'static core::ffi::CStr {
        t::SCRIPT_NAMES.get(usize::from(self.0)).copied().unwrap_or(c"Unknown")
    }

    /// Which way it's written across a line: right to left where most of its letters are of Bidi class R or AL.
    /// Common, Inherited and unknown ids answer left to right.
    #[doc(alias("is_rtl"))]
    pub fn direction(self) -> ScriptDirection {
        match horizontal_direction(self) {
            Direction::RightToLeft => ScriptDirection::RightToLeft,
            _ => ScriptDirection::LeftToRight,
        }
    }

    /// The OpenType script tags it maps to, most specific first: Devanagari is `dev3`, `dev2`, `deva`, and a
    /// shaper tries them in order.
    pub fn opentype_tags(
        self,
    ) -> impl ExactSizeIterator<Item = crate::daecore::Tag> + DoubleEndedIterator + core::iter::FusedIterator + Clone {
        crate::daecore::daeshaper::ot::tag::script_tags(self).into_iter()
    }

    /// Its ISO 15924 code, such as `"Arab"`, which names it in every build; None for an id this build has no script
    /// for.
    pub fn iso15924(self) -> Option<&'static str> {
        t::SCRIPT_ISO_CODES.get(usize::from(self.0)).copied()
    }

    /// The script an ISO 15924 code names, matched without regard to case; None for one this build has no script
    /// for.
    pub fn from_iso15924(code: &str) -> Option<Script> {
        Script::from_iso_tag(u32::from_be_bytes(code.as_bytes().try_into().ok()?))
    }

    // The same for a code packed big-endian, as C passes one. Setting 0x20 in each byte lowercases a letter and turns
    // no other byte into one. A perfect hash, as a binary search's eight dependent loads cost 32 cycles.
    pub(crate) fn from_iso_tag(code: u32) -> Option<Script> {
        let key = code | 0x2020_2020;
        let slot = t::SCRIPT_ISO_SLOTS[(key.wrapping_mul(t::SCRIPT_ISO_MUL) >> 21) as usize];
        let id = usize::from(slot).checked_sub(1)?;
        (t::SCRIPT_ISO_KEYS.get(id) == Some(&key)).then_some(Script(id as u16))
    }

    /// Whether it takes its identity from what surrounds it: true for Common (punctuation, digits, spaces),
    /// Inherited (combining marks) and unknown ids.
    pub fn is_context_dependent(self) -> bool {
        matches!(self, Script::COMMON | Script::INHERITED) || usize::from(self.0) >= t::SCRIPT_NAMES.len()
    }
}

/// Its Unicode name, as [`name`](Script::name) gives it.
impl core::fmt::Display for Script {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.name())
    }
}

impl core::fmt::Debug for Script {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Script({:?})", self.name())
    }
}

/// A Unicode name such as `"Arabic"` or an ISO 15924 code such as `"Arab"`, matched without regard to case. One this
/// build has no script for is an [`InvalidArgument`] error.
impl core::str::FromStr for Script {
    type Err = Error;

    fn from_str(s: &str) -> Result<Script, Error> {
        let by_name = t::SCRIPT_NAMES.iter().position(|name| name.to_bytes().eq_ignore_ascii_case(s.as_bytes()));
        by_name
            .map(|id| Script(id as u16))
            .or_else(|| Script::from_iso15924(s))
            .ok_or(Error::invalid("script: no script by that name or code"))
    }
}

// The way a script runs across a line, as `SCRIPT_DIRECTIONS` records it from its letters' Bidi classes.
pub(crate) fn horizontal_direction(s: Script) -> Direction {
    t::SCRIPT_DIRECTIONS.get(usize::from(s.0)).copied().unwrap_or(Direction::LeftToRight)
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/unicode/scripts.rs"]
mod tests;
