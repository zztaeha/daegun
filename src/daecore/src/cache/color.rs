use super::*;

impl FontCache {
    fn colr_v1_var_data(&self) -> Option<Shared<crate::daecore::daetype::colr_v1::ColrV1VarData>> {
        {
            let cache = read(&self.colr_v1_var_data);
            if let Some(data) = cache.as_ref() { return Some(Shared::clone(data)); }
        }
        let colr = self.table_map.get("COLR")?;
        let data = Shared::new(crate::daecore::daetype::colr_v1::parse_colr_v1_var_data(colr));
        *write(&self.colr_v1_var_data) = Some(Shared::clone(&data));
        Some(data)
    }

    fn colr_v1_scalars(
        &self,
        var_data: &crate::daecore::daetype::colr_v1::ColrV1VarData,
        location: &[f64],
    ) -> Shared<Vec<f64>> {
        {
            let cache = read(&self.colr_v1_scalars);
            if let Some((at, scalars)) = cache.as_ref()
                && at.as_slice() == location
            {
                return Shared::clone(scalars);
            }
        }
        let scalars = Shared::new(crate::daecore::daetype::colr_v1::colr_v1_region_scalars(var_data, location));
        *write(&self.colr_v1_scalars) = Some((location.to_vec(), Shared::clone(&scalars)));
        scalars
    }

    pub fn colr_v1_paint(&self, gid: u16, location: &[f64], palette_index: u16) -> Option<crate::daecore::daetype::colr_v1::Paint> {
        let var_data = self.colr_v1_var_data()?;
        let scalars = self.colr_v1_scalars(&var_data, location);
        crate::daecore::daetype::colr_v1::colr_v1_paint_graph_with_scalars(&self.table_map, gid, palette_index, &var_data, &scalars)
    }

    pub fn colr_clip_box(&self, gid: u16, location: &[f64]) -> Option<[i32; 4]> {
        let colr = self.table_map.get("COLR")?;
        let var_data = self.colr_v1_var_data()?;
        let scalars = self.colr_v1_scalars(&var_data, location);
        crate::daecore::daetype::colr_v1::clip_box(colr, gid, &var_data, &scalars)
    }
}
