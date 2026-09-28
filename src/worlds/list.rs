use std::sync::Mutex;

use derive_more::{Deref, DerefMut};
use mutual::{Ref, SharedData, SharedList};

use crate::*;

#[derive(Deref, DerefMut)]
pub struct ListWorld {
    #[deref] #[deref_mut]
    list: SharedList<Table>,
    /// Held while creating a table so two threads can't both create the same group.
    create_lock: Mutex<()>
}

impl ListWorld {
    pub fn new() -> Self {
        Self { list: SharedList::new(), create_lock: Mutex::new(()) }
    }

    fn find_table(&self, group: &ComponentIDGroup) -> Option<Ref<Table>> {
        self.list.find(|table| table.group() == group)
    }
}

impl Default for ListWorld {
    fn default() -> Self { Self::new() }
}

impl WorldImpl for ListWorld {
    fn insert(&self, entity_id: EntityID, mut components: DynComponents) {
        components.sort_by_key(|c| c.lock_ref().get_id());
        
        let reqs = components.iter()
            .map(|a| a.lock_ref().get_id())
            .collect::<Box<_>>();
        
        // fast path: the table already exists, no locking
        if let Some(table) = self.find_table(&reqs) {
            table.insert(entity_id, components);
            return;
        }

        // slow path: search again under the lock, since another thread may have created it
        let guard = self.create_lock.lock().unwrap();
        if let Some(table) = self.find_table(&reqs) {
            drop(guard);
            table.insert(entity_id, components);
            return;
        }

        // fill the table before pushing so other threads never see it empty
        let table = Table::default(&reqs);
        table.insert(entity_id, components);
        self.list.push(table);
    }

    fn raw_query<'a>(&'a self, mut req_components: ComponentIDGroup) -> Box<dyn Iterator<Item = Cursor>> {
        req_components.sort();
        Box::new(
            self.list.iter()
                .filter(move |table| group_matches(table.group(), &req_components))
                .map(|table| table.cursor())
            )
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
