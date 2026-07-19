//! Anarchy is an experimental ECS crate aiming for the highest possible degree of
//! concurrency in an ECS `World`, by locking data at the lowest point needed (generally
//! the individual component or resource) rather than locking whole tables or the world.
//!
//! This crate is a work in progress: expect incomplete pieces (e.g. `BSTWorldDatabase`),
//! rough edges, and breaking changes. See the crate's README for usage examples.

use std::ops::{Deref, DerefMut};

use mutual::{AsAny, CastableSharedData, MutCastGuard, RefCastGuard, RelaxedMutex, SharedMap};

pub mod database;
pub mod ecs;
pub mod events;
pub mod flex_local;
pub mod logger;
pub mod scheduler;
pub mod thread_mutex;

pub use database::*;
pub use ecs::*;
pub use events::*;
pub use flex_local::*;
pub use logger::*;
pub use scheduler::*;
pub use thread_mutex::*;

pub use anarchy_macros as macros;
pub use anyhow as anyhow;

/// The central store of a running simulation: entities and their components (via the
/// `LinearDatabase` it derefs to) plus a map of singleton resources, each independently
/// lockable so systems can run concurrently against different parts of the world.
#[derive(Clone)]
pub struct World {
    resources: SharedMap<ResourceID, RelaxedMutex<Box<dyn Resource>>>,
    database: LinearDatabase
}

impl World {
    /// Returns a reference to this world's entity/component database.
    pub fn database(&self) -> &LinearDatabase { &self.database }

    /// Creates new World instance.
    pub fn new() -> Self {
        // create startup resources
        let resources = SharedMap::new();
        let tracker: Box<dyn Resource> = Box::new(EventTracker::default());
        resources.insert(tracker.get_id(), RelaxedMutex::new(tracker));

        Self {
            database: LinearDatabase::new(),
            resources
        }
    }

    /// Inserts a boxed resource into this world.
    pub fn insert_resource_box(&self, resource: Box<dyn Resource>) {
        if self.resources.contains(&resource.get_id()) { return }
        self.resources.insert(resource.get_id(), RelaxedMutex::new(resource));
    }

    /// Inserts a resource into this world.
    pub fn insert_resource<R: ResourceMeta + Resource + 'static>(&self, resource: R) {
        if self.resources.contains(&R::id()) { return }
        self.resources.insert(R::id(), RelaxedMutex::new(Box::new(resource)));
    }

    /// Removes a resource from this world by an arbitrary resource ID.
    pub fn remove_resource_raw(&self, id: ResourceID) {
        self.resources.remove(&id);
    }

    /// Removes a resource from this world.
    pub fn remove_resource<R: ResourceMeta + Resource>(&self) {
        self.resources.remove(&R::id());
    }

    /// Attempts to get a `RefGuard` to a boxed resource of the given id.
    pub fn get_resource_box_ref<'a>(&'a self, id: u32) -> Option<RelaxedMutex<Box<dyn Resource>>> {
        self.resources
            .get(&id)
            .map(|mutex| mutex.clone())
    }

    /// Attempts to get a `MutGuard` to a boxed resource of the given id.
    pub fn get_resource_box_mut<'a>(&'a self, id: u32) -> Option<RelaxedMutex<Box<dyn Resource>>> {
        self.resources
            .get(&id)
            .map(|mutex| mutex.clone())
    }

    /// Attempts to get a `RefCastGuard` to the resource specified as generic argument R.
    pub fn get_resource_ref<R: AsAny + ResourceMeta + Resource>(&self) -> Option<RefCastGuard<Box<dyn Resource>, R>> {
        self.resources
            .get(&R::id())
            .map(|mutex| mutex.lock_cast_ref())
    }

    /// Attempts to get a `MutCastGuard` to the resource specified as generic argument R.
    pub fn get_resource_mut<R: AsAny + ResourceMeta + Resource>(&self) -> Option<MutCastGuard<Box<dyn Resource>, R>> {
        self.resources
            .get(&R::id())
            .map(|mutex| mutex.lock_cast_mut())
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for World {
    type Target = LinearDatabase;
    fn deref(&self) -> &Self::Target { &self.database }
}

impl DerefMut for World {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.database }
}
