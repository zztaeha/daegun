use super::*;

impl FontCache {
    // Double-checked: `build` is the caller's closure and must not run under the guard, which would be a
    // deadlock under `threading` and a borrow panic under `RefCell`. Racing callers may both build.
    pub(crate) fn subtable_indexes_cached(
        &self,
        table_index: usize,
        lookup_count: usize,
        index: u16,
        build: impl FnOnce() -> Vec<crate::daecore::daeshaper::ot::SubtableIndex>,
    ) -> Option<Shared<Vec<crate::daecore::daeshaper::ot::SubtableIndex>>> {
        {
            let slots = self.subtable_indexes.get(table_index)?;
            let mut slots = write(slots);
            if slots.len() != lookup_count {
                slots.clear();
                slots.resize_with(lookup_count, || None);
            }
            if let Some(Some(hit)) = slots.get(index as usize) {
                return Some(Shared::clone(hit));
            }
        }

        let built = Shared::new(build());

        let slots = self.subtable_indexes.get(table_index)?;
        let mut slots = write(slots);
        let slot = slots.get_mut(index as usize)?;
        Some(Shared::clone(slot.get_or_insert(built)))
    }

    pub(crate) fn build_index(
        &self,
        entries: &[(u32, u16)],
        absent: u16,
    ) -> Option<crate::daecore::daetype::format::index::SparseIndex> {
        let left = self.index_budget.get();
        let index = crate::daecore::daetype::format::index::SparseIndex::build(entries, absent, left)?;
        self.index_budget.set(left.saturating_sub(index.bytes()));
        Some(index)
    }

    #[allow(clippy::type_complexity, reason = "two optional indexes, named at the one call site")]
    pub(crate) fn gdef_class_indexes(
        &self,
        entries: impl FnOnce() -> (Option<Vec<(u32, u16)>>, Option<Vec<(u32, u16)>>),
    ) -> (
        Option<Shared<crate::daecore::daetype::format::index::SparseIndex>>,
        Option<Shared<crate::daecore::daetype::format::index::SparseIndex>>,
    ) {
        {
            let g = read(&self.gdef_class_indexes);
            if g.built {
                return (g.glyph_classes.clone(), g.mark_attach.clone());
            }
        }

        let (classes, marks) = entries();
        let build = |e: Option<Vec<(u32, u16)>>| {
            e.and_then(|e| self.build_index(&e, 0)).map(Shared::new)
        };
        let built = (build(classes), build(marks));

        let mut g = write(&self.gdef_class_indexes);
        if !g.built {
            g.built = true;
            g.glyph_classes = built.0;
            g.mark_attach = built.1;
        }
        (g.glyph_classes.clone(), g.mark_attach.clone())
    }

    pub(crate) fn lookup_digest_cached(
        &self,
        table_index: usize,
        lookup_count: usize,
        index: u16,
        compute: impl FnOnce() -> crate::daecore::daeshaper::ot::digest::Digest,
    ) -> crate::daecore::daeshaper::ot::digest::Digest {
        let Some(slots) = self.lookup_digests.get(table_index) else { return compute() };
        {
            let mut slots = write(slots);
            if slots.len() != lookup_count {
                slots.clear();
                slots.resize(lookup_count, None);
            }
            if let Some(digest) = slots.get(index as usize).copied().flatten() {
                return digest;
            }
        }

        let digest = compute();

        let mut slots = write(slots);
        if let Some(slot) = slots.get_mut(index as usize) {
            *slot = Some(digest);
        }
        digest
    }

    pub fn shaped_run<S: AsRef<str>>(&self, axis_values: &[(S, f64)], text: &str, vertical: bool) -> Option<Shared<crate::daecore::text::shape::ShapedRun>> {
        let ctx = crate::daecore::cache::RunContext::default();
        self.shaped_run_in_context(axis_values, text, vertical, &ctx)
    }

    pub fn shaped_run_directional<S: AsRef<str>>(&self, axis_values: &[(S, f64)], text: &str, vertical: bool, rtl: bool) -> Option<Shared<crate::daecore::text::shape::ShapedRun>> {
        let ctx = crate::daecore::cache::RunContext { rtl: Some(rtl), ..Default::default() };
        self.shaped_run_in_context(axis_values, text, vertical, &ctx)
    }

    pub fn shaped_run_in_context<S: AsRef<str>>(
        &self, axis_values: &[(S, f64)], text: &str, vertical: bool,
        ctx: &crate::daecore::cache::RunContext,
    ) -> Option<Shared<crate::daecore::text::shape::ShapedRun>> {
        use crate::daecore::daeshaper::buffer::Buffer;
        let n = Buffer::CONTEXT_LENGTH;
        let pre: String = {
            let c: Vec<char> = ctx.before.chars().collect();
            c[c.len().saturating_sub(n)..].iter().collect()
        };
        let post: String = ctx.after.chars().take(n).collect();

        // Built by moving rather than cloning, and read back out of the key afterwards: a cache hit
        // would otherwise pay for an owned copy of everything it is only looking up.
        let key = (
            canonical_axes(axis_values), text.to_string(), vertical, ctx.rtl, pre, post,
            ctx.script.map(String::from), ctx.language.map(String::from),
            ctx.seed_script.map(|s| s.0),
        );
        {
            let cache = read(&self.shape_cache);
            if let Some(hit) = cache.runs.get(&key) {
                return Some(Shared::clone(hit));
            }
        }
        let (axes, pre, post) = (&key.0, &key.4, &key.5);
        let opts = crate::daecore::text::shape::ShapeOptions {
            before: pre,
            after: post,
            script: ctx.script,
            language: ctx.language,
            seed_script: ctx.seed_script,
            ..Default::default()
        };
        let run = Shared::new(crate::daecore::text::shape::shape_run_stated_with_options(
            self, axes, text, vertical, ctx.rtl, &opts,
        )?);
        let (cost, budget) = (ShapeCache::cost(&key, &run), self.shape_budget.get());
        if cost <= SHAPE_CACHE_ENTRY_MAX && cost <= budget {
            let mut cache = write(&self.shape_cache);
            if cache.bytes + cost > budget {
                cache.runs.clear();
                cache.bytes = 0;
            }
            if let Some(old) = cache.runs.insert(key.clone(), Shared::clone(&run)) {
                cache.bytes = cache.bytes.saturating_sub(ShapeCache::cost(&key, &old));
            }
            cache.bytes += cost;
        }
        Some(run)
    }

    pub fn shaped_run_justified(
        &self, axis_values: &[(String, f64)], text: &str, vertical: bool,
        mods: &crate::daecore::daetype::jstf::JstfModLists, shrink: bool,
    ) -> Option<crate::daecore::text::shape::ShapedRun> {
        crate::daecore::text::shape::shape_run_justified(self, axis_values, text, vertical, mods, shrink)
    }

    pub(crate) fn cmap_index(&self) -> Option<Shared<crate::daecore::daetype::format::index::SparseIndex>> {
        read(&self.cmap).index.clone().flatten()
    }

    pub fn glyph_id(&self, codepoint: u32) -> Option<u16> {
        let cmap = self.table_map.get("cmap")?;
        let mut state = write(&self.cmap);
        if let Some(Some(index)) = &state.index {
            return Some(index.lookup(codepoint)).filter(|&g| g != 0);
        }
        let gid = state.lookup.get_or_insert_with(|| crate::daecore::daetype::subsetter::CmapLookup::new(cmap)).glyph_id(cmap, codepoint)?;
        drop(state);
        self.glyph_in_range(gid)
    }

    pub(crate) fn warm_cmap_index(&self) {
        if read(&self.cmap).index.is_some() {
            return;
        }
        let built = self.table_map.get("cmap").and_then(|cmap| {
            let mut entries = crate::daecore::daetype::subsetter::cmap_entries(cmap, CMAP_INDEX_MAX_ENTRIES)?;
            entries.retain(|&(_, g)| self.glyph_in_range(g).is_some());
            self.build_index(&entries, 0).map(Shared::new)
        });
        write(&self.cmap).index = Some(built);
    }

    pub fn variation_glyph_id(&self, base: u32, selector: u32) -> Option<u16> {
        let cmap = self.table_map.get("cmap")?;
        match crate::daecore::daetype::subsetter::cmap_variation_glyph_id(cmap, base, selector)? {
            crate::daecore::daetype::subsetter::UvsLookup::Explicit(gid) => self.glyph_in_range(gid),
            crate::daecore::daetype::subsetter::UvsLookup::UseDefault => self.glyph_id(base),
        }
    }
}

fn plan_key(
    script: Option<crate::daecore::daeshaper::unicode::Script>,
    direction: crate::daecore::daeshaper::buffer::Direction,
    script_tags: &[crate::daecore::daeshaper::ot::tag::Tag],
    language_tags: &[crate::daecore::daeshaper::ot::tag::Tag],
    user_features: &[crate::daecore::daeshaper::plan::UserFeature],
    lookup_overrides: &[crate::daecore::daeshaper::ot::map::LookupOverride],
    coords: &[i32],
) -> Vec<u8> {
    use crate::daecore::daeshaper::buffer::Direction;
    use crate::daecore::daeshaper::ot::map::TableIndex;

    let mut k = Vec::with_capacity(64);
    match script {
        Some(s) => { k.push(1); k.extend_from_slice(&s.0.to_le_bytes()); }
        None => k.push(0),
    }
    k.push(match direction {
        Direction::LeftToRight => 0,
        Direction::RightToLeft => 1,
        Direction::TopToBottom => 2,
        Direction::BottomToTop => 3,
    });
    for tags in [script_tags, language_tags] {
        k.extend_from_slice(&(tags.len() as u32).to_le_bytes());
        for t in tags {
            k.extend_from_slice(&t.0.to_le_bytes());
        }
    }
    k.extend_from_slice(&(user_features.len() as u32).to_le_bytes());
    for f in user_features {
        k.extend_from_slice(&f.tag.0.to_le_bytes());
        k.extend_from_slice(&f.value.to_le_bytes());
        k.extend_from_slice(&f.start.to_le_bytes());
        k.extend_from_slice(&f.end.to_le_bytes());
    }
    k.extend_from_slice(&(lookup_overrides.len() as u32).to_le_bytes());
    for o in lookup_overrides {
        k.push(match o.table { TableIndex::Gsub => 0, TableIndex::Gpos => 1 });
        k.extend_from_slice(&o.index.to_le_bytes());
        k.push(u8::from(o.enable));
    }
    k.extend_from_slice(&(coords.len() as u32).to_le_bytes());
    for c in coords {
        k.extend_from_slice(&c.to_le_bytes());
    }
    k
}

impl FontCache {
    #[allow(clippy::too_many_arguments, reason = "everything a plan is keyed on, and no fewer")]
    pub(crate) fn shape_plan_cached(
        &self,
        script: Option<crate::daecore::daeshaper::unicode::Script>,
        face: &crate::daecore::daeshaper::face::Face,
        direction: crate::daecore::daeshaper::buffer::Direction,
        script_tags: &[crate::daecore::daeshaper::ot::tag::Tag],
        language_tags: &[crate::daecore::daeshaper::ot::tag::Tag],
        user_features: &[crate::daecore::daeshaper::plan::UserFeature],
        lookup_overrides: &[crate::daecore::daeshaper::ot::map::LookupOverride],
        coords: &[i32],
    ) -> Shared<crate::daecore::daeshaper::plan::ShapePlan> {
        let key = plan_key(script, direction, script_tags, language_tags, user_features,
                           lookup_overrides, coords);
        {
            let cache = read(&self.plan_cache);
            if let Some(hit) = cache.get(&key) {
                return Shared::clone(hit);
            }
        }

        let plan = Shared::new(crate::daecore::daeshaper::plan::ShapePlan::with_script(
            script, face, direction, script_tags, language_tags, user_features, lookup_overrides,
            coords,
        ));

        let mut cache = write(&self.plan_cache);
        if cache.len() >= PLAN_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, Shared::clone(&plan));
        plan
    }
}

const PLAN_CACHE_MAX: usize = 64;

impl FontCache {
    pub(crate) fn take_buffer(&self) -> crate::daecore::daeshaper::buffer::Buffer {
        write(&self.spare_buffer).take().unwrap_or_default()
    }

    pub(crate) fn give_buffer(&self, mut buf: crate::daecore::daeshaper::buffer::Buffer) {
        buf.reset();
        *write(&self.spare_buffer) = Some(buf);
    }
}
