//! `opener.json`: the exchange format for simulated build orders.
//!
//! A producer (for example `eco-opener` or `tickspam`) simulates a build
//! order and writes one [`Opener`]. A consumer (for example the opener
//! browser GUI) reads it and shows the build order step by step.
//!
//! Conventions:
//! - Times are seconds of game time from the game start (`*_s` fields).
//! - Positions are map coordinates in elmos. The origin is the top-left
//!   corner of the map, `x` increases to the east, `z` increases to the
//!   south. One map square is 8 elmos, one "map size unit" is 512 elmos.
//! - Resource amounts are in metal and energy. Rates are per second.
//! - Unit names are BAR unit def names, for example `armmex`.
//!
//! The format is versioned with [`SCHEMA_VERSION`]. A consumer must reject
//! a file with a different major version.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The schema version that this crate reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// One simulated build order with its full timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Opener {
    /// Always [`SCHEMA_VERSION`] for files written by this crate.
    pub schema_version: u32,
    /// Name and version of the program that wrote the file.
    pub generator: String,
    /// Commit of the Beyond-All-Reason repository that the unit defs come from.
    #[serde(default)]
    pub bar_commit: Option<String>,
    pub map: MapInfo,
    /// Faction prefix: `arm`, `cor` or `leg`.
    pub faction: String,
    pub objective: Objective,
    /// Static data for each unit def that the build order uses, by unit name.
    pub units: BTreeMap<String, UnitInfo>,
    /// Every unit that can build: the commander, constructors and factories.
    /// `steps[].builder_id` refers to `builders[].id`.
    pub builders: Vec<Builder>,
    /// The build order, sorted by `start_s`.
    pub steps: Vec<Step>,
    /// Economy state at a fixed interval, sorted by `t_s`.
    pub samples: Vec<Sample>,
    /// Movement of each mobile builder, as waypoints. A consumer interpolates
    /// linearly between two waypoints. Builders that do not move have no entry.
    #[serde(default)]
    pub tracks: Vec<Track>,
    /// Model assumptions and simplifications, as plain sentences.
    #[serde(default)]
    pub assumptions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapInfo {
    /// Map name as the game shows it, for example `All That Glitters v2.2.3`.
    pub name: String,
    /// Map size in elmos.
    pub size: Size,
    /// Start position of the commander.
    pub start_pos: Pos,
    /// All metal spots that the simulation knew about, used or not.
    pub metal_spots: Vec<MetalSpot>,
    pub wind: Wind,
    /// Tidal strength in energy per second for a tidal generator.
    #[serde(default)]
    pub tidal: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub x: f64,
    pub z: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pos {
    pub x: f64,
    pub z: f64,
}

impl Pos {
    pub fn distance(self, other: Pos) -> f64 {
        (self.x - other.x).hypot(self.z - other.z)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MetalSpot {
    pub x: f64,
    pub z: f64,
    /// Metal per second that a T1 extractor makes on this spot.
    pub metal: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Wind {
    pub min: f64,
    pub max: f64,
    /// Energy per second per wind generator that the simulation used.
    pub used: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Objective {
    /// Machine name, for example `max_eco` or `max_units`.
    pub kind: String,
    /// Game time at which the objective is measured.
    pub horizon_s: f64,
    /// Objective value of this build order. Higher is better.
    pub score: f64,
    /// One sentence that explains how `score` is computed.
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnitInfo {
    /// Display name, for example `Metal Extractor`.
    pub human_name: String,
    pub metal_cost: f64,
    pub energy_cost: f64,
    /// Build time in build power seconds: a builder with `w` build power
    /// needs `build_time / w` seconds.
    pub build_time: f64,
    /// Footprint in map squares (8 elmos each).
    pub footprint: Footprint,
    pub kind: UnitKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Footprint {
    pub x: u32,
    pub z: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Commander,
    /// Mobile unit that can build structures.
    Constructor,
    Factory,
    MetalExtractor,
    EnergyProducer,
    /// Converts energy to metal.
    MetalMaker,
    Storage,
    /// Any other unit, for example a combat unit.
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Builder {
    pub id: u32,
    pub unit: String,
    /// Time at which the builder is complete and starts to work.
    pub created_s: f64,
    /// Index of the step that made this builder. `None` for the commander.
    #[serde(default)]
    pub created_by_step: Option<u32>,
    /// Build power.
    pub build_power: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    /// Position of this step in `steps`.
    pub index: u32,
    pub builder_id: u32,
    pub action: Action,
    /// The unit that is built. For `assist`, the unit of the target step.
    pub unit: String,
    /// For `assist`: index of the step that this builder helps.
    #[serde(default)]
    pub target_step: Option<u32>,
    /// Build position. For a unit from a factory, the factory position.
    pub position: Pos,
    /// Time at which the builder gets this order (it finished its previous order).
    pub queued_s: f64,
    /// Time at which the builder arrives in build range and starts to build.
    pub start_s: f64,
    /// Time at which the unit is complete (for `assist`: the builder stops).
    pub finish_s: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// The builder starts a new unit.
    Build,
    /// The builder adds its build power to the unit of `target_step`.
    Assist,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub t_s: f64,
    /// Stored metal and energy.
    pub metal: f64,
    pub energy: f64,
    pub metal_storage: f64,
    pub energy_storage: f64,
    /// Production per second.
    pub metal_income: f64,
    pub energy_income: f64,
    /// Consumption per second (construction and upkeep).
    pub metal_usage: f64,
    pub energy_usage: f64,
    /// Production per second that was lost because the storage was full.
    pub metal_wasted: f64,
    pub energy_wasted: f64,
    /// Sum of the build power of all builders that exist.
    pub build_power: f64,
    /// Build power that builds at this time (not idle, not moving, not stalled).
    pub build_power_used: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub builder_id: u32,
    pub waypoints: Vec<Waypoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Waypoint {
    pub t_s: f64,
    pub x: f64,
    pub z: f64,
}

#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Json(serde_json::Error),
    Version(u32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Json(e) => write!(f, "invalid opener json: {e}"),
            Error::Version(v) => {
                write!(f, "opener schema version {v} is not supported (expected {SCHEMA_VERSION})")
            }
        }
    }
}

impl std::error::Error for Error {}

impl Opener {
    pub fn from_json(text: &str) -> Result<Self, Error> {
        let opener: Opener = serde_json::from_str(text).map_err(Error::Json)?;
        if opener.schema_version != SCHEMA_VERSION {
            return Err(Error::Version(opener.schema_version));
        }
        Ok(opener)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("opener serializes")
    }

    pub fn read(path: impl AsRef<Path>) -> Result<Self, Error> {
        Self::from_json(&std::fs::read_to_string(path).map_err(Error::Io)?)
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        std::fs::write(path, self.to_json() + "\n").map_err(Error::Io)
    }

    /// The last sample at or before `t_s`, if any.
    pub fn sample_at(&self, t_s: f64) -> Option<&Sample> {
        let i = self.samples.partition_point(|s| s.t_s <= t_s);
        i.checked_sub(1).map(|i| &self.samples[i])
    }

    /// Position of a builder at `t_s`, interpolated from its track.
    /// Returns `None` for a builder without a track.
    pub fn builder_pos_at(&self, builder_id: u32, t_s: f64) -> Option<Pos> {
        let wp = &self.tracks.iter().find(|t| t.builder_id == builder_id)?.waypoints;
        let first = wp.first()?;
        if t_s <= first.t_s {
            return Some(Pos { x: first.x, z: first.z });
        }
        for w in wp.windows(2) {
            let (a, b) = (w[0], w[1]);
            if t_s <= b.t_s {
                let f = if b.t_s > a.t_s { (t_s - a.t_s) / (b.t_s - a.t_s) } else { 1.0 };
                return Some(Pos { x: a.x + (b.x - a.x) * f, z: a.z + (b.z - a.z) * f });
            }
        }
        let last = wp.last()?;
        Some(Pos { x: last.x, z: last.z })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../examples/opener.json");

    #[test]
    fn example_round_trips() {
        let opener = Opener::from_json(EXAMPLE).unwrap();
        let again = Opener::from_json(&opener.to_json()).unwrap();
        assert_eq!(opener, again);
    }

    #[test]
    fn rejects_other_version() {
        let text = EXAMPLE.replacen("\"schema_version\": 1", "\"schema_version\": 2", 1);
        assert!(matches!(Opener::from_json(&text), Err(Error::Version(2))));
    }

    #[test]
    fn interpolates_track() {
        let opener = Opener::from_json(EXAMPLE).unwrap();
        let p = opener.builder_pos_at(0, 1.0).unwrap();
        let t = &opener.tracks[0].waypoints;
        assert!(p.x >= t[0].x.min(t[1].x) && p.x <= t[0].x.max(t[1].x));
    }
}
