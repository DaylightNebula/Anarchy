use std::{collections::LinkedList, sync::{Mutex, atomic::{AtomicU32, Ordering}}};

use mutual::SharedData;

/// A special structure that allows us to recycle U32 IDs as well as get
/// brand new ones.  When `next` is called, the first recycled ID in the
/// internal list will be removed from the list and returned, otherwise
/// a new ID will be created.  When `recycle` is called, the ID given
/// will be added to the internal recycle list.  Once an ID is returned
/// from `next` it MUST be recycled via `recycle` when it is no longer
/// in use to allow that ID to be used again.
pub struct U32Recycler {
    next_id: AtomicU32,
    recycled_ids: Mutex<LinkedList<u32>>
}

impl U32Recycler {
    /// Create a new `U32Recycler`.
    pub const fn new() -> Self {
        Self {
            next_id: AtomicU32::new(0),
            recycled_ids: Mutex::new(LinkedList::new())
        }
    }

    /// Gets a unused ID from this structure.  This ID will either be the first
    /// recycled ID returned to this structure, or the next new ID available.  New
    /// IDs start at 0 and increment by one each time.
    pub fn next(&self) -> u32 {
        let mut recycled_ids = self.recycled_ids.lock_mut();
        let recycled_id = recycled_ids.pop_front();
        if let Some(id) = recycled_id { return id }
        return self.next_id.fetch_add(1, Ordering::Release)
    }

    /// Recycle an ID that is no longer in use.  This ID may be returned
    /// by `next` in the future.
    pub fn recycle(&self, id: u32) {
        self.recycled_ids.lock_mut().push_back(id);
    }
}
