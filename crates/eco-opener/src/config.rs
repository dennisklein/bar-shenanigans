//! Run configuration, written in Lua.
//!
//! ```lua
//! return {
//!   bar_root = "../../Beyond-All-Reason",            -- relative to this file
//!   map = "../data/maps/all_that_glitters.lua",
//!   start = "P1",
//!   commander = "armcom",
//!   -- units that a plan may use, besides the commander
//!   units = { "armmex", "armsolar", "armwin", "armlab", "armck" },
//!   horizon_s = 300,
//!   -- "max_eco", or Lua source that returns function(o) return score end
//!   objective = "max_eco",
//!   search = { iterations = 60000, runs = 8, seed = 1 },
//!   sim = { dt = 0.1 },
//! }
//! ```

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use mlua::{Lua, LuaSerdeExt, Value};
use serde::Deserialize;

use crate::search::SearchConfig;
use crate::sim::Settings;

#[derive(Debug, Clone, Deserialize)]
struct Raw {
    bar_root: String,
    map: String,
    start: String,
    commander: String,
    units: Vec<String>,
    #[serde(default = "default_horizon")]
    horizon_s: f64,
    #[serde(default = "default_objective")]
    objective: String,
    #[serde(default)]
    search: RawSearch,
    #[serde(default)]
    sim: RawSim,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawSearch {
    iterations: Option<usize>,
    runs: Option<usize>,
    threads: Option<usize>,
    seed: Option<u64>,
    max_orders: Option<usize>,
    max_constructors: Option<usize>,
    max_factories: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RawSim {
    dt: Option<f64>,
    start_metal: Option<f64>,
    start_energy: Option<f64>,
    converter_level: Option<f64>,
}

fn default_horizon() -> f64 {
    300.0
}

fn default_objective() -> String {
    "max_eco".into()
}

#[derive(Debug, Clone)]
pub struct Config {
    pub path: PathBuf,
    pub bar_root: PathBuf,
    pub map: PathBuf,
    pub start: String,
    pub commander: String,
    pub units: Vec<String>,
    /// `None` for the built-in max_eco objective, else Lua source.
    pub objective_lua: Option<String>,
    pub settings: Settings,
    pub search: SearchConfig,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("cannot read config {}", path.display()))?;
        let lua = Lua::new();
        let value: Value = lua
            .load(&text)
            .set_name(format!("@{}", path.display()))
            .eval()
            .map_err(|e| anyhow!("config {}: {e}", path.display()))?;
        let raw: Raw = lua.from_value(value).map_err(|e| anyhow!("config {}: {e}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let d = Settings::default();
        let s = SearchConfig::default();
        Ok(Config {
            path: path.to_owned(),
            bar_root: dir.join(&raw.bar_root),
            map: dir.join(&raw.map),
            start: raw.start,
            commander: raw.commander,
            units: raw.units,
            objective_lua: (raw.objective != "max_eco").then_some(raw.objective),
            settings: Settings {
                horizon_s: raw.horizon_s,
                dt: raw.sim.dt.unwrap_or(d.dt),
                start_metal: raw.sim.start_metal.unwrap_or(d.start_metal),
                start_energy: raw.sim.start_energy.unwrap_or(d.start_energy),
                converter_level: raw.sim.converter_level.unwrap_or(d.converter_level),
                ..d
            },
            search: SearchConfig {
                iterations: raw.search.iterations.unwrap_or(s.iterations),
                runs: raw.search.runs.unwrap_or(s.runs),
                threads: raw.search.threads.unwrap_or(s.threads),
                seed: raw.search.seed.unwrap_or(s.seed),
                max_orders: raw.search.max_orders.unwrap_or(s.max_orders),
                max_constructors: raw.search.max_constructors.unwrap_or(s.max_constructors),
                max_factories: raw.search.max_factories.unwrap_or(s.max_factories),
                ..s
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_config_with_defaults() {
        let dir = std::env::temp_dir().join(format!("eco-opener-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.lua");
        std::fs::write(
            &path,
            r#"return {
                bar_root = "bar", map = "m.lua", start = "P2", commander = "armcom",
                units = { "armmex" }, search = { iterations = 10 },
                objective = [[return function(o) return o.metal_income end]],
            }"#,
        )
        .unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.bar_root, dir.join("bar"));
        assert_eq!(c.settings.horizon_s, 300.0);
        assert_eq!(c.search.iterations, 10);
        assert!(c.objective_lua.unwrap().contains("metal_income"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
