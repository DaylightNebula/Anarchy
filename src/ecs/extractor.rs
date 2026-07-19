use anyhow::bail;
use derive_more::{Deref, DerefMut};

use crate::{ScheduleID, World};

/// Allow the extractor implementor to pull data for a system in a standardized way.
pub trait SystemExtractor<'a, I> {
    /// Extracts `Self` from the given `World` and, if present, the system's shared input.
    /// Implementors that consume the input (e.g. `Input`) return `None` in its place so
    /// later extractors in the same system can detect a double-extraction attempt.
    fn extract(
        id: ScheduleID,
        world: &'a World,
        inputs: Option<&'a I>
    ) -> anyhow::Result<(Self, Option<&'a I>)> where Self: Sized;
}

/// Allows the creator of the input to declare inputs to a system.
#[derive(Debug, Deref, DerefMut)]
pub struct Input<'a, T>(&'a T);
impl <'a, T> SystemExtractor<'a, &'a T> for Input<'a, T> {
    fn extract(
        _id: ScheduleID, 
        _world: &'a World, 
        mut inputs: Option<&'a &'a T>
    ) -> anyhow::Result<(Self, Option<&'a &'a T>)> {
        let Some(inputs) = inputs.take() else { bail!("Inputs missing") };

        Ok((Self(inputs), None))
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
    ) -> anyhow::Result<(Self, Option<&'a I>)> {
        Ok((Self(id), _inputs))
    }
}
