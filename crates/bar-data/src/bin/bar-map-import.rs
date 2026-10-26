//! Writes a map spec Lua file from a Spring map archive.
//!
//! Usage: `bar-map-import <map.sd7 | extracted dir> <bar_root> [out.lua] [--metadata map_list.yaml]`
//!
//! With `--metadata`, the start positions come from BAR's maps-metadata
//! (the ids and positions of the BAR lobby), else from `mapinfo.lua`.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

const USAGE: &str = "usage: bar-map-import <map.sd7 | extracted dir> <bar_root> [out.lua] [--metadata map_list.yaml]";

fn main() -> Result<()> {
    let mut args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let mut metadata = None;
    if let Some(i) = args.iter().position(|a| a.as_os_str() == "--metadata") {
        if i + 1 >= args.len() {
            bail!(USAGE);
        }
        metadata = Some(args.remove(i + 1));
        args.remove(i);
    }
    let (archive, bar_root) = match &args[..] {
        [a, b] | [a, b, _] => (a, b),
        _ => bail!(USAGE),
    };
    let mut spec = bar_data::import::import_map(archive, bar_root)?;
    let mut positions_from = "mapinfo.lua";
    if let Some(m) = &metadata {
        let text = std::fs::read_to_string(m).with_context(|| format!("cannot read {}", m.display()))?;
        match bar_data::import::metadata_start_positions(&text, &spec.name)? {
            Some(p) => {
                spec.start_positions = p;
                positions_from = "BAR maps-metadata";
            }
            None => eprintln!("maps-metadata has no start positions for {}; using mapinfo.lua", spec.name),
        }
    }
    let total: f64 = spec.metal_spots.iter().map(|s| s.metal).sum();
    eprintln!(
        "{}: {:.0} x {:.0} elmos, wind {}..{}, tidal {}, {} start positions ({positions_from}), {} metal spots ({total:.1} metal/s)",
        spec.name,
        spec.size.x,
        spec.size.z,
        spec.wind.min,
        spec.wind.max,
        spec.tidal,
        spec.start_positions.len(),
        spec.metal_spots.len(),
    );
    let header = format!(
        "Map spec for eco-opener, made by bar-map-import from {}.\nMetal spots: BAR common/upgets/api_resource_spot_finder.lua on the map's metal map.\nStart positions: {positions_from}.",
        spec.source.as_deref().unwrap_or("?")
    );
    let text = spec.to_lua(&header);
    match args.get(2) {
        Some(out) => std::fs::write(out, text)?,
        None => print!("{text}"),
    }
    Ok(())
}
