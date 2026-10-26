//! Browser for `opener.json` build orders: timeline, build queue,
//! economy curves and build positions on the map.

mod app;
mod map;
mod model;

use std::path::PathBuf;

use anyhow::Result;

const DEFAULT_PATHS: [&str; 2] = ["openers/all_that_glitters_eco.json", "crates/opener-format/examples/opener.json"];

fn main() -> Result<()> {
    let path = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .or_else(|| DEFAULT_PATHS.iter().map(PathBuf::from).find(|p| p.exists()));
    let app = app::App::new(path);
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native("Opener browser", options, Box::new(|_| Ok(Box::new(app)))).map_err(|e| anyhow::anyhow!("{e}"))
}
