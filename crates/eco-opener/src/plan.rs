//! A build order as one order queue per builder, readable and writable as Lua.
//!
//! ```lua
//! return {
//!   -- orders of the commander
//!   commander = { "armmex", "armmex", "armwin", "armlab" },
//!   -- one queue per factory, in the order the factories are built
//!   factories = { { "armck", "armck" } },
//!   -- one queue per mobile constructor, in the order they are built
//!   constructors = { { "armmex", "armmex" }, { "armwin", "armwin" } },
//! }
//! ```
//!
//! A builder with an empty queue assists: first the nearest factory that
//! is producing, else the nearest construction in progress.

use std::fmt::Write as _;

use anyhow::{anyhow, Result};
use mlua::{Lua, Table};

use crate::catalog::{Catalog, UnitId};

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Plan {
    pub commander: Vec<UnitId>,
    pub factories: Vec<Vec<UnitId>>,
    pub constructors: Vec<Vec<UnitId>>,
}

/// Identifies one queue of a [`Plan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueRef {
    Commander,
    Factory(usize),
    Constructor(usize),
}

impl Plan {
    pub fn queue(&self, q: QueueRef) -> &[UnitId] {
        match q {
            QueueRef::Commander => &self.commander,
            QueueRef::Factory(i) => self.factories.get(i).map_or(&[], |v| v),
            QueueRef::Constructor(i) => self.constructors.get(i).map_or(&[], |v| v),
        }
    }

    pub fn queue_mut(&mut self, q: QueueRef) -> &mut Vec<UnitId> {
        let (list, i) = match q {
            QueueRef::Commander => return &mut self.commander,
            QueueRef::Factory(i) => (&mut self.factories, i),
            QueueRef::Constructor(i) => (&mut self.constructors, i),
        };
        if list.len() <= i {
            list.resize(i + 1, Vec::new());
        }
        &mut list[i]
    }

    /// All queues, including empty ones.
    pub fn queues(&self) -> Vec<QueueRef> {
        let mut q = vec![QueueRef::Commander];
        q.extend((0..self.factories.len()).map(QueueRef::Factory));
        q.extend((0..self.constructors.len()).map(QueueRef::Constructor));
        q
    }

    /// Removes empty queues at the end of the factory and constructor lists.
    pub fn trim(&mut self) {
        while self.factories.last().is_some_and(Vec::is_empty) {
            self.factories.pop();
        }
        while self.constructors.last().is_some_and(Vec::is_empty) {
            self.constructors.pop();
        }
    }

    pub fn order_count(&self) -> usize {
        self.commander.len()
            + self.factories.iter().map(Vec::len).sum::<usize>()
            + self.constructors.iter().map(Vec::len).sum::<usize>()
    }

    pub fn from_lua(source: &str, chunk_name: &str, catalog: &Catalog) -> Result<Self> {
        let lua = Lua::new();
        let t: Table = lua
            .load(source)
            .set_name(format!("@{chunk_name}"))
            .eval()
            .map_err(|e| anyhow!("plan {chunk_name}: {e}"))?;
        let names = |v: Vec<String>| -> Result<Vec<UnitId>> {
            v.iter().map(|n| catalog.id(n).ok_or_else(|| anyhow!("plan {chunk_name}: unknown unit {n}"))).collect()
        };
        let lists = |key: &str| -> Result<Vec<Vec<UnitId>>> {
            let v: Option<Vec<Vec<String>>> = t.get(key).map_err(|e| anyhow!("plan {chunk_name}: {key}: {e}"))?;
            v.unwrap_or_default().into_iter().map(names).collect()
        };
        let commander: Option<Vec<String>> =
            t.get("commander").map_err(|e| anyhow!("plan {chunk_name}: commander: {e}"))?;
        Ok(Plan {
            commander: names(commander.unwrap_or_default())?,
            factories: lists("factories")?,
            constructors: lists("constructors")?,
        })
    }

    pub fn to_lua(&self, catalog: &Catalog, header: &str) -> String {
        let list = |v: &[UnitId]| {
            let items: Vec<String> = v.iter().map(|&u| format!("\"{}\"", catalog.get(u).name)).collect();
            format!("{{ {} }}", items.join(", "))
        };
        let mut s = String::new();
        for line in header.lines() {
            let _ = writeln!(s, "-- {line}");
        }
        let _ = writeln!(s, "return {{");
        let _ = writeln!(s, "\tcommander = {},", list(&self.commander));
        for (key, lists) in [("factories", &self.factories), ("constructors", &self.constructors)] {
            let _ = writeln!(s, "\t{key} = {{");
            for l in lists {
                let _ = writeln!(s, "\t\t{},", list(l));
            }
            let _ = writeln!(s, "\t}},");
        }
        let _ = writeln!(s, "}}");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::testing::catalog;

    #[test]
    fn lua_round_trip() {
        let c = catalog();
        let src = r#"return {
            commander = { "mex", "mex", "lab" },
            factories = { { "ck" } },
            constructors = { { "wind", "mex" } },
        }"#;
        let plan = Plan::from_lua(src, "test", &c).unwrap();
        assert_eq!(plan.order_count(), 6);
        let again = Plan::from_lua(&plan.to_lua(&c, "generated\nby test"), "again", &c).unwrap();
        assert_eq!(plan, again);
    }

    #[test]
    fn unknown_unit_is_an_error() {
        let c = catalog();
        assert!(Plan::from_lua(r#"return { commander = { "nope" } }"#, "t", &c).is_err());
    }

    #[test]
    fn queue_mut_grows() {
        let mut p = Plan::default();
        p.queue_mut(QueueRef::Constructor(2)).push(1);
        assert_eq!(p.constructors.len(), 3);
        p.constructors[2].clear();
        p.trim();
        assert!(p.constructors.is_empty());
    }
}
