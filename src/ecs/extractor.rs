use derive_more::{Deref, DerefMut};

use crate::{ScheduleID, World};

/// Allow the extractor implementor to pull data for a system in a standardized way.
pub trait SystemExtractor<'a, I> {
    fn extract(
        id: ScheduleID, 
        world: &'a World, 
        inputs: Option<I>
    ) -> (Self, Option<I>) where Self: Sized;
}

/// Allows the creator of the input to declare inputs to a system.
#[derive(Default, Debug, Deref, DerefMut)]
pub struct Input<T>(T);
impl <'a, T> SystemExtractor<'a, T> for Input<T> {
    fn extract(
        _id: ScheduleID, 
        _world: &'a World, 
        mut inputs: Option<T>
    ) -> (Self, Option<T>) {
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
        _inputs: Option<I>
    ) -> (Self, Option<I>) {
        (Self(id), _inputs)
    }
}
