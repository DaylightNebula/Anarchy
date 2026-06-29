use std::thread::ThreadId;

use mutual::{DashMap, MutGuard, RefGuard, SharedData};

use crate::{RelaxedMutex, scheduler::ScheduleID};

/// A sharable data structure that contains an mutext to an independent instance of T
/// for each `FlexLocalId` given.
/// Clone will create an instance that references the same inner data of the original instance.
#[derive(Clone)]
pub struct FlexLocal<T: Default> {
    inner: DashMap<FlexLocalId, RelaxedMutex<T>>
}

impl <T: Default + 'static> Default for FlexLocal<T> {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum FlexLocalId {
    Schedule(ScheduleID),
    Thread(ThreadId),
    Other(u32)
}

impl <T: Default + 'static> FlexLocal<T> {
    // Create a new instance of `FlexLocal`.
    pub fn new() -> Self {
        Self { inner: DashMap::new() }
    }

    /// Gets an immutable reference to the saved data for the given id.
    /// WARN: This may block if a `MutGuard` is held to the same id until it is dropped.
    pub fn get(&self, id: FlexLocalId) -> RelaxedMutex<T> {
        // let inner = self.inner
        //     .compute_if_absent(id, || RelaxedMutex::new(T::default()));
        let inner = self.inner.entry(id)
            .or_insert_with(|| RelaxedMutex::new(T::default()));
        inner.clone()
    }

    /// Gets an immutable reference to the data local to the current thread.
    pub fn get_thread_local(&self) -> RelaxedMutex<T> {
        self.get(FlexLocalId::Thread(std::thread::current().id()))
    }

    /// Gets an immutable reference to the data local to the given schedules ID.
    pub fn get_schedule_local(&self, id: ScheduleID) -> RelaxedMutex<T> {
        let id = FlexLocalId::Schedule(id);
        self.get(id)
    }

    /// Gets an immutable reference to the data local to the given ID.
    pub fn get_other_local(&self, id: u32) -> RelaxedMutex<T> {
        self.get(FlexLocalId::Other(id))
    }

    /// Sets the data stored at the given id.
    pub fn set(&self, id: FlexLocalId, data: T) {
        self.inner.insert(id, RelaxedMutex::new(data));
    }

    /// Returns an iterator to immutable references to all data stored in the internal map.
    /// WARN: This does not guarantee elements will be returned if elements are added while iterating
    /// through the returned iterator.
    pub fn iter_ref(&self) -> impl Iterator<Item = (FlexLocalId, RefGuard<T>)> {
        self.inner.iter().map(|node| {
            let key = *node.key();
            let value = node.value().lock_ref();
            (key, value)
        })
    }

    /// Returns an iterator to mutable references to all data stored in the internal map.
    /// WARN: This does not guarantee elements will be returned if elements are added while iterating
    /// through the returned iterator.
    pub fn iter_mut(&self) -> impl Iterator<Item = (FlexLocalId, MutGuard<T>)> {
        self.inner.iter().map(|node| {
            let key = *node.key();
            let value = node.value().lock_mut();
            (key, value)
        })
    }
}

#[cfg(test)]
mod tests {
    use mutual::SharedData;
    use crate::{FlexLocal, FlexLocalId, ScheduleID};

    #[test]
    pub fn test_flex_local_schedule() {
        let id = ScheduleID { id: "TEST", tick_rate: 234, max_threads: 2 };
        let local = FlexLocal::default();
        local.set(FlexLocalId::Schedule(id), 1.0_f32);
        local.set(FlexLocalId::Schedule(id), 2.0_f32);
        
        assert!(*local.get_schedule_local(id).lock_ref() == 2.0);
    }
}
