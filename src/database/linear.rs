use ahash::AHashSet;
use mutual::{Ref, SharedList};

use crate::{database::WorldDatabase, ecs::{components::bit_masks_match, entities::{Entity, EntityID}}};

type ArchetypeId = Box<[u8]>;

/// The default `WorldDatabase` implementation.  Entities are grouped into rows keyed by
/// their exact archetype (component bit mask); a query scans every row and returns those
/// whose archetype matches the query's mask.
#[derive(Clone)]
pub struct LinearDatabase {
    rows: SharedList<(ArchetypeId, SharedList<Entity>)>
}

impl LinearDatabase {
    /// Creates a new, empty `LinearDatabase`.
    pub fn new() -> Self {
        Self {
            rows: SharedList::new()
        }
    }

    /// Returns every row whose archetype matches `mask`, without extracting components.
    pub fn query_raw(&self, mask: &[u8]) -> Vec<SharedList<Entity>> {
        let mut vec = Vec::new();

        for row in self.rows.iter() {
            if bit_masks_match(&row.0, mask) {
                vec.push(SharedList::clone(&row.1));
            }
        }

        return vec;
    }

    /// Removes every entity in `set` from rows matching `mask`, stopping early once `set`
    /// is emptied.
    pub fn remove_set(&self, mask: &[u8], mut set: AHashSet<EntityID>) {
        self.rows.iter()
            .filter(|a| bit_masks_match(&a.0, &mask))
            .for_each(|row| {
                row.1.remove_all(|entity| {
                    set.remove(&entity.0)
                });
            });
    }
}

impl WorldDatabase for LinearDatabase {
    fn insert_raw(&self, mask: &[u8], entity: Entity) {
        // convert given mask to an archetype ID
        let archetype = mask.to_vec().into_boxed_slice();

        // find a row that matches the above archetype
        let row = self.rows.iter()
            .find(|a| a.0 == archetype);

        // add entity to any found row, or create a new row for this entity
        if let Some(row) = row {
            row.1.push(entity);
        } else {
            let row = SharedList::new();
            row.push(entity);
            self.rows.push((archetype, row));
        }
    }

    fn remove_raw(&self, mask: &[u8], id: EntityID) {
        let archetype = mask.to_vec().into_boxed_slice();

        let remove_row = {
            let row = self.rows.iter()
                .find(|a| a.0 == archetype);

            if let Some(row) = row {
                row.1.remove_search(|other| other.0 == id);
                if row.1.len() == 0 { Some(mask) }
                else { None }
            } else { None }
        };

        if let Some(remove_row) = remove_row {
            self.rows.remove_search(|a| &*a.0 == remove_row);
        }
    }

    // fn query<'a>(&'a self, mask: Box<[u8]>) -> impl Iterator<Item = &'a Entity> {
    //     self.rows.iter()
    //         .filter_map(move |row| {
    //             if bit_masks_match(&row.0, &mask) {
    //                 Some(row.1.iter())
    //             } else { None }
    //         })
    //         .flatten()
    // }

    fn query(&self, mask: Box<[u8]>) -> impl Iterator<Item = (Box<[u8]>, Box<dyn Iterator<Item = Ref<Entity>>>)> {
        self.rows.iter()
            .filter_map(move |row| {
                if bit_masks_match(&row.0, &mask) {
                    let entity_mask = row.0.clone();
                    let entity_iter = Box::new(row.1.iter()) as Box<dyn Iterator<Item = Ref<Entity>>>;
                    Some((entity_mask, entity_iter))
                } else { None }
            })
    }
    
    fn remove_all<'a, I: IntoIterator<Item = u32> + Clone>(&self, mask: &[u8], iter: I) {
        self.remove_set(mask, iter.into_iter().collect::<AHashSet<EntityID>>());
    }
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use std::sync::{atomic::Ordering, OnceLock};

    use mutual::RelaxedMutex;

    use crate::{database::{linear::LinearDatabase, WorldDatabase}, ecs::{components::{self, Component, ComponentID, ComponentMeta, NEXT_BIT_MASK}, entities::Entity}, AsAny};

    static TESTA_BIT_MASK: OnceLock<ComponentID> = OnceLock::new();
    static TESTB_BIT_MASK: OnceLock<ComponentID> = OnceLock::new();
    static TESTC_BIT_MASK: OnceLock<ComponentID> = OnceLock::new();
    
    pub struct TestA(u32);
    impl ComponentMeta for TestA {
        fn bit_mask() -> ComponentID {
            *TESTA_BIT_MASK.get_or_init(|| {
                NEXT_BIT_MASK.fetch_add(1, Ordering::Relaxed)
            })
        }
    }
    impl Component for TestA {
        fn get_bit_mask(&self) -> ComponentID { Self::bit_mask() }
    }
    impl AsAny for TestA {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }
    
    pub struct TestB(u32);
    impl ComponentMeta for TestB {
        fn bit_mask() -> ComponentID {
            *TESTB_BIT_MASK.get_or_init(|| {
                NEXT_BIT_MASK.fetch_add(1, Ordering::Relaxed)
            })
        }
    }
    impl Component for TestB {
        fn get_bit_mask(&self) -> ComponentID { Self::bit_mask() }
    }
    impl AsAny for TestB {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }
    
    pub struct TestC(u32);
    impl ComponentMeta for TestC {
        fn bit_mask() -> ComponentID {
            *TESTC_BIT_MASK.get_or_init(|| {
                NEXT_BIT_MASK.fetch_add(1, Ordering::Relaxed)
            })
        }
    }
    impl Component for TestC {
        fn get_bit_mask(&self) -> ComponentID { Self::bit_mask() }
    }
    impl AsAny for TestC {
        fn as_any(&self) -> &dyn std::any::Any { self }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    }

    #[test]
    pub fn linear_db_insert() {
        let db = LinearDatabase::new();

        let a: Vec<RelaxedMutex<Box<dyn Component>>> = vec![RelaxedMutex::new(Box::new(TestA(0))), RelaxedMutex::new(Box::new(TestC(3)))];
        let b: Vec<RelaxedMutex<Box<dyn Component>>> = vec![RelaxedMutex::new(Box::new(TestB(2))), RelaxedMutex::new(Box::new(TestC(1)))];
        let c: Vec<RelaxedMutex<Box<dyn Component>>> = vec![RelaxedMutex::new(Box::new(TestB(6))), RelaxedMutex::new(Box::new(TestC(9)))];

        let a = Entity::new_raw(0, a.into_boxed_slice());
        let b = Entity::new_raw(1, b.into_boxed_slice());
        let c = Entity::new_raw(2, c.into_boxed_slice());

        db.insert_raw(&a.build_bit_mask(), a);
        db.insert_raw(&b.build_bit_mask(), b);
        db.insert_raw(&c.build_bit_mask(), c);

        let c_mask = components::build_bit_mask(&[TestC::bit_mask()]);
        let c_query = db.query_raw(&c_mask);

        assert!(c_query.len() == 2);
    }

    #[test]
    pub fn linear_db_insert_remove() {
        let db = LinearDatabase::new();

        let a: Vec<RelaxedMutex<Box<dyn Component>>> = vec![RelaxedMutex::new(Box::new(TestA(0))), RelaxedMutex::new(Box::new(TestC(3)))];
        let b: Vec<RelaxedMutex<Box<dyn Component>>> = vec![RelaxedMutex::new(Box::new(TestB(2))), RelaxedMutex::new(Box::new(TestC(1)))];
        let c: Vec<RelaxedMutex<Box<dyn Component>>> = vec![RelaxedMutex::new(Box::new(TestB(6))), RelaxedMutex::new(Box::new(TestC(9)))];

        let a = Entity::new_raw(0, a.into_boxed_slice());
        let b = Entity::new_raw(1, b.into_boxed_slice());
        let c = Entity::new_raw(2, c.into_boxed_slice());

        db.insert_raw(&a.build_bit_mask(), a);
        db.insert_raw(&b.build_bit_mask(), b);
        db.insert_raw(&c.build_bit_mask(), c);

        let c_mask = components::build_bit_mask(&[TestC::bit_mask()]);
        let ac_mask = components::build_bit_mask(&[TestA::bit_mask(), TestC::bit_mask()]);
        db.remove_raw(&ac_mask, 0);
        let c_query = db.query_raw(&c_mask);

        assert!(c_query.len() == 1);
    }

    #[test]
    pub fn linear_drop_test() {
        let db = LinearDatabase::new();

        let a: Vec<RelaxedMutex<Box<dyn Component>>> = vec![RelaxedMutex::new(Box::new(TestA(0))), RelaxedMutex::new(Box::new(TestC(3)))];
        let b: Vec<RelaxedMutex<Box<dyn Component>>> = vec![RelaxedMutex::new(Box::new(TestB(2))), RelaxedMutex::new(Box::new(TestC(1)))];
        let c: Vec<RelaxedMutex<Box<dyn Component>>> = vec![RelaxedMutex::new(Box::new(TestB(6))), RelaxedMutex::new(Box::new(TestC(9)))];

        let a = Entity::new_raw(0, a.into_boxed_slice());
        let b = Entity::new_raw(1, b.into_boxed_slice());
        let c = Entity::new_raw(2, c.into_boxed_slice());

        db.insert_raw(&a.build_bit_mask(), a);
        db.insert_raw(&b.build_bit_mask(), b);
        db.insert_raw(&c.build_bit_mask(), c);
        
        drop(db);
        assert!(true);
    }
}
