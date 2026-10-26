use std::sync::atomic::{AtomicU64, Ordering};

use mutual::RelaxedMutex;

use crate::{Component, ComponentMeta, DynComponent, DynComponents, EntityID};

static NEXT_ENTITY_ID: AtomicU64 = AtomicU64::new(0);

/// Get the next unique entity ID.
pub fn next_entity_id() -> u64 {
    return NEXT_ENTITY_ID.fetch_add(1, Ordering::Relaxed);
}

/// Create a new entity builder.
///
/// ```rust
/// world.spawn(
///     EntityBuilder::new()
///         .add(TestComponentA)
///         .add(TestComponentB { id: 3 })
/// );
/// ```
#[derive(Debug)]
pub struct EntityBuilder {
    id: EntityID,
    comps: Vec<RelaxedMutex<DynComponent>>
}

impl Default for EntityBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityBuilder {
    /// Create a new entity builder.
    pub fn new() -> EntityBuilder {
        Self::with_capacity(0)
    }

    /// Create a new entity builder with a certain capacity.
    pub fn with_capacity(capacity: usize) -> EntityBuilder {
        EntityBuilder { id: next_entity_id(), comps: Vec::with_capacity(capacity) }
    }

    /// Add a component to this builder.
    pub fn push<C: Component + ComponentMeta + 'static>(mut self, component: C) -> Self {
        self.comps.push(RelaxedMutex::new(Box::new(component)));
        self
    }

    /// Get the ID of the entity that will be made from this builder.
    pub fn id(&self) -> EntityID {
        self.id
    }

    /// Get a immutable slice of all components in this builder.
    pub fn comps(&self) -> &[RelaxedMutex<DynComponent>] {
        &self.comps
    }

    pub fn complete(self) -> (EntityID, DynComponents) {
        (self.id, self.comps.into_boxed_slice())
    }
}
