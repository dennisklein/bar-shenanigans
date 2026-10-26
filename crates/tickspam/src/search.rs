//! Beam search for the commander build order with the most Ticks at the horizon.
//!
//! A search node is a simulation that is paused at the moment the commander
//! needs its next order. A child adds one [`Action`] and runs until the
//! commander needs an order again, so the search never simulates a prefix
//! twice. To rate a node, a copy of it ends the plan (the commander assists
//! the first lab) and runs to the horizon. The beam keeps the best `width`
//! nodes per depth.

use serde::{Deserialize, Serialize};

use crate::catalog::Role;
use crate::plan::{Action, Base, Placer};
use crate::sim::{Order, Sim};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub max_labs: usize,
    pub max_nanos: usize,
    pub max_actions: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_labs: 4, max_nanos: 8, max_actions: 40 }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Found {
    pub actions: Vec<Action>,
    /// Ticks complete at the horizon.
    pub ticks: u32,
    /// Ranking value: ticks plus a fraction for Ticks under construction.
    pub score: f64,
}

#[derive(Clone)]
struct Node {
    sim: Sim,
    actions: Vec<Action>,
    placer: Placer,
    /// False when the commander did not get free before the horizon.
    open: bool,
}

/// Finishes a copy of `sim` (commander assists lab 0) and runs to `horizon`.
pub fn rollout(sim: &Sim, horizon: u32) -> (u32, f64) {
    let mut s = sim.clone();
    if !s.commander_queue_empty() || !s.commander_assisting() {
        s.push_order(Order::Assist { lab: 0 });
    }
    s.run_until(horizon);
    (s.ticks(), score(&s))
}

fn score(s: &Sim) -> f64 {
    let partial: f64 =
        s.units.iter().filter(|u| u.role == Role::Tick && u.being_built).map(|u| u.progress as f64).sum();
    // resources left are worth a little: they break ties toward cheaper plans
    let rest = (s.team.energy as f64 / 300.0 + s.team.metal as f64 / 21.0).min(1.0) * 1e-3;
    s.ticks() as f64 + 0.5 * partial.min(0.999) + rest
}

fn allowed(node: &Node, a: Action, base: &Base, limits: &Limits) -> bool {
    match a {
        Action::Mex => node.placer.mexes() < base.spots.len(),
        Action::Lab => node.placer.labs() < limits.max_labs,
        Action::Nano => node.placer.nanos() < limits.max_nanos,
        Action::Wind => node.sim.catalog.wind.wind_generator > 0.0 && node.sim.rules.wind > 0.0,
        Action::Solar => true,
    }
}

fn expand(node: &Node, a: Action, base: &Base, horizon: u32) -> Option<(Node, u32, f64)> {
    let mut child = node.clone();
    let order = base.order(&mut child.placer, a)?;
    child.sim.push_order(order);
    child.actions.push(a);
    child.open = child.sim.run_until_commander_free(horizon);
    let (ticks, score) = rollout(&child.sim, horizon);
    Some((child, ticks, score))
}

/// Beam search from `root` (a fresh game with an empty commander queue).
pub fn beam(root: &Sim, base: &Base, horizon: u32, width: usize, limits: &Limits) -> Found {
    let mut root = root.clone();
    root.record = false;
    let (t0, s0) = rollout(&root, horizon);
    let mut best = Found { actions: vec![], ticks: t0, score: s0 };
    let mut frontier = vec![Node { sim: root, actions: vec![], placer: Placer::default(), open: true }];

    for _depth in 0..limits.max_actions {
        let jobs: Vec<(&Node, Action)> = frontier
            .iter()
            .filter(|n| n.open)
            .flat_map(|n| Action::ALL.into_iter().map(move |a| (n, a)))
            .filter(|(n, a)| allowed(n, *a, base, limits))
            .collect();
        if jobs.is_empty() {
            break;
        }
        let mut children = parallel_map(&jobs, |(n, a)| expand(n, *a, base, horizon));
        children.retain(|c| c.is_some());
        let mut children: Vec<(Node, u32, f64)> = children.into_iter().flatten().collect();
        children.sort_by(|a, b| b.2.total_cmp(&a.2));
        for (node, ticks, score) in &children {
            if *score > best.score {
                best = Found { actions: node.actions.clone(), ticks: *ticks, score: *score };
            }
        }
        children.truncate(width);
        frontier = children.into_iter().map(|c| c.0).collect();
    }
    best
}

/// Exhaustive search over all action lists up to `depth` (for tests).
pub fn exhaustive(root: &Sim, base: &Base, horizon: u32, depth: usize, limits: &Limits) -> Found {
    fn rec(n: &Node, base: &Base, horizon: u32, depth: usize, limits: &Limits, best: &mut Found) {
        if depth == 0 || !n.open {
            return;
        }
        for a in Action::ALL {
            if !allowed(n, a, base, limits) {
                continue;
            }
            if let Some((child, ticks, score)) = expand(n, a, base, horizon) {
                if score > best.score {
                    *best = Found { actions: child.actions.clone(), ticks, score };
                }
                rec(&child, base, horizon, depth - 1, limits, best);
            }
        }
    }
    let mut root = root.clone();
    root.record = false;
    let (t0, s0) = rollout(&root, horizon);
    let mut best = Found { actions: vec![], ticks: t0, score: s0 };
    let node = Node { sim: root, actions: vec![], placer: Placer::default(), open: true };
    rec(&node, base, horizon, depth, limits, &mut best);
    best
}

fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len().max(1));
    let chunk = items.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> =
            items.chunks(chunk).map(|c| scope.spawn(|| c.iter().map(&f).collect::<Vec<R>>())).collect();
        handles.into_iter().flat_map(|h| h.join().expect("search thread panicked")).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::plan::Spot;
    use crate::sim::{Pos, Rules, GAME_SPEED};

    fn setup() -> (Sim, Base) {
        let start = Pos::new(500.0, 500.0);
        let spots: Vec<Spot> =
            (0..4).map(|i| Spot { pos: Pos::new(560.0 + 90.0 * i as f32, 440.0), metal: 2.0 }).collect();
        let base = Base::new(start, Pos::new(4096.0, 4096.0), &spots);
        let sim = Sim::new(Catalog::reference(), Rules::default(), start, vec![]);
        (sim, base)
    }

    #[test]
    fn beam_matches_exhaustive_on_a_short_horizon() {
        let (sim, base) = setup();
        let horizon = 75 * GAME_SPEED;
        let limits = Limits { max_labs: 2, max_nanos: 2, max_actions: 4 };
        let ex = exhaustive(&sim, &base, horizon, 4, &limits);
        let bm = beam(&sim, &base, horizon, 64, &limits);
        assert!(ex.ticks > 0);
        assert!(bm.score >= ex.score - 1e-9, "beam {bm:?} exhaustive {ex:?}");
    }

    #[test]
    fn lab_first_is_needed_for_any_tick() {
        let (sim, _) = setup();
        let (ticks, _) = rollout(&sim, 60 * GAME_SPEED);
        assert_eq!(ticks, 0);
    }
}
