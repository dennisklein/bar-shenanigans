//! `tickspam`: tick spam configurations, search and replay.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use tickspam::catalog::Catalog;
use tickspam::configs;
use tickspam::export;
use tickspam::plan::{summary, Plan};
use tickspam::scenario::Scenario;
use tickspam::search::{self, Limits};
use tickspam::sim::{AssistMode, Rules, GAME_SPEED};

const USAGE: &str = "\
usage:
  tickspam configs [--bar DIR] [--labs N] [--nanos N] [--seconds S] [--mode fight|guard] [--json FILE]
  tickspam search  --bar DIR --map FILE --start ID [--horizon S] [--width N] [--mode fight|guard]
                   [--max-labs N] [--max-nanos N] [--out opener.json]
  tickspam replay  --bar DIR --map FILE --start ID --plan FILE [--horizon S] [--mode fight|guard]
                   [--out opener.json]

--bar is a Beyond-All-Reason checkout (unit defs are read through Lua).
Without --bar, `configs` uses the built-in copy of the Armada values.";

struct Args {
    cmd: String,
    opts: Vec<(String, String)>,
}

impl Args {
    fn parse() -> Result<Args> {
        let mut it = std::env::args().skip(1);
        let Some(cmd) = it.next() else {
            bail!("{USAGE}")
        };
        let mut opts = Vec::new();
        while let Some(k) = it.next() {
            let Some(name) = k.strip_prefix("--") else {
                bail!("unexpected argument {k}\n{USAGE}")
            };
            let v = it
                .next()
                .with_context(|| format!("--{name} needs a value"))?;
            opts.push((name.to_string(), v));
        }
        Ok(Args { cmd, opts })
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.opts
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    fn path(&self, name: &str) -> Result<PathBuf> {
        self.get(name)
            .map(PathBuf::from)
            .with_context(|| format!("--{name} is required\n{USAGE}"))
    }

    fn num<T: std::str::FromStr>(&self, name: &str, default: T) -> Result<T> {
        match self.get(name) {
            None => Ok(default),
            Some(v) => v
                .parse()
                .map_err(|_| anyhow::anyhow!("--{name}: bad number {v}")),
        }
    }

    fn rules(&self) -> Result<Rules> {
        let mode = match self.get("mode").unwrap_or("fight") {
            "fight" | "patrol" => AssistMode::Fight,
            "guard" => AssistMode::Guard,
            m => bail!("--mode: expected fight or guard, got {m}"),
        };
        Ok(Rules {
            assist_mode: mode,
            ..Rules::default()
        })
    }

    fn scenario(&self) -> Result<Scenario> {
        Scenario::load(
            &self.path("bar")?,
            &self.path("map")?,
            self.get("start").context("--start is required")?,
            self.rules()?,
        )
    }
}

fn main() -> Result<()> {
    let args = Args::parse()?;
    match args.cmd.as_str() {
        "configs" => cmd_configs(&args),
        "search" => cmd_search(&args),
        "replay" => cmd_replay(&args),
        _ => bail!("unknown command {}\n{USAGE}", args.cmd),
    }
}

fn cmd_configs(args: &Args) -> Result<()> {
    let catalog = match args.get("bar") {
        Some(dir) => Catalog::from_unit_defs(&bar_data::UnitDefs::load(dir.as_ref())?)?,
        None => Catalog::reference(),
    };
    let rules = args.rules()?;
    let results = configs::table(
        &catalog,
        &rules,
        args.num("labs", 4)?,
        args.num("nanos", 8)?,
        args.num("seconds", 180)?,
    );
    println!(
        "assist mode {:?}; resources unlimited; mean over {} slow update phases",
        rules.assist_mode,
        configs::PHASES
    );
    println!("labs nanos  ticks/min (min-max)   ideal  efficiency  metal  energy  E/s used");
    for r in &results {
        println!(
            "{:4} {:5}  {:6.1} ({:5.1}-{:5.1})  {:6.1}  {:9.0}%  {:5.0}  {:6.0}  {:8.0}",
            r.labs,
            r.nanos,
            r.ticks_per_min,
            r.ticks_per_min_min,
            r.ticks_per_min_max,
            r.ideal_ticks_per_min,
            r.efficiency * 100.0,
            r.metal_cost,
            r.energy_cost,
            r.energy_per_s
        );
    }
    if let Some(path) = args.get("json") {
        std::fs::write(path, serde_json::to_string_pretty(&results)? + "\n")?;
        eprintln!("wrote {path}");
    }
    Ok(())
}

fn write_opener(
    args: &Args,
    scn: &Scenario,
    actions: &[tickspam::plan::Action],
    name: &str,
    horizon: u32,
) -> Result<u32> {
    let mut sim = scn.sim(scn.base.orders(actions));
    sim.run_until(horizon);
    if let Some(out) = args.get("out") {
        export::opener(scn, &sim, name).write(out)?;
        eprintln!("wrote {out}");
    }
    Ok(sim.ticks())
}

fn cmd_search(args: &Args) -> Result<()> {
    let scn = args.scenario()?;
    let horizon = args.num::<u32>("horizon", 180)? * GAME_SPEED;
    let limits = Limits {
        max_labs: args.num("max-labs", 4)?,
        max_nanos: args.num("max-nanos", 8)?,
        ..Limits::default()
    };
    let width = args.num("width", 128)?;
    eprintln!(
        "map {}, start {:?}, wind {:.2} E/s per turbine",
        scn.map.name, scn.start, scn.rules.wind
    );
    let t = std::time::Instant::now();
    let found = search::beam(&scn.sim(vec![]), &scn.base, horizon, width, &limits);
    eprintln!("searched in {:.1} s", t.elapsed().as_secs_f32());
    let ticks = write_opener(args, &scn, &found.actions, "beam search", horizon)?;
    println!("best: {} Ticks at {} s", ticks, horizon / GAME_SPEED);
    println!("build: {}", summary(&found.actions));
    Ok(())
}

fn cmd_replay(args: &Args) -> Result<()> {
    let scn = args.scenario()?;
    let plan = Plan::load(&args.path("plan")?)?;
    let horizon = args.num::<u32>("horizon", 180)? * GAME_SPEED;
    let ticks = write_opener(args, &scn, &plan.actions, &plan.name, horizon)?;
    println!(
        "{}: {} Ticks at {} s",
        plan.name,
        ticks,
        horizon / GAME_SPEED
    );
    println!("build: {}", plan.summary());
    Ok(())
}
