use super::*;
pub use crate::daecore::daetype::instancer::AxisTag;

// A newtype and not an alias because f64 has no Eq or Hash. `FontCache::axis_key` drops non-finite
// values first, so bit-pattern equality never has to handle NaN.
#[derive(Clone, Default)]
pub(crate) struct AxisKey(Pairs);

// Up to `INLINE` axes are held in place, so a call naming a location allocates nothing for its key.
const INLINE: usize = 4;

#[derive(Clone)]
enum Pairs {
    Inline(u8, [(AxisTag, f64); INLINE]),
    Heap(Vec<(AxisTag, f64)>),
}

impl Default for Pairs {
    fn default() -> Self {
        Pairs::Inline(0, [(AxisTag::new(&[0; 4]), 0.0); INLINE])
    }
}

impl core::fmt::Debug for AxisKey {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("AxisKey").field(&&**self).finish()
    }
}

impl PartialEq for AxisKey {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self.iter().zip(other.iter()).all(|(a, b)| a.0 == b.0 && a.1.to_bits() == b.1.to_bits())
    }
}
impl Eq for AxisKey {}

// Ordered on the bit patterns equality uses, as f64 has no Ord: not numeric for negatives, whose patterns invert, but a
// map key needs only a total order agreeing with Eq.
impl Ord for AxisKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.iter().map(|(tag, v)| (tag, v.to_bits())).cmp(other.iter().map(|(tag, v)| (tag, v.to_bits())))
    }
}

impl PartialOrd for AxisKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// A location as its font resolved it, the key and normalized coordinates in one block, so each call making a location
// bumps one count. Equal and ordered by key alone, as the font's coordinates follow from it.
pub(crate) struct ResolvedAxes {
    pub(crate) key: AxisKey,
    pub(crate) coords: Vec<f64>,
}

impl PartialEq for ResolvedAxes {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl Eq for ResolvedAxes {}

impl Ord for ResolvedAxes {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

impl PartialOrd for ResolvedAxes {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// The default instance's key, borrowed where a location names none rather than built and dropped each call.
pub(crate) static EMPTY_KEY: AxisKey = AxisKey(Pairs::Inline(0, [(AxisTag::new(&[0; 4]), 0.0); INLINE]));

impl AxisKey {
    // The pairs as given, for a cache built for one location alone; a value that isn't finite is left out.
    pub(crate) fn of(pairs: &[(AxisTag, f64)]) -> AxisKey {
        pairs.iter().copied().filter(|p| p.1.is_finite()).collect()
    }

    // The pairs a caller asked for, where `key`, made of them, doesn't hold them as given: a tag that parses and a
    // finite value, each tag's last, -0.0 as 0.0, which locates the same. None where it does, as mostly it does.
    pub(crate) fn asked<P: AxisPair>(pairs: &[P], key: &AxisKey) -> Option<AxisKey> {
        let given = |(p, k): (&P, &(AxisTag, f64))| p.axis_id() == Some(k.0.0) && (p.value() + 0.0) == k.1;
        if pairs.len() == key.len() && pairs.iter().zip(key.iter()).all(given) {
            return None;
        }
        let finite = |p: &P| Some((crate::daecore::Tag(p.axis_id()?), p.value() + 0.0)).filter(|p| p.1.is_finite());
        let mut asked: AxisKey = pairs.iter().rev().filter_map(finite).collect();
        let held = match &mut asked.0 {
            Pairs::Inline(n, held) => &mut held[..usize::from(*n)],
            Pairs::Heap(pairs) => &mut pairs[..],
        };
        // Stable, so of a tag named twice the last given comes first, and is the one kept.
        held.sort_by_key(|p| p.0);
        let mut kept = 0;
        for i in 0..held.len() {
            if kept == 0 || held[kept - 1].0 != held[i].0 {
                held[kept] = held[i];
                kept += 1;
            }
        }
        match &mut asked.0 {
            Pairs::Inline(n, _) => *n = kept as u8,
            Pairs::Heap(pairs) => pairs.truncate(kept),
        }
        let same = asked.len() == key.len() && asked.iter().all(|p| key.contains(p));
        (!same).then_some(asked)
    }

    // What the key holds on the heap, as a cache keyed on it charges it.
    pub(crate) fn heap_bytes(&self) -> usize {
        match &self.0 {
            Pairs::Inline(..) => 0,
            Pairs::Heap(pairs) => crate::daecore::daekit::base::sync::heap_of(pairs),
        }
    }

    fn push(&mut self, pair: (AxisTag, f64)) {
        match &mut self.0 {
            Pairs::Inline(n, held) if usize::from(*n) < INLINE => {
                held[usize::from(*n)] = pair;
                *n += 1;
            }
            Pairs::Inline(_, held) => {
                let mut pairs = held.to_vec();
                pairs.push(pair);
                self.0 = Pairs::Heap(pairs);
            }
            Pairs::Heap(pairs) => pairs.push(pair),
        }
    }
}

impl FromIterator<(AxisTag, f64)> for AxisKey {
    fn from_iter<I: IntoIterator<Item = (AxisTag, f64)>>(pairs: I) -> AxisKey {
        let mut key = AxisKey::default();
        pairs.into_iter().for_each(|pair| key.push(pair));
        key
    }
}

impl core::ops::Deref for AxisKey {
    type Target = [(AxisTag, f64)];
    fn deref(&self) -> &Self::Target {
        match &self.0 {
            Pairs::Inline(n, held) => &held[..usize::from(*n)],
            Pairs::Heap(pairs) => pairs,
        }
    }
}

// The one rule for a caller's tag, axis or layout: four bytes, a short one padded with spaces as fonts store it
// ("lao ", "MKD "). A longer one is no tag, so it matches nothing.
pub(crate) fn tag_bytes(tag: &str) -> Option<[u8; 4]> {
    Some(match *tag.as_bytes() {
        [] => [b' '; 4],
        [a] => [a, b' ', b' ', b' '],
        [a, b] => [a, b, b' ', b' '],
        [a, b, c] => [a, b, c, b' '],
        [a, b, c, d] => [a, b, c, d],
        _ => return None,
    })
}

/// What names an axis in a caller's location: a `Tag`, or a string read as one by `tag_bytes`' rule.
pub trait AxisName {
    /// The tag as a number, none for a string longer than four bytes.
    fn axis_id(&self) -> Option<u32>;
}

impl AxisName for crate::daecore::Tag {
    fn axis_id(&self) -> Option<u32> {
        Some(self.0)
    }
}

impl<S: AsRef<str>> AxisName for S {
    fn axis_id(&self) -> Option<u32> {
        tag_bytes(self.as_ref()).map(u32::from_be_bytes)
    }
}

/// A caller's axis and its value, as Rust pairs them or as C lays them out.
pub trait AxisPair {
    fn axis_id(&self) -> Option<u32>;
    fn value(&self) -> f64;
}

impl<S: AxisName> AxisPair for (S, f64) {
    fn axis_id(&self) -> Option<u32> {
        self.0.axis_id()
    }

    fn value(&self) -> f64 {
        self.1
    }
}

// One of fvar's axes as a caller's value meets it: its tag as a number (None where an earlier axis has it) and its
// range (None for a tag listed twice, which has no one range, so any value stands).
#[derive(Debug)]
pub(crate) struct AxisRange {
    id: Option<u32>,
    range: Option<(f64, f64, f64)>,
}

impl AxisRange {
    // What a key holds for `value` on this axis: the value clamped to the range, nothing at the default. Plus 0.0
    // turns -0.0 into 0.0, which locates the same.
    fn entry(&self, value: f64) -> Option<(AxisTag, f64)> {
        let tag = crate::daecore::Tag(self.id?);
        match self.range {
            Some((lo, default, hi)) => {
                let value = value.clamp(lo, hi) + 0.0;
                (value != default).then_some((tag, value))
            }
            None => Some((tag, value + 0.0)),
        }
    }
}

// fvar's axes in its order, none for a font without a readable fvar, and their tags sorted, each with where it lies.
pub(crate) struct AxisRanges {
    in_fvar: Vec<AxisRange>,
    by_tag: Vec<(u32, usize)>,
}

pub(crate) fn axis_ranges(fvar: Option<&[u8]>) -> AxisRanges {
    let Some(Ok(axes)) = fvar.map(crate::daecore::daetype::decoder::axis_records) else {
        return AxisRanges { in_fvar: Vec::new(), by_tag: Vec::new() };
    };
    let mut in_fvar: Vec<AxisRange> = axes
        .map(|a| AxisRange {
            id: Some(u32::from_be_bytes(a.tag)),
            range: Some((a.min.min(a.max), a.default, a.max.max(a.min))),
        })
        .collect();
    let mut by_tag = Vec::with_capacity(in_fvar.len());
    by_tag.extend(in_fvar.iter().enumerate().filter_map(|(i, a)| Some((a.id?, i))));
    by_tag.sort_unstable();
    by_tag.dedup_by(|later, first| {
        let twice = later.0 == first.0;
        if twice {
            in_fvar[first.1].range = None;
            in_fvar[later.1].id = None;
        }
        twice
    });
    AxisRanges { in_fvar, by_tag }
}

impl FontCache {
    // The one way a caller's axes become a key, so one location is one key: the font's own axes in fvar's order,
    // clamped, left out at their default, the last repeat winning; the default and static fonts have the empty key.
    #[inline]
    pub fn axis_key<P: AxisPair>(&self, axis_values: &[P]) -> AxisKey {
        if axis_values.is_empty() {
            return AxisKey::default();
        }
        self.named_axis_key(axis_values)
    }

    // Its one caller would take it in whole, and then be too large to inline where it is called.
    #[inline(never)]
    fn named_axis_key<P: AxisPair>(&self, axis_values: &[P]) -> AxisKey {
        let ranges = self.font.axes.get_or_init(|| axis_ranges(self.font.fvar.as_deref()));
        let AxisRanges { in_fvar: axes, by_tag } = ranges;
        if axes.is_empty() {
            return AxisKey::default();
        }
        // Where in fvar the axis the caller's `n`th pair names lies, none for a tag the font lacks or a value that
        // isn't finite.
        let find = |n: usize, pair: &P| {
            let id = pair.axis_id().filter(|_| pair.value().is_finite())?;
            // Callers mostly name the axes as fvar lists them, which one comparison checks before a search.
            match axes.get(n) {
                Some(axis) if axis.id == Some(id) => Some(n),
                _ => by_tag.binary_search_by_key(&id, |&(id, _)| id).ok().map(|i| by_tag[i].1),
            }
        };
        // Named in fvar's order, each axis once, the key is built as the pairs come.
        let mut key = AxisKey::default();
        let mut last = None;
        for (n, pair) in axis_values.iter().enumerate() {
            let Some(i) = find(n, pair) else { continue };
            if last.is_some_and(|last| last >= i) {
                // Named in another order, or an axis twice: each axis takes its last value.
                let mut values = vec![None; axes.len()];
                for (n, pair) in axis_values.iter().enumerate() {
                    if let Some(i) = find(n, pair) {
                        values[i] = Some(pair.value());
                    }
                }
                return axes.iter().zip(values).filter_map(|(axis, value)| axis.entry(value?)).collect();
            }
            last = Some(i);
            if let Some(entry) = axes[i].entry(pair.value()) {
                key.push(entry);
            }
        }
        key
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daekit/cache/key.rs"]
mod tests;
