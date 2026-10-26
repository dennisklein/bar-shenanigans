//! Converts a simulation trace into `opener.json`.

use std::collections::BTreeMap;

use opener_format::{
    Builder, MapInfo, Objective as ObjectiveInfo, Opener, Pos, Size, Step, Track, Wind, SCHEMA_VERSION,
};

use crate::catalog::Catalog;
use crate::objective::Objective;
use crate::sim::{Outcome, Settings, Trace, World};

/// Describes the map for the export.
pub struct MapMeta {
    pub name: String,
    pub wind_min: f64,
    pub wind_max: f64,
}

pub const ASSUMPTIONS: &[&str] = &[
    "Builders walk in a straight line at full speed; no terrain, pathing, acceleration or turning.",
    "A building is placed at the nearest free 16 elmo grid position to its builder; only overlap is checked.",
    "A metal extractor goes on the nearest free metal spot to its builder.",
    "Wind is constant at the map's average wind (BAR common/wind_functions.lua).",
    "When resources run out, all construction slows down by the same factor.",
    "Energy converters use the energy above 75 % of the storage (BAR default).",
    "A builder without orders assists the nearest producing factory, else the nearest construction.",
    "No reclaim, no combat, no commander upgrades.",
];

#[allow(clippy::too_many_arguments)]
pub fn to_opener(
    catalog: &Catalog,
    world: &World,
    settings: &Settings,
    map: &MapMeta,
    objective: &Objective,
    outcome: &Outcome,
    trace: &Trace,
    faction: &str,
    bar_commit: Option<String>,
) -> Opener {
    // Steps that started before the horizon, sorted by start time.
    let mut order: Vec<usize> = (0..trace.steps.len()).filter(|&i| trace.steps[i].start_s.is_some()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (&trace.steps[a], &trace.steps[b]);
        sa.start_s.unwrap().total_cmp(&sb.start_s.unwrap()).then(a.cmp(&b))
    });
    let new_index: BTreeMap<usize, u32> = order.iter().enumerate().map(|(n, &i)| (i, n as u32)).collect();

    let mut units = BTreeMap::new();
    let mut add_unit = |id| {
        let u = catalog.get(id);
        units.entry(u.name.clone()).or_insert_with(|| u.info());
    };

    let mut steps = Vec::new();
    for (n, &i) in order.iter().enumerate() {
        let s = &trace.steps[i];
        add_unit(s.unit);
        let target_step = s.target_step.and_then(|t| new_index.get(&t).copied());
        if s.target_step.is_some() && target_step.is_none() {
            continue;
        }
        steps.push(Step {
            index: n as u32,
            builder_id: s.builder as u32,
            action: s.action,
            unit: catalog.get(s.unit).name.clone(),
            target_step,
            position: s.pos,
            queued_s: s.queued_s,
            start_s: s.start_s.unwrap(),
            finish_s: s.finish_s.unwrap_or(settings.horizon_s),
            complete: s.finish_s.is_some(),
        });
    }
    // Re-number after dropped steps.
    let renumber: BTreeMap<u32, u32> = steps.iter().enumerate().map(|(n, s)| (s.index, n as u32)).collect();
    for s in &mut steps {
        s.index = renumber[&s.index];
        s.target_step = s.target_step.and_then(|t| renumber.get(&t).copied());
    }

    let builders = trace
        .builders
        .iter()
        .enumerate()
        .map(|(id, b)| {
            add_unit(b.unit);
            Builder {
                id: id as u32,
                unit: catalog.get(b.unit).name.clone(),
                created_s: b.created_s,
                created_by_step: b
                    .created_by_step
                    .and_then(|s| new_index.get(&s))
                    .and_then(|s| renumber.get(s).copied()),
                build_power: b.build_power,
            }
        })
        .collect();

    let tracks = trace
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, w)| w.windows(2).any(|p| p[0].x != p[1].x || p[0].z != p[1].z))
        .map(|(id, w)| Track { builder_id: id as u32, waypoints: w.clone() })
        .collect();

    let wind_unit = catalog.units.iter().map(|u| u.wind).fold(0.0, f64::max);
    Opener {
        schema_version: SCHEMA_VERSION,
        generator: format!("eco-opener {}", env!("CARGO_PKG_VERSION")),
        bar_commit,
        map: MapInfo {
            name: map.name.clone(),
            size: Size { x: world.map_size.x, z: world.map_size.z },
            start_pos: Pos { x: world.start.x, z: world.start.z },
            metal_spots: world.spots.clone(),
            wind: Wind {
                min: map.wind_min,
                max: map.wind_max,
                used: if wind_unit > 0.0 { world.wind.min(wind_unit) } else { world.wind },
            },
            tidal: world.tidal,
        },
        faction: faction.to_owned(),
        objective: ObjectiveInfo {
            kind: objective.kind().to_owned(),
            horizon_s: settings.horizon_s,
            score: objective.score(outcome, settings),
            description: objective.description(),
        },
        units,
        builders,
        steps,
        samples: trace.samples.clone(),
        tracks,
        assumptions: ASSUMPTIONS.iter().map(|s| s.to_string()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::testing::catalog;
    use crate::plan::Plan;
    use crate::sim::simulate_traced;
    use opener_format::{Action, MetalSpot};

    #[test]
    fn export_is_consistent() {
        let c = catalog();
        let world = World {
            map_size: Pos { x: 4096.0, z: 4096.0 },
            start: Pos { x: 1000.0, z: 1000.0 },
            spots: vec![MetalSpot { x: 1100.0, z: 1000.0, metal: 2.0 }, MetalSpot { x: 1500.0, z: 1500.0, metal: 2.0 }],
            wind: 10.0,
            tidal: 0.0,
        };
        let s = Settings { horizon_s: 150.0, ..Settings::default() };
        let id = |n| c.id(n).unwrap();
        let plan = Plan {
            commander: vec![id("mex"), id("wind"), id("lab")],
            factories: vec![vec![id("ck"), id("ck")]],
            constructors: vec![vec![id("mex")], vec![id("wind")]],
        };
        let (out, trace) = simulate_traced(&c, &world, &s, &plan);
        let o = Objective::default_for(&c);
        let meta = MapMeta { name: "Test".into(), wind_min: 0.0, wind_max: 20.0 };
        let opener = to_opener(&c, &world, &s, &meta, &o, &out, &trace, "arm", None);

        let json = opener.to_json();
        let back = Opener::from_json(&json).unwrap();
        assert_eq!(back.to_json(), json);

        for (i, st) in opener.steps.iter().enumerate() {
            assert_eq!(st.index as usize, i);
            assert!(st.queued_s <= st.start_s && st.start_s <= st.finish_s);
            assert!((st.builder_id as usize) < opener.builders.len());
            assert!(opener.units.contains_key(&st.unit));
            if st.action == Action::Assist {
                let t = &opener.steps[st.target_step.unwrap() as usize];
                assert_eq!(t.unit, st.unit);
            }
        }
        assert!(opener.steps.windows(2).all(|w| w[0].start_s <= w[1].start_s));
        for b in &opener.builders[1..] {
            let st = &opener.steps[b.created_by_step.unwrap() as usize];
            assert_eq!(st.unit, b.unit);
            assert!((st.finish_s - b.created_s).abs() < 1e-9);
        }
        assert_eq!(opener.samples.len(), 151);
    }
}
