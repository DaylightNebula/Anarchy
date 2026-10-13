//! [`ListWorld`], entity storage that keeps every table in one list.

use std::sync::{Arc, Mutex};

use mutual::SharedList;

use crate::*;

/// A world that keeps its tables in one list, and checks every table on each
/// query and insert.  Simpler than [`IndexedWorld`], but slows down as the
/// number of tables grows.
pub struct ListWorld {
    list: SharedList<Arc<ChunkedTable>>,
    /// Every entity by id, see [`WorldImpl::entity`].
    entities: IdMap<Arc<ChunkedTable>>,
    /// Held while creating a table so two threads can't both create the same group.
    create_lock: Mutex<()>
}

impl ListWorld {
    /// Create an empty world.
    pub fn new() -> Self {
        Self { list: SharedList::new(), entities: IdMap::default(), create_lock: Mutex::new(()) }
    }

    /// The table for exactly the sorted `group`, if there is one.
    fn find_table(&self, group: &ComponentIDGroup) -> Option<Arc<ChunkedTable>> {
        self.list.find(|table| table.group() == group).map(|table| (*table).clone())
    }
}

impl Default for ListWorld {
    fn default() -> Self { Self::new() }
}

impl WorldImpl for ListWorld {
    fn table_for(&self, infos: Vec<&'static ComponentInfo>) -> anyhow::Result<Arc<ChunkedTable>> {
        let group = group_of(&infos);
        // fast path: the table already exists, no locking
        if let Some(table) = self.find_table(&group) { return Ok(table) }

        // slow path: search again under the lock, since another thread may have created it
        let _guard = self.create_lock.lock().unwrap();
        if let Some(table) = self.find_table(&group) { return Ok(table) }
        let table = Arc::new(ChunkedTable::from_infos(infos)?);
        self.list.push(table.clone());
        Ok(table)
    }

    fn index(&self, id: EntityID, table: Arc<ChunkedTable>) {
        self.entities.insert(id, table);
    }

    fn tables(&self) -> Vec<Arc<ChunkedTable>> {
        self.list.iter().map(|table| (*table).clone()).collect()
    }

    fn raw_query<'a>(&'a self, mut req_components: ComponentIDGroup) -> Box<dyn Iterator<Item = Arc<ChunkedTable>>> {
        req_components.sort();
        Box::new(
            self.list.iter()
                .filter(move |table| group_matches(table.group(), &req_components))
                .map(|table| (*table).clone())
            )
    }

    fn table_of(&self, id: EntityID) -> Option<Arc<ChunkedTable>> {
        self.entities.get(&id).map(|table| table.clone())
    }

    fn entity_ids(&self) -> Box<dyn Iterator<Item = EntityID> + '_> {
        Box::new(self.entities.iter().map(|entry| *entry.key()))
    }

    fn remove(&self, id: EntityID) -> Option<DynComponents> {
        remove_indexed(&self.entities, id)
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
