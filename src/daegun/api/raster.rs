use super::*;

impl Font {
    pub fn outline_glyph(&self, gid: u16, pen: &mut dyn crate::daecore::daetype::outline::OutlinePen) -> Option<()> {
        self.outline_glyph_instanced(gid, &[], pen)
    }

    pub fn outline_glyph_instanced(&self, gid: u16, axes: &[(&str, f64)], pen: &mut dyn crate::daecore::daetype::outline::OutlinePen) -> Option<()> {
        self.outline_glyph_keyed(gid, &crate::daecore::cache::canonical_axes(axes), pen)
    }

    pub(crate) fn prewarmed_outline(
        &self,
        gid: u16,
        axes: &crate::sync::Shared<crate::daecore::cache::AxisKey>,
    ) -> Option<crate::sync::Shared<crate::daecore::daetype::outline::Path>> {
        let mut cache = crate::sync::write(&self.outlines);
        if cache.len() == 0 { return None; }
        cache
            .get(&(gid, crate::sync::Shared::clone(axes)))
            .map(crate::sync::Shared::clone)
    }

    pub fn prewarm(&self, gids: impl IntoIterator<Item = u16>, axes: &[(&str, f64)]) -> usize {
        let axes_shared = self.cache.intern_axes(&crate::daecore::cache::canonical_axes(axes));
        let mut added = 0;
        for gid in gids {
            let key = (gid, crate::sync::Shared::clone(&axes_shared));
            if crate::sync::write(&self.outlines).get(&key).is_some() { continue; }
            let mut path = crate::daecore::daetype::outline::Path::default();
            if self.outline_glyph_keyed(gid, &axes_shared, &mut path).is_none() { continue; }
            if path.is_empty() { continue; }
            path.shrink_to_fit();
            if crate::sync::write(&self.outlines).insert(key, crate::sync::Shared::new(path)) {
                added += 1;
            }
        }
        added
    }

    pub fn clear_prewarm(&self) {
        crate::sync::write(&self.outlines).clear();
    }

    // None where no hinter takes the glyph: HintMode::None, a px not finite and above zero or past 65,535
    // ppem, a glyph too large to hint there, or one no hinter reads, including bytecode that fails or turns
    // itself off at that size. Draw it unhinted. Points are 26.6 from the glyph's origin.
    pub fn hinted_glyph(
        &self,
        gid: u16,
        px: f32,
        axes: &[(&str, f64)],
        mode: HintMode,
    ) -> Option<crate::daecore::daetype::hinting::HintedOutline> {
        if !(px > 0.0 && px.is_finite()) {
            return None;
        }
        self.hinted_outline(gid, px, &crate::daecore::cache::canonical_axes(axes), mode)
    }

    // A CFF2 font's are its default instance's, as the CFF hinter reads them.
    pub fn cff_hints(&self, gid: u16) -> Option<crate::daecore::daetype::outline::CffHints> {
        let instanced;
        let cache: &FontCache = if self.is_cff2 {
            instanced = self.cache.instanced_font_cache_keyed(&crate::daecore::cache::canonical_axes::<&str>(&[]));
            &instanced
        } else {
            &self.cache
        };
        let cff = cache.cff()?;
        let outlines = cache.cff_outlines()?;
        let mut pen = crate::daecore::daetype::hinting::auto::CollectPen::new();
        crate::daecore::daetype::outline::outline_cff_glyph_hinted(&outlines, cff, gid, &mut pen).ok()
    }

    pub(crate) fn outline_glyph_keyed(
        &self,
        gid: u16,
        axes: &crate::daecore::cache::AxisKey,
        pen: &mut dyn crate::daecore::daetype::outline::OutlinePen,
    ) -> Option<()> {
        self.outline_glyph_drawn(gid, axes, pen).ok()
    }

    // The decoder's reason when a glyph does not draw. The pen may already hold part of it.
    pub(crate) fn outline_glyph_drawn(
        &self,
        gid: u16,
        axes: &crate::daecore::cache::AxisKey,
        pen: &mut dyn crate::daecore::daetype::outline::OutlinePen,
    ) -> Result<(), String> {
        // A prewarmed outline replays instead of decoding. Nothing is interned unless something was
        // prewarmed, so a caller who never prewarms pays one read of an empty cache.
        if crate::sync::read(&self.outlines).len() != 0
            && let Some(path) = self.prewarmed_outline(gid, &self.cache.intern_axes(axes))
        {
            path.replay(None, pen);
            return Ok(());
        }
        let location = self.cache.compute_location_keyed(axes);
        if !self.is_cff2 && location.iter().all(|&v| v == 0.0) {
            return draw_glyph_outline(&self.cache, gid, pen);
        }
        let instanced = self.cache.instanced_font_cache_keyed(axes);
        draw_glyph_outline(&instanced, gid, pen)
    }

    pub(super) fn hinted_outline(&self, gid: u16, px: f32, axes: &crate::daecore::cache::AxisKey, mode: HintMode)
        -> Option<crate::daecore::daetype::hinting::HintedOutline>
    {
        if mode == HintMode::None { return None; }
        let upm = self.upm();
        let size = crate::daecore::daetype::hinting::f26dot6::Size::from_px(px)?;

        let location = self.cache.compute_location_keyed(axes);
        let instanced;
        let cache: &FontCache = if !self.is_cff2 && location.iter().all(|&v| v == 0.0) {
            &self.cache
        } else {
            instanced = self.cache.instanced_font_cache_keyed(axes);
            &instanced
        };

        if mode != HintMode::AutoForce
            && let Some(glyf) = cache.table_map.get("glyf")
            && let Some(loca) = cache.loca_offsets()
            && let Some(out) = cache.hint_glyph_cached(glyf, &loca, gid, size, upm, mode)
        {
            return Some(out);
        }

        if !mode.may_autohint() || mode == HintMode::Auto && cache.glyph_programs_off(size, upm, mode) { return None; }

        if mode != HintMode::AutoForce
            && let Some(out) = Self::cff_hinted_outline(cache, gid, size, upm)
        {
            return Some(out);
        }

        let mut pen = crate::daecore::daetype::hinting::auto::CollectPen::new();
        self.outline_glyph_keyed(gid, axes, &mut pen)?;
        let pts = pen.finish();
        if pts.is_empty() || pts.len() > crate::daecore::daetype::hinting::MAX_POINTS || !scales_inside(&pts, size, upm) {
            return None;
        }
        if let Some(out) = cache.try_autohint(&pts, size) { return out; }
        self.ensure_autohinter(cache, axes);
        cache.try_autohint(&pts, size)?
    }

    fn cff_hinted_outline(cache: &FontCache, gid: u16, size: crate::daecore::daetype::hinting::f26dot6::Size, upm: u16)
        -> Option<crate::daecore::daetype::hinting::HintedOutline>
    {
        use crate::daecore::daetype::hinting::{auto::CollectPen, HintedOutline};

        let cff = cache.cff()?;
        let outlines = cache.cff_outlines()?;
        let mut pen = CollectPen::new();
        let hints =
            crate::daecore::daetype::outline::outline_cff_glyph_hinted(&outlines, cff, gid, &mut pen).ok()?;
        let pts = pen.finish();
        if pts.is_empty() || !scales_inside(&pts, size, upm) { return None; }

        let y = crate::daecore::daetype::hinting::cff::apply(&pts, &hints, size, upm)?;
        let x = pts.x.iter()
            .map(|&v| crate::daecore::daetype::hinting::f26dot6::scale_f32(v, size, upm))
            .collect();
        let flags = pts.flags.iter().map(|&f| crate::daecore::daetype::hinting::auto::hinted_flag(f)).collect();
        Some(HintedOutline { x, y, flags, contour_ends: pts.contour_ends })
    }

    fn ensure_autohinter(&self, cache: &FontCache, axes: &crate::daecore::cache::AxisKey) {
        use crate::daecore::daetype::hinting::auto::AutoHinter;
        let upm = self.upm();
        if let Some(zones) = cache.autohint_blues() {
            cache.set_autohinter(AutoHinter::from_zones(zones, upm));
            return;
        }
        let mut resolve = |c: char| cache.glyph_id(c as u32);
        let mut outline_of = |gid: u16| {
            let mut pen = crate::daecore::daetype::hinting::auto::CollectPen::new();
            self.outline_glyph_keyed(gid, axes, &mut pen)?;
            let pts = pen.finish();
            (!pts.is_empty()).then_some(pts)
        };
        let zones = AutoHinter::compute_zones(upm, &mut resolve, &mut outline_of);
        cache.set_autohint_blues(zones.clone());
        cache.set_autohinter(AutoHinter::from_zones(zones, upm));
    }
}

fn scales_inside(
    pts: &crate::daecore::daetype::hinting::auto::AutoPoints,
    size: crate::daecore::daetype::hinting::f26dot6::Size,
    upm: u16,
) -> bool {
    let coords = pts.x.iter().chain(&pts.y).map(|&v| f64::from(v));
    crate::daecore::daetype::hinting::f26dot6::fits(coords, size, upm)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daecore::daetype::outline::OutlinePen;

    // A planted outline no decode could produce, so this proves the lookup is wired rather than that a
    // replay draws what a decode draws.
    #[test]
    fn the_outline_path_serves_a_prewarmed_outline() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/test-fonts/inter/InterVariable.ttf");
        let f = Font::from_vec(std::fs::read(path).expect("fixture")).expect("parses");
        let gid = f.glyph_id('H' as u32).expect("H");
        let mut planted = Path::default();
        planted.move_to(1.0, 2.0);
        planted.line_to(30.0, 4.0);
        planted.line_to(5.0, 60.0);
        planted.close();
        let bold = [("wght", 700.0)];
        let key = (gid, f.cache.intern_axes(&crate::daecore::cache::canonical_axes(&bold)));
        crate::sync::write(&f.outlines).insert(key, crate::sync::Shared::new(planted.clone()));

        let mut got = Path::default();
        f.outline_glyph_instanced(gid, &bold, &mut got).expect("served");
        assert_eq!(got, planted, "the outline path decoded instead of replaying the prewarmed outline");
        let mut other = Path::default();
        f.outline_glyph_instanced(gid, &[("wght", 400.0)], &mut other).expect("decoded");
        assert_ne!(other, planted, "another location was served the planted outline");
        assert_eq!(f.glyph_quads(gid, &bold).map(|q| q.len()), Ok(3), "glyph_quads decoded past the cache");

        let key = (gid, f.cache.intern_axes(&crate::daecore::cache::canonical_axes::<&str>(&[])));
        crate::sync::write(&f.outlines).insert(key, crate::sync::Shared::new(planted.clone()));
        let mut plain = Path::default();
        f.outline_glyph(gid, &mut plain).expect("served");
        assert_eq!(plain, planted, "outline_glyph decoded instead of replaying the prewarmed outline");
    }
}
