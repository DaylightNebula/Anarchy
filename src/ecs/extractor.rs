use derive_more::{Deref, DerefMut};

use crate::{ScheduleID, World};

/// Allow the extractor implementor to pull data for a system in a standardized way.
pub trait SystemExtractor<'a, I> {
    fn extract(
        id: ScheduleID, 
        world: &'a World, 
        inputs: Option<&'a I>
    ) -> (Self, Option<&'a I>) where Self: Sized;
}

/// Allows the creator of the input to declare inputs to a system.
#[derive(Debug, Deref, DerefMut)]
pub struct Input<'a, T>(&'a T);
impl <'a, T> SystemExtractor<'a, &'a T> for Input<'a, T> {
    fn extract(
        _id: ScheduleID, 
        _world: &'a World, 
        mut inputs: Option<&'a &'a T>
    ) -> (Self, Option<&'a &'a T>) {
        (Self(
            inputs
                .take()
                .expect("Input extract called twice in one system.  This is not how this should be used!")
        ), None)
    }
}

/// Extract the schedule ID.
#[derive(Debug, Deref, DerefMut)]
pub struct GetScheduleID(ScheduleID);
impl <'a, I> SystemExtractor<'a, I> for GetScheduleID {
    fn extract(
        id: ScheduleID, 
        _world: &'a World, 
        _inputs: Option<&'a I>
    ) -> (Self, Option<&'a I>) {
        (Self(id), _inputs)
    }
}
