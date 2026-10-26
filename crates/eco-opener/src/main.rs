use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use bar_data::map::{Spot, StartPos};
use bar_data::{MapSpec, UnitDefs};
use clap::{Parser, Subcommand};
use eco_opener::catalog::Catalog;
use eco_opener::config::{Config, SpotRule};
use eco_opener::export::{to_opener, MapMeta};
use eco_opener::objective::Objective;
use eco_opener::plan::Plan;
use eco_opener::search::Problem;
use eco_opener::sim::{simulate_traced, Outcome, World};
use opener_format::{MetalSpot, Pos};

/// Simulate Beyond-All-Reason economy openers and search for the best one.
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Search for the best opener and write it as opener.json and as a Lua plan.
    Search {
        /// Lua run configuration.
        config: PathBuf,
        /// Output opener.json.
        #[arg(long, default_value = "opener.json")]
        out: PathBuf,
        /// Output Lua plan of the best opener (default: next to --out).
        #[arg(long)]
        plan_out: Option<PathBuf>,
        /// Start position id, overrides the config.
        #[arg(long)]
        start: Option<String>,
        /// Annealing steps per run, overrides the config.
        #[arg(long)]
        iterations: Option<usize>,
        /// Number of runs, overrides the config.
        #[arg(long)]
        runs: Option<usize>,
        /// Start the search from this Lua plan instead of the built-in one.
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// Simulate a Lua plan and write it as opener.json.
    Replay {
        config: PathBuf,
        plan: PathBuf,
        #[arg(long, default_value = "opener.json")]
        out: PathBuf,
        #[arg(long)]
        start: Option<String>,
    },
}

struct Setup {
    config: Config,
    defs: UnitDefs,
    catalog: Catalog,
    world: World,
    map: MapMeta,
    objective: Objective,
    faction: String,
}

fn setup(config_path: &Path, start: Option<String>) -> Result<Setup> {
    let mut config = Config::load(config_path)?;
    if let Some(s) = start {
        config.start = s;
    }
    let defs = UnitDefs::load(&config.bar_root)?;
    let catalog = Catalog::new(&defs, &config.commander, &config.units)?;
    let spec = MapSpec::load(&config.map)?;
    let start =
        spec.start(&config.start).ok_or_else(|| anyhow!("map {} has no start position {}", spec.name, config.start))?;
    let wind_lua = std::fs::read_to_string(config.bar_root.join("common/wind_functions.lua"))
        .context("cannot read common/wind_functions.lua of the BAR repository")?;
    let wind = spec.average_wind(&wind_lua, f64::INFINITY)?;
    let world = World {
        map_size: Pos { x: spec.size.x, z: spec.size.z },
        start: Pos { x: start.x, z: start.z },
        spots: usable_spots(&spec, start, config.metal_spots),
        wind,
        tidal: spec.tidal,
    };
    let objective = match &config.objective_lua {
        None => Objective::default_for(&catalog),
        Some(src) => Objective::Lua { source: src.clone(), chunk: config.path.display().to_string() },
    };
    objective.validate()?;
    let faction = config.commander.chars().take(3).collect();
    let map = MapMeta { name: spec.name.clone(), wind_min: spec.wind.min, wind_max: spec.wind.max };
    Ok(Setup { config, defs, catalog, world, map, objective, faction })
}

/// The metal spots of the player at `start`.
fn usable_spots(spec: &MapSpec, start: &StartPos, rule: SpotRule) -> Vec<MetalSpot> {
    let d = |p: &StartPos, s: &Spot| (p.x - s.x).hypot(p.z - s.z);
    spec.metal_spots
        .iter()
        .filter(|s| match rule {
            SpotRule::All => true,
            SpotRule::NearestStart => spec.start_positions.iter().all(|o| d(start, s) <= d(o, s)),
        })
        .map(|s| MetalSpot { x: s.x, z: s.z, metal: s.metal })
        .collect()
}

fn report(s: &Setup, out: &Outcome, score: f64) {
    eprintln!(
        "score {score:.3}: metal {:.2}/s, energy {:.1}/s (upkeep {:.1}), build power {:.0}, bank {:.0} M {:.0} E, wasted {:.0} M {:.0} E, skipped orders {}",
        out.metal_income,
        out.energy_income,
        out.energy_upkeep,
        out.build_power,
        out.metal,
        out.energy,
        out.metal_wasted,
        out.energy_wasted,
        out.skipped_orders,
    );
    eprintln!(
        "map {} start {} ({:.0}, {:.0}), {} metal spots, wind {:.1}",
        s.map.name,
        s.config.start,
        s.world.start.x,
        s.world.start.z,
        s.world.spots.len(),
        s.world.wind
    );
}

fn write_outputs(s: &Setup, plan: &Plan, out_path: &Path, plan_path: Option<&Path>, header: &str) -> Result<()> {
    let (out, trace) = simulate_traced(&s.catalog, &s.world, &s.config.settings, plan);
    let opener = to_opener(
        &s.catalog,
        &s.world,
        &s.config.settings,
        &s.map,
        &s.objective,
        &out,
        &trace,
        &s.faction,
        s.defs.commit.clone(),
    );
    report(s, &out, opener.objective.score);
    opener.write(out_path).map_err(|e| anyhow!("{}: {e}", out_path.display()))?;
    eprintln!("wrote {}", out_path.display());
    if let Some(p) = plan_path {
        std::fs::write(p, plan.to_lua(&s.catalog, header)).with_context(|| format!("cannot write {}", p.display()))?;
        eprintln!("wrote {}", p.display());
    }
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Search { config, out, plan_out, start, iterations, runs, from } => {
            let mut s = setup(&config, start)?;
            if let Some(i) = iterations {
                s.config.search.iterations = i;
            }
            if let Some(r) = runs {
                s.config.search.runs = r;
            }
            let problem = Problem::new(&s.catalog, &s.world, &s.config.settings, &s.objective);
            let from = match from {
                Some(p) => Some(Plan::from_lua(&std::fs::read_to_string(&p)?, &p.display().to_string(), &s.catalog)?),
                None => None,
            };
            let cfg = &s.config.search;
            eprintln!(
                "searching: {} runs x {} steps on {} threads, horizon {} s",
                cfg.runs, cfg.iterations, cfg.threads, s.config.settings.horizon_s
            );
            let r = problem.search(cfg, from.as_ref());
            eprintln!(
                "{} evaluations in {:.1} s, run scores {:?}",
                r.evaluations,
                r.seconds,
                r.run_scores.iter().map(|x| (x * 1000.0).round() / 1000.0).collect::<Vec<_>>()
            );
            let plan_out = plan_out.unwrap_or_else(|| out.with_extension("lua"));
            let header = format!(
                "Best opener found by eco-opener {}\nmap {}, start {}, horizon {} s, objective {}\nscore {:.3}, seed {}, {} runs x {} steps",
                env!("CARGO_PKG_VERSION"),
                s.map.name,
                s.config.start,
                s.config.settings.horizon_s,
                s.objective.kind(),
                r.score,
                cfg.seed,
                cfg.runs,
                cfg.iterations,
            );
            write_outputs(&s, &r.plan, &out, Some(&plan_out), &header)
        }
        Command::Replay { config, plan, out, start } => {
            let s = setup(&config, start)?;
            let text = std::fs::read_to_string(&plan).with_context(|| format!("cannot read {}", plan.display()))?;
            let p = Plan::from_lua(&text, &plan.display().to_string(), &s.catalog)?;
            write_outputs(&s, &p, &out, None, "")
        }
    }
}
