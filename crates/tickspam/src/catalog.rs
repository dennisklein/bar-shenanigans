//! The unit stats that the tick spam simulation needs.
//!
//! The values come from the BAR unit defs (see [`Catalog::from_unit_defs`]).
//! Tests use [`Catalog::reference`], a copy of the same values.

use serde::{Deserialize, Serialize};

/// What a unit does in the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Commander,
    Lab,
    Nano,
    Mex,
    Wind,
    Solar,
    Tick,
}

impl Role {
    pub const ALL: [Role; 7] = [Role::Commander, Role::Lab, Role::Nano, Role::Mex, Role::Wind, Role::Solar, Role::Tick];
}

/// Engine-relevant values of one unit def.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stats {
    pub name: String,
    pub human_name: String,
    pub metal_cost: f32,
    pub energy_cost: f32,
    pub build_time: f32,
    /// Build power per second.
    pub worker_time: f32,
    pub metal_make: f32,
    pub energy_make: f32,
    /// Positive: energy use per second while active (a mex stops without it).
    /// Negative: energy production per second (BAR solars).
    pub energy_upkeep: f32,
    pub wind_generator: f32,
    pub extracts_metal: f32,
    pub metal_storage: f32,
    pub energy_storage: f32,
    pub build_distance: f32,
    /// Elmos per second.
    pub speed: f32,
    pub footprint_x: u32,
    pub footprint_z: u32,
}

impl Stats {
    /// Approximate unit radius from the footprint, used for build range.
    /// The engine uses the model radius, which is close to this value.
    pub fn radius(&self) -> f32 {
        let hx = self.footprint_x as f32 * 4.0;
        let hz = self.footprint_z as f32 * 4.0;
        hx.hypot(hz)
    }
}

/// The unit defs of one faction that tick spam uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    pub commander: Stats,
    pub lab: Stats,
    pub nano: Stats,
    pub mex: Stats,
    pub wind: Stats,
    pub solar: Stats,
    pub tick: Stats,
}

/// BAR unit def names of the Armada units in the catalog.
pub const ARM_NAMES: [(Role, &str); 7] = [
    (Role::Commander, "armcom"),
    (Role::Lab, "armlab"),
    (Role::Nano, "armnanotc"),
    (Role::Mex, "armmex"),
    (Role::Wind, "armwin"),
    (Role::Solar, "armsolar"),
    (Role::Tick, "armflea"),
];

impl Catalog {
    pub fn get(&self, role: Role) -> &Stats {
        match role {
            Role::Commander => &self.commander,
            Role::Lab => &self.lab,
            Role::Nano => &self.nano,
            Role::Mex => &self.mex,
            Role::Wind => &self.wind,
            Role::Solar => &self.solar,
            Role::Tick => &self.tick,
        }
    }

    /// Builds the Armada catalog from unit defs that the BAR Lua loader read.
    pub fn from_unit_defs(defs: &bar_data::UnitDefs) -> anyhow::Result<Catalog> {
        let get = |name: &str| -> anyhow::Result<Stats> {
            let d = defs.get(name).ok_or_else(|| anyhow::anyhow!("unit def {name} not found"))?;
            Ok(Stats {
                name: d.name.clone(),
                human_name: d.human_name.clone(),
                metal_cost: d.metal_cost as f32,
                energy_cost: d.energy_cost as f32,
                build_time: d.build_time as f32,
                worker_time: d.worker_time as f32,
                metal_make: d.metal_make as f32,
                energy_make: d.energy_make as f32,
                energy_upkeep: d.energy_upkeep as f32,
                wind_generator: d.wind_generator as f32,
                extracts_metal: d.extracts_metal as f32,
                metal_storage: d.metal_storage as f32,
                energy_storage: d.energy_storage as f32,
                build_distance: d.build_distance as f32,
                speed: d.speed as f32,
                footprint_x: d.footprint_x,
                footprint_z: d.footprint_z,
            })
        };
        let catalog = Catalog {
            commander: get("armcom")?,
            lab: get("armlab")?,
            nano: get("armnanotc")?,
            mex: get("armmex")?,
            wind: get("armwin")?,
            solar: get("armsolar")?,
            tick: get("armflea")?,
        };
        anyhow::ensure!(
            defs.get("armlab").is_some_and(|d| d.build_options.iter().any(|o| o == "armflea")),
            "armlab cannot build armflea"
        );
        Ok(catalog)
    }

    /// Armada values from the BAR unit defs, for tests and for runs without a
    /// BAR checkout. [`Catalog::from_unit_defs`] is the source of truth.
    pub fn reference() -> Catalog {
        #[allow(clippy::too_many_arguments)]
        fn s(name: &str, human: &str, m: f32, e: f32, bt: f32, wt: f32, fp: u32, f: impl FnOnce(&mut Stats)) -> Stats {
            let mut st = Stats {
                name: name.into(),
                human_name: human.into(),
                metal_cost: m,
                energy_cost: e,
                build_time: bt,
                worker_time: wt,
                metal_make: 0.0,
                energy_make: 0.0,
                energy_upkeep: 0.0,
                wind_generator: 0.0,
                extracts_metal: 0.0,
                metal_storage: 0.0,
                energy_storage: 0.0,
                build_distance: 0.0,
                speed: 0.0,
                footprint_x: fp,
                footprint_z: fp,
            };
            f(&mut st);
            st
        }
        Catalog {
            commander: s("armcom", "Armada Commander", 2700.0, 26000.0, 75000.0, 300.0, 2, |s| {
                s.metal_make = 2.0;
                s.energy_make = 30.0;
                s.metal_storage = 500.0;
                s.energy_storage = 500.0;
                s.build_distance = 145.0;
                s.speed = 37.5;
            }),
            lab: s("armlab", "Bot Lab", 500.0, 950.0, 5000.0, 150.0, 6, |s| {
                s.metal_storage = 100.0;
                s.energy_storage = 100.0;
            }),
            nano: s("armnanotc", "Construction Turret", 230.0, 3200.0, 5300.0, 200.0, 3, |s| {
                s.build_distance = 400.0;
            }),
            mex: s("armmex", "Metal Extractor", 50.0, 500.0, 1800.0, 0.0, 4, |s| {
                s.energy_upkeep = 3.0;
                s.extracts_metal = 0.001;
                s.metal_storage = 50.0;
            }),
            wind: s("armwin", "Wind Turbine", 40.0, 175.0, 1600.0, 0.0, 3, |s| {
                s.wind_generator = 25.0;
                s.energy_storage = 0.5;
            }),
            solar: s("armsolar", "Solar Collector", 155.0, 0.0, 2600.0, 0.0, 5, |s| {
                s.energy_upkeep = -20.0;
                s.energy_storage = 50.0;
            }),
            tick: s("armflea", "Tick", 21.0, 300.0, 800.0, 0.0, 1, |s| {
                s.speed = 132.0;
            }),
        }
    }
}
