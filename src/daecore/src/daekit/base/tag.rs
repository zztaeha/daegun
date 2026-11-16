#[cfg(doc)]
use crate::ErrorKind::InvalidArgument;
use core::fmt;
use core::str::FromStr;

use crate::daecore::Error;

/// A four-byte OpenType tag naming a table, script, language, feature or axis. Tags order as the big-endian number
/// their bytes spell, as fonts sort them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(pub(crate) u32);

impl Tag {
    /// The tag these four bytes spell: `Tag::new(b"wght")`.
    #[must_use]
    pub const fn new(bytes: &[u8; 4]) -> Tag {
        Tag(u32::from_be_bytes(*bytes))
    }

    /// Its four bytes.
    pub const fn to_bytes(self) -> [u8; 4] {
        self.0.to_be_bytes()
    }

    pub(crate) const DEFAULT_SCRIPT: Tag = Tag::new(b"DFLT");
    pub(crate) const DEFAULT_LANGUAGE: Tag = Tag::new(b"dflt");
}

impl From<[u8; 4]> for Tag {
    fn from(bytes: [u8; 4]) -> Tag {
        Tag::new(&bytes)
    }
}

/// The tag whose bytes spell this big-endian number, as OpenType stores it and C's `daegun_tag` holds it.
impl From<u32> for Tag {
    fn from(number: u32) -> Tag {
        Tag(number)
    }
}

/// The big-endian number its bytes spell.
impl From<Tag> for u32 {
    fn from(tag: Tag) -> u32 {
        tag.0
    }
}

/// One to four printable ASCII characters and no space, as OpenType has them, a shorter tag padded with spaces as
/// fonts store it (`"lao"` and `"lao "` are both `"lao "`). Anything else, an empty string among them, is an
/// [`InvalidArgument`] error.
///
/// ```
/// use daegun::Tag;
///
/// assert_eq!("lao".parse(), Ok(Tag::new(b"lao ")));
/// assert!("".parse::<Tag>().is_err() && " lao".parse::<Tag>().is_err() && "é".parse::<Tag>().is_err());
/// // A font's own tag may hold any byte, which `{:?}` escapes.
/// assert_eq!(format!("{:?}", Tag::from(0)), r#"Tag("\0\0\0\0")"#);
/// ```
impl FromStr for Tag {
    type Err = Error;

    fn from_str(s: &str) -> Result<Tag, Error> {
        let name = s.trim_end_matches(' ');
        let mut bytes = [b' '; 4];
        if name.is_empty() || name.len() > 4 || !name.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(Error::invalid("tag: not one to four printable ASCII characters, padded with spaces"));
        }
        bytes[..name.len()].copy_from_slice(name.as_bytes());
        Ok(Tag::new(&bytes))
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.to_bytes().iter().try_for_each(|&b| fmt::Write::write_char(f, char::from(b)))
    }
}

// Escaped, as a font's own tag may hold a byte no tag should.
impl fmt::Debug for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Tag(\"")?;
        self.to_bytes().iter().try_for_each(|&b| write!(f, "{}", char::from(b).escape_debug()))?;
        f.write_str("\")")
    }
}
