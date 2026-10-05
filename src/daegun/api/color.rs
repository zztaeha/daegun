use super::*;

// What a COLR layer resolves to when it defers to the caller's text color and the caller did not
// name one. Opaque, because transparent would drop the layer silently.
pub(crate) const FOREGROUND: crate::paint::Rgba =
    crate::paint::Rgba { r: 0, g: 0, b: 0, a: 255 };

// A scene refused for its size, which C answers apart from a glyph with no scene.
pub(crate) struct TooManyPoints;

impl Font {
    pub fn glyph_bitmap(&self, gid: u16, target_ppem: u16) -> Option<GlyphBitmap> {
        crate::daecore::daetype::bitmap::glyph_bitmap(&self.cache.table_map, gid, target_ppem)
    }

    pub fn colr_layers(&self, gid: u16) -> Option<Vec<ColrLayer>> {
        crate::daecore::daetype::colr_v0::colr_layers(&self.cache.table_map, gid)
    }

    pub fn colr_layers_for_palette(&self, gid: u16, palette_index: u16) -> Option<Vec<ColrLayer>> {
        crate::daecore::daetype::colr_v0::colr_layers_for_palette(&self.cache.table_map, gid, palette_index)
    }

    pub fn palette_count(&self) -> u16 {
        crate::daecore::daetype::colr_v0::cpal_palette_count(&self.cache.table_map)
    }

    pub fn palette_info(&self) -> Vec<PaletteInfo> {
        crate::daecore::daetype::colr_v0::cpal_palette_info(&self.cache.table_map)
    }

    // Each palette entry's name ID ('name' table), so a color picker can label the colors a palette
    // sets: None for an entry the font leaves unnamed, and an empty list when it names none.
    pub fn palette_entry_labels(&self) -> Vec<Option<u16>> {
        crate::daecore::daetype::colr_v0::cpal_palette_entry_labels(&self.cache.table_map)
    }

    pub fn colr_scene(
        &self,
        gid: u16,
        axes: &[(&str, f64)],
        palette_index: u16,
    ) -> Option<crate::paint::DisplayList> {
        self.colr_scene_with(gid, axes, palette_index, FOREGROUND)
    }

    // Font units, y up, each path held once and placed by its op's transform; at most 128 levels of
    // clips and layers, a v1 glyph inside its clip box. None past MAX_FLATTEN_POINTS points in all.
    pub fn colr_scene_with(
        &self,
        gid: u16,
        axes: &[(&str, f64)],
        palette_index: u16,
        foreground: crate::paint::Rgba,
    ) -> Option<crate::paint::DisplayList> {
        self.scene(gid, axes, palette_index, foreground).ok().flatten()
    }

    // Each glyph's outline is held once, and past MAX_FLATTEN_POINTS points in all the scene is refused:
    // layers name glyphs freely, so a small font could otherwise make one scene hold gigabytes.
    pub(crate) fn scene(
        &self,
        gid: u16,
        axes: &[(&str, f64)],
        palette_index: u16,
        foreground: crate::paint::Rgba,
    ) -> Result<Option<crate::paint::DisplayList>, TooManyPoints> {
        let mut out = crate::paint::DisplayList::default();
        let (mut held, mut over) = (0usize, false);
        let mut outline = |g: u16| {
            if over {
                return None;
            }
            let mut p = crate::daecore::daetype::outline::Path::default();
            self.outline_glyph_instanced(g, axes, &mut p)?;
            held += p.parts().1.len();
            over = held > crate::MAX_FLATTEN_POINTS;
            (!over && !p.is_empty()).then_some(p)
        };
        // A version 1 glyph the spec says must not be drawn, unbounded with no clip box, leaves the
        // version 0 layers, if the font has them, to draw instead.
        let drawn = self.colr_v1_paint(gid, axes, palette_index).is_some_and(|paint| {
            let clip = self.colr_clip_box(gid, axes);
            crate::paint::colr::lower(&paint, crate::paint::IDENTITY, clip, &mut outline, foreground, &mut out)
        });
        if !drawn {
            let Some(layers) = self.colr_layers_for_palette(gid, palette_index) else { return Ok(None) };
            let mut paths = alloc::collections::BTreeMap::new();
            // Back to front, which is the order COLR v0 records them in.
            for (layer_gid, r, g, b, a, is_foreground) in layers {
                let path =
                    *paths.entry(layer_gid).or_insert_with(|| outline(layer_gid).map(|p| out.push_path(p)));
                let Some(path) = path else { continue };
                let color =
                    if is_foreground { foreground } else { crate::paint::Rgba { r, g, b, a } };
                out.push(crate::paint::Op::Fill {
                    path,
                    paint: crate::paint::Paint::Solid(color),
                    rule: crate::daecore::daetype::outline::FillRule::NonZero,
                    transform: crate::paint::IDENTITY,
                });
            }
        }
        if over {
            return Err(TooManyPoints);
        }
        Ok((!out.is_empty()).then_some(out))
    }

    // A COLR version 1 glyph's clip box, (x_min, y_min, x_max, y_max) in font units: what it draws
    // stays inside. A variable box is rounded outward at the location.
    pub fn colr_clip_box(&self, gid: u16, axes: &[(&str, f64)]) -> Option<[i32; 4]> {
        let location = self.cache.compute_location_rs(&owned_axes(axes));
        self.cache.colr_clip_box(gid, &location)
    }

    pub fn colr_v1_paint(&self, gid: u16, axes: &[(&str, f64)], palette_index: u16) -> Option<Paint> {
        let location = self.cache.compute_location_rs(&owned_axes(axes));
        self.cache.colr_v1_paint(gid, &location, palette_index)
    }
}
