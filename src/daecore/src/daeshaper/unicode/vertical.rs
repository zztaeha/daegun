// How characters set down a line: their Vertical_Orientation (UAX #50) and the presentation forms Unicode encodes for
// vertical text, which the generated table lists from UnicodeData.txt's `<vertical>` decompositions.
use super::{props, t, VerticalOrientation};

/// How a character sets in vertical text (UAX #50).
///
/// ```
/// use daegun::unicode::{vertical_orientation, VerticalOrientation};
///
/// assert_eq!(vertical_orientation('日'), VerticalOrientation::Upright);
/// assert_eq!(vertical_orientation('a'), VerticalOrientation::Rotated);
/// ```
#[doc(alias("is_upright"))]
pub fn vertical_orientation(c: char) -> VerticalOrientation {
    props(c).vertical_orientation
}

// Whether a character stands upright in vertical text: one upright only in a vertical form does where it has one.
pub(crate) fn is_upright(c: char, has_vertical_form: bool) -> bool {
    match vertical_orientation(c) {
        VerticalOrientation::Rotated => false,
        VerticalOrientation::TransformedRotated => has_vertical_form,
        VerticalOrientation::TransformedUpright | VerticalOrientation::Upright => true,
    }
}

/// The presentation form a character takes in vertical text, from the Vertical Forms and CJK Compatibility Forms
/// blocks, such as `︒` for `。`; None for one with none.
///
/// ```
/// use daegun::unicode::vertical_form;
///
/// assert_eq!(vertical_form('。'), Some('︒'));
/// assert_eq!(vertical_form('a'), None);
/// ```
pub const fn vertical_form(c: char) -> Option<char> {
    // A binary search, as a `const fn` can't call the slice's.
    let (forms, wanted) = (&t::VERTICAL_FORMS, c as u32);
    let (mut low, mut high) = (0, forms.len());
    while low < high {
        let middle = (low + high) / 2;
        let (key, form) = forms[middle];
        match key as u32 {
            key if key == wanted => return Some(form),
            key if key < wanted => low = middle + 1,
            _ => high = middle,
        }
    }
    None
}

#[cfg(test)]
#[path = "../../../tests/unit/daeshaper/unicode/vertical.rs"]
mod tests;
