//! Map description for build order simulation, loaded from a Lua file.
//!
//! A map spec file returns one table:
//!
//! ```lua
//! return {
//!   name = "Example Map v1",
//!   size = { x = 6144, z = 10240 },        -- elmos
//!   wind = { min = 0, max = 20 },
//!   tidal = 0,
//!   start_positions = {
//!     { id = "P1", x = 668, z = 755 },
//!   },
//!   metal_spots = {
//!     { x = 600, z = 680, metal = 1.8 },   -- metal/s of a T1 extractor
//!   },
//! }
//! ```

use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use mlua::{Lua, LuaSerdeExt, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapSpec {
    pub name: String,
    pub size: Xz,
    pub wind: WindRange,
    #[serde(default)]
    pub tidal: f64,
    pub start_positions: Vec<StartPos>,
    pub metal_spots: Vec<Spot>,
    /// Where the data comes from, for example the map archive name.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Xz {
    pub x: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindRange {
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartPos {
    pub id: String,
    pub x: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub x: f64,
    pub z: f64,
    /// Metal per second of a T1 extractor (extractsmetal 0.001) on this spot.
    pub metal: f64,
}

impl MapSpec {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read map spec {}", path.display()))?;
        Self::from_lua(&text, &path.display().to_string())
    }

    pub fn from_lua(source: &str, chunk_name: &str) -> Result<Self> {
        let lua = Lua::new();
        let value: Value = lua
            .load(source)
            .set_name(format!("@{chunk_name}"))
            .eval()
            .map_err(|e| anyhow!("map spec {chunk_name}: {e}"))?;
        let spec: MapSpec =
            lua.from_value(value).map_err(|e| anyhow!("map spec {chunk_name}: {e}"))?;
        spec.validate()?;
        Ok(spec)
    }

    fn validate(&self) -> Result<()> {
        if self.start_positions.is_empty() {
            bail!("map spec {}: no start positions", self.name);
        }
        let inside = |x: f64, z: f64| (0.0..=self.size.x).contains(&x) && (0.0..=self.size.z).contains(&z);
        for p in &self.start_positions {
            if !inside(p.x, p.z) {
                bail!("map spec {}: start position {} is outside the map", self.name, p.id);
            }
        }
        for s in &self.metal_spots {
            if !inside(s.x, s.z) || s.metal <= 0.0 {
                bail!("map spec {}: bad metal spot at {}, {}", self.name, s.x, s.z);
            }
        }
        Ok(())
    }

    pub fn start(&self, id: &str) -> Option<&StartPos> {
        self.start_positions.iter().find(|p| p.id.eq_ignore_ascii_case(id))
    }

    /// Mean energy per second of a wind generator with maximum output
    /// `cap`, from BAR's precomputed table in `common/wind_functions.lua`.
    ///
    /// `wind_functions` is the source of that file. Its table has the
    /// result of a Monte Carlo simulation of the engine's wind model.
    pub fn average_wind(&self, wind_functions: &str, cap: f64) -> Result<f64> {
        let lua = Lua::new();
        let game = lua.create_table()?;
        game.set("windMin", self.wind.min.round() as i64)?;
        game.set("windMax", self.wind.max.round() as i64)?;
        lua.globals().set("Game", game)?;
        let avg: f64 = lua
            .load(wind_functions)
            .set_name("@common/wind_functions.lua")
            .eval::<mlua::Table>()?
            .get::<mlua::Function>("getAverageWind")?
            .call(())?;
        Ok(avg.min(cap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC: &str = r#"
        return {
          name = "Test Map",
          size = { x = 4096, z = 4096 },
          wind = { min = 0, max = 20 },
          start_positions = { { id = "P1", x = 500, z = 500 } },
          metal_spots = { { x = 600, z = 600, metal = 2.0 } },
        }
    "#;

    #[test]
    fn loads_spec() {
        let spec = MapSpec::from_lua(SPEC, "test").unwrap();
        assert_eq!(spec.metal_spots.len(), 1);
        assert_eq!(spec.start("p1").unwrap().x, 500.0);
        assert_eq!(spec.tidal, 0.0);
    }

    #[test]
    fn rejects_spot_outside_map() {
        let bad = SPEC.replace("x = 600", "x = 6000");
        assert!(MapSpec::from_lua(&bad, "test").is_err());
    }

    #[test]
    fn average_wind_uses_lookup_then_fallback() {
        let lookup = r#"
            local t = { [0] = { [20] = "14.9" } }
            local f = {}
            function f.getAverageWind()
              local a = t[Game.windMin] and t[Game.windMin][Game.windMax]
              return a and tonumber(a) or math.max(Game.windMin, Game.windMax * 0.75)
            end
            return f
        "#;
        let spec = MapSpec::from_lua(SPEC, "test").unwrap();
        assert_eq!(spec.average_wind(lookup, 25.0).unwrap(), 14.9);
        assert_eq!(spec.average_wind(lookup, 10.0).unwrap(), 10.0);
    }
}
