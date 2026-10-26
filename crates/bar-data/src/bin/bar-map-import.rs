//! Writes a map spec Lua file from a Spring map archive.
//!
//! Usage: `bar-map-import <map.sd7 | extracted dir> <bar_root> [out.lua]`

use std::path::PathBuf;

use anyhow::{bail, Result};

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let (archive, bar_root) = match &args[..] {
        [a, b] | [a, b, _] => (a, b),
        _ => bail!("usage: bar-map-import <map.sd7 | extracted dir> <bar_root> [out.lua]"),
    };
    let spec = bar_data::import::import_map(archive, bar_root)?;
    let total: f64 = spec.metal_spots.iter().map(|s| s.metal).sum();
    eprintln!(
        "{}: {:.0} x {:.0} elmos, wind {}..{}, tidal {}, {} start positions, {} metal spots ({total:.1} metal/s)",
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
        "Map spec for eco-opener, made by bar-map-import from {}.\nMetal spots: BAR common/upgets/api_resource_spot_finder.lua on the map's metal map.",
        spec.source.as_deref().unwrap_or("?")
    );
    let text = spec.to_lua(&header);
    match args.get(2) {
        Some(out) => std::fs::write(out, text)?,
        None => print!("{text}"),
    }
    Ok(())
}
