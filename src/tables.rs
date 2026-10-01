use derive_more::{Deref, DerefMut};

use crate::*;

pub mod doubly_linked_list;
pub mod single_linked_list;

pub use single_linked_list::*;

#[derive(Deref, DerefMut)]
pub struct Table(Box<dyn TableImpl>);

impl Table {
    pub fn new<T: TableImpl + 'static>(implementation: T) -> Self {
        Self(Box::new(implementation))
    }

    pub fn default(comp_ids: &[ComponentID]) -> Self {
        Self::new(SingleLinkedListTable::new(comp_ids))
    }
}

pub trait TableImpl: Send + Sync {
    fn group<'a>(&'a self) -> &'a ComponentIDGroup;
    fn cursor(&self) -> Cursor;

    fn insert(&self, id: EntityID, components: DynComponents) {
        self.cursor().insert(id, components);
    }

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

#[derive(Deref, DerefMut)]
pub struct Cursor(Box<dyn CursorImpl>);

impl Cursor {
    pub fn new<C: CursorImpl + 'static>(implementation: C) -> Self {
        Self(Box::new(implementation))
    }
}

pub trait CursorImpl {
    /// The sorted component ids of the table this cursor walks, every entity it yields has exactly these.
    fn group(&self) -> &ComponentIDGroup;
    fn has_next(&self) -> bool;
    fn next(&self) -> Option<(EntityID, DynComponents)>;
    fn pop(&self) -> Option<(EntityID, DynComponents)>;
    fn insert(&self, id: EntityID, components: DynComponents);
}