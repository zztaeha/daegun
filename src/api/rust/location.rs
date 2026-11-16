use core::fmt;

use super::Font;
use crate::daecore::daekit::cache::{AxisKey, ResolvedAxes};
use crate::daecore::Tag;
use crate::sync::{Lined, Shared};

/// A variable font's place in its design space, resolved once by [`Font::location`] and passed to each call that
/// depends on one. `Location::default()` is the default instance, of any font. A location keeps the axis values it
/// was asked for, and another font resolves it again from them, so it means one thing to every font: two locations
/// are equal when they were asked for the same values, whichever font made them.
#[derive(Clone, Default)]
pub struct Location {
    resolved: Option<Resolved>,
}

// What the font made of the caller's axes, its number, and the axes as asked where the key doesn't hold them as given:
// another font resolves those for itself, as the making font's clamping and dropped defaults would misplace them.
#[derive(Clone)]
struct Resolved {
    font: crate::sync::Id,
    at: Shared<Lined<ResolvedAxes>>,
    asked: Option<Shared<AxisKey>>,
}

impl Location {
    #[cfg(feature = "capi")]
    pub(crate) const DEFAULT: Location = Location { resolved: None };

    /// The axes the font that made it sets, in fvar's order, each clamped to its range. An axis at its default, one
    /// the font lacks and a value that isn't finite are left out.
    pub fn coords(&self) -> &[(Tag, f64)] {
        self.resolved.as_ref().map_or(&[], |r| &r.at.key)
    }

    // Whether it is the default of every font, made of no values by no font.
    pub(crate) fn is_default(&self) -> bool {
        self.resolved.is_none()
    }

    // The values asked for, each tag's last finite one, in the order made: what makes two locations one.
    fn asked(&self) -> &[(Tag, f64)] {
        match &self.resolved {
            Some(r) => r.asked.as_deref().unwrap_or(&r.at.key),
            None => &[],
        }
    }

    /// The normalized coordinates, one an axis in fvar's order, from -1 to 1 after avar. Empty at the default
    /// instance, where every one is 0.
    #[doc(alias("normalized_axes"))]
    pub fn normalized(&self) -> &[f64] {
        let normalized = self.resolved.as_ref().map_or(&[][..], |r| &r.at.coords[..]);
        if normalized.iter().all(|&v| v == 0.0) { &[] } else { normalized }
    }
}

// Each side names a tag once, in whatever order it was made in.
impl PartialEq for Location {
    fn eq(&self, other: &Location) -> bool {
        let (a, b) = (self.asked(), other.asked());
        a.len() == b.len() && a.iter().all(|pair| b.contains(pair))
    }
}

// Coordinates are finite, so equality is total.
impl Eq for Location {}

impl core::hash::Hash for Location {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        // Summed, so the order the pairs lie in counts no more than it does to equality.
        let sum = self.asked().iter().fold(0u64, |sum, &(tag, value)| {
            sum.wrapping_add((u64::from(tag.0) << 32 ^ value.to_bits()).wrapping_mul(0x9e37_79b9_7f4a_7c15))
        });
        (self.asked().len(), sum).hash(state);
    }
}

impl fmt::Debug for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Location").field(&self.coords()).finish()
    }
}

impl Font {
    /// The location these axis values name: user coordinates, as fvar ranges them. A tag the font lacks is
    /// ignored, a value is clamped to its axis, and the last of a tag named twice wins. Passed to another font, the
    /// location is resolved again from these values.
    ///
    /// # Examples
    ///
    /// ```
    /// use daegun::shape::ShapeOptions;
    /// use daegun::{Location, Tag};
    ///
    /// # fn main() -> daegun::Result<()> {
    /// # let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
    /// # let Ok(bytes) = std::fs::read(path) else { return Ok(()) };
    /// # let font = daegun::Font::from_bytes(&bytes)?;
    /// let bold = font.location(&[(Tag::new(b"wght"), 700.0)]);
    /// // Resolved once, then handed to every call at that weight.
    /// let wide = font.shape("Wave", &bold, &ShapeOptions::DEFAULT).width();
    /// # assert!(wide > font.shape("Wave", &Location::default(), &ShapeOptions::DEFAULT).width());
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn location(&self, coords: &[(Tag, f64)]) -> Location {
        self.resolve(coords)
    }

    // The same from any caller's pairs, C's coordinates as C lays them out among them.
    pub(crate) fn resolve<P: crate::daecore::daekit::cache::AxisPair>(&self, coords: &[P]) -> Location {
        let at = self.cache.resolve_axes(&self.cache.axis_key(coords));
        let asked = AxisKey::asked(coords, &at.key).map(Shared::new);
        Location { resolved: Some(Resolved { font: self.token, at, asked }) }
    }

    // The key a location is to this font: its own as made, or another font's asked values resolved again.
    pub(crate) fn key<'l>(&self, location: &'l Location) -> Key<'l> {
        match &location.resolved {
            None => Key::Default,
            Some(r) if r.font == self.token => Key::Made(&r.at),
            Some(r) => Key::Interned(self.foreign(r.asked.as_deref().unwrap_or(&r.at.key))),
        }
    }

    // A key's coordinates as well, the location's own or the font's default, so no call looks them up again.
    pub(crate) fn resolved<'k>(&'k self, key: &'k Key<'_>) -> &'k ResolvedAxes {
        match key {
            Key::Default => self.cache.default_axes(),
            Key::Made(at) => at,
            Key::Interned(at) => at,
        }
    }

    // Out of line, as it is rare and would cost every call at a location its frame.
    #[inline(never)]
    fn foreign(&self, coords: &[(Tag, f64)]) -> Shared<Lined<ResolvedAxes>> {
        self.cache.resolve_axes(&self.cache.axis_key(coords))
    }
}

// Two pointers rather than a key held in place, so a call at a location keeps it in registers.
pub(crate) enum Key<'l> {
    Default,
    Made(&'l ResolvedAxes),
    Interned(Shared<Lined<ResolvedAxes>>),
}

impl core::ops::Deref for Key<'_> {
    type Target = AxisKey;

    fn deref(&self) -> &AxisKey {
        match self {
            Key::Default => &crate::daecore::daekit::cache::EMPTY_KEY,
            Key::Made(at) => &at.key,
            Key::Interned(at) => &at.key,
        }
    }
}

#[cfg(test)]
#[path = "tests/unit/location.rs"]
mod tests;
