//! [`ListWorld`], entity storage that keeps every table in one list.

use std::sync::{Arc, Mutex};

use derive_more::{Deref, DerefMut};
use mutual::{DashMap, Ref, SharedData, SharedList};

use crate::*;

/// A world that keeps its tables in one list, and checks every table on each
/// query and insert.  Simpler than [`IndexedWorld`], but slows down as the
/// number of tables grows.
#[derive(Deref, DerefMut)]
pub struct ListWorld {
    #[deref] #[deref_mut]
    list: SharedList<Table>,
    /// Every entity by id, see [`WorldImpl::entity`].
    entities: DashMap<EntityID, EntityRef>,
    /// Held while creating a table so two threads can't both create the same group.
    create_lock: Mutex<()>
}

impl ListWorld {
    /// Create an empty world.
    pub fn new() -> Self {
        Self { list: SharedList::new(), entities: DashMap::default(), create_lock: Mutex::new(()) }
    }

    fn find_table(&self, group: &ComponentIDGroup) -> Option<Ref<Table>> {
        self.list.find(|table| table.group() == group)
    }
}

impl ListWorld {
    /// Inserts into the table for the sorted `reqs`, creating it if needed.
    fn insert_into_table(&self, entity_id: EntityID, reqs: &ComponentIDGroup, components: DynComponents) {
        // fast path: the table already exists, no locking
        if let Some(table) = self.find_table(reqs) {
            table.insert(entity_id, components);
            return;
        }

        // slow path: search again under the lock, since another thread may have created it
        let guard = self.create_lock.lock().unwrap();
        if let Some(table) = self.find_table(reqs) {
            drop(guard);
            table.insert(entity_id, components);
            return;
        }

        // fill the table before pushing so other threads never see it empty
        let table = Table::default(reqs);
        table.insert(entity_id, components);
        self.list.push(table);
    }
}

impl Default for ListWorld {
    fn default() -> Self { Self::new() }
}

impl WorldImpl for ListWorld {
    fn insert(&self, entity_id: EntityID, mut components: DynComponents) {
        components.sort_by_key(|c| c.lock_ref().get_id());
        
        let reqs = Arc::new(components.iter()
            .map(|a| a.lock_ref().get_id())
            .collect::<ComponentIDGroup>());
        let entity = EntityRef { group: reqs.clone(), components: components.clone() };

        self.insert_into_table(entity_id, &reqs, components);
        self.entities.insert(entity_id, entity);
    }

    fn raw_query<'a>(&'a self, mut req_components: ComponentIDGroup) -> Box<dyn Iterator<Item = Cursor>> {
        req_components.sort();
        Box::new(
            self.list.iter()
                .filter(move |table| group_matches(table.group(), &req_components))
                .map(|table| table.cursor())
            )
    }

    fn entity(&self, id: EntityID) -> Option<EntityRef> {
        self.entities.get(&id).map(|entity| entity.clone())
    }

    fn remove(&self, id: EntityID) -> Option<DynComponents> {
        let (_, entity) = self.entities.remove(&id)?;
        self.find_table(&entity.group)?.remove(id).map(|(_, components)| components)
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::worlds::test_support::TestWorld;

    impl TestWorld for ListWorld {
        fn new_world() -> Self { ListWorld::new() }
        fn table_count(&self) -> usize { self.list.len() }
        fn groups(&self) -> Vec<Vec<ComponentID>> {
            self.list.iter().map(|table| table.group().to_vec()).collect()
        }
    }

    crate::worlds::world_tests!(ListWorld);
}
