//! Typed queries over the entities in a world.

use crate::*;

pub mod components;
pub mod groups;

pub use components::*;
pub use groups::*;

/// Walks every entity holding the components in `QG`, handing out locked
/// access to them.
///
/// `QG` is a [`QueryComponent`] (`&A`, `&mut A`, `Option<&A>` or `Option<&mut A>`)
/// or a tuple of up to 8 of them.  Each component is locked as it is handed out
/// and stays locked until its guard is dropped.
///
/// [`Query::new`] asks the world for every listed component, `Option` ones
/// included, so an optional component only resolves to `None` when the query is
/// built with [`Query::from_iter`] over tables that lack it.
pub struct Query<QG: QueryGroup> {
    /// Iterator of cursors over tables
    raw_cursors: Box<dyn Iterator<Item = Cursor>>,
    /// The current table's cursor, with where the queried components sit in that table.
    cursor: Option<(Cursor, QG::Indices)>
}

impl <QG: QueryGroup> Query<QG> {
    /// Query every table in `world` that holds all of `QG`'s components.
    pub fn new<W: WorldImpl>(world: &W) -> Self {
        Self::from_iter(world.raw_query(QG::req_comps()))
    }

    /// Query the tables walked by each cursor in `iter`.  Tables missing a
    /// required component make [`next`](Self::next) return an error.
    pub fn from_iter(iter: Box<dyn Iterator<Item = Cursor>>) -> Self {
        Self { raw_cursors: iter, cursor: None }
    }

    /// Returns the next entity and its components, or `None` once every table is done.
    ///
    /// # Errors
    ///
    /// Errors if the entity is missing a required component.
    pub fn next(&mut self) -> anyhow::Result<Option<(EntityID, QG::Output)>> {
        loop {
            if let Some((cursor, indices)) = &self.cursor
                && let Some((entity_id, comps)) = cursor.next() {
                return Ok(Some((entity_id, QG::from_comps(&comps, indices)?)));
            }

            // current table is done (or empty), move on to the next one and resolve its layout,
            // if there are no more tables we are done
            let Some(cursor) = self.raw_cursors.next() else { return Ok(None) };
            let indices = QG::resolve(cursor.group());
            self.cursor = Some((cursor, indices));
        }
    }
}

impl <QG: QueryGroup> SystemParam for Query<QG> {
    fn extract(world: &World, _exec_state: &SharedExecutionState, _system: SystemKey) -> anyhow::Result<Self> {
        Ok(Self::new(world))
    }
}
