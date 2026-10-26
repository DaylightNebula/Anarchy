//! Worlds, which hold every entity and resource.

use std::{fmt::Debug, ops::Deref, sync::Arc};

use anyhow::Context;
use mutual::{CastableSharedData, DashMap, MutCastGuard, MutGuard, RefCastGuard, RefGuard, RelaxedMutex, SharedData};

use crate::{Bundle, ChunkedTable, ComponentID, ComponentIDGroup, ComponentInfo, ComponentMut, ComponentRef, DynComponents, DynResource, EntityBuilder, EntityID, EventQueue, Hierarchy, QueryGroup, Resource, ResourceID, ResourceMeta, tables::chunked::unbox};

pub mod handle;
pub mod indexed;
pub mod list;

pub use handle::*;
pub use indexed::*;
pub use list::*;

/// A shared handle to a world's entities, resources and [`Hierarchy`].  Clones
/// are cheap and share the same data.
///
/// Derefs to the [`WorldImpl`] storing its entities, [`IndexedWorld`] by default.
#[derive(Clone)]
pub struct World(Arc<WorldInner>);

/// The data behind a [`World`], shared by its clones.
struct WorldInner {
    entities: Box<dyn WorldImpl>,
    resources: DashMap<ResourceID, RelaxedMutex<DynResource>>,
    hierarchy: Hierarchy
}

impl World {
    /// Create a world that stores its entities in `world`.
    pub fn new<W: WorldImpl + 'static>(world: W) -> Self {
        Self(Arc::new(WorldInner { entities: Box::new(world), resources: DashMap::default(), hierarchy: Hierarchy::new() }))
    }

    /// A handle to the entity `id` in this world, see [`EntityHandle`].
    pub fn handle(&self, id: EntityID) -> EntityHandle<'_> {
        EntityHandle::new(id, self)
    }

    /// The parent and child links between this world's entities.
    pub fn hierarchy(&self) -> &Hierarchy {
        &self.0.hierarchy
    }

    /// Insert a resource into the world, replacing any resource of the same type.
    pub fn insert_resource<R: Resource>(&self, resource: R) {
        let resource: DynResource = Box::new(resource);
        self.0.resources.insert(resource.get_id(), RelaxedMutex::new(resource));
    }

    /// Insert the resource built by `f` unless the world already holds one of
    /// type `R`.  Safe to race, `f` only runs for the call that inserts.
    pub fn init_resource_with<R: ResourceMeta>(&self, f: impl FnOnce() -> R) {
        self.0.resources.entry(R::id())
            .or_insert_with(|| RelaxedMutex::new(Box::new(f()) as DynResource));
    }

    /// Send an event from outside any system, see [`Event::send`](crate::Event::send).
    pub fn send_event<E: Clone + Send + 'static>(&self, event: E) {
        self.init_resource_with(EventQueue::<E>::default);
        self.resource_mut::<EventQueue<E>>().unwrap().send(event);
    }

    /// Remove a resource from the world, returns true if it was present.
    /// Existing guards to the resource stay valid until dropped.
    pub fn remove_resource<R: ResourceMeta>(&self) -> bool {
        self.0.resources.remove(&R::id()).is_some()
    }

    /// Remove the resource `id` from the world, returns true if it was present.
    pub fn remove_resource_dyn(&self, id: ResourceID) -> bool {
        self.0.resources.remove(&id).is_some()
    }

    /// Returns true if the world holds a resource of type `R`.
    pub fn has_resource<R: ResourceMeta>(&self) -> bool {
        self.0.resources.contains_key(&R::id())
    }

    /// Get immutable access to a resource, blocks while a mutable guard to it is held.
    pub fn resource<R: ResourceMeta>(&self) -> anyhow::Result<RefCastGuard<Box<dyn Resource>, R>> {
        let res = &*self.0.resources.get(&R::id()).context("No resource found")?;
        let res = res.lock_cast_ref();
        Ok(res)
    }

    /// Get mutable access to a resource, blocks while any other guard to it is held.
    pub fn resource_mut<R: ResourceMeta>(&self) -> anyhow::Result<MutCastGuard<Box<dyn Resource>, R>> {
        let res = &*self.0.resources.get(&R::id()).context("No resource found")?;
        let res = res.lock_cast_mut();
        Ok(res)
    }

    /// The ids of every resource in the world.
    pub fn resource_ids(&self) -> Vec<ResourceID> {
        self.0.resources.iter().map(|entry| *entry.key()).collect()
    }

    /// Type erased immutable access to the resource `id`, or `None` if there is none.
    /// Blocks while a mutable guard to it is held.
    pub fn resource_dyn(&self, id: ResourceID) -> Option<RefGuard<DynResource>> {
        // clone the mutex out so the map isn't locked while waiting on the resource
        let res = self.0.resources.get(&id)?.clone();
        Some(res.lock_ref())
    }

    /// Type erased mutable access to the resource `id`, or `None` if there is none.
    /// Blocks while any other guard to it is held.
    pub fn resource_dyn_mut(&self, id: ResourceID) -> Option<MutGuard<DynResource>> {
        let res = self.0.resources.get(&id)?.clone();
        Some(res.lock_mut())
    }

    /// Insert a type erased resource, replacing any resource of the same type.
    pub fn insert_resource_dyn(&self, resource: DynResource) {
        self.0.resources.insert(resource.get_id(), RelaxedMutex::new(resource));
    }

    /// Spawn an entity from the given entity builder.  Calls `insert` on the id and
    /// components pulled from the builder.
    ///
    /// # Panics
    ///
    /// Panics where [`try_insert`](Self::try_insert) would return an error.
    pub fn spawn(&self, builder: EntityBuilder) -> EntityID {
        self.spawn_with_parent(None, builder)
    }

    /// Spawn an entity from the given entity builder with the given optional
    /// parent.  Calls `insert` on the id and components pulled from the builder.
    ///
    /// # Panics
    ///
    /// Panics where [`try_insert`](Self::try_insert) would return an error.
    pub fn spawn_with_parent(&self, parent: Option<EntityID>, builder: EntityBuilder) -> EntityID {
        let (id, overide_parent, comps, children) = builder.complete();
        self.insert(id, comps);

        if let Some(parent) = overide_parent.or(parent) {
            let _ = self.hierarchy().set_parent(id, parent);
        }

        for child in children.into_iter() {
            self.spawn_with_parent(Some(id), child);
        }

        return id;
    }
}

impl Deref for World {
    type Target = dyn WorldImpl;
    fn deref(&self) -> &Self::Target {
        &*self.0.entities
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new(IndexedWorld::new())
    }
}

/// An entity and the table it lives in, see [`WorldImpl::entity`].  Cheap to clone,
/// and nothing is borrowed until a method is called.
#[derive(Clone)]
pub struct Entity {
    /// The entity's id.
    pub id: EntityID,
    /// The sorted ids of the entity's components, shared with its table.
    pub group: Arc<ComponentIDGroup>,
    /// The table holding the entity.
    pub table: Arc<ChunkedTable>
}

impl Entity {
    /// The component `id` borrowed shared, or `None` if the entity has no such component
    /// or has been removed.  Blocks while a mutable guard holds it.
    pub fn component(&self, id: ComponentID) -> Option<ComponentRef> {
        self.table.component(self.id, id)
    }

    /// The component `id` borrowed mutably, see [`component`](Self::component).  Blocks
    /// while any other guard holds it.
    pub fn component_mut(&self, id: ComponentID) -> Option<ComponentMut> {
        self.table.component_mut(self.id, id)
    }

    /// Guards over the components in `QG`, or `None` if the entity has been removed.
    ///
    /// # Errors
    ///
    /// Errors if the entity is missing a required component.
    pub fn get<QG: QueryGroup>(&self) -> anyhow::Result<Option<QG::Output>> {
        self.table.fetch::<QG>(self.id)
    }
}

impl Debug for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Entity").field("id", &self.id).field("group", &self.group).finish()
    }
}

/// Entity storage behind a [`World`], which groups entities into [`ChunkedTable`]s by
/// their set of components.  Any number of threads may use it at once.
///
/// Entities are also indexed by id so they can be found without walking a table.
/// Ids are not checked for uniqueness: the index keeps the last entity inserted
/// with an id, while every copy stays in the tables.
pub trait WorldImpl: Send + Sync {
    /// The table for exactly the components `infos` describe, in any order, creating it
    /// if there is none.
    ///
    /// # Errors
    ///
    /// Errors if there are no infos, or one is listed twice or isn't registered.
    fn table_for(&self, infos: Vec<&'static ComponentInfo>) -> anyhow::Result<Arc<ChunkedTable>>;

    /// Indexes `id` as living in `table`, after inserting it there.
    fn index(&self, id: EntityID, table: Arc<ChunkedTable>);

    /// Every table, in no particular order.
    fn tables(&self) -> Vec<Arc<ChunkedTable>>;

    /// Create a raw query that produces an iterator over the tables that hold every
    /// required component given.  Required components may not be empty.
    fn raw_query<'a>(&'a self, req_components: ComponentIDGroup) -> Box<dyn Iterator<Item = Arc<ChunkedTable>>>;

    /// The entity with `id`, or `None` if there is none.  A single hash lookup.
    fn entity(&self, id: EntityID) -> Option<Entity> {
        self.table_of(id).map(|table| Entity { id, group: table.shared_group(), table })
    }

    /// The table holding the entity with `id`, or `None` if there is none.
    fn table_of(&self, id: EntityID) -> Option<Arc<ChunkedTable>>;

    /// The id of every entity, in no particular order.  Parts of the by-id index stay
    /// read locked while iterating, so inserts and removes may wait until it is dropped.
    fn entity_ids(&self) -> Box<dyn Iterator<Item = EntityID> + '_>;

    /// Remove the entity with `id` and return its components, or `None` if there is none.
    /// Waits for every guard on its components, so don't call it while holding one.
    /// Components whose types are only bytes are dropped rather than returned.
    fn remove(&self, id: EntityID) -> Option<DynComponents>;

    /// Insert a new entity into the world with a given set of boxed components, which
    /// are moved into its table.  An entity without components is stored, but no query
    /// finds it.
    ///
    /// # Panics
    ///
    /// Panics where [`try_insert`](Self::try_insert) would return an error.
    fn insert(&self, entity_id: EntityID, components: DynComponents) {
        self.try_insert(entity_id, components).unwrap_or_else(|error| panic!("can't insert entity {entity_id}: {error:#}"))
    }

    /// [`insert`](Self::insert), returning an error instead of panicking.
    ///
    /// # Errors
    ///
    /// Errors if two are the same component, or one has no [`ComponentInfo`] (see
    /// [`Component::info`]).
    fn try_insert(&self, entity_id: EntityID, components: DynComponents) -> anyhow::Result<()> {
        let components = unbox(components);
        let infos = components.iter()
            .map(|component| component.info().with_context(|| format!("{component:?} has no registered ComponentInfo")))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let table = self.table_for(infos)?;
        table.insert_dyn(entity_id, components)?;
        self.index(entity_id, table);
        Ok(())
    }

    /// Insert a new entity with the components in `bundle`, without boxing them.
    ///
    /// # Errors
    ///
    /// Errors if `bundle` lists a component twice.
    fn insert_bundle<B: Bundle>(&self, entity_id: EntityID, bundle: B) -> anyhow::Result<()> where Self: Sized {
        let table = self.table_for((0..B::LEN).map(B::info).collect())?;
        table.insert(entity_id, bundle)?;
        self.index(entity_id, table);
        Ok(())
    }

    /// Guards over the components in `QG` on the entity with `id`, or `None` if there is
    /// no such entity.  See [`Query`](crate::Query) for what `QG` may be.
    ///
    /// # Errors
    ///
    /// Errors if the entity is missing a required component.
    fn get<QG: QueryGroup>(&self, id: EntityID) -> anyhow::Result<Option<QG::Output>> where Self: Sized {
        let Some(table) = self.table_of(id) else { return Ok(None) };
        table.fetch::<QG>(id)
    }

    /// Tries to [compact](ChunkedTable::try_compact) every table, returns how many did.
    fn compact(&self) -> usize {
        self.tables().iter().filter(|table| table.try_compact()).count()
    }
}

impl WorldImpl for World {
    fn table_for(&self, infos: Vec<&'static ComponentInfo>) -> anyhow::Result<Arc<ChunkedTable>> {
        self.0.entities.table_for(infos)
    }

    fn index(&self, id: EntityID, table: Arc<ChunkedTable>) {
        self.0.entities.index(id, table)
    }

    fn tables(&self) -> Vec<Arc<ChunkedTable>> {
        self.0.entities.tables()
    }

    fn raw_query<'a>(&'a self, req_components: ComponentIDGroup) -> Box<dyn Iterator<Item = Arc<ChunkedTable>>> {
        self.0.entities.raw_query(req_components)
    }

    fn entity(&self, id: EntityID) -> Option<Entity> {
        self.0.entities.entity(id)
    }

    fn table_of(&self, id: EntityID) -> Option<Arc<ChunkedTable>> {
        self.0.entities.table_of(id)
    }

    fn entity_ids(&self) -> Box<dyn Iterator<Item = EntityID> + '_> {
        self.0.entities.entity_ids()
    }

    /// Also unlinks the entity in the [`Hierarchy`], leaving its children as roots.
    fn remove(&self, id: EntityID) -> Option<DynComponents> {
        let removed = self.0.entities.remove(id);
        self.0.hierarchy.remove(id);
        removed
    }
}

/// A concurrent map keyed by entity id, hashed with FxHash since ids are already well spread.
pub(crate) type IdMap<V> = DashMap<EntityID, V, rustc_hash::FxBuildHasher>;

/// Removes `id` from `entities` and takes its components out of its table, for worlds
/// that index entities in a map.
pub(crate) fn remove_indexed(entities: &IdMap<Arc<ChunkedTable>>, id: EntityID) -> Option<DynComponents> {
    let (_, table) = entities.remove(&id)?;
    table.take(id).map(|components| components.into_iter().map(RelaxedMutex::new).collect())
}

/// The sorted group of the components `infos` describe.
pub(crate) fn group_of(infos: &[&'static ComponentInfo]) -> ComponentIDGroup {
    let mut group = infos.iter().map(|info| info.id).collect::<ComponentIDGroup>();
    group.sort_unstable();
    group
}

/// Returns true if every id in `subset` is in `set`, both must be sorted.
pub(crate) fn group_matches(
    set: &ComponentIDGroup,
    subset: &ComponentIDGroup
) -> bool {
    if set.len() < subset.len() { return false }

    let mut subset_ptr = 0;
    for elem in set.iter() {
        let addr = subset[subset_ptr];
        if addr < *elem { return false }
        if addr == *elem {
            subset_ptr += 1;
            if subset_ptr >= subset.len() { return true }
        }
    }

    return false;
}

/// Behaviour tests shared by every `WorldImpl`, the world type must implement `test_support::TestWorld`.
#[cfg(test)]
macro_rules! world_tests {
    ($world:ty) => {
        use crate::worlds::test_support::*;

        fn world() -> $world { <$world as TestWorld>::new_world() }

        // ---- insert ----

        #[test]
        fn insert_creates_table_per_group() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1))]));
            assert_eq!(world.table_count(), 1);
            world.insert(2, comps(vec![Box::new(A(2))]));
            assert_eq!(world.table_count(), 1);
            world.insert(3, comps(vec![Box::new(A(3)), Box::new(B(3))]));
            assert_eq!(world.table_count(), 2);
            world.insert(4, comps(vec![Box::new(B(4))]));
            assert_eq!(world.table_count(), 3);
        }

        #[test]
        fn insert_reuses_table_regardless_of_component_order() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(1)), Box::new(C(1))]));
            world.insert(2, comps(vec![Box::new(C(2)), Box::new(A(2)), Box::new(B(2))]));
            world.insert(3, comps(vec![Box::new(B(3)), Box::new(C(3)), Box::new(A(3))]));
            assert_eq!(world.table_count(), 1);
            assert_eq!(entities(&world, &[A::id(), B::id(), C::id()]), [1, 2, 3]);
        }

        #[test]
        fn insert_does_not_reuse_superset_or_subset_table() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(1))]));
            // a subset of an existing group must get its own table
            world.insert(2, comps(vec![Box::new(A(2))]));
            // as must a superset
            world.insert(3, comps(vec![Box::new(A(3)), Box::new(B(3)), Box::new(C(3))]));
            // and a same-sized group that only overlaps
            world.insert(4, comps(vec![Box::new(A(4)), Box::new(C(4))]));
            assert_eq!(world.table_count(), 4);

            let groups = world.groups();
            for expected in [
                sorted(vec![A::id(), B::id()]),
                sorted(vec![A::id()]),
                sorted(vec![A::id(), B::id(), C::id()]),
                sorted(vec![A::id(), C::id()])
            ] {
                assert_eq!(groups.iter().filter(|g| **g == expected).count(), 1, "missing group {expected:?}");
            }
        }

        #[test]
        fn insert_stores_components_sorted_by_id() {
            let world = world();
            world.insert(1, comps(vec![Box::new(D(1)), Box::new(B(1)), Box::new(C(1)), Box::new(A(1))]));

            let expected = sorted(vec![A::id(), B::id(), C::id(), D::id()]);
            assert_eq!(world.groups(), [expected.clone()]);
            assert_eq!(query(&world, &[A::id()]), [(1, expected)]);
        }

        #[test]
        fn insert_keeps_component_values_with_their_entity() {
            let world = world();
            for i in 0..50 {
                world.insert(i, comps(vec![Box::new(B(i as u32 * 2)), Box::new(A(i as u32))]));
            }

            let mut seen = entities(&world, &[A::id(), B::id()]).into_iter()
                .map(|id| { let (a, b) = world.get::<(&A, &B)>(id).unwrap().unwrap(); (id, a.0, b.0) })
                .collect::<Vec<_>>();
            seen.sort();
            assert_eq!(seen, (0..50).map(|i| (i, i as u32, i as u32 * 2)).collect::<Vec<_>>());
        }

        #[test]
        fn insert_many_entities_across_many_groups() {
            let world = world();
            for i in 0..160u32 {
                let mask = i % 16;
                if mask == 0 { continue }
                world.insert(i as EntityID, comps(by_mask(mask)));
            }
            assert_eq!(world.table_count(), 15);

            let expect = |bits: u32| -> Vec<EntityID> {
                (0..160u64).filter(|i| i % 16 != 0 && (i % 16) as u32 & bits == bits).collect()
            };
            assert_eq!(entities(&world, &[A::id()]), expect(1));
            assert_eq!(entities(&world, &[B::id()]), expect(2));
            assert_eq!(entities(&world, &[A::id(), C::id()]), expect(5));
            assert_eq!(entities(&world, &[B::id(), C::id(), D::id()]), expect(14));
            assert_eq!(entities(&world, &[A::id(), B::id(), C::id(), D::id()]), expect(15));
        }

        #[test]
        fn insert_duplicate_entity_ids_are_kept() {
            // the world doesn't dedupe ids, both copies should be stored
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1))]));
            world.insert(1, comps(vec![Box::new(A(2))]));
            assert_eq!(entities(&world, &[A::id()]), [1, 1]);
        }

        #[test]
        fn concurrent_inserts_lose_nothing() {
            const THREADS: u64 = 8;
            const PER_THREAD: u64 = 2_000;

            let world = world();
            std::thread::scope(|s| for t in 0..THREADS {
                let world = &world;
                s.spawn(move || for i in 0..PER_THREAD {
                    let id = t * PER_THREAD + i;
                    let c: Vec<DynComponent> = match id % 3 {
                        0 => vec![Box::new(A(0))],
                        1 => vec![Box::new(A(0)), Box::new(B(0))],
                        _ => vec![Box::new(B(0)), Box::new(C(0))]
                    };
                    world.insert(id, comps(c));
                });
            });

            let all = |filter: fn(&u64) -> bool| (0..THREADS * PER_THREAD).filter(filter).collect::<Vec<_>>();
            assert_eq!(entities(&world, &[A::id()]), all(|i| i % 3 != 2));
            assert_eq!(entities(&world, &[B::id()]), all(|i| i % 3 != 0));
            assert_eq!(entities(&world, &[C::id()]), all(|i| i % 3 == 2));
            assert_eq!(world.table_count(), 3);
        }

        #[test]
        fn concurrent_new_group_creates_one_table() {
            const THREADS: usize = 8;
            for _ in 0..200 {
                let world = world();
                let barrier = std::sync::Barrier::new(THREADS);
                std::thread::scope(|s| for t in 0..THREADS {
                    let (world, barrier) = (&world, &barrier);
                    s.spawn(move || {
                        barrier.wait();
                        world.insert(t as EntityID, comps(vec![Box::new(A(0)), Box::new(B(0))]));
                    });
                });
                assert_eq!(world.table_count(), 1);
                assert_eq!(entities(&world, &[A::id(), B::id()]), (0..THREADS as EntityID).collect::<Vec<_>>());
            }
        }

        #[test]
        fn concurrent_inserts_are_all_indexed() {
            const THREADS: u64 = 8;
            const PER_THREAD: u64 = 500;

            let world = world();
            std::thread::scope(|s| for t in 0..THREADS {
                let world = &world;
                s.spawn(move || for i in 0..PER_THREAD {
                    let id = t * PER_THREAD + i;
                    world.insert(id, comps(by_mask(id as u32 % 15 + 1)));
                });
            });

            for id in 0..THREADS * PER_THREAD {
                let entity = world.entity(id).expect("entity missing from index");
                let expected = sorted(by_mask(id as u32 % 15 + 1).iter().map(|c| c.get_id()).collect());
                assert_eq!(entity.group.to_vec(), expected);
            }
        }

        // ---- entity / get / remove ----

        #[test]
        fn entity_finds_components_by_id() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1))]));
            world.insert(2, comps(vec![Box::new(C(20)), Box::new(A(2)), Box::new(B(10))]));
            world.insert(3, comps(vec![Box::new(B(30))]));

            let entity = world.entity(2).unwrap();
            assert_eq!(entity.group.to_vec(), sorted(vec![A::id(), B::id(), C::id()]));
            assert_eq!(entity.get::<&A>().unwrap().unwrap().0, 2);
            assert_eq!(entity.get::<&B>().unwrap().unwrap().0, 10);
            assert_eq!(entity.get::<&C>().unwrap().unwrap().0, 20);
            assert_eq!(world.entity(3).unwrap().get::<&B>().unwrap().unwrap().0, 30);
            assert_eq!(format!("{:?}", entity.component(C::id()).unwrap()), "C(20)");
            entity.component_mut(C::id()).unwrap().downcast_mut::<C>().unwrap().0 = 21;
            assert_eq!(world.get::<&C>(2).unwrap().unwrap().0, 21);
            assert!(entity.component(D::id()).is_none());
        }

        #[test]
        fn unknown_entity_is_none() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1))]));
            assert!(world.entity(2).is_none());
            assert!(world.get::<&A>(2).unwrap().is_none());
            assert!(world.remove(2).is_none());
            assert_eq!(entities(&world, &[A::id()]), [1]);
        }

        #[test]
        fn get_writes_through_to_storage() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(10))]));
            world.insert(2, comps(vec![Box::new(A(2)), Box::new(B(20))]));
            {
                let (mut a, b) = world.get::<(&mut A, &B)>(2).unwrap().unwrap();
                a.0 += b.0;
            }

            let mut values = entities(&world, &[A::id()]).into_iter()
                .map(|id| (id, world.get::<&A>(id).unwrap().unwrap().0))
                .collect::<Vec<_>>();
            values.sort();
            assert_eq!(values, [(1, 1), (2, 22)]);
        }

        #[test]
        fn get_missing_component() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1))]));
            assert!(world.get::<(&A, &B)>(1).is_err());
            let (a, b) = world.get::<(&A, Option<&B>)>(1).unwrap().unwrap();
            assert_eq!((a.0, b.is_none()), (1, true));
        }

        #[test]
        fn remove_clears_index_and_table() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(1))]));
            world.insert(2, comps(vec![Box::new(A(2)), Box::new(B(2))]));
            world.insert(3, comps(vec![Box::new(A(3))]));

            let removed = world.remove(2).unwrap();
            assert_eq!(value_of::<A>(&removed, |a| a.0), 2);
            assert!(world.entity(2).is_none());
            assert!(world.remove(2).is_none());
            assert_eq!(entities(&world, &[A::id()]), [1, 3]);
            assert!(world.entity(1).is_some());
        }

        // ---- raw_query ----

        #[test]
        fn query_empty_world() {
            let world = world();
            assert_eq!(world.raw_query(ids(&[A::id()])).count(), 0);
        }

        #[test]
        fn query_missing_component_yields_nothing() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(1))]));
            assert_eq!(world.raw_query(ids(&[C::id()])).count(), 0);
            assert_eq!(world.raw_query(ids(&[A::id(), C::id()])).count(), 0);
        }

        #[test]
        fn query_matches_every_table_containing_requirements() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1))]));
            world.insert(2, comps(vec![Box::new(A(2)), Box::new(B(2))]));
            world.insert(3, comps(vec![Box::new(B(3))]));
            world.insert(4, comps(vec![Box::new(A(4)), Box::new(B(4)), Box::new(C(4))]));
            world.insert(5, comps(vec![Box::new(A(5)), Box::new(C(5))]));

            assert_eq!(world.raw_query(ids(&[A::id()])).count(), 4);
            assert_eq!(entities(&world, &[A::id()]), [1, 2, 4, 5]);
            assert_eq!(entities(&world, &[B::id()]), [2, 3, 4]);
            assert_eq!(entities(&world, &[C::id()]), [4, 5]);
            assert_eq!(entities(&world, &[A::id(), B::id()]), [2, 4]);
            assert_eq!(entities(&world, &[A::id(), C::id()]), [4, 5]);
            assert_eq!(entities(&world, &[B::id(), C::id()]), [4]);
            assert_eq!(entities(&world, &[A::id(), B::id(), C::id()]), [4]);
            assert_eq!(entities(&world, &[D::id()]), [] as [EntityID; 0]);
        }

        #[test]
        fn query_requirement_order_does_not_matter() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(1)), Box::new(C(1))]));
            world.insert(2, comps(vec![Box::new(A(2)), Box::new(C(2))]));

            let expected = entities(&world, &[A::id(), C::id()]);
            assert_eq!(expected, [1, 2]);
            assert_eq!(entities(&world, &[C::id(), A::id()]), expected);
            assert_eq!(entities(&world, &[C::id(), B::id(), A::id()]), [1]);
            assert_eq!(entities(&world, &[B::id(), A::id(), C::id()]), [1]);
        }

        #[test]
        fn query_returns_all_components_of_matched_entities() {
            let world = world();
            world.insert(1, comps(vec![Box::new(C(1)), Box::new(A(1)), Box::new(B(1))]));
            // only asking for B still hands back the whole entity, sorted by id
            assert_eq!(query(&world, &[B::id()]), [(1, sorted(vec![A::id(), B::id(), C::id()]))]);
        }

        #[test]
        fn query_yields_one_cursor_per_matching_table() {
            let world = world();
            for i in 0..5 { world.insert(i, comps(vec![Box::new(A(0))])); }
            for i in 5..8 { world.insert(i, comps(vec![Box::new(A(0)), Box::new(B(0))])); }

            let mut sizes = world.raw_query(ids(&[A::id()]))
                .map(|table| table.len())
                .collect::<Vec<_>>();
            sizes.sort();
            assert_eq!(sizes, [3, 5]);
        }

        #[test]
        fn query_sees_entities_inserted_after_creation() {
            // cursors are created lazily, so an entity added to a table before it's reached is still visited
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1))]));
            let cursors = world.raw_query(ids(&[A::id()]));
            world.insert(2, comps(vec![Box::new(A(2))]));
            let found = cursors.flat_map(|table| table.ids()).collect::<Vec<_>>();
            assert!(found.contains(&1));
            assert!(found.contains(&2));
        }

        #[test]
        fn query_components_share_storage_with_world() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(10))]));

            // mutate through one query...
            for table in world.raw_query(ids(&[A::id()])) {
                table.for_each::<&mut A>(|_, a| a.0 = 42).unwrap();
            }

            // ...and see it through another
            let mut values = vec![];
            for table in world.raw_query(ids(&[B::id(), A::id()])) {
                table.for_each::<(&A, &B)>(|_, (a, b)| values.push((a.0, b.0))).unwrap();
            }
            assert_eq!(values, [(42, 10)]);
        }

        #[test]
        fn query_is_repeatable() {
            let world = world();
            for i in 0..10 { world.insert(i, comps(vec![Box::new(A(0)), Box::new(B(0))])); }
            let first = query(&world, &[A::id()]);
            assert_eq!(first.len(), 10);
            assert_eq!(query(&world, &[A::id()]), first);
            assert_eq!(query(&world, &[A::id()]), first);
        }
    };
}

#[cfg(test)]
pub(crate) use world_tests;

#[cfg(test)]
pub(crate) mod test_support {
    use mutual::{RelaxedMutex, SharedData};

    use crate::*;

    /// Hooks the shared `world_tests!` need to look inside a world.
    pub trait TestWorld: WorldImpl + Sync + Sized {
        fn new_world() -> Self;
        fn table_count(&self) -> usize;
        /// The group of every table, in no particular order.
        fn groups(&self) -> Vec<Vec<ComponentID>>;
    }

    macro_rules! components {
        ($($name:ident),*) => {$(
            #[derive(Debug, PartialEq, Component)]
            pub struct $name(pub u32);
        )*};
    }

    components!(A, B, C, D);

    pub fn comps(components: Vec<DynComponent>) -> DynComponents {
        components.into_iter().map(RelaxedMutex::new).collect()
    }

    /// One of A..D for each of the low 4 bits set in `mask`.
    pub fn by_mask(mask: u32) -> Vec<DynComponent> {
        let mut c: Vec<DynComponent> = vec![];
        if mask & 1 != 0 { c.push(Box::new(A(mask))); }
        if mask & 2 != 0 { c.push(Box::new(B(mask))); }
        if mask & 4 != 0 { c.push(Box::new(C(mask))); }
        if mask & 8 != 0 { c.push(Box::new(D(mask))); }
        c
    }

    pub fn ids(ids: &[ComponentID]) -> ComponentIDGroup {
        Box::from(ids)
    }

    /// Every (entity, component ids) pair the query yields, sorted by entity so table and insertion order don't matter.
    pub fn query(world: &impl WorldImpl, req: &[ComponentID]) -> Vec<(EntityID, Vec<ComponentID>)> {
        let mut out = world.raw_query(ids(req))
            .flat_map(|table| table.ids().into_iter().map(move |id| (id, table.group().to_vec())))
            .collect::<Vec<_>>();
        out.sort();
        out
    }

    pub fn entities(world: &impl WorldImpl, req: &[ComponentID]) -> Vec<EntityID> {
        query(world, req).into_iter().map(|(id, _)| id).collect()
    }

    pub fn sorted(mut ids: Vec<ComponentID>) -> Vec<ComponentID> {
        ids.sort();
        ids
    }

    pub fn value_of<T: ComponentMeta>(components: &DynComponents, read: fn(&T) -> u32) -> u32 {
        let comp = components.iter().find(|c| c.lock_ref().get_id() == T::id()).unwrap();
        read(comp.lock_ref().as_any().downcast_ref::<T>().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_support::ids;

    #[test]
    fn group_matches_subsets() {
        let set = ids(&[1, 3, 5, 7]);
        assert!(group_matches(&set, &ids(&[1, 3, 5, 7])));
        assert!(group_matches(&set, &ids(&[1])));
        assert!(group_matches(&set, &ids(&[7])));
        assert!(group_matches(&set, &ids(&[3, 7])));
        assert!(group_matches(&set, &ids(&[1, 5])));

        assert!(!group_matches(&set, &ids(&[2])));
        assert!(!group_matches(&set, &ids(&[1, 2])));
        assert!(!group_matches(&set, &ids(&[7, 8])));
        assert!(!group_matches(&set, &ids(&[0, 1])));
        assert!(!group_matches(&set, &ids(&[1, 3, 5, 7, 9])));
        assert!(!group_matches(&ids(&[1]), &ids(&[1, 1])));
    }
}

#[cfg(test)]
mod resource_tests {
    use crate::*;

    #[derive(Debug, PartialEq, Resource)]
    struct Counter(u32);

    #[derive(Debug, Resource)]
    struct Other;

    #[test]
    fn missing_resource() {
        let world = World::default();
        assert!(!world.has_resource::<Counter>());
        assert!(world.resource::<Counter>().is_err());
        assert!(world.resource_mut::<Counter>().is_err());
    }

    #[test]
    fn insert_and_get() {
        let world = World::default();
        world.insert_resource(Counter(1));
        world.insert_resource(Other);
        assert!(world.has_resource::<Counter>());
        assert_eq!(*world.resource::<Counter>().unwrap(), Counter(1));
        assert!(world.resource::<Other>().is_ok());
    }

    #[test]
    fn mutate_is_visible_to_clones() {
        let world = World::default();
        world.insert_resource(Counter(1));
        world.clone().resource_mut::<Counter>().unwrap().0 += 1;
        assert_eq!(world.resource::<Counter>().unwrap().0, 2);
    }

    #[test]
    fn insert_replaces() {
        let world = World::default();
        world.insert_resource(Counter(1));
        world.insert_resource(Counter(5));
        assert_eq!(world.resource::<Counter>().unwrap().0, 5);
    }

    #[test]
    fn remove() {
        let world = World::default();
        world.insert_resource(Counter(1));
        let held = world.resource::<Counter>().unwrap();
        assert!(world.remove_resource::<Counter>());
        assert!(!world.remove_resource::<Counter>());
        assert!(world.resource::<Counter>().is_err());
        assert_eq!(held.0, 1);
    }
}
