use std::sync::{Arc, Mutex};

use mutual::{ArcSwap, SharedData};
use rustc_hash::FxHashMap;

use crate::*;

/// A world that keeps an index from each component to the tables holding it,
/// so queries only visit tables that have the rarest required component.
///
/// The index is copy-on-write: queries read it with a single atomic load,
/// and it's only rebuilt when a new table is created, which is rare.
pub struct IndexedWorld {
    index: ArcSwap<Index>,
    /// Held while creating a table so two threads can't both create the same group,
    /// and so index rebuilds never race each other.
    create_lock: Mutex<()>
}

#[derive(Default, Clone)]
struct Index {
    /// Exact group to its table, used by insert.
    by_group: FxHashMap<ComponentIDGroup, Arc<Table>>,
    /// Component to every table whose group contains it, used by queries.
    by_component: FxHashMap<ComponentID, Arc<[Arc<Table>]>>
}

impl Index {
    /// Returns a copy of this index with `table` added.
    fn with(&self, table: Arc<Table>) -> Self {
        let mut index = self.clone();
        for id in table.group().iter() {
            let tables = index.by_component.entry(*id).or_insert_with(|| Arc::from([]));
            *tables = tables.iter().cloned().chain(std::iter::once(table.clone())).collect();
        }
        index.by_group.insert(table.group().clone(), table);
        index
    }
}

impl IndexedWorld {
    pub fn new() -> Self {
        Self { index: ArcSwap::from_pointee(Index::default()), create_lock: Mutex::new(()) }
    }

    fn find_table(&self, group: &ComponentIDGroup) -> Option<Arc<Table>> {
        self.index.load().by_group.get(group).cloned()
    }
}

impl Default for IndexedWorld {
    fn default() -> Self { Self::new() }
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

        // only tables holding the rarest required component can match,
        // and if any required component has no tables nothing can match
        let mut candidates: Option<&Arc<[Arc<Table>]>> = None;
        for id in req_components.iter() {
            let Some(tables) = index.by_component.get(id) else { return Box::new(std::iter::empty()) };
            if candidates.is_none_or(|c| tables.len() < c.len()) {
                candidates = Some(tables);
            }
        }
        let Some(candidates) = candidates else { return Box::new(std::iter::empty()) };

        Box::new(QueryIter {
            candidates: candidates.clone(),
            pos: 0,
            // with one requirement every candidate matches, so skip the check
            filter: req_components.len() > 1,
            req: req_components
        })
    }
}

struct QueryIter {
    candidates: Arc<[Arc<Table>]>,
    pos: usize,
    req: ComponentIDGroup,
    filter: bool
}

impl Iterator for QueryIter {
    type Item = Cursor;

    fn next(&mut self) -> Option<Cursor> {
        while let Some(table) = self.candidates.get(self.pos) {
            self.pos += 1;
            if !self.filter || group_matches(table.group(), &self.req) {
                return Some(table.cursor());
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.candidates.len() - self.pos;
        (if self.filter { 0 } else { left }, Some(left))
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
    fn query_picks_rarest_component() {
        let world = world();
        // A is in 4 tables, D in only one
        world.insert(1, comps(vec![Box::new(A(1))]));
        world.insert(2, comps(vec![Box::new(A(2)), Box::new(B(2))]));
        world.insert(3, comps(vec![Box::new(A(3)), Box::new(C(3))]));
        world.insert(4, comps(vec![Box::new(A(4)), Box::new(D(4))]));

        let iter = world.raw_query(ids(&[A::id(), D::id()]));
        assert_eq!(iter.size_hint(), (0, Some(1)));
        assert_eq!(entities(&world, &[A::id(), D::id()]), [4]);

        // a single requirement needs no filtering, so the hint is exact
        let iter = world.raw_query(ids(&[A::id()]));
        assert_eq!(iter.size_hint(), (4, Some(4)));
    }
}
