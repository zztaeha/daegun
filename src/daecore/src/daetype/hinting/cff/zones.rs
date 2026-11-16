use super::dict::Private;
use super::ladder::Scale;
use super::placement::Placement;
use super::stems::{Edges, Stem};
use super::{HALF, ONE};
use crate::daecore::daetype::base::base_script_info;
use crate::daecore::daetype::Tables;
use crate::daecore::Tag;

// An alignment zone (Type 1, 5.3 and 5.4), in 16.16 font units: its flat edge, the family's where that one is
// nearer than a pixel, lands on `pixel`.
#[derive(Clone, Copy, Debug, Default)]
struct Zone {
    low: i64,
    high: i64,
    flat: i64,
    pixel: i64,
    bottom: bool,
}

// A Private DICT's zones at one scale (BlueValues' first pair and OtherBlues bottom zones, its others top zones),
// whether overshoot is suppressed there, and an ideographic FD's em box edges with the pixels they land on.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Zones {
    zones: [Zone; 12],
    count: usize,
    scale: Scale,
    shift: i64,
    fuzz: i64,
    suppress: bool,
    frame: Option<[(i64, i64); 2]>,
}

// The ideographic em box (OpenType BASE, "Ideographic em-box"): HorizAxis 'ideo' its bottom edge, 'idtp' its top or,
// without that, the bottom plus head's em; the 'hani' script's values, else the default script's. 16.16 font units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EmBox {
    bottom: i64,
    top: i64,
}

impl EmBox {
    // None where BASE names no horizontal 'ideo' baseline, or the box it gives is empty.
    pub(crate) fn read(tables: &Tables, em: u16) -> Option<EmBox> {
        let edges = |script: Option<[u8; 4]>| {
            let baselines = base_script_info(tables, script, false, &[])?.baselines;
            let bottom = *baselines.get(&Tag::new(b"ideo"))?;
            Some((bottom, baselines.get(&Tag::new(b"idtp")).copied().unwrap_or(bottom + f64::from(em))))
        };
        let (bottom, top) = edges(Some(*b"hani")).or_else(|| edges(None))?;
        let units = |v: f64| (v * 65536.0) as i64;
        (bottom < top).then(|| EmBox { bottom: units(bottom), top: units(top) })
    }

    // The box in a placed FD's glyph space; None where drawn y doesn't follow glyph y alone.
    pub(crate) fn placed(self, placement: Option<Placement>) -> Option<EmBox> {
        let Some(p) = placement else { return Some(self) };
        Some(EmBox { bottom: p.glyph_y(self.bottom)?, top: p.glyph_y(self.top)? })
    }
}

// The nearest pixel to a device value, a half up.
fn nearest_pixel(device: i64) -> i64 {
    (device + HALF).div_euclid(ONE) * ONE
}

impl Zones {
    pub(crate) fn new(p: &Private, scale: Scale, em_box: Option<EmBox>) -> Zones {
        let (shift, fuzz) = (p.blue_shift, p.blue_fuzz);
        let mut z = Zones { zones: [Zone::default(); 12], count: 0, scale, shift, fuzz, suppress: false, frame: None };
        let bottoms = p.blue_values.get().iter().take(1).chain(p.other_blues.get()).map(|&pair| (pair, true));
        let tops = p.blue_values.get().iter().skip(1).map(|&pair| (pair, false));
        let mut tallest = 0;
        for ((low, high), bottom) in bottoms.chain(tops) {
            if low > high {
                continue;
            }
            tallest = tallest.max(high - low);
            let flat = family_flat(p, if bottom { high } else { low }, bottom, scale);
            z.zones[z.count] = Zone { low, high, flat, pixel: 0, bottom };
            z.count += 1;
        }
        // Under BlueScale pixels a unit (5.6); a font whose tallest zone would reach a pixel there is held to the
        // spec's restriction, which turns suppression off first.
        let per_unit = i128::from(scale.slope());
        z.suppress = per_unit < i128::from(p.blue_scale) && per_unit * i128::from(tallest) < 1 << 48;
        // At these sizes lowercase is a few pixels tall, and a row lost from its x-height closes its counters: a
        // flat edge rounds as if a third of a pixel further out, giving up a row only when barely into it.
        let lean = if z.suppress { ONE / 3 } else { 0 };
        for zone in &mut z.zones[..z.count] {
            let flat = scale.of(zone.flat);
            zone.pixel = nearest_pixel(if zone.bottom { flat - lean } else { flat + lean });
        }
        // An ideographic FD (LanguageGroup 1, Type 1 5.11) sets its glyphs in the em box: an edge on its nearest
        // pixel, so ideographs keep one height at any size.
        let edge = |glyph: i64| (glyph, nearest_pixel(scale.of(glyph)));
        z.frame = em_box.filter(|_| p.language_group == 1).map(|b| [edge(b.bottom), edge(b.top)]);
        z
    }

    pub(super) fn scale(&self) -> Scale {
        self.scale
    }

    pub(super) fn frame(&self) -> Option<[(i64, i64); 2]> {
        self.frame
    }

    // Where a stem's edges land when a zone aligns it: its bottom edge in a bottom zone, else its top edge in a top
    // zone, the other edge kept at the stem's scaled width.
    pub(super) fn align(&self, stem: &Stem) -> Option<(i64, i64)> {
        let width = self.scale.of(stem.top - stem.bottom);
        let caught = |edge: i64, bottom: bool| {
            let mut zones = self.zones[..self.count].iter().filter(|z| z.bottom == bottom);
            zones.find(|z| z.low - self.fuzz <= edge && edge <= z.high + self.fuzz)
        };
        if stem.edges != Edges::Top
            && let Some(zone) = caught(stem.bottom, true)
        {
            let at = self.land(zone, stem.bottom);
            return Some((at, at + width));
        }
        if stem.edges != Edges::Bottom
            && let Some(zone) = caught(stem.top, false)
        {
            let at = self.land(zone, stem.top);
            return Some((at - width, at));
        }
        None
    }

    // An aligned edge: on the flat pixel while overshoot is suppressed; else past it by its distance beyond the
    // flat edge, rounded to whole pixels and at least one where that's BlueShift units or more (5.7).
    fn land(&self, zone: &Zone, edge: i64) -> i64 {
        let beyond = if zone.bottom { zone.flat - edge } else { edge - zone.flat };
        let overshoot = match beyond {
            _ if self.suppress => 0,
            ..=0 => 0,
            _ if beyond >= self.shift => nearest_pixel(self.scale.of(beyond)).max(ONE),
            _ => nearest_pixel(self.scale.of(beyond)),
        };
        if zone.bottom { zone.pixel - overshoot } else { zone.pixel + overshoot }
    }
}

// A flat edge, or the family's (Type 1, 5.5) where one of the same kind is under a pixel from it, the nearest; a
// bottom zone looks at FamilyBlues' first pair and FamilyOtherBlues, a top zone at FamilyBlues' others.
fn family_flat(p: &Private, flat: i64, bottom: bool, scale: Scale) -> i64 {
    let family = p.family_blues.get();
    let bottoms = family.iter().take(1).chain(p.family_other_blues.get()).map(|&(_, high)| high);
    let tops = family.iter().skip(1).map(|&(low, _)| low);
    let mut best: Option<i64> = None;
    for c in bottoms.filter(|_| bottom).chain(tops.filter(|_| !bottom)) {
        let off = (flat - c).abs();
        if scale.of(off) < ONE && best.is_none_or(|b| off < (flat - b).abs()) {
            best = Some(c);
        }
    }
    best.unwrap_or(flat)
}

#[cfg(test)]
#[path = "../../../../tests/unit/daetype/hinting/cff/zones.rs"]
mod tests;
