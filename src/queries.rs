use std::marker::PhantomData;

use crate::*;

pub mod components;
pub mod groups;

pub use components::*;
pub use groups::*;

pub struct Query<QG: QueryGroup> {
    raw_cursors: Box<dyn Iterator<Item = Cursor>>,
    cursor: Option<Cursor>,
    _phantom: PhantomData<QG>
}

impl <QG: QueryGroup> Query<QG> {
    pub fn new<W: WorldImpl>(world: &W) -> Self {
        Self::from_iter(world.raw_query(QG::req_comps()))
    }

    pub fn from_iter(iter: Box<dyn Iterator<Item = Cursor>>) -> Self {
        Self {
            raw_cursors: iter,
            cursor: None,
            _phantom: PhantomData::default()
        }
    }

    pub fn next(&mut self) -> anyhow::Result<Option<(EntityID, QG::Output)>> {
        // if cursor is empty or has no more components, attempt to get another cursor
        if self.cursor.as_ref().map(|a| !a.has_next()).unwrap_or(true) {
            self.cursor = self.raw_cursors.next();
        }

        // if we still dont have a cursor, we are done, return none
        if self.cursor.as_ref().map(|a| !a.has_next()).unwrap_or(true) {
            return Ok(None);
        }

        // extract comps
        let (entity_id, comps) = self.cursor.as_ref().unwrap().next().unwrap();
        let extracted = QG::from_comps(comps)?;
        Ok(Some((entity_id, extracted)))
    }
}
