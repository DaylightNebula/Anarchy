use crate::*;

pub mod components;
pub mod groups;

pub use components::*;
pub use groups::*;

pub struct Query<QG: QueryGroup> {
    /// Iterator of cursors over tables
    raw_cursors: Box<dyn Iterator<Item = Cursor>>,
    /// The current table's cursor, with where the queried components sit in that table.
    cursor: Option<(Cursor, QG::Indices)>
}

impl <QG: QueryGroup> Query<QG> {
    pub fn new<W: WorldImpl>(world: &W) -> Self {
        Self::from_iter(world.raw_query(QG::req_comps()))
    }

    pub fn from_iter(iter: Box<dyn Iterator<Item = Cursor>>) -> Self {
        Self { raw_cursors: iter, cursor: None }
    }

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
    fn extract(world: &World, _exec_state: &SharedExecutionState) -> Self {
        Self::new(world)
    }
}
