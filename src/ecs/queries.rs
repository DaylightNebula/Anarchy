use std::{marker::PhantomData};

use ahash::AHashSet;
use mutual::{RelaxedMutex, SharedData};

use crate::{Component, LinearDatabase, MaskBuilder, SystemExtractor, WorldDatabase, entities::{Entity, EntityID}};

pub mod indiv;
pub mod group;

pub use indiv::*;
pub use group::*;

/// The system extract to query a world.
pub struct Query<'a, Q: ComponentGroupExtractor> {
    db: &'a LinearDatabase,
    _phantom: PhantomData<Q>
}

impl <'a, Q: ComponentGroupExtractor> Query<'a, Q> {
    /// Create a new Query from a reference to the given database.
    pub fn new(db: &'a LinearDatabase) -> Self {
        Self { db, _phantom: PhantomData::default() }
    }

    /// Create an iterator from this query to loop through all entities
    /// that match this query.
    pub fn as_iter<'b>(&self) -> QueryIter<'b, Q> 
        where 'a: 'b
    {
        Q::new_iter(self.db)
    }

    /// Remove an entity from the internal database that matches this query.
    /// We do these by queries instead of globally for greatly increased effecienty
    pub fn remove(&self, id: EntityID) {
        let mut req_builder = MaskBuilder::new();
        Q::append_req_mask(&mut req_builder);
        let req_mask = req_builder.build();
        self.db.remove_raw(&req_mask, id);
    }

    /// Filters all entities by the given function, removing all from the internal
    /// database that matches this query.  We do these by queries instead fo globally
    /// for greatly increased effeciency.  The given function will remove all entities
    /// that the given function returns true when given the entities components as
    /// described by this query.
    pub fn remove_all<I: IntoIterator<Item = EntityID> + Clone>(&self, iter: I) {
        let mut req_builder = MaskBuilder::new();
        Q::append_req_mask(&mut req_builder);
        let req_mask = req_builder.build();
        self.db.remove_all(&req_mask, iter);
    }

    /// Removes a set of entities from the internal database by there IDs.  This is a 
    /// slightly faster version of `remove_all` that plays nicer with `LinearDatabase`. 
    pub fn remove_set(&self, set: AHashSet<EntityID>) {
        let mut req_builder = MaskBuilder::new();
        Q::append_req_mask(&mut req_builder);
        let req_mask = req_builder.build();
        self.db.remove_set(&req_mask, set);
    }
}

impl <'a, I, Q: ComponentGroupExtractor> SystemExtractor<'a, I> for Query<'a, Q> 
    where Self: QueryCreator<InputTypes = Q> 
{
    fn extract(
        _id: crate::ScheduleID, 
        world: &'a crate::World, 
        _inputs: Option<I>
    ) -> (Self, Option<I>) {
        (Self::new(world.database()), _inputs)
    }
}

/// A simple wrapper to a boxed iterator of entities that match a query.
pub struct QueryIter<'a, Q: ComponentGroupExtractor>(Box<dyn Iterator<Item = (EntityID, Box<[u8]>, Q::GroupOutput)> + 'a>);
impl <'a, Q: ComponentGroupExtractor> QueryIter<'a, Q> {
    pub fn iter_with_id(self) -> impl Iterator<Item = (EntityID, Box<[u8]>, Q::GroupOutput)> {
        self.0
    }
}

impl <'a, Q: ComponentGroupExtractor> Query<'a, Q> {
    pub fn iter_with_id(&self) -> impl Iterator<Item = (EntityID, Box<[u8]>, Q::GroupOutput)> {
        self.as_iter().0
    }
}

impl <'a, Q: ComponentGroupExtractor> Iterator for QueryIter<'a, Q> {
    type Item = Q::GroupOutput;
    fn next(&mut self) -> Option<Self::Item> {
        self.0.next().map(|a| a.2)
    }
}

/// Implemented by `Query` multiple times below to allow a QueryIter 
/// to be created for a reference to the current `World` database.
pub trait QueryCreator {
    type InputTypes: ComponentGroupExtractor;
    fn new<'a, D: WorldDatabase>(db: &'a D) -> QueryIter<'a, Self::InputTypes>;
}

impl <'b, Q: ComponentGroupExtractor> QueryCreator for Query<'b, Q> {
    type InputTypes = Q;

    fn new<'a, D: WorldDatabase>(db: &'a D) -> QueryIter<'a, Q> {
        Q::new_iter(db)
    }
}

pub type ExtractContext = Vec<Option<usize>>;

/// This version of extract comps is designed to excelerate queries by keeping track of indexed of 
/// components in entities.  Optimized for queries component extraction. 
pub fn extract_comps<'a>(
    entity: &'a Entity, 
    search_ids: &[u32], 
    ctx: &'a Option<ExtractContext>
) -> (impl Iterator<Item = Option<&'a RelaxedMutex<Box<dyn Component>>>> + 'a, Option<ExtractContext>) {
    if let Some(ctx) = ctx {
        let result = ctx.iter().map(|opt| opt.map(|idx| &entity.1[idx]));
        (itertools::Either::Left(result), None)
    } else {
        let mut out = vec![None; search_ids.len()];
        let mut ctx: ExtractContext = vec![None; search_ids.len()];
        
        entity.1.iter().enumerate().for_each(|(c_idx, comp)| {
            if comp.current_thread_using() { return }
            let comp_id = { comp.lock_ref().get_bit_mask() };
            if let Some(idx) = search_ids.iter().position(|a| *a == comp_id) {
                out[idx] = Some(comp);
                ctx[idx] = Some(c_idx);
            }
        });

        (itertools::Either::Right(out.into_iter()), Some(ctx))
    }
}

/// This version of extract comps is designed to excelerate queries by keeping track of indexed of 
/// components in entities.  Optimized for contexts where ExtractContext is shared.
pub fn extract_comps_distributed<'a>(
    entity: &'a Entity, 
    search_ids: &[u32], 
    ctx: Option<&'a ExtractContext>
) -> (impl Iterator<Item = Option<&'a RelaxedMutex<Box<dyn Component>>>> + 'a, Option<ExtractContext>) {
    if let Some(ctx) = ctx {
        let result = ctx.iter().map(|opt| opt.map(|idx| &entity.1[idx]));
        (itertools::Either::Left(result), None)
    } else {
        let mut out = vec![None; search_ids.len()];
        let mut ctx_vec: ExtractContext = vec![None; search_ids.len()];
        
        entity.1.iter().enumerate().for_each(|(c_idx, comp)| {
            if comp.current_thread_using() { return }
            let comp_id = { comp.lock_ref().get_bit_mask() };
            if let Some(idx) = search_ids.iter().position(|a| *a == comp_id) {
                out[idx] = Some(comp);
                ctx_vec[idx] = Some(c_idx);
            }
        });

        (itertools::Either::Right(out.into_iter()), Some(ctx_vec))
    }
}

#[allow(unused)]
pub(crate) fn find_set_bits(bytes: &[u8], req_mask: &[u8], opt_mask: &[u8]) -> Vec<Option<usize>> {
    bytes
        .iter()
        .enumerate()
        .filter(|a| a.0 < req_mask.len() || a.0 < opt_mask.len())
        .flat_map(|(byte_idx, &byte)| {
            let req_mask = req_mask.get(byte_idx);
            let opt_mask = opt_mask.get(byte_idx);

            (0..8).filter_map(move |bit_idx| {
                let is_req = req_mask.map(|byte| byte & (1 << bit_idx) != 0).unwrap_or(false);
                let is_opt = opt_mask.map(|byte| byte & (1 << bit_idx) != 0).unwrap_or(false);

                if is_req || is_opt {
                    if byte & (1 << bit_idx) != 0 { Some(Some(byte_idx * 8 + bit_idx)) }
                    else { Some(None) }
                } else { None }
            })
        })
        .collect()
}
