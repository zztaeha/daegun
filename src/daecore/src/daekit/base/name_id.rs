use core::fmt;

/// An ID the name table keys a string by: those below 256 the specification's, from 256 on the font's own, which
/// fvar, STAT and CPAL name their axes, values and palettes with.
#[expect(clippy::exhaustive_structs, reason = "a newtype over the table's ID")]
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct NameId(
    /// The ID as the table stores it.
    pub u16,
);

impl NameId {
    /// The copyright notice.
    pub const COPYRIGHT_NOTICE: NameId = NameId(0);
    /// The family name, as a family of up to four styles groups it.
    pub const FAMILY_NAME: NameId = NameId(1);
    /// The subfamily name: Regular, Italic, Bold or Bold Italic.
    pub const SUBFAMILY_NAME: NameId = NameId(2);
    /// The unique identifier.
    pub const UNIQUE_ID: NameId = NameId(3);
    /// The full name.
    pub const FULL_NAME: NameId = NameId(4);
    /// The version string.
    pub const VERSION_STRING: NameId = NameId(5);
    /// The PostScript name.
    pub const POSTSCRIPT_NAME: NameId = NameId(6);
    /// The trademark.
    pub const TRADEMARK: NameId = NameId(7);
    /// The manufacturer's name.
    pub const MANUFACTURER: NameId = NameId(8);
    /// The designer's name.
    pub const DESIGNER: NameId = NameId(9);
    /// The description.
    pub const DESCRIPTION: NameId = NameId(10);
    /// The vendor's URL.
    pub const VENDOR_URL: NameId = NameId(11);
    /// The designer's URL.
    pub const DESIGNER_URL: NameId = NameId(12);
    /// The license description.
    pub const LICENSE_DESCRIPTION: NameId = NameId(13);
    /// The license's URL.
    pub const LICENSE_URL: NameId = NameId(14);
    /// The typographic family name, for a family past four styles.
    pub const TYPOGRAPHIC_FAMILY_NAME: NameId = NameId(16);
    /// The typographic subfamily name.
    pub const TYPOGRAPHIC_SUBFAMILY_NAME: NameId = NameId(17);
    /// The compatible full name, Macintosh only.
    pub const COMPATIBLE_FULL_NAME: NameId = NameId(18);
    /// The sample text.
    pub const SAMPLE_TEXT: NameId = NameId(19);
    /// The PostScript CID findfont name.
    pub const POSTSCRIPT_CID_FINDFONT_NAME: NameId = NameId(20);
    /// The WWS family name.
    pub const WWS_FAMILY_NAME: NameId = NameId(21);
    /// The WWS subfamily name.
    pub const WWS_SUBFAMILY_NAME: NameId = NameId(22);
    /// The light background palette's name.
    pub const LIGHT_BACKGROUND_PALETTE: NameId = NameId(23);
    /// The dark background palette's name.
    pub const DARK_BACKGROUND_PALETTE: NameId = NameId(24);
    /// The prefix of the PostScript names of a variable font's instances.
    pub const VARIATIONS_POSTSCRIPT_NAME_PREFIX: NameId = NameId(25);
}

impl From<u16> for NameId {
    fn from(id: u16) -> NameId {
        NameId(id)
    }
}

impl From<NameId> for u16 {
    fn from(id: NameId) -> u16 {
        id.0
    }
}

impl fmt::Display for NameId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
