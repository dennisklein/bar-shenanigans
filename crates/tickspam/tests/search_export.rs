//! End to end on a small test map: search, then export `opener.json`.

use tickspam::catalog::Catalog;
use tickspam::export;
use tickspam::plan::Action;
use tickspam::scenario::Scenario;
use tickspam::search::{self, Limits};
use tickspam::sim::{Rules, GAME_SPEED};

const MAP: &str = r#"
return {
  name = "Tick Test Map",
  size = { x = 4096, z = 4096 },
  wind = { min = 5, max = 20 },
  start_positions = { { id = "P1", x = 600, z = 600 } },
  metal_spots = {
    { x = 520, z = 470, metal = 2.0 }, { x = 700, z = 470, metal = 2.0 },
    { x = 450, z = 720, metal = 2.0 }, { x = 900, z = 600, metal = 2.0 },
  },
}
"#;

fn scenario() -> Scenario {
    let map = bar_data::MapSpec::from_lua(MAP, "test map").unwrap();
    let rules = Rules {
        wind: 12.0,
        ..Rules::default()
    };
    Scenario::from_parts(Catalog::reference(), rules, map, "P1").unwrap()
}

#[test]
fn search_finds_ticks_and_exports_a_valid_opener() {
    let scn = scenario();
    let horizon = 180 * GAME_SPEED;
    let found = search::beam(&scn.sim(vec![]), &scn.base, horizon, 16, &Limits::default());
    assert!(found.actions.contains(&Action::Lab));
    assert!(found.ticks >= 8, "{found:?}");

    // the replay of the found plan gives the same result
    let mut sim = scn.sim(scn.base.orders(&found.actions));
    sim.run_until(horizon);
    assert_eq!(sim.ticks(), found.ticks);

    let opener = export::opener(&scn, &sim, "test");
    let again = opener_format::Opener::from_json(&opener.to_json()).unwrap();
    assert_eq!(again.objective.score, found.ticks as f64);
    let ticks_done = again
        .steps
        .iter()
        .filter(|s| s.unit == "armflea" && s.action == opener_format::Action::Build && s.complete)
        .count();
    assert_eq!(ticks_done as u32, found.ticks);
    // steps are sorted and assists point at a build step of the same unit
    assert!(again.steps.windows(2).all(|w| w[0].start_s <= w[1].start_s));
    for s in again
        .steps
        .iter()
        .filter(|s| s.action == opener_format::Action::Assist)
    {
        let t = &again.steps[s.target_step.unwrap() as usize];
        assert_eq!(t.unit, s.unit);
        assert!(s.start_s >= t.start_s && s.finish_s <= t.finish_s + 1e-9);
    }
    assert!(!again.samples.is_empty());
}

#[test]
fn bundled_plans_load() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("plans");
    for entry in std::fs::read_dir(dir).unwrap() {
        let plan = tickspam::plan::Plan::load(&entry.unwrap().path()).unwrap();
        assert!(plan.actions.contains(&Action::Lab), "{}", plan.name);
    }
}
