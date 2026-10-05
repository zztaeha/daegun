use super::*;

// A newtype and not an alias because f64 has no Eq or Hash. `canonical_axes` drops non-finite
// values first, so bit-pattern equality never has to handle NaN.
#[derive(Clone, Debug, Default)]
pub struct AxisKey(Vec<(String, f64)>);

impl PartialEq for AxisKey {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().zip(&other.0).all(|(a, b)| a.0 == b.0 && a.1.to_bits() == b.1.to_bits())
    }
}
impl Eq for AxisKey {}

// Ordered on the same bit patterns equality uses, since f64 has no Ord either. Not numerically
// meaningful for negatives – the patterns invert – but a map key needs only a total order that
// agrees with Eq.
impl Ord for AxisKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.0
            .iter()
            .map(|(tag, v)| (tag, v.to_bits()))
            .cmp(other.0.iter().map(|(tag, v)| (tag, v.to_bits())))
    }
}

impl PartialOrd for AxisKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl core::ops::Deref for AxisKey {
    type Target = [(String, f64)];
    fn deref(&self) -> &Self::Target { &self.0 }
}

// An OpenType tag is four bytes, and `fvar` stores short ones space-padded. A caller writing
// ("M1", 1.0) for an axis tagged "M1  " otherwise gets silence rather than an error.
pub fn normalize_tag(tag: &str) -> String {
    let mut out = String::with_capacity(4);
    out.push_str(tag);
    while out.len() < 4 { out.push(' '); }
    out
}

// Sorted and deduplicated in place. A font is asked for one or two axes, and a BTreeMap costs a
// node allocation and a second vector to collect back out of it for that.
pub fn canonical_axes<S: AsRef<str>>(axis_values: &[(S, f64)]) -> AxisKey {
    let mut out: Vec<(String, f64)> = Vec::with_capacity(axis_values.len());
    for (tag, value) in axis_values {
        if !value.is_finite() { continue; }
        let tag = normalize_tag(tag.as_ref());
        match out.binary_search_by(|(seen, _)| seen.as_str().cmp(tag.as_str())) {
            Ok(i) => out[i].1 = *value,
            Err(i) => out.insert(i, (tag, *value)),
        }
    }
    AxisKey(out)
}
