//! Beyond-All-Reason game data for build order simulation.
//!
//! - [`UnitDefs`] runs BAR's own Lua gamedata scripts (through `mlua`) on a
//!   checkout of the Beyond-All-Reason repository, so the values include
//!   BAR's post-processing.
//! - [`MapSpec`] loads a map description (metal spots, wind, start
//!   positions) from a Lua file. [`import::import_map`] makes one from a
//!   map archive with BAR's own metal spot finder.

pub mod import;
pub mod map;
pub mod unitdefs;
mod vfs;

pub use map::MapSpec;
pub use unitdefs::{UnitDef, UnitDefs};
