//! Worlds, which hold every entity and resource.

use std::{ops::Deref, sync::Arc};

use mutual::{DashMap, Mut, Ref, RefGuard, RelaxedMutex, SharedData};

use crate::{ComponentIDGroup, Cursor, DynComponents, DynResource, EntityID, Resource, ResourceID, ResourceMeta};

pub mod indexed;
pub mod list;

pub use indexed::*;
pub use list::*;

/// A shared handle to a world's entities and resources.  Clones are cheap and
/// share the same data.
///
/// Derefs to the [`WorldImpl`] storing its entities, [`IndexedWorld`] by default.
#[derive(Clone)]
pub struct World(Arc<WorldInner>);

struct WorldInner {
    entities: Box<dyn WorldImpl>,
    resources: DashMap<ResourceID, RelaxedMutex<DynResource>>
}

impl World {
    /// Create a world that stores its entities in `world`.
    pub fn new<W: WorldImpl + 'static>(world: W) -> Self {
        Self(Arc::new(WorldInner { entities: Box::new(world), resources: DashMap::default() }))
    }

    /// Insert a resource into the world, replacing any resource of the same type.
    pub fn insert_resource<R: Resource>(&self, resource: R) {
        let resource: DynResource = Box::new(resource);
        self.0.resources.insert(resource.get_id(), RelaxedMutex::new(resource));
    }

    /// Remove a resource from the world, returns true if it was present.
    /// Existing guards to the resource stay valid until dropped.
    pub fn remove_resource<R: ResourceMeta>(&self) -> bool {
        self.0.resources.remove(&R::id()).is_some()
    }

    /// Returns true if the world holds a resource of type `R`.
    pub fn has_resource<R: ResourceMeta>(&self) -> bool {
        self.0.resources.contains_key(&R::id())
    }

    /// Get immutable access to a resource, blocks while a mutable guard to it is held.
    pub fn resource<R: ResourceMeta>(&self) -> Option<Ref<R>> {
        let guard = self.resource_mutex::<R>()?.lock_ref();
        Some(Ref::new(
            guard,
            // the guard comes back type erased, so unwrap it before downcasting the resource
            |guard| guard.downcast_ref::<RefGuard<DynResource>>().unwrap().as_any().downcast_ref().unwrap()
        ))
    }

    /// Get mutable access to a resource, blocks while any other guard to it is held.
    pub fn resource_mut<R: ResourceMeta>(&self) -> Option<Mut<R>> {
        let guard = self.resource_mutex::<R>()?.lock_mut();
        Some(Mut::new(
            guard,
            |res| res.as_any().downcast_ref().unwrap(),
            |res| res.as_any_mut().downcast_mut().unwrap()
        ))
    }

    /// Clone the resource's mutex out of the map so the map's shard lock is not held while locking it.
    fn resource_mutex<R: ResourceMeta>(&self) -> Option<RelaxedMutex<DynResource>> {
        self.0.resources.get(&R::id()).map(|res| res.clone())
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

/// Entity storage behind a [`World`], which groups entities into [`Table`](crate::Table)s
/// by their set of components.  Any number of threads may use it at once.
pub trait WorldImpl: Send + Sync {
    /// Insert a new entity into the world with a given set of components.
    /// The entity must have at least one component.
    fn insert(&self, entity_id: EntityID, components: DynComponents);

    /// Create a raw query that produces an iterator of cursors across
    /// the tables that meet the required components given.
    /// Required components may not be empty.
    fn raw_query<'a>(&'a self, req_components: ComponentIDGroup) -> Box<dyn Iterator<Item = Cursor>>;
}

impl WorldImpl for World {
    fn insert(&self, entity_id: EntityID, components: DynComponents) {
        self.0.entities.insert(entity_id, components)
    }

    fn raw_query<'a>(&'a self, req_components: ComponentIDGroup) -> Box<dyn Iterator<Item = Cursor>> {
        self.0.entities.raw_query(req_components)
    }
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
        use mutual::SharedData;

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

            let mut seen = world.raw_query(ids(&[A::id(), B::id()]))
                .flat_map(|cursor| std::iter::from_fn(move || cursor.next()))
                .map(|(id, c)| (id, value_of::<A>(&c, |a| a.0), value_of::<B>(&c, |b| b.0)))
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
                .map(|cursor| std::iter::from_fn(|| cursor.next()).count())
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
            let found = cursors
                .flat_map(|cursor| std::iter::from_fn(move || cursor.next()))
                .map(|(id, _)| id)
                .collect::<Vec<_>>();
            assert!(found.contains(&1));
            assert!(found.contains(&2));
        }

        #[test]
        fn query_components_share_storage_with_world() {
            let world = world();
            world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(10))]));

            // mutate through one query...
            for cursor in world.raw_query(ids(&[A::id()])) {
                while let Some((_, c)) = cursor.next() {
                    let a = c.iter().find(|c| c.lock_ref().get_id() == A::id()).unwrap();
                    a.lock_mut().as_any_mut().downcast_mut::<A>().unwrap().0 = 42;
                }
            }

            // ...and see it through another
            let values = world.raw_query(ids(&[B::id(), A::id()]))
                .flat_map(|cursor| std::iter::from_fn(move || cursor.next()))
                .map(|(_, c)| (value_of::<A>(&c, |a| a.0), value_of::<B>(&c, |b| b.0)))
                .collect::<Vec<_>>();
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
            .flat_map(|cursor| std::iter::from_fn(move || cursor.next()))
            .map(|(id, c)| (id, c.iter().map(|c| c.lock_ref().get_id()).collect()))
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
        assert!(world.resource::<Counter>().is_none());
        assert!(world.resource_mut::<Counter>().is_none());
    }

    #[test]
    fn insert_and_get() {
        let world = World::default();
        world.insert_resource(Counter(1));
        world.insert_resource(Other);
        assert!(world.has_resource::<Counter>());
        assert_eq!(*world.resource::<Counter>().unwrap(), Counter(1));
        assert!(world.resource::<Other>().is_some());
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
        assert!(world.resource::<Counter>().is_none());
        // guards taken before the removal stay valid
        assert_eq!(held.0, 1);
    }
}
