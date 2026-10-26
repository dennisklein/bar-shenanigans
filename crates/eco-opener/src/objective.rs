//! What "best opener" means.

use anyhow::{anyhow, Result};
use mlua::{Function, Lua};

use crate::catalog::Catalog;
use crate::sim::{Outcome, Settings};

/// Energy per metal when no converter is in the catalog. BAR's T1
/// converter changes 70 energy into 1 metal.
const FALLBACK_ENERGY_PER_METAL: f64 = 70.0;

/// Weight of the stored resources, as a tie-breaker between plans with
/// the same income.
const BANK_WEIGHT: f64 = 1e-4;

pub enum Objective {
    /// `metal_income + (energy_income - energy_upkeep) / energy_per_metal`
    /// at the horizon: the income in metal, with energy valued at the
    /// converter rate.
    MaxEco { energy_per_metal: f64 },
    /// A Lua function `function(o) return number end`. `o` has the fields
    /// of [`Outcome`] and `horizon_s`.
    Lua { source: String, chunk: String },
}

impl Objective {
    pub fn default_for(catalog: &Catalog) -> Self {
        let energy_per_metal = catalog.conversion_rate().map_or(FALLBACK_ENERGY_PER_METAL, |r| 1.0 / r);
        Objective::MaxEco { energy_per_metal }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Objective::MaxEco { .. } => "max_eco",
            Objective::Lua { .. } => "lua",
        }
    }

    pub fn description(&self) -> String {
        match self {
            Objective::MaxEco { energy_per_metal } => format!(
                "Metal income plus net energy income / {energy_per_metal:.0} (energy at the converter rate) \
                 at the horizon. Stored resources only break ties."
            ),
            Objective::Lua { chunk, .. } => format!("Lua objective from {chunk}."),
        }
    }

    pub fn score(&self, o: &Outcome, s: &Settings) -> f64 {
        match self {
            Objective::MaxEco { energy_per_metal } => {
                let income = o.metal_income + (o.energy_income - o.energy_upkeep) / energy_per_metal;
                income + BANK_WEIGHT * (o.metal + o.energy / energy_per_metal)
            }
            Objective::Lua { source, .. } => LUA_OBJECTIVE.with(|cell| {
                let mut cell = cell.borrow_mut();
                if cell.as_ref().is_none_or(|(src, _)| src != source) {
                    *cell = Some((source.clone(), self.compile().expect("objective validated before")));
                }
                cell.as_ref().unwrap().1.call(o, s).unwrap_or(f64::NEG_INFINITY)
            }),
        }
    }

    /// Checks that a Lua objective compiles and returns a number.
    pub fn validate(&self) -> Result<()> {
        if let Objective::Lua { .. } = self {
            let f = self.compile()?;
            f.call(&Outcome::default(), &Settings::default())?;
        }
        Ok(())
    }

    fn compile(&self) -> Result<LuaObjective> {
        let Objective::Lua { source, chunk } = self else { unreachable!() };
        let lua = Lua::new();
        let func: Function = lua
            .load(source.as_str())
            .set_name(format!("@{chunk}"))
            .eval()
            .map_err(|e| anyhow!("objective {chunk}: {e}"))?;
        let key = lua.create_registry_value(func)?;
        Ok(LuaObjective { lua, key })
    }
}

/// A compiled Lua objective. One per thread, because a Lua state is not `Send`.
struct LuaObjective {
    lua: Lua,
    key: mlua::RegistryKey,
}

impl LuaObjective {
    fn call(&self, o: &Outcome, s: &Settings) -> Result<f64> {
        let run = || -> mlua::Result<f64> {
            let t = self.lua.create_table()?;
            t.set("metal_income", o.metal_income)?;
            t.set("energy_income", o.energy_income)?;
            t.set("energy_upkeep", o.energy_upkeep)?;
            t.set("metal", o.metal)?;
            t.set("energy", o.energy)?;
            t.set("build_power", o.build_power)?;
            t.set("metal_wasted", o.metal_wasted)?;
            t.set("energy_wasted", o.energy_wasted)?;
            t.set("horizon_s", s.horizon_s)?;
            let f: Function = self.lua.registry_value(&self.key)?;
            f.call(t)
        };
        run().map_err(|e| anyhow!("objective: {e}"))
    }
}

thread_local! {
    /// The compiled objective of this thread, with its source.
    static LUA_OBJECTIVE: std::cell::RefCell<Option<(String, LuaObjective)>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::testing::catalog;

    #[test]
    fn max_eco_values_energy_at_converter_rate() {
        let o = Objective::default_for(&catalog());
        let out = Outcome { metal_income: 10.0, energy_income: 170.0, energy_upkeep: 30.0, ..Outcome::default() };
        let s = o.score(&out, &Settings::default());
        assert!((s - (10.0 + 140.0 * 0.01429)).abs() < 1e-6, "{s}");
    }

    #[test]
    fn lua_objective_runs() {
        let o = Objective::Lua {
            source: "return function(o) return o.metal_income * 2 + o.horizon_s end".into(),
            chunk: "test".into(),
        };
        o.validate().unwrap();
        let out = Outcome { metal_income: 3.0, ..Outcome::default() };
        assert_eq!(o.score(&out, &Settings { horizon_s: 100.0, ..Settings::default() }), 106.0);
    }

    #[test]
    fn bad_lua_objective_is_an_error() {
        let o = Objective::Lua { source: "return 42".into(), chunk: "bad".into() };
        assert!(o.validate().is_err());
    }
}
