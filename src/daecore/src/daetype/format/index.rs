use alloc::vec::Vec;

const PAGE_BITS: u32 = 8;
const PAGE: usize = 1 << PAGE_BITS;

const NO_PAGE: u32 = u32::MAX;

#[cfg_attr(test, derive(PartialEq, Debug))]
enum Kind {
    Direct { base: u32, values: Vec<u16> },
    Paged { base_page: u32, page_of: Vec<u32>, values: Vec<u16> },
}

#[cfg_attr(test, derive(PartialEq, Debug))]
pub struct SparseIndex {
    absent: u16,
    kind: Kind,
}

// A bit for every page of keys up to the last Unicode scalar value.
const PAGE_WORDS: usize = (0x11_0000 >> PAGE_BITS) / 64;

// What `SparseIndex::build`'s entries are put to (`Put`): first sizing the index, then filling it.
pub struct Entries<'a>(Pass<'a>);

enum Pass<'a> {
    Size { first: u32, last: u32, pages: &'a mut [u64; PAGE_WORDS], wide: bool },
    Direct { first: u32, values: &'a mut [u16], absent: u16 },
    Paged { first_page: u32, page_of: &'a [u32], values: &'a mut [u16], absent: u16 },
}

// Where entries go: a key at a time, or a run of keys whose values rise by `step` (0 or 1) a key, wrapping.
pub trait Put {
    fn put(&mut self, key: u32, value: u32);

    // About how many runs are coming, for a sink that keeps them.
    fn expect(&mut self, _runs: usize) {}

    fn put_run(&mut self, first: u32, last: u32, value: u32, step: u32) {
        for (i, key) in (first..=last).enumerate() {
            self.put(key, value.wrapping_add(step.wrapping_mul(i as u32)));
        }
    }
}

// The values a run writes into `slots`, each slot still `absent` taking its value, so a key keeps its first.
fn fill(slots: &mut [u16], value: u16, step: u16, absent: u16) {
    let mut value = value;
    for slot in slots {
        if *slot == absent {
            *slot = value;
        }
        value = value.wrapping_add(step);
    }
}

impl Put for Entries<'_> {
    fn put(&mut self, key: u32, value: u32) {
        Entries::put(self, key, value);
    }

    fn put_run(&mut self, first: u32, last: u32, value: u32, step: u32) {
        Entries::put_run(self, first, last, value, step);
    }
}

impl Entries<'_> {
    #[inline]
    pub fn put(&mut self, key: u32, value: u32) {
        self.put_run(key, key, value, 0);
    }

    #[inline]
    pub fn put_run(&mut self, first: u32, last: u32, value: u32, step: u32) {
        if first > last {
            return;
        }
        if first >> PAGE_BITS == last >> PAGE_BITS {
            self.put_page(first, last, value, step);
        } else {
            self.put_pages(first, last, value, step);
        }
    }

    // A run within one page. The sizing pass refuses a value past 16 bits, as it does a key past Unicode, so the
    // filling passes store each one whole.
    #[inline]
    fn put_page(&mut self, first: u32, last: u32, value: u32, step: u32) {
        let (slots, absent) = match &mut self.0 {
            Pass::Size { first: lo, last: hi, pages, wide } => {
                let page = (first >> PAGE_BITS) as usize;
                match pages.get_mut(page / 64) {
                    Some(word) => *word |= 1 << (page % 64),
                    None => *wide = true,
                }
                *wide |= u64::from(value) + u64::from(step) * u64::from(last - first) > u64::from(u16::MAX);
                *lo = (*lo).min(first);
                *hi = (*hi).max(last);
                return;
            }
            Pass::Direct { first: base, values, absent } => {
                let at = first.checked_sub(*base).map(|at| at as usize);
                (at.and_then(|at| values.get_mut(at..=at + (last - first) as usize)), *absent)
            }
            Pass::Paged { first_page, page_of, values, absent } => {
                let base = (first >> PAGE_BITS).checked_sub(*first_page).and_then(|p| page_of.get(p as usize));
                let base = base.filter(|&&base| base != NO_PAGE).map(|&base| base as usize);
                let at = base.map(|base| base + (first as usize & (PAGE - 1)));
                (at.and_then(|at| values.get_mut(at..=at + (last - first) as usize)), *absent)
            }
        };
        if let Some(slots) = slots {
            fill(slots, value as u16, step as u16, absent);
        }
    }

    // A run over pages, a page at a time. Past Unicode, one key is enough for the sizing pass to refuse the build.
    #[inline(never)]
    fn put_pages(&mut self, first: u32, last: u32, value: u32, step: u32) {
        let last = last.min(first.max(0x11_0000));
        let mut key = first;
        loop {
            let end = (key | (PAGE as u32 - 1)).min(last);
            self.put_page(key, end, value.wrapping_add(step.wrapping_mul(key - first)), step);
            if end == last {
                break;
            }
            key = end + 1;
        }
    }
}

// Entries kept as the runs they were put as, for a source that costs more to read twice than its runs do.
#[derive(Default)]
pub struct Runs(Vec<(u32, u32, u32, u32)>);

impl Put for Runs {
    fn put(&mut self, key: u32, value: u32) {
        self.0.push((key, key, value, 0));
    }

    fn expect(&mut self, runs: usize) {
        self.0.reserve(runs);
    }

    fn put_run(&mut self, first: u32, last: u32, value: u32, step: u32) {
        self.0.push((first, last, value, step));
    }
}

impl Runs {
    // Each run again, in the order it was put.
    pub fn put_to(&self, e: &mut Entries) {
        self.0.iter().for_each(|&(first, last, value, step)| e.put_run(first, last, value, step));
    }
}

// A set of glyphs, for entries read in no order: each source read once into it, then given as its runs. It grows to
// the highest glyph held, at most 24 bits' worth, past which no table names a glyph.
#[derive(Default)]
pub struct GlyphBits(Vec<u64>);

impl GlyphBits {
    pub fn new() -> GlyphBits {
        GlyphBits(Vec::new())
    }

    #[inline]
    pub fn insert(&mut self, glyph: u32) {
        let word = glyph as usize / 64;
        if word >= 1 << 18 {
            return;
        }
        if word >= self.0.len() {
            self.0.resize((word + 1).max((self.0.len() * 2).min(1024)), 0);
        }
        self.0[word] |= 1 << (glyph % 64);
    }

    // Each run of glyphs held, first to last, ascending.
    pub fn runs(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        let mut at = 0u32;
        let end = self.0.len() as u32 * 64;
        core::iter::from_fn(move || {
            let held = |g: u32| g < end && self.0[g as usize / 64] >> (g % 64) & 1 != 0;
            while at < end && !held(at) {
                let word = self.0[at as usize / 64] >> (at % 64);
                at = if word == 0 { (at | 63) + 1 } else { at + word.trailing_zeros() };
            }
            if at >= end {
                return None;
            }
            let first = at;
            while held(at) {
                let word = !self.0[at as usize / 64] >> (at % 64);
                at = if word == 0 { (at | 63) + 1 } else { at + word.trailing_zeros() };
            }
            Some((first, at - 1))
        })
    }
}

impl SparseIndex {
    #[inline]
    pub fn lookup(&self, key: u32) -> u16 {
        match &self.kind {
            Kind::Direct { base, values } => key
                .checked_sub(*base)
                .and_then(|i| values.get(i as usize))
                .copied()
                .unwrap_or(self.absent),
            Kind::Paged { base_page, page_of, values } => (key >> PAGE_BITS)
                .checked_sub(*base_page)
                .and_then(|p| page_of.get(p as usize).copied())
                .filter(|&base| base != NO_PAGE)
                .and_then(|base| values.get(base as usize + (key as usize & (PAGE - 1))))
                .copied()
                .unwrap_or(self.absent),
        }
    }

    pub fn bytes(&self) -> usize {
        match &self.kind {
            Kind::Direct { values, .. } => values.len() * 2,
            Kind::Paged { page_of, values, .. } => page_of.len() * 4 + values.len() * 2,
        }
    }

    // From the entries `each` gives, alike both times (to size, then to fill), a key's first value other than `absent`
    // kept. None for no entries, a key past Unicode, a value past 16 bits, an index over `budget`, or a refusal.
    pub fn build(each: &dyn Fn(&mut Entries) -> Option<()>, absent: u16, budget: usize) -> Option<SparseIndex> {
        let mut pages = [0u64; PAGE_WORDS];
        let mut size = Entries(Pass::Size { first: u32::MAX, last: 0, pages: &mut pages, wide: false });
        each(&mut size)?;
        let Pass::Size { first, last, wide, .. } = size.0 else { return None };
        if first > last || wide {
            return None;
        }

        let span = (last - first) as usize + 1;
        let direct_bytes = span.saturating_mul(2);

        let first_page = first >> PAGE_BITS;
        let last_page = last >> PAGE_BITS;
        let directory = (last_page - first_page) as usize + 1;
        let words = &pages[first_page as usize / 64..=last_page as usize / 64];
        let populated = words.iter().map(|w| w.count_ones() as usize).sum::<usize>();
        let paged_bytes = directory
            .saturating_mul(4)
            .saturating_add(populated.saturating_mul(PAGE * 2));

        if direct_bytes.min(paged_bytes) > budget {
            return None;
        }

        // Direct up to 4 KB past paged: a small table read per glyph skips the page directory's extra load.
        let kind = if direct_bytes <= budget && direct_bytes <= paged_bytes.saturating_add(4096) {
            let mut values = alloc::vec![absent; span];
            each(&mut Entries(Pass::Direct { first, values: &mut values, absent }))?;
            Kind::Direct { base: first, values }
        } else {
            // Pages take their places in key order.
            let mut page_of = alloc::vec![NO_PAGE; directory];
            let mut next = 0u32;
            for page in first_page..=last_page {
                if pages[page as usize / 64] >> (page % 64) & 1 != 0 {
                    page_of[(page - first_page) as usize] = next;
                    next += PAGE as u32;
                }
            }
            let mut values = alloc::vec![absent; populated * PAGE];
            each(&mut Entries(Pass::Paged { first_page, page_of: &page_of, values: &mut values, absent }))?;
            Kind::Paged { base_page: first_page, page_of, values }
        };

        Some(SparseIndex { absent, kind })
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/daetype/format/index.rs"]
mod tests;
