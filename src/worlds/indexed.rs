//! [`IndexedWorld`], the default entity storage.

use std::{fmt::Debug, sync::{Arc, Mutex}};

use mutual::{ArcSwap, SharedData};
use rustc_hash::FxHashMap;

use crate::*;

/// A world that keeps an index from each component to the tables holding it,
/// so queries only visit tables that have the rarest required component.
/// Queries with several requirements are cached, so asking again only costs
/// a hash lookup however many tables the world has.
///
/// The index is copy-on-write: queries read it with a single atomic load,
/// and it's only rebuilt when a new table is created or a new query is cached,
/// both of which are rare.
pub struct IndexedWorld {
    index: ArcSwap<Index>,
    /// Held while creating a table or caching a query so two threads can't both
    /// create the same group, and so index rebuilds never race each other.
    create_lock: Mutex<()>
}

#[derive(Default, Clone)]
struct Index {
    /// Exact group to its table, used by insert.
    by_group: FxHashMap<ComponentIDGroup, Arc<Table>>,
    /// Component to every table whose group contains it, used by queries.
    by_component: FxHashMap<ComponentID, Arc<[Arc<Table>]>>,
    /// Sorted requirements (more than one) to every table matching them, kept up to date by `with`.
    by_query: FxHashMap<ComponentIDGroup, Arc<[Arc<Table>]>>
}

impl Index {
    /// Returns a copy of this index with `table` added.
    fn with(&self, table: Arc<Table>) -> Self {
        let mut index = self.clone();
        for id in table.group().iter() {
            let tables = index.by_component.entry(*id).or_insert_with(|| Arc::from([]));
            *tables = append(tables, &table);
        }
        // a new table is the only thing that can change what a query matches
        for (req, tables) in index.by_query.iter_mut() {
            if group_matches(table.group(), req) {
                *tables = append(tables, &table);
            }
        }
        index.by_group.insert(table.group().clone(), table);
        index
    }

    /// Every table matching the sorted `req`, `None` if a required component has no tables.
    fn matching(&self, req: &ComponentIDGroup) -> Option<Arc<[Arc<Table>]>> {
        // only tables holding the rarest required component can match
        let mut candidates: Option<&Arc<[Arc<Table>]>> = None;
        for id in req.iter() {
            let tables = self.by_component.get(id)?;
            if candidates.is_none_or(|c| tables.len() < c.len()) {
                candidates = Some(tables);
            }
        }
        Some(candidates?.iter().filter(|table| group_matches(table.group(), req)).cloned().collect())
    }
}

fn append(tables: &Arc<[Arc<Table>]>, table: &Arc<Table>) -> Arc<[Arc<Table>]> {
    tables.iter().cloned().chain(std::iter::once(table.clone())).collect()
}

impl IndexedWorld {
    /// Create an empty world.
    pub fn new() -> Self {
        Self { index: ArcSwap::from_pointee(Index::default()), create_lock: Mutex::new(()) }
    }

    fn find_table(&self, group: &ComponentIDGroup) -> Option<Arc<Table>> {
        self.index.load().by_group.get(group).cloned()
    }

    /// Finds every table matching the sorted `req` and caches the result, unless
    /// a required component has no tables, which would let the cache grow without bound.
    fn cache_query(&self, req: ComponentIDGroup) -> Option<Arc<[Arc<Table>]>> {
        // match against the index under the lock, so a table created meanwhile can't be missed
        let _guard = self.create_lock.lock().unwrap();
        let index = self.index.load();
        if let Some(tables) = index.by_query.get(&req) {
            return Some(tables.clone());
        }

        let tables = index.matching(&req)?;
        let mut index = Index::clone(&index);
        index.by_query.insert(req, tables.clone());
        self.index.store(Arc::new(index));
        Some(tables)
    }
}

impl Default for IndexedWorld {
    fn default() -> Self { Self::new() }
}

impl Debug for IndexedWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IndexedWorld")
    }
}

impl WorldImpl for IndexedWorld {
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
        let _guard = self.create_lock.lock().unwrap();
        if let Some(table) = self.find_table(&reqs) {
            drop(_guard);
            table.insert(entity_id, components);
            return;
        }

        // fill the table before publishing so other threads never see it empty
        let table = Table::default(&reqs);
        table.insert(entity_id, components);
        let index = self.index.load().with(Arc::new(table));
        self.index.store(Arc::new(index));
    }

    fn raw_query<'a>(&'a self, mut req_components: ComponentIDGroup) -> Box<dyn Iterator<Item = Cursor>> {
        req_components.sort();
        let index = self.index.load();

        // a single requirement's tables are already indexed exactly
        let tables = if let [id] = *req_components {
            index.by_component.get(&id).cloned()
        } else if let Some(tables) = index.by_query.get(&req_components) {
            Some(tables.clone())
        } else {
            drop(index);
            self.cache_query(req_components)
        };

        match tables {
            Some(tables) => Box::new(QueryIter { tables, pos: 0 }),
            None => Box::new(std::iter::empty())
        }
    }
}

/// Walks a list of tables that all match the query.
struct QueryIter {
    tables: Arc<[Arc<Table>]>,
    pos: usize
}

impl Iterator for QueryIter {
    type Item = Cursor;

    fn next(&mut self) -> Option<Cursor> {
        let table = self.tables.get(self.pos)?;
        self.pos += 1;
        Some(table.cursor())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.tables.len() - self.pos;
        (left, Some(left))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::worlds::test_support::TestWorld;

    impl TestWorld for IndexedWorld {
        fn new_world() -> Self { IndexedWorld::new() }
        fn table_count(&self) -> usize { self.index.load().by_group.len() }
        fn groups(&self) -> Vec<Vec<ComponentID>> {
            self.index.load().by_group.keys().map(|group| group.to_vec()).collect()
        }
    }

    crate::worlds::world_tests!(IndexedWorld);

    #[test]
    fn component_index_holds_exactly_the_tables_with_each_component() {
        let world = world();
        for mask in 1..16u32 {
            world.insert(mask as EntityID, comps(by_mask(mask)));
        }

        let index = world.index.load();
        for (bit, id) in [A::id(), B::id(), C::id(), D::id()].into_iter().enumerate() {
            let mut expected = (1..16u32)
                .filter(|mask| mask & (1 << bit) != 0)
                .map(|mask| sorted(by_mask(mask).iter().map(|c| c.get_id()).collect()))
                .collect::<Vec<_>>();
            let mut got = index.by_component[&id].iter()
                .map(|table| table.group().to_vec())
                .collect::<Vec<_>>();
            expected.sort();
            got.sort();
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn query_for_unknown_component_visits_no_tables() {
        let world = world();
        world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(1))]));
        // D has no tables, so this short-circuits even though A matches
        let iter = world.raw_query(ids(&[A::id(), D::id()]));
        assert_eq!(iter.size_hint(), (0, Some(0)));
    }

    #[test]
    fn query_returns_exactly_the_matching_tables() {
        let world = world();
        // A is in 4 tables, but only one also has D
        world.insert(1, comps(vec![Box::new(A(1))]));
        world.insert(2, comps(vec![Box::new(A(2)), Box::new(B(2))]));
        world.insert(3, comps(vec![Box::new(A(3)), Box::new(C(3))]));
        world.insert(4, comps(vec![Box::new(A(4)), Box::new(D(4))]));

        let iter = world.raw_query(ids(&[A::id(), D::id()]));
        assert_eq!(iter.size_hint(), (1, Some(1)));
        assert_eq!(entities(&world, &[A::id(), D::id()]), [4]);

        let iter = world.raw_query(ids(&[A::id()]));
        assert_eq!(iter.size_hint(), (4, Some(4)));
    }

    fn cached(world: &IndexedWorld) -> usize { world.index.load().by_query.len() }

    #[test]
    fn repeated_query_is_cached_once() {
        let world = world();
        world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(1)), Box::new(C(1))]));
        world.insert(2, comps(vec![Box::new(A(2)), Box::new(C(2))]));

        let first = entities(&world, &[A::id(), C::id()]);
        assert_eq!(first, [1, 2]);
        assert_eq!(cached(&world), 1);
        assert_eq!(entities(&world, &[A::id(), C::id()]), first);
        // requirement order doesn't make a new entry
        assert_eq!(entities(&world, &[C::id(), A::id()]), first);
        assert_eq!(cached(&world), 1);
    }

    #[test]
    fn single_and_unknown_requirements_are_not_cached() {
        let world = world();
        world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(1))]));
        assert_eq!(entities(&world, &[A::id()]), [1]);
        assert_eq!(entities(&world, &[A::id(), D::id()]), [] as [EntityID; 0]);
        assert_eq!(cached(&world), 0);
    }

    #[test]
    fn cached_query_sees_new_tables() {
        let world = world();
        world.insert(1, comps(vec![Box::new(A(1)), Box::new(B(1))]));
        assert_eq!(entities(&world, &[A::id(), B::id()]), [1]);

        // inserting into an existing table leaves the cache alone
        let key = ids(&sorted(vec![A::id(), B::id()]));
        let before = world.index.load().by_query[&key].clone();
        world.insert(2, comps(vec![Box::new(B(2)), Box::new(A(2))]));
        assert!(Arc::ptr_eq(&before, &world.index.load().by_query[&key]));
        assert_eq!(entities(&world, &[A::id(), B::id()]), [1, 2]);

        // a new matching table is added, a new non-matching one is not
        world.insert(3, comps(vec![Box::new(A(3)), Box::new(B(3)), Box::new(C(3))]));
        world.insert(4, comps(vec![Box::new(A(4)), Box::new(C(4))]));
        assert_eq!(world.raw_query(ids(&[B::id(), A::id()])).size_hint(), (2, Some(2)));
        assert_eq!(entities(&world, &[A::id(), B::id()]), [1, 2, 3]);
    }

    #[test]
    fn cached_empty_query_fills_in() {
        let world = world();
        world.insert(1, comps(vec![Box::new(A(1))]));
        world.insert(2, comps(vec![Box::new(B(2))]));
        assert_eq!(entities(&world, &[A::id(), B::id()]), [] as [EntityID; 0]);
        assert_eq!(cached(&world), 1);

        world.insert(3, comps(vec![Box::new(A(3)), Box::new(B(3))]));
        assert_eq!(entities(&world, &[A::id(), B::id()]), [3]);
    }

    #[test]
    fn cache_matches_full_scan_under_concurrent_inserts() {
        const THREADS: u32 = 8;
        let reqs = [[A::id(), B::id()], [B::id(), D::id()], [A::id(), C::id()], [C::id(), D::id()]];
        for _ in 0..50 {
            let world = world();
            world.insert(0, comps(by_mask(0b1111)));
            let barrier = std::sync::Barrier::new(THREADS as usize * 2);
            std::thread::scope(|s| for t in 0..THREADS {
                let (world, barrier, reqs) = (&world, &barrier, &reqs);
                // writers create new groups while readers fill the cache
                s.spawn(move || {
                    barrier.wait();
                    for mask in (1..16u32).filter(|m| m % THREADS == t) {
                        world.insert(mask as EntityID, comps(by_mask(mask)));
                    }
                });
                s.spawn(move || {
                    barrier.wait();
                    for req in reqs { world.raw_query(ids(req)).count(); }
                });
            });

            let index = world.index.load();
            for (req, tables) in index.by_query.iter() {
                let mut got = tables.iter().map(|t| t.group().to_vec()).collect::<Vec<_>>();
                let mut expected = index.by_group.keys()
                    .filter(|group| group_matches(group, req))
                    .map(|group| group.to_vec())
                    .collect::<Vec<_>>();
                got.sort();
                expected.sort();
                assert_eq!(got, expected, "stale cache for {req:?}");
            }
        }
    }
}
