//! A map, a start position and the unit data: everything a run needs.

use std::path::Path;

use anyhow::{Context, Result};

use crate::catalog::Catalog;
use crate::plan::{Base, Spot};
use crate::sim::{Order, Pos, Rules, Sim};

#[derive(Debug, Clone)]
pub struct Scenario {
    pub catalog: Catalog,
    pub rules: Rules,
    pub map: bar_data::MapSpec,
    pub start: Pos,
    pub base: Base,
    /// Commit of the BAR checkout, if known.
    pub bar_commit: Option<String>,
}

impl Scenario {
    /// Loads unit defs from `bar_root` and the map spec from `map_file`.
    /// The wind turbine output is BAR's average for the map's wind range.
    pub fn load(
        bar_root: &Path,
        map_file: &Path,
        start_id: &str,
        rules: Rules,
    ) -> Result<Scenario> {
        let defs = bar_data::UnitDefs::load(bar_root)?;
        let catalog = Catalog::from_unit_defs(&defs)?;
        let map = bar_data::MapSpec::load(map_file)?;
        let wind_src = std::fs::read_to_string(bar_root.join("common/wind_functions.lua"))
            .context("read common/wind_functions.lua")?;
        let wind = map.average_wind(&wind_src, catalog.wind.wind_generator as f64)? as f32;
        let mut s = Scenario::from_parts(catalog, Rules { wind, ..rules }, map, start_id)?;
        s.bar_commit = defs.commit.clone();
        Ok(s)
    }

    /// A scenario from loaded parts. `rules.wind` is used as given.
    pub fn from_parts(
        catalog: Catalog,
        rules: Rules,
        map: bar_data::MapSpec,
        start_id: &str,
    ) -> Result<Scenario> {
        let sp = map.start(start_id).with_context(|| {
            let ids: Vec<&str> = map.start_positions.iter().map(|p| p.id.as_str()).collect();
            format!(
                "map {} has no start position {start_id} (has {})",
                map.name,
                ids.join(", ")
            )
        })?;
        let start = Pos::new(sp.x as f32, sp.z as f32);
        let spots: Vec<Spot> = map
            .metal_spots
            .iter()
            .map(|s| Spot {
                pos: Pos::new(s.x as f32, s.z as f32),
                metal: s.metal as f32,
            })
            .collect();
        let base = Base::new(
            start,
            Pos::new(map.size.x as f32, map.size.z as f32),
            &spots,
        );
        Ok(Scenario {
            catalog,
            rules,
            map,
            start,
            base,
            bar_commit: None,
        })
    }

    /// A fresh game with these commander orders.
    pub fn sim(&self, orders: Vec<Order>) -> Sim {
        Sim::new(self.catalog.clone(), self.rules.clone(), self.start, orders)
    }
}
