//! Converts a finished simulation into `opener.json` for the opener browser.

use std::collections::{BTreeMap, HashMap};

use opener_format as of;

use crate::catalog::Role;
use crate::scenario::Scenario;
use crate::sim::{Event, Sim, UnitId, GAME_SPEED};

/// Model assumptions, written into every `opener.json`.
pub const ASSUMPTIONS: &[&str] = &[
    "Frame-accurate model of the Recoil engine at 30 frames per second: staggered 0.5 s slow updates, income in half-second chunks, all-or-nothing build power per builder and frame.",
    "A lab queues its next Tick only in its own slow update, and starts it when the previous Tick has left the build pad.",
    "Nanos on fight/patrol help the nearest unit under construction in range after one slow update; nanos on guard need two.",
    "The commander assists (guards) the first lab after its last build order.",
    "Wind turbines make the map's average wind (BAR wind_functions table) all the time; the real wind changes.",
    "The commander walks in a straight line at full speed; build positions are a fixed compact base layout, mexes nearest first.",
    "No enemy, no reclaim, no unit losses; Ticks stay alive.",
];

fn kind(role: Role) -> of::UnitKind {
    match role {
        Role::Commander => of::UnitKind::Commander,
        Role::Lab => of::UnitKind::Factory,
        Role::Mex => of::UnitKind::MetalExtractor,
        Role::Wind | Role::Solar => of::UnitKind::EnergyProducer,
        Role::Nano | Role::Tick => of::UnitKind::Other,
    }
}

fn secs(frame: u32) -> f64 {
    frame as f64 / GAME_SPEED as f64
}

/// Builds the opener for `sim`, which must have run to `sim.frame` with
/// recording on. `name` names the plan, for example "beam search".
pub fn opener(scn: &Scenario, sim: &Sim, name: &str) -> of::Opener {
    let horizon = sim.frame;
    let pos = |p: crate::sim::Pos| of::Pos { x: p.x as f64, z: p.z as f64 };

    // one build step per unit, in start order
    struct Raw {
        builder: UnitId,
        unit: UnitId,
        assist: bool,
        queued: u32,
        start: u32,
        finish: Option<u32>,
    }
    let mut raw: Vec<Raw> = Vec::new();
    let mut last_order: HashMap<UnitId, u32> = HashMap::new();
    let mut open_assist: HashMap<(UnitId, UnitId), usize> = HashMap::new();
    for e in &sim.events {
        match *e {
            Event::Ordered { frame, builder, .. } => {
                last_order.insert(builder, frame);
            }
            Event::Started { frame, builder, unit } => {
                let queued = last_order.remove(&builder).unwrap_or(frame);
                raw.push(Raw { builder, unit, assist: false, queued, start: frame, finish: None });
            }
            Event::AssistStart { frame, builder, unit } => {
                open_assist.insert((builder, unit), raw.len());
                raw.push(Raw { builder, unit, assist: true, queued: frame, start: frame, finish: None });
            }
            Event::AssistStop { frame, builder, unit } => {
                if let Some(i) = open_assist.remove(&(builder, unit)) {
                    raw[i].finish = Some(frame);
                }
            }
            Event::Finished { .. } | Event::Move { .. } => {}
        }
    }
    for r in raw.iter_mut().filter(|r| !r.assist) {
        r.finish = sim.units[r.unit].finished_frame;
    }
    // an assist ends at the latest when its unit is complete
    for r in raw.iter_mut().filter(|r| r.assist) {
        if let Some(done) = sim.units[r.unit].finished_frame {
            r.finish = Some(r.finish.map_or(done, |f| f.min(done)));
        }
    }
    raw.sort_by_key(|r| (r.start, r.assist, r.builder));
    let build_step: HashMap<UnitId, u32> =
        raw.iter().enumerate().filter(|(_, r)| !r.assist).map(|(i, r)| (r.unit, i as u32)).collect();

    let steps: Vec<of::Step> = raw
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let u = &sim.units[r.unit];
            let position = if u.role == Role::Tick { sim.units[r.builder].pos } else { u.pos };
            of::Step {
                index: i as u32,
                builder_id: r.builder as u32,
                action: if r.assist { of::Action::Assist } else { of::Action::Build },
                unit: scn.catalog.get(u.role).name.clone(),
                target_step: if r.assist { build_step.get(&r.unit).copied() } else { None },
                position: pos(position),
                queued_s: secs(r.queued),
                start_s: secs(r.start),
                finish_s: secs(r.finish.unwrap_or(horizon)),
                complete: r.finish.is_some(),
            }
        })
        .collect();

    let mut builders = vec![of::Builder {
        id: 0,
        unit: scn.catalog.commander.name.clone(),
        created_s: 0.0,
        created_by_step: None,
        build_power: scn.catalog.commander.worker_time as f64,
    }];
    for (id, u) in sim.units.iter().enumerate() {
        if matches!(u.role, Role::Lab | Role::Nano) {
            if let (Some(done), Some(&step)) = (u.finished_frame, build_step.get(&id)) {
                builders.push(of::Builder {
                    id: id as u32,
                    unit: scn.catalog.get(u.role).name.clone(),
                    created_s: secs(done),
                    created_by_step: Some(step),
                    build_power: scn.catalog.get(u.role).worker_time as f64,
                });
            }
        }
    }

    let mut waypoints = vec![of::Waypoint { t_s: 0.0, x: scn.start.x as f64, z: scn.start.z as f64 }];
    for e in &sim.events {
        if let Event::Move { frame, builder: 0, from, to, arrive } = *e {
            waypoints.push(of::Waypoint { t_s: secs(frame), x: from.x as f64, z: from.z as f64 });
            waypoints.push(of::Waypoint { t_s: secs(arrive), x: to.x as f64, z: to.z as f64 });
        }
    }

    let units: BTreeMap<String, of::UnitInfo> = Role::ALL
        .iter()
        .map(|&r| {
            let st = scn.catalog.get(r);
            (
                st.name.clone(),
                of::UnitInfo {
                    human_name: st.human_name.clone(),
                    metal_cost: st.metal_cost as f64,
                    energy_cost: st.energy_cost as f64,
                    build_time: st.build_time as f64,
                    footprint: of::Footprint { x: st.footprint_x, z: st.footprint_z },
                    kind: kind(r),
                },
            )
        })
        .collect();

    let samples = sim
        .samples
        .iter()
        .map(|s| of::Sample {
            t_s: secs(s.frame),
            metal: s.metal as f64,
            energy: s.energy as f64,
            metal_storage: s.metal_storage as f64,
            energy_storage: s.energy_storage as f64,
            metal_income: s.metal_income as f64,
            energy_income: s.energy_income as f64,
            metal_usage: s.metal_usage as f64,
            energy_usage: s.energy_usage as f64,
            metal_wasted: s.metal_wasted as f64,
            energy_wasted: s.energy_wasted as f64,
            build_power: s.build_power as f64,
            build_power_used: s.build_power_used as f64,
        })
        .collect();

    let map = &scn.map;
    let mut assumptions: Vec<String> = ASSUMPTIONS.iter().map(|s| s.to_string()).collect();
    assumptions.push(format!("Plan: {name}."));
    of::Opener {
        schema_version: of::SCHEMA_VERSION,
        generator: format!("tickspam {}", env!("CARGO_PKG_VERSION")),
        bar_commit: scn.bar_commit.clone(),
        map: of::MapInfo {
            name: map.name.clone(),
            size: of::Size { x: map.size.x, z: map.size.z },
            start_pos: pos(scn.start),
            metal_spots: map.metal_spots.iter().map(|s| of::MetalSpot { x: s.x, z: s.z, metal: s.metal }).collect(),
            wind: of::Wind { min: map.wind.min, max: map.wind.max, used: scn.rules.wind as f64 },
            tidal: map.tidal,
        },
        faction: "arm".into(),
        objective: of::Objective {
            kind: "max_units".into(),
            horizon_s: secs(horizon),
            score: sim.ticks() as f64,
            description: format!("Ticks ({}) complete at the horizon.", scn.catalog.tick.name),
        },
        units,
        builders,
        steps,
        samples,
        tracks: vec![of::Track { builder_id: 0, waypoints }],
        assumptions,
    }
}
