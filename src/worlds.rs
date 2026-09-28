use derive_more::{Deref, DerefMut};

use crate::{ComponentIDGroup, Cursor, DynComponents, EntityID};

pub mod list;

pub use list::*;

#[derive(Deref, DerefMut)]
pub struct World(Box<dyn WorldImpl>);

pub trait WorldImpl {
    /// Insert a new entity into the world with a given set of components.
    /// The entity must have at least one component.
    fn insert(&self, entity_id: EntityID, components: DynComponents);

    /// Create a raw query that produces an iterator of cursors across
    /// the tables that meet the required components given.
    /// Required components may not be empty.
    fn raw_query<'a>(&'a self, req_components: ComponentIDGroup) -> Box<dyn Iterator<Item = Cursor>>;
}