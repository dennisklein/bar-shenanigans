//! Frame-accurate simulation of a tick spam build.
//!
//! The model follows the Recoil engine (BAR's engine) at 30 frames per
//! second. These engine rules decide the result, so the code copies them:
//!
//! - Unit slow updates are staggered. Every 15 frames the engine walks the
//!   unit list from the start and slow-updates `n / 15 + 1` units per frame
//!   (`CUnitHandler::SlowUpdateUnits`). Command AI, income and upkeep run
//!   only in the slow update.
//! - Income comes in half-second chunks at each unit's slow update
//!   (`CUnit::SlowUpdate`). A mex first pays its energy upkeep; if the team
//!   does not have it, the mex makes no metal for that chunk.
//! - Build power is all or nothing per builder and frame. A builder adds
//!   `step = min(power / buildtime, 1 - progress)` only if the team has
//!   `cost * step` of both resources, in unit list order
//!   (`CUnit::AddBuildPower`).
//! - A factory queues its next unit only in its own slow update after the
//!   previous unit is complete (`CFactoryCAI::SlowUpdate`), and starts it
//!   when the build pad is clear (`CFactory::StartBuild`).
//! - A builder that guards a factory needs two slow updates to join a new
//!   unit: the first pushes a repair order, the second executes it
//!   (`CBuilderCAI::ExecuteGuard`). A builder on fight or patrol needs one
//!   (`CBuilderCAI::ExecuteFight`).
//! - A mobile builder starts a structure in a slow update after it is in
//!   build range and its build animation is done (`CBuilder::StartBuild`).

use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, Role};

pub const GAME_SPEED: u32 = 30;
pub const UNIT_SLOWUPDATE_RATE: u32 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Pos {
    pub x: f32,
    pub z: f32,
}

impl Pos {
    pub fn new(x: f32, z: f32) -> Pos {
        Pos { x, z }
    }

    pub fn distance(self, o: Pos) -> f32 {
        (self.x - o.x).hypot(self.z - o.z)
    }
}

/// How an assisting builder follows its lab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistMode {
    /// Guard order on the lab. Joins a new unit two slow updates after it starts.
    Guard,
    /// Fight or patrol order next to the lab. Joins after one slow update.
    Fight,
}

/// Timing values that the engine does not expose as unit def values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    /// Frames after a tick is complete until it leaves the build pad and the
    /// lab can start the next one. Estimate from tick acceleration
    /// (0.575 elmo/frame²) and the 8 elmo map square: about 6 to 10 frames.
    pub pad_clear_frames: u32,
    /// Frames for the lab's open animation before the first unit
    /// (`armlab.bos` Activate: 15° at 45°/s).
    pub yard_open_frames: u32,
    /// Extra slow updates that a mobile builder waits in range until its
    /// build animation sets the build stance (armcom: arm turn).
    pub stance_slow_updates: u32,
    /// How nanos assist. With `fight` a nano helps the nearest unit under
    /// construction in its range, so it can serve several labs.
    pub assist_mode: AssistMode,
    /// How the commander assists a lab at the end of its plan.
    pub commander_assist_mode: AssistMode,
    /// Wind strength that a wind turbine uses (capped at its generator value).
    pub wind: f32,
    pub start_metal: f32,
    pub start_energy: f32,
    /// Team storage at the start, without the commander's own storage.
    pub start_metal_storage: f32,
    pub start_energy_storage: f32,
}

impl Default for Rules {
    fn default() -> Self {
        Rules {
            pad_clear_frames: 8,
            yard_open_frames: 10,
            stance_slow_updates: 1,
            assist_mode: AssistMode::Fight,
            commander_assist_mode: AssistMode::Guard,
            wind: 10.0,
            start_metal: 1000.0,
            start_energy: 1000.0,
            start_metal_storage: 1000.0,
            start_energy_storage: 1000.0,
        }
    }
}

pub type UnitId = usize;

/// One commander order.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "order", rename_all = "snake_case")]
pub enum Order {
    /// Build a structure. For a mex, `metal` is the spot's metal per second.
    /// For a nano, `lab` is the index (in build order) of the lab it guards.
    Build {
        role: Role,
        pos: Pos,
        #[serde(default)]
        metal: f32,
        #[serde(default)]
        lab: Option<usize>,
    },
    /// Assist lab number `lab` (in build order) until the end.
    Assist { lab: usize },
}

#[derive(Debug, Clone)]
pub struct Unit {
    pub role: Role,
    pub pos: Pos,
    pub being_built: bool,
    pub progress: f32,
    pub created_frame: u32,
    pub finished_frame: Option<u32>,
    /// The builder that started this unit.
    pub builder: Option<UnitId>,
    /// Metal per second (mex only).
    pub metal_extract: f32,
    /// Lab index (in build order) that a nano guards.
    pub lab_index: Option<usize>,
    brain: Brain,
}

#[derive(Debug, Clone)]
enum Brain {
    Inert,
    Commander(Box<Commander>),
    Lab(LabState),
    Assist(AssistState),
}

#[derive(Debug, Clone, Default)]
struct LabState {
    /// `curBuildDef`: a tick is queued and waits for yard and pad.
    queued: bool,
    cur_build: Option<UnitId>,
    yard_ready: Option<u32>,
    pad_free: u32,
}

#[derive(Debug, Clone)]
struct AssistState {
    lab_index: usize,
    mode: AssistMode,
    /// Front command: repair of this unit.
    cmd_repair: Option<UnitId>,
    /// The unit that the builder adds build power to.
    cur_build: Option<UnitId>,
}

#[derive(Debug, Clone)]
enum CmdState {
    Idle,
    Approach {
        arrive: u32,
        stance_left: u32,
        order: Order,
    },
    Building {
        target: UnitId,
    },
    Assist(AssistState),
}

#[derive(Debug, Clone)]
struct Commander {
    orders: Vec<Order>,
    next: usize,
    state: CmdState,
}

/// Something that happened, for the timeline export.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// `builder` got order `order` (index into the commander's orders).
    Ordered {
        frame: u32,
        builder: UnitId,
        order: usize,
    },
    /// A mobile builder moves from `from` to `to`.
    Move {
        frame: u32,
        builder: UnitId,
        from: Pos,
        to: Pos,
        arrive: u32,
    },
    Started {
        frame: u32,
        builder: UnitId,
        unit: UnitId,
    },
    Finished {
        frame: u32,
        unit: UnitId,
    },
    AssistStart {
        frame: u32,
        builder: UnitId,
        unit: UnitId,
    },
    AssistStop {
        frame: u32,
        builder: UnitId,
        unit: UnitId,
    },
}

/// Economy totals over one second, for the timeline export.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub frame: u32,
    pub metal: f32,
    pub energy: f32,
    pub metal_storage: f32,
    pub energy_storage: f32,
    pub metal_income: f32,
    pub energy_income: f32,
    pub metal_usage: f32,
    pub energy_usage: f32,
    pub metal_wasted: f32,
    pub energy_wasted: f32,
    pub build_power: f32,
    pub build_power_used: f32,
}

#[derive(Debug, Clone)]
pub struct Team {
    pub metal: f32,
    pub energy: f32,
    pub metal_storage: f32,
    pub energy_storage: f32,
}

#[derive(Debug, Clone, Copy, Default)]
struct Acc {
    mi: f32,
    ei: f32,
    mu: f32,
    eu: f32,
    mw: f32,
    ew: f32,
    bp_used: f32,
}

/// The simulation state. Clone it to branch a search.
#[derive(Debug, Clone)]
pub struct Sim {
    pub catalog: Catalog,
    pub rules: Rules,
    pub frame: u32,
    pub team: Team,
    pub units: Vec<Unit>,
    /// Labs in build order (unit ids), including labs under construction.
    pub labs: Vec<UnitId>,
    pub ticks_done: u32,
    pub events: Vec<Event>,
    pub samples: Vec<Sample>,
    /// Record events and samples (off in the search, for speed).
    pub record: bool,
    su_iter: usize,
    acc: Acc,
}

impl Sim {
    /// A new game: the commander stands at `start` and executes `orders`.
    pub fn new(catalog: Catalog, rules: Rules, start: Pos, orders: Vec<Order>) -> Sim {
        let team = Team {
            metal: rules.start_metal,
            energy: rules.start_energy,
            metal_storage: rules.start_metal_storage + catalog.commander.metal_storage,
            energy_storage: rules.start_energy_storage + catalog.commander.energy_storage,
        };
        let mut sim = Sim {
            catalog,
            rules,
            frame: 0,
            team,
            units: Vec::new(),
            labs: Vec::new(),
            ticks_done: 0,
            events: Vec::new(),
            samples: Vec::new(),
            record: true,
            su_iter: 0,
            acc: Acc::default(),
        };
        sim.units.push(Unit {
            role: Role::Commander,
            pos: start,
            being_built: false,
            progress: 1.0,
            created_frame: 0,
            finished_frame: Some(0),
            builder: None,
            metal_extract: 0.0,
            lab_index: None,
            brain: Brain::Commander(Box::new(Commander {
                orders,
                next: 0,
                state: CmdState::Idle,
            })),
        });
        sim
    }

    /// Adds a complete unit, for test setups (for example a lab with nanos).
    pub fn spawn(&mut self, role: Role, pos: Pos, lab_index: Option<usize>) -> UnitId {
        let id = self.create(role, pos, None, 0.0, lab_index);
        self.units[id].progress = 1.0;
        self.finish(id);
        id
    }

    /// Adds `n` inert units in front of everything built later. Other
    /// players' units shift the slow update phases the same way, so tests
    /// can average over phases with it.
    pub fn pad_units(&mut self, n: usize) {
        for _ in 0..n {
            self.units.push(Unit {
                role: Role::Tick,
                pos: Pos::new(-1e6, -1e6),
                being_built: false,
                progress: 1.0,
                created_frame: self.frame,
                finished_frame: Some(self.frame),
                builder: None,
                metal_extract: 0.0,
                lab_index: None,
                brain: Brain::Inert,
            });
        }
    }

    /// Infinite resources, for configuration tests.
    pub fn set_unlimited_resources(&mut self) {
        self.team.metal = 1e9;
        self.team.energy = 1e9;
        self.team.metal_storage = 1e9;
        self.team.energy_storage = 1e9;
    }

    pub fn commander_orders(&self) -> &[Order] {
        match &self.units[0].brain {
            Brain::Commander(c) => &c.orders,
            _ => &[],
        }
    }

    /// Appends an order to the commander's queue.
    pub fn push_order(&mut self, order: Order) {
        if let Brain::Commander(c) = &mut self.units[0].brain {
            c.orders.push(order);
        }
    }

    /// True when the commander has no order left to start (it may still build).
    pub fn commander_queue_empty(&self) -> bool {
        match &self.units[0].brain {
            Brain::Commander(c) => c.next >= c.orders.len(),
            _ => true,
        }
    }

    /// True when the commander has finished all its orders and does nothing.
    pub fn commander_idle(&self) -> bool {
        match &self.units[0].brain {
            Brain::Commander(c) => c.next >= c.orders.len() && matches!(c.state, CmdState::Idle),
            _ => true,
        }
    }

    /// True when the commander assists a lab (its last order).
    pub fn commander_assisting(&self) -> bool {
        matches!(&self.units[0].brain, Brain::Commander(c) if matches!(c.state, CmdState::Assist(_)))
    }

    pub fn time_s(&self) -> f32 {
        self.frame as f32 / GAME_SPEED as f32
    }

    /// Ticks complete at the current frame.
    pub fn ticks(&self) -> u32 {
        self.ticks_done
    }

    /// Runs until `frame` (exclusive).
    pub fn run_until(&mut self, frame: u32) {
        while self.frame < frame {
            self.step();
        }
    }

    /// Runs until the commander has started its last queued order or `limit`.
    /// Returns false when `limit` was reached first.
    pub fn run_until_commander_free(&mut self, limit: u32) -> bool {
        while self.frame < limit {
            if self.commander_free_for_next_order() {
                return true;
            }
            self.step();
        }
        false
    }

    /// True when the commander would take a new order in its next slow update.
    fn commander_free_for_next_order(&self) -> bool {
        match &self.units[0].brain {
            Brain::Commander(c) => {
                c.next >= c.orders.len()
                    && match c.state {
                        CmdState::Idle => true,
                        CmdState::Building { target } => !self.units[target].being_built,
                        _ => false,
                    }
            }
            _ => true,
        }
    }

    /// One simulation frame.
    pub fn step(&mut self) {
        let f = self.frame;

        if f % UNIT_SLOWUPDATE_RATE == 0 {
            self.su_iter = 0;
        }
        let n = self.units.len();
        let count = (n / UNIT_SLOWUPDATE_RATE as usize + 1).min(n - self.su_iter.min(n));
        let beg = self.su_iter;
        self.su_iter += count;
        for i in beg..beg + count {
            self.slow_update(i);
        }

        let n = self.units.len();
        for i in 0..n {
            self.update(i);
        }

        self.frame += 1;
        if self.record && self.frame % GAME_SPEED == 0 {
            self.push_sample();
        }
    }

    fn push_sample(&mut self) {
        let a = std::mem::take(&mut self.acc);
        let bp: f32 = self
            .units
            .iter()
            .filter(|u| !u.being_built)
            .map(|u| match u.role {
                Role::Commander | Role::Lab | Role::Nano => self.catalog.get(u.role).worker_time,
                _ => 0.0,
            })
            .sum();
        self.samples.push(Sample {
            frame: self.frame,
            metal: self.team.metal,
            energy: self.team.energy,
            metal_storage: self.team.metal_storage,
            energy_storage: self.team.energy_storage,
            metal_income: a.mi,
            energy_income: a.ei,
            metal_usage: a.mu,
            energy_usage: a.eu,
            metal_wasted: a.mw,
            energy_wasted: a.ew,
            build_power: bp,
            build_power_used: a.bp_used / GAME_SPEED as f32,
        });
    }

    fn log(&mut self, e: Event) {
        if self.record {
            self.events.push(e);
        }
    }

    fn add_resources(&mut self, m: f32, e: f32) {
        self.team.metal += m;
        self.team.energy += e;
        self.acc.mi += m;
        self.acc.ei += e;
        if self.team.metal > self.team.metal_storage {
            self.acc.mw += self.team.metal - self.team.metal_storage;
            self.team.metal = self.team.metal_storage;
        }
        if self.team.energy > self.team.energy_storage {
            self.acc.ew += self.team.energy - self.team.energy_storage;
            self.team.energy = self.team.energy_storage;
        }
    }

    fn use_resources(&mut self, m: f32, e: f32) -> bool {
        if self.team.metal >= m && self.team.energy >= e {
            self.team.metal -= m;
            self.team.energy -= e;
            self.acc.mu += m;
            self.acc.eu += e;
            true
        } else {
            false
        }
    }

    fn create(
        &mut self,
        role: Role,
        pos: Pos,
        builder: Option<UnitId>,
        metal_extract: f32,
        lab_index: Option<usize>,
    ) -> UnitId {
        let id = self.units.len();
        self.units.push(Unit {
            role,
            pos,
            being_built: true,
            progress: 0.0,
            created_frame: self.frame,
            finished_frame: None,
            builder,
            metal_extract,
            lab_index,
            brain: Brain::Inert,
        });
        if role == Role::Lab {
            self.labs.push(id);
        }
        if let Some(b) = builder {
            self.log(Event::Started {
                frame: self.frame,
                builder: b,
                unit: id,
            });
        }
        id
    }

    fn finish(&mut self, id: UnitId) {
        let role = self.units[id].role;
        let st = self.catalog.get(role);
        let (ms, es) = (st.metal_storage, st.energy_storage);
        let u = &mut self.units[id];
        u.being_built = false;
        u.finished_frame = Some(self.frame);
        u.brain = match role {
            Role::Lab => Brain::Lab(LabState::default()),
            Role::Nano => Brain::Assist(AssistState {
                lab_index: u.lab_index.unwrap_or(0),
                mode: self.rules.assist_mode,
                cmd_repair: None,
                cur_build: None,
            }),
            _ => Brain::Inert,
        };
        self.team.metal_storage += ms;
        self.team.energy_storage += es;
        if role == Role::Tick {
            self.ticks_done += 1;
        }
        self.log(Event::Finished {
            frame: self.frame,
            unit: id,
        });
    }

    /// `CUnit::AddBuildPower` for construction. Returns true if it built.
    fn add_build_power(&mut self, target: UnitId, power_per_frame: f32) -> bool {
        let u = &self.units[target];
        if !u.being_built {
            return false;
        }
        let st = self.catalog.get(u.role);
        let step = (power_per_frame / st.build_time).min(1.0 - u.progress);
        let (m, e) = (st.metal_cost * step, st.energy_cost * step);
        if !self.use_resources(m, e) {
            return false;
        }
        self.acc.bp_used += power_per_frame;
        let u = &mut self.units[target];
        u.progress += step;
        if u.progress >= 1.0 {
            self.finish(target);
        }
        true
    }

    fn power_per_frame(&self, role: Role) -> f32 {
        self.catalog.get(role).worker_time / GAME_SPEED as f32
    }

    fn slow_update(&mut self, i: UnitId) {
        if self.units[i].being_built {
            return;
        }
        // command AI first, then income (CUnit::SlowUpdate)
        let mut brain = std::mem::replace(&mut self.units[i].brain, Brain::Inert);
        match &mut brain {
            Brain::Inert => {}
            Brain::Commander(c) => self.commander_su(i, c),
            Brain::Lab(l) => {
                if l.cur_build.is_none() && !l.queued {
                    l.queued = true;
                }
            }
            Brain::Assist(a) => self.assist_su(i, a),
        }
        self.units[i].brain = brain;
        self.produce(i);
    }

    fn produce(&mut self, i: UnitId) {
        let role = self.units[i].role;
        let st = self.catalog.get(role);
        let (mm, em, up, wg) = (
            st.metal_make,
            st.energy_make,
            st.energy_upkeep,
            st.wind_generator,
        );
        let extract = self.units[i].metal_extract;
        if mm > 0.0 || em > 0.0 {
            self.add_resources(mm * 0.5, em * 0.5);
        }
        if up < 0.0 {
            self.add_resources(0.0, -up * 0.5);
        }
        if up > 0.0 {
            if self.use_resources(0.0, up * 0.5) && extract > 0.0 {
                self.add_resources(extract * 0.5, 0.0);
            }
        } else if extract > 0.0 {
            self.add_resources(extract * 0.5, 0.0);
        }
        if wg > 0.0 {
            self.add_resources(0.0, self.rules.wind.min(wg) * 0.5);
        }
    }

    fn update(&mut self, i: UnitId) {
        if self.units[i].being_built {
            return;
        }
        let mut brain = std::mem::replace(&mut self.units[i].brain, Brain::Inert);
        match &mut brain {
            Brain::Inert => {}
            Brain::Commander(c) => match &mut c.state {
                CmdState::Building { target } => {
                    let p = self.power_per_frame(Role::Commander);
                    self.add_build_power(*target, p);
                }
                CmdState::Assist(a) => self.assist_update(i, Role::Commander, a),
                _ => {}
            },
            Brain::Lab(l) => self.lab_update(i, l),
            Brain::Assist(a) => self.assist_update(i, Role::Nano, a),
        }
        self.units[i].brain = brain;
    }

    fn lab_update(&mut self, i: UnitId, l: &mut LabState) {
        let f = self.frame;
        if l.queued {
            let ready = *l.yard_ready.get_or_insert(f + self.rules.yard_open_frames);
            if f >= ready && f >= l.pad_free {
                let pos = self.units[i].pos;
                let t = self.create(Role::Tick, pos, Some(i), 0.0, None);
                l.cur_build = Some(t);
                l.queued = false;
            }
        }
        if let Some(t) = l.cur_build {
            let p = self.power_per_frame(Role::Lab);
            self.add_build_power(t, p);
            if !self.units[t].being_built {
                l.cur_build = None;
                l.pad_free = f + self.rules.pad_clear_frames;
            }
        }
    }

    /// The unit that builder `i` wants to assist now.
    fn assist_target(&self, i: UnitId, a: &AssistState) -> Option<UnitId> {
        match a.mode {
            AssistMode::Guard => self.guard_target(a.lab_index),
            AssistMode::Fight => self.nearest_buildee(i),
        }
    }

    /// `CBuilderCAI::FindRepairTargetAndRepair`: the nearest unit under
    /// construction in build range.
    fn nearest_buildee(&self, i: UnitId) -> Option<UnitId> {
        let me = &self.units[i];
        let range = self.catalog.get(me.role).build_distance;
        self.units
            .iter()
            .enumerate()
            .filter(|(_, u)| u.being_built)
            .map(|(id, u)| {
                (
                    id,
                    u.pos.distance(me.pos),
                    self.catalog.get(u.role).radius(),
                )
            })
            .filter(|&(_, d, r)| d <= range + r)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _, _)| id)
    }

    /// `CBuilderCAI::ExecuteGuard` on a factory: its current buildee, or the
    /// factory itself while it is under construction.
    fn guard_target(&self, lab_index: usize) -> Option<UnitId> {
        let lab = *self.labs.get(lab_index)?;
        let u = &self.units[lab];
        if u.being_built {
            return Some(lab);
        }
        match &u.brain {
            Brain::Lab(l) => l.cur_build.filter(|&t| self.units[t].being_built),
            _ => None,
        }
    }

    fn assist_su(&mut self, i: UnitId, a: &mut AssistState) {
        if let Some(t) = a.cmd_repair {
            if self.units[t].being_built {
                if a.cur_build != Some(t) {
                    a.cur_build = Some(t);
                    self.log(Event::AssistStart {
                        frame: self.frame,
                        builder: i,
                        unit: t,
                    });
                }
                return;
            }
            // FinishCommand, then the guard or fight order runs in the same slow update
            a.cmd_repair = None;
        }
        if let Some(t) = self.assist_target(i, a) {
            a.cmd_repair = Some(t);
            if a.mode == AssistMode::Fight {
                a.cur_build = Some(t);
                self.log(Event::AssistStart {
                    frame: self.frame,
                    builder: i,
                    unit: t,
                });
            }
        }
    }

    fn assist_update(&mut self, i: UnitId, role: Role, a: &mut AssistState) {
        if let Some(t) = a.cur_build {
            let p = self.power_per_frame(role);
            if !self.add_build_power(t, p) && !self.units[t].being_built {
                a.cur_build = None;
                self.log(Event::AssistStop {
                    frame: self.frame,
                    builder: i,
                    unit: t,
                });
            }
        }
    }

    fn commander_su(&mut self, i: UnitId, c: &mut Commander) {
        let f = self.frame;
        if let CmdState::Building { target } = c.state {
            if self.units[target].being_built {
                return;
            }
            c.state = CmdState::Idle;
        }
        if let CmdState::Assist(a) = &mut c.state {
            let mut a = a.clone();
            self.assist_su(i, &mut a);
            c.state = CmdState::Assist(a);
            return;
        }
        if matches!(c.state, CmdState::Idle) && c.next < c.orders.len() {
            let order = c.orders[c.next];
            let (dest, radius) = match order {
                Order::Build { role, pos, .. } => (pos, self.catalog.get(role).radius()),
                Order::Assist { lab } => match self.labs.get(lab) {
                    Some(&l) => (self.units[l].pos, self.catalog.lab.radius()),
                    // the lab does not exist yet: wait
                    None => return,
                },
            };
            self.log(Event::Ordered {
                frame: f,
                builder: i,
                order: c.next,
            });
            c.next += 1;
            let range = self.catalog.commander.build_distance + radius;
            let here = self.units[i].pos;
            let dist = here.distance(dest);
            let mut arrive = f;
            if dist > range {
                let travel = dist - range;
                let frames =
                    (travel / (self.catalog.commander.speed / GAME_SPEED as f32)).ceil() as u32;
                arrive = f + frames;
                let k = travel / dist;
                let to = Pos::new(
                    here.x + (dest.x - here.x) * k,
                    here.z + (dest.z - here.z) * k,
                );
                self.units[i].pos = to;
                self.log(Event::Move {
                    frame: f,
                    builder: i,
                    from: here,
                    to,
                    arrive,
                });
            }
            c.state = CmdState::Approach {
                arrive,
                stance_left: self.rules.stance_slow_updates,
                order,
            };
        }
        if let CmdState::Approach {
            arrive,
            stance_left,
            order,
        } = &mut c.state
        {
            if f < *arrive {
                return;
            }
            match *order {
                Order::Assist { lab } => {
                    let mut a = AssistState {
                        lab_index: lab,
                        mode: self.rules.commander_assist_mode,
                        cmd_repair: None,
                        cur_build: None,
                    };
                    self.assist_su(i, &mut a);
                    c.state = CmdState::Assist(a);
                }
                Order::Build {
                    role,
                    pos,
                    metal,
                    lab,
                } => {
                    if *stance_left > 0 {
                        *stance_left -= 1;
                        return;
                    }
                    let t = self.create(role, pos, Some(i), metal, lab);
                    c.state = CmdState::Building { target: t };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lab_setup(labs: usize, nanos_per_lab: usize, mode: AssistMode) -> Sim {
        let rules = Rules {
            assist_mode: mode,
            ..Rules::default()
        };
        let mut sim = Sim::new(Catalog::reference(), rules, Pos::new(0.0, 0.0), vec![]);
        sim.set_unlimited_resources();
        for k in 0..labs {
            sim.spawn(Role::Lab, Pos::new(200.0 * k as f32, 0.0), None);
            for _ in 0..nanos_per_lab {
                sim.spawn(Role::Nano, Pos::new(200.0 * k as f32, 100.0), Some(k));
            }
        }
        sim
    }

    #[test]
    fn lab_alone_builds_tick_in_buildtime_plus_overhead() {
        let mut sim = lab_setup(1, 0, AssistMode::Guard);
        sim.run_until(60 * GAME_SPEED);
        // 800 / 150 = 5.33 s per tick, plus queue and pad overhead
        let ticks = sim.ticks();
        assert!((10..=11).contains(&ticks), "ticks = {ticks}");
    }

    #[test]
    fn tick_costs_its_resources_and_stalls_on_energy() {
        let mut sim = lab_setup(1, 0, AssistMode::Guard);
        sim.team.metal = 1000.0;
        sim.team.energy = 0.0;
        sim.team.metal_storage = 1000.0;
        sim.team.energy_storage = 1000.0;
        sim.run_until(60 * GAME_SPEED);
        let used: f32 = sim.samples.iter().map(|s| s.energy_usage).sum();
        let built: f32 = sim
            .units
            .iter()
            .filter(|u| u.role == Role::Tick)
            .map(|u| u.progress * 300.0)
            .sum();
        assert!((used - built).abs() < 1.0, "used {used} built {built}");
        // only the commander's 30 E/s: 1800 E in 60 s, 6 ticks at most
        assert!(
            sim.ticks() <= 6 && sim.ticks() >= 5,
            "ticks {}",
            sim.ticks()
        );
    }

    #[test]
    fn fight_nanos_join_faster_than_guard_nanos() {
        let mut g = lab_setup(1, 4, AssistMode::Guard);
        let mut f = lab_setup(1, 4, AssistMode::Fight);
        g.run_until(120 * GAME_SPEED);
        f.run_until(120 * GAME_SPEED);
        assert!(
            f.ticks() > g.ticks(),
            "fight {} guard {}",
            f.ticks(),
            g.ticks()
        );
    }

    #[test]
    fn nanos_have_diminishing_returns() {
        let rate = |n| {
            let mut s = lab_setup(1, n, AssistMode::Guard);
            s.run_until(120 * GAME_SPEED);
            s.ticks() as f32
        };
        let (r0, r1, r8) = (rate(0), rate(1), rate(8));
        assert!(r1 > r0);
        // ideal scaling would give (150 + 8 * 200) / 150 = 11.7x
        assert!(r8 < r0 * 6.0, "r0 {r0} r8 {r8}");
    }

    #[test]
    fn commander_builds_orders_in_sequence() {
        let orders = vec![
            Order::Build {
                role: Role::Wind,
                pos: Pos::new(100.0, 0.0),
                metal: 0.0,
                lab: None,
            },
            Order::Build {
                role: Role::Wind,
                pos: Pos::new(500.0, 0.0),
                metal: 0.0,
                lab: None,
            },
        ];
        let mut sim = Sim::new(
            Catalog::reference(),
            Rules::default(),
            Pos::new(0.0, 0.0),
            orders,
        );
        sim.run_until(30 * GAME_SPEED);
        let winds: Vec<_> = sim.units.iter().filter(|u| u.role == Role::Wind).collect();
        assert_eq!(winds.len(), 2);
        assert!(winds.iter().all(|u| !u.being_built));
        // 1600 / 300 = 5.33 s each
        let f0 = winds[0].finished_frame.unwrap();
        assert!((160..=200).contains(&f0), "first wind at {f0}");
        assert!(winds[1].created_frame > f0);
    }

    #[test]
    fn mex_needs_energy_upkeep() {
        let mut sim = Sim::new(
            Catalog::reference(),
            Rules::default(),
            Pos::new(0.0, 0.0),
            vec![],
        );
        let m = sim.spawn(Role::Mex, Pos::new(0.0, 0.0), None);
        sim.units[m].metal_extract = 2.0;
        sim.team.energy = 0.0;
        sim.team.metal = 0.0;
        // commander makes 30 E/s, so the mex can pay its 3 E/s
        sim.run_until(10 * GAME_SPEED);
        assert!(
            (sim.team.metal - 40.0).abs() < 2.5,
            "metal {}",
            sim.team.metal
        );
    }
}
