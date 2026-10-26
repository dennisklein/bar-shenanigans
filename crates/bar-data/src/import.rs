//! Builds a [`MapSpec`] from a Spring map archive (`.sd7` or an extracted
//! directory).
//!
//! - `mapinfo.lua` gives the wind, tidal strength, maximum metal,
//!   extractor radius and start positions.
//! - The `.smf` file gives the map size and the metal map.
//! - The metal spots come from BAR's own spot finder
//!   (`common/upgets/api_resource_spot_finder.lua`), run in Lua on that
//!   metal map, so they are the spots that the game shows.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::rc::Rc;

use anyhow::{anyhow, bail, Context, Result};
use mlua::{Function, Lua, Table, Value};

use crate::map::{MapSpec, Spot, StartPos, WindRange, Xz};
use crate::vfs::Vfs;

/// Files of a map archive, by lower-case path.
pub struct MapArchive {
    files: BTreeMap<String, Vec<u8>>,
    pub source: String,
}

impl MapArchive {
    pub fn open(path: &Path) -> Result<Self> {
        let source = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let mut files = BTreeMap::new();
        if path.is_dir() {
            collect_dir(path, "", &mut files)?;
        } else {
            let mut reader = sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty())
                .map_err(|e| anyhow!("cannot open map archive {}: {e}", path.display()))?;
            let names: Vec<String> = reader
                .archive()
                .files
                .iter()
                .filter(|e| !e.is_directory())
                .map(|e| e.name().to_owned())
                .filter(|n| wanted(n))
                .collect();
            for n in names {
                let data = reader.read_file(&n).map_err(|e| anyhow!("{}: {n}: {e}", path.display()))?;
                files.insert(n.replace('\\', "/").to_lowercase(), data);
            }
        }
        Ok(MapArchive { files, source })
    }

    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.files.get(&name.to_lowercase()).map(Vec::as_slice)
    }

    fn text(&self, name: &str) -> Option<String> {
        self.get(name).map(|b| String::from_utf8_lossy(b).into_owned())
    }

    fn smf(&self) -> Option<(&str, &[u8])> {
        self.files.iter().find(|(k, _)| k.ends_with(".smf")).map(|(k, v)| (k.as_str(), v.as_slice()))
    }
}

/// Only the small files are read; textures stay in the archive.
fn wanted(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with(".smf") || n.ends_with(".lua")
}

fn collect_dir(dir: &Path, rel: &str, out: &mut BTreeMap<String, Vec<u8>>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = format!("{rel}{}", entry.file_name().to_string_lossy());
        if entry.file_type()?.is_dir() {
            collect_dir(&entry.path(), &format!("{name}/"), out)?;
        } else if wanted(&name) {
            out.insert(name.to_lowercase(), std::fs::read(entry.path())?);
        }
    }
    Ok(())
}

/// The parts of an SMF file that the import needs.
#[derive(Debug, Clone)]
pub struct Smf {
    /// Size in map squares (8 elmos).
    pub squares_x: usize,
    pub squares_z: usize,
    /// Raw metal map, `(squares_x / 2) * (squares_z / 2)` bytes, row by row.
    pub metal: Vec<u8>,
}

impl Smf {
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < 80 || &data[..15] != b"spring map file" {
            bail!("not an SMF file");
        }
        let int = |o: usize| i32::from_le_bytes(data[o..o + 4].try_into().unwrap());
        let (sx, sz) = (int(24), int(28));
        let metal_ptr = int(68);
        if sx <= 0 || sz <= 0 || metal_ptr <= 0 {
            bail!("bad SMF header");
        }
        let (sx, sz, ptr) = (sx as usize, sz as usize, metal_ptr as usize);
        let len = (sx / 2) * (sz / 2);
        let metal = data.get(ptr..ptr + len).ok_or_else(|| anyhow!("SMF metal map is cut off"))?.to_vec();
        Ok(Smf { squares_x: sx, squares_z: sz, metal })
    }

    /// Map size in elmos.
    pub fn size(&self) -> Xz {
        Xz { x: self.squares_x as f64 * 8.0, z: self.squares_z as f64 * 8.0 }
    }
}

/// Values from `mapinfo.lua`.
#[derive(Debug, Clone)]
pub struct MapInfo {
    pub name: String,
    pub version: String,
    pub max_metal: f64,
    pub extractor_radius: f64,
    pub tidal: f64,
    pub wind: WindRange,
    pub start_positions: Vec<StartPos>,
}

impl MapInfo {
    pub fn parse(archive: &MapArchive) -> Result<Self> {
        let text = archive.text("mapinfo.lua").ok_or_else(|| anyhow!("map archive has no mapinfo.lua"))?;
        let lua = Lua::new();
        install_map_vfs(&lua, archive)?;
        let t: Table = lua.load(&text).set_name("@mapinfo.lua").eval().map_err(|e| anyhow!("mapinfo.lua: {e}"))?;
        // The engine reads mapinfo.lua case-insensitively.
        let t: Table = lua.globals().get::<Function>("__lowerkeys")?.call(t)?;
        let num = |t: &Table, k: &str, d: f64| -> f64 {
            match t.get::<Value>(k) {
                Ok(Value::Integer(i)) => i as f64,
                Ok(Value::Number(n)) => n,
                Ok(Value::String(s)) => s.to_str().ok().and_then(|s| s.parse().ok()).unwrap_or(d),
                _ => d,
            }
        };
        let atmo: Option<Table> = t.get("atmosphere").ok();
        // Engine defaults (Recoil MapInfo): wind 5..25, extractor radius 500.
        let wind = WindRange {
            min: atmo.as_ref().map_or(5.0, |a| num(a, "minwind", 5.0)),
            max: atmo.as_ref().map_or(25.0, |a| num(a, "maxwind", 25.0)),
        };
        let mut start_positions = Vec::new();
        if let Ok(teams) = t.get::<Table>("teams") {
            let mut keyed: Vec<(i64, f64, f64)> = Vec::new();
            for pair in teams.pairs::<i64, Table>() {
                let (i, team) = pair?;
                if let Ok(sp) = team.get::<Table>("startpos") {
                    keyed.push((i, num(&sp, "x", 0.0), num(&sp, "z", 0.0)));
                }
            }
            keyed.sort_by_key(|k| k.0);
            for (n, (_, x, z)) in keyed.into_iter().enumerate() {
                start_positions.push(StartPos { id: format!("P{}", n + 1), x, z });
            }
        }
        Ok(MapInfo {
            name: t.get::<Option<String>>("name")?.unwrap_or_default(),
            version: t.get::<Option<String>>("version")?.unwrap_or_default(),
            max_metal: num(&t, "maxmetal", 0.02),
            extractor_radius: num(&t, "extractorradius", 500.0),
            tidal: num(&t, "tidalstrength", 0.0),
            wind,
            start_positions,
        })
    }
}

/// Lowercases keys recursively and gives map Lua files a minimal engine.
fn install_map_vfs(lua: &Lua, archive: &MapArchive) -> Result<()> {
    let files = lua.create_table()?;
    for (k, v) in &archive.files {
        if k.ends_with(".lua") {
            files.set(k.as_str(), String::from_utf8_lossy(v).into_owned())?;
        }
    }
    lua.globals().set("__map_files", files)?;
    lua.load(
        r#"
        local files = __map_files
        __map_files = nil
        VFS = { MAP = "m", MOD = "M", BASE = "b", ZIP = "Mmb", RAW = "r" }
        function VFS.FileExists(p) return files[p:lower()] ~= nil end
        function VFS.LoadFile(p) return files[p:lower()] end
        function VFS.Include(p, env)
            local src = files[p:lower()]
            if not src then error("VFS.Include: " .. p .. " not found", 2) end
            local f = assert(loadstring(src, "@" .. p))
            setfenv(f, env or _G)
            return f()
        end
        Spring = {
            GetMapOptions = function() return {} end,
            GetModOptions = function() return {} end,
            Echo = function() end,
            Log = function() end,
        }
        Game = { version = "bar-data" }
        function __lowerkeys(t)
            local n = {}
            for k, v in pairs(t) do
                if type(v) == "table" then v = __lowerkeys(v) end
                n[type(k) == "string" and k:lower() or k] = v
            end
            return n
        end
        Engine = { version = "2025.06.0" }
        "#,
    )
    .set_name("@bar-data/map-engine.lua")
    .exec()?;
    Ok(())
}

/// Runs BAR's metal spot finder on a metal map.
///
/// `metal[z * width + x]` is the metal amount of metal square (x, z), as
/// `Spring.GetMetalAmount` returns it (raw value * max metal / 255).
pub fn find_metal_spots(
    bar_root: &Path,
    size: Xz,
    width: usize,
    metal: Rc<Vec<f64>>,
    extractor_radius: f64,
) -> Result<Vec<Spot>> {
    let bar = Vfs::new(bar_root)?;
    let finder = bar
        .read("common/upgets/api_resource_spot_finder.lua")
        .ok_or_else(|| anyhow!("BAR repository has no common/upgets/api_resource_spot_finder.lua"))?;
    let lua = Lua::new();
    for f in ["common/numberfunctions.lua", "common/tablefunctions.lua"] {
        let src = bar.read(f).ok_or_else(|| anyhow!("BAR repository has no {f}"))?;
        lua.load(&src).set_name(format!("@{f}")).exec().map_err(|e| anyhow!("{f}: {e}"))?;
    }
    let g = lua.globals();
    let game = lua.create_table()?;
    game.set("metalMapSquareSize", 16)?;
    game.set("squareSize", 8)?;
    game.set("footprintScale", 2)?;
    game.set("extractorRadius", extractor_radius)?;
    game.set("mapSizeX", size.x)?;
    game.set("mapSizeZ", size.z)?;
    game.set("mapName", "")?;
    g.set("Game", game)?;
    let spring = lua.create_table()?;
    let height = metal.len() / width.max(1);
    let m = metal.clone();
    spring.set(
        "GetMetalAmount",
        lua.create_function(move |_, (x, z): (i64, i64)| {
            if x < 0 || z < 0 || x as usize >= width || z as usize >= height {
                return Ok(0.0);
            }
            Ok(m[z as usize * width + x as usize])
        })?,
    )?;
    spring.set("GetGroundHeight", lua.create_function(|_, ()| Ok(0.0))?)?;
    spring.set("SetGameRulesParam", lua.create_function(|_, ()| Ok(()))?)?;
    spring.set("GetAllFeatures", lua.create_function(|lua, ()| lua.create_table())?)?;
    spring.set("TestBuildOrder", lua.create_function(|_, ()| Ok(1))?)?;
    g.set("Spring", spring)?;
    g.set("UnitDefs", lua.create_table()?)?;
    g.set("FeatureDefs", lua.create_table()?)?;
    g.set("GG", lua.create_table()?)?;
    let handler = lua.create_table()?;
    handler.set("IsSyncedCode", lua.create_function(|_, ()| Ok(true))?)?;
    g.set("gadgetHandler", handler)?;
    let gadget = lua.create_table()?;
    g.set("gadget", gadget.clone())?;
    lua.load(&finder).set_name("@api_resource_spot_finder.lua").exec().map_err(|e| anyhow!("spot finder: {e}"))?;
    let init: Function = gadget.get("Initialize")?;
    init.call::<()>(gadget).map_err(|e| anyhow!("spot finder: {e}"))?;
    let finder: Table = g.get::<Table>("GG")?.get("resource_spot_finder")?;
    let list = match finder.get::<Value>("metalSpotsList")? {
        Value::Table(t) => t,
        _ => bail!("the spot finder found no metal spots (metal map without spots, or a metal map)"),
    };
    let mut spots = Vec::new();
    for s in list.sequence_values::<Table>() {
        let s = s?;
        let (x, z, worth): (f64, f64, f64) = (s.get("x")?, s.get("z")?, s.get("worth")?);
        // BAR shows worth / 1000 as the spot value: the income of a T1 extractor.
        spots.push(Spot { x, z, metal: (worth / 1000.0 * 1000.0).round() / 1000.0 });
    }
    Ok(spots)
}

/// BAR's `map_metal_spot_placer.lua`: a map without a metal map can list
/// its spots in `mapconfig/map_metal_layout.lua`; the gadget paints them.
fn paint_layout(archive: &MapArchive, size: Xz, max_metal: f64, metal: &mut [f64], width: usize) -> Result<bool> {
    let Some(src) = archive.text("mapconfig/map_metal_layout.lua") else { return Ok(false) };
    let lua = Lua::new();
    install_map_vfs(&lua, archive)?;
    let t: Table =
        lua.load(&src).set_name("@map_metal_layout.lua").eval().map_err(|e| anyhow!("map_metal_layout.lua: {e}"))?;
    let spots: Table = t.get("spots")?;
    let factor = 0.43 * 9.0 / 21.0;
    let (wx, wz) = ((size.x / 16.0) as i64, (size.z / 16.0) as i64);
    for s in spots.sequence_values::<Table>() {
        let s = s?;
        let (px, pz, m): (f64, f64, f64) = (s.get("x")?, s.get("z")?, s.get("metal")?);
        let (xi, zi) = ((px / 16.0).floor() as i64, (pz / 16.0).floor() as i64);
        for dx in -2i64..=2 {
            for dz in -2i64..=2 {
                if dx.abs() == 2 && dz.abs() == 2 {
                    continue;
                }
                let (x, z) = (xi + dx, zi + dz);
                if x > 0 && x < wx && z > 0 && z < wz {
                    // Spring.SetMetalAmount stores a raw value; reading it back scales by max metal.
                    let raw = (m * factor * 255.0).clamp(0.0, 255.0).floor();
                    metal[z as usize * width + x as usize] = raw * max_metal / 255.0;
                }
            }
        }
    }
    Ok(true)
}

/// Builds a map spec from a map archive and a BAR checkout.
pub fn import_map(archive_path: &Path, bar_root: &Path) -> Result<MapSpec> {
    let archive = MapArchive::open(archive_path)?;
    let info = MapInfo::parse(&archive)?;
    let (smf_name, smf_data) = archive.smf().ok_or_else(|| anyhow!("map archive has no .smf file"))?;
    let smf = Smf::parse(smf_data).with_context(|| smf_name.to_owned())?;
    let size = smf.size();
    let width = smf.squares_x / 2;
    let mut metal: Vec<f64> = smf.metal.iter().map(|&r| r as f64 * info.max_metal / 255.0).collect();
    if metal.iter().all(|&m| m == 0.0) {
        paint_layout(&archive, size, info.max_metal, &mut metal, width)?;
    }
    let spots = find_metal_spots(bar_root, size, width, Rc::new(metal), info.extractor_radius)?;
    let name = if info.version.is_empty() || info.name.contains(&info.version) {
        info.name.clone()
    } else {
        format!("{} {}", info.name, info.version)
    };
    let spec = MapSpec {
        name,
        size,
        wind: info.wind,
        tidal: info.tidal,
        start_positions: info.start_positions,
        metal_spots: spots,
        source: Some(archive.source.clone()),
    };
    Ok(spec)
}

impl MapSpec {
    /// The spec as a Lua file that [`MapSpec::load`] reads.
    pub fn to_lua(&self, header: &str) -> String {
        let mut s = String::new();
        for line in header.lines() {
            let _ = writeln!(s, "-- {line}");
        }
        let _ = writeln!(s, "return {{");
        let _ = writeln!(s, "\tname = {:?},", self.name);
        if let Some(src) = &self.source {
            let _ = writeln!(s, "\tsource = {src:?},");
        }
        let _ = writeln!(s, "\tsize = {{ x = {}, z = {} }},", self.size.x, self.size.z);
        let _ = writeln!(s, "\twind = {{ min = {}, max = {} }},", self.wind.min, self.wind.max);
        let _ = writeln!(s, "\ttidal = {},", self.tidal);
        let _ = writeln!(s, "\tstart_positions = {{");
        for p in &self.start_positions {
            let _ = writeln!(s, "\t\t{{ id = {:?}, x = {}, z = {} }},", p.id, p.x, p.z);
        }
        let _ = writeln!(s, "\t}},");
        let _ = writeln!(s, "\tmetal_spots = {{");
        for p in &self.metal_spots {
            let _ = writeln!(s, "\t\t{{ x = {}, z = {}, metal = {} }},", p.x, p.z, p.metal);
        }
        let _ = writeln!(s, "\t}},");
        let _ = writeln!(s, "}}");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smf_bytes(sx: i32, sz: i32, metal: &[u8]) -> Vec<u8> {
        let mut d = vec![0u8; 80];
        d[..16].copy_from_slice(b"spring map file\0");
        d[24..28].copy_from_slice(&sx.to_le_bytes());
        d[28..32].copy_from_slice(&sz.to_le_bytes());
        d[68..72].copy_from_slice(&80i32.to_le_bytes());
        d.extend_from_slice(metal);
        d
    }

    #[test]
    fn parses_smf_header_and_metal_map() {
        let metal: Vec<u8> = (0..16).collect();
        let smf = Smf::parse(&smf_bytes(8, 8, &metal)).unwrap();
        assert_eq!(smf.size(), Xz { x: 64.0, z: 64.0 });
        assert_eq!(smf.metal, metal);
        assert!(Smf::parse(&smf_bytes(8, 8, &metal[..4])).is_err());
        assert!(Smf::parse(b"not a map").is_err());
    }

    #[test]
    fn spec_lua_round_trip() {
        let spec = MapSpec {
            name: "Test \"Map\"".into(),
            size: Xz { x: 1024.0, z: 2048.0 },
            wind: WindRange { min: 2.0, max: 18.0 },
            tidal: 13.0,
            start_positions: vec![StartPos { id: "P1".into(), x: 100.0, z: 200.0 }],
            metal_spots: vec![Spot { x: 300.0, z: 400.0, metal: 1.8 }],
            source: Some("test.sd7".into()),
        };
        let back = MapSpec::from_lua(&spec.to_lua("header"), "t").unwrap();
        assert_eq!(back, spec);
    }
}
