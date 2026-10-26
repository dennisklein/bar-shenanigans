//! Search for the plan with the best objective value.
//!
//! Simulated annealing over [`Plan`]s, with independent runs on several
//! threads. Each step changes the plan a little (insert, delete, replace,
//! swap or move one order), simulates it and keeps it with the usual
//! Metropolis rule. A final pass deletes every order that does not help.

use std::time::Instant;

use crate::catalog::{Catalog, UnitId};
use crate::objective::Objective;
use crate::plan::{Plan, QueueRef};
use crate::sim::{simulate, Settings, World};

#[derive(Debug, Clone)]
pub struct SearchConfig {
    /// Annealing steps per run.
    pub iterations: usize,
    /// Independent runs; they run in parallel on `threads` threads.
    pub runs: usize,
    pub threads: usize,
    pub seed: u64,
    pub start_temperature: f64,
    pub end_temperature: f64,
    /// Upper limit for the number of orders in a plan.
    pub max_orders: usize,
    /// Upper limit for factory and constructor queues.
    pub max_factories: usize,
    pub max_constructors: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        SearchConfig {
            iterations: 60_000,
            runs: 8,
            threads: std::thread::available_parallelism().map_or(4, |n| n.get()),
            seed: 1,
            start_temperature: 1.0,
            end_temperature: 0.005,
            max_orders: 120,
            max_factories: 2,
            max_constructors: 10,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SearchResult {
    pub plan: Plan,
    pub score: f64,
    /// Best score of every run, in run order.
    pub run_scores: Vec<f64>,
    pub evaluations: usize,
    pub seconds: f64,
}

/// Small deterministic random number generator (SplitMix64).
#[derive(Debug, Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`; `n` must not be 0.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn pick<T: Copy>(&mut self, v: &[T]) -> Option<T> {
        (!v.is_empty()).then(|| v[self.below(v.len())])
    }
}

/// Context shared by all runs.
pub struct Problem<'a> {
    pub catalog: &'a Catalog,
    pub world: &'a World,
    pub settings: &'a Settings,
    pub objective: &'a Objective,
    /// Units the search may order, per queue kind.
    pub commander_options: Vec<UnitId>,
    pub factory_options: Vec<UnitId>,
    pub constructor_options: Vec<UnitId>,
}

impl<'a> Problem<'a> {
    /// Derives the order options from the catalog: everything that the
    /// builders of each kind can build.
    pub fn new(catalog: &'a Catalog, world: &'a World, settings: &'a Settings, objective: &'a Objective) -> Self {
        let mut factory_options = Vec::new();
        let mut constructor_options = Vec::new();
        for u in &catalog.units {
            if !u.is_builder() || u.kind == opener_format::UnitKind::Commander {
                continue;
            }
            let into = if u.is_mobile() { &mut constructor_options } else { &mut factory_options };
            for &o in &u.build_options {
                if !into.contains(&o) {
                    into.push(o);
                }
            }
        }
        Problem {
            catalog,
            world,
            settings,
            objective,
            commander_options: catalog.get(catalog.commander).build_options.clone(),
            factory_options,
            constructor_options,
        }
    }

    pub fn score(&self, plan: &Plan) -> f64 {
        self.objective.score(&simulate(self.catalog, self.world, self.settings, plan), self.settings)
    }

    fn options(&self, q: QueueRef) -> &[UnitId] {
        match q {
            QueueRef::Commander => &self.commander_options,
            QueueRef::Factory(_) => &self.factory_options,
            QueueRef::Constructor(_) => &self.constructor_options,
        }
    }

    /// A plain opener to start from: mex, energy, factory, constructors.
    pub fn initial_plan(&self) -> Plan {
        let c = self.catalog;
        let find = |pred: &dyn Fn(&crate::catalog::Unit) -> bool, opts: &[UnitId]| {
            opts.iter().copied().find(|&o| pred(c.get(o)))
        };
        let mex = |opts: &[UnitId]| find(&|u| u.extracts_metal > 0.0, opts);
        let energy = |opts: &[UnitId]| {
            let wind = find(&|u| u.wind > 0.0, opts).filter(|_| self.world.wind >= 7.0);
            wind.or_else(|| find(&|u| u.energy_make > 0.0 && u.metal_cost < 200.0, opts))
        };
        let factory =
            find(&|u| u.is_builder() && !u.is_mobile() && !u.build_options.is_empty(), &self.commander_options);
        let mut plan = Plan::default();
        let com = &self.commander_options;
        plan.commander.extend(mex(com).into_iter().cycle().take(2));
        plan.commander.extend(energy(com).into_iter().cycle().take(2));
        plan.commander.extend(factory);
        if let Some(f) = factory {
            let con = c.get(f).build_options.iter().copied().find(|&o| c.get(o).is_mobile() && c.get(o).is_builder());
            if let Some(con) = con {
                plan.factories.push(vec![con; 4]);
                let opts = &c.get(con).build_options;
                for _ in 0..4 {
                    let mut q = Vec::new();
                    q.extend(mex(opts).into_iter().cycle().take(2));
                    q.extend(energy(opts));
                    plan.constructors.push(q);
                }
            }
        }
        plan
    }

    fn mutate(&self, plan: &mut Plan, rng: &mut Rng, cfg: &SearchConfig) {
        let mut queues = vec![QueueRef::Commander];
        queues.extend((0..cfg.max_factories).map(QueueRef::Factory));
        queues.extend((0..cfg.max_constructors).map(QueueRef::Constructor));
        let non_empty: Vec<QueueRef> = plan.queues().into_iter().filter(|&q| !plan.queue(q).is_empty()).collect();
        let total = plan.order_count();
        match rng.below(7) {
            // Insert a new order.
            0 | 1 if total < cfg.max_orders => {
                let q = rng.pick(&queues).unwrap();
                if let Some(u) = rng.pick(self.options(q)) {
                    let v = plan.queue_mut(q);
                    let at = rng.below(v.len() + 1);
                    v.insert(at, u);
                }
            }
            // Delete an order.
            2 => {
                if let Some(q) = rng.pick(&non_empty) {
                    let v = plan.queue_mut(q);
                    let at = rng.below(v.len());
                    v.remove(at);
                }
            }
            // Replace an order.
            3 => {
                if let Some(q) = rng.pick(&non_empty) {
                    if let Some(u) = rng.pick(self.options(q)) {
                        let v = plan.queue_mut(q);
                        let at = rng.below(v.len());
                        v[at] = u;
                    }
                }
            }
            // Swap two orders of a queue (often neighbours).
            4 => {
                if let Some(q) = rng.pick(&non_empty) {
                    let v = plan.queue_mut(q);
                    if v.len() >= 2 {
                        let a = rng.below(v.len() - 1);
                        let b = if rng.below(2) == 0 { a + 1 } else { rng.below(v.len()) };
                        v.swap(a, b);
                    }
                }
            }
            // Move an order to another queue.
            5 => {
                if let Some(from) = rng.pick(&non_empty) {
                    let to = rng.pick(&queues).unwrap();
                    let at = rng.below(plan.queue(from).len());
                    let u = plan.queue(from)[at];
                    if self.options(to).contains(&u) {
                        plan.queue_mut(from).remove(at);
                        let v = plan.queue_mut(to);
                        let pos = rng.below(v.len() + 1);
                        v.insert(pos, u);
                    }
                }
            }
            // Duplicate an order in place.
            _ if total < cfg.max_orders => {
                if let Some(q) = rng.pick(&non_empty) {
                    let v = plan.queue_mut(q);
                    let at = rng.below(v.len());
                    let u = v[at];
                    v.insert(at, u);
                }
            }
            _ => {}
        }
        plan.trim();
    }

    fn anneal(&self, start: &Plan, cfg: &SearchConfig, seed: u64) -> (Plan, f64, usize) {
        let mut rng = Rng::new(seed);
        let mut cur = start.clone();
        let mut cur_score = self.score(&cur);
        let (mut best, mut best_score) = (cur.clone(), cur_score);
        let ratio = cfg.end_temperature / cfg.start_temperature;
        for i in 0..cfg.iterations {
            let temp = cfg.start_temperature * ratio.powf(i as f64 / cfg.iterations as f64);
            let mut next = cur.clone();
            for _ in 0..1 + rng.below(2) {
                self.mutate(&mut next, &mut rng, cfg);
            }
            let s = self.score(&next);
            if s >= cur_score || rng.unit() < ((s - cur_score) / temp).exp() {
                cur = next;
                cur_score = s;
                if s > best_score {
                    best = cur.clone();
                    best_score = s;
                }
            }
        }
        (best, best_score, cfg.iterations + 1)
    }

    /// Deletes orders that do not lower the score, first to last.
    pub fn prune(&self, plan: &Plan) -> (Plan, f64) {
        let mut plan = plan.clone();
        let mut score = self.score(&plan);
        loop {
            let mut changed = false;
            for q in plan.queues() {
                let mut i = 0;
                while i < plan.queue(q).len() {
                    let mut p = plan.clone();
                    p.queue_mut(q).remove(i);
                    p.trim();
                    let s = self.score(&p);
                    if s >= score {
                        plan = p;
                        score = s;
                        changed = true;
                        if !plan.queues().contains(&q) {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            if !changed {
                return (plan, score);
            }
        }
    }

    pub fn search(&self, cfg: &SearchConfig, start: Option<&Plan>) -> SearchResult {
        let t0 = Instant::now();
        let start = start.cloned().unwrap_or_else(|| self.initial_plan());
        let results: Vec<(Plan, f64, usize)> = std::thread::scope(|scope| {
            let mut out = Vec::new();
            let mut pending: Vec<usize> = (0..cfg.runs).collect();
            while !pending.is_empty() {
                let batch: Vec<usize> = pending.drain(..cfg.threads.min(pending.len()).max(1)).collect();
                let handles: Vec<_> = batch
                    .iter()
                    .map(|&r| {
                        let start = &start;
                        scope.spawn(move || {
                            self.anneal(start, cfg, cfg.seed.wrapping_mul(1_000_003).wrapping_add(r as u64))
                        })
                    })
                    .collect();
                out.extend(handles.into_iter().map(|h| h.join().expect("search thread")));
            }
            out
        });
        let run_scores = results.iter().map(|r| r.1).collect();
        let evaluations = results.iter().map(|r| r.2).sum();
        let best = results.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).expect("at least one run");
        let (plan, score) = self.prune(&best.0);
        SearchResult { plan, score, run_scores, evaluations, seconds: t0.elapsed().as_secs_f64() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::testing::catalog;
    use opener_format::{MetalSpot, Pos};

    fn world() -> World {
        let mut spots = Vec::new();
        for i in 0..12 {
            let a = i as f64 * 0.52;
            let r = 150.0 + 60.0 * i as f64;
            spots.push(MetalSpot { x: 1500.0 + r * a.cos(), z: 1500.0 + r * a.sin(), metal: 2.0 });
        }
        World {
            map_size: Pos { x: 4096.0, z: 4096.0 },
            start: Pos { x: 1500.0, z: 1500.0 },
            spots,
            wind: 10.0,
            tidal: 0.0,
        }
    }

    #[test]
    fn rng_is_deterministic_and_in_range() {
        let (mut a, mut b) = (Rng::new(7), Rng::new(7));
        for _ in 0..100 {
            let x = a.below(10);
            assert_eq!(x, b.below(10));
            assert!(x < 10);
            let u = a.unit();
            b.unit();
            assert!((0.0..1.0).contains(&u));
        }
    }

    #[test]
    fn search_beats_initial_plan() {
        let c = catalog();
        let w = world();
        let s = Settings { horizon_s: 180.0, ..Settings::default() };
        let o = Objective::default_for(&c);
        let p = Problem::new(&c, &w, &s, &o);
        let init = p.score(&p.initial_plan());
        let cfg = SearchConfig { iterations: 3000, runs: 2, threads: 2, ..SearchConfig::default() };
        let r = p.search(&cfg, None);
        assert!(r.score >= init, "{} < {}", r.score, init);
        assert!((p.score(&r.plan) - r.score).abs() < 1e-9);
        // Deterministic for a fixed seed.
        let again = p.search(&cfg, None);
        assert_eq!(again.plan, r.plan);
    }

    #[test]
    fn prune_removes_useless_orders() {
        let c = catalog();
        let w = world();
        let s = Settings { horizon_s: 60.0, ..Settings::default() };
        let o = Objective::default_for(&c);
        let p = Problem::new(&c, &w, &s, &o);
        let mex = c.id("mex").unwrap();
        // The factory queue has no factory: its orders never run.
        let plan = Plan { commander: vec![mex], factories: vec![vec![c.id("ck").unwrap()]], constructors: vec![] };
        let (pruned, _) = p.prune(&plan);
        assert_eq!(pruned.commander, vec![mex]);
        assert!(pruned.factories.is_empty());
    }
}
