//! Tables, which store every entity with one exact set of components.

use derive_more::{Deref, DerefMut};

use crate::*;

pub mod doubly_linked_list;
pub mod single_linked_list;

pub use single_linked_list::*;

/// A type erased table, derefs to its [`TableImpl`].
#[derive(Deref, DerefMut)]
pub struct Table(Box<dyn TableImpl>);

impl Table {
    /// Wrap a table implementation.
    pub fn new<T: TableImpl + 'static>(implementation: T) -> Self {
        Self(Box::new(implementation))
    }

    /// Create an empty [`SingleLinkedListTable`] for the sorted `comp_ids`.
    pub fn default(comp_ids: &[ComponentID]) -> Self {
        Self::new(SingleLinkedListTable::new(comp_ids))
    }
}

/// Storage for every entity with one exact set of components.  Any number of
/// threads may use a table, and its cursors, at once.
pub trait TableImpl: Send + Sync {
    /// The sorted component ids every entity in this table has.
    fn group<'a>(&'a self) -> &'a ComponentIDGroup;
    /// A new cursor at the start of the table.
    fn cursor(&self) -> Cursor;

    /// Insert an entity at the front of the table.  `components` must be
    /// sorted by id, matching [`group`](Self::group).
    fn insert(&self, id: EntityID, components: DynComponents) {
        self.cursor().insert(id, components);
    }

    /// Remove the first entity with `id` and return it, or `None` if there is none.
    /// Walks the table to find it.
    fn remove(&self, id: EntityID) -> Option<(EntityID, DynComponents)> {
        let cursor = self.cursor();
        while let Some(value) = cursor.next() {
            if value.0 == id {
                return cursor.pop();
            }
        }

        None
    }
}

/// A type erased cursor, derefs to its [`CursorImpl`].
#[derive(Deref, DerefMut)]
pub struct Cursor(Box<dyn CursorImpl>);

impl Cursor {
    /// Wrap a cursor implementation.
    pub fn new<C: CursorImpl + 'static>(implementation: C) -> Self {
        Self(Box::new(implementation))
    }
}

/// A position in a table, between two entities.  A cursor stays valid while
/// other cursors insert into and pop from the same table.
pub trait CursorImpl {
    /// The sorted component ids of the table this cursor walks, every entity it yields has exactly these.
    fn group(&self) -> &ComponentIDGroup;
    /// True if there is an entity after the cursor.
    fn has_next(&self) -> bool;
    /// Returns the entity after the cursor and steps over it.  The components
    /// share storage with the table, so changes to them are seen by everyone.
    fn next(&self) -> Option<(EntityID, DynComponents)>;
    /// Unlinks the entity after the cursor (the one `next` would return) and returns it.
    fn pop(&self) -> Option<(EntityID, DynComponents)>;
    /// Inserts an entity before the cursor, so `next` does not return it.
    fn insert(&self, id: EntityID, components: DynComponents);
}