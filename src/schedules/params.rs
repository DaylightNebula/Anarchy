use crate::{SharedExecutionState, World};

/// A parameter that a system may use as a parameter to extract
/// something from the game world.
pub trait SystemParam {
    fn extract(world: &World, exec_state: &SharedExecutionState) -> Self;
}

impl SystemParam for () {
    fn extract(_world: &World, _exec_state: &SharedExecutionState) -> Self {
        ()
    }
}

impl SystemParam for World {
    fn extract(world: &World, _exec_state: &SharedExecutionState) -> Self {
        world.clone()
    }
}
