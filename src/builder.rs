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
    parent: Option<EntityID>,
    comps: Vec<RelaxedMutex<DynComponent>>,
    children: Vec<EntityBuilder>
}

impl Default for EntityBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityBuilder {
    /// Create a new entity builder.
    pub fn new() -> EntityBuilder {
        Self::with_id(next_entity_id())
    }

    /// Create new entity with a given ID.
    pub fn with_id(id: EntityID) -> EntityBuilder {
        Self::with_id_and_capacity(id, 1)
    }

    /// Create a new entity builder with a certain capacity.
    pub fn with_capacity(capacity: usize) -> EntityBuilder {
        Self::with_id_and_capacity(next_entity_id(), capacity)
    }

    /// Create a new entity builder with a certain ID and capacity.
    pub fn with_id_and_capacity(id: EntityID, capacity: usize) -> EntityBuilder {
        EntityBuilder { id, parent: None, comps: Vec::with_capacity(capacity), children: Vec::new() }
    }

    /// Set the parent entity of this builder.  If a parent of the
    /// given ID does not exist when the builder is spawned, no
    /// parent will be set.
    pub fn parent(mut self, parent_id: EntityID) -> Self {
        self.parent = Some(parent_id);
        return self;
    }

    /// Add a component to this builder.
    pub fn push<C: Component + ComponentMeta + 'static>(mut self, component: C) -> Self {
        self.comps.push(RelaxedMutex::new(Box::new(component)));
        return self;
    }

    /// Push an iterator of components to the builders component tracker.
    pub fn extend<I>(mut self, iter: I) -> Self
        where I: Iterator<Item = DynComponent>
    {
        iter.for_each(|c| self.comps.push(RelaxedMutex::new(c)));
        return self;
    }

    /// Add a child builder to this entity builder.
    pub fn push_children(mut self, child: EntityBuilder) -> Self {
        self.children.push(child);
        return self;
    }

    /// Extend the children in this builder with an iterator of builders.
    pub fn extend_children<I>(mut self, iter: I) -> Self
        where I: Iterator<Item = EntityBuilder>
    {
        self.children.extend(iter);
        return self;
    }

    /// Get the ID of the entity that will be made from this builder.
    pub fn id(&self) -> EntityID {
        self.id
    }

    /// Get a immutable slice of all components in this builder.
    pub fn comps(&self) -> &[RelaxedMutex<DynComponent>] {
        &self.comps
    }

    /// Consume this entity builder and turn it into its components.
    pub fn complete(self) -> (EntityID, Option<EntityID>, DynComponents, Vec<EntityBuilder>) {
        (self.id, self.parent, self.comps.into_boxed_slice(), self.children)
    }
}
