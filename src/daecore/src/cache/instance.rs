use super::*;

impl FontCache {
    pub fn get_or_instance(&self, axis_values: &[(String, f64)]) -> Shared<Vec<u8>> {
        let key = canonical_axes(axis_values);
        {
            let cache = read(&self.instance_cache);
            if let Some(ttf) = cache.get(&key) {
                return Shared::clone(ttf);
            }
        }
        let ttf = Shared::new(crate::daecore::daetype::instancer::instance_font_from_map(&self.table_map, &key)
            .unwrap_or_else(|_| crate::daecore::daetype::decoder::build_ttf(&self.table_map)));
        // The newest instance stays whatever the budget: kept nowhere, every call at that location would
        // instance the font again. Past the budget, it is the only one kept.
        let cost = ttf.len();
        let mut cache = write(&self.instance_cache);
        if self.instance_cache_bytes.get().saturating_add(cost) > self.instance_budget.get() {
            cache.clear();
            self.instance_cache_bytes.set(0);
        }
        if let Some(old) = cache.insert(key, Shared::clone(&ttf)) {
            self.instance_cache_bytes
                .set(self.instance_cache_bytes.get().saturating_sub(old.len()));
        }
        self.instance_cache_bytes.set(self.instance_cache_bytes.get().saturating_add(cost));
        ttf
    }

    pub fn instanced_font_cache(&self, axis_values: &[(String, f64)]) -> Shared<FontCache> {
        self.instanced_font_cache_keyed(&canonical_axes(axis_values))
    }

    pub fn instanced_font_cache_keyed(&self, key: &AxisKey) -> Shared<FontCache> {
        {
            let cache = read(&self.instanced_cache);
            if let Some(fc) = cache.get(key) { return Shared::clone(fc); }
        }
        let map = crate::daecore::daetype::instancer::instance_tables_for_drawing(&self.table_map, key)
            .map(|tables| {
                tables.into_iter().map(|(tag, data)| (tag, TableBytes::from_vec(data.into_owned()))).collect()
            })
            .unwrap_or_else(|_| self.table_map.clone());
        let fc = Shared::new(FontCache::new(map));
        // Kept whatever the budget, as the instance above is.
        let cost = fc.retained_bytes();
        let mut cache = write(&self.instanced_cache);
        if self.instanced_cache_bytes.get().saturating_add(cost) > self.instance_budget.get() {
            cache.clear();
            self.instanced_cache_bytes.set(0);
        }
        if let Some(old) = cache.insert(key.clone(), Shared::clone(&fc)) {
            self.instanced_cache_bytes
                .set(self.instanced_cache_bytes.get().saturating_sub(old.retained_bytes()));
        }
        self.instanced_cache_bytes.set(self.instanced_cache_bytes.get().saturating_add(cost));
        fc
    }

    pub fn compute_location_rs(&self, axis_values: &[(String, f64)]) -> Shared<Vec<f64>> {
        self.compute_location_keyed(&canonical_axes(axis_values))
    }

    pub fn intern_axes(&self, axes: &AxisKey) -> Shared<AxisKey> {
        if axes.is_empty() {
            return Shared::clone(&self.default_axes);
        }
        {
            let cache = read(&self.axis_intern);
            if let Some(v) = cache.get(axes) { return Shared::clone(v); }
        }
        let shared = Shared::new(axes.clone());
        let mut cache = write(&self.axis_intern);
        if cache.len() >= LOCATION_BY_AXIS_CAP { cache.clear(); }
        cache.insert(axes.clone(), Shared::clone(&shared));
        shared
    }

    pub fn compute_location_keyed(&self, axes: &AxisKey) -> Shared<Vec<f64>> {
        {
            let cache = read(&self.location_by_axis);
            if let Some(v) = cache.get(axes) { return Shared::clone(v); }
        }
        let location = Shared::new(crate::daecore::daetype::instancer::compute_location(&self.table_map, axes).unwrap_or_default());
        let mut cache = write(&self.location_by_axis);
        if cache.len() >= LOCATION_BY_AXIS_CAP { cache.clear(); }
        cache.insert(axes.clone(), Shared::clone(&location));
        location
    }
}
