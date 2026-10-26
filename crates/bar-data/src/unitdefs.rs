//! Unit defs, loaded by running BAR's own gamedata scripts in Lua.

use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use anyhow::{anyhow, Context, Result};
use mlua::{Lua, Table, Value};
use serde::{Deserialize, Serialize};

use crate::vfs::Vfs;

const ENGINE_LUA: &str = include_str!("../lua/engine.lua");
const SYSTEM_LUA: &str = include_str!("../lua/system.lua");

/// The unit def fields that matter for economy and build order simulation.
///
/// Field values are the values after BAR's post-processing
/// (`gamedata/unitdefs_post.lua` and `gamedata/alldefs_post.lua`) with the
/// default mod options. Missing fields get the engine defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitDef {
    pub name: String,
    /// English display name from `language/en/units.json`.
    pub human_name: String,
    pub metal_cost: f64,
    pub energy_cost: f64,
    /// Build time in build power seconds.
    pub build_time: f64,
    /// Build power.
    pub worker_time: f64,
    pub build_options: Vec<String>,
    /// Metal per second, unconditional.
    pub metal_make: f64,
    /// Energy per second, unconditional.
    pub energy_make: f64,
    /// Energy per second used while active. Negative means production
    /// (BAR solar collectors use this).
    pub energy_upkeep: f64,
    /// Metal per second used while active.
    pub metal_upkeep: f64,
    /// Maximum energy per second of a wind generator (capped by map wind).
    pub wind_generator: f64,
    /// Tidal generator flag/strength (the map tidal strength applies).
    pub tidal_generator: f64,
    /// Extraction factor of a metal extractor (0.001 for a T1 mex).
    pub extracts_metal: f64,
    pub metal_storage: f64,
    pub energy_storage: f64,
    /// Build range in elmos.
    pub build_distance: f64,
    /// Movement speed in elmos per second.
    pub speed: f64,
    /// Footprint in map squares.
    pub footprint_x: u32,
    pub footprint_z: u32,
    pub is_builder: bool,
    /// A builder that cannot move and builds mobile units.
    pub is_factory: bool,
    /// Start in the "on" state (affects upkeep and metal makers).
    pub activate_when_built: bool,
    /// String form of every `customparams` entry, for example
    /// `energyconv_capacity` and `energyconv_efficiency` of metal makers.
    pub customparams: BTreeMap<String, String>,
}

impl UnitDef {
    pub fn is_commander(&self) -> bool {
        self.customparams.get("iscommander").is_some_and(|v| v == "true")
    }

    pub fn param(&self, key: &str) -> Option<f64> {
        self.customparams.get(key).and_then(|v| v.parse().ok())
    }

    /// Energy per second converted by a metal maker, if this is one.
    pub fn energy_conversion(&self) -> Option<(f64, f64)> {
        Some((self.param("energyconv_capacity")?, self.param("energyconv_efficiency")?))
    }
}

/// All unit defs of a BAR repository checkout.
#[derive(Debug, Clone, Default)]
pub struct UnitDefs {
    defs: BTreeMap<String, UnitDef>,
    /// Commit hash of the checkout, if it is a git repository.
    pub commit: Option<String>,
    /// Errors and warnings that the gamedata scripts logged.
    pub warnings: Vec<String>,
}

impl UnitDefs {
    /// Runs `gamedata/unitdefs.lua` of the BAR repository at `bar_root`.
    pub fn load(bar_root: &Path) -> Result<Self> {
        let vfs = Rc::new(Vfs::new(bar_root)?);
        let lua = Lua::new();
        install_host(&lua, vfs.clone())?;
        lua.load(ENGINE_LUA).set_name("@bar-data/engine.lua").exec()?;
        lua.load(
            r#"
            VFS_MODES = VFS.MAP .. VFS.MOD .. VFS.BASE
            VFS.Include("init.lua")
            return VFS.Include("gamedata/unitdefs.lua")
            "#,
        )
        .set_name("@bar-data/load.lua")
        .eval::<Table>()
        .map_err(|e| anyhow!("BAR gamedata scripts failed: {e}"))
        .and_then(|table| {
            let names = load_names(&vfs);
            let mut defs = BTreeMap::new();
            for pair in table.pairs::<String, Table>() {
                let (name, t) = pair?;
                let def = convert(&name, &t, &names).with_context(|| format!("unit def {name}"))?;
                defs.insert(name, def);
            }
            let warnings: Vec<String> = lua.globals().get::<Vec<String>>("__bar_warnings")?;
            Ok(UnitDefs { defs, commit: git_commit(bar_root), warnings })
        })
    }

    pub fn get(&self, name: &str) -> Option<&UnitDef> {
        self.defs.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = &UnitDef> {
        self.defs.values()
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Builds unit defs from a list, for tests and synthetic data.
    pub fn from_defs(defs: impl IntoIterator<Item = UnitDef>) -> Self {
        UnitDefs { defs: defs.into_iter().map(|d| (d.name.clone(), d)).collect(), commit: None, warnings: Vec::new() }
    }
}

fn install_host(lua: &Lua, vfs: Rc<Vfs>) -> Result<()> {
    let g = lua.globals();
    let v = vfs.clone();
    g.set("__bar_read", lua.create_function(move |_, p: String| Ok(v.read(&p)))?)?;
    let v = vfs.clone();
    g.set("__bar_exists", lua.create_function(move |_, p: String| Ok(v.exists(&p)))?)?;
    let v = vfs.clone();
    g.set(
        "__bar_dirlist",
        lua.create_function(move |_, (d, pat, rec): (String, String, bool)| Ok(v.dir_list(&d, &pat, rec)))?,
    )?;
    let v = vfs;
    g.set("__bar_subdirs", lua.create_function(move |_, d: String| Ok(v.sub_dirs(&d)))?)?;
    let embedded = lua.create_table()?;
    embedded.set("gamedata/system.lua", SYSTEM_LUA)?;
    g.set("__bar_embedded", embedded)?;
    Ok(())
}

fn load_names(vfs: &Vfs) -> BTreeMap<String, String> {
    let Some(text) = vfs.read("language/en/units.json") else { return BTreeMap::new() };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return BTreeMap::new() };
    json["units"]["names"]
        .as_object()
        .map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect())
        .unwrap_or_default()
}

fn git_commit(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(root).args(["rev-parse", "HEAD"]).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

fn num(t: &Table, key: &str, default: f64) -> Result<f64> {
    Ok(match t.get::<Value>(key)? {
        Value::Integer(i) => i as f64,
        Value::Number(n) => n,
        Value::String(s) => s.to_str()?.trim().parse().unwrap_or(default),
        Value::Boolean(b) => f64::from(u8::from(b)),
        _ => default,
    })
}

fn flag(t: &Table, key: &str, default: bool) -> Result<bool> {
    Ok(match t.get::<Value>(key)? {
        Value::Boolean(b) => b,
        Value::Integer(i) => i != 0,
        Value::Number(n) => n != 0.0,
        Value::String(s) => matches!(s.to_str()?.trim(), "1" | "true"),
        _ => default,
    })
}

fn convert(name: &str, t: &Table, names: &BTreeMap<String, String>) -> Result<UnitDef> {
    let mut build_options = Vec::new();
    if let Ok(opts) = t.get::<Table>("buildoptions") {
        let mut keyed: Vec<(i64, String)> = Vec::new();
        for pair in opts.pairs::<Value, String>() {
            let (k, v) = pair?;
            if let Value::Integer(i) = k {
                keyed.push((i, v));
            }
        }
        keyed.sort();
        build_options = keyed.into_iter().map(|(_, v)| v).collect();
    }
    let mut customparams = BTreeMap::new();
    if let Ok(cp) = t.get::<Table>("customparams") {
        for pair in cp.pairs::<String, Value>() {
            let (k, v) = pair?;
            let s = match v {
                Value::String(s) => s.to_str()?.to_owned(),
                Value::Integer(i) => i.to_string(),
                Value::Number(n) => n.to_string(),
                Value::Boolean(b) => b.to_string(),
                _ => continue,
            };
            customparams.insert(k, s);
        }
    }
    let worker_time = num(t, "workertime", 0.0)?;
    let speed = num(t, "speed", 0.0)?;
    let is_builder = flag(t, "builder", false)? && worker_time > 0.0;
    Ok(UnitDef {
        name: name.to_owned(),
        human_name: names.get(name).cloned().unwrap_or_else(|| name.to_owned()),
        metal_cost: num(t, "metalcost", 0.0)?,
        energy_cost: num(t, "energycost", 0.0)?,
        build_time: num(t, "buildtime", 0.0)?,
        worker_time,
        // Nano turrets do not move either, but have no build options.
        is_factory: is_builder && speed == 0.0 && !build_options.is_empty(),
        build_options,
        metal_make: num(t, "metalmake", 0.0)?,
        energy_make: num(t, "energymake", 0.0)?,
        energy_upkeep: num(t, "energyupkeep", 0.0)?,
        metal_upkeep: num(t, "metalupkeep", 0.0)?,
        wind_generator: num(t, "windgenerator", 0.0)?,
        tidal_generator: num(t, "tidalgenerator", 0.0)?,
        extracts_metal: num(t, "extractsmetal", 0.0)?,
        metal_storage: num(t, "metalstorage", 0.0)?,
        energy_storage: num(t, "energystorage", 0.0)?,
        build_distance: num(t, "builddistance", 128.0)?,
        speed,
        footprint_x: num(t, "footprintx", 1.0)? as u32,
        footprint_z: num(t, "footprintz", 1.0)? as u32,
        is_builder,
        activate_when_built: flag(t, "activatewhenbuilt", false)?,
        customparams,
    })
}
