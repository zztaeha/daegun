use super::*;

impl Font {
    pub fn glyph_ids(&self, text: &str) -> Vec<Option<u16>> {
        text.chars().map(|c| self.cache.glyph_id(c as u32)).collect()
    }

    pub fn glyph_id(&self, codepoint: u32) -> Option<u16> {
        self.cache.glyph_id(codepoint)
    }

    pub fn has_glyph(&self, codepoint: u32) -> bool {
        self.cache.glyph_id(codepoint).is_some()
    }

    pub fn coverage(&self) -> Vec<(u32, u16)> {
        const MAX_ENTRIES: usize = 200_000;
        self.cache
            .table_map
            .get("cmap")
            .and_then(|cmap| crate::daecore::daetype::subsetter::cmap_entries(cmap, MAX_ENTRIES))
            .map(|mut e| {
                e.retain(|&(_, g)| self.cache.glyph_in_range(g).is_some());
                e
            })
            .unwrap_or_default()
    }

    pub fn codepoints(&self) -> Vec<u32> {
        self.coverage().into_iter().map(|(c, _)| c).collect()
    }

    pub fn glyph_bounds(&self, gid: u16, axes: &[(&str, f64)]) -> Option<(f64, f64, f64, f64)> {
        let mut path = crate::daecore::daetype::outline::Path::default();
        self.outline_glyph_instanced(gid, axes, &mut path)?;
        let (x0, y0, x1, y1) = path.bounds()?;
        let s = self.cache.scale_factor();
        Some((x0 * s, y0 * s, x1 * s, y1 * s))
    }

    pub fn variation_glyph_id(&self, base: u32, selector: u32) -> Option<u16> {
        self.cache.variation_glyph_id(base, selector)
    }

    pub fn advance_widths(&self, gids: &[u16], axes: &[(&str, f64)]) -> Vec<f64> {
        let axis_values = owned_axes(axes);
        gids.iter().map(|&gid| self.cache.advance_width_rs(&axis_values, gid) as f64).collect()
    }

    pub fn vertical_advance(&self, gid: u16, axes: &[(&str, f64)]) -> u32 {
        // Bounded by the glyph count, not just by table length: `VORG` and a `cmap` both answer for
        // ids past `maxp` – a confident wrong number where every other method here declines.
        if gid >= self.num_glyphs() {
            return 0;
        }
        let stated = self.cache.vertical_advance_rs(&owned_axes(axes), gid);
        if stated != 0 {
            return stated;
        }
        let height = self.ascender() - self.descender();
        if height > 0 { height as u32 } else { 1000 }
    }

    pub fn vertical_origin(&self, gid: u16, axes: &[(&str, f64)]) -> Option<i32> {
        if gid >= self.num_glyphs() {
            return None;
        }
        self.cache.font_vertical_origin_rs(&owned_axes(axes), gid)
    }

    pub fn default_vertical_origin(&self) -> i32 {
        if self.cache.table_map.contains_key("glyf") { return 0; }
        crate::daecore::daetype::vorg::vorg_default_origin_y(&self.cache.table_map)
            .map_or(0, |v| (v as f64 * self.cache.scale_factor()).round() as i32)
    }

    // A ligature's carets in the 1000-unit em, x or y by direction: one for each the font lists, in its
    // increasing coordinate order, None where one cannot be resolved.
    pub fn ligature_carets(&self, gid: u16, axes: &[(&str, f64)], vertical: bool) -> Vec<Option<f64>> {
        let Some(gdef) = self.cache.table_map.get("GDEF") else { return Vec::new() };
        let owned = owned_axes(axes);
        let location = self.cache.compute_location_rs(&owned);
        // Format 2 names a point of the outline at the location.
        let points = || {
            let instanced = (location.iter().any(|&c| c != 0.0) && self.cache.table_map.contains_key("gvar"))
                .then(|| self.cache.instanced_font_cache(&owned));
            let source = instanced.as_deref().unwrap_or(&self.cache);
            let (glyf, loca) = (source.table_map.get("glyf")?, source.loca_offsets()?);
            crate::daecore::daetype::outline::glyph_points(glyf.as_slice(), loca.as_slice(), gid).ok()
        };
        let scale = self.cache.scale_factor();
        crate::daecore::daetype::lig_caret::ligature_carets(gdef, gid, points, &location, vertical, |at| self.cache.gdef_var_store(at))
            .into_iter()
            .map(|c| c.map(|v| v * scale))
            .collect()
    }

    pub fn caret_positions(&self, text: &str, axes: &[(&str, f64)], vertical: bool) -> Option<Vec<f64>> {
        let run = self.shape(text, axes, vertical)?;
        let n_chars = text.chars().count();
        let rtl = crate::text::shape::run_is_rtl(text, vertical);

        let mut glyph_x = Vec::with_capacity(run.glyphs.len() + 1);
        let mut x = 0.0;
        for a in &run.advances {
            glyph_x.push(x);
            x += *a;
        }
        glyph_x.push(x);
        let total = x;

        let mut out = alloc::vec![0.0f64; n_chars + 1];
        out[n_chars] = if rtl { 0.0 } else { total };

        let mut counts: alloc::collections::BTreeMap<usize, usize> = alloc::collections::BTreeMap::new();
        for &c in &run.clusters {
            *counts.entry(c as usize).or_insert(0) += 1;
        }

        for (i, &cluster) in run.clusters.iter().enumerate() {
            let first = cluster as usize;
            if first > n_chars { continue; }
            let covered = counts.get(&first).copied().unwrap_or(1).max(1);
            let next = counts.range(first + 1..).next().map(|(&k, _)| k).unwrap_or(n_chars);
            let span = next.saturating_sub(first);
            let (left, right) = (glyph_x[i], glyph_x[i + 1]);
            out[first] = if rtl { right } else { left };
            if span <= 1 || covered > 1 {
                continue;
            }

            let gid = run.glyphs[i];
            let carets = self.ligature_carets(gid, axes, vertical);
            let top = if vertical { self.vertical_origin(gid, axes).map_or(0.0, f64::from) } else { 0.0 };
            let edges = Edges { left, right, top, rtl, vertical };
            for (k, at) in (1..span).zip(first + 1..=n_chars) {
                out[at] = edges.boundary(k, span, &carets);
            }
        }

        Some(out)
    }

    pub fn glyph_class(&self, gid: u16) -> Option<GlyphClass> {
        self.cache.glyph_in_range(gid)?;
        let gdef = self.cache.table_map.get("GDEF")?;
        match crate::daecore::daetype::subsetter::otl::gdef::glyph_class(gdef, gid) {
            1 => Some(GlyphClass::Base),
            2 => Some(GlyphClass::Ligature),
            3 => Some(GlyphClass::Mark),
            4 => Some(GlyphClass::Component),
            _ => None,
        }
    }

    pub fn mark_attachment_class(&self, gid: u16) -> u16 {
        if self.cache.glyph_in_range(gid).is_none() {
            return 0;
        }
        self.cache
            .table_map
            .get("GDEF")
            .map_or(0, |gdef| crate::daecore::daetype::subsetter::otl::gdef::mark_attach_class(gdef, gid))
    }

    pub fn glyph_name(&self, gid: u16) -> Option<String> {
        crate::daecore::daetype::glyph_names::glyph_name(
            self.cache.table_map.get("post").map(|t| t.as_slice()),
            self.cache.cff().map(|t| t.as_slice()),
            self.num_glyphs(),
            gid,
        )
    }

    pub fn glyph_names(&self) -> Vec<Option<String>> {
        crate::daecore::daetype::glyph_names::glyph_names(
            self.cache.table_map.get("post").map(|t| t.as_slice()),
            self.cache.cff().map(|t| t.as_slice()),
            self.num_glyphs(),
        )
    }
}

// A ligature glyph's place in a run, for finding where its components meet.
struct Edges {
    left: f64,
    right: f64,
    top: f64,
    rtl: bool,
    vertical: bool,
}

impl Edges {
    // Where the k-th of `span` components ends: carets run up the glyph, so right to left or down a
    // line that is the k-th caret from the end. A missing caret falls evenly between the edges.
    fn boundary(&self, k: usize, span: usize, carets: &[Option<f64>]) -> f64 {
        let index = if self.rtl || self.vertical { carets.len().checked_sub(k) } else { k.checked_sub(1) };
        let even = (self.right - self.left) * k as f64 / span as f64;
        match index.and_then(|j| carets.get(j).copied().flatten()) {
            Some(c) if self.vertical => self.left + self.top - c,
            Some(c) => self.left + c,
            None if self.rtl => self.right - even,
            None => self.left + even,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Edges;

    fn boundaries(edges: &Edges, carets: &[Option<f64>]) -> [f64; 2] {
        [1, 2].map(|k| edges.boundary(k, 3, carets))
    }

    const HORIZONTAL: Edges = Edges { left: 100.0, right: 1000.0, top: 0.0, rtl: false, vertical: false };

    #[test]
    fn left_to_right_components_end_at_the_carets_in_order() {
        assert_eq!(boundaries(&HORIZONTAL, &[Some(300.0), Some(600.0)]), [400.0, 700.0]);
    }

    // The first component of right-to-left text is the glyph's rightmost, so it ends at the last caret,
    // measured from the left edge as every caret is. Read from the right edge, both would come out mirrored.
    #[test]
    fn right_to_left_components_end_at_the_carets_from_the_right() {
        let rtl = Edges { rtl: true, ..HORIZONTAL };
        assert_eq!(boundaries(&rtl, &[Some(300.0), Some(600.0)]), [700.0, 400.0]);
    }

    // Down a vertical line from the glyph's top at 880 above its origin, the first boundary is the
    // highest caret.
    #[test]
    fn vertical_components_end_down_from_the_top() {
        let vertical = Edges { top: 880.0, vertical: true, ..HORIZONTAL };
        assert_eq!(boundaries(&vertical, &[Some(300.0), Some(600.0)]), [380.0, 680.0]);
    }

    // A caret that cannot be resolved falls evenly between the edges; the rest keep their places.
    #[test]
    fn an_unresolved_caret_falls_evenly_and_the_rest_stay() {
        assert_eq!(boundaries(&HORIZONTAL, &[None, Some(600.0)]), [400.0, 700.0]);
        assert_eq!(boundaries(&HORIZONTAL, &[Some(250.0), None]), [350.0, 700.0]);
    }
}
