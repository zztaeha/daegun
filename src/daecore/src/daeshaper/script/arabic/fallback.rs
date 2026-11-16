// Arabic fallback shaping: a font whose layout has none of the Arabic forms gets lookups built from the presentation
// forms its cmap maps, run after `rlig`, each under its feature's mask.

use alloc::collections::BTreeSet;
use alloc::vec::Vec;

use crate::daecore::daeshaper::run::{Run, Mask};
use crate::daecore::daeshaper::face::Face;
use crate::daecore::daeshaper::generated::arabic_fallback::{LAM_ALEF, SHAPING, SHAPING_FIRST};
use crate::daecore::daeshaper::ot::apply::TableRun;
use crate::daecore::daeshaper::ot::digest::Digest;
use crate::daecore::daeshaper::ot::gsub::{KINDS, LIGATURE, SINGLE};
use crate::daecore::daeshaper::ot::lookup_flags::IGNORE_MARKS;
use crate::daecore::daeshaper::ot::tag::Tag;
use crate::daecore::daeshaper::ot::{lookup_index, LayoutTable};
use crate::daecore::daeshaper::plan::Features;

// The features of `SHAPING`'s columns, then the lam-alef ligatures'.
const FEATURES: [&[u8; 4]; 5] = [b"init", b"medi", b"fina", b"isol", b"rlig"];

// A GSUB holding the fallback's lookups, and the mask each runs under, in the order they run.
pub(crate) struct Fallback {
    gsub: Vec<u8>,
    masks: Vec<Mask>,
    // Each lookup's, built as a font's own are, so the run passes over glyphs no coverage holds.
    digests: Vec<Digest>,
}

// What a subset keeps for its fallback to shape its text as the font's does: codes for its cmap.
pub(crate) struct SubsetNeeds {
    pub(crate) codes: BTreeSet<u32>,
}

impl Fallback {
    pub(crate) fn new(map: &Features, face: &Face) -> Option<Self> {
        let mut fallback = from_unicode(map, face)?;
        fallback.digests = digests(&fallback.gsub);
        Some(fallback)
    }

    // For text mapped as `text`: the forms and ligatures the text reaches, and every entry of a glyph with several, so
    // the subset's lowest code is the font's.
    pub(crate) fn subset_needs(&self, map: &Features, face: &Face, text: &[(u32, u32)]) -> SubsetNeeds {
        let wanted = |i: usize| map.on_bit(Tag::new(FEATURES[i])) != 0;
        let mut singles = Vec::new();
        for form in (0..4).filter(|&i| wanted(i)) {
            for (code, forms) in (SHAPING_FIRST..).zip(&SHAPING).filter(|(_, forms)| forms[form] != 0) {
                let substitute = u32::from(forms[form]);
                if let (Some(glyph), Some(to)) = (glyph16(face, code), glyph16(face, substitute))
                    && glyph != to
                {
                    singles.push((form, glyph, to, [code, substitute]));
                }
            }
        }
        let sets = if wanted(4) { lam_alef(face) } else { Vec::new() };

        let mut reached: BTreeSet<u16> = text.iter().filter_map(|&(_, glyph)| u16::try_from(glyph).ok()).collect();
        loop {
            let before = reached.len();
            reached.extend(singles.iter().filter(|e| reached.contains(&e.1)).map(|e| e.2).collect::<Vec<_>>());
            let formed: Vec<u16> =
                sets.iter().filter(|s| reached.contains(&s.lam)).flat_map(|s| s.formed.iter().map(|f| f.1)).collect();
            reached.extend(formed);
            if reached.len() == before {
                break;
            }
        }
        let all = repeated(singles.iter().filter(|e| reached.contains(&e.1)).map(|e| (e.0, e.1)).collect())
            || repeated(sets.iter().filter(|s| reached.contains(&s.lam)).map(|s| s.lam).collect());
        let keep = |glyph: &u16| all || reached.contains(glyph);
        let mut codes: BTreeSet<u32> = singles.iter().filter(|e| keep(&e.1)).flat_map(|e| e.3).collect();
        let lams = sets.iter().filter(|s| keep(&s.lam));
        codes.extend(lams.flat_map(|s| s.formed.iter().flat_map(|f| f.2).chain([s.code])));
        SubsetNeeds { codes }
    }

    pub(crate) fn shape(&self, face: &Face, buffer: &mut Run) {
        let Some(table) = LayoutTable::parse(&self.gsub) else { return };
        let mut lookups = TableRun::synthetic(face, &table, buffer);
        for (index, &mask) in (0..).zip(&self.masks) {
            lookups.apply_lookup(index, mask, self.digests.get(usize::from(index)));
        }
    }
}

fn digests(gsub: &[u8]) -> Vec<Digest> {
    let Some(table) = LayoutTable::parse(gsub) else { return Vec::new() };
    let mut work = usize::MAX;
    let mut index = |l| lookup_index(&l, &KINDS, false, |_, _| None, |_, _| None, |_| false, &mut work);
    (0..table.lookup_count()).map(|i| table.lookup(i).map(&mut index).unwrap_or_default().digest).collect()
}

// The fallback writes 16-bit lookups, so a glyph past 65,535 is none to it.
fn glyph16(face: &Face, code: u32) -> Option<u16> {
    face.glyph_of(code).and_then(|g| u16::try_from(g).ok())
}

fn from_unicode(map: &Features, face: &Face) -> Option<Fallback> {
    let letters: Vec<Option<u16>> = (SHAPING_FIRST..).take(SHAPING.len()).map(|code| glyph16(face, code)).collect();
    let mut masks = Vec::new();
    let mut lookups = Vec::new();
    for (i, feature) in FEATURES.iter().enumerate() {
        let mask = map.on_bit(Tag::new(feature));
        if mask == 0 {
            continue;
        }
        let lookup = if i < 4 { single(face, &letters, i) } else { ligature(face) };
        if let Some(lookup) = lookup {
            masks.push(mask);
            lookups.push(lookup);
        }
    }
    if masks.is_empty() {
        return None;
    }
    let offsets: Vec<usize> = lookups.iter().scan(0, |at, l| Some(core::mem::replace(at, *at + l.len()))).collect();
    Some(Fallback { gsub: gsub(&offsets, &lookups.concat())?, masks, digests: Vec::new() })
}

// A GSUB of no scripts or features, whose lookups lie in `body` at `offsets`.
fn gsub(offsets: &[usize], body: &[u8]) -> Option<Vec<u8>> {
    let list = 2 + 2 * offsets.len();
    let mut out = words(&[1, 0, 10, 12, 14, 0, 0]);
    out.extend(words(&[u16::try_from(offsets.len()).ok()?]));
    for &at in offsets {
        out.extend(words(&[u16::try_from(list + at).ok()?]));
    }
    out.extend_from_slice(body);
    Some(out)
}

// `SHAPING`'s column `form` as a single substitution, which skips a form drawn as its letter. `letters` holds each
// row's glyph.
fn single(face: &Face, letters: &[Option<u16>], form: usize) -> Option<Vec<u8>> {
    let mut pairs: Vec<(u16, u16)> = letters
        .iter()
        .zip(&SHAPING)
        .filter(|(_, forms)| forms[form] != 0)
        .filter_map(|(&glyph, forms)| Some((glyph?, glyph16(face, forms[form].into())?)))
        .filter(|(glyph, substitute)| glyph != substitute)
        .collect();
    pairs.sort_by_key(|&(glyph, _)| glyph);
    let glyphs: Vec<u16> = pairs.iter().map(|&(glyph, _)| glyph).collect();
    let resolved = resolve(&glyphs, |i| pairs[i].1)?;

    let (coverage, n) = (6 + 2 * resolved.len(), resolved.len());
    let mut out = words(&[SINGLE, IGNORE_MARKS, 1, 8, 2, u16::try_from(coverage).ok()?, u16::try_from(n).ok()?]);
    out.extend(resolved.iter().flat_map(|&(_, substitute)| substitute.to_be_bytes()));
    out.extend(coverage_of(&resolved));
    Some(out)
}

// Each lam form the font maps, in code order, with the lam-alef ligatures it forms: the alef's glyph, the ligature's
// and their codes. A ligature forms only when the font maps all three of its codes; a lam forming none is left out.
struct LamSet {
    lam: u16,
    code: u32,
    formed: Vec<(u16, u16, [u32; 2])>,
}

fn lam_alef(face: &Face) -> Vec<LamSet> {
    let mut sets: Vec<LamSet> = Vec::new();
    for &[lam, alef, ligature] in &LAM_ALEF {
        let (code, codes) = (u32::from(lam), [u32::from(alef), u32::from(ligature)]);
        let glyphs = (glyph16(face, code), glyph16(face, codes[0]), glyph16(face, codes[1]));
        let (Some(lam), Some(alef), Some(ligature)) = glyphs else { continue };
        match sets.last_mut() {
            Some(set) if set.code == code => set.formed.push((alef, ligature, codes)),
            _ => sets.push(LamSet { lam, code, formed: alloc::vec![(alef, ligature, codes)] }),
        }
    }
    sets
}

// The lam-alef ligatures as a ligature substitution, which skips marks between lam and alef.
fn ligature(face: &Face) -> Option<Vec<u8>> {
    let mut sets = lam_alef(face);
    sets.sort_by_key(|set| set.lam);
    let glyphs: Vec<u16> = sets.iter().map(|set| set.lam).collect();
    let resolved = resolve(&glyphs, |i| &sets[i].formed)?;

    let mut out = words(&[LIGATURE, IGNORE_MARKS, 1, 8]);
    let subtable = out.len();
    out.extend(words(&[1, 0, u16::try_from(resolved.len()).ok()?]));
    let offsets = out.len();
    out.resize(offsets + 2 * resolved.len(), 0);
    for (k, (_, set)) in resolved.iter().enumerate() {
        let at = out.len();
        put(&mut out, offsets + 2 * k, at - subtable)?;
        out.extend(words(&[u16::try_from(set.len()).ok()?]));
        out.resize(at + 2 + 2 * set.len(), 0);
        for (j, &(alef, ligature, _)) in set.iter().enumerate() {
            let here = out.len() - at;
            put(&mut out, at + 2 + 2 * j, here)?;
            out.extend(words(&[ligature, 2, alef]));
        }
    }
    let coverage = out.len() - subtable;
    put(&mut out, subtable + 2, coverage)?;
    out.extend(coverage_of(&resolved));
    Some(out)
}

// One entry for each glyph of `glyphs`, sorted stably from code order, a glyph several codes share taking the first's,
// the lowest code's.
fn resolve<T>(glyphs: &[u16], entry: impl Fn(usize) -> T) -> Option<Vec<(u16, T)>> {
    let first = |&i: &usize| i == 0 || glyphs[i - 1] != glyphs[i];
    let resolved: Vec<(u16, T)> = (0..glyphs.len()).filter(first).map(|i| (glyphs[i], entry(i))).collect();
    (!resolved.is_empty()).then_some(resolved)
}

fn repeated<T: Ord>(mut keys: Vec<T>) -> bool {
    keys.sort_unstable();
    keys.windows(2).any(|w| w[0] == w[1])
}

fn coverage_of<T>(resolved: &[(u16, T)]) -> Vec<u8> {
    let mut out = words(&[1, resolved.len() as u16]);
    out.extend(resolved.iter().flat_map(|(glyph, _)| glyph.to_be_bytes()));
    out
}

fn put(out: &mut [u8], at: usize, value: usize) -> Option<()> {
    out[at..at + 2].copy_from_slice(&u16::try_from(value).ok()?.to_be_bytes());
    Some(())
}

fn words(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_be_bytes()).collect()
}

#[cfg(test)]
#[path = "../../../../tests/unit/daeshaper/script/arabic/fallback.rs"]
mod tests;
