//! Time-step simulation of one team's economy and builders.
//!
//! The model follows the Recoil engine and BAR gadgets where it matters
//! for an opener:
//!
//! - A builder with build power `w` adds `w / build_time` progress per
//!   second to its target and uses `metal_cost * w / build_time` metal and
//!   `energy_cost * w / build_time` energy per second.
//! - When the team cannot pay for all construction in a step, every
//!   builder progresses by the same fraction (the stall factor).
//! - Metal extractors need their energy upkeep; without it they extract
//!   only the paid fraction.
//! - Energy converters (BAR `game_energy_conversion.lua`) convert the
//!   energy above 75 % of the energy storage, up to their capacity.
//! - Resources above the storage are lost.
//! - Mobile builders walk in a straight line at their speed until the
//!   target is within build range (build distance plus target radius).
//!
//! Not modelled: terrain, pathing, acceleration and turning, build
//! placement rules beyond overlap, wind variation (the average is used),
//! reclaim.

use opener_format::{Action, MetalSpot, Pos, Sample, Waypoint};

use crate::catalog::{Catalog, UnitId};
use crate::plan::{Plan, QueueRef};

/// The static environment of a simulation.
#[derive(Debug, Clone)]
pub struct World {
    pub map_size: Pos,
    pub start: Pos,
    pub spots: Vec<MetalSpot>,
    /// Average wind speed; a wind generator makes `min(wind, its max)`.
    pub wind: f64,
    /// Energy per second of a tidal generator.
    pub tidal: f64,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub horizon_s: f64,
    pub dt: f64,
    pub start_metal: f64,
    pub start_energy: f64,
    pub start_metal_storage: f64,
    pub start_energy_storage: f64,
    /// Energy storage fraction above which converters work.
    pub converter_level: f64,
    /// Interval of the samples in a trace.
    pub sample_interval_s: f64,
}

impl Default for Settings {
    /// BAR defaults: 1000 metal and energy, 1000 storage each
    /// (`game_team_resources.lua`), converters at 75 %.
    fn default() -> Self {
        Settings {
            horizon_s: 300.0,
            dt: 0.1,
            start_metal: 1000.0,
            start_energy: 1000.0,
            start_metal_storage: 1000.0,
            start_energy_storage: 1000.0,
            converter_level: 0.75,
            sample_interval_s: 1.0,
        }
    }
}

/// State at the horizon.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Outcome {
    /// Metal per second from extractors and units (no converters).
    pub metal_income: f64,
    /// Energy per second from all producers.
    pub energy_income: f64,
    /// Energy per second used by upkeep.
    pub energy_upkeep: f64,
    pub metal: f64,
    pub energy: f64,
    pub build_power: f64,
    /// Metal and energy lost because the storage was full.
    pub metal_wasted: f64,
    pub energy_wasted: f64,
    /// Orders that were skipped (not buildable, no free metal spot).
    pub skipped_orders: u32,
}

/// Full timeline of a simulation, for export.
#[derive(Debug, Clone, Default)]
pub struct Trace {
    pub builders: Vec<TraceBuilder>,
    pub steps: Vec<TraceStep>,
    pub samples: Vec<Sample>,
    pub tracks: Vec<Vec<Waypoint>>,
}

#[derive(Debug, Clone)]
pub struct TraceBuilder {
    pub unit: UnitId,
    pub created_s: f64,
    pub created_by_step: Option<usize>,
    pub build_power: f64,
}

#[derive(Debug, Clone)]
pub struct TraceStep {
    pub builder: usize,
    pub action: Action,
    pub unit: UnitId,
    pub target_step: Option<usize>,
    pub pos: Pos,
    pub queued_s: f64,
    pub start_s: Option<f64>,
    pub finish_s: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Job {
    Idle,
    Build(usize),
    Assist(usize),
}

#[derive(Debug, Clone)]
struct Builder {
    unit: UnitId,
    pos: Pos,
    queue: Option<QueueRef>,
    next: usize,
    job: Job,
    /// Trace step of the current job.
    step: usize,
    moving: bool,
    /// The queue is exhausted; the builder only assists.
    done_queue: bool,
}

#[derive(Debug, Clone)]
struct Construction {
    unit: UnitId,
    pos: Pos,
    progress: f64,
    started: bool,
    done: bool,
    by_factory: bool,
    /// Radius for the build range: the factory radius for factory production.
    radius: f64,
    spot: Option<usize>,
    step: usize,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x0: f64,
    z0: f64,
    x1: f64,
    z1: f64,
}

impl Rect {
    fn around(p: Pos, fx: u32, fz: u32) -> Rect {
        let (hx, hz) = (4.0 * fx as f64, 4.0 * fz as f64);
        Rect { x0: p.x - hx, z0: p.z - hz, x1: p.x + hx, z1: p.z + hz }
    }

    fn overlaps(&self, o: &Rect, gap: f64) -> bool {
        self.x0 < o.x1 + gap && o.x0 < self.x1 + gap && self.z0 < o.z1 + gap && o.z0 < self.z1 + gap
    }

    fn distance_to(&self, p: Pos) -> f64 {
        let dx = (self.x0 - p.x).max(p.x - self.x1).max(0.0);
        let dz = (self.z0 - p.z).max(p.z - self.z1).max(0.0);
        dx.hypot(dz)
    }
}

/// Free space kept around a metal spot so that an extractor fits.
const SPOT_CLEARANCE: f64 = 24.0;
/// Gap between two buildings, so that units can walk between them.
const BUILDING_GAP: f64 = 8.0;

pub struct Sim<'a> {
    catalog: &'a Catalog,
    world: &'a World,
    settings: &'a Settings,
    plan: &'a Plan,
    t: f64,
    ticks: u64,
    metal: f64,
    energy: f64,
    metal_storage: f64,
    energy_storage: f64,
    base_metal: f64,
    mex_metal: f64,
    energy_make: f64,
    upkeep: f64,
    conv_capacity: f64,
    conv_efficiency: f64,
    builders: Vec<Builder>,
    cons: Vec<Construction>,
    rects: Vec<Rect>,
    spot_taken: Vec<bool>,
    factories_built: usize,
    constructors_built: usize,
    out: Outcome,
    trace: Option<Trace>,
    steps_count: usize,
    last_metal_used: f64,
    last_energy_used: f64,
    last_metal_wasted: f64,
    last_energy_wasted: f64,
    last_bp_used: f64,
}

impl<'a> Sim<'a> {
    pub fn new(catalog: &'a Catalog, world: &'a World, settings: &'a Settings, plan: &'a Plan) -> Self {
        let mut sim = Sim {
            catalog,
            world,
            settings,
            plan,
            t: 0.0,
            ticks: 0,
            metal: settings.start_metal,
            energy: settings.start_energy,
            metal_storage: settings.start_metal_storage,
            energy_storage: settings.start_energy_storage,
            base_metal: 0.0,
            mex_metal: 0.0,
            energy_make: 0.0,
            upkeep: 0.0,
            conv_capacity: 0.0,
            conv_efficiency: 0.0,
            builders: Vec::new(),
            cons: Vec::new(),
            rects: Vec::new(),
            spot_taken: vec![false; world.spots.len()],
            factories_built: 0,
            constructors_built: 0,
            out: Outcome::default(),
            trace: None,
            steps_count: 0,
            last_metal_used: 0.0,
            last_energy_used: 0.0,
            last_metal_wasted: 0.0,
            last_energy_wasted: 0.0,
            last_bp_used: 0.0,
        };
        let com = catalog.commander;
        let u = catalog.get(com);
        // Team storage already includes the commander (game_team_resources.lua).
        sim.base_metal += u.metal_make;
        sim.energy_make += u.energy_make;
        sim.rects.clear();
        sim.builders.push(Builder {
            unit: com,
            pos: world.start,
            queue: Some(QueueRef::Commander),
            next: 0,
            job: Job::Idle,
            step: usize::MAX,
            moving: false,
            done_queue: false,
        });
        sim
    }

    pub fn with_trace(mut self) -> Self {
        let mut trace = Trace::default();
        let com = self.catalog.commander;
        trace.builders.push(TraceBuilder {
            unit: com,
            created_s: 0.0,
            created_by_step: None,
            build_power: self.catalog.get(com).build_power,
        });
        trace.tracks.push(vec![Waypoint { t_s: 0.0, x: self.world.start.x, z: self.world.start.z }]);
        self.trace = Some(trace);
        self
    }

    /// Runs to the horizon.
    pub fn run(mut self) -> (Outcome, Option<Trace>) {
        let steps = (self.settings.horizon_s / self.settings.dt).round() as usize;
        let sample_every = (self.settings.sample_interval_s / self.settings.dt).round().max(1.0) as usize;
        self.sample();
        for i in 0..steps {
            self.step();
            if (i + 1) % sample_every == 0 {
                self.sample();
            }
        }
        self.finish()
    }

    fn finish(mut self) -> (Outcome, Option<Trace>) {
        self.out.metal_income = self.base_metal + self.mex_metal;
        self.out.energy_income = self.energy_make;
        self.out.energy_upkeep = self.upkeep;
        self.out.metal = self.metal;
        self.out.energy = self.energy;
        self.out.build_power = self.builders.iter().map(|b| self.catalog.get(b.unit).build_power).sum();
        if let Some(trace) = &mut self.trace {
            for (i, b) in self.builders.iter().enumerate() {
                let track = &mut trace.tracks[i];
                if track.last().is_none_or(|w| w.x != b.pos.x || w.z != b.pos.z || w.t_s < self.t) {
                    track.push(Waypoint { t_s: self.t, x: b.pos.x, z: b.pos.z });
                }
            }
        }
        (self.out, self.trace)
    }

    fn new_step(&mut self, builder: usize, action: Action, unit: UnitId, pos: Pos, target: Option<usize>) -> usize {
        let idx = self.steps_count;
        self.steps_count += 1;
        if let Some(trace) = &mut self.trace {
            trace.steps.push(TraceStep {
                builder,
                action,
                unit,
                target_step: target,
                pos,
                queued_s: self.t,
                start_s: None,
                finish_s: None,
            });
        }
        idx
    }

    fn step_start(&mut self, step: usize) {
        let t = self.t;
        if let Some(trace) = &mut self.trace {
            trace.steps[step].start_s.get_or_insert(t);
        }
    }

    fn step_finish(&mut self, step: usize) {
        let t = self.t;
        if let Some(trace) = &mut self.trace {
            let s = &mut trace.steps[step];
            s.start_s.get_or_insert(t);
            s.finish_s = Some(t);
        }
    }

    fn step(&mut self) {
        let dt = self.settings.dt;
        for b in 0..self.builders.len() {
            if self.builders[b].job == Job::Idle {
                self.take_order(b);
            }
        }
        self.move_builders(dt);

        // Construction demand of the builders that are in range.
        let (mut dm, mut de, mut bp) = (0.0, 0.0, 0.0);
        for b in &self.builders {
            if let Job::Build(c) | Job::Assist(c) = b.job {
                let c = &self.cons[c];
                if c.started && !c.done && !b.moving {
                    let u = self.catalog.get(c.unit);
                    let w = self.catalog.get(b.unit).build_power;
                    let rate = w / u.build_time * dt;
                    dm += rate * u.metal_cost;
                    de += rate * u.energy_cost;
                    bp += w;
                }
            }
        }

        // Income and upkeep.
        self.energy += self.energy_make * dt;
        let need = self.upkeep * dt;
        let paid = if need > 0.0 { (self.energy / need).min(1.0) } else { 1.0 };
        self.energy -= need * paid;
        self.metal += (self.base_metal + self.mex_metal * paid) * dt;

        // Construction, scaled down when resources run out.
        let mut f: f64 = 1.0;
        if dm > 0.0 {
            f = f.min(self.metal / dm);
        }
        if de > 0.0 {
            f = f.min(self.energy / de);
        }
        let f = f.clamp(0.0, 1.0);
        self.metal -= dm * f;
        self.energy -= de * f;
        self.last_metal_used = dm * f / dt;
        self.last_energy_used = (de * f + need * paid) / dt;
        self.last_bp_used = bp * f;

        let mut finished = Vec::new();
        for b in 0..self.builders.len() {
            if let Job::Build(c) | Job::Assist(c) = self.builders[b].job {
                let w = self.catalog.get(self.builders[b].unit).build_power;
                let con = &mut self.cons[c];
                if con.started && !con.done && !self.builders[b].moving {
                    con.progress += w / self.catalog.get(con.unit).build_time * dt * f;
                    if con.progress >= 1.0 - 1e-9 {
                        con.done = true;
                        finished.push(c);
                    }
                }
            }
        }

        // Converters use the energy above the converter level.
        if self.conv_capacity > 0.0 {
            let excess = self.energy - self.energy_storage * self.settings.converter_level;
            if excess > 0.0 {
                let e = excess.min(self.conv_capacity * dt);
                self.energy -= e;
                self.metal += e * self.conv_efficiency;
                self.last_energy_used += e / dt;
            }
        }

        let (wm, we) = ((self.metal - self.metal_storage).max(0.0), (self.energy - self.energy_storage).max(0.0));
        self.metal -= wm;
        self.energy -= we;
        self.out.metal_wasted += wm;
        self.out.energy_wasted += we;
        self.last_metal_wasted = wm / dt;
        self.last_energy_wasted = we / dt;

        self.ticks += 1;
        self.t = self.ticks as f64 * dt;
        for c in finished {
            self.complete(c);
        }
    }

    fn sample(&mut self) {
        let Some(trace) = &mut self.trace else { return };
        let build_power = self.builders.iter().map(|b| self.catalog.get(b.unit).build_power).sum();
        trace.samples.push(Sample {
            t_s: (self.t * 1000.0).round() / 1000.0,
            metal: self.metal,
            energy: self.energy,
            metal_storage: self.metal_storage,
            energy_storage: self.energy_storage,
            metal_income: self.base_metal + self.mex_metal,
            energy_income: self.energy_make,
            metal_usage: self.last_metal_used,
            energy_usage: self.last_energy_used,
            metal_wasted: self.last_metal_wasted,
            energy_wasted: self.last_energy_wasted,
            build_power,
            build_power_used: self.last_bp_used,
        });
    }

    fn take_order(&mut self, b: usize) {
        loop {
            let builder = &self.builders[b];
            let Some(q) = builder.queue.filter(|_| !builder.done_queue) else { break };
            let Some(&unit) = self.plan.queue(q).get(builder.next) else {
                self.builders[b].done_queue = true;
                break;
            };
            self.builders[b].next += 1;
            if self.start_build(b, unit) {
                return;
            }
            self.out.skipped_orders += 1;
        }
        self.start_assist(b);
    }

    fn start_build(&mut self, b: usize, unit: UnitId) -> bool {
        let bu = self.catalog.get(self.builders[b].unit);
        if !bu.can_build(unit) {
            return false;
        }
        let u = self.catalog.get(unit);
        let factory = !bu.is_mobile();
        let here = self.builders[b].pos;
        let (pos, spot) = if factory {
            (here, None)
        } else if u.extracts_metal > 0.0 {
            let Some(s) = self.nearest_free_spot(here) else { return false };
            self.spot_taken[s] = true;
            (Pos { x: self.world.spots[s].x, z: self.world.spots[s].z }, Some(s))
        } else {
            let Some(p) = self.place(here, u.footprint) else { return false };
            (p, None)
        };
        if !factory {
            self.rects.push(Rect::around(pos, u.footprint.0, u.footprint.1));
        }
        let step = self.new_step(b, Action::Build, unit, pos, None);
        self.cons.push(Construction {
            unit,
            pos,
            progress: 0.0,
            started: factory,
            done: false,
            by_factory: factory,
            radius: if factory { bu.radius } else { u.radius },
            spot,
            step,
        });
        let builder = &mut self.builders[b];
        builder.job = Job::Build(self.cons.len() - 1);
        builder.step = step;
        if factory {
            self.step_start(step);
        }
        true
    }

    fn start_assist(&mut self, b: usize) {
        let builder = &self.builders[b];
        let bu = self.catalog.get(builder.unit);
        let mobile = bu.is_mobile();
        // A factory does not assist; a static builder assists within range.
        if !mobile && !bu.build_options.is_empty() {
            return;
        }
        let mut best: Option<(bool, f64, usize)> = None;
        for (i, c) in self.cons.iter().enumerate() {
            if !c.started || c.done {
                continue;
            }
            let d = builder.pos.distance(c.pos);
            if !mobile && d > bu.build_distance + c.radius {
                continue;
            }
            let key = (!c.by_factory, d, i);
            if best.is_none_or(|bk| (key.0, key.1) < (bk.0, bk.1)) {
                best = Some(key);
            }
        }
        if let Some((_, _, c)) = best {
            let (unit, pos, target) = (self.cons[c].unit, self.cons[c].pos, self.cons[c].step);
            let step = self.new_step(b, Action::Assist, unit, pos, Some(target));
            let builder = &mut self.builders[b];
            builder.job = Job::Assist(c);
            builder.step = step;
        }
    }

    fn move_builders(&mut self, dt: f64) {
        for b in 0..self.builders.len() {
            let (Job::Build(c) | Job::Assist(c)) = self.builders[b].job else { continue };
            let bu = self.catalog.get(self.builders[b].unit);
            let target = self.cons[c].pos;
            let range = bu.build_distance + self.cons[c].radius;
            let pos = self.builders[b].pos;
            let d = pos.distance(target);
            let was_moving = self.builders[b].moving;
            let mut moved_to = pos;
            let arrived = if d <= range + 1e-6 {
                true
            } else if bu.is_mobile() {
                let step = bu.speed * dt;
                let go = (d - range).min(step);
                moved_to = Pos { x: pos.x + (target.x - pos.x) / d * go, z: pos.z + (target.z - pos.z) / d * go };
                go >= d - range - 1e-6
            } else {
                false
            };
            if !was_moving && !arrived {
                self.waypoint(b, pos);
            }
            self.builders[b].pos = moved_to;
            self.builders[b].moving = !arrived;
            if arrived {
                if was_moving {
                    let t = self.t + dt;
                    if let Some(trace) = &mut self.trace {
                        trace.tracks[b].push(Waypoint { t_s: t, x: moved_to.x, z: moved_to.z });
                    }
                }
                let step = self.builders[b].step;
                if let Job::Build(_) = self.builders[b].job {
                    self.cons[c].started = true;
                }
                self.step_start(step);
            }
        }
    }

    fn waypoint(&mut self, b: usize, p: Pos) {
        let t = self.t;
        if let Some(trace) = &mut self.trace {
            let track = &mut trace.tracks[b];
            if track.last().is_none_or(|w| w.t_s < t || w.x != p.x || w.z != p.z) {
                track.push(Waypoint { t_s: t, x: p.x, z: p.z });
            }
        }
    }

    fn complete(&mut self, c: usize) {
        let unit = self.cons[c].unit;
        let u = self.catalog.get(unit);
        let step = self.cons[c].step;
        self.step_finish(step);
        for b in 0..self.builders.len() {
            if let Job::Build(x) | Job::Assist(x) = self.builders[b].job {
                if x == c {
                    if let Job::Assist(_) = self.builders[b].job {
                        let s = self.builders[b].step;
                        self.step_finish(s);
                    }
                    self.builders[b].job = Job::Idle;
                    self.builders[b].moving = false;
                }
            }
        }

        self.base_metal += u.metal_make;
        if let Some(s) = self.cons[c].spot {
            self.mex_metal += self.world.spots[s].metal * u.extracts_metal / 0.001;
        }
        self.energy_make +=
            u.energy_make + u.wind.min(self.world.wind) + if u.tidal > 0.0 { self.world.tidal } else { 0.0 };
        self.upkeep += u.energy_upkeep;
        self.metal_storage += u.metal_storage;
        self.energy_storage += u.energy_storage;
        if let Some((cap, eff)) = u.conversion {
            self.conv_capacity += cap;
            self.conv_efficiency = self.conv_efficiency.max(eff);
        }

        if u.is_builder() {
            let pos = self.cons[c].pos;
            let queue = if !u.is_mobile() && !u.build_options.is_empty() {
                self.factories_built += 1;
                Some(QueueRef::Factory(self.factories_built - 1))
            } else if u.is_mobile() {
                self.constructors_built += 1;
                Some(QueueRef::Constructor(self.constructors_built - 1))
            } else {
                None
            };
            let spawn = if self.cons[c].by_factory {
                // Leaves the factory to the front (south).
                let fr = self
                    .builders
                    .iter()
                    .find(|b| b.pos == pos && !self.catalog.get(b.unit).is_mobile())
                    .map_or(0.0, |b| self.catalog.get(b.unit).radius);
                Pos { x: pos.x, z: (pos.z + fr + 16.0).min(self.world.map_size.z) }
            } else {
                pos
            };
            self.builders.push(Builder {
                unit,
                pos: spawn,
                queue,
                next: 0,
                job: Job::Idle,
                step: usize::MAX,
                moving: false,
                done_queue: false,
            });
            let t = self.t;
            if let Some(trace) = &mut self.trace {
                trace.builders.push(TraceBuilder {
                    unit,
                    created_s: t,
                    created_by_step: Some(step),
                    build_power: u.build_power,
                });
                trace.tracks.push(vec![Waypoint { t_s: t, x: spawn.x, z: spawn.z }]);
            }
        }
    }

    fn nearest_free_spot(&self, p: Pos) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        for (i, s) in self.world.spots.iter().enumerate() {
            if self.spot_taken[i] {
                continue;
            }
            let d = p.distance(Pos { x: s.x, z: s.z });
            if best.is_none_or(|(bd, _)| d < bd) {
                best = Some((d, i));
            }
        }
        best.map(|(_, i)| i)
    }

    /// Nearest free build position to `near`, on the 16 elmo grid.
    fn place(&self, near: Pos, footprint: (u32, u32)) -> Option<Pos> {
        const GRID: f64 = 16.0;
        let snap = |v: f64| (v / GRID).round() * GRID;
        let (cx, cz) = (snap(near.x), snap(near.z));
        for ring in 0..64i32 {
            let mut best: Option<(f64, Pos)> = None;
            for i in -ring..=ring {
                for (dx, dz) in [(i, -ring), (i, ring), (-ring, i), (ring, i)] {
                    let p = Pos { x: cx + dx as f64 * GRID, z: cz + dz as f64 * GRID };
                    if self.free(p, footprint) {
                        let d = p.distance(near);
                        if best.is_none_or(|(bd, _)| d < bd) {
                            best = Some((d, p));
                        }
                    }
                }
            }
            if let Some((_, p)) = best {
                return Some(p);
            }
        }
        None
    }

    fn free(&self, p: Pos, footprint: (u32, u32)) -> bool {
        let r = Rect::around(p, footprint.0, footprint.1);
        if r.x0 < 0.0 || r.z0 < 0.0 || r.x1 > self.world.map_size.x || r.z1 > self.world.map_size.z {
            return false;
        }
        if self.rects.iter().any(|o| o.overlaps(&r, BUILDING_GAP)) {
            return false;
        }
        !self.world.spots.iter().any(|s| r.distance_to(Pos { x: s.x, z: s.z }) < SPOT_CLEARANCE)
    }
}

/// Runs `plan` without a trace.
pub fn simulate(catalog: &Catalog, world: &World, settings: &Settings, plan: &Plan) -> Outcome {
    Sim::new(catalog, world, settings, plan).run().0
}

/// Runs `plan` and records the full timeline.
pub fn simulate_traced(catalog: &Catalog, world: &World, settings: &Settings, plan: &Plan) -> (Outcome, Trace) {
    let (out, trace) = Sim::new(catalog, world, settings, plan).with_trace().run();
    (out, trace.expect("trace enabled"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::testing::catalog;

    fn world() -> World {
        World {
            map_size: Pos { x: 4096.0, z: 4096.0 },
            start: Pos { x: 1000.0, z: 1000.0 },
            spots: vec![
                MetalSpot { x: 1100.0, z: 1000.0, metal: 2.0 },
                MetalSpot { x: 1000.0, z: 1150.0, metal: 2.0 },
                MetalSpot { x: 2000.0, z: 1000.0, metal: 2.0 },
            ],
            wind: 10.0,
            tidal: 0.0,
        }
    }

    fn plan(c: &Catalog, com: &[&str], fac: &[&[&str]], cons: &[&[&str]]) -> Plan {
        let ids = |v: &[&str]| v.iter().map(|n| c.id(n).unwrap()).collect::<Vec<_>>();
        Plan {
            commander: ids(com),
            factories: fac.iter().map(|v| ids(v)).collect(),
            constructors: cons.iter().map(|v| ids(v)).collect(),
        }
    }

    #[test]
    fn idle_commander_keeps_base_income() {
        let c = catalog();
        let s = Settings { horizon_s: 10.0, ..Settings::default() };
        let out = simulate(&c, &world(), &s, &Plan::default());
        assert_eq!(out.metal_income, 2.0);
        assert_eq!(out.energy_income, 30.0);
        // Storage is full from the start: income is wasted.
        assert!((out.metal_wasted - 20.0).abs() < 1e-6, "{}", out.metal_wasted);
    }

    #[test]
    fn mex_build_time_matches_build_power() {
        let c = catalog();
        let s = Settings { horizon_s: 30.0, ..Settings::default() };
        let p = plan(&c, &["mex"], &[], &[]);
        let (out, trace) = simulate_traced(&c, &world(), &s, &p);
        assert_eq!(out.metal_income, 4.0);
        assert_eq!(out.energy_upkeep, 3.0);
        let st = &trace.steps[0];
        // Spot is 100 elmos away, inside build range (145 + 16): no walk.
        assert_eq!(st.start_s, Some(0.0));
        // 1800 build time / 300 build power = 6 s.
        assert!((st.finish_s.unwrap() - 6.0).abs() < 0.11, "{:?}", st.finish_s);
    }

    #[test]
    fn walks_to_far_spot() {
        let c = catalog();
        let s = Settings { horizon_s: 60.0, ..Settings::default() };
        let p = plan(&c, &["mex", "mex", "mex"], &[], &[]);
        let (_, trace) = simulate_traced(&c, &world(), &s, &p);
        let far = trace.steps.iter().find(|st| st.pos.x == 2000.0).unwrap();
        // ~ (1000 - 161) / 37.5 = 22 s of walking after the previous order.
        let walk = far.start_s.unwrap() - far.queued_s;
        assert!(walk > 18.0 && walk < 26.0, "walk {walk}");
        assert!(trace.tracks[0].len() >= 2);
    }

    #[test]
    fn energy_stall_slows_construction() {
        let c = catalog();
        // No starting energy, only the commander's 30 E/s for a 500 E mex.
        let s = Settings { horizon_s: 40.0, start_energy: 0.0, ..Settings::default() };
        let p = plan(&c, &["mex"], &[], &[]);
        let (_, trace) = simulate_traced(&c, &world(), &s, &p);
        let fin = trace.steps[0].finish_s.unwrap();
        // 500 E at 30 E/s takes about 16.7 s instead of 6 s.
        assert!(fin > 15.0 && fin < 18.5, "finish {fin}");
    }

    #[test]
    fn factory_makes_constructor_that_follows_its_queue() {
        let c = catalog();
        let s = Settings { horizon_s: 120.0, ..Settings::default() };
        let p = plan(&c, &["lab"], &[&["ck"]], &[&["mex"]]);
        let (out, trace) = simulate_traced(&c, &world(), &s, &p);
        assert_eq!(trace.builders.len(), 3);
        let con = &trace.builders[2];
        assert_eq!(c.get(con.unit).name, "ck");
        // The commander has no orders left and assists the lab.
        assert!(trace.steps.iter().any(|st| st.action == Action::Assist && st.builder == 0));
        let mex = trace.steps.iter().find(|st| c.get(st.unit).name == "mex").unwrap();
        assert_eq!(mex.builder, 2);
        assert!(mex.finish_s.is_some());
        assert!(out.metal_income > 2.0);
    }

    #[test]
    fn skips_orders_without_free_spot() {
        let c = catalog();
        let s = Settings { horizon_s: 120.0, ..Settings::default() };
        let p = plan(&c, &["mex", "mex", "mex", "mex"], &[], &[]);
        let out = simulate(&c, &world(), &s, &p);
        assert_eq!(out.skipped_orders, 1);
        assert_eq!(out.metal_income, 8.0);
    }

    #[test]
    fn buildings_do_not_overlap() {
        let c = catalog();
        let s = Settings { horizon_s: 200.0, ..Settings::default() };
        let p = plan(&c, &["solar", "solar", "wind", "wind", "lab"], &[], &[]);
        let (_, trace) = simulate_traced(&c, &world(), &s, &p);
        let rects: Vec<Rect> = trace
            .steps
            .iter()
            .filter(|st| st.action == Action::Build)
            .map(|st| {
                let u = c.get(st.unit);
                Rect::around(st.pos, u.footprint.0, u.footprint.1)
            })
            .collect();
        for (i, a) in rects.iter().enumerate() {
            for b in &rects[i + 1..] {
                assert!(!a.overlaps(b, 0.0));
            }
        }
    }

    #[test]
    fn converter_turns_excess_energy_into_metal() {
        let c = catalog();
        let s = Settings { horizon_s: 120.0, start_metal: 0.0, ..Settings::default() };
        let p = plan(&c, &["makr"], &[], &[]);
        let (out, trace) = simulate_traced(&c, &world(), &s, &p);
        let last = trace.samples.last().unwrap();
        assert!(last.energy <= 0.75 * last.energy_storage + 1.0);
        // Income counts production only, not conversion.
        assert_eq!(out.metal_income, 2.0);
        assert!(out.metal > 2.0 * 100.0);
    }

    #[test]
    fn samples_cover_horizon() {
        let c = catalog();
        let s = Settings { horizon_s: 30.0, ..Settings::default() };
        let (_, trace) = simulate_traced(&c, &world(), &s, &Plan::default());
        assert_eq!(trace.samples.len(), 31);
        assert_eq!(trace.samples.last().unwrap().t_s, 30.0);
    }
}
