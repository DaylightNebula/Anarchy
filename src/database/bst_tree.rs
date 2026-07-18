//! An experimental, work-in-progress alternative to `LinearDatabase` that indexes entities
//! by walking a binary tree of component IDs (one level per possible component) instead of
//! scanning a flat list of archetypes.  This is unfinished: removal is entirely `todo!()`,
//! the entity ID lookup table is only partially wired up, and it does not implement
//! `WorldDatabase`, so `World` cannot use it yet. Do not rely on this for anything beyond
//! experimentation.

use std::{collections::LinkedList, fmt::Debug, sync::{Arc, atomic::AtomicBool}};

use mutual::{CowData, Ref, SharedData, SharedList};

use crate::{ComponentID, Entity, EntityID, build_bit_mask};

/// Number of entity slots per block in `BSTWorldDatabase::entity_id_lookup`.
pub const ENTITY_BLOCK_SIZE: usize = 1024;

/// See the module-level documentation: an incomplete tree-based `WorldDatabase` alternative.
pub struct BSTWorldDatabase {
    pub root: CowData<BSTTreeNode>,
    pub entity_id_lookup: SharedList<[Option<Arc<Entity>>; ENTITY_BLOCK_SIZE]>
}

/// A single node in the component-ID binary tree used by `BSTWorldDatabase`.  Each node
/// represents a decision on one component ID: `has` leads to entities that carry it,
/// `doesnt_have` to those that don't; `table` holds entities whose archetype terminates
/// exactly at this node.
pub struct BSTTreeNode {
    pub id: ComponentID,
    pub has: CowData<BSTTreeNode>,
    pub doesnt_have: CowData<BSTTreeNode>,
    pub table: CowData<BSTTable>
}

impl Debug for BSTTreeNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let table_size = if self.table.is_null() { 0 } else { self.table.get_ref().table.len() };
        f.debug_struct("BSTTreeNode")
            .field("id", &self.id)
            .field("has", &self.has)
            .field("doesnt_have", &self.doesnt_have)
            .field("table", &table_size)
            .finish()
    }
}

/// The leaf of a `BSTTreeNode`: the entities whose archetype terminates at that node.
pub struct BSTTable {
    pub table: SharedList<Arc<Entity>>,
    pub locked: AtomicBool
}

/// A single result chunk from `BSTWorldDatabase::query`: the archetype mask of the node
/// paired with an iterator over its entities.
pub type EntityChunk = (Box<[u8]>, Box<dyn Iterator<Item = Ref<Entity>>>);

#[allow(dead_code, unused)]
impl BSTWorldDatabase {
    /// Creates a new, empty `BSTWorldDatabase`.
    pub fn new() -> BSTWorldDatabase {
        BSTWorldDatabase { root: CowData::new(Self::new_node(0)), entity_id_lookup: SharedList::new() }
    }

    /// Creates a new, empty `BSTTreeNode` for the given component ID.
    pub fn new_node(comp_id: ComponentID) -> BSTTreeNode {
        BSTTreeNode {
            id: comp_id,
            has: CowData::null(),
            doesnt_have: CowData::null(),
            table: CowData::null()
        }
    }

    /// Creates a new, empty `BSTTable`.
    pub fn new_table() -> BSTTable {
        BSTTable { table: SharedList::new(), locked: AtomicBool::new(false) }
    }

    /// Walks the tree, returning an `EntityChunk` per visited node whose entities should be
    /// considered for a query on `comp_ids`.
    pub fn query(&self, comp_ids: &[ComponentID]) -> impl Iterator<Item = EntityChunk> {
        let mut stack = LinkedList::new();
        stack.push_back((self.root.get_ref(), 0));
    
        std::iter::from_fn(move || {
            // get current node
            let Some((current, init_current_comp_id)) = stack.pop_front() else { return None };
            let mask = build_bit_mask(comp_ids);
            let mut current_comp_id = init_current_comp_id;

            // check if the query requires this component
            let only_has = current_comp_id < comp_ids.len() && comp_ids[current_comp_id] == current.id;
            if only_has { current_comp_id += 1 }
            let is_complete = current_comp_id >= comp_ids.len();

            // add next nodes to stack
            if only_has {
                if !current.has.is_null() { stack.push_back((current.has.get_ref(), current_comp_id)); }
            } else {
                if !current.has.is_null() { stack.push_back((current.has.get_ref(), current_comp_id)); }
                if !current.doesnt_have.is_null() { stack.push_back((current.doesnt_have.get_ref(), current_comp_id)); }
            }

            // return iterator over current table
            if current.table.is_null() || !is_complete {
                Some((mask, Box::new(std::iter::empty()) as Box<dyn Iterator<Item = Ref<Entity>>>))
            } else {
                Some((
                    mask, 
                    Box::new(
                        current.table
                            .get_ref()
                            .table
                            .iter()
                            .map(|a| Ref::new(a, |data| &***data.downcast_ref::<Ref<Arc<Entity>>>().unwrap()))
                    )
                ))
            }
        })
    }

    /// Inserts an entity into the tree, walking or creating nodes for each of its
    /// component IDs in order. Empty entities are discarded.
    pub fn insert(&self, entity: Entity) {
        let mut node = self.root.get_ref();
        let entity = Arc::new(entity);

        // discard empty entities
        if entity.1.is_empty() { return }

        // search for node for entity, build nodes as needed
        let last_comp_id = entity.1.last().unwrap().lock_ref().get_bit_mask();
        let mut entity_comp_pos = 0;
        for comp_id in 0 ..= last_comp_id {
            let has_comp = entity_comp_pos < entity.1.len() && entity.1[entity_comp_pos].lock_ref().get_bit_mask() == comp_id;

            if has_comp {
                entity_comp_pos += 1;

                // if this is the last component ID in the entity, insert here, otherwise, move along has path
                if last_comp_id == comp_id {
                    if node.table.is_null() { node.table.set(Self::new_table()); }
                    node.table.get_ref().table.push(entity.clone());
                    // self.entity_id_lookup.insert(entity.0, entity);
                    break; // we need this break to satisfy rust borrow checker
                } else {
                    if node.has.is_null() { node.has.set(Self::new_node(comp_id + 1)); }
                    node = node.has.get_ref();
                }
            } else {
                if node.doesnt_have.is_null() { node.doesnt_have.set(Self::new_node(comp_id + 1)); }
                node = node.doesnt_have.get_ref();
            }
        }

        // add to look up table
        let min_entity_blocks = (entity.id() as usize + ENTITY_BLOCK_SIZE - 1) / ENTITY_BLOCK_SIZE;
        if self.entity_id_lookup.len() < min_entity_blocks {
            let to_add = (min_entity_blocks - self.entity_id_lookup.len());
            for idx in 0 .. to_add {
                let mut block = [const { None }; ENTITY_BLOCK_SIZE];
                if idx == to_add { block[entity.id() as usize % ENTITY_BLOCK_SIZE] = Some(entity.clone()); }
                self.entity_id_lookup.push(block);
            }
        } else {
            let block_idx = entity.id() as usize / ENTITY_BLOCK_SIZE;
            // self.entity_id_lookup.
        }
    }

    fn remove_by_id(&self, entity: EntityID) {
        todo!()
    }

    fn remove_by_entity(&self, entity: Entity) {
        todo!()
    }

    fn remove_all<'a, I: IntoIterator<Item = u32> + Clone>(&self, set: I) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use mutual::{AsAny, SharedData};
    use crate::{BSTWorldDatabase, Component, ComponentID, ComponentMeta, EntityBuilder};

    pub struct TestCompA;
    impl ComponentMeta for TestCompA {
        fn bit_mask() -> ComponentID { 0 }
    }
    impl Component for TestCompA {
        fn get_bit_mask(&self) -> ComponentID { Self::bit_mask() }
    }
    impl AsAny for TestCompA {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }
    
    pub struct TestCompB;
    impl ComponentMeta for TestCompB {
        fn bit_mask() -> ComponentID { 1 }
    }
    impl Component for TestCompB {
        fn get_bit_mask(&self) -> ComponentID { Self::bit_mask() }
    }
    impl AsAny for TestCompB {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    pub struct TestCompC(u32);
    impl ComponentMeta for TestCompC {
        fn bit_mask() -> ComponentID { 2 }
    }
    impl Component for TestCompC {
        fn get_bit_mask(&self) -> ComponentID { Self::bit_mask() }
    }
    impl AsAny for TestCompC {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    #[test]
    fn insertion_test() {
        let test_a = EntityBuilder::default()
            .add(TestCompA)
            .build();

        let test_b = EntityBuilder::default()
            .add(TestCompA)
            .add(TestCompB)
            .build();

        let test_c = EntityBuilder::default()
            .add(TestCompB)
            .build();

        let test_ac = EntityBuilder::default()
            .add(TestCompA)
            .add(TestCompC(35))
            .build();

        let db = BSTWorldDatabase::new();
        db.insert(test_a);
        db.insert(test_b);
        db.insert(test_c);
        db.insert(test_ac);

        let filter_a = [TestCompA::bit_mask()];
        let filter_b = [TestCompB::bit_mask()];
        let filter_c = [TestCompC::bit_mask()];
        let filter_ab = [TestCompA::bit_mask(), TestCompB::bit_mask()];
        let filter_ac = [TestCompA::bit_mask(), TestCompC::bit_mask()];

        let query_a: usize = db.query(&filter_a).map(|a| a.1.count()).sum();
        let query_b: usize = db.query(&filter_b).map(|a| a.1.count()).sum();
        let query_c: usize = db.query(&filter_c).map(|a| a.1.count()).sum();
        let query_ab: usize = db.query(&filter_ab).map(|a| a.1.count()).sum();
        let query_ac: usize = db.query(&filter_ac).map(|a| a.1.count()).sum();

        println!("A {query_a}");
        println!("B {query_b}");
        println!("C {query_c}");
        println!("AB {query_ab}");
        println!("AC {query_ac}");

        assert!(query_a == 3);
        assert!(query_b == 2);
        assert!(query_c == 1);
        assert!(query_b == 2);
        assert!(query_ab == 1);
        assert!(query_ac == 1);

        let last = db.query(&filter_ac).map(|a| a.1).flatten().last().expect("Failed to find entity");
        let last = last.1[1].lock_ref().as_any().downcast_ref::<TestCompC>().expect("Failed to get component").0;
        assert!(last == 35);
    }
}
