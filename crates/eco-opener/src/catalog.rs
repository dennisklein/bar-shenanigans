//! Compact unit table for the simulator, built from BAR unit defs.

use anyhow::{bail, Result};
use bar_data::{UnitDef, UnitDefs};
use opener_format::{Footprint, UnitInfo, UnitKind};

/// Index into [`Catalog::units`].
pub type UnitId = u16;

#[derive(Debug, Clone)]
pub struct Unit {
    pub name: String,
    pub human_name: String,
    pub kind: UnitKind,
    pub metal_cost: f64,
    pub energy_cost: f64,
    pub build_time: f64,
    pub build_power: f64,
    pub speed: f64,
    pub build_distance: f64,
    /// Half of the larger footprint side, in elmos.
    pub radius: f64,
    pub footprint: (u32, u32),
    /// Metal per second, always on.
    pub metal_make: f64,
    /// Energy per second, always on, including solar (negative upkeep).
    pub energy_make: f64,
    /// Energy per second used while active (positive).
    pub energy_upkeep: f64,
    /// Maximum output of a wind generator.
    pub wind: f64,
    pub tidal: f64,
    pub extracts_metal: f64,
    pub metal_storage: f64,
    pub energy_storage: f64,
    /// Energy to metal converter: (energy per second, metal per energy).
    pub conversion: Option<(f64, f64)>,
    pub build_options: Vec<UnitId>,
}

impl Unit {
    pub fn is_builder(&self) -> bool {
        self.build_power > 0.0
    }

    pub fn is_mobile(&self) -> bool {
        self.speed > 0.0
    }

    pub fn can_build(&self, other: UnitId) -> bool {
        self.build_options.contains(&other)
    }

    pub fn info(&self) -> UnitInfo {
        UnitInfo {
            human_name: self.human_name.clone(),
            metal_cost: self.metal_cost,
            energy_cost: self.energy_cost,
            build_time: self.build_time,
            footprint: Footprint { x: self.footprint.0, z: self.footprint.1 },
            kind: self.kind,
        }
    }
}

/// The units that a build order of one faction can use.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub units: Vec<Unit>,
    pub commander: UnitId,
}

impl Catalog {
    /// Collects the commander `commander` and every unit that it can
    /// build, directly or through other builders, restricted to `allowed`
    /// (unit names). The commander is always included.
    pub fn new(defs: &UnitDefs, commander: &str, allowed: &[String]) -> Result<Self> {
        let Some(com) = defs.get(commander) else { bail!("unknown commander {commander}") };
        let mut names = vec![com.name.clone()];
        let mut i = 0;
        while i < names.len() {
            let def = defs.get(&names[i]).expect("names come from defs");
            for opt in &def.build_options {
                if allowed.contains(opt) && !names.contains(opt) && defs.get(opt).is_some() {
                    names.push(opt.clone());
                }
            }
            i += 1;
        }
        for a in allowed {
            if !names.contains(a) {
                bail!("unit {a} is not buildable from {commander} with the allowed units");
            }
        }
        let mut units: Vec<Unit> = names.iter().map(|n| convert(defs.get(n).unwrap())).collect();
        for (unit, name) in units.iter_mut().zip(&names) {
            unit.build_options = defs
                .get(name)
                .unwrap()
                .build_options
                .iter()
                .filter_map(|o| names.iter().position(|n| n == o))
                .map(|p| p as UnitId)
                .collect();
        }
        Ok(Catalog { units, commander: 0 })
    }

    pub fn id(&self, name: &str) -> Option<UnitId> {
        self.units.iter().position(|u| u.name == name).map(|p| p as UnitId)
    }

    pub fn get(&self, id: UnitId) -> &Unit {
        &self.units[id as usize]
    }

    /// Metal per energy of the best converter, used to value energy.
    pub fn conversion_rate(&self) -> Option<f64> {
        self.units.iter().filter_map(|u| u.conversion.map(|c| c.1)).reduce(f64::max)
    }
}

fn convert(def: &UnitDef) -> Unit {
    let kind = if def.is_commander() {
        UnitKind::Commander
    } else if def.is_factory {
        UnitKind::Factory
    } else if def.is_builder && def.speed > 0.0 {
        UnitKind::Constructor
    } else if def.extracts_metal > 0.0 {
        UnitKind::MetalExtractor
    } else if def.energy_conversion().is_some() {
        UnitKind::MetalMaker
    } else if def.energy_make > 0.0 || def.energy_upkeep < 0.0 || def.wind_generator > 0.0 || def.tidal_generator > 0.0
    {
        UnitKind::EnergyProducer
    } else if def.metal_storage > 0.0 || def.energy_storage > 0.0 {
        UnitKind::Storage
    } else {
        UnitKind::Other
    };
    Unit {
        name: def.name.clone(),
        human_name: def.human_name.clone(),
        kind,
        metal_cost: def.metal_cost,
        energy_cost: def.energy_cost,
        build_time: def.build_time.max(1.0),
        build_power: if def.is_builder { def.worker_time } else { 0.0 },
        speed: def.speed,
        build_distance: def.build_distance,
        radius: 4.0 * def.footprint_x.max(def.footprint_z) as f64,
        footprint: (def.footprint_x, def.footprint_z),
        metal_make: def.metal_make,
        energy_make: def.energy_make + (-def.energy_upkeep).max(0.0),
        energy_upkeep: def.energy_upkeep.max(0.0),
        wind: def.wind_generator,
        tidal: def.tidal_generator,
        extracts_metal: def.extracts_metal,
        metal_storage: def.metal_storage,
        energy_storage: def.energy_storage,
        conversion: def.energy_conversion(),
        build_options: Vec::new(),
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! A small synthetic catalog with BAR-like numbers, for unit tests.
    use super::*;
    use std::collections::BTreeMap;

    pub fn def(name: &str, f: impl FnOnce(&mut UnitDef)) -> UnitDef {
        let mut d = UnitDef {
            name: name.into(),
            human_name: name.into(),
            metal_cost: 0.0,
            energy_cost: 0.0,
            build_time: 1.0,
            worker_time: 0.0,
            build_options: vec![],
            metal_make: 0.0,
            energy_make: 0.0,
            energy_upkeep: 0.0,
            metal_upkeep: 0.0,
            wind_generator: 0.0,
            tidal_generator: 0.0,
            extracts_metal: 0.0,
            metal_storage: 0.0,
            energy_storage: 0.0,
            build_distance: 128.0,
            speed: 0.0,
            footprint_x: 2,
            footprint_z: 2,
            is_builder: false,
            is_factory: false,
            activate_when_built: true,
            customparams: BTreeMap::new(),
        };
        f(&mut d);
        d
    }

    pub fn defs() -> UnitDefs {
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        UnitDefs::from_defs([
            def("com", |d| {
                d.metal_cost = 2700.0;
                d.build_time = 75000.0;
                d.worker_time = 300.0;
                d.is_builder = true;
                d.speed = 37.5;
                d.build_distance = 145.0;
                d.metal_make = 2.0;
                d.energy_make = 30.0;
                d.metal_storage = 500.0;
                d.energy_storage = 500.0;
                d.build_options = s(&["mex", "solar", "wind", "lab", "makr"]);
                d.customparams.insert("iscommander".into(), "true".into());
            }),
            def("mex", |d| {
                d.metal_cost = 50.0;
                d.energy_cost = 500.0;
                d.build_time = 1800.0;
                d.extracts_metal = 0.001;
                d.energy_upkeep = 3.0;
                d.metal_storage = 50.0;
                d.footprint_x = 4;
                d.footprint_z = 4;
            }),
            def("solar", |d| {
                d.metal_cost = 155.0;
                d.build_time = 2600.0;
                d.energy_upkeep = -20.0;
                d.energy_storage = 50.0;
                d.footprint_x = 5;
                d.footprint_z = 5;
            }),
            def("wind", |d| {
                d.metal_cost = 40.0;
                d.energy_cost = 175.0;
                d.build_time = 1600.0;
                d.wind_generator = 25.0;
                d.footprint_x = 3;
                d.footprint_z = 3;
            }),
            def("makr", |d| {
                d.metal_cost = 1.0;
                d.energy_cost = 1150.0;
                d.build_time = 2600.0;
                d.customparams.insert("energyconv_capacity".into(), "70".into());
                d.customparams.insert("energyconv_efficiency".into(), "0.01429".into());
            }),
            def("lab", |d| {
                d.metal_cost = 500.0;
                d.energy_cost = 950.0;
                d.build_time = 5000.0;
                d.worker_time = 150.0;
                d.is_builder = true;
                d.is_factory = true;
                d.metal_storage = 100.0;
                d.energy_storage = 100.0;
                d.footprint_x = 6;
                d.footprint_z = 6;
                d.build_options = s(&["ck"]);
            }),
            def("ck", |d| {
                d.metal_cost = 110.0;
                d.energy_cost = 1600.0;
                d.build_time = 3450.0;
                d.worker_time = 80.0;
                d.is_builder = true;
                d.speed = 36.0;
                d.build_distance = 130.0;
                d.energy_make = 7.0;
                d.energy_storage = 50.0;
                d.build_options = s(&["mex", "solar", "wind", "lab", "makr"]);
            }),
        ])
    }

    pub fn catalog() -> Catalog {
        let allowed = ["mex", "solar", "wind", "lab", "ck", "makr"].map(String::from);
        Catalog::new(&defs(), "com", &allowed).unwrap()
    }

    #[test]
    fn builds_catalog() {
        let c = catalog();
        assert_eq!(c.get(c.commander).name, "com");
        assert_eq!(c.get(c.id("lab").unwrap()).kind, UnitKind::Factory);
        assert_eq!(c.get(c.id("ck").unwrap()).kind, UnitKind::Constructor);
        assert_eq!(c.get(c.id("solar").unwrap()).energy_make, 20.0);
        assert_eq!(c.get(c.id("solar").unwrap()).kind, UnitKind::EnergyProducer);
        assert!(c.get(c.id("lab").unwrap()).can_build(c.id("ck").unwrap()));
        assert!((c.conversion_rate().unwrap() - 0.01429).abs() < 1e-9);
    }

    #[test]
    fn rejects_unreachable_unit() {
        let allowed = ["mex", "ck"].map(String::from);
        assert!(Catalog::new(&defs(), "com", &allowed).is_err());
    }
}
