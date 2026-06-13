use std::fmt::Debug;

use mutual::{CastableSharedData, SharedData, relaxed_mutex::{MutCastGuard, RefCastGuard, RelaxedMutex}};

use crate::{U32Recycler, ecs::components::{self, Component, ComponentMeta}};

pub type EntityID = u32;
pub type EntityStorage = Box<[RelaxedMutex<Box<dyn Component>>]>;

pub static NEXT_ENTITY_ID: U32Recycler = U32Recycler::new();

/// Contains a sorted list of each component in a table entry
/// that represents an entities components.
/// When this object is created, the vector should be sorted
/// and then the order of the list should ALWAYS be preserved.
pub struct Entity(pub EntityID, pub EntityStorage);

impl Debug for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entity")
            .field("id", &self.0)
            .finish()
    }
}

impl Entity {
    pub fn id(&self) -> EntityID { self.0 }
    pub fn components(&self) -> &EntityStorage { &self.1 }

    pub fn new(components: EntityStorage) -> Self {
        Self(NEXT_ENTITY_ID.next(), components)
    }

    pub fn new_raw(id: EntityID, components: EntityStorage) -> Self {
        Self(id, components)
    }

    pub fn build_bit_mask(&self) -> Box<[u8]> {
        let ids = self.1.iter()
            .map(|a| a.lock_ref().get_bit_mask())
            .collect::<Vec<_>>();
        components::build_bit_mask(&ids)
    }

    pub fn lock_ref<'a, C: ComponentMeta>(&'a self) -> RefCastGuard<Box<dyn Component>, C> {
        self.1.iter()
            .filter(|a| 
                !a.current_thread_using() && a.lock_ref().get_bit_mask() == C::bit_mask()
            )
            .next()
            .unwrap()
            .lock_cast_ref()
    }

    pub fn lock_mut<'a, C: ComponentMeta>(&'a self) -> MutCastGuard<Box<dyn Component>, C> {
        self.1.iter()
            .filter(|a| 
                !a.current_thread_using() && a.lock_ref().get_bit_mask() == C::bit_mask()
                // a.try_lock_ref()
                //     .map(|a| a.get_bit_mask() == C::bit_mask())
                //     .unwrap_or(false)
            )
            .next()
            .unwrap()
            .lock_cast_mut()
    }
}

#[derive(Default)]
pub struct EntityBuilder(Vec<RelaxedMutex<Box<dyn Component>>>);
impl EntityBuilder {
    /// Add a mutex to a boxed component to this entity builder.
    pub fn add_raw(mut self, comp: RelaxedMutex<Box<dyn Component>>) -> Self {
        self.0.push(comp);
        return self;
    }

    /// Add a boxed component to this entity builder.
    pub fn add_boxed(mut self, comp: Box<dyn Component>) -> Self {
        self.0.push(RelaxedMutex::new(comp));
        return self;
    }

    /// Add a component to this entity builder.
    pub fn add<T: Component + 'static>(mut self, comp: T) -> Self {
        self.0.push(RelaxedMutex::new(Box::new(comp)));
        return self;
    }

    // Build this builder into an `Entity`.
    pub fn build(self) -> Entity {
        Entity::new(self.0.into_boxed_slice())
    }
}
