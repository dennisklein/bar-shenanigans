//! Build plans: what the commander builds, in which order, and where.
//!
//! A plan is a list of [`Action`]s. [`Base`] turns each action into a
//! commander [`Order`] with a position: mexes go to the nearest free metal
//! spot, the other structures to fixed slots of a compact base around the
//! start position. After the last action the commander assists the first lab.

use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, Role};
use crate::sim::{Order, Pos};

/// One commander build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Mex,
    Wind,
    Solar,
    Lab,
    Nano,
}

impl Action {
    pub const ALL: [Action; 5] = [Action::Mex, Action::Wind, Action::Solar, Action::Lab, Action::Nano];

    pub fn role(self) -> Role {
        match self {
            Action::Mex => Role::Mex,
            Action::Wind => Role::Wind,
            Action::Solar => Role::Solar,
            Action::Lab => Role::Lab,
            Action::Nano => Role::Nano,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Action::Mex => "mex",
            Action::Wind => "wind",
            Action::Solar => "solar",
            Action::Lab => "lab",
            Action::Nano => "nano",
        }
    }
}

/// A named build plan, as read from a Lua plan file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub name: String,
    #[serde(default)]
    pub source: Option<String>,
    pub actions: Vec<Action>,
}

impl Plan {
    /// Reads a plan file. The file is Lua that returns a table, for example
    /// `return { name = "2 mex lab", actions = { "mex", "mex", "lab" } }`.
    pub fn load(path: &Path) -> Result<Plan> {
        let src = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Plan::from_lua(&src, &path.display().to_string())
    }

    pub fn from_lua(src: &str, chunk_name: &str) -> Result<Plan> {
        use mlua::LuaSerdeExt;
        let lua = mlua::Lua::new();
        let value = lua
            .load(src)
            .set_name(format!("@{chunk_name}"))
            .eval::<mlua::Value>()
            .map_err(|e| anyhow::anyhow!("plan {chunk_name}: {e}"))?;
        let plan: Plan = lua.from_value(value).map_err(|e| anyhow::anyhow!("plan {chunk_name}: {e}"))?;
        if plan.actions.is_empty() {
            bail!("plan {chunk_name}: no actions");
        }
        Ok(plan)
    }

    pub fn summary(&self) -> String {
        summary(&self.actions)
    }
}

/// Short text of an action list, for example `mex x2, lab, wind x3`.
pub fn summary(actions: &[Action]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < actions.len() {
        let mut j = i;
        while j < actions.len() && actions[j] == actions[i] {
            j += 1;
        }
        let n = j - i;
        parts.push(if n > 1 { format!("{} x{n}", actions[i].name()) } else { actions[i].name().into() });
        i = j;
    }
    parts.join(", ")
}

/// A metal spot.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub pos: Pos,
    pub metal: f32,
}

/// Where the commander builds: metal spots and base slots.
#[derive(Debug, Clone)]
pub struct Base {
    pub start: Pos,
    /// Unit vector from the start position toward the map center.
    fwd: (f32, f32),
    /// Metal spots, nearest to the start first.
    pub spots: Vec<Spot>,
}

/// Position assignment state while a plan turns into orders.
#[derive(Debug, Clone, Default)]
pub struct Placer {
    mexes: usize,
    labs: usize,
    energy: usize,
    /// Nanos per lab.
    nanos: Vec<usize>,
    /// Nanos that wait for a lab that is not planned yet.
    pending_nanos: usize,
}

impl Placer {
    pub fn labs(&self) -> usize {
        self.labs
    }

    pub fn mexes(&self) -> usize {
        self.mexes
    }

    pub fn nanos(&self) -> usize {
        self.nanos.iter().sum::<usize>() + self.pending_nanos
    }
}

impl Base {
    pub fn new(start: Pos, map_size: Pos, spots: &[Spot]) -> Base {
        let (dx, dz) = (map_size.x / 2.0 - start.x, map_size.z / 2.0 - start.z);
        let len = dx.hypot(dz).max(1.0);
        let mut spots = spots.to_vec();
        spots.sort_by(|a, b| a.pos.distance(start).total_cmp(&b.pos.distance(start)));
        Base { start, fwd: (dx / len, dz / len), spots }
    }

    fn at(&self, f: f32, s: f32) -> Pos {
        let (fx, fz) = self.fwd;
        // side vector: fwd rotated by 90 degrees
        Pos::new(self.start.x + fx * f - fz * s, self.start.z + fz * f + fx * s)
    }

    /// Alternating offsets 0, +1, -1, +2, -2, ...
    fn alt(k: usize) -> f32 {
        let h = k.div_ceil(2) as f32;
        if k % 2 == 1 {
            h
        } else {
            -h
        }
    }

    fn lab_pos(&self, k: usize) -> Pos {
        self.at(200.0, 130.0 * Base::alt(k))
    }

    fn nano_pos(&self, lab: usize, j: usize) -> Pos {
        // a row behind the lab, inside the 400 elmo nano range
        let lab_side = 130.0 * Base::alt(lab);
        self.at(110.0 - 32.0 * (j / 4) as f32, lab_side + 30.0 * Base::alt(j % 4))
    }

    fn energy_pos(&self, k: usize) -> Pos {
        // a grid behind the start position, 6 slots per row
        let row = k / 6;
        let col = k % 6;
        self.at(-90.0 - 56.0 * row as f32, 56.0 * Base::alt(col))
    }

    /// The order for `action`, or `None` if no metal spot is left.
    pub fn order(&self, placer: &mut Placer, action: Action) -> Option<Order> {
        let role = action.role();
        let order = match action {
            Action::Mex => {
                let spot = self.spots.get(placer.mexes)?;
                placer.mexes += 1;
                Order::Build { role, pos: spot.pos, metal: spot.metal, lab: None }
            }
            Action::Wind | Action::Solar => {
                let pos = self.energy_pos(placer.energy);
                placer.energy += 1;
                Order::Build { role, pos, metal: 0.0, lab: None }
            }
            Action::Lab => {
                let pos = self.lab_pos(placer.labs);
                placer.labs += 1;
                placer.nanos.push(0);
                Order::Build { role, pos, metal: 0.0, lab: None }
            }
            Action::Nano => {
                // the lab with the fewest nanos; lab 0 if none is planned yet
                let lab = (0..placer.labs).min_by_key(|&k| placer.nanos[k]).unwrap_or(0);
                let j = if placer.labs == 0 {
                    placer.pending_nanos += 1;
                    placer.pending_nanos - 1
                } else {
                    placer.nanos[lab] += 1;
                    placer.nanos[lab] - 1 + if lab == 0 { placer.pending_nanos } else { 0 }
                };
                Order::Build { role, pos: self.nano_pos(lab, j), metal: 0.0, lab: Some(lab) }
            }
        };
        Some(order)
    }

    /// All orders of `actions`, then assist lab 0. Actions without a free
    /// metal spot are skipped.
    pub fn orders(&self, actions: &[Action]) -> Vec<Order> {
        let mut placer = Placer::default();
        let mut out: Vec<Order> = actions.iter().filter_map(|&a| self.order(&mut placer, a)).collect();
        out.push(Order::Assist { lab: 0 });
        out
    }
}

/// Checks that a catalog can build every action (for example a lab can
/// build Ticks).
pub fn check_catalog(c: &Catalog) -> Result<()> {
    if c.tick.build_time <= 0.0 || c.lab.worker_time <= 0.0 {
        bail!("catalog has no usable lab or tick");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_lua_plan() {
        let plan =
            Plan::from_lua(r#"return { name = "test", actions = { "mex", "mex", "lab", "wind" } }"#, "test").unwrap();
        assert_eq!(plan.actions, vec![Action::Mex, Action::Mex, Action::Lab, Action::Wind]);
        assert_eq!(plan.summary(), "mex x2, lab, wind");
    }

    #[test]
    fn rejects_unknown_action() {
        assert!(Plan::from_lua(r#"return { name = "x", actions = { "fusion" } }"#, "x").is_err());
    }

    #[test]
    fn places_mexes_nearest_first_and_nanos_near_labs() {
        let spots =
            [Spot { pos: Pos::new(900.0, 900.0), metal: 2.0 }, Spot { pos: Pos::new(600.0, 520.0), metal: 1.8 }];
        let base = Base::new(Pos::new(500.0, 500.0), Pos::new(4000.0, 4000.0), &spots);
        let orders = base.orders(&[Action::Mex, Action::Lab, Action::Nano, Action::Nano, Action::Mex]);
        let Order::Build { pos, metal, .. } = orders[0] else { panic!() };
        assert_eq!((pos, metal), (Pos::new(600.0, 520.0), 1.8));
        let Order::Build { pos: lab, .. } = orders[1] else { panic!() };
        for o in &orders[2..4] {
            let Order::Build { pos, lab: Some(0), .. } = *o else { panic!() };
            assert!(pos.distance(lab) < 400.0);
        }
        assert_eq!(orders.last(), Some(&Order::Assist { lab: 0 }));
    }
}
