//! Typed queries over the entities in a world.

use std::{marker::PhantomData, sync::Arc};

use crate::*;

pub mod components;
pub mod groups;
pub mod guards;

pub use components::*;
pub use groups::*;
pub use guards::*;

/// Walks every entity holding the components in `QG`, handing out guards over them.
///
/// `QG` is a [`QueryComponent`] (`&A`, `&mut A`, `Option<&A>` or `Option<&mut A>`)
/// or a tuple of up to 8 of them.  Each entity's components are borrowed as it is handed
/// out, and stay borrowed until their guards are dropped.  `Option` terms aren't required,
/// so they resolve to `None` on entities without the component.
///
/// While a query is part way through a table, that table can't compact.
pub struct Query<QG: QueryGroup> {
    /// The tables still to walk.
    tables: Box<dyn Iterator<Item = Arc<ChunkedTable>>>,
    /// The table being walked.
    current: Option<Walk>,
    _group: PhantomData<QG>
}

/// A table a [`Query`] is inside, and where it has got to.
struct Walk {
    table: Arc<ChunkedTable>,
    plan: Plan,
    next: usize
}

impl Drop for Walk {
    fn drop(&mut self) { self.table.exit() }
}

impl <QG: QueryGroup> Query<QG> {
    /// Query every table in `world` that holds all of `QG`'s required components.
    pub fn new<W: WorldImpl + ?Sized>(world: &W) -> Self {
        Self::from_iter(world.raw_query(QG::required()))
    }

    /// Query the tables in `iter`.  Tables missing a required component make
    /// [`next`](Self::next) return an error.
    pub fn from_iter(iter: Box<dyn Iterator<Item = Arc<ChunkedTable>>>) -> Self {
        Self { tables: iter, current: None, _group: PhantomData }
    }

    /// Returns the next entity and its components, or `None` once every table is done.
    /// Blocks while other guards hold the entity's components mutably.
    ///
    /// # Errors
    ///
    /// Errors if a table is missing a required component or holds a different type
    /// under its id, or `QG` borrows a component mutably more than once.
    pub fn next(&mut self) -> anyhow::Result<Option<(EntityID, QG::Output)>> {
        loop {
            if let Some(walk) = &mut self.current {
                if let Some((id, chunk, row)) = walk.table.next_locked(&walk.plan, &mut walk.next) {
                    return Ok(Some((id, walk.table.output::<QG>(&walk.plan, chunk, row))));
                }
                self.current = None;
            }

            // move on to the next table and resolve its layout, if there are no more we are done
            let Some(table) = self.tables.next() else { return Ok(None) };
            let plan = table.plan::<QG>()?;
            table.enter_raw();
            self.current = Some(Walk { table, plan, next: 0 });
        }
    }
}

impl <QG: QueryGroup> SystemParam for Query<QG> {
    fn extract(world: &World, _exec_state: &SharedExecutionState, _system: SystemKey) -> anyhow::Result<Self> {
        Ok(Self::new(world))
    }
}
