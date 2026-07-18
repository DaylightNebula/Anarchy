pub mod bst_tree;
pub mod linear;

pub use bst_tree::*;
pub use linear::*;

use mutual::Ref;

use crate::entities::{Entity, EntityID};

pub trait WorldDatabase {
    /// Query all entities who have the components in the given mask.
    /// In this case that mask represents component IDs.  Each component 
    /// ID is stored at its equivalent bit position (i.e. 9 is byte 1 bin 1), 
    /// and checked against the entities returned by the query.
    fn query<'a>(&'a self, mask: Box<[u8]>) -> impl Iterator<Item = (Box<[u8]>, Box<dyn Iterator<Item = Ref<Entity>>>)>;

    /// Remove an iterator of IDs from the database.  Those entities must all
    /// be in the mask provided.  See above for a description of what the mask 
    /// represents.
    fn remove_all<'a, I: IntoIterator<Item = u32> + Clone>(&self, mask: &[u8], set: I);

    /// Insert an entity to this database.  This function also takes a 
    /// mask of where to insert the entity.  See above for a description
    /// of what the mask represents.
    fn insert_raw(&self, mask: &[u8], entity: Entity);

    /// Removes an entity from this database.  This fucntion also takes a
    /// mask of the components in the entity to search the database.  See
    /// above for description of what this mask represents.
    fn remove_raw(&self, mask: &[u8], entity: EntityID);

    /// Inserts an entity, deriving its mask from its own components.
    fn insert(&self, entity: Entity) {
        self.insert_raw(&entity.build_bit_mask(), entity);
    }

    /// Removes an entity, deriving its search mask from its own components.
    fn remove(&self, entity: &Entity) {
        self.remove_raw(&entity.build_bit_mask(), entity.0);
    }
}
