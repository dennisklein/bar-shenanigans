//! Steady-state throughput of lab and nano configurations.
//!
//! This is the question of the "Most Efficient Tick Spam Configuration"
//! video: with resources not limiting, how many Ticks per minute does a
//! setup of `labs` bot labs with `nanos` construction turrets make, and how
//! much of its build power does it really use?

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, Role};
use crate::sim::{AssistMode, Pos, Rules, Sim, GAME_SPEED};

/// Distance between lab centers (a lab is 48 elmos wide).
pub const LAB_SPACING: f32 = 112.0;
/// Distance between nano centers (a nano is 24 elmos wide).
pub const NANO_SPACING: f32 = 40.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigResult {
    pub labs: usize,
    /// Total nanos, spread as evenly as possible over the labs.
    pub nanos: usize,
    pub mode: AssistMode,
    /// Mean over the slow update phases.
    pub ticks_per_min: f32,
    pub ticks_per_min_min: f32,
    pub ticks_per_min_max: f32,
    /// Ticks per minute if all build power worked all the time.
    pub ideal_ticks_per_min: f32,
    /// `ticks_per_min / ideal_ticks_per_min`.
    pub efficiency: f32,
    /// Metal and energy cost of the labs and nanos.
    pub metal_cost: f32,
    pub energy_cost: f32,
    /// Energy per second that the Ticks use at this rate.
    pub energy_per_s: f32,
}

/// Measures one configuration: warm up, then count Ticks over `measure_s`.
///
/// The result is the mean over [`PHASES`] runs. Each run puts a different
/// number of other units in front of the setup, which shifts the slow
/// update phases like other players' units do in a real game.
pub fn measure(catalog: &Catalog, rules: &Rules, labs: usize, nanos: usize, measure_s: u32) -> ConfigResult {
    let runs: Vec<f32> = (0..PHASES).map(|p| ticks_per_min(catalog, rules, labs, nanos, measure_s, p)).collect();
    let per_min = runs.iter().sum::<f32>() / runs.len() as f32;
    let min = runs.iter().copied().fold(f32::INFINITY, f32::min);
    let max = runs.iter().copied().fold(0.0, f32::max);

    let bp = labs as f32 * catalog.lab.worker_time + nanos as f32 * catalog.nano.worker_time;
    let ideal = bp / catalog.tick.build_time * 60.0;
    ConfigResult {
        labs,
        nanos,
        mode: rules.assist_mode,
        ticks_per_min: per_min,
        ticks_per_min_min: min,
        ticks_per_min_max: max,
        ideal_ticks_per_min: ideal,
        efficiency: per_min / ideal,
        metal_cost: labs as f32 * catalog.lab.metal_cost + nanos as f32 * catalog.nano.metal_cost,
        energy_cost: labs as f32 * catalog.lab.energy_cost + nanos as f32 * catalog.nano.energy_cost,
        energy_per_s: per_min / 60.0 * catalog.tick.energy_cost,
    }
}

/// Number of slow update phase offsets that [`measure`] averages over.
pub const PHASES: usize = 15;

fn ticks_per_min(catalog: &Catalog, rules: &Rules, labs: usize, nanos: usize, measure_s: u32, padding: usize) -> f32 {
    let mut sim = Sim::new(catalog.clone(), rules.clone(), Pos::new(0.0, -300.0), vec![]);
    sim.record = false;
    sim.set_unlimited_resources();
    sim.pad_units(padding);
    // labs side by side, nanos in a row behind them (as in the video)
    for k in 0..labs {
        sim.spawn(Role::Lab, Pos::new(LAB_SPACING * k as f32, 0.0), None);
    }
    let center = LAB_SPACING * (labs as f32 - 1.0) / 2.0;
    for i in 0..nanos {
        let x = center + NANO_SPACING * (i as f32 - (nanos as f32 - 1.0) / 2.0);
        sim.spawn(Role::Nano, Pos::new(x, 100.0), Some(i % labs));
    }
    let warmup = 20 * GAME_SPEED;
    sim.run_until(warmup);
    let t0 = sim.ticks();
    sim.run_until(warmup + measure_s * GAME_SPEED);
    let ticks = (sim.ticks() - t0) as f32;
    ticks * 60.0 / measure_s as f32
}

/// All configurations with `1..=max_labs` labs and `0..=max_nanos` nanos.
pub fn table(catalog: &Catalog, rules: &Rules, max_labs: usize, max_nanos: usize, measure_s: u32) -> Vec<ConfigResult> {
    let mut out = Vec::new();
    for labs in 1..=max_labs {
        for nanos in 0..=max_nanos {
            out.push(measure(catalog, rules, labs, nanos, measure_s));
        }
    }
    out
}

/// The cheapest configuration (in metal + energy / 60, a common BAR rule
/// of thumb) that makes at least `ticks_per_min`.
pub fn cheapest_for_rate(results: &[ConfigResult], ticks_per_min: f32) -> Option<&ConfigResult> {
    results.iter().filter(|r| r.ticks_per_min >= ticks_per_min).min_by(|a, b| {
        let ca = a.metal_cost + a.energy_cost / 60.0;
        let cb = b.metal_cost + b.energy_cost / 60.0;
        ca.total_cmp(&cb)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_lab_without_nanos_is_near_ideal() {
        let r = measure(&Catalog::reference(), &Rules::default(), 1, 0, 120);
        // 60 / 5.33 s = 11.25 ideal; queue and pad overhead cost a few percent
        assert!(r.efficiency > 0.9 && r.efficiency < 1.0, "{r:?}");
    }

    #[test]
    fn two_labs_beat_one_lab_with_one_nano() {
        let c = Catalog::reference();
        let rules = Rules::default();
        let two_labs = measure(&c, &rules, 2, 0, 120);
        let lab_nano = measure(&c, &rules, 1, 1, 120);
        // 300 vs 350 build power, but the nano joins each Tick late
        assert!(two_labs.ticks_per_min > lab_nano.ticks_per_min, "{two_labs:?} {lab_nano:?}");
    }
}
