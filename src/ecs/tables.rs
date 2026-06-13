use std::{cell::UnsafeCell, collections::VecDeque, fmt::Debug, ops::Deref, sync::{atomic::{AtomicBool, Ordering}, Mutex}, thread::ThreadId};

use ahash::AHashSet;
use mutual::{SharedData, RelaxedMutex};

use crate::ecs::{components::bit_masks_match, entities::{Entity, EntityID}};

/// Contains an unsorted list of each entry or entity in
/// a table.  These entities MUST have a the same components.
pub struct Table { 
    pub(crate) bit_mask: Box<[u8]>,
    pub(crate) rows: UnsafeCell<VecDeque<RelaxedMutex<Entity>>>,
    pub(crate) to_add: Mutex<Vec<Entity>>,
    pub(crate) to_remove: Mutex<Option<AHashSet<EntityID>>>,
    pub(crate) ref_access: Mutex<AHashSet<ThreadId>>,
    pub(crate) locked: AtomicBool
}

impl Debug for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Table")
            .field("bit_mask", &self.bit_mask)
            .field("rows", unsafe { &*self.rows.get() })
            .field("to_add", &self.to_add.lock().unwrap())
            .field("to_remove", &self.to_remove.lock().unwrap())
            .finish()
    }
}

impl Table {
    /// Creates a new sharable thread-safe table.
    /// WARN: The component mask (`comp_mask`) must be sorted with the
    /// lowest component ID first.
    pub fn new(bit_mask: Box<[u8]>) -> Table {
        Table { 
            bit_mask: bit_mask,
            rows: UnsafeCell::new(VecDeque::new()), 
            to_add: Mutex::new(Vec::new()), 
            to_remove: Mutex::new(Some(AHashSet::default())),
            ref_access: Mutex::new(AHashSet::default()),
            locked: AtomicBool::new(false)
        }
    }

    /// Tests if the given component mask (`mask`) matches this tables
    /// assigned component mask (`comp_mask`).
    /// WARN: the given mask must be sorted with the lowest component ID first.
    pub fn matches_mask(&self, mask: &[u8]) -> bool {
        bit_masks_match(&self.bit_mask, mask)
    }

    /// Obtains an immutable reference to the contained rows in this table.
    /// This will only block if a mutablility lock has been triggered (usually by `sync`).
    /// The existence of other immutable references to the table WILL NOT
    /// block the thread executing this function.
    pub fn lock_ref(&self) -> RefTableGuard<'_> {
        // wait until table unlocked
        while self.locked.load(Ordering::Acquire) {}

        // record that this thread is holding an immutable reference to this table
        let mut access = self.ref_access.lock().unwrap();
        access.insert(std::thread::current().id());

        // return a guard
        RefTableGuard { mutex: self }
    }

    /// Executes any additions or removals that may be waiting to be executed.
    /// Before locking, this checks if there are any waiting additions or removals.
    /// If there is anything waiting, this will lock the table and wait for any
    /// immutable references to the table are dropped before to first perform
    /// removals, then perform additions.
    pub(crate) fn sync(&self) {
        // if there is nothing to add or remove, skip this function
        if self.to_add.lock().unwrap().is_empty() && self.to_remove.lock().unwrap().as_ref().unwrap().is_empty() {
            return;
        }

        // mark locked and insert this threads ID
        {
            self.locked.swap(true, Ordering::Release);
            self.ref_access.lock().unwrap().insert(std::thread::current().id());
        }

        // wait until only this thread has this table locked
        {
            loop {
                let count = {
                    let access = self.ref_access.lock().unwrap();
                    access.len()
                };
                if count <= 1 {
                    break;
                }
                std::thread::yield_now();
            }
        }

        // execute removes
        {
            // let remove_set = self.to_remove
            //     .lock()
            //     .unwrap()
            //     .drain()
            //     .collect::<HashSet<EntityID>>();
            // self.to_remove.lock_mut().take

            let new = AHashSet::new();
            let remove_set = std::mem::replace(self.to_remove.lock().unwrap().as_mut().unwrap(), new);

            unsafe { &mut *self.rows.get() }
                .retain(|entity| !remove_set.contains(&entity.lock_ref().0));
        }

        // execute additions
        {
            unsafe { &mut *self.rows.get() }
                .extend(
                    self.to_add
                        .lock()
                        .unwrap()
                        .drain(..)
                        .map(|entity| RelaxedMutex::new(entity))
                );
        }

        // unlock
        {
            self.locked.swap(false, Ordering::Release);
            self.ref_access.lock().unwrap().remove(&std::thread::current().id());
        }
    }

    /// Queues the given entity to be added to the table during the next `sync`
    /// call.  The `sync` function will be called by this function if there is
    /// no active immutable refrences to this table.
    pub fn insert(&self, entity: Entity) {
        self.to_add.lock().unwrap().push(entity);
        if self.ref_access.lock().unwrap().is_empty() {
            self.sync();
        }
    }

    /// Queues the entity with the given ID to be removed from thsi table during the
    /// next `sync` call.  The `sync` function will be called by this
    /// function if there is not active immutable references to this table.
    pub fn remove(&self, id: EntityID) {
        self.to_remove.lock().unwrap().as_mut().unwrap().insert(id);
        if self.ref_access.lock().unwrap().is_empty() {
            self.sync();
        }
    }

    /// Removes entities of all IDs in the given iterator.  If an ID is given
    /// that is not present in the table, it will be skipped.
    pub fn remove_all<'a, I: IntoIterator<Item = u32>>(&self, iter: I) {
        self.to_remove.lock().unwrap().as_mut().unwrap().extend(iter.into_iter());
    }
}

unsafe impl Send for Table {}
unsafe impl Sync for Table {}

/// A simple structure that gives an immutable reference to a tables rows (list of entities).
/// There may be any number of these existing at one time in your code, however, they
/// do not guarantee access to all entities in the table as those are protected by there
/// own mutex's.  Everytime a guard is dropped, an `sync` call is made to the referenced
/// table, see `Table` documentation for more information.
pub struct RefTableGuard<'a> {
    mutex: &'a Table
}

impl <'a> Deref for RefTableGuard<'a> {
    type Target = VecDeque<RelaxedMutex<Entity>>;
    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.rows.get() }
    }
}

impl Drop for RefTableGuard<'_> {
    /// On drop, remove this thread from the tracked list of threads.
    fn drop(&mut self) {
        {
            let mut access = self.mutex.ref_access.lock().unwrap();
            access.remove(&std::thread::current().id());
        }
        self.mutex.sync();
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::{atomic::Ordering, Arc, OnceLock}, time::Duration};

    use mutual::{AsAny, SharedData};

    use crate::{ecs::{components::{build_bit_mask, Component, ComponentID, ComponentMeta, NEXT_BIT_MASK}, tables::Table}};

    static TESTA_BIT_MASK: OnceLock<ComponentID> = OnceLock::new();
    
    pub struct TestA(String);
    impl ComponentMeta for TestA {
        fn bit_mask() -> ComponentID {
            *TESTA_BIT_MASK.get_or_init(|| {
                NEXT_BIT_MASK.fetch_add(1, Ordering::Relaxed)
            })
        }
    }
    impl Component for TestA {
        fn get_bit_mask(&self) -> ComponentID { Self::bit_mask() }
    }
    impl AsAny for TestA {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    #[test]
    pub fn test_shared_iter() {
        let table = Arc::new(Table::new(build_bit_mask(&[TestA::bit_mask()])));
        let table2 = table.clone();

        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(1));
            table2.lock_ref().iter().for_each(|entity_mutex| {
                let entity = entity_mutex.lock_ref();
                let component = entity.1.get(0).unwrap();
                let component = component.lock_ref();
                let component = component.as_any().downcast_ref::<TestA>().unwrap();
                println!("Component {:?}", component.0);
            });
            println!("Complete");
        });

        // table.insert(Entity(0, vec![Box::new(TestA("asdf".to_string()))].into_boxed_slice() as Box<[Box<dyn Component>]>));
        // table.insert(Entity(0, vec![Box::new(TestA("hjkl".to_string()))].into_boxed_slice() as Box<[Box<dyn Component>]>));

        // let _ = thread.join();
    }
}
